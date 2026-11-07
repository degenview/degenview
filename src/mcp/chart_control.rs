//! Chart control tools: move a workspace to another timeframe, switch a chart's type, and remove
//! a chart. They call the workspace's own methods, the ones behind the toolbar's timeframe
//! buttons and the chart's type menu and Remove button.

use crate::mcp::bridge::{McpCommand, McpReply, REQUEST_TIMEOUT};
use crate::mcp::tools::{
    DegenViewMcp, client_of, failure, invalid, parse_chart_id, parse_id, unexpected,
};
use crate::mcp::types::ChartInfo;
use crate::model::mcp::Permission;
use crate::model::{ChartType, Timeframe};
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SET_TIMEFRAME: &str = "degenview_set_timeframe";
pub const SET_CHART_TYPE: &str = "degenview_set_chart_type";
pub const REMOVE_CHART: &str = "degenview_remove_chart";

/// The timeframes a workspace offers, as its toolbar labels them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum McpTimeframe {
    #[serde(rename = "1H")]
    Hour,
    #[serde(rename = "1D")]
    Day,
    #[serde(rename = "1W")]
    Week,
    #[serde(rename = "1M")]
    Month,
    #[serde(rename = "3M")]
    Quarter,
    #[serde(rename = "1Y")]
    Year,
}

impl McpTimeframe {
    pub fn timeframe(self) -> Timeframe {
        match self {
            Self::Hour => Timeframe::Hour,
            Self::Day => Timeframe::Day,
            Self::Week => Timeframe::Week,
            Self::Month => Timeframe::Month,
            Self::Quarter => Timeframe::Quarter,
            Self::Year => Timeframe::Year,
        }
    }
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
    pub timeframe: McpTimeframe,
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

#[tool_router(router = chart_control_router, vis = "pub(super)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_set_timeframe",
        description = "Move a workspace to another timeframe (1H, 1D, 1W, 1M, 3M, 1Y). The timeframe belongs to the workspace, so every chart in it reloads its candles for the new bar length and goes back to the newest bars; replay re-bases. Candles and indicators read right after may still be loading: check the charts' `status` in degenview_list_charts. Answers `unchanged` when the workspace already uses it.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn set_timeframe(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<SetTimeframeParams>,
    ) -> Result<Json<TimeframeResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsViewWrite,
                McpCommand::SetTimeframe {
                    workspace_id,
                    timeframe: params.timeframe.timeframe(),
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<SetChartTypeParams>,
    ) -> Result<Json<ChartChangeResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
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
                McpCommand::SetChartType {
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
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<RemoveChartParams>,
    ) -> Result<Json<ChartChangeResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsDelete,
                McpCommand::RemoveChart {
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

    #[test]
    fn timeframes_map_to_the_toolbar_ones() {
        let all = [
            McpTimeframe::Hour,
            McpTimeframe::Day,
            McpTimeframe::Week,
            McpTimeframe::Month,
            McpTimeframe::Quarter,
            McpTimeframe::Year,
        ];
        let labels: Vec<_> = all.iter().map(|t| t.timeframe().label()).collect();
        assert_eq!(labels, ["1H", "1D", "1W", "1M", "3M", "1Y"]);
        let parsed: McpTimeframe = serde_json::from_str("\"3M\"").unwrap();
        assert_eq!(parsed, McpTimeframe::Quarter);
        assert!(serde_json::from_str::<McpTimeframe>("\"4h\"").is_err());
    }
}
