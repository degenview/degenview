//! Live candles from a socket and how they meet the REST snapshots that refresh a chart.
//!
//! The rules are pure so the chart and every compared market fold alike: the workspace owns the
//! feed and the clock.

use super::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::{Kline, Timeframe};
use std::collections::BTreeMap;
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

/// Folds the live klines of a source's own (base) candle length into the series of a coarser
/// timeframe, the way the source's candles of that length fold: a `22m` chart takes the
/// one-minute stream.
///
/// A kline is a running total of its bar, sent over and over until the bar closes, so the forming
/// bucket is rebuilt from the latest state of each base bar it holds, never added to. What the
/// socket did not see of the bucket (the bars before it connected) is what the series held of it
/// when its first kline arrived, so the bucket keeps its open, its range and its volume.
///
/// How much of a live bar that held candle already counts depends on when it was requested: a bar
/// that opened after the request is not in it and counts in full, one that opened before is in it
/// up to the volume it had when its first kline arrived, so only what it traded since counts.
#[derive(Debug, Default)]
pub struct LiveFold {
    forming: Option<Forming>,
    /// When the REST answer the series now holds was requested, in epoch milliseconds.
    as_of: Option<i64>,
}

/// The bucket being built.
#[derive(Debug)]
struct Forming {
    target: Timeframe,
    open_time: i64,
    /// The candle the series held for this bucket before the first live kline of it.
    held: Option<Kline>,
    /// When `held` was requested, in epoch milliseconds, if known: a base bar that opened at or
    /// after it is not in `held`.
    cutoff: Option<i64>,
    /// The volume of each live base bar that `held` already counts.
    baseline: BTreeMap<i64, f64>,
    /// The latest state of each live base bar of the bucket.
    bars: BTreeMap<i64, Kline>,
}

impl Forming {
    fn begin(target: Timeframe, open_time: i64, held: Option<&Kline>, cutoff: Option<i64>) -> Self {
        Self {
            target,
            open_time,
            held: held.filter(|held| held.open_time == open_time).cloned(),
            cutoff,
            baseline: BTreeMap::new(),
            bars: BTreeMap::new(),
        }
    }

    /// Takes in the state of a live base bar. When it is the first kline of a bar that `held`
    /// already counts, what it had traded by then is not counted again. With no cutoff known, the
    /// only such bar is the one open when the socket connected: the bucket's first.
    fn take(&mut self, kline: Kline) {
        if self.held.is_some() && !self.bars.contains_key(&kline.open_time) {
            let in_held = match self.cutoff {
                Some(cutoff) => kline.open_time < cutoff,
                None => self.bars.is_empty(),
            };
            if in_held {
                self.baseline.insert(kline.open_time, kline.volume);
            }
        }
        self.bars.insert(kline.open_time, kline);
    }

    /// Makes `held`, a REST candle of this bucket requested at `as_of`, the candle the live bars
    /// join, for a bucket the socket began before the first answer came: each bar that opened
    /// before the request is in it up to the volume it has now.
    fn adopt(&mut self, held: Kline, as_of: i64) {
        self.baseline = self
            .bars
            .values()
            .filter(|bar| bar.open_time < as_of)
            .map(|bar| (bar.open_time, bar.volume))
            .collect();
        self.held = Some(held);
        self.cutoff = Some(as_of);
    }

    fn is_of(&self, target: Timeframe, open_time: i64) -> bool {
        self.target == target && self.open_time == open_time
    }

    /// The bucket as a candle: its bars folded, joined with what was held before them.
    fn candle(&self) -> Option<Kline> {
        let mut bars = self.bars.values();
        let first = bars.next()?;
        let mut candle = Kline {
            open_time: self.open_time,
            close_time: next_bucket_start_ms(self.target, self.open_time) - 1,
            ..first.clone()
        };
        for bar in bars {
            candle.absorb(bar);
        }
        if let Some(held) = &self.held {
            candle.open = held.open;
            candle.high = candle.high.max(held.high);
            candle.low = candle.low.min(held.low);
            candle.volume += held.volume - self.baseline.values().sum::<f64>();
        }
        Some(candle)
    }
}

impl LiveFold {
    /// Forgets the bucket in progress: the series now holds a fresher account of it than the
    /// socket's bars (a refresh replaced it), or is another market's or timeframe's.
    pub fn reset(&mut self) {
        self.forming = None;
        self.as_of = None;
    }

    /// Merges a REST answer into `current`, the candles held, as [`merge_fetched`] does, and
    /// tells the fold what the answer is. `as_of_ms` is when it was requested, if known.
    ///
    /// An answer no socket bar is newer than (`live_open` is `None`) replaces the forming bucket,
    /// so the fold starts it again. When the socket's bars began a bucket before any answer came,
    /// the fold holds only what the socket saw of it: the answer's candle of that bucket becomes
    /// what the bucket was before. A bucket that already has such a candle is left as it is, so
    /// refreshes do not move it.
    pub fn merge_fetched(
        &mut self,
        fetched: Vec<Kline>,
        current: &[Kline],
        live_open: Option<i64>,
        as_of_ms: Option<i64>,
        limit: usize,
    ) -> Vec<Kline> {
        if live_open.is_none() {
            self.reset();
            self.as_of = as_of_ms;
            return merge_fetched(fetched, current, None, limit);
        }
        let adoptable = match (&mut self.forming, as_of_ms) {
            (Some(forming), Some(as_of)) if forming.held.is_none() => fetched
                .iter()
                .find(|candle| candle.open_time == forming.open_time)
                .map(|candle| (forming, candle.clone(), as_of)),
            _ => None,
        };
        let Some((forming, held, as_of)) = adoptable else {
            return merge_fetched(fetched, current, live_open, limit);
        };
        forming.adopt(held, as_of);
        let open_time = forming.open_time;
        let candle = forming.candle();
        self.as_of = as_of_ms;
        let mut merged = merge_fetched(fetched, current, live_open, limit);
        if let Some(candle) = candle
            && let Some(slot) = merged
                .iter_mut()
                .rfind(|merged| merged.open_time == open_time)
        {
            *slot = candle;
        }
        merged
    }

    /// Folds `kline`, a live kline of `base` candles, into `candles`, the series of `target`:
    /// the forming bucket's candle replaces the newest candle of its open time or follows it. A
    /// kline of a bucket older than the newest candle is ignored, so the series stays ascending.
    /// Returns the open time of the bucket that changed, or `None` when nothing did.
    ///
    /// When `base` is the target, the kline is the candle itself and replaces it as it is.
    pub fn apply(
        &mut self,
        target: Timeframe,
        base: Timeframe,
        candles: &mut Vec<Kline>,
        kline: Kline,
    ) -> Option<i64> {
        if base == target {
            self.forming = None;
            let open_time = kline.open_time;
            return apply_live(candles, kline).then_some(open_time);
        }
        let open_time = bucket_start_ms(target, kline.open_time);
        if candles
            .last()
            .is_some_and(|last| last.open_time > open_time)
        {
            return None;
        }
        let mut forming = match self.forming.take() {
            Some(forming) if forming.is_of(target, open_time) => forming,
            _ => Forming::begin(target, open_time, candles.last(), self.as_of),
        };
        forming.take(kline);
        let candle = forming.candle();
        self.forming = Some(forming);
        apply_live(candles, candle?).then_some(open_time)
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
