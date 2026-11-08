//! The pure part of `degenview_get_session_stats`: a question over a chart's own candles, as the
//! tool returns it. The counting is `model::session_stats`; this turns its answer into the
//! result a client reads, and decides what an answer that cannot be given is called.

use crate::mcp::bridge::McpError;
use crate::model::Kline;
use crate::model::session_stats::{
    self, Estimate, Filter, MarketHours, Outcome, Stats, Unavailable,
};
use schemars::JsonSchema;
use serde::Serialize;

/// What a client asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub outcome: Outcome,
    pub filter: Filter,
}

/// What the answer belongs to.
pub struct Meta {
    pub workspace_id: u64,
    pub chart_id: u64,
    pub instrument: String,
    pub timeframe: String,
    pub bar_ms: i64,
    pub hours: MarketHours,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct EstimateOut {
    pub hits: usize,
    pub n: usize,
    /// `hits / n` as a fraction (0.8 is 80%); null for no days.
    pub proportion: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct YearOut {
    pub year: i64,
    #[serde(flatten)]
    pub estimate: EstimateOut,
}

/// Where in the day the outcome happened, over the days it happened on.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TimingOut {
    /// Days it happened on.
    pub n: usize,
    /// Bar indexes counted from the day's first bar (0). Nearest-rank percentiles.
    pub median_bar: usize,
    pub p25_bar: usize,
    pub p75_bar: usize,
    pub p90_bar: usize,
    /// The length of a bar, so an index can be turned into time after the open.
    pub bar_minutes: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SessionStatsResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub instrument: String,
    pub timeframe: String,
    /// The question asked: `gap_fill`, `day_up`, `prior_high_break`, `prior_low_break`,
    /// `first_bar_high_break` or `first_bar_low_break`.
    pub outcome: String,
    /// The question in words.
    pub question: String,
    /// The days counted, in words ("every day", "Tue · gap up 0.1%–0.6%").
    pub conditions: String,
    /// What a day is: `US regular hours` (a US stock) or `UTC days`.
    pub session_basis: String,
    /// Whether the chart's bars were grouped into days (hourly) rather than one bar a day.
    pub intraday: bool,
    /// Sessions found in the bars, qualifying or not.
    pub sessions: usize,
    pub first_session: Option<String>,
    pub last_session: Option<String>,
    /// Days that qualified and days the outcome happened on.
    pub n: usize,
    pub hits: usize,
    /// The share of qualifying days it happened on, as a fraction; null when none qualified.
    pub estimate: Option<f64>,
    /// The 95% Wilson interval around `estimate`.
    pub ci_low: Option<f64>,
    pub ci_high: Option<f64>,
    /// The older and newer half of the qualifying days; null below 10 days a half.
    pub older_half: Option<EstimateOut>,
    pub newer_half: Option<EstimateOut>,
    /// Whether the halves' intervals overlap.
    pub halves_agree: Option<bool>,
    pub per_year: Vec<YearOut>,
    pub timing: Option<TimingOut>,
    pub notes: Vec<String>,
    /// Always the same reminder: these are counts, not forecasts.
    pub disclaimer: String,
}

fn estimate_out(estimate: Estimate) -> EstimateOut {
    EstimateOut {
        hits: estimate.hits,
        n: estimate.n,
        proportion: estimate.proportion(),
    }
}

fn date(date: (i64, i64, i64)) -> String {
    format!("{:04}-{:02}-{:02}", date.0, date.1, date.2)
}

fn result(meta: &Meta, stats: &Stats) -> SessionStatsResult {
    let interval = stats.estimate.interval();
    SessionStatsResult {
        workspace_id: meta.workspace_id.to_string(),
        chart_id: meta.chart_id.to_string(),
        instrument: meta.instrument.clone(),
        timeframe: meta.timeframe.clone(),
        outcome: stats.outcome.slug().to_owned(),
        question: stats.outcome.question().to_owned(),
        conditions: stats.filter.describe(),
        session_basis: stats.hours.label().to_owned(),
        intraday: stats.intraday,
        sessions: stats.sessions,
        first_session: stats.span.map(|(first, _)| date(first)),
        last_session: stats.span.map(|(_, last)| date(last)),
        n: stats.estimate.n,
        hits: stats.estimate.hits,
        estimate: stats.estimate.proportion(),
        ci_low: interval.map(|(low, _)| low),
        ci_high: interval.map(|(_, high)| high),
        older_half: stats.stability.map(|halves| estimate_out(halves.older)),
        newer_half: stats.stability.map(|halves| estimate_out(halves.newer)),
        halves_agree: stats.stability.map(|halves| halves.agree),
        per_year: stats
            .per_year
            .iter()
            .map(|row| YearOut {
                year: row.year,
                estimate: estimate_out(row.estimate),
            })
            .collect(),
        timing: stats.timing.map(|timing| TimingOut {
            n: timing.n,
            median_bar: timing.median,
            p25_bar: timing.p25,
            p75_bar: timing.p75,
            p90_bar: timing.p90,
            bar_minutes: meta.bar_ms as f64 / 60_000.0,
        }),
        notes: stats
            .notes
            .iter()
            .map(|note| note.text().to_owned())
            .collect(),
        disclaimer: "Historical frequencies of this chart's bars with their sample size, not \
                     predictions and not advice. A condition chosen after looking at the data \
                     overstates what it finds."
            .to_owned(),
    }
}

/// Answers `query` over `candles`.
pub fn session_stats(
    meta: &Meta,
    candles: &[Kline],
    query: &Query,
) -> Result<SessionStatsResult, McpError> {
    session_stats::compute(
        candles,
        meta.bar_ms,
        meta.hours,
        query.outcome,
        &query.filter,
    )
    .map(|stats| result(meta, &stats))
    .map_err(|reason| match reason {
        Unavailable::NoBars => McpError::NotFound(reason.text().to_owned()),
        Unavailable::TooCoarse | Unavailable::NeedsIntraday => {
            McpError::Unsupported(reason.text().to_owned())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::session_stats::Side;

    const DAY_MS: i64 = 86_400_000;

    fn meta(bar_ms: i64) -> Meta {
        Meta {
            workspace_id: 1,
            chart_id: 2,
            instrument: "BINANCE:BTCUSDT".into(),
            timeframe: "1D".into(),
            bar_ms,
            hours: MarketHours::Continuous,
        }
    }

    fn bars(count: usize) -> Vec<Kline> {
        (0..count)
            .map(|i| {
                let open = 100.0 + i as f64;
                Kline {
                    open_time: 1_704_067_200_000 + i as i64 * DAY_MS,
                    close_time: 1_704_067_200_000 + (i as i64 + 1) * DAY_MS - 1,
                    open,
                    high: open + 3.0,
                    low: open - 3.0,
                    close: if i % 4 == 0 { open - 1.0 } else { open + 1.0 },
                    volume: 1.0,
                }
            })
            .collect()
    }

    fn query(outcome: Outcome) -> Query {
        Query {
            outcome,
            filter: Filter::default(),
        }
    }

    #[test]
    fn the_result_carries_the_count_the_interval_and_the_reminder() {
        let result = session_stats(&meta(DAY_MS), &bars(40), &query(Outcome::DayUp)).unwrap();
        assert_eq!((result.hits, result.n), (30, 40));
        assert_eq!(result.estimate, Some(0.75));
        let (low, high) = (result.ci_low.unwrap(), result.ci_high.unwrap());
        assert!(low < 0.75 && 0.75 < high);
        assert_eq!(result.outcome, "day_up");
        assert_eq!(result.conditions, "every day");
        assert_eq!(result.session_basis, "UTC days");
        assert_eq!(result.first_session.as_deref(), Some("2024-01-01"));
        assert_eq!(result.per_year.len(), 1);
        assert_eq!(result.older_half.unwrap().n, 20);
        assert!(result.disclaimer.contains("not predictions"));
        assert_eq!(result.timing, None);
    }

    #[test]
    fn conditions_narrow_and_are_named() {
        let mut asked = query(Outcome::DayUp);
        asked.filter.prior_day = Side::Down;
        let result = session_stats(&meta(DAY_MS), &bars(40), &asked).unwrap();
        assert_eq!(result.conditions, "prior day down");
        assert!(result.n < 40);
    }

    #[test]
    fn what_cannot_be_answered_has_its_own_error() {
        let refused = session_stats(&meta(7 * DAY_MS), &bars(10), &query(Outcome::DayUp));
        assert!(matches!(refused, Err(McpError::Unsupported(_))));
        let refused = session_stats(&meta(DAY_MS), &bars(10), &query(Outcome::FirstBarLowBreak));
        assert!(matches!(refused, Err(McpError::Unsupported(_))));
        let empty = session_stats(&meta(DAY_MS), &[], &query(Outcome::DayUp));
        assert!(matches!(empty, Err(McpError::NotFound(_))));
    }
}
