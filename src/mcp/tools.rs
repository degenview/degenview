//! The MCP tool surface: the tools' names and permissions, the router every domain's tools are
//! put together in, and the handler that guards them. The tools themselves live with their
//! domain (`mcp::chart::tools`, `mcp::alert::tools`, `mcp::pine::tools`, `mcp::market::tools`…).
//!
//! Tools are thin adapters. They validate their arguments, then ask the app to do the work
//! through the [`bridge`](crate::mcp::bridge); no workspace or SQLite state is touched here.
//! Authorization is enforced in `call_tool` on every call — `tools/list` merely hides what the
//! caller could not use anyway.

use crate::mcp::audit::{AuditGuard, outcome_of};
use crate::mcp::bridge::{BridgeClient, McpError};
use crate::mcp::chart::control::{REMOVE_CHART, SET_CHART_TYPE, SET_TIMEFRAME};
use crate::mcp::indicator::tools::{GET_INDICATOR_VALUES, REMOVE_INDICATOR};
use crate::mcp::market::tools::{
    CREATE_WATCHLIST, DELETE_WATCHLIST, GET_MARKET_CANDLES, GET_QUOTES, GET_WATCHLIST,
    LIST_WATCHLISTS, SEARCH_SYMBOLS, UPDATE_WATCHLIST,
};
use crate::mcp::paper::tools::{
    CANCEL_PAPER_ORDERS, CLOSE_PAPER_POSITIONS, GET_PAPER_ACCOUNT, LIST_PAPER_ACCOUNTS,
    LIST_PAPER_FILLS, LIST_PAPER_TRADES, MODIFY_PAPER_ORDER, PLACE_PAPER_ORDER,
};
use crate::mcp::portfolio::tools::{
    GET_PORTFOLIO_HOLDINGS, LIST_PORTFOLIO_TRANSACTIONS, LIST_PORTFOLIOS,
};
use crate::mcp::replay::{CONTROL_REPLAY, GET_REPLAY};
use crate::mcp::session_stats::tools::GET_SESSION_STATS;
use crate::mcp::snapshot::GET_CHART_SNAPSHOT;
use crate::mcp::state::{ClientIdentity, McpShared};
use crate::model::mcp::{McpAuditOutcome, Permission};
use crate::model::{BuiltIn, DataSource};
use rmcp::handler::server::common::{AsRequestContext, FromContextPart};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, JsonObject,
    ListToolsResult, PaginatedRequestParams, ResultType, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, tool_handler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

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
pub const ADD_PINE_TO_CHART: &str = "degenview_add_pine_script_to_chart";
pub const UPDATE_PINE_INSTANCE: &str = "degenview_update_pine_instance";
pub const REMOVE_PINE_INSTANCE: &str = "degenview_remove_pine_instance";
pub const GET_CANDLES: &str = "degenview_get_candles";
pub const LIST_DRAWINGS: &str = "degenview_list_drawings";
pub const ADD_DRAWINGS: &str = "degenview_add_drawings";
pub const UPDATE_DRAWING: &str = "degenview_update_drawing";
pub const DELETE_DRAWINGS: &str = "degenview_delete_drawings";
pub const PAUSE_ALERT: &str = "degenview_pause_price_alert";
pub const RESUME_ALERT: &str = "degenview_resume_price_alert";
pub const DELETE_ALERT: &str = "degenview_delete_price_alert";

/// The permission a tool needs, or `None` for a name this server does not have.
pub fn required_permission(tool: &str) -> Option<Permission> {
    match tool {
        LIST_WORKSPACES | LIST_CHARTS | LIST_INDICATORS => Some(Permission::ChartsRead),
        ADD_CHART => Some(Permission::ChartsCreate),
        ADD_INDICATOR | REMOVE_INDICATOR => Some(Permission::ChartsIndicatorsWrite),
        ADD_PINE_TO_CHART | UPDATE_PINE_INSTANCE | REMOVE_PINE_INSTANCE => {
            Some(Permission::ChartsPineInstancesWrite)
        }
        GET_REPLAY => Some(Permission::ChartsRead),
        GET_CHART_SNAPSHOT => Some(Permission::ChartsSnapshotRead),
        CONTROL_REPLAY => Some(Permission::ChartsReplayWrite),
        SET_TIMEFRAME | SET_CHART_TYPE => Some(Permission::ChartsViewWrite),
        REMOVE_CHART => Some(Permission::ChartsDelete),
        GET_CANDLES | GET_INDICATOR_VALUES | GET_SESSION_STATS => {
            Some(Permission::ChartsCandlesRead)
        }
        LIST_DRAWINGS => Some(Permission::DrawingsRead),
        ADD_DRAWINGS | UPDATE_DRAWING => Some(Permission::DrawingsWrite),
        DELETE_DRAWINGS => Some(Permission::DrawingsDelete),
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
        LIST_PAPER_ACCOUNTS | GET_PAPER_ACCOUNT | LIST_PAPER_FILLS | LIST_PAPER_TRADES => {
            Some(Permission::PaperRead)
        }
        PLACE_PAPER_ORDER | MODIFY_PAPER_ORDER => Some(Permission::PaperOrdersPlace),
        CANCEL_PAPER_ORDERS => Some(Permission::PaperOrdersCancel),
        CLOSE_PAPER_POSITIONS => Some(Permission::PaperPositionsClose),
        LIST_PORTFOLIOS | GET_PORTFOLIO_HOLDINGS | LIST_PORTFOLIO_TRANSACTIONS => {
            Some(Permission::PortfolioRead)
        }
        SEARCH_SYMBOLS | GET_QUOTES | GET_MARKET_CANDLES => Some(Permission::MarketsRead),
        LIST_WATCHLISTS | GET_WATCHLIST => Some(Permission::WatchlistsRead),
        CREATE_WATCHLIST | UPDATE_WATCHLIST => Some(Permission::WatchlistsWrite),
        DELETE_WATCHLIST => Some(Permission::WatchlistsDelete),
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
    pub(super) fn data_source(self) -> DataSource {
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
    pub(super) fn built_in(self) -> BuiltIn {
        match self {
            Self::Volume => BuiltIn::Volume,
            Self::Rsi => BuiltIn::Rsi,
            Self::Ema => BuiltIn::Ema,
            Self::Bollinger => BuiltIn::Bollinger,
            Self::TrendFlips => BuiltIn::TrendFlips,
        }
    }
}

pub(super) fn parse_chart_id(raw: &str) -> Result<u64, CallToolResult> {
    parse_id(raw, "chart_id", "degenview_list_charts")
}

pub(super) fn failure(error: &McpError) -> CallToolResult {
    let mut value = json!({
        "error": error.code(),
        "message": error.message(),
    });
    if let (Some(fields), Some(object)) = (error.details(), value.as_object_mut())
        && let Value::Object(fields) = fields
    {
        object.extend(fields);
    }
    CallToolResult::structured_error(value)
}

pub(super) fn invalid(message: impl Into<String>) -> CallToolResult {
    failure(&McpError::InvalidArgument(message.into()))
}

/// A numeric id an earlier call listed. `what` names the argument and `listed_by` the tool whose
/// result holds it, so a wrong value says where a right one comes from.
pub(super) fn parse_id(raw: &str, what: &str, listed_by: &str) -> Result<u64, CallToolResult> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| invalid(format!("{what} must be an id returned by {listed_by}")))
}

pub(super) fn parse_workspace_id(raw: &str) -> Result<u64, CallToolResult> {
    parse_id(raw, "workspace_id", "degenview_list_workspaces")
}

/// The client the HTTP guard authenticated for this request, as a tool argument: a tool names
/// `Caller(client): Caller` beside its `Parameters` and rmcp hands it over. A request with no
/// authenticated client never reaches a tool (`call_tool` refuses it first), so a missing one is a
/// protocol error here, not a tool result.
pub(super) struct Caller(pub(super) ClientIdentity);

impl<C: AsRequestContext> FromContextPart<C> for Caller {
    fn from_context_part(context: &mut C) -> Result<Self, ErrorData> {
        identity(context.as_request_context())
            .cloned()
            .map(Self)
            .ok_or_else(|| ErrorData::invalid_params("Not authenticated", None))
    }
}

/// The client the HTTP guard (`transport::auth`) attached to the request it let in.
fn identity(context: &RequestContext<RoleServer>) -> Option<&ClientIdentity> {
    context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<ClientIdentity>())
}

pub(super) fn unexpected() -> CallToolResult {
    failure(&McpError::AppUnavailable(
        "Unexpected response from DegenView".into(),
    ))
}

#[derive(Clone)]
pub struct DegenViewMcp {
    pub(super) bridge: BridgeClient,
    pub(super) shared: Arc<McpShared>,
    tool_router: ToolRouter<Self>,
}

impl DegenViewMcp {
    /// Each domain routes its own tools; this is where they are put together.
    pub fn new(bridge: BridgeClient, shared: Arc<McpShared>) -> Self {
        Self {
            bridge,
            shared,
            tool_router: Self::chart_router()
                + Self::alert_router()
                + Self::pine_router()
                + Self::market_router()
                + Self::indicator_router()
                + Self::session_stats_router()
                + Self::chart_control_router()
                + Self::paper_router()
                + Self::portfolio_router()
                + Self::replay_router()
                + Self::snapshot_router(),
        }
    }

    /// Every tool this server routes, by name.
    #[cfg(test)]
    pub(crate) fn routed_tool_names(&self) -> Vec<String> {
        self.tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }
}

/// Why a tool call ended without a result of its own.
enum Stop {
    /// A name that is not a tool here (or has no permission mapping).
    UnknownTool(String),
    /// The client hung up or cancelled it.
    Cancelled,
    /// The router refused the request itself (not a tool's own error, which is a result).
    Rpc(ErrorData),
}

impl Stop {
    fn into_error(self) -> ErrorData {
        match self {
            Self::UnknownTool(name) => {
                ErrorData::invalid_params(format!("Unknown tool: {name}"), None)
            }
            Self::Cancelled => ErrorData::internal_error("The request was cancelled", None),
            Self::Rpc(error) => error,
        }
    }

    /// How the activity log records it.
    fn audit(&self) -> (McpAuditOutcome, &'static str) {
        match self {
            Self::UnknownTool(_) => (McpAuditOutcome::Denied, "UNKNOWN_TOOL"),
            Self::Cancelled => (McpAuditOutcome::Cancelled, ""),
            Self::Rpc(_) => (McpAuditOutcome::Failed, "RPC_ERROR"),
        }
    }
}

impl DegenViewMcp {
    /// Checks a call's permission and runs it, stopping it if the client leaves or cancels.
    async fn run_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
        client: Option<&ClientIdentity>,
    ) -> Result<CallToolResponse, Stop> {
        // Fail closed: a tool with no permission mapping is never run, however its name got
        // routed. (It is also hidden from `tools/list`.) A new tool must be added to
        // `required_permission`; `every_routed_tool_has_exactly_one_permission` enforces it.
        let Some(permission) = required_permission(&request.name) else {
            return Err(Stop::UnknownTool(request.name.to_string()));
        };
        let allowed = client.is_some_and(|client| self.shared.allows(&client.id, permission));
        if !allowed {
            return Ok(failure(&McpError::PermissionDenied(format!(
                "This credential lacks the {} permission",
                permission.id()
            )))
            .into());
        }
        // The call stops when the client hangs up (rmcp cancels `context.ct`, but leaves the
        // handler running) or sends `notifications/cancelled` for it. Stateless mode serves that
        // notification as a separate POST, so `honour_cancellations` reads it and cancels the
        // token this call registered under its client and request id.
        // Dropping the tool's future is what stops work that has not started: a queued bridge
        // request is cancelled by `CancelOnDrop`, a queued script job by `AbortOnDrop`. Work the
        // app has already started keeps its real outcome.
        let hung_up = context.ct.clone();
        let registered =
            client.map(|client| self.shared.calls().register(&client.id, context.id.clone()));
        let cancelled = registered
            .as_ref()
            .map_or_else(tokio_util::sync::CancellationToken::new, |call| {
                call.token()
            });
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        tokio::select! {
            result = self.tool_router.call(tcc) => result.map_err(Stop::Rpc),
            () = hung_up.cancelled() => Err(Stop::Cancelled),
            () = cancelled.cancelled() => Err(Stop::Cancelled),
        }
    }
}

/// [`api_keys::alpaca_configured`](crate::service::keychain::api_keys::alpaca_configured), asked
/// off the async worker: its first answer is a Keychain lookup.
pub(super) async fn alpaca_configured() -> bool {
    tokio::task::spawn_blocking(crate::service::keychain::api_keys::alpaca_configured)
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
    instructions = "Inspect and change DegenView, a market dashboard, within what this credential is allowed. Charts: list workspaces first, then list or add charts and built-in indicators by workspace_id; read candles and indicator values (RSI, EMA, Bollinger, trend flips), add or remove built-in indicators, change a workspace's timeframe and a chart's type, control a workspace's replay, take a picture of a chart, remove charts, and add, edit or delete drawings. Pine scripts: list, read, compile and edit scripts (an edit is a draft until saved, and every edit needs the revision you last read), and apply a saved script to a chart. Price alerts: list, create, edit, pause and delete. Markets: search the providers for symbols, read quotes, and read candles of any market. Watchlists: list and read them (optionally with quotes), create, edit and delete. Paper trading (simulated money): read accounts, positions, orders, fills and closed trades; place, change and cancel orders and close positions with their own permissions. Some changes wait for the user's approval in DegenView: deleting alerts, charts, drawings, watchlists or script instances, saving over a script, and paper orders, changes and closes. Such a call answers CONFIRMATION_REQUIRED (with a confirmation_id and what it would do) and changes nothing; ask the user to approve it, then repeat the exact same call. CONFIRMATION_DENIED means the user said no: do not repeat it unless they ask. Only the tools this credential may use are listed."
)]
impl ServerHandler for DegenViewMcp {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // Every call is recorded in the activity log, however it ends. The guard writes its row
        // when the call finishes below, or as cancelled if this future is dropped first (the
        // client hung up, or the server stopped).
        let client = identity(&context).cloned();
        let audit = client.as_ref().map(|client| {
            AuditGuard::begin(
                &self.shared,
                client,
                &request.name,
                &context.id,
                request.arguments.as_ref(),
            )
        });
        let result = self.run_tool(request, context, client.as_ref()).await;
        if let Some(audit) = audit {
            match &result {
                Ok(CallToolResponse::Complete(result)) => {
                    let (outcome, code) = outcome_of(result);
                    audit.finish(outcome, &code);
                }
                // This server never asks a client for more input; if that changes, so must this.
                Ok(_) => audit.finish(McpAuditOutcome::Failed, "INPUT_REQUIRED"),
                Err(stop) => {
                    let (outcome, code) = stop.audit();
                    audit.finish(outcome, code);
                }
            }
        }
        result.map_err(Stop::into_error)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let client = identity(&context);
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
