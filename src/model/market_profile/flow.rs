//! Lower-timeframe candles with the exchange's own taker-buy volume.
//!
//! [`FlowKline`] is a [`Kline`] plus the base volume that was bought by takers (Binance kline column
//! 9), which gives an **exact** buy/sell split per candle where the candle alone gives none. It is a
//! side struct on purpose: `Kline` is used everywhere (caches, sync, Pine) and gains nothing from it.
//!
//! [`CandleBook`] keeps one market's candles at one interval, deduplicated by open time (the newest
//! delivery of a candle wins, so a forming candle updates in place) and bounded.

use crate::model::kline::Kline;
use std::collections::BTreeMap;

/// Candles kept by default: the most a profile fetch asks for (`MAX_INTRABAR_CANDLES`).
pub const DEFAULT_CAPACITY: usize = 60_000;

#[derive(Clone, Debug, PartialEq)]
pub struct FlowKline {
    pub kline: Kline,
    /// Base volume bought by takers, when the exchange says; never more than the candle's volume.
    pub taker_buy: Option<f64>,
}

impl FlowKline {
    /// A candle with no exchange split.
    pub fn plain(kline: Kline) -> Self {
        Self {
            kline,
            taker_buy: None,
        }
    }

    /// The exchange's buy and sell volume of the candle, when it gave a split.
    pub fn split(&self) -> Option<(f64, f64)> {
        let buy = self.taker_buy?;
        Some((buy, (self.kline.volume - buy).max(0.0)))
    }

    /// A candle a profile can use: finite prices with `low <= high`, finite non-negative volume.
    pub fn is_valid(&self) -> bool {
        let k = &self.kline;
        [k.open, k.high, k.low, k.close]
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
            && k.low <= k.high
            && k.volume.is_finite()
            && k.volume >= 0.0
    }

    /// The same candle with an impossible taker-buy volume (not a number, negative) dropped and one
    /// above the candle's volume clamped to it.
    fn sanitized(mut self) -> Self {
        self.taker_buy = self
            .taker_buy
            .filter(|buy| buy.is_finite() && *buy >= 0.0)
            .map(|buy| buy.min(self.kline.volume));
        self
    }
}

#[derive(Clone, Debug)]
pub struct CandleBook {
    interval_ms: i64,
    capacity: usize,
    candles: BTreeMap<i64, FlowKline>,
}

impl CandleBook {
    /// A book of candles `interval_ms` long keeping at most `capacity` (at least one).
    pub fn new(interval_ms: i64, capacity: usize) -> Self {
        Self {
            interval_ms: interval_ms.max(1),
            capacity: capacity.max(1),
            candles: BTreeMap::new(),
        }
    }

    pub fn interval_ms(&self) -> i64 {
        self.interval_ms
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.candles.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.candles.is_empty()
    }

    pub fn oldest_open(&self) -> Option<i64> {
        self.candles.keys().next().copied()
    }

    pub fn newest_open(&self) -> Option<i64> {
        self.candles.keys().next_back().copied()
    }

    /// Adds candles; a candle with an open time already held replaces it. Invalid candles are
    /// skipped. Returns how many were stored. The oldest go when the book is over capacity.
    pub fn insert(&mut self, incoming: impl IntoIterator<Item = FlowKline>) -> usize {
        let mut stored = 0;
        for candle in incoming {
            let candle = candle.sanitized();
            if !candle.is_valid() {
                continue;
            }
            self.candles.insert(candle.kline.open_time, candle);
            stored += 1;
        }
        while self.candles.len() > self.capacity {
            self.candles.pop_first();
        }
        stored
    }

    /// Like [`Self::insert`], but when the book is over capacity the **newest** go: for older history
    /// arriving into a full book, whose oldest candles are the ones just fetched.
    pub fn insert_evicting_newest(
        &mut self,
        incoming: impl IntoIterator<Item = FlowKline>,
    ) -> usize {
        let before = self.capacity;
        // Let the insert run unbounded, then trim from the other end.
        self.capacity = usize::MAX;
        let stored = self.insert(incoming);
        self.capacity = before;
        while self.candles.len() > self.capacity {
            self.candles.pop_last();
        }
        stored
    }

    /// The candles opening in `start_ms <= open_time < end_ms`, oldest first.
    pub fn between(&self, start_ms: i64, end_ms: i64) -> impl Iterator<Item = &FlowKline> {
        self.candles
            .range(start_ms..end_ms.max(start_ms))
            .map(|(_, candle)| candle)
    }

    #[cfg(test)]
    /// Open times in `[start_ms, end_ms)` (aligned to the interval) for which no candle is held:
    /// where a profile has a hole. Bounded to `limit` entries.
    pub fn missing(&self, start_ms: i64, end_ms: i64, limit: usize) -> Vec<i64> {
        let first = start_ms.div_euclid(self.interval_ms) * self.interval_ms;
        let mut missing = Vec::new();
        let mut open = first;
        while open < end_ms && missing.len() < limit {
            if open + self.interval_ms > start_ms && !self.candles.contains_key(&open) {
                missing.push(open);
            }
            open += self.interval_ms;
        }
        missing
    }

    #[cfg(test)]
    /// Drops every candle opening before `time_ms`.
    pub fn forget_before(&mut self, time_ms: i64) {
        self.candles = self.candles.split_off(&time_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: i64 = 60_000;

    fn candle(open_time: i64, close: f64, volume: f64, taker_buy: Option<f64>) -> FlowKline {
        FlowKline {
            kline: Kline {
                open_time,
                close_time: open_time + MINUTE - 1,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close,
                volume,
            },
            taker_buy,
        }
    }

    #[test]
    fn the_exchange_split_never_exceeds_the_candles_volume() {
        let split = candle(0, 100.0, 10.0, Some(4.0)).split().unwrap();
        assert_eq!(split, (4.0, 6.0));
        let mut book = CandleBook::new(MINUTE, 10);
        book.insert([candle(0, 100.0, 10.0, Some(12.0))]);
        let held = book.between(0, MINUTE).next().unwrap();
        assert_eq!(held.taker_buy, Some(10.0));
        assert_eq!(held.split(), Some((10.0, 0.0)));
        assert_eq!(candle(0, 100.0, 10.0, None).split(), None);
    }

    #[test]
    fn an_impossible_taker_buy_is_dropped_not_trusted() {
        let mut book = CandleBook::new(MINUTE, 10);
        book.insert([
            candle(0, 100.0, 10.0, Some(f64::NAN)),
            candle(MINUTE, 100.0, 10.0, Some(-3.0)),
        ]);
        assert!(book.between(0, 2 * MINUTE).all(|c| c.taker_buy.is_none()));
    }

    #[test]
    fn invalid_candles_are_skipped() {
        let mut book = CandleBook::new(MINUTE, 10);
        let mut inverted = candle(0, 100.0, 1.0, None);
        inverted.kline.low = 102.0;
        let mut nan = candle(MINUTE, 100.0, 1.0, None);
        nan.kline.close = f64::NAN;
        let negative = candle(2 * MINUTE, 100.0, -1.0, None);
        assert_eq!(book.insert([inverted, nan, negative]), 0);
        assert!(book.is_empty());
    }

    #[test]
    fn the_newest_delivery_of_a_candle_replaces_the_older() {
        let mut book = CandleBook::new(MINUTE, 10);
        book.insert([candle(0, 100.0, 5.0, None)]);
        book.insert([candle(0, 100.5, 9.0, Some(3.0))]);
        assert_eq!(book.len(), 1);
        let held = book.between(0, MINUTE).next().unwrap();
        assert_eq!(held.kline.volume, 9.0);
        assert_eq!(held.kline.close, 100.5);
    }

    #[test]
    fn the_oldest_candles_go_when_the_book_is_full() {
        let mut book = CandleBook::new(MINUTE, 3);
        book.insert((0..5).map(|i| candle(i * MINUTE, 100.0, 1.0, None)));
        assert_eq!(book.len(), 3);
        assert_eq!(book.oldest_open(), Some(2 * MINUTE));
        assert_eq!(book.newest_open(), Some(4 * MINUTE));
    }

    #[test]
    fn between_is_half_open_and_ordered() {
        let mut book = CandleBook::new(MINUTE, 10);
        book.insert([3, 1, 2, 0].map(|i| candle(i * MINUTE, 100.0, 1.0, None)));
        let opens: Vec<i64> = book
            .between(MINUTE, 3 * MINUTE)
            .map(|c| c.kline.open_time)
            .collect();
        assert_eq!(opens, [MINUTE, 2 * MINUTE]);
        assert_eq!(book.between(3 * MINUTE, MINUTE).count(), 0);
    }

    #[test]
    fn holes_are_reported_aligned_and_bounded() {
        let mut book = CandleBook::new(MINUTE, 10);
        book.insert([0, 1, 4].map(|i| candle(i * MINUTE, 100.0, 1.0, None)));
        assert_eq!(
            book.missing(0, 6 * MINUTE, 10),
            [2 * MINUTE, 3 * MINUTE, 5 * MINUTE]
        );
        assert_eq!(book.missing(0, 6 * MINUTE, 2), [2 * MINUTE, 3 * MINUTE]);
        assert!(book.missing(0, 2 * MINUTE, 10).is_empty());
    }

    #[test]
    fn forgetting_by_time_keeps_the_rest() {
        let mut book = CandleBook::new(MINUTE, 10);
        book.insert((0..5).map(|i| candle(i * MINUTE, 100.0, 1.0, None)));
        book.forget_before(3 * MINUTE);
        assert_eq!(book.oldest_open(), Some(3 * MINUTE));
        assert_eq!(book.len(), 2);
    }

    #[test]
    fn older_history_into_a_full_book_keeps_what_was_just_fetched() {
        let mut book = CandleBook::new(MINUTE, 5);
        book.insert((10..15).map(|i| candle(i * MINUTE, 100.0, 1.0, None)));
        // Inserting older candles the usual way drops them: they are the oldest.
        let mut usual = book.clone();
        usual.insert((5..10).map(|i| candle(i * MINUTE, 100.0, 1.0, None)));
        assert_eq!(usual.oldest_open(), Some(10 * MINUTE));
        // The older-history insert keeps them and drops the newest instead.
        book.insert_evicting_newest((5..10).map(|i| candle(i * MINUTE, 100.0, 1.0, None)));
        assert_eq!(book.len(), 5);
        assert_eq!(book.oldest_open(), Some(5 * MINUTE));
        assert_eq!(book.newest_open(), Some(9 * MINUTE));
    }
}
