//! Live candles from a socket and how they meet the REST snapshots that refresh a chart.
//!
//! The rules are pure so the chart and every compared market fold alike: the workspace owns the
//! feed and the clock.

use crate::model::Kline;
use std::time::Instant;

/// Folds a live kline into `candles`: it replaces the newest candle of its open time or follows
/// it. A kline older than the newest candle (a late final tick of the bar before one REST already
/// brought) is ignored, so the series stays ascending. Returns whether the candles changed.
pub fn apply_live(candles: &mut Vec<Kline>, kline: Kline) -> bool {
    match candles.last_mut() {
        Some(last) if last.open_time == kline.open_time => {
            let changed = *last != kline;
            *last = kline;
            changed
        }
        Some(last) if last.open_time > kline.open_time => false,
        _ => {
            candles.push(kline);
            true
        }
    }
}

/// The newest bar a socket delivered, and when: what a REST refresh must not overwrite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveMark {
    open_time: i64,
    arrived: Instant,
}

impl LiveMark {
    pub fn new(open_time: i64, arrived: Instant) -> Self {
        Self { open_time, arrived }
    }

    /// The open time a refresh must keep held bars from, for a request begun at
    /// `request_started`. A socket that went quiet before the request began is not fresher than
    /// the answer, so the snapshot may update the forming bar again.
    pub fn guard_for(&self, request_started: Instant) -> Option<i64> {
        (self.arrived >= request_started).then_some(self.open_time)
    }
}

/// A REST snapshot merged over the candles held: bars the socket delivered at or after
/// `live_open_time` (from [`LiveMark::guard_for`]) win over the snapshot's copy of them. Keeps the
/// newest `limit` candles.
pub fn merge_fetched(
    mut fetched: Vec<Kline>,
    current: &[Kline],
    live_open_time: Option<i64>,
    limit: usize,
) -> Vec<Kline> {
    if let Some(live_open) = live_open_time
        && let Some(last_fetched) = fetched.last()
    {
        let newer = current
            .partition_point(|candle| candle.open_time < live_open.max(last_fetched.open_time));
        if newer < current.len() {
            let first_newer = current[newer].open_time;
            fetched.retain(|candle| candle.open_time < first_newer);
            fetched.extend_from_slice(&current[newer..]);
        }
    }
    if fetched.len() > limit {
        fetched.drain(..fetched.len() - limit);
    }
    fetched
}

#[cfg(test)]
mod tests;
