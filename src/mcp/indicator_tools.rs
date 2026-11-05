//! Indicator tools: read RSI, EMA, Bollinger and trend-flip values for a chart, and take a
//! built-in indicator off it. Adding one is `degenview_add_indicator`; what a chart has applied
//! is in `degenview_list_charts`.

use crate::mcp::bridge::{McpCommand, McpReply, REQUEST_TIMEOUT};
use crate::mcp::drawings::{TimeArg, parse_time};
use crate::mcp::indicator_values::{
    IndicatorValuesResult, MAX_EMA_PERIOD, MIN_EMA_PERIOD, Reading, ReadingRequest,
};
use crate::mcp::tools::{
    DegenViewMcp, McpIndicator, client_of, failure, invalid, parse_chart_id, parse_id, unexpected,
};
use crate::mcp::types::AddIndicatorResult;
use crate::model::mcp::Permission;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub const GET_INDICATOR_VALUES: &str = "degenview_get_indicator_values";
pub const REMOVE_INDICATOR: &str = "degenview_remove_indicator";

const DEFAULT_BARS: usize = 100;
const MAX_BARS: usize = crate::mcp::chart_data::MAX_CANDLES;
const DEFAULT_EMA_PERIOD: usize = crate::model::EMA_DEFAULT_PERIOD;

/// The indicators whose values can be read (volume is a column of `degenview_get_candles`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadableIndicator {
    Rsi,
    Ema,
    Bollinger,
    TrendFlips,
}

impl ReadableIndicator {
    fn reading(self) -> Reading {
        match self {
            Self::Rsi => Reading::Rsi,
            Self::Ema => Reading::Ema,
            Self::Bollinger => Reading::Bollinger,
            Self::TrendFlips => Reading::TrendFlips,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndicatorValuesParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts`.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// Which to read. Default: all four. The indicator does not have to be applied to the chart.
    #[schemars(length(min = 1, max = 4))]
    pub indicators: Option<Vec<ReadableIndicator>>,
    /// The EMA length in bars, 2 to 400. Default 20.
    #[schemars(range(min = 2, max = 400))]
    pub ema_period: Option<u32>,
    /// How many of the newest bars to return, 1 to 1000. Default 100.
    #[schemars(range(min = 1, max = 1000))]
    pub limit: Option<u32>,
    /// Return the bars that start before this time (ISO-8601 UTC or epoch milliseconds), to page
    /// back. Use the first of the previous page's `times`.
    pub before: Option<TimeArg>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveIndicatorParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts` in that workspace.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    pub indicator: McpIndicator,
}

/// The request a tool call asks for, or the first argument that is wrong.
fn reading_request(params: &IndicatorValuesParams) -> Result<ReadingRequest, CallToolResult> {
    let mut readings: Vec<Reading> = Vec::new();
    match &params.indicators {
        None => readings.extend(Reading::ALL),
        Some(asked) if asked.is_empty() => return Err(invalid("indicators is empty")),
        Some(asked) => {
            for indicator in asked {
                if !readings.contains(&indicator.reading()) {
                    readings.push(indicator.reading());
                }
            }
        }
    }
    let ema_period = params.ema_period.map_or(DEFAULT_EMA_PERIOD, |p| p as usize);
    if !(MIN_EMA_PERIOD..=MAX_EMA_PERIOD).contains(&ema_period) {
        return Err(invalid(format!(
            "ema_period must be between {MIN_EMA_PERIOD} and {MAX_EMA_PERIOD}"
        )));
    }
    let limit = params.limit.map_or(DEFAULT_BARS, |limit| limit as usize);
    if !(1..=MAX_BARS).contains(&limit) {
        return Err(invalid(format!("limit must be between 1 and {MAX_BARS}")));
    }
    let before = params
        .before
        .as_ref()
        .map(parse_time)
        .transpose()
        .map_err(|error| failure(&error))?;
    Ok(ReadingRequest {
        readings,
        ema_period,
        limit,
        before,
    })
}

#[tool_router(router = indicator_router, vis = "pub(super)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_get_indicator_values",
        description = "Read built-in indicator values for a chart: RSI (14), an EMA of any length (default 20), Bollinger Bands (20, 2 deviations) and Supertrend trend flips, aligned with the bars the chart draws (its own candles, or the built bars of a Heikin Ashi, Renko, Kagi, Line Break, Point & Figure or Range chart: see `chart_type`) and computed with the same code the chart draws. Returns `times` (oldest first), the `closes`, and one named series per indicator with a value per bar (null while warming up). Computed over every bar the chart holds, so values are warm; page back with `before`. The indicator need not be applied to the chart. Use it instead of recomputing from degenview_get_candles.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_indicator_values(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<IndicatorValuesParams>,
    ) -> Result<Json<IndicatorValuesResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let request = reading_request(&params)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsCandlesRead,
                McpCommand::GetIndicatorValues {
                    workspace_id,
                    chart_id,
                    request,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::IndicatorValues(values)) => Ok(Json(*values)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_remove_indicator",
        description = "Take a built-in indicator off a chart (a hidden one counts as applied). Returns `removed`, or `not_applied` when the chart did not have it, so a retry is harmless, plus the chart's indicators afterwards. The inverse of degenview_add_indicator.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_indicator(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<RemoveIndicatorParams>,
    ) -> Result<Json<AddIndicatorResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsIndicatorsWrite,
                McpCommand::RemoveIndicator {
                    workspace_id,
                    chart_id,
                    indicator: params.indicator.built_in(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(json: serde_json::Value) -> IndicatorValuesParams {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn defaults_are_all_four_indicators_a_20_ema_and_100_bars() {
        let request = reading_request(&params(serde_json::json!({
            "workspace_id": "1", "chart_id": "2"
        })))
        .unwrap();
        assert_eq!(request.readings, Reading::ALL);
        assert_eq!(
            (request.ema_period, request.limit, request.before),
            (20, 100, None)
        );
    }

    #[test]
    fn repeats_collapse_and_bad_values_are_refused() {
        let request = reading_request(&params(serde_json::json!({
            "workspace_id": "1", "chart_id": "2",
            "indicators": ["ema", "rsi", "ema"], "ema_period": 7,
        })))
        .unwrap();
        assert_eq!(request.readings, [Reading::Ema, Reading::Rsi]);
        assert_eq!(request.ema_period, 7);
        for bad in [
            serde_json::json!({"indicators": []}),
            serde_json::json!({"ema_period": 1}),
            serde_json::json!({"ema_period": 401}),
            serde_json::json!({"limit": 0}),
            serde_json::json!({"limit": 1001}),
            serde_json::json!({"before": "yesterday"}),
        ] {
            let mut args = serde_json::json!({"workspace_id": "1", "chart_id": "2"});
            args.as_object_mut()
                .unwrap()
                .extend(bad.as_object().unwrap().clone());
            assert!(reading_request(&params(args.clone())).is_err(), "{args}");
        }
    }
}
