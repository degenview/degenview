//! The MCP tool surface: the chart tools and the handler that guards them.
//!
//! Tools are thin adapters. They validate their arguments, then ask the app to do the work
//! through the [`bridge`](crate::mcp::bridge); no workspace or SQLite state is touched here.
//! Authorization is enforced in `call_tool` on every call — `tools/list` merely hides what the
//! caller could not use anyway.

use crate::mcp::bridge::{
    AlertUpdate, BridgeClient, McpCommand, McpError, McpReply, NewAlert, REQUEST_TIMEOUT,
};
use crate::mcp::pine::{self, Target, Version};
use crate::mcp::state::{ClientIdentity, McpShared};
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, AlertChangeResult, AlertInfo, AlertsResult,
    AppliedPineDetail, AppliedPineList, ChartsResult, CreateAlertResult, IndicatorsResult,
    PineAlertsResult, PineCompileResult, PineEditResult, PineInputsResult, PineReportResult,
    PineScriptSource, PineScriptsResult, PineStylesResult, UpdateAlertResult, WorkspacesResult,
    indicator_catalog,
};
use crate::model::alert::{AlertCurrency, AlertFrequency};
use crate::model::mcp::Permission;
use crate::model::watchlist_text::validate_market;
use crate::model::{BuiltIn, DataSource, EMA_PERIODS};
use crate::script_manager::store::ScriptStore;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, JsonObject,
    ListToolsResult, PaginatedRequestParams, ResultType, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const LIST_WORKSPACES: &str = "degenview_list_workspaces";
pub const LIST_CHARTS: &str = "degenview_list_charts";
pub const ADD_CHART: &str = "degenview_add_chart";
pub const LIST_INDICATORS: &str = "degenview_list_indicators";
pub const ADD_INDICATOR: &str = "degenview_add_indicator";
pub const LIST_ALERTS: &str = "degenview_list_price_alerts";
pub const CREATE_ALERT: &str = "degenview_create_price_alert";
pub const UPDATE_ALERT: &str = "degenview_update_price_alert";
pub const LIST_PINE_SCRIPTS: &str = "degenview_list_pine_scripts";
pub const GET_PINE_SCRIPT: &str = "degenview_get_pine_script";
pub const COMPILE_PINE_SCRIPT: &str = "degenview_compile_pine_script";
pub const GET_PINE_INPUTS: &str = "degenview_get_pine_inputs";
pub const UPDATE_PINE_SCRIPT: &str = "degenview_update_pine_script";
pub const SAVE_PINE_SCRIPT: &str = "degenview_save_pine_script";
pub const LIST_CHART_PINE: &str = "degenview_list_chart_pine_instances";
pub const GET_PINE_INSTANCE: &str = "degenview_get_pine_instance";
pub const GET_PINE_STYLES: &str = "degenview_get_pine_styles";
pub const GET_PINE_REPORT: &str = "degenview_get_pine_report";
pub const LIST_PINE_ALERTS: &str = "degenview_list_pine_alerts";
pub const PAUSE_ALERT: &str = "degenview_pause_price_alert";
pub const RESUME_ALERT: &str = "degenview_resume_price_alert";
pub const DELETE_ALERT: &str = "degenview_delete_price_alert";

/// The permission a tool needs, or `None` for a name this server does not have.
pub fn required_permission(tool: &str) -> Option<Permission> {
    match tool {
        LIST_WORKSPACES | LIST_CHARTS | LIST_INDICATORS => Some(Permission::ChartsRead),
        ADD_CHART => Some(Permission::ChartsCreate),
        ADD_INDICATOR => Some(Permission::ChartsIndicatorsWrite),
        LIST_ALERTS => Some(Permission::AlertsRead),
        CREATE_ALERT => Some(Permission::AlertsCreate),
        UPDATE_ALERT => Some(Permission::AlertsEdit),
        PAUSE_ALERT | RESUME_ALERT => Some(Permission::AlertsPause),
        DELETE_ALERT => Some(Permission::AlertsDelete),
        LIST_PINE_SCRIPTS => Some(Permission::PineScriptsList),
        GET_PINE_SCRIPT => Some(Permission::PineScriptsRead),
        UPDATE_PINE_SCRIPT => Some(Permission::PineScriptsWrite),
        SAVE_PINE_SCRIPT => Some(Permission::PineScriptsSave),
        COMPILE_PINE_SCRIPT => Some(Permission::PineDiagnosticsRead),
        GET_PINE_INPUTS | GET_PINE_INSTANCE => Some(Permission::PineInputsRead),
        LIST_CHART_PINE => Some(Permission::PineScriptsList),
        GET_PINE_STYLES => Some(Permission::PineStylesRead),
        GET_PINE_REPORT => Some(Permission::PineReportsRead),
        LIST_PINE_ALERTS => Some(Permission::PineAlertsRead),
        _ => None,
    }
}

/// Providers a chart can be added from by symbol alone. Prediction markets (they need an event)
/// and CoinMarketCap (indices, not markets) are not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum McpSource {
    Binance,
    Coinbase,
    Coingecko,
    Dexscreener,
    Alpaca,
}

impl McpSource {
    fn data_source(self) -> DataSource {
        match self {
            Self::Binance => DataSource::Binance,
            Self::Coinbase => DataSource::Coinbase,
            Self::Coingecko => DataSource::CoinGecko,
            Self::Dexscreener => DataSource::DexScreener,
            Self::Alpaca => DataSource::Alpaca,
        }
    }
}

/// The built-in indicators, by the ids `degenview_list_indicators` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpIndicator {
    Volume,
    Rsi,
    Ema,
    Bollinger,
    TrendFlips,
}

impl McpIndicator {
    fn built_in(self) -> BuiltIn {
        match self {
            Self::Volume => BuiltIn::Volume,
            Self::Rsi => BuiltIn::Rsi,
            Self::Ema => BuiltIn::Ema,
            Self::Bollinger => BuiltIn::Bollinger,
            Self::TrendFlips => BuiltIn::TrendFlips,
        }
    }
}

/// Settings for the indicator. Only EMA has any.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndicatorInputs {
    /// EMA period: one of 9, 20, 50, 100, 200.
    #[schemars(range(min = 1, max = 1000))]
    pub period: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddIndicatorParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts` in that workspace.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    pub indicator: McpIndicator,
    /// Omit for indicators with fixed settings.
    pub inputs: Option<IndicatorInputs>,
}

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
    raw.trim()
        .parse::<u64>()
        .map_err(|_| invalid("alert_id must be an id returned by degenview_list_price_alerts"))
}

const SETTLE_POLLS: usize = 20;
const SETTLE_INTERVAL: Duration = Duration::from_millis(100);

/// Which text of a script: the last saved source, or the unsaved draft kept beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpScriptVersion {
    Saved,
    Draft,
}

impl McpScriptVersion {
    fn version(self) -> Version {
        match self {
            Self::Saved => Version::Saved,
            Self::Draft => Version::Draft,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPineScriptParams {
    /// A `script_id` from `degenview_list_pine_scripts`.
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// `saved` or `draft`. Asking for a draft that does not exist is an error, never the saved text.
    pub version: McpScriptVersion,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompilePineParams {
    /// A script to compile; give this or `source`, not both.
    #[schemars(length(min = 1, max = 64))]
    pub script_id: Option<String>,
    /// With `script_id`: `saved` (default) or `draft`.
    pub version: Option<McpScriptVersion>,
    /// Source to check without saving it anywhere; give this or `script_id`.
    #[schemars(length(max = 200000))]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePineParams {
    /// A `script_id` from `degenview_list_pine_scripts`.
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// The complete new source text.
    #[schemars(length(max = 200000))]
    pub source: String,
    /// The `source_hash` of the text you read and based this on. If the script is not at that
    /// revision any more (the user typed, or another client edited) the edit is refused with a
    /// CONFLICT and nothing is overwritten.
    #[schemars(length(min = 64, max = 64))]
    pub expected_revision: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavePineParams {
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// The `source_hash` of the draft you reviewed (the revision returned by the last edit).
    #[schemars(length(min = 64, max = 64))]
    pub expected_revision: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PineInputsParams {
    #[schemars(length(min = 1, max = 64))]
    pub script_id: String,
    /// `saved` (default) or `draft`.
    pub version: Option<McpScriptVersion>,
}

/// Source accepted inline for a compile (the request body is capped near this anyway).
const MAX_INLINE_SOURCE_CHARS: usize = 200_000;

/// Script ids are file-store ids; refuse anything that is not plain id characters.
fn clean_script_id(raw: &str) -> Result<String, CallToolResult> {
    let id = raw.trim();
    let plain = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if plain {
        Ok(id.to_owned())
    } else {
        Err(invalid(
            "script_id must be an id returned by degenview_list_pine_scripts",
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartPineParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts`.
    #[schemars(length(max = 20))]
    pub chart_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PineInstanceParams {
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// An `instance_id` from `degenview_list_chart_pine_instances`. The same script can be applied
    /// to a chart more than once; each application is its own instance.
    #[schemars(length(max = 20))]
    pub instance_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PineReportParams {
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    #[schemars(length(max = 20))]
    pub chart_id: String,
    #[schemars(length(max = 20))]
    pub instance_id: String,
    /// Closed trades to skip (oldest first). Default 0.
    #[schemars(range(min = 0, max = 1000000))]
    pub trades_offset: Option<u32>,
    /// Closed trades to return, 1 to 200. Default 50.
    #[schemars(range(min = 1, max = 200))]
    pub trades_limit: Option<u32>,
    /// Include the equity curve, thinned to at most 200 points. Default false.
    pub include_equity: Option<bool>,
}

fn parse_chart_id(raw: &str) -> Result<u64, CallToolResult> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| invalid("chart_id must be an id returned by degenview_list_charts"))
}

fn parse_instance_id(raw: &str) -> Result<u64, CallToolResult> {
    raw.trim().parse::<u64>().map_err(|_| {
        invalid("instance_id must be an id returned by degenview_list_chart_pine_instances")
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListChartsParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddChartParams {
    /// A `workspace_id` from `degenview_list_workspaces`. Must be a chart workspace.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// The market data provider.
    pub source: McpSource,
    /// The provider's own symbol: `BTCUSDT` (binance), `BTC-USD` (coinbase), `bitcoin`
    /// (coingecko), `chain/address` (dexscreener), `AAPL` (alpaca).
    #[schemars(length(min = 1, max = 128))]
    pub symbol: String,
}

fn failure(error: &McpError) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "error": error.code(),
        "message": error.message(),
    }))
}

fn invalid(message: impl Into<String>) -> CallToolResult {
    failure(&McpError::InvalidArgument(message.into()))
}

fn parse_id(raw: &str, what: &str) -> Result<u64, CallToolResult> {
    raw.trim().parse::<u64>().map_err(|_| {
        invalid(format!(
            "{what} must be an id returned by degenview_list_workspaces"
        ))
    })
}

/// The client the HTTP guard authenticated for this request.
fn client_of(parts: &http::request::Parts) -> Result<ClientIdentity, CallToolResult> {
    parts
        .extensions
        .get::<ClientIdentity>()
        .cloned()
        .ok_or_else(|| failure(&McpError::PermissionDenied("Not authenticated".into())))
}

fn unexpected() -> CallToolResult {
    failure(&McpError::AppUnavailable(
        "Unexpected response from DegenView".into(),
    ))
}

#[derive(Clone)]
pub struct DegenViewMcp {
    bridge: BridgeClient,
    shared: Arc<McpShared>,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl DegenViewMcp {
    pub fn new(bridge: BridgeClient, shared: Arc<McpShared>) -> Self {
        Self {
            bridge,
            shared,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "degenview_list_workspaces",
        description = "List DegenView chart workspaces (one per tab). Returns each workspace's stable id, title, chart count and timeframe, and which one is active. Use a workspace_id with the other chart tools.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_workspaces(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<WorkspacesResult>, CallToolResult> {
        let client = client_of(&parts)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsRead,
                McpCommand::ListWorkspaces,
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Workspaces(workspaces)) => Ok(Json(WorkspacesResult { workspaces })),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_list_charts",
        description = "List the charts in one workspace: market, provider, timeframe, chart type, applied indicators and Pine scripts, and load state. Candle data is not returned.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_charts(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<ListChartsParams>,
    ) -> Result<Json<ChartsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsRead,
                McpCommand::ListCharts { workspace_id },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Charts(charts)) => Ok(Json(charts)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_list_indicators",
        description = "List the built-in chart indicators DegenView supports: id, where it draws, which chart types it works on, and its parameters with defaults and allowed values.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_indicators(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<IndicatorsResult>, CallToolResult> {
        client_of(&parts)?;
        Ok(Json(IndicatorsResult {
            indicators: indicator_catalog(),
        }))
    }

    #[tool(
        name = "degenview_add_indicator",
        description = "Apply a built-in indicator to a chart, or update it if already applied (re-shows it; an EMA takes a period). Each indicator exists once per chart. Returns the chart's indicators afterwards.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn add_indicator(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AddIndicatorParams>,
    ) -> Result<Json<AddIndicatorResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = params
            .chart_id
            .trim()
            .parse::<u64>()
            .map_err(|_| invalid("chart_id must be an id returned by degenview_list_charts"))?;
        let ema_period = match params.inputs.and_then(|inputs| inputs.period) {
            None => None,
            Some(period) if params.indicator != McpIndicator::Ema => {
                return Err(invalid(format!(
                    "{period} given, but only ema takes a period"
                )));
            }
            Some(period) if EMA_PERIODS.contains(&(period as usize)) => Some(period as usize),
            Some(period) => {
                return Err(invalid(format!(
                    "EMA period {period} is not supported; use one of {EMA_PERIODS:?}"
                )));
            }
        };
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsIndicatorsWrite,
                McpCommand::ApplyIndicator {
                    workspace_id,
                    chart_id,
                    indicator: params.indicator.built_in(),
                    ema_period,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::IndicatorApplied(result)) => Ok(Json(result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

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
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<AlertsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::AlertsRead,
                McpCommand::ListAlerts { only: None },
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<CreateAlertParams>,
    ) -> Result<Json<CreateAlertResult>, CallToolResult> {
        let client = client_of(&parts)?;
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
                &client.id,
                Permission::AlertsCreate,
                McpCommand::CreateAlert(Box::new(request)),
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
                .settle(&client, Permission::AlertsCreate, id, |alerts| {
                    !alerts.is_empty()
                })
                .await
                .and_then(|alerts| alerts.into_iter().next())
            {
                created = CreateAlertResult {
                    status: "created".into(),
                    alert: stored,
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<UpdateAlertParams>,
    ) -> Result<Json<UpdateAlertResult>, CallToolResult> {
        let client = client_of(&parts)?;
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
                &client.id,
                Permission::AlertsEdit,
                McpCommand::UpdateAlert(Box::new(request)),
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
                .settle(&client, Permission::AlertsEdit, alert_id, |alerts| {
                    alerts
                        .first()
                        .is_some_and(|alert| same_settings(alert, &expected))
                })
                .await
                .and_then(|alerts| alerts.into_iter().next())
            {
                updated = UpdateAlertResult {
                    status: "updated".into(),
                    alert: stored,
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        self.set_alert_paused(&parts, &params.alert_id, true).await
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        self.set_alert_paused(&parts, &params.alert_id, false).await
    }

    #[tool(
        name = "degenview_delete_price_alert",
        description = "Permanently delete a price alert. This cannot be undone.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_price_alert(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let alert_id = parse_alert_id(&params.alert_id)?;
        let mut changed = match self
            .bridge
            .call(
                &client.id,
                Permission::AlertsDelete,
                McpCommand::DeleteAlert { alert_id },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertChanged(changed)) => changed,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if self
            .settle(&client, Permission::AlertsDelete, alert_id, |alerts| {
                alerts.is_empty()
            })
            .await
            .is_some()
        {
            changed.status = "deleted".into();
        }
        Ok(Json(changed))
    }

    #[tool(
        name = "degenview_list_pine_scripts",
        description = "List the Pine scripts in DegenView's Script Manager: id, name, type (indicator, strategy, library), whether an unsaved draft exists, and the last compile status. No file paths are returned.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_pine_scripts(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<PineScriptsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        self.pine_job(&client, Permission::PineScriptsList, |store| {
            pine::list_scripts(store)
        })
        .await
        .map(Json)
    }

    #[tool(
        name = "degenview_get_pine_script",
        description = "Read a Pine script's source: the saved text or the unsaved draft (never one in place of the other), with its SHA-256 source_hash and revision id.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_script(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<GetPineScriptParams>,
    ) -> Result<Json<PineScriptSource>, CallToolResult> {
        let client = client_of(&parts)?;
        let id = clean_script_id(&params.script_id)?;
        let version = params.version.version();
        self.pine_job(&client, Permission::PineScriptsRead, move |store| {
            pine::get_script(store, &id, version)
        })
        .await
        .map(Json)
    }

    #[tool(
        name = "degenview_update_pine_script",
        description = "Replace a Pine script's DRAFT with new source text. Only the draft changes: the saved source, charts using the script and alert subscriptions are untouched, and nothing is run. The edit is checked against what the user sees now (their open editor, else the draft, else the saved source) using expected_revision; if that moved, it is refused with CONFLICT. The open Script Manager editor shows the new text. Returns the new revision and the draft's compile diagnostics. Use degenview_save_pine_script to make it the saved source.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_pine_script(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<UpdatePineParams>,
    ) -> Result<Json<PineEditResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let id = clean_script_id(&params.script_id)?;
        if !pine::is_revision(&params.expected_revision) {
            return Err(invalid(
                "expected_revision must be the 64-character source_hash from degenview_get_pine_script",
            ));
        }
        if params.source.chars().count() > pine::MAX_EDIT_CHARS {
            return Err(failure(&McpError::ResourceLimit(format!(
                "source is over {} characters",
                pine::MAX_EDIT_CHARS
            ))));
        }
        let source = params.source.clone();
        let mut result = match self
            .bridge
            .call(
                &client.id,
                Permission::PineScriptsWrite,
                McpCommand::EditPineScript {
                    script_id: id.clone(),
                    expected_revision: params.expected_revision,
                    source: Some(params.source),
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineEdit(result)) => *result,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        // Diagnostics for the text that is now the draft: compiled off the app thread.
        let script_id = id.clone();
        result.compile = self
            .pine_job(&client, Permission::PineScriptsWrite, move |store| {
                let mut compiled = pine::compile_script(store, Target::Inline(source))?;
                compiled.script_id = Some(script_id);
                compiled.version = "draft".into();
                Ok(compiled)
            })
            .await
            .ok();
        Ok(Json(result))
    }

    #[tool(
        name = "degenview_save_pine_script",
        description = "Save a Pine script's current draft over its saved source, as the editor's Save does (records a revision, keeps history). Needs expected_revision to match the draft you reviewed, and the draft must compile without errors (COMPILATION_FAILED otherwise; the draft is kept). Charts keep running the source they were applied with; alert subscriptions to this script will show 'script changed'. Saving cannot be undone from here (revision history stays in the Script Manager).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn save_pine_script(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<SavePineParams>,
    ) -> Result<Json<PineEditResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let id = clean_script_id(&params.script_id)?;
        if !pine::is_revision(&params.expected_revision) {
            return Err(invalid(
                "expected_revision must be the 64-character source_hash from degenview_get_pine_script",
            ));
        }
        let mut result = match self
            .bridge
            .call(
                &client.id,
                Permission::PineScriptsSave,
                McpCommand::EditPineScript {
                    script_id: id.clone(),
                    expected_revision: params.expected_revision,
                    source: None,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineEdit(result)) => *result,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        result.compile = self
            .pine_job(&client, Permission::PineScriptsSave, move |store| {
                pine::compile_script(store, Target::Script(id, Version::Saved))
            })
            .await
            .ok();
        Ok(Json(result))
    }

    #[tool(
        name = "degenview_compile_pine_script",
        description = "Compile a Pine script (saved or draft) or a source string with DegenView's own compiler and return its errors and warnings with codes and line/column positions. Compiling does not run the script: no trades, alerts, webhooks, saves or chart changes. A name nothing defines is reported as an error (PINE3050 variable, PINE3051 function). The check is lenient (a name declared anywhere counts, and members of builtin namespaces such as ta.* are judged only at run time), so a clean compile does not prove the script runs; apply it to a chart (degenview_add_indicator) and read its last-run error with degenview_get_pine_instance.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn compile_pine_script(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<CompilePineParams>,
    ) -> Result<Json<PineCompileResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let target = match (params.script_id, params.source) {
            (Some(id), None) => Target::Script(
                clean_script_id(&id)?,
                params
                    .version
                    .map_or(Version::Saved, McpScriptVersion::version),
            ),
            (None, Some(source)) if params.version.is_none() => {
                if source.chars().count() > MAX_INLINE_SOURCE_CHARS {
                    return Err(failure(&McpError::ResourceLimit(format!(
                        "source is over {MAX_INLINE_SOURCE_CHARS} characters"
                    ))));
                }
                Target::Inline(source)
            }
            (None, Some(_)) => {
                return Err(invalid("version applies to a script_id, not to source"));
            }
            (Some(_), Some(_)) => return Err(invalid("give script_id or source, not both")),
            (None, None) => return Err(invalid("give script_id or source")),
        };
        self.pine_job(&client, Permission::PineDiagnosticsRead, move |store| {
            pine::compile_script(store, target)
        })
        .await
        .map(Json)
    }

    #[tool(
        name = "degenview_get_pine_inputs",
        description = "List a Pine script's input definitions (id, title, type, default, min/max/step, options, group, tooltip) as the compiler declares them.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_inputs(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<PineInputsParams>,
    ) -> Result<Json<PineInputsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let id = clean_script_id(&params.script_id)?;
        let version = params
            .version
            .map_or(Version::Saved, McpScriptVersion::version);
        self.pine_job(&client, Permission::PineInputsRead, move |store| {
            pine::script_inputs(store, &id, version)
        })
        .await
        .map(Json)
    }

    #[tool(
        name = "degenview_list_chart_pine_instances",
        description = "List the Pine scripts applied to one chart: instance id, the script it came from, visibility, the source hash it runs, and whether its last run was ok, failed, or has not happened yet (scripts run only while their chart is drawn). The same script can appear more than once.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_chart_pine_instances(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<ChartPineParams>,
    ) -> Result<Json<AppliedPineList>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::PineScriptsList,
                McpCommand::ListChartPine {
                    workspace_id,
                    chart_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::ChartPine(list)) => Ok(Json(list)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_get_pine_instance",
        description = "Inspect one applied Pine script: its input overrides, style overrides, visibility, last-run status and error, and whether its source still matches the saved script it came from. Source text is not included (see degenview_get_pine_script).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_instance(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<PineInstanceParams>,
    ) -> Result<Json<AppliedPineDetail>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let instance_id = parse_instance_id(&params.instance_id)?;
        let mut detail = match self
            .bridge
            .call(
                &client.id,
                Permission::PineInputsRead,
                McpCommand::GetChartPine {
                    workspace_id,
                    chart_id,
                    instance_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::ChartPineDetail(detail)) => detail,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        detail.script_status = self
            .script_status(
                &client,
                Permission::PineInputsRead,
                detail.instance.script_id.clone(),
                detail.instance.source_hash.clone(),
            )
            .await;
        Ok(Json(detail))
    }

    #[tool(
        name = "degenview_get_pine_styles",
        description = "List the style options of an applied Pine script (each plot, line, fill, marker...) with the script's own default color and width and any overrides set on this instance. Options exist only once the script has run on its chart.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_styles(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<PineInstanceParams>,
    ) -> Result<Json<PineStylesResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let instance_id = parse_instance_id(&params.instance_id)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::PineStylesRead,
                McpCommand::GetPineStyles {
                    workspace_id,
                    chart_id,
                    instance_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineStyles(styles)) => Ok(Json(styles)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_get_pine_report",
        description = "Read the strategy backtest report a chart computed for an applied Pine strategy: metrics, settings, a page of closed trades, open trades and optionally the equity curve, with the instrument, timeframe, bar count, inputs and source hash it was produced with. If there is no report (not a strategy, not run yet, failed) the result says so; an old report is never passed off as current. Nothing is run by this call.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_pine_report(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<PineReportParams>,
    ) -> Result<Json<PineReportResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let instance_id = parse_instance_id(&params.instance_id)?;
        let limit = params.trades_limit.unwrap_or(50);
        let offset = params.trades_offset.unwrap_or(0);
        if !(1..=200).contains(&limit) {
            return Err(invalid("trades_limit must be between 1 and 200"));
        }
        if offset > 1_000_000 {
            return Err(invalid("trades_offset must be at most 1000000"));
        }
        let mut report = match self
            .bridge
            .call(
                &client.id,
                Permission::PineReportsRead,
                McpCommand::GetPineReport {
                    workspace_id,
                    chart_id,
                    instance_id,
                    offset: offset as usize,
                    limit: limit as usize,
                    include_equity: params.include_equity.unwrap_or(false),
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineReport(report)) => *report,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if let Some(context) = report.context.as_mut() {
            context.script_status = self
                .script_status(
                    &client,
                    Permission::PineReportsRead,
                    context.script_id.clone(),
                    context.source_hash.clone(),
                )
                .await;
        }
        Ok(Json(report))
    }

    #[tool(
        name = "degenview_list_pine_alerts",
        description = "List the alert subscriptions of Pine scripts: script, market, timeframe, state (active, paused, script_changed, script_deleted) and note. DegenView keeps no log of fired script alerts, so no history is returned (event_history_available is false); reading never runs a script or sends anything.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_pine_alerts(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<PineAlertsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::PineAlertsRead,
                McpCommand::ListPineAlerts,
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::PineAlerts(alerts)) => Ok(Json(alerts)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_add_chart",
        description = "Add a chart for a market to an existing chart workspace. The chart uses the workspace's timeframe and is placed by the normal layout rules. If the workspace already charts that market, that chart is returned instead of adding a duplicate.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn add_chart(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AddChartParams>,
    ) -> Result<Json<AddChartResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let source = params.source.data_source();
        let instrument = validate_market(source, &params.symbol).map_err(invalid)?;
        if source == DataSource::Alpaca && !alpaca_configured().await {
            return Err(failure(&McpError::Unsupported(
                "Alpaca needs API credentials: add them in DegenView Settings > API Keys".into(),
            )));
        }
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsCreate,
                McpCommand::AddChart {
                    workspace_id,
                    instrument,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::ChartAdded(added)) => Ok(Json(added)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}

impl DegenViewMcp {
    /// Runs a script-library read or compile off the async workers and off the app thread, at
    /// most `PINE_JOBS` at a time. The permission is checked again once the work is done: a
    /// credential revoked meanwhile gets nothing.
    async fn pine_job<T: Send + 'static>(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        job: impl FnOnce(&mut ScriptStore) -> Result<T, McpError> + Send + 'static,
    ) -> Result<T, CallToolResult> {
        let Some(permit) = self.shared.try_pine_job() else {
            return Err(failure(&McpError::ResourceLimit(
                "Too many script requests are running; try again shortly".into(),
            )));
        };
        let shared = self.shared.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut store = shared.script_store().ok_or_else(|| {
                McpError::AppUnavailable("The script library is unavailable".into())
            })?;
            job(&mut store)
        })
        .await
        .map_err(|_| {
            failure(&McpError::AppUnavailable(
                "The request was interrupted".into(),
            ))
        })?;
        if !self.shared.allows(&client.id, permission) {
            return Err(failure(&McpError::PermissionDenied(
                "This credential no longer has the required permission".into(),
            )));
        }
        outcome.map_err(|error| failure(&error))
    }

    /// Whether an applied script's source still matches the saved script it came from. The
    /// answer is a courtesy: if the library cannot be read it is `unknown`, not an error.
    async fn script_status(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        script_id: Option<String>,
        applied_hash: String,
    ) -> String {
        self.pine_job(client, permission, move |store| {
            pine::script_status(store, script_id.as_deref(), &applied_hash)
        })
        .await
        .unwrap_or_else(|_| "unknown".to_owned())
    }

    /// The alert runtime applies queued changes on its own schedule. Looks for the change to
    /// show up (briefly), so a reply can say what is true now rather than only "queued".
    /// Returns the alert's entry (empty once deleted) as soon as `done` holds.
    async fn settle(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        alert_id: u64,
        done: impl Fn(&[AlertInfo]) -> bool,
    ) -> Option<Vec<AlertInfo>> {
        for _ in 0..SETTLE_POLLS {
            let reply = self
                .bridge
                .call(
                    &client.id,
                    permission,
                    McpCommand::ListAlerts {
                        only: Some(alert_id),
                    },
                    Duration::from_secs(2),
                )
                .await;
            match reply {
                Ok(McpReply::Alerts(result)) if done(&result.alerts) => return Some(result.alerts),
                Ok(_) => {}
                Err(_) => return None,
            }
            tokio::time::sleep(SETTLE_INTERVAL).await;
        }
        None
    }

    async fn set_alert_paused(
        &self,
        parts: &http::request::Parts,
        raw_id: &str,
        paused: bool,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        let client = client_of(parts)?;
        let alert_id = parse_alert_id(raw_id)?;
        let mut changed = match self
            .bridge
            .call(
                &client.id,
                Permission::AlertsPause,
                McpCommand::SetAlertPaused { alert_id, paused },
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
            if self
                .settle(&client, Permission::AlertsPause, alert_id, |alerts| {
                    alerts.first().is_some_and(|alert| alert.status == wanted)
                })
                .await
                .is_some()
            {
                changed.status = wanted.into();
            }
        }
        Ok(Json(changed))
    }
}

/// The Keychain read stays off the async worker.
async fn alpaca_configured() -> bool {
    tokio::task::spawn_blocking(|| {
        crate::service::keychain_api_keys::AlpacaCredentialsStore::shared().is_configured()
    })
    .await
    .unwrap_or(false)
}

/// Drops the `uint`, `uint8` … `uint64` formats schemars puts on unsigned integers. They are not
/// JSON Schema formats, so clients warn about each one (Claude Code prints `unknown format "uint"
/// ignored in schema`); the `minimum: 0` schemars emits beside them already says the same thing.
fn with_standard_formats(mut tool: Tool) -> Tool {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if map
                    .get("format")
                    .and_then(Value::as_str)
                    .is_some_and(|format| format.starts_with("uint"))
                {
                    map.remove("format");
                }
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    fn clean(schema: &Arc<JsonObject>) -> Arc<JsonObject> {
        let mut value = Value::Object(schema.as_ref().clone());
        strip(&mut value);
        match value {
            Value::Object(map) => Arc::new(map),
            _ => unreachable!("an object stays an object"),
        }
    }
    tool.input_schema = clean(&tool.input_schema);
    tool.output_schema = tool.output_schema.as_ref().map(clean);
    tool
}

#[tool_handler(
    router = self.tool_router,
    name = "DegenView",
    instructions = "Inspect and add charts in DegenView, a market dashboard. List workspaces first, then list or add charts by workspace_id."
)]
impl ServerHandler for DegenViewMcp {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if let Some(permission) = required_permission(&request.name) {
            let allowed = context
                .extensions
                .get::<http::request::Parts>()
                .and_then(|parts| parts.extensions.get::<ClientIdentity>())
                .is_some_and(|client| self.shared.allows(&client.id, permission));
            if !allowed {
                return Ok(failure(&McpError::PermissionDenied(format!(
                    "This credential lacks the {} permission",
                    permission.id()
                )))
                .into());
            }
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let client = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<ClientIdentity>());
        let tools = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|tool| {
                client.is_some_and(|client| {
                    required_permission(&tool.name)
                        .is_some_and(|permission| self.shared.allows(&client.id, permission))
                })
            })
            .map(with_standard_formats)
            .collect();
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools,
            meta: None,
            next_cursor: None,
            // What a client may use depends on who it is and can change at any moment.
            ttl_ms: Some(0),
            cache_scope: Some(CacheScope::Private),
        })
    }
}
