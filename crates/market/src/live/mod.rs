//! Live candles from a socket and how they meet the REST snapshots that refresh a chart.
//!
//! The rules are pure so the chart and every compared market fold alike: the workspace owns the
//! feed and the clock.

use crate::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::{Kline, Timeframe};
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
/// when its first kline arrived, so the bucket keeps its open, its range and its volume. The one
/// bar that was open then is already in that figure, so only what it traded since counts. When the
/// REST answer brings the bucket's own base bars ([`LiveFold::merge_fetched`]), the bucket is those
/// bars instead, and the held candle is not needed.
#[derive(Debug, Default)]
pub struct LiveFold {
    forming: Option<Forming>,
}

/// The bucket being built.
#[derive(Debug)]
struct Forming {
    target: Timeframe,
    open_time: i64,
    /// The candle the series held for this bucket before the first live kline of it.
    held: Option<Kline>,
    /// The volume of that first kline, already in `held`.
    first_volume: f64,
    /// The latest state of each live base bar of the bucket.
    bars: BTreeMap<i64, Kline>,
}

impl Forming {
    fn begin(target: Timeframe, open_time: i64, held: Option<&Kline>, first: &Kline) -> Self {
        Self {
            target,
            open_time,
            held: held.filter(|held| held.open_time == open_time).cloned(),
            first_volume: first.volume,
            bars: BTreeMap::new(),
        }
    }

    /// A bucket made of base bars alone, the way REST brought them.
    fn from_bars(target: Timeframe, open_time: i64, bars: Vec<Kline>) -> Self {
        let mut forming = Self {
            target,
            open_time,
            held: None,
            first_volume: 0.0,
            bars: BTreeMap::new(),
        };
        for bar in bars {
            forming.take(bar);
        }
        forming
    }

    fn is_of(&self, target: Timeframe, open_time: i64) -> bool {
        self.target == target && self.open_time == open_time
    }

    /// Takes in the state of a base bar, from the socket or from REST. A bar's volume is a running
    /// total that only grows, so the state with more of it is the later one and replaces what is
    /// stored; one with less is an older account and is dropped. On a tie the arriving state wins.
    fn take(&mut self, bar: Kline) {
        match self.bars.get(&bar.open_time) {
            Some(stored) if bar.volume < stored.volume => {}
            _ => {
                self.bars.insert(bar.open_time, bar);
            }
        }
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
            candle.volume += held.volume - self.first_volume;
        }
        Some(candle)
    }
}

impl LiveFold {
    /// Forgets the bucket in progress: the series now holds a fresher account of it than the
    /// socket's bars (a refresh replaced it), or is another market's or timeframe's.
    pub fn reset(&mut self) {
        self.forming = None;
    }

    /// Merges a REST answer into `current`, the candles held, as [`merge_fetched`] does, and
    /// tells the fold what the answer is. `bars` are the base candles of the answer's newest
    /// bucket of `target`, fetched on their own (empty when the source has the target itself or
    /// the request failed).
    ///
    /// An answer no socket bar is newer than (`live_open` is `None`) replaces the forming bucket,
    /// so the fold starts it again. With `bars`, the forming bucket is those bars merged with the
    /// socket's, bar by bar (the state with more volume wins), so what the socket missed before
    /// it connected, or while it was quiet, is not lost and nothing is counted twice; a bucket
    /// the socket has not reached yet starts from them, and one newer than them is left alone.
    /// The merged series then takes the forming candle in place of its newest candle, never an
    /// older one: REST's finished copy of an earlier bucket stays. With no `bars` the held-candle
    /// rule of [`apply`](Self::apply) stands in.
    pub fn merge_fetched(
        &mut self,
        target: Timeframe,
        fetched: Vec<Kline>,
        bars: Vec<Kline>,
        current: &[Kline],
        live_open: Option<i64>,
        limit: usize,
    ) -> Vec<Kline> {
        if live_open.is_none() {
            self.reset();
        }
        let bucket = bars
            .last()
            .map(|bar| bucket_start_ms(target, bar.open_time));
        if let Some(bucket) = bucket {
            match &mut self.forming {
                Some(forming) if forming.is_of(target, bucket) => {
                    for bar in bars {
                        forming.take(bar);
                    }
                    forming.held = None;
                }
                Some(forming) if forming.target == target && forming.open_time > bucket => {}
                _ => self.forming = Some(Forming::from_bars(target, bucket, bars)),
            }
        }
        let mut merged = merge_fetched(fetched, current, live_open, limit);
        if let Some(bucket) = bucket
            && let Some(forming) = self.forming.as_ref().filter(|f| f.is_of(target, bucket))
            && merged.last().is_none_or(|last| last.open_time <= bucket)
            && let Some(candle) = forming.candle()
        {
            apply_live(&mut merged, candle);
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
            _ => Forming::begin(target, open_time, candles.last(), &kline),
        };
        forming.take(kline);
        let candle = forming.candle();
        self.forming = Some(forming);
        apply_live(candles, candle?).then_some(open_time)
    }
}

/// The newest bar a socket delivered, and when: what a REST refresh must not overwrite.
///
/// `T` is the caller's monotonic clock reading. The app uses `Instant`; a build where `Instant`
/// is unavailable (the browser) passes its own, such as `performance.now()` as an `f64`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveMark<T = Instant> {
    open_time: i64,
    arrived: T,
}

impl<T: Copy + PartialOrd> LiveMark<T> {
    pub fn new(open_time: i64, arrived: T) -> Self {
        Self { open_time, arrived }
    }

    /// The open time a refresh must keep held bars from, for a request begun at
    /// `request_started`. A socket that went quiet before the request began is not fresher than
    /// the answer, so the snapshot may update the forming bar again.
    pub fn guard_for(&self, request_started: T) -> Option<i64> {
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
