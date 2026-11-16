//! Chart control tools: move a workspace to another timeframe, switch a chart's type, and remove
//! a chart. They call the workspace's own methods, the ones behind the toolbar's timeframe
//! buttons and the chart's type menu and Remove button.

use crate::mcp::bridge::{McpReply, REQUEST_TIMEOUT};
use crate::mcp::chart::ChartCommand;
use crate::mcp::tools::{
    Caller, DegenViewMcp, failure, invalid, parse_chart_id, parse_workspace_id, unexpected,
};
use crate::mcp::types::ChartInfo;
use crate::model::mcp::Permission;
use crate::model::{ChartType, Timeframe};
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SET_TIMEFRAME: &str = "degenview_set_timeframe";
pub const SET_CHART_TYPE: &str = "degenview_set_chart_type";
pub const REMOVE_CHART: &str = "degenview_remove_chart";

/// How a timeframe is written in a tool's arguments, for the descriptions and the schema.
pub const TIMEFRAME_GRAMMAR: &str = "a count and a unit: m minutes, h hours, D days, W weeks, M months, or 1Y; e.g. 15m, 4h, 1D, 2W, 3M. Seconds are not offered";

/// The timeframe a tool argument names (`15m`, `4h`, `1D`, `1H`, `3M`, `1Y`, ...), or the tool
/// error that says what is accepted.
pub fn parse_timeframe(text: &str) -> Result<Timeframe, CallToolResult> {
    Timeframe::parse(text).map_err(|error| {
        invalid(format!(
            "{text:?} is not a timeframe: {error}. A timeframe is {TIMEFRAME_GRAMMAR}"
        ))
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TimeframeResult {
    pub workspace_id: String,
    /// The workspace's timeframe now.
    pub timeframe: String,
    pub previous_timeframe: String,
    /// `changed`, or `unchanged` when the workspace already used it (nothing reloads).
    pub status: String,
    /// Charts that reload their candles; they show `loading` until the new bars arrive.
    pub chart_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ChartChangeResult {
    pub workspace_id: String,
    pub chart_id: String,
    /// `changed`, `unchanged` or `removed`.
    pub status: String,
    /// The chart as it is now; null once removed.
    pub chart: Option<ChartInfo>,
    /// Charts the workspace holds afterwards.
    pub charts_in_workspace: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetTimeframeParams {
    /// A `workspace_id` from `degenview_list_workspaces`. Every chart in it changes.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// The bar length: a count and a unit (m minutes, h hours, D days, W weeks, M months, or 1Y),
    /// for example 15m, 4h, 1D, 2W, 3M.
    #[schemars(length(min = 1, max = 16))]
    pub timeframe: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetChartTypeParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts`.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// One of: candles, hollow_candles, bars, high_low_bars, volume_candles, line,
    /// line_with_markers, step_line, area, hlc_area, baseline, columns, heikin_ashi, renko,
    /// line_break, kagi, point_and_figure, range.
    #[schemars(length(min = 1, max = 32))]
    pub chart_type: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveChartParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts`.
    #[schemars(length(max = 20))]
    pub chart_id: String,
}

/// The chart type named by `slug` (`candles`, `heikin_ashi`, …).
pub fn chart_type_named(slug: &str) -> Option<ChartType> {
    let slug = slug.trim();
    ChartType::ALL.into_iter().find(|kind| kind.slug() == slug)
}

#[tool_router(router = chart_control_router, vis = "pub(in crate::mcp)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_set_timeframe",
        description = "Move a workspace to another timeframe (a count and unit: m minutes, h hours, D days, W weeks, M months, or 1Y; e.g. 15m, 4h, 1D, 2W, 3M). A source that has no such candle builds it from finer ones, or refuses the chart with its reason. The timeframe belongs to the workspace, so every chart in it reloads its candles for the new bar length and goes back to the newest bars; replay re-bases. Candles and indicators read right after may still be loading: check the charts' `status` in degenview_list_charts. Answers `unchanged` when the workspace already uses it.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn set_timeframe(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<SetTimeframeParams>,
    ) -> Result<Json<TimeframeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let timeframe = parse_timeframe(&params.timeframe)?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsViewWrite,
                ChartCommand::SetTimeframe {
                    workspace_id,
                    timeframe,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Timeframe(result)) => Ok(Json(result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_set_chart_type",
        description = "Switch a market chart to another chart type, as its type menu does: candles, hollow candles, bars, lines, areas, Heikin Ashi, Renko and the rest. Each type keeps the settings it had. A market drawn as a line (prediction markets, CoinMarketCap) and a source with no volume for volume candles refuse with UNSUPPORTED. Price-based types (Heikin Ashi, Renko, Kagi, Line Break, Point & Figure, Range) draw bars built from the candles; degenview_get_candles still returns the market's own. Answers `unchanged` when the chart already draws it.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_chart_type(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<SetChartTypeParams>,
    ) -> Result<Json<ChartChangeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let chart_type = chart_type_named(&params.chart_type).ok_or_else(|| {
            invalid(format!(
                "{:?} is not a chart type; use one of: {}",
                params.chart_type,
                ChartType::ALL.map(ChartType::slug).join(", ")
            ))
        })?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsViewWrite,
                ChartCommand::SetChartType {
                    workspace_id,
                    chart_id,
                    chart_type,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::ChartChanged(result)) => Ok(Json(*result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_remove_chart",
        description = "Remove a market chart from a workspace, as the chart's Remove button does. The drawings on it go with it and cannot be brought back from here; alerts and watchlists are not touched, and degenview_add_chart can chart the market again (with a clean chart). Portfolio cards cannot be removed this way. Asks the user to approve it in DegenView first: the first call answers CONFIRMATION_REQUIRED with a confirmation_id and changes nothing; once the user has approved, repeat the exact same call (any change to the arguments is a new question). If the user declines you get CONFIRMATION_DENIED: do not retry it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn remove_chart(
        &self,
        Caller(client): Caller,
        Parameters(params): Parameters<RemoveChartParams>,
    ) -> Result<Json<ChartChangeResult>, CallToolResult> {
        let workspace_id = parse_workspace_id(&params.workspace_id)?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsDelete,
                ChartCommand::RemoveChart {
                    workspace_id,
                    chart_id,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::ChartChanged(result)) => Ok(Json(*result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chart_type_is_named_by_its_slug_and_nothing_else_is() {
        for kind in ChartType::ALL {
            assert_eq!(chart_type_named(kind.slug()), Some(kind));
        }
        assert_eq!(chart_type_named(" renko "), Some(ChartType::Renko));
        assert_eq!(chart_type_named("Candles"), None);
        assert_eq!(chart_type_named("macd"), None);
        assert_eq!(chart_type_named(""), None);
    }

    fn message(text: &str) -> String {
        let result = parse_timeframe(text).expect_err("not a timeframe");
        format!("{:?}", result.content)
    }

    #[test]
    fn timeframes_are_read_as_a_count_and_a_unit() {
        for (text, expected) in [
            ("1H", Timeframe::HOUR_1),
            ("1h", Timeframe::HOUR_1),
            ("1D", Timeframe::DAY_1),
            ("1W", Timeframe::WEEK_1),
            ("1M", Timeframe::MONTH_1),
            ("3M", Timeframe::MONTH_3),
            ("1Y", Timeframe::YEAR_1),
            ("15m", Timeframe::MINUTE_15),
            ("4h", Timeframe::HOUR_4),
        ] {
            assert_eq!(parse_timeframe(text).ok(), Some(expected), "{text}");
        }
        assert_eq!(
            parse_timeframe("22m").ok().map(|t| t.label()),
            Some("22m".to_owned())
        );
        assert_eq!(
            parse_timeframe("2D").ok().map(|t| t.label()),
            Some("2D".to_owned())
        );
    }

    #[test]
    fn what_is_no_timeframe_is_refused_with_the_reason_and_the_grammar() {
        assert!(message("30s").contains("Seconds aren't supported"));
        assert!(message("0m").contains("can't be zero"));
        assert!(message("2000m").contains("Minutes go from 1 to 1440"));
        assert!(message("soon").contains("a count and a unit"));
        assert!(message("").contains("m minutes, h hours"));
    }
}
