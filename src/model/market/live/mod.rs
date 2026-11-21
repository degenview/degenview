//! Live candles from a socket and how they meet the REST snapshots that refresh a chart.
//!
//! The rules are pure so the chart and every compared market fold alike: the workspace owns the
//! feed and the clock.

use super::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::{Kline, Timeframe};
use std::collections::BTreeMap;
use std::time::Instant;

mod trades;

pub use trades::{Trade, TradeFold};

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

/// What the klines of a live feed are to the bars they name.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Klines {
    /// Each is the bar so far, whole, as a venue's candle stream sends it.
    #[default]
    Bars,
    /// Each is built from trades ([`TradeFold`]): it holds what traded since the socket connected,
    /// not the bar, so it adds to the candle the chart holds of that bar (REST's account of it)
    /// instead of replacing it.
    Trades,
}

/// What a live feed sends: klines of `base` length, each of the kind `klines`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stream {
    pub base: Timeframe,
    pub klines: Klines,
}

#[cfg(test)]
impl Stream {
    /// A stream of whole bars, as a venue's candle channel sends them.
    pub fn bars(base: Timeframe) -> Self {
        Self {
            base,
            klines: Klines::Bars,
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
///
/// A stream of [`Klines::Trades`] is folded the same way even when its base is the target: its
/// klines are increments on the held candle, whose open and range they must keep, never the candle
/// itself. A REST answer then becomes the held candle again ([`LiveFold::merge_fetched`]): the
/// trades it already counts (those seen by the time it arrived) are not added to it twice.
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
    klines: Klines,
}

impl Forming {
    fn begin(
        target: Timeframe,
        open_time: i64,
        held: Option<&Kline>,
        first: &Kline,
        klines: Klines,
    ) -> Self {
        Self {
            target,
            open_time,
            held: held.filter(|held| held.open_time == open_time).cloned(),
            first_volume: first.volume,
            bars: BTreeMap::new(),
            klines,
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
            klines: Klines::Bars,
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

    /// The volume of the live bars of the bucket.
    fn traded(&self) -> f64 {
        self.bars.values().map(|bar| bar.volume).sum()
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

    /// A REST answer for the bucket the trades of a [`Klines::Trades`] stream are adding to: its
    /// account of the bucket (folded from `bars`, or the target's own candle in `fetched`) becomes
    /// the held candle, and the trades seen so far count as already in it, so the klines that
    /// follow add only what traded after. Returns the bucket's open time; `None` when no trade
    /// stream is forming, or the answer says nothing of its bucket.
    fn reseed(&mut self, target: Timeframe, fetched: &[Kline], bars: &[Kline]) -> Option<i64> {
        let forming = self
            .forming
            .as_mut()
            .filter(|forming| forming.target == target && forming.klines == Klines::Trades)?;
        let account = match bars.last() {
            Some(last) if bucket_start_ms(target, last.open_time) == forming.open_time => {
                Forming::from_bars(target, forming.open_time, bars.to_vec()).candle()
            }
            Some(_) => None,
            None => fetched
                .iter()
                .rev()
                .find(|candle| candle.open_time == forming.open_time)
                .cloned(),
        }?;
        forming.held = Some(account);
        forming.first_volume = forming.traded();
        Some(forming.open_time)
    }

    /// Merges a REST answer into `current`, the candles held, as [`merge_fetched`] does, and
    /// tells the fold what the answer is. `bars` are the base candles of the answer's newest
    /// bucket of `target`, fetched on their own (empty when the source has the target itself or
    /// the request failed).
    ///
    /// An answer no socket bar is newer than (`live_open` is `None`) replaces the forming bucket,
    /// so the fold starts it again (a trade stream's klines are a running total: the fold keeps
    /// them, and the answer is the candle they add to). With `bars`, the forming bucket is those bars merged with the
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
        let quiet = live_open.is_none();
        // The klines of a trade stream are a running total the answer cannot void: the fold keeps
        // them and counts them as already in the answer.
        if quiet
            && self
                .forming
                .as_ref()
                .is_none_or(|f| f.klines == Klines::Bars)
        {
            self.reset();
        }
        let reseeded = self.reseed(target, &fetched, &bars);
        let bucket = match reseeded {
            Some(open_time) => Some(open_time),
            None => {
                let bucket = bars
                    .last()
                    .map(|bar| bucket_start_ms(target, bar.open_time));
                if let Some(bucket) = bucket {
                    match &mut self.forming {
                        Some(forming)
                            if forming.is_of(target, bucket) && forming.klines == Klines::Bars =>
                        {
                            for bar in bars {
                                forming.take(bar);
                            }
                            forming.held = None;
                        }
                        Some(forming) if forming.target == target && forming.open_time > bucket => {
                        }
                        _ => self.forming = Some(Forming::from_bars(target, bucket, bars)),
                    }
                }
                bucket
            }
        };
        let mut merged = merge_fetched(fetched, current, live_open, limit);
        // An answer no socket kline is newer than is the newest account of the bar, close included.
        if let Some(bucket) = bucket
            && !(quiet && reseeded.is_some())
            && let Some(forming) = self.forming.as_ref().filter(|f| f.is_of(target, bucket))
            && merged.last().is_none_or(|last| last.open_time <= bucket)
            && let Some(candle) = forming.candle()
        {
            apply_live(&mut merged, candle);
        }
        merged
    }

    /// Folds `kline`, a live kline of `stream`, into `candles`, the series of `target`: the
    /// forming bucket's candle replaces the newest candle of its open time or follows it. A kline
    /// of a bucket older than the newest candle is ignored, so the series stays ascending. Returns
    /// the open time of the bucket that changed, or `None` when nothing did.
    ///
    /// When the stream's base is the target and its klines are whole bars, the kline is the candle
    /// itself and replaces it as it is.
    pub fn apply(
        &mut self,
        target: Timeframe,
        stream: Stream,
        candles: &mut Vec<Kline>,
        kline: Kline,
    ) -> Option<i64> {
        if stream.base == target && stream.klines == Klines::Bars {
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
            Some(forming)
                if forming.is_of(target, open_time) && forming.klines == stream.klines =>
            {
                forming
            }
            // The bucket REST made of its base bars is the candle the trades add to.
            Some(forming)
                if forming.is_of(target, open_time) && stream.klines == Klines::Trades =>
            {
                Forming::begin(
                    target,
                    open_time,
                    forming.candle().as_ref(),
                    &kline,
                    stream.klines,
                )
            }
            _ => Forming::begin(target, open_time, candles.last(), &kline, stream.klines),
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
