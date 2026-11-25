//! Where candles open and close, for any [`Timeframe`], and folding finer candles into coarser
//! ones. Every provider, the chart and Pine cut buckets here, so one market looks the same
//! whichever source serves it.
//!
//! Buckets are anchored in UTC (the [`Grid::Utc`] every crypto source shares):
//!
//! - **Seconds, minutes and hours** restart at midnight: a `22m` day has 65 full buckets and a
//!   shorter last one that ends at 00:00 (a `7s` day ends the same way).
//! - **Days** count from 1970-01-01: the day number floored to a multiple of the count.
//! - **Weeks** start on Monday: the week number (from Monday 1969-12-29) floored to a multiple of
//!   the count.
//! - **Months** count from 1970-01: the month number floored to a multiple of the count, so `3M`
//!   is the calendar quarter and `12M` the calendar year.
//!
//! A bucket closes 1 ms before the next opens.
//!
//! A US stock's intraday buckets are cut on its session instead ([`Grid::UsEquities`]: anchored at
//! 09:30 New York, restarted by every session, cut at the 16:00 close, nothing outside it). A
//! source says which grid its markets use (`DataSource::grid`) and plans with it ([`FoldPlan`]).

use crate::format::{civil_from_days, days_from_civil};
use crate::model::{Kline, TimeUnit, Timeframe};

mod grid;
mod plan;

pub use grid::Grid;
pub use plan::FoldPlan;

const DAY_MS: i64 = 86_400_000;
/// Day 0 (1970-01-01) was a Thursday: Monday 1969-12-29 is three days before it.
const DAYS_BEFORE_FIRST_MONDAY: i64 = 3;
/// The most candles a gap fill may insert in one go; a longer silence is left as it is.
const MAX_FILLED: i64 = 20_000;

/// Open time of the bucket of `tf` that contains `t_ms`.
pub fn bucket_start_ms(tf: Timeframe, t_ms: i64) -> i64 {
    let count = i64::from(tf.count());
    let days = t_ms.div_euclid(DAY_MS);
    match tf.unit() {
        TimeUnit::Second | TimeUnit::Minute | TimeUnit::Hour => {
            let midnight = days * DAY_MS;
            let length = tf.nominal_ms();
            midnight + (t_ms - midnight).div_euclid(length) * length
        }
        TimeUnit::Day => days.div_euclid(count) * count * DAY_MS,
        TimeUnit::Week => {
            let weeks = (days + DAYS_BEFORE_FIRST_MONDAY).div_euclid(7);
            (weeks.div_euclid(count) * count * 7 - DAYS_BEFORE_FIRST_MONDAY) * DAY_MS
        }
        TimeUnit::Month => month_start_ms(month_index(days).div_euclid(count) * count),
    }
}

/// Open time of the bucket after the one that contains `t_ms`.
pub fn next_bucket_start_ms(tf: Timeframe, t_ms: i64) -> i64 {
    let count = i64::from(tf.count());
    let start = bucket_start_ms(tf, t_ms);
    match tf.unit() {
        // The last bucket of a day is cut short at midnight.
        TimeUnit::Second | TimeUnit::Minute | TimeUnit::Hour => {
            let midnight = start.div_euclid(DAY_MS) * DAY_MS;
            (start + tf.nominal_ms()).min(midnight + DAY_MS)
        }
        TimeUnit::Day => start + count * DAY_MS,
        TimeUnit::Week => start + count * 7 * DAY_MS,
        TimeUnit::Month => month_start_ms(month_index(start.div_euclid(DAY_MS)) + count),
    }
}

/// Months since 1970-01 of the month holding `days` (since the epoch).
fn month_index(days: i64) -> i64 {
    let (year, month, _) = civil_from_days(days);
    (year - 1970) * 12 + month - 1
}

/// Midnight UTC on the first of the month `index` months after 1970-01.
fn month_start_ms(index: i64) -> i64 {
    days_from_civil(1970 + index.div_euclid(12), index.rem_euclid(12) + 1, 1) * DAY_MS
}

/// Whether every bucket of `base` lies inside one bucket of `target`, and every bucket of `target`
/// is whole buckets of `base`.
fn nests_in(base: Timeframe, target: Timeframe) -> bool {
    let (base_count, target_count) = (base.count(), target.count());
    match (base.unit(), target.unit()) {
        (
            TimeUnit::Second | TimeUnit::Minute | TimeUnit::Hour,
            TimeUnit::Second | TimeUnit::Minute | TimeUnit::Hour,
        ) => DAY_MS % base.nominal_ms() == 0 && target.nominal_ms() % base.nominal_ms() == 0,
        (TimeUnit::Second | TimeUnit::Minute | TimeUnit::Hour, _) => {
            DAY_MS % base.nominal_ms() == 0
        }
        (TimeUnit::Day, TimeUnit::Day) => target_count % base_count == 0,
        (TimeUnit::Day, TimeUnit::Week | TimeUnit::Month) => base_count == 1,
        (TimeUnit::Week, TimeUnit::Week) | (TimeUnit::Month, TimeUnit::Month) => {
            target_count % base_count == 0
        }
        _ => false,
    }
}

/// Insert a flat, volume-less candle (at the previous close) for every interval a source skipped
/// between candles, and up to the interval holding `now_ms`, so bars are evenly spaced like an
/// exchange's. Intervals are `step_ms` wide and aligned to the epoch (hours, days).
pub fn fill_gaps(candles: Vec<Kline>, step_ms: i64, now_ms: i64) -> Vec<Kline> {
    if step_ms <= 0 || candles.is_empty() {
        return candles;
    }
    let mut out = Vec::with_capacity(candles.len());
    let mut filled = 0;
    for candle in candles {
        if let Some(previous) = out.last().cloned() {
            fill_between(&mut out, &previous, candle.open_time, step_ms, &mut filled);
        }
        out.push(candle);
    }
    let current = now_ms.div_euclid(step_ms) * step_ms;
    if let Some(last) = out.last().cloned() {
        fill_between(&mut out, &last, current + step_ms, step_ms, &mut filled);
    }
    out
}

/// Flat candles after `previous`, one per `step_ms`, strictly before `until`.
fn fill_between(
    out: &mut Vec<Kline>,
    previous: &Kline,
    until: i64,
    step_ms: i64,
    filled: &mut i64,
) {
    let missing = (until - previous.open_time) / step_ms - 1;
    if missing <= 0 || *filled + missing > MAX_FILLED {
        return;
    }
    *filled += missing;
    for index in 1..=missing {
        let open_time = previous.open_time + index * step_ms;
        out.push(Kline {
            open_time,
            close_time: open_time + step_ms - 1,
            open: previous.close,
            high: previous.close,
            low: previous.close,
            close: previous.close,
            volume: 0.0,
        });
    }
}

#[cfg(test)]
mod tests;
