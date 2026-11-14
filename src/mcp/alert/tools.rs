//! Price alert tools: list, create, edit, pause, resume and delete the app's price alerts. They
//! plan on the app thread with the alert sheet's own rules and hand the change to the alert
//! runtime, then wait briefly to say what is true now.

use crate::mcp::alert::{AlertCommand, AlertCommandOutcome, AlertUpdate, NewAlert};
use crate::mcp::bridge::{McpError, McpReply, REQUEST_TIMEOUT};
use crate::mcp::state::ClientIdentity;
use crate::mcp::tools::{Caller, DegenViewMcp, McpSource, failure, invalid, parse_id, unexpected};
use crate::mcp::types::{
    AlertChangeResult, AlertInfo, AlertsResult, CreateAlertResult, UpdateAlertResult,
};
use crate::model::alert::{AlertCurrency, AlertFrequency};
use crate::model::mcp::Permission;
use crate::model::watchlist::text::validate_market;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::time::Duration;

/// When a price alert fires. Percentage conditions are not offered yet: they need a live
/// reference price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpCondition {
    CrossesAbove,
    CrossesBelow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum McpCurrency {
    Usd,
    Eur,
    Gbp,
    Jpy,
    Chf,
}

impl McpCurrency {
    fn alert_currency(self) -> AlertCurrency {
        match self {
            Self::Usd => AlertCurrency::Usd,
            Self::Eur => AlertCurrency::Eur,
            Self::Gbp => AlertCurrency::Gbp,
            Self::Jpy => AlertCurrency::Jpy,
            Self::Chf => AlertCurrency::Chf,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpFrequency {
    Once,
    EveryTime,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAlertParams {
    /// The market data provider.
    pub source: McpSource,
    /// The provider's own symbol, as for `degenview_add_chart`.
    #[schemars(length(min = 1, max = 128))]
    pub symbol: String,
    pub condition: McpCondition,
    /// The trigger price as a plain decimal string in `currency`, e.g. `"70000"` or `"0.00000278"`.
    #[schemars(length(min = 1, max = 32))]
    pub target_price: String,
    /// Defaults to USD.
    pub currency: Option<McpCurrency>,
    /// `once` (default) fires one time; `every_time` re-arms after each crossing.
    pub frequency: Option<McpFrequency>,
    /// A label shown with the alert.
    #[schemars(length(max = 200))]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAlertParams {
    /// An `alert_id` from `degenview_list_price_alerts`.
    #[schemars(length(max = 20))]
    pub alert_id: String,
    /// Only the settings given change. At least one is required.
    pub condition: Option<McpCondition>,
    /// The new trigger price, as for `degenview_create_price_alert`.
    #[schemars(length(min = 1, max = 32))]
    pub target_price: Option<String>,
    pub currency: Option<McpCurrency>,
    pub frequency: Option<McpFrequency>,
    /// The new note; an empty string clears it.
    #[schemars(length(max = 200))]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlertIdParams {
    /// An `alert_id` from `degenview_list_price_alerts`.
    #[schemars(length(max = 20))]
    pub alert_id: String,
}

/// A decimal like `70000` or `0.5`: digits and at most one point, nothing else, so no sign,
/// exponent, `inf` or `NaN`. Returns it trimmed when it is a finite number above zero.
pub fn clean_price_text(raw: &str) -> Option<String> {
    let text = raw.trim();
    let well_formed = !text.is_empty()
        && text.len() <= 32
        && text.chars().all(|c| c.is_ascii_digit() || c == '.')
        && text.matches('.').count() <= 1
        && !text.starts_with('.')
        && !text.ends_with('.');
    let value = text.parse::<f64>().ok()?;
    (well_formed && value.is_finite() && value > 0.0).then(|| text.to_owned())
}

fn clean_note(raw: &str) -> Result<String, CallToolResult> {
    let note = raw.trim();
    if note.chars().any(char::is_control) {
        return Err(invalid("note must not contain control characters"));
    }
    Ok(note.to_owned())
}

/// Whether two renderings of an alert have the same settings (timestamps aside).
fn same_settings(a: &AlertInfo, b: &AlertInfo) -> bool {
    (
        &a.condition,
        &a.target_price,
        &a.percent,
        &a.currency,
        &a.frequency,
        &a.status,
        &a.note,
    ) == (
        &b.condition,
        &b.target_price,
        &b.percent,
        &b.currency,
        &b.frequency,
        &b.status,
        &b.note,
    )
}

fn parse_alert_id(raw: &str) -> Result<u64, CallToolResult> {
    parse_id(raw, "alert_id", "degenview_list_price_alerts")
}

const SETTLE_POLLS: usize = 20;
const SETTLE_INTERVAL: Duration = Duration::from_millis(100);

#[tool_router(router = alert_router, vis = "pub(in crate::mcp)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_list_price_alerts",
        description = "List DegenView price alerts: market, condition, target price, currency, frequency, status (active, paused, triggered), note and timestamps. Webhook endpoints and their secrets are never included. Alerts only evaluate while DegenView is running.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_price_alerts(
        &self,
        Caller(client): Caller,
    ) -> Result<Json<AlertsResult>, CallToolResult> {
        match self
            .bridge
            .call(
                &client,
                Permission::AlertsRead,
                AlertCommand::List { only: None },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Alerts(alerts)) => Ok(Json(alerts)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_create_price_alert",
        description = "Create a price alert that fires when a market crosses a price. Uses the same rules as the app's alert sheet; an identical existing alert is returned instead of a duplicate. The alert only evaluates while DegenView is running. Creating one may ask the user to allow notifications.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_price_alert(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<CreateAlertParams>,
    ) -> Result<Json<CreateAlertResult>, CallToolResult> {
        let source = params.source.data_source();
        let instrument = validate_market(source, &params.symbol).map_err(invalid)?;
        let target_text = clean_price_text(&params.target_price).ok_or_else(|| {
            invalid("target_price must be a plain positive decimal such as \"70000\" or \"0.5\"")
        })?;
        let note = clean_note(&params.note.unwrap_or_default())?;
        let request = NewAlert {
            instrument,
            crosses_above: params.condition == McpCondition::CrossesAbove,
            target_text,
            currency: params
                .currency
                .map_or(AlertCurrency::Usd, McpCurrency::alert_currency),
            frequency: match params.frequency {
                Some(McpFrequency::EveryTime) => AlertFrequency::EveryTime,
                _ => AlertFrequency::Once,
            },
            note,
        };
        let mut created = match self
            .bridge
            .call(
                &client,
                Permission::AlertsCreate,
                AlertCommand::Create(Box::new(request)),
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertCreated(created)) => created,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if created.status == "queued" {
            let id = parse_alert_id(&created.alert.alert_id)?;
            if let Some(stored) = self
                .settle(
                    &client,
                    Permission::AlertsCreate,
                    id,
                    created.command_id,
                    |alerts| !alerts.is_empty(),
                )
                .await
                .map_err(|error| failure(&error))?
                .and_then(|alerts| alerts.into_iter().next())
            {
                created = CreateAlertResult {
                    status: "created".into(),
                    alert: stored,
                    command_id: None,
                };
            }
        }
        Ok(Json(created))
    }

    #[tool(
        name = "degenview_update_price_alert",
        description = "Edit an existing price alert: its direction, trigger price, currency, frequency or note (only the settings you give change; the market cannot). A paused alert stays paused. Webhook settings are untouched. Percentage alerts only take a new frequency or note.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_price_alert(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<UpdateAlertParams>,
    ) -> Result<Json<UpdateAlertResult>, CallToolResult> {
        let alert_id = parse_alert_id(&params.alert_id)?;
        if params.condition.is_none()
            && params.target_price.is_none()
            && params.currency.is_none()
            && params.frequency.is_none()
            && params.note.is_none()
        {
            return Err(invalid(
                "give at least one of condition, target_price, currency, frequency or note",
            ));
        }
        let target_text = match &params.target_price {
            None => None,
            Some(raw) => Some(clean_price_text(raw).ok_or_else(|| {
                invalid(
                    "target_price must be a plain positive decimal such as \"70000\" or \"0.5\"",
                )
            })?),
        };
        let note = match &params.note {
            None => None,
            Some(raw) => Some(clean_note(raw)?),
        };
        let request = AlertUpdate {
            alert_id,
            crosses_above: params.condition.map(|c| c == McpCondition::CrossesAbove),
            target_text,
            currency: params.currency.map(McpCurrency::alert_currency),
            frequency: params.frequency.map(|f| match f {
                McpFrequency::Once => AlertFrequency::Once,
                McpFrequency::EveryTime => AlertFrequency::EveryTime,
            }),
            note,
        };
        let mut updated = match self
            .bridge
            .call(
                &client,
                Permission::AlertsEdit,
                AlertCommand::Update(Box::new(request)),
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertUpdated(updated)) => updated,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if updated.status == "queued" {
            let expected = updated.alert.clone();
            if let Some(stored) = self
                .settle(
                    &client,
                    Permission::AlertsEdit,
                    alert_id,
                    updated.command_id,
                    |alerts| {
                        alerts
                            .first()
                            .is_some_and(|alert| same_settings(alert, &expected))
                    },
                )
                .await
                .map_err(|error| failure(&error))?
                .and_then(|alerts| alerts.into_iter().next())
            {
                updated = UpdateAlertResult {
                    status: "updated".into(),
                    alert: stored,
                    command_id: None,
                };
            }
        }
        Ok(Json(updated))
    }

    #[tool(
        name = "degenview_pause_price_alert",
        description = "Pause a price alert so it stops evaluating; its settings are kept. Pausing a paused alert changes nothing.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn pause_price_alert(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        self.set_alert_paused(&client, &params.alert_id, true).await
    }

    #[tool(
        name = "degenview_resume_price_alert",
        description = "Resume a paused or fired price alert; it judges the price afresh and fires on the next crossing. Resuming an active alert changes nothing.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn resume_price_alert(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        self.set_alert_paused(&client, &params.alert_id, false)
            .await
    }

    #[tool(
        name = "degenview_delete_price_alert",
        description = "Permanently delete a price alert. This cannot be undone. Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_price_alert(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        let alert_id = parse_alert_id(&params.alert_id)?;
        let mut changed = match self
            .bridge
            .call(
                &client,
                Permission::AlertsDelete,
                AlertCommand::Delete { alert_id },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertChanged(changed)) => changed,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if matches!(
            self.settle(
                &client,
                Permission::AlertsDelete,
                alert_id,
                None,
                |alerts| { alerts.is_empty() }
            )
            .await,
            Ok(Some(_))
        ) {
            changed.status = "deleted".into();
        }
        Ok(Json(changed))
    }
}

impl DegenViewMcp {
    /// The alert runtime applies queued changes on its own schedule. Looks for the change to
    /// show up (briefly), so a reply can say what is true now rather than only "queued".
    /// Returns the alert's entry (empty once deleted) as soon as `done` holds, `Ok(None)` if it
    /// did not show up in time, and an error if the runtime refused the command
    /// (`command_id`) because the alerts changed under it.
    async fn settle(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        alert_id: u64,
        command_id: Option<u64>,
        done: impl Fn(&[AlertInfo]) -> bool,
    ) -> Result<Option<Vec<AlertInfo>>, McpError> {
        for _ in 0..SETTLE_POLLS {
            if let Some(command_id) = command_id {
                let outcome = self
                    .bridge
                    .call(
                        client,
                        permission,
                        AlertCommand::CommandOutcome { command_id },
                        Duration::from_secs(2),
                    )
                    .await;
                match outcome {
                    Ok(McpReply::AlertCommand(AlertCommandOutcome::Rejected(reason))) => {
                        return Err(AlertCommandOutcome::rejection_error(reason));
                    }
                    Ok(_) => {}
                    Err(_) => return Ok(None),
                }
            }
            let reply = self
                .bridge
                .call(
                    client,
                    permission,
                    AlertCommand::List {
                        only: Some(alert_id),
                    },
                    Duration::from_secs(2),
                )
                .await;
            match reply {
                Ok(McpReply::Alerts(result)) if done(&result.alerts) => {
                    return Ok(Some(result.alerts));
                }
                Ok(_) => {}
                Err(_) => return Ok(None),
            }
            tokio::time::sleep(SETTLE_INTERVAL).await;
        }
        Ok(None)
    }

    async fn set_alert_paused(
        &self,
        client: &ClientIdentity,
        raw_id: &str,
        paused: bool,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        let alert_id = parse_alert_id(raw_id)?;
        let mut changed = match self
            .bridge
            .call(
                &client,
                Permission::AlertsPause,
                AlertCommand::SetPaused { alert_id, paused },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertChanged(changed)) => changed,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if changed.status == "queued" {
            let wanted = if paused { "paused" } else { "active" };
            if matches!(
                self.settle(&client, Permission::AlertsPause, alert_id, None, |alerts| {
                    alerts.first().is_some_and(|alert| alert.status == wanted)
                })
                .await,
                Ok(Some(_))
            ) {
                changed.status = wanted.into();
            }
        }
        Ok(Json(changed))
    }
}
