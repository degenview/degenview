//! A live candle built from trades, for a venue that streams trades but not candles (Bitstamp).

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
    /// Whether the trade can be part of a candle or a price: a real price and a size that is not
    /// negative.
    pub fn is_sound(&self) -> bool {
        self.price.is_finite() && self.price > 0.0 && self.size.is_finite() && self.size >= 0.0
    }
}

/// Folds one market's trades into the forming candle of `base` length. The candle it hands back
/// after each trade is a kline of [`Klines::Trades`](super::Klines::Trades), taken by
/// [`LiveFold::apply`](super::LiveFold::apply) as an increment on the candle the chart holds.
///
/// The open is the first trade of the bucket, the high and low and the volume run, the close is
/// the latest trade. A trade older than the bucket in progress is dropped (a late trade of a bar
/// that has closed); a trade of a newer bucket starts a candle of its own. Trades of one bucket
/// count in the order they arrive, so a trade that arrives out of order moves the close.
///
/// **What the candle holds.** The trades seen since the fold began, never the bar: the bar that was
/// open at that moment traded before it, so its open, range and volume are not in the candle. That
/// is why the fold is not a candle channel: [`LiveFold`](super::LiveFold) keeps the candle it holds
/// of the bar (REST's account), takes its open and widens its range with the trades, and adds
/// only the volume the trades brought after it. The fold is kept for as long as the feed lives, a
/// reconnect included: its volume only grows, and the trades missed in between are what REST's
/// next account of the bar restores.
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
    use super::super::{Klines, LiveFold, Stream};
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
    fn a_new_base_forgets_the_candle_in_progress() {
        let mut fold = TradeFold::default();
        fold.apply(Timeframe::MINUTE_1, trade(T0, 100.0, 1.0));
        let other = fold
            .apply(Timeframe::MINUTE_5, trade(T0 + 1_000, 90.0, 1.0))
            .unwrap();
        assert_eq!((other.open, other.volume), (90.0, 1.0));
    }

    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;

    fn stream(base: Timeframe) -> Stream {
        Stream {
            base,
            klines: Klines::Trades,
        }
    }

    /// A candle REST holds of the bar of `length` that opens at `open_time`.
    fn rest(
        open_time: i64,
        length: i64,
        (open, high, low, close): (f64, f64, f64, f64),
        volume: f64,
    ) -> Kline {
        Kline {
            open_time,
            close_time: open_time + length - 1,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    /// What a chart does with each trade of a feed: the trade-built kline goes through the fold.
    struct Feed {
        trades: TradeFold,
        live: LiveFold,
        target: Timeframe,
        base: Timeframe,
        candles: Vec<Kline>,
    }

    impl Feed {
        fn new(target: Timeframe, base: Timeframe, candles: Vec<Kline>) -> Self {
            Self {
                trades: TradeFold::default(),
                live: LiveFold::default(),
                target,
                base,
                candles,
            }
        }

        fn trade(&mut self, time_ms: i64, price: f64, size: f64) {
            let kline = self
                .trades
                .apply(self.base, trade(time_ms, price, size))
                .unwrap();
            self.live
                .apply(self.target, stream(self.base), &mut self.candles, kline);
        }

        fn newest(&self) -> (f64, f64, f64, f64, f64) {
            let bar = self.candles.last().unwrap();
            (bar.open, bar.high, bar.low, bar.close, bar.volume)
        }
    }

    /// A chart of a coarser length takes the trade-built minutes through `LiveFold` like a venue's
    /// own candles: the bucket is the minutes' fold.
    #[test]
    fn trade_built_minutes_fold_into_a_coarser_chart() {
        let mut feed = Feed::new(Timeframe::MINUTE_5, Timeframe::MINUTE_1, Vec::new());
        for (offset, price, size) in [
            (0, 100.0, 1.0),
            (30_000, 104.0, 1.0),
            (MINUTE + 5_000, 96.0, 2.0),
            (2 * MINUTE, 99.0, 1.0),
        ] {
            feed.trade(T0 + offset, price, size);
        }
        assert_eq!(feed.candles.len(), 1);
        assert_eq!(feed.candles[0].open_time, T0);
        assert_eq!(feed.newest(), (100.0, 104.0, 96.0, 99.0, 5.0));
    }

    /// The finding: a day chart that connects at 18:00 held the day's bar from REST, and the
    /// trade-built candle (first trade as open, high and low, tiny volume) replaced it.
    #[test]
    fn a_native_chart_adds_the_trades_to_the_bar_it_held_and_keeps_its_open_and_range() {
        for (base, length) in [(Timeframe::DAY_1, DAY), (Timeframe::HOUR_1, HOUR)] {
            let at = T0 + length / 4 * 3;
            let before = rest(T0 - length, length, (90.0, 95.0, 88.0, 94.0), 40.0);
            let bar = rest(T0, length, (100.0, 110.0, 95.0, 102.0), 50.0);
            let mut feed = Feed::new(base, base, vec![before.clone(), bar]);

            feed.trade(at, 103.0, 1.0);
            assert_eq!(
                feed.newest(),
                (100.0, 110.0, 95.0, 103.0, 50.0),
                "{base:?}: the first trade moves only the close, and is counted in what was held"
            );
            feed.trade(at + MINUTE, 90.0, 2.0);
            feed.trade(at + 2 * MINUTE, 104.0, 1.0);
            assert_eq!(
                feed.newest(),
                (100.0, 110.0, 90.0, 104.0, 53.0),
                "{base:?}: the range widens, the volume adds what traded after the first trade"
            );
            assert_eq!(feed.candles.len(), 2);
            assert_eq!(feed.candles[0], before);

            // The next bar opens at its first trade, with nothing held.
            feed.trade(T0 + length + 5_000, 111.0, 3.0);
            assert_eq!(feed.candles.len(), 3);
            assert_eq!(feed.newest(), (111.0, 111.0, 111.0, 111.0, 3.0));
        }
    }

    #[test]
    fn a_chart_with_no_candle_of_the_bar_builds_it_from_its_trades() {
        let mut feed = Feed::new(Timeframe::HOUR_1, Timeframe::HOUR_1, Vec::new());
        feed.trade(T0 + 10_000, 100.0, 1.0);
        feed.trade(T0 + 20_000, 98.0, 2.0);
        assert_eq!(feed.newest(), (100.0, 100.0, 98.0, 98.0, 3.0));
    }

    /// A week chart of a venue with days only: the held week keeps its open and range, and the
    /// days the trades span (Wednesday, then Thursday) add up.
    #[test]
    fn a_folded_chart_adds_the_trades_of_every_day_to_the_bucket_it_held() {
        let week = 7 * DAY;
        let bar = rest(T0, week, (100.0, 110.0, 95.0, 102.0), 500.0);
        let mut feed = Feed::new(Timeframe::WEEK_1, Timeframe::DAY_1, vec![bar]);

        let wednesday = T0 + 2 * DAY + 10 * HOUR;
        feed.trade(wednesday, 103.0, 1.0);
        assert_eq!(feed.newest(), (100.0, 110.0, 95.0, 103.0, 500.0));
        feed.trade(wednesday + HOUR, 120.0, 2.0);
        assert_eq!(feed.newest(), (100.0, 120.0, 95.0, 120.0, 502.0));

        let thursday = T0 + 3 * DAY + HOUR;
        feed.trade(thursday, 90.0, 4.0);
        assert_eq!(feed.candles.len(), 1);
        assert_eq!(
            feed.newest(),
            (100.0, 120.0, 90.0, 90.0, 506.0),
            "Wednesday's trades stay in the week"
        );
    }

    /// The answer REST brings after the trades began is the candle the trades add to: the trades it
    /// already counts are not added again, those after it are.
    #[test]
    fn a_rest_answer_after_the_trades_began_becomes_the_candle_they_add_to() {
        let mut feed = Feed::new(Timeframe::DAY_1, Timeframe::DAY_1, Vec::new());
        let at = T0 + 18 * HOUR;
        feed.trade(at, 103.0, 1.0);
        feed.trade(at + MINUTE, 90.0, 2.0);
        assert_eq!(feed.newest(), (103.0, 103.0, 90.0, 90.0, 3.0));

        // REST's day holds those three units (and the trades before the socket connected).
        let older = rest(T0 - DAY, DAY, (90.0, 95.0, 88.0, 94.0), 40.0);
        let account = rest(T0, DAY, (100.0, 110.0, 85.0, 90.0), 50.0);
        let merged = feed.live.merge_fetched(
            Timeframe::DAY_1,
            vec![older.clone(), account],
            Vec::new(),
            &feed.candles,
            Some(T0),
            500,
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0], older);
        let bar = merged.last().unwrap();
        assert_eq!(
            (bar.open, bar.high, bar.low, bar.close, bar.volume),
            (100.0, 110.0, 85.0, 90.0, 50.0),
            "REST's account stands whole; the trades it counts add nothing"
        );
        feed.candles = merged;

        feed.trade(at + 2 * MINUTE, 95.0, 1.5);
        assert_eq!(feed.newest(), (100.0, 110.0, 85.0, 95.0, 51.5));
    }

    /// The same for a folded chart: REST's base bars of the week, the last of them partial.
    #[test]
    fn a_rest_answer_with_the_buckets_base_bars_becomes_the_week_the_trades_add_to() {
        let mut feed = Feed::new(Timeframe::WEEK_1, Timeframe::DAY_1, Vec::new());
        let wednesday = T0 + 2 * DAY + 10 * HOUR;
        feed.trade(wednesday, 103.0, 1.0);

        let monday = rest(T0, DAY, (100.0, 110.0, 95.0, 102.0), 20.0);
        let tuesday = rest(T0 + DAY, DAY, (102.0, 108.0, 99.0, 101.0), 30.0);
        let today = rest(T0 + 2 * DAY, DAY, (101.0, 104.0, 100.0, 103.0), 6.0);
        let week = rest(T0, 7 * DAY, (100.0, 110.0, 95.0, 103.0), 56.0);
        let merged = feed.live.merge_fetched(
            Timeframe::WEEK_1,
            vec![week],
            vec![monday, tuesday, today],
            &feed.candles,
            Some(T0),
            500,
        );
        let bar = merged.last().unwrap();
        assert_eq!(
            (bar.open, bar.high, bar.low, bar.close, bar.volume),
            (100.0, 110.0, 95.0, 103.0, 56.0)
        );
        feed.candles = merged;

        feed.trade(wednesday + HOUR, 112.0, 2.0);
        assert_eq!(
            feed.newest(),
            (100.0, 112.0, 95.0, 112.0, 58.0),
            "the days REST had before the socket stay in the week"
        );
    }

    /// A reconnect loses the trades in between and nothing else: the bar goes on from where it was,
    /// its volume never falls, and the chart keeps taking the klines.
    #[test]
    fn a_reconnect_mid_bar_goes_on_with_the_bar_in_progress() {
        let bar = rest(T0, DAY, (100.0, 110.0, 95.0, 102.0), 50.0);
        let mut feed = Feed::new(Timeframe::DAY_1, Timeframe::DAY_1, vec![bar]);
        feed.trade(T0 + HOUR, 101.0, 1.0);
        feed.trade(T0 + 2 * HOUR, 99.0, 1.0);
        let before = feed.newest();
        assert_eq!(before, (100.0, 110.0, 95.0, 99.0, 51.0));

        // The socket was down for three hours and is back: the fold is the same one.
        feed.trade(T0 + 5 * HOUR, 97.0, 1.0);
        let after = feed.newest();
        assert_eq!(after, (100.0, 110.0, 95.0, 97.0, 52.0));
        assert!(after.4 >= before.4);
    }

    /// An answer no socket kline is newer than replaces the bar, and the trades that follow add to
    /// the replacement, counting from the volume the fold had when it arrived.
    #[test]
    fn an_answer_the_socket_was_quiet_for_replaces_the_bar_and_the_trades_that_follow_add_to_it() {
        let mut feed = Feed::new(Timeframe::DAY_1, Timeframe::DAY_1, Vec::new());
        feed.trade(T0 + HOUR, 103.0, 1.0);
        feed.trade(T0 + 2 * HOUR, 90.0, 2.0);

        let account = rest(T0, DAY, (100.0, 110.0, 85.0, 91.0), 60.0);
        let merged = feed.live.merge_fetched(
            Timeframe::DAY_1,
            vec![account],
            Vec::new(),
            &feed.candles,
            None,
            500,
        );
        assert_eq!(
            (merged.last().unwrap().close, merged.last().unwrap().volume),
            (91.0, 60.0),
            "the newest account of the bar, close included"
        );
        feed.candles = merged;

        // The fold's klines carry the 3 units since the feed began, which the answer counts; the
        // next trade makes it 4.
        feed.trade(T0 + 3 * HOUR, 92.0, 1.0);
        assert_eq!(feed.newest(), (100.0, 110.0, 85.0, 92.0, 61.0));
    }
}
