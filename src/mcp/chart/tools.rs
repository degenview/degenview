//! Chart tools: list workspaces and their charts, add a chart, list and apply built-in
//! indicators, read a chart's candles, and list, add, change and delete its drawings.

use crate::mcp::bridge::{McpCommand, McpError, McpReply, REQUEST_TIMEOUT};
use crate::mcp::chart::data::{DEFAULT_CANDLES, MAX_CANDLES};
use crate::mcp::chart::drawings::{
    AnchorArg, DrawingEdit, MAX_DELETES_PER_CALL, MAX_DRAWINGS_PER_CALL, McpDash, NewDrawingArg,
    StylePatch, TimeArg, parse_drawing_id, parse_time,
};
use crate::mcp::tools::{
    Caller, DegenViewMcp, McpIndicator, McpSource, alpaca_configured, failure, invalid,
    parse_chart_id, parse_workspace_id, unexpected,
};
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, CandlesResult, ChartsResult, DrawingsChangeResult,
    DrawingsResult, IndicatorsResult, WorkspacesResult, indicator_catalog,
};
use crate::model::mcp::Permission;
use crate::model::watchlist::text::validate_market;
use crate::model::{DataSource, EMA_PERIODS};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

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
pub struct GetCandlesParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts`.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// How many of the newest bars to return, 1 to 1000. Default 200.
    #[schemars(range(min = 1, max = 1000))]
    pub limit: Option<u32>,
    /// Return the bars that start before this time (ISO-8601 UTC or epoch milliseconds), to page
    /// back through what the chart holds. Use the `range.from` of the previous page.
    pub before: Option<TimeArg>,
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddDrawingsParams {
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// The drawings to add, up to 25. All are added as one undo step, or none are.
    #[schemars(length(min = 1, max = 25))]
    pub drawings: Vec<NewDrawingArg>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateDrawingParams {
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// A `drawing_id` from `degenview_list_drawings`.
    #[schemars(length(max = 20))]
    pub drawing_id: String,
    /// Move the first point (a trend line's start, or one corner of a rectangle).
    pub a: Option<AnchorArg>,
    /// Move the second point.
    pub b: Option<AnchorArg>,
    /// Line colour as `#RRGGBB` or `#RRGGBBAA`.
    #[schemars(length(max = 9))]
    pub color: Option<String>,
    /// Line width in pixels, 0.5 to 60.
    #[schemars(range(min = 0.5, max = 60.0))]
    pub width: Option<f64>,
    pub dash: Option<McpDash>,
    pub extend_left: Option<bool>,
    pub extend_right: Option<bool>,
    /// Rectangles only: the fill as `#RRGGBB` (translucent) or `#RRGGBBAA`.
    #[schemars(length(max = 9))]
    pub fill_color: Option<String>,
    /// Rectangles only: the label; an empty string clears it.
    #[schemars(length(max = 500))]
    pub text: Option<String>,
    /// Hide or show the drawing.
    pub hidden: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteDrawingsParams {
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// `drawing_id`s from `degenview_list_drawings`, up to 50.
    #[schemars(length(min = 1, max = 50))]
    pub drawing_ids: Vec<String>,
}

#[tool_router(router = chart_router, vis = "pub(in crate::mcp)")]
impl DegenViewMcp {
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
        Caller(client): Caller,
    ) -> Result<Json<WorkspacesResult>, CallToolResult> {
        match self
            .bridge
            .call(
                &client,
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
        Caller(client): Caller,
        Parameters(params): Parameters<ListChartsParams>,
    ) -> Result<Json<ChartsResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        match self
            .bridge
            .call(
                &client,
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
        // Nothing of the caller's is read, but a tool is only ever called by one.
        Caller(_): Caller,
    ) -> Result<Json<IndicatorsResult>, CallToolResult> {
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
        Caller(client): Caller,
        Parameters(params): Parameters<AddIndicatorParams>,
    ) -> Result<Json<AddIndicatorResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
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
                &client,
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
        Caller(client): Caller,
        Parameters(params): Parameters<AddChartParams>,
    ) -> Result<Json<AddChartResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
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
                &client,
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

    #[tool(
        name = "degenview_get_candles",
        description = "Read a chart's price candles: the market's own bars (never Renko/Heikin Ashi bricks), cut at the replay position, oldest first. Returns rows under named `columns` (time, open, high, low, close, volume), a summary of the highest high and lowest low, and whether the newest bar is still forming. Times are ISO-8601 UTC; pass them back as a drawing's `time`. Returns what the chart has loaded (about 500 to 1500 bars): page back with `before`, and `has_older_loaded` says whether there is more. Use it to find swing highs and lows before drawing trend lines with degenview_add_drawings.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_candles(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<GetCandlesParams>,
    ) -> Result<Json<CandlesResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let limit = params.limit.map_or(DEFAULT_CANDLES, |limit| limit as usize);
        if !(1..=MAX_CANDLES).contains(&limit) {
            return Err(invalid(format!(
                "limit must be between 1 and {MAX_CANDLES}"
            )));
        }
        let before = params
            .before
            .as_ref()
            .map(parse_time)
            .transpose()
            .map_err(|error| failure(&error))?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsCandlesRead,
                McpCommand::GetCandles {
                    workspace_id,
                    chart_id,
                    limit,
                    before,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Candles(candles)) => Ok(Json(*candles)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_list_drawings",
        description = "List the drawings on a chart in stacking order: id, tool (trend_line, rectangle, fib_retracement, brush, …), points as ISO time + price, style, label, and whether it is locked, hidden or editable by this server (the tools degenview_add_drawings can draw, when not locked).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_drawings(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<ChartPineParams>,
    ) -> Result<Json<DrawingsResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        match self
            .bridge
            .call(
                &client,
                Permission::DrawingsRead,
                McpCommand::ListDrawings {
                    workspace_id,
                    chart_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Drawings(drawings)) => Ok(Json(*drawings)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_add_drawings",
        description = "Draw on a chart. Tools: trend_line, rectangle, ray, extended_line, arrow and fib_retracement (two points, `a` and `b`); horizontal_line, horizontal_ray, vertical_line, crossline and text (one point, `a`, no `b`; text needs `text`). Each point is a time and a price: copy the time from degenview_get_candles (it snaps to that bar), and give a price or `price_from` (open/high/low/close of the bar at that time, so a line can touch the wicks exactly). A time past the newest bar is allowed, so a line can run into the future; `extend_left` / `extend_right` prolong a trend line or rectangle to the chart edge (the others have their own reach). `fill_color` is for rectangles; `text` labels a rectangle or is the text of a `text` drawing; a style a tool does not have is refused. Other tools (circles, channels, pitchforks, brushes, …) are listed but not drawn. Up to 25 drawings per call, added as one undo step in the app, or none if any is invalid. Drawings are market-anchored: they stay put across timeframes. Returns the new ids and the drawings.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_drawings(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<AddDrawingsParams>,
    ) -> Result<Json<DrawingsChangeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        if params.drawings.is_empty() || params.drawings.len() > MAX_DRAWINGS_PER_CALL {
            return Err(invalid(format!(
                "give between 1 and {MAX_DRAWINGS_PER_CALL} drawings"
            )));
        }
        let drawings = params
            .drawings
            .iter()
            .enumerate()
            .map(|(index, drawing)| {
                drawing.check().map_err(|error| match error {
                    McpError::InvalidArgument(message) => {
                        McpError::InvalidArgument(format!("Drawing {}: {message}", index + 1))
                    }
                    other => other,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| failure(&error))?;
        match self
            .bridge
            .call(
                &client,
                Permission::DrawingsWrite,
                McpCommand::AddDrawings {
                    workspace_id,
                    chart_id,
                    drawings,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::DrawingsChanged(changed)) => Ok(Json(*changed)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_update_drawing",
        description = "Change a drawing degenview_add_drawings can draw: move its points (same time/price rules; a one-point drawing has only `a`), restyle it, relabel a rectangle or text, or hide/show it. Only what you give changes. Locked drawings are refused (CONFLICT): unlock them in DegenView first. The change is one undo step in the app.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_drawing(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<UpdateDrawingParams>,
    ) -> Result<Json<DrawingsChangeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let drawing_id = parse_drawing_id(&params.drawing_id).map_err(|error| failure(&error))?;
        let patch = StylePatch::check(
            params.color.as_deref(),
            params.width,
            params.dash,
            (params.extend_left, params.extend_right),
            params.fill_color.as_deref(),
            params.text.as_deref(),
        )
        .map_err(|error| failure(&error))?;
        let a = params
            .a
            .as_ref()
            .map(|point| point.check("a"))
            .transpose()
            .map_err(|error| failure(&error))?;
        let b = params
            .b
            .as_ref()
            .map(|point| point.check("b"))
            .transpose()
            .map_err(|error| failure(&error))?;
        if a.is_none() && b.is_none() && patch.is_empty() && params.hidden.is_none() {
            return Err(invalid("give at least one thing to change"));
        }
        match self
            .bridge
            .call(
                &client,
                Permission::DrawingsWrite,
                McpCommand::EditDrawing {
                    workspace_id,
                    chart_id,
                    edit: Box::new(DrawingEdit {
                        drawing_id,
                        a,
                        b,
                        patch,
                        hidden: params.hidden,
                    }),
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::DrawingsChanged(changed)) => Ok(Json(*changed)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_delete_drawings",
        description = "Remove drawings from a chart (up to 50). All or none: if any id is unknown or locked, nothing is removed. The user can undo the removal in the app. Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn delete_drawings(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<DeleteDrawingsParams>,
    ) -> Result<Json<DrawingsChangeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        if params.drawing_ids.is_empty() || params.drawing_ids.len() > MAX_DELETES_PER_CALL {
            return Err(invalid(format!(
                "give between 1 and {MAX_DELETES_PER_CALL} drawing ids"
            )));
        }
        let drawing_ids = params
            .drawing_ids
            .iter()
            .map(|id| parse_drawing_id(id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| failure(&error))?;
        match self
            .bridge
            .call(
                &client,
                Permission::DrawingsDelete,
                McpCommand::RemoveDrawings {
                    workspace_id,
                    chart_id,
                    drawing_ids,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::DrawingsChanged(changed)) => Ok(Json(*changed)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}
