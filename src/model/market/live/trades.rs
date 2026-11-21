//! A live candle built from trades, for a venue that streams trades but not candles (Bitstamp).

// Nothing streams trades yet; the first venue that does uses all of it.
#![cfg_attr(not(test), allow(dead_code))]

use super::super::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::{Kline, Timeframe};

/// One executed trade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trade {
    pub time_ms: i64,
    pub price: f64,
    /// In the market's base asset, as the candle volume of the venue is.
    pub size: f64,
}

impl Trade {
    /// Whether the trade can be part of a candle: a real price and a size that is not negative.
    fn is_sound(&self) -> bool {
        self.price.is_finite() && self.price > 0.0 && self.size.is_finite() && self.size >= 0.0
    }
}

/// Folds one market's trades into the forming candle of `base` length. The candle it hands back
/// after each trade is the value a venue's own candle stream would send, so
/// [`LiveFold::apply`](super::LiveFold::apply) takes it like any other.
///
/// The open is the first trade of the bucket, the high and low and the volume run, the close is
/// the latest trade. A trade older than the bucket in progress is dropped (a late trade of a bar
/// that has closed); a trade of a newer bucket starts a candle of its own. Trades of one bucket
/// count in the order they arrive, so a trade that arrives out of order moves the close.
///
/// **Known limit.** The candle holds the trades seen since the socket connected, so the bar that
/// was open at that moment is partial: its open, its range and its volume miss what traded before.
/// The rest is complete. [`merge_fetched`](super::merge_fetched) repairs it when the bar closes
/// (REST's copy of a bar older than the live one is taken whole), and in the meantime a REST answer
/// that brings the bucket's own base bars keeps the fuller of the two for each (a bar's volume only
/// grows), which for this bar is REST's, until the trades seen catch up with it.
#[derive(Debug, Default)]
pub struct TradeFold {
    forming: Option<Forming>,
}

#[derive(Debug)]
struct Forming {
    base: Timeframe,
    candle: Kline,
}

impl TradeFold {
    /// Forgets the candle in progress: the socket reconnected, so the trades missed in between
    /// are not in it and the next one must not extend it as if they were.
    pub fn reset(&mut self) {
        self.forming = None;
    }

    /// Takes in `trade` and returns the forming candle of `base` it belongs to; `None` when the
    /// trade changed nothing (unsound, or older than the bucket in progress).
    pub fn apply(&mut self, base: Timeframe, trade: Trade) -> Option<Kline> {
        if !trade.is_sound() {
            return None;
        }
        let open_time = bucket_start_ms(base, trade.time_ms);
        match &mut self.forming {
            Some(forming) if forming.base == base && forming.candle.open_time == open_time => {
                let candle = &mut forming.candle;
                candle.high = candle.high.max(trade.price);
                candle.low = candle.low.min(trade.price);
                candle.close = trade.price;
                candle.volume += trade.size;
            }
            // A bucket the candle in progress has already left.
            Some(forming) if forming.base == base && forming.candle.open_time > open_time => {
                return None;
            }
            _ => {
                self.forming = Some(Forming {
                    base,
                    candle: Kline {
                        open_time,
                        close_time: next_bucket_start_ms(base, open_time) - 1,
                        open: trade.price,
                        high: trade.price,
                        low: trade.price,
                        close: trade.price,
                        volume: trade.size,
                    },
                });
            }
        }
        self.forming.as_ref().map(|forming| forming.candle.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::super::LiveFold;
    use super::*;

    const MINUTE: i64 = 60_000;
    /// 2024-01-01T00:00:00Z, a bucket start for every length below.
    const T0: i64 = 1_704_067_200_000;

    fn trade(time_ms: i64, price: f64, size: f64) -> Trade {
        Trade {
            time_ms,
            price,
            size,
        }
    }

    #[test]
    fn a_bucket_opens_at_its_first_trade_and_runs_its_range_close_and_volume() {
        let mut fold = TradeFold::default();
        let base = Timeframe::MINUTE_1;
        let first = fold.apply(base, trade(T0 + 5_000, 100.0, 1.0)).unwrap();
        assert_eq!(
            first,
            Kline {
                open_time: T0,
                close_time: T0 + MINUTE - 1,
                open: 100.0,
                high: 100.0,
                low: 100.0,
                close: 100.0,
                volume: 1.0,
            }
        );
        fold.apply(base, trade(T0 + 10_000, 105.0, 2.0));
        fold.apply(base, trade(T0 + 20_000, 98.0, 0.5));
        let last = fold.apply(base, trade(T0 + 59_999, 101.0, 0.5)).unwrap();
        assert_eq!(
            (last.open, last.high, last.low, last.close, last.volume),
            (100.0, 105.0, 98.0, 101.0, 4.0)
        );
    }

    #[test]
    fn a_trade_of_the_next_bucket_starts_a_fresh_candle() {
        let mut fold = TradeFold::default();
        let base = Timeframe::MINUTE_1;
        fold.apply(base, trade(T0 + 10_000, 100.0, 3.0));
        let next = fold.apply(base, trade(T0 + MINUTE, 90.0, 1.0)).unwrap();
        assert_eq!(next.open_time, T0 + MINUTE);
        assert_eq!(
            (next.open, next.high, next.low, next.close, next.volume),
            (90.0, 90.0, 90.0, 90.0, 1.0)
        );
    }

    #[test]
    fn a_trade_older_than_the_bucket_in_progress_is_dropped() {
        let mut fold = TradeFold::default();
        let base = Timeframe::MINUTE_1;
        fold.apply(base, trade(T0 + MINUTE + 1_000, 100.0, 1.0));
        assert_eq!(fold.apply(base, trade(T0 + 59_000, 500.0, 9.0)), None);
        let kept = fold
            .apply(base, trade(T0 + MINUTE + 2_000, 101.0, 1.0))
            .unwrap();
        assert_eq!((kept.high, kept.volume), (101.0, 2.0));
    }

    #[test]
    fn a_trade_that_is_not_one_changes_nothing() {
        let mut fold = TradeFold::default();
        let base = Timeframe::MINUTE_1;
        for (price, size) in [
            (f64::NAN, 1.0),
            (f64::INFINITY, 1.0),
            (0.0, 1.0),
            (-5.0, 1.0),
            (100.0, f64::NAN),
            (100.0, -1.0),
        ] {
            assert_eq!(fold.apply(base, trade(T0, price, size)), None);
        }
        assert!(fold.forming.is_none());
        // A trade of no size still prints a price.
        assert_eq!(fold.apply(base, trade(T0, 100.0, 0.0)).unwrap().volume, 0.0);
    }

    #[test]
    fn buckets_are_cut_where_the_chart_cuts_them() {
        let mut fold = TradeFold::default();
        let base = Timeframe::new(22, crate::model::TimeUnit::Minute).unwrap();
        let candle = fold.apply(base, trade(T0 + 23 * MINUTE, 1.0, 1.0)).unwrap();
        assert_eq!(candle.open_time, T0 + 22 * MINUTE);
        assert_eq!(candle.close_time, T0 + 44 * MINUTE - 1);
        // The last bucket of a day is cut short at midnight.
        let late = fold
            .apply(base, trade(T0 + 24 * 60 * MINUTE - MINUTE, 1.0, 1.0))
            .unwrap();
        assert_eq!(late.close_time, T0 + 24 * 60 * MINUTE - 1);
    }

    #[test]
    fn a_new_base_or_a_reset_forgets_the_candle_in_progress() {
        let mut fold = TradeFold::default();
        fold.apply(Timeframe::MINUTE_1, trade(T0, 100.0, 1.0));
        let other = fold
            .apply(Timeframe::MINUTE_5, trade(T0 + 1_000, 90.0, 1.0))
            .unwrap();
        assert_eq!((other.open, other.volume), (90.0, 1.0));
        fold.reset();
        let fresh = fold
            .apply(Timeframe::MINUTE_5, trade(T0 + 2_000, 95.0, 1.0))
            .unwrap();
        assert_eq!((fresh.open, fresh.volume), (95.0, 1.0));
    }

    fn rest_bar(open_time: i64, close: f64, volume: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + MINUTE - 1,
            open: 100.0,
            high: 110.0,
            low: 95.0,
            close,
            volume,
        }
    }

    /// A chart of a coarser length takes the trade-built minutes through `LiveFold` like a venue's
    /// own candles: the bucket is the minutes' fold.
    #[test]
    fn trade_built_minutes_fold_into_a_coarser_chart() {
        let mut trades = TradeFold::default();
        let mut live = LiveFold::default();
        let mut candles = Vec::new();
        for (offset, price, size) in [
            (0, 100.0, 1.0),
            (30_000, 104.0, 1.0),
            (MINUTE + 5_000, 96.0, 2.0),
            (2 * MINUTE, 99.0, 1.0),
        ] {
            let kline = trades
                .apply(Timeframe::MINUTE_1, trade(T0 + offset, price, size))
                .unwrap();
            live.apply(
                Timeframe::MINUTE_5,
                Timeframe::MINUTE_1,
                &mut candles,
                kline,
            );
        }
        assert_eq!(candles.len(), 1);
        let bucket = &candles[0];
        assert_eq!(bucket.open_time, T0);
        assert_eq!(
            (
                bucket.open,
                bucket.high,
                bucket.low,
                bucket.close,
                bucket.volume
            ),
            (100.0, 104.0, 96.0, 99.0, 5.0)
        );
    }

    /// Connected before the minute began, the trade-built bar is as full as REST's, and the REST
    /// answer that carries the bucket's own minutes changes nothing but what the trades missed.
    #[test]
    fn a_complete_trade_built_bar_and_rests_copy_of_it_agree() {
        let mut trades = TradeFold::default();
        let mut live = LiveFold::default();
        let mut candles = vec![rest_bar(T0 - MINUTE, 100.0, 7.0)];
        let mut last = None;
        for (offset, price, size) in [(0, 100.0, 1.0), (20_000, 103.0, 1.0), (40_000, 101.0, 1.0)] {
            last = trades.apply(Timeframe::MINUTE_1, trade(T0 + offset, price, size));
            live.apply(
                Timeframe::MINUTE_5,
                Timeframe::MINUTE_1,
                &mut candles,
                last.clone().unwrap(),
            );
        }
        let last = last.unwrap();
        // REST's minute holds the same trades, so the larger (here equal) state is kept.
        let merged = live.merge_fetched(
            Timeframe::MINUTE_5,
            vec![rest_bar(T0 - 5 * MINUTE, 99.0, 50.0)],
            vec![Kline {
                open: last.open,
                high: last.high,
                low: last.low,
                close: last.close,
                volume: last.volume,
                ..rest_bar(T0, 0.0, 0.0)
            }],
            &candles,
            Some(T0),
            500,
        );
        let bucket = merged.last().unwrap();
        assert_eq!(
            (
                bucket.open,
                bucket.high,
                bucket.low,
                bucket.close,
                bucket.volume
            ),
            (100.0, 103.0, 100.0, 101.0, 3.0)
        );
    }

    /// Connected in the middle of a minute, the trade-built bar misses what traded before. A REST
    /// answer that brings that minute has the larger volume and is kept (the bar is complete again),
    /// and the trades that follow take over once their total passes it.
    #[test]
    fn rest_fills_the_part_of_the_connect_minute_the_trades_missed() {
        let mut trades = TradeFold::default();
        let mut live = LiveFold::default();
        let mut candles = Vec::new();
        let kline = trades
            .apply(Timeframe::MINUTE_1, trade(T0 + 40_000, 101.0, 1.0))
            .unwrap();
        live.apply(
            Timeframe::MINUTE_5,
            Timeframe::MINUTE_1,
            &mut candles,
            kline,
        );

        // REST saw the whole minute so far: 5 units, a wider range, a different open.
        let merged = live.merge_fetched(
            Timeframe::MINUTE_5,
            Vec::new(),
            vec![rest_bar(T0, 101.0, 5.0)],
            &candles,
            Some(T0),
            500,
        );
        let bucket = merged.last().unwrap();
        assert_eq!(
            (bucket.open, bucket.high, bucket.low, bucket.volume),
            (100.0, 110.0, 95.0, 5.0),
            "REST's fuller account of the minute stands"
        );
        candles = merged;

        // The trades that follow are not lost, once they have caught up with REST's figure.
        let mut price = 102.0;
        for _ in 0..6 {
            let kline = trades
                .apply(Timeframe::MINUTE_1, trade(T0 + 50_000, price, 1.0))
                .unwrap();
            live.apply(
                Timeframe::MINUTE_5,
                Timeframe::MINUTE_1,
                &mut candles,
                kline,
            );
            price += 1.0;
        }
        let bucket = candles.last().unwrap();
        assert_eq!(bucket.close, 107.0, "the close moves with the trades again");
        assert_eq!(bucket.volume, 7.0);
    }
}
