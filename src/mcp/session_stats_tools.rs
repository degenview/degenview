//! `degenview_get_session_stats`: how often something happened on the days a chart covers, with
//! its sample size and a 95% interval. A read of the chart's own candles, like the indicator
//! values, so it needs the candles permission and changes nothing.

use crate::mcp::bridge::{McpCommand, McpReply, REQUEST_TIMEOUT};
use crate::mcp::session_stats::{Query, SessionStatsResult};
use crate::mcp::tools::{
    DegenViewMcp, client_of, failure, invalid, parse_chart_id, parse_id, unexpected,
};
use crate::model::mcp::Permission;
use crate::model::session_stats::{Filter, Outcome, Side, WeekdaySet};
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub const GET_SESSION_STATS: &str = "degenview_get_session_stats";

/// The largest gap, in percent, a filter may name; a bigger one is a typo for a fraction.
const MAX_GAP_PCT: f64 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeArg {
    GapFill,
    DayUp,
    PriorHighBreak,
    PriorLowBreak,
    FirstBarHighBreak,
    FirstBarLowBreak,
}

impl OutcomeArg {
    fn outcome(self) -> Outcome {
        match self {
            Self::GapFill => Outcome::GapFill,
            Self::DayUp => Outcome::DayUp,
            Self::PriorHighBreak => Outcome::PriorHighBreak,
            Self::PriorLowBreak => Outcome::PriorLowBreak,
            Self::FirstBarHighBreak => Outcome::FirstBarHighBreak,
            Self::FirstBarLowBreak => Outcome::FirstBarLowBreak,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SideArg {
    Any,
    Up,
    Down,
}

impl SideArg {
    fn side(self) -> Side {
        match self {
            Self::Any => Side::Any,
            Self::Up => Side::Up,
            Self::Down => Side::Down,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionStatsParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts`.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    /// What to count. `gap_fill`: the day's range touched the previous close (days that gapped
    /// only). `day_up`: closed above its open. `prior_high_break` / `prior_low_break`: went beyond
    /// the previous day's high / low. `first_bar_high_break` / `first_bar_low_break`: a later bar
    /// went beyond the first bar's high / low (needs the 1H timeframe).
    pub outcome: OutcomeArg,
    /// Only count these weekdays, as `Mon`…`Sun`. Default: every day.
    #[schemars(length(min = 1, max = 7))]
    pub weekdays: Option<Vec<String>>,
    /// Only days that opened above (`up`) or below (`down`) the previous close. Default `any`.
    pub gap: Option<SideArg>,
    /// Only gaps at least this large, in percent of the previous close, ignoring the sign.
    #[schemars(range(min = 0, max = 1000))]
    pub gap_min_pct: Option<f64>,
    /// Only gaps at most this large, in percent of the previous close, ignoring the sign.
    #[schemars(range(min = 0, max = 1000))]
    pub gap_max_pct: Option<f64>,
    /// Only days after a previous day that closed above (`up`) or below (`down`) its open.
    pub prior_day: Option<SideArg>,
}

/// The question a call asks, or the first argument that is wrong.
fn query(params: &SessionStatsParams) -> Result<Query, CallToolResult> {
    let weekdays = match &params.weekdays {
        None => WeekdaySet::ALL,
        Some(names) => {
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            WeekdaySet::from_names(&names)
                .filter(|set| !set.is_empty())
                .ok_or_else(|| {
                    invalid("weekdays must be names from Mon, Tue, Wed, Thu, Fri, Sat, Sun")
                })?
        }
    };
    for (name, value) in [
        ("gap_min_pct", params.gap_min_pct),
        ("gap_max_pct", params.gap_max_pct),
    ] {
        if let Some(value) = value
            && !(value.is_finite() && (0.0..=MAX_GAP_PCT).contains(&value))
        {
            return Err(invalid(format!(
                "{name} must be a percent between 0 and {MAX_GAP_PCT}"
            )));
        }
    }
    if let (Some(least), Some(most)) = (params.gap_min_pct, params.gap_max_pct)
        && least > most
    {
        return Err(invalid("gap_min_pct is larger than gap_max_pct"));
    }
    Ok(Query {
        outcome: params.outcome.outcome(),
        filter: Filter {
            weekdays,
            gap: params.gap.map_or(Side::Any, SideArg::side),
            gap_min_pct: params.gap_min_pct,
            gap_max_pct: params.gap_max_pct,
            prior_day: params.prior_day.map_or(Side::Any, SideArg::side),
        },
    })
}

#[tool_router(router = session_stats_router, vis = "pub(super)")]
impl DegenViewMcp {
    #[tool(
        name = "degenview_get_session_stats",
        description = "Count how often something happened on the days a chart's bars cover, as P(outcome | conditions): for example how often a gap was filled on Tuesdays that gapped up 0.1% to 0.6%. Returns the qualifying days `n`, the days it happened on `hits`, the `estimate` with a 95% Wilson interval, the older half against the newer half, a row per year and, on 1H bars, the bar a gap was filled or a first-bar range was broken in. A day is a US stock's regular session (New York) or, for any other market, the UTC date; a gap is the day's open against the previous day's close, so a market that trades round the clock has almost none. Works on the 1H and 1D timeframes and on the market's own candles up to the replay position. These are historical frequencies with sample sizes, never predictions: say `n` and the interval whenever you quote the estimate, and treat a condition picked after looking at the data as overfit.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_session_stats(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<SessionStatsParams>,
    ) -> Result<Json<SessionStatsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = parse_chart_id(&params.chart_id)?;
        let query = query(&params)?;
        match self
            .bridge
            .call(
                &client,
                Permission::ChartsCandlesRead,
                McpCommand::GetSessionStats {
                    workspace_id,
                    chart_id,
                    query,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::SessionStats(result)) => Ok(Json(*result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}
