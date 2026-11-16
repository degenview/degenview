//! How a prediction market's line is drawn at a timeframe. A line has no candles to fold: each
//! point is a price, so the timeframe sets how far back the line reaches and how far apart its
//! points are, for any source that draws one.

use crate::model::Timeframe;

const DAY_MS: i64 = 86_400_000;
const WEEK_MS: i64 = 7 * DAY_MS;

/// The line spans this many candles of the timeframe, but...
const BARS: i64 = 60;
/// ...never less than a month (an hour's worth of days left a flat stub on a market that is older)
/// and never more than a year, the longest history the sources keep points for.
const MIN_SPAN_MS: i64 = 30 * DAY_MS;
const MAX_SPAN_MS: i64 = 364 * DAY_MS;
/// A point is never further apart than a week, so a year of line stays a line.
const MAX_STEP_MS: i64 = WEEK_MS;
/// The most points one line holds: a one-minute line reaches back hours, not a month.
pub const MAX_POINTS: i64 = 1_000;

/// Where a line starts and how often it has a point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineWindow {
    /// How far back the line reaches.
    pub span_ms: i64,
    /// The spacing of its points, at the finest the timeframe asks.
    pub step_ms: i64,
}

impl LineWindow {
    /// The spacing in whole minutes, at least one.
    pub fn step_minutes(self) -> i64 {
        (self.step_ms / 60_000).max(1)
    }
}

/// The window of a line at `timeframe`: a point per candle length (at most a week apart) over
/// sixty candles, between a month and a year, and not more than a thousand points.
pub fn window(timeframe: Timeframe) -> LineWindow {
    let length = timeframe.nominal_ms();
    let step_ms = length.min(MAX_STEP_MS);
    let span_ms = (length * BARS)
        .clamp(MIN_SPAN_MS, MAX_SPAN_MS)
        .min(step_ms * MAX_POINTS);
    LineWindow { span_ms, step_ms }
}

#[cfg(test)]
mod tests;
