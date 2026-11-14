use super::chart::source_slug;
use crate::host::alert::AlertHost;
use crate::host::mcp::confirmation;
use crate::mcp::alert::{AlertCommand, AlertCommandOutcome, AlertUpdate, NewAlert};
use crate::mcp::bridge::{McpError, McpReply};
use crate::mcp::confirm::Summary;
use crate::mcp::types::{
    AlertChangeResult, AlertChangeStatus, AlertConditionKind, AlertFrequencyKind, AlertInfo,
    AlertStatus, AlertsResult, CreateAlertResult, CreateAlertStatus, UpdateAlertResult,
    UpdateAlertStatus,
};
use crate::model::alert::draft::{Direction, PriceAlertDraft};
use crate::model::alert::{
    AlertCondition, AlertCurrency, AlertFrequency, AlertId, AlertPrecondition, AlertRuntimeCommand,
    AlertRuntimeCommandPayload, AlertState, PriceAlert,
};
use crate::model::watchlist::now_ms;
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::store::{SharedStore, Store};
use crate::workspace::ChartWorkspace;
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// The price-alert sheet's Save, as the app runs it for a new alert: it queues the save and, as
/// the sheet does, asks for notifications and registers the alert service.
pub trait SaveAlert {
    /// Returns the queued command's id. An error means nothing was queued and nothing else
    /// happened. `precondition` is checked by the runtime when it applies the save.
    fn save_checked(
        &self,
        store: &Store,
        alert: Box<PriceAlert>,
        baseline: Option<f64>,
        precondition: Option<AlertPrecondition>,
        now: i64,
    ) -> rusqlite::Result<u64>;
}

/// What the alert commands reach in the app.
pub struct AlertAccess<'a> {
    /// The alert host the commands plan against.
    pub host: &'a Mutex<AlertHost>,
    /// What the alert sheet prices a draft with.
    pub quotes: &'a WatchlistQuoteBook,
    pub fx_rates: &'a HashMap<AlertCurrency, f64>,
    /// The database whose command queue the alert runtime reads.
    pub store: &'a SharedStore,
    pub save: &'a dyn SaveAlert,
}

impl AlertAccess<'_> {
    fn host(&self) -> Result<MutexGuard<'_, AlertHost>, McpError> {
        self.host.lock().map_err(|_| alerts_unavailable())
    }

    /// Hands a change to the alert runtime through the command queue, as the Alerts window does.
    /// Returns the command's id, which [`AlertCommand::CommandOutcome`] can be asked about.
    fn queue(
        &self,
        ctx: &egui::Context,
        payload: AlertRuntimeCommandPayload,
        precondition: Option<AlertPrecondition>,
    ) -> Result<u64, McpError> {
        let mut command = AlertRuntimeCommand::new(payload, now_ms());
        command.precondition = precondition;
        let store = self.store.lock().map_err(|_| alerts_unavailable())?;
        store
            .enqueue_alert_command(&command)
            .map_err(|_| McpError::AppUnavailable("Could not queue the alert change".into()))?;
        ctx.request_repaint();
        Ok(command.id)
    }
}

/// The error MCP gives while price alerts cannot be reached.
pub(super) fn alerts_unavailable() -> McpError {
    McpError::AppUnavailable("Price alerts are unavailable right now".into())
}

/// Runs a price alert command on the app thread.
pub fn run(
    command: AlertCommand,
    alerts: &AlertAccess,
    ctx: &egui::Context,
) -> Result<McpReply, McpError> {
    match command {
        AlertCommand::List { only } => Ok(McpReply::Alerts(list_alerts(&*alerts.host()?, only))),
        AlertCommand::Create(request) => create(&request, alerts, ctx),
        AlertCommand::CommandOutcome { command_id } => Ok(McpReply::AlertCommand(
            alert_command_outcome(&*alerts.host()?, command_id),
        )),
        AlertCommand::Update(request) => update(&request, alerts, ctx),
        AlertCommand::SetPaused { alert_id, paused } => {
            let plan = plan_pause_alert(&*alerts.host()?, alert_id, paused)?;
            let status = match plan {
                StatePlan::Already if paused => AlertChangeStatus::AlreadyPaused,
                StatePlan::Already => AlertChangeStatus::AlreadyActive,
                StatePlan::Change { id, state } => {
                    alerts.queue(
                        ctx,
                        AlertRuntimeCommandPayload::SetState {
                            id,
                            state,
                            baseline: None,
                        },
                        None,
                    )?;
                    AlertChangeStatus::Queued
                }
            };
            Ok(McpReply::AlertChanged(AlertChangeResult {
                alert_id: alert_id.to_string(),
                status,
            }))
        }
        AlertCommand::Delete { alert_id } => {
            let id = plan_delete_alert(&*alerts.host()?, alert_id)?;
            alerts.queue(ctx, AlertRuntimeCommandPayload::Delete(id), None)?;
            Ok(McpReply::AlertChanged(AlertChangeResult {
                alert_id: id.to_string(),
                status: AlertChangeStatus::Queued,
            }))
        }
    }
}

fn create(
    request: &NewAlert,
    alerts: &AlertAccess,
    ctx: &egui::Context,
) -> Result<McpReply, McpError> {
    let now = now_ms();
    let plan = plan_create_alert(
        &*alerts.host()?,
        alerts.quotes,
        alerts.fx_rates,
        request,
        now,
    )?;
    match plan {
        CreatePlan::Existing(alert) => Ok(McpReply::AlertCreated(CreateAlertResult {
            status: CreateAlertStatus::AlreadyExists,
            alert: *alert,
            command_id: None,
        })),
        CreatePlan::New { alert, baseline } => {
            let info = alert_info(&alert, None);
            let store = alerts.store.lock().map_err(|_| alerts_unavailable())?;
            // The same commit the price-alert sheet's Save does, but a save that never reached
            // the queue is an error here, not a "queued".
            let command_id = alerts
                .save
                .save_checked(
                    &store,
                    alert,
                    baseline,
                    // Other requests in this frame planned against the same snapshot: the runtime
                    // decides again (duplicate, cap) when it applies.
                    Some(AlertPrecondition::Create {
                        max_alerts: MAX_ALERTS,
                    }),
                    now,
                )
                .map_err(|_| McpError::AppUnavailable("Could not queue the alert".into()))?;
            ctx.request_repaint();
            Ok(McpReply::AlertCreated(CreateAlertResult {
                status: CreateAlertStatus::Queued,
                alert: info,
                command_id: Some(command_id),
            }))
        }
    }
}

fn update(
    request: &AlertUpdate,
    alerts: &AlertAccess,
    ctx: &egui::Context,
) -> Result<McpReply, McpError> {
    let plan = plan_update_alert(
        &*alerts.host()?,
        alerts.quotes,
        alerts.fx_rates,
        request,
        now_ms(),
    )?;
    match plan {
        UpdatePlan::Unchanged(alert) => Ok(McpReply::AlertUpdated(UpdateAlertResult {
            status: UpdateAlertStatus::Unchanged,
            alert: *alert,
            command_id: None,
        })),
        UpdatePlan::Change {
            alert,
            base,
            baseline,
        } => {
            let info = alert_info(&alert, None);
            // Planned from a snapshot other requests may also have planned from: the runtime
            // applies it only to the alert it was planned from.
            let command_id = alerts.queue(
                ctx,
                AlertRuntimeCommandPayload::Save { alert, baseline },
                Some(AlertPrecondition::Unchanged(base)),
            )?;
            Ok(McpReply::AlertUpdated(UpdateAlertResult {
                status: UpdateAlertStatus::Queued,
                alert: info,
                command_id: Some(command_id),
            }))
        }
    }
}

/// What a price alert command the user is asked about would do (see [`confirmation`]).
pub fn describe(command: &AlertCommand, alerts: &AlertAccess) -> Result<Summary, McpError> {
    match command {
        AlertCommand::Delete { alert_id } => {
            confirmation::delete_alert(&*alerts.host()?, *alert_id)
        }
        _ => confirmation::plainly(command),
    }
}

/// Alerts one `degenview_list_price_alerts` call returns.
pub(super) const ALERT_PAGE: usize = 200;

/// No further alert is accepted from a client beyond this many in total.
pub const MAX_ALERTS: usize = 500;

/// A number as a plain decimal (`70000`, `0.00000278`, `2.5`): rounded to 12 significant digits,
/// which drops float noise (`110.00000000000001`) but not any digit a quote carries, and never
/// written in exponent form.
pub(super) fn decimal(value: f64) -> String {
    let rounded = format!("{value:.11e}").parse::<f64>().unwrap_or(value);
    format!("{rounded}")
}

pub fn alert_info(alert: &PriceAlert, last_triggered_ms: Option<i64>) -> AlertInfo {
    let currency = alert.currency;
    let price = decimal;
    let (condition, percent, reference) = match &alert.condition {
        AlertCondition::CrossesAbove { .. } => (AlertConditionKind::CrossesAbove, None, None),
        AlertCondition::CrossesBelow { .. } => (AlertConditionKind::CrossesBelow, None, None),
        AlertCondition::RisesBy {
            percent, reference, ..
        } => (
            AlertConditionKind::RisesBy,
            Some(decimal(*percent)),
            Some(price(*reference)),
        ),
        AlertCondition::FallsBy {
            percent, reference, ..
        } => (
            AlertConditionKind::FallsBy,
            Some(decimal(*percent)),
            Some(price(*reference)),
        ),
        AlertCondition::Unsupported { .. } => (AlertConditionKind::Unsupported, None, None),
    };
    AlertInfo {
        alert_id: alert.id.to_string(),
        instrument: alert.asset.key(),
        source: source_slug(alert.asset.source()).to_owned(),
        symbol: alert.asset.instrument.symbol.clone(),
        condition,
        target_price: alert.condition.target().map(price),
        percent,
        reference_price: reference,
        currency: currency.code().to_owned(),
        frequency: match alert.frequency {
            AlertFrequency::Once => AlertFrequencyKind::Once,
            AlertFrequency::EveryTime => AlertFrequencyKind::EveryTime,
        },
        status: match alert.state {
            AlertState::Active => AlertStatus::Active,
            AlertState::Paused => AlertStatus::Paused,
            AlertState::Triggered => AlertStatus::Triggered,
            AlertState::Unsupported => AlertStatus::Unsupported,
        },
        note: alert.note.clone(),
        sends_to_webhooks: !alert.webhook_endpoint_ids.is_empty(),
        created_at: crate::format::iso8601(alert.created_at_ms),
        updated_at: crate::format::iso8601(alert.updated_at_ms),
        last_triggered_at: last_triggered_ms.map(crate::format::iso8601),
    }
}

fn last_triggered(host: &AlertHost, id: AlertId) -> Option<i64> {
    host.snapshot()
        .history
        .iter()
        .filter(|event| event.alert_id == id)
        .map(|event| event.timestamp_ms)
        .max()
}

fn info_for(host: &AlertHost, alert: &PriceAlert) -> AlertInfo {
    alert_info(alert, last_triggered(host, alert.id))
}

/// Alerts as the runtime holds them: the page, or just the one asked for.
pub fn list_alerts(host: &AlertHost, only: Option<AlertId>) -> AlertsResult {
    let alerts = &host.snapshot().alerts;
    if let Some(id) = only {
        let found: Vec<_> = alerts
            .iter()
            .filter(|alert| alert.id == id)
            .map(|alert| info_for(host, alert))
            .collect();
        return AlertsResult {
            total: found.len(),
            truncated: false,
            alerts: found,
        };
    }
    AlertsResult {
        total: alerts.len(),
        truncated: alerts.len() > ALERT_PAGE,
        alerts: alerts
            .iter()
            .take(ALERT_PAGE)
            .map(|alert| info_for(host, alert))
            .collect(),
    }
}

pub enum CreatePlan {
    /// An identical alert exists already; hand it back instead of adding a duplicate.
    Existing(Box<AlertInfo>),
    New {
        alert: Box<PriceAlert>,
        /// The market now, in the alert's currency, when a live price is known.
        baseline: Option<f64>,
    },
}

pub(super) fn require_loaded(host: &AlertHost) -> Result<(), McpError> {
    match host.load_error() {
        None => Ok(()),
        Some(_) => Err(McpError::AppUnavailable(
            "Alert data could not be loaded, so alerts cannot be changed".into(),
        )),
    }
}

/// Builds the alert exactly as the price-alert sheet does (its own parsing and rules), without
/// saving it. The caller commits a `New` plan through the same path the sheet's Save uses.
pub fn plan_create_alert(
    host: &AlertHost,
    book: &WatchlistQuoteBook,
    rates: &std::collections::HashMap<AlertCurrency, f64>,
    request: &NewAlert,
    now_ms: i64,
) -> Result<CreatePlan, McpError> {
    require_loaded(host)?;
    if host.snapshot().alerts.len() >= MAX_ALERTS {
        return Err(McpError::ResourceLimit(format!(
            "There are already {MAX_ALERTS} price alerts; delete some first"
        )));
    }
    let asset =
        ChartWorkspace::alert_asset_for(&request.instrument.symbol, request.instrument.source);
    let mut draft = PriceAlertDraft::new(asset, None, Some(request.currency));
    draft.direction = if request.crosses_above {
        Direction::Above
    } else {
        Direction::Below
    };
    draft.target = request.target_text.clone();
    draft.frequency = request.frequency;
    draft.note = request.note.clone();
    feed(&mut draft, book, rates);
    let alert = draft.result(now_ms).ok_or_else(|| {
        McpError::InvalidArgument("target_price must be a positive number".into())
    })?;
    if let Some(existing) = host.find_identical(&alert) {
        return Ok(CreatePlan::Existing(Box::new(info_for(host, existing))));
    }
    Ok(CreatePlan::New {
        baseline: draft.current_price(),
        alert: Box::new(alert),
    })
}

/// Prices the draft from the shared quote book, as the sheet does every frame.
fn feed(
    draft: &mut PriceAlertDraft,
    book: &WatchlistQuoteBook,
    rates: &std::collections::HashMap<AlertCurrency, f64>,
) {
    let quote = book.get_by_key(&draft.asset().key());
    draft.feed_quote(quote.as_ref(), rates);
}

pub enum UpdatePlan {
    /// The request matches the alert as it is: nothing to queue.
    Unchanged(Box<AlertInfo>),
    Change {
        alert: Box<PriceAlert>,
        /// The alert as the edit was planned from: the runtime applies the edit only while the
        /// alert is still like this.
        base: Box<PriceAlert>,
        baseline: Option<f64>,
    },
}

/// Edits an alert as the Alerts window's Edit sheet does (its parsing, currency and arming rules),
/// without saving it. A remote edit never resumes a paused alert, and an alert that already fired
/// only becomes active again when its condition, currency or frequency changed. Webhook settings
/// are carried over untouched.
pub fn plan_update_alert(
    host: &AlertHost,
    book: &WatchlistQuoteBook,
    rates: &std::collections::HashMap<AlertCurrency, f64>,
    request: &AlertUpdate,
    now_ms: i64,
) -> Result<UpdatePlan, McpError> {
    require_loaded(host)?;
    let existing = find_alert(host, request.alert_id)?.clone();
    let percentage = matches!(
        existing.condition,
        AlertCondition::RisesBy { .. } | AlertCondition::FallsBy { .. }
    );
    if existing.condition.target().is_none() {
        return Err(McpError::Unsupported(
            "This alert uses a condition this version of DegenView does not support".into(),
        ));
    }
    if percentage
        && (request.crosses_above.is_some()
            || request.target_text.is_some()
            || request.currency.is_some())
    {
        return Err(McpError::Unsupported(
            "A percentage alert can only have its frequency and note changed".into(),
        ));
    }
    let mut draft = PriceAlertDraft::new(
        existing.asset.clone(),
        Some(existing.clone()),
        Some(existing.currency),
    );
    if let Some(currency) = request.currency {
        draft.set_currency(currency);
    }
    if let Some(above) = request.crosses_above {
        draft.direction = if above {
            Direction::Above
        } else {
            Direction::Below
        };
    }
    if let Some(target) = &request.target_text {
        draft.target = target.clone();
    }
    if let Some(frequency) = request.frequency {
        draft.frequency = frequency;
    }
    if let Some(note) = &request.note {
        draft.note = note.clone();
    }
    feed(&mut draft, book, rates);
    let mut alert = draft.result(now_ms).ok_or_else(|| {
        McpError::InvalidArgument("target_price must be a positive number".into())
    })?;
    let settings_changed = alert.condition != existing.condition
        || alert.currency != existing.currency
        || alert.frequency != existing.frequency;
    alert.state = match existing.state {
        AlertState::Paused => AlertState::Paused,
        AlertState::Triggered if !settings_changed => AlertState::Triggered,
        _ => AlertState::Active,
    };
    if !settings_changed && alert.note == existing.note && alert.state == existing.state {
        return Ok(UpdatePlan::Unchanged(Box::new(info_for(host, &existing))));
    }
    if let Some(other) = host.find_identical(&alert) {
        return Err(McpError::Conflict(format!(
            "Price alert {} already has exactly these settings",
            other.id
        )));
    }
    Ok(UpdatePlan::Change {
        baseline: draft.current_price(),
        alert: Box::new(alert),
        base: Box::new(existing),
    })
}

pub enum StatePlan {
    /// Already in the requested state: nothing to queue.
    Already,
    Change {
        id: AlertId,
        state: AlertState,
    },
}

pub(super) fn find_alert(host: &AlertHost, id: AlertId) -> Result<&PriceAlert, McpError> {
    host.snapshot()
        .alerts
        .iter()
        .find(|alert| alert.id == id)
        .ok_or_else(|| McpError::NotFound(format!("No price alert {id}")))
}

/// Pause or resume, deciding as the Alerts window's toggle does: pausing an active alert, and
/// resuming anything else (a fired once-alert is re-armed).
pub fn plan_pause_alert(
    host: &AlertHost,
    id: AlertId,
    paused: bool,
) -> Result<StatePlan, McpError> {
    require_loaded(host)?;
    let alert = find_alert(host, id)?;
    if alert.state == AlertState::Unsupported {
        return Err(McpError::Unsupported(
            "This alert uses a condition this version of DegenView does not support".into(),
        ));
    }
    let already = if paused {
        alert.state == AlertState::Paused
    } else {
        alert.state == AlertState::Active
    };
    if already {
        return Ok(StatePlan::Already);
    }
    Ok(StatePlan::Change {
        id,
        state: if paused {
            AlertState::Paused
        } else {
            AlertState::Active
        },
    })
}

/// What became of the alert command `id`: applied, refused because the alerts changed under it,
/// or not applied yet.
pub fn alert_command_outcome(host: &AlertHost, id: u64) -> AlertCommandOutcome {
    if let Some(reason) = host.rejection(id) {
        AlertCommandOutcome::Rejected(reason)
    } else if host.snapshot().processed_command_ids.contains(&id) {
        AlertCommandOutcome::Applied
    } else {
        AlertCommandOutcome::Pending
    }
}

pub fn plan_delete_alert(host: &AlertHost, id: AlertId) -> Result<AlertId, McpError> {
    require_loaded(host)?;
    find_alert(host, id).map(|alert| alert.id)
}
