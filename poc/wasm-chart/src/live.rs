//! Live trades folded into the candles on screen, with the app's own live-fold rules.
//!
//! Coinbase has no candle channel, so the base bar (the candle length Coinbase serves the chart
//! from) is built here from the `ticker` feed, as the running total of its bar that
//! [`LiveFold`] expects, and the fold turns it into the chart's candles. A REST snapshot is merged
//! over the result on a timer, and bars the feed delivered after the request began win over it.

use degenview_market::Kline;
use degenview_market::bucket::{FoldPlan, bucket_start_ms, next_bucket_start_ms};
use degenview_market::coinbase::Ticker;
use degenview_market::live::{LiveFold, LiveMark};

use crate::fetch::Fetched;

pub struct Live {
    plan: FoldPlan,
    limit: usize,
    fold: LiveFold,
    /// The base bar being traded now: REST's state of it, plus every trade since.
    bar: Option<Kline>,
    last_trade: Option<u64>,
    /// The newest bar the feed delivered, and when, on the page's monotonic clock (milliseconds).
    mark: Option<LiveMark<f64>>,
}

impl Live {
    pub fn new(fetched: &Fetched) -> Self {
        Self {
            plan: fetched.plan,
            limit: fetched.limit,
            fold: LiveFold::default(),
            bar: fetched.base.last().cloned(),
            last_trade: None,
            mark: None,
        }
    }

    /// Folds a trade that arrived at `now` into `candles`. Returns whether they changed.
    pub fn apply(&mut self, tick: &Ticker, now: f64, candles: &mut Vec<Kline>) -> bool {
        let Some(time) = tick.time_ms else {
            return false;
        };
        // The message a subscription starts with repeats the previous trade: its price counts,
        // its size is already in REST's bar. A trade seen before is not counted twice.
        let size = match (tick.trade_id, self.last_trade) {
            (Some(id), Some(last)) if id <= last => return false,
            (Some(id), last) => {
                self.last_trade = Some(id);
                if last.is_none() {
                    0.0
                } else {
                    tick.last_size.unwrap_or(0.0)
                }
            }
            (None, _) => tick.last_size.unwrap_or(0.0),
        };
        let base = self.plan.base();
        let bucket = bucket_start_ms(base, time);
        let bar = match self.bar.take() {
            Some(bar) if bar.open_time == bucket => traded(bar, tick.price, size),
            Some(bar) if bar.open_time > bucket => {
                self.bar = Some(bar);
                return false;
            }
            _ => Kline {
                open_time: bucket,
                close_time: next_bucket_start_ms(base, bucket) - 1,
                open: tick.price,
                high: tick.price,
                low: tick.price,
                close: tick.price,
                volume: size,
            },
        };
        self.bar = Some(bar.clone());
        self.mark = Some(LiveMark::new(bucket, now));
        self.fold
            .apply(self.plan.target(), base, candles, bar)
            .is_some()
    }

    /// Merges a REST answer whose request began at `started`, over `candles`.
    pub fn merge(&mut self, fetched: &Fetched, started: f64, candles: &mut Vec<Kline>) {
        let live_open = self.mark.and_then(|mark| mark.guard_for(started));
        *candles = self.fold.merge_fetched(
            self.plan.target(),
            fetched.candles.clone(),
            self.plan.newest_bucket(&fetched.base),
            candles,
            live_open,
            self.limit,
        );
        let Some(rest) = fetched.base.last() else {
            return;
        };
        match &mut self.bar {
            // A bar's volume only grows: the account with more of it is the later one.
            Some(bar) if bar.open_time == rest.open_time && rest.volume > bar.volume => {
                bar.volume = rest.volume;
                bar.open = rest.open;
                bar.high = bar.high.max(rest.high);
                bar.low = bar.low.min(rest.low);
            }
            Some(bar) if bar.open_time >= rest.open_time => {}
            slot => *slot = Some(rest.clone()),
        }
    }
}

/// `bar` after a trade of `size` at `price`.
fn traded(mut bar: Kline, price: f64, size: f64) -> Kline {
    bar.high = bar.high.max(price);
    bar.low = bar.low.min(price);
    bar.close = price;
    bar.volume += size;
    bar
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_market::Timeframe;

    const HOUR: i64 = 3_600_000;
    /// 2024-01-15T00:00:00Z.
    const DAY_START: i64 = 1_705_276_800_000;

    fn bar(hour: i64, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Kline {
        let open_time = DAY_START + hour * HOUR;
        Kline {
            open_time,
            close_time: open_time + HOUR - 1,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    fn tick(trade_id: u64, hour: f64, price: f64, size: f64) -> Ticker {
        Ticker {
            product_id: "BTC-USD".into(),
            price,
            last_size: Some(size),
            trade_id: Some(trade_id),
            time_ms: Some(DAY_START + (hour * HOUR as f64) as i64),
            open_24h: None,
            volume_24h: None,
        }
    }

    /// A chart of `target` over the hourly `base` bars, as the page loads one.
    fn loaded(target: &str, base: Vec<Kline>) -> (Live, Vec<Kline>) {
        let natives = [Timeframe::HOUR_1];
        let plan = FoldPlan::new(Timeframe::parse(target).unwrap(), &natives).unwrap();
        let fetched = Fetched {
            plan,
            limit: 50,
            candles: plan.fold(base.clone(), false),
            base,
        };
        (Live::new(&fetched), fetched.candles)
    }

    #[test]
    fn a_trade_updates_the_forming_candle_and_a_later_hour_starts_a_new_one() {
        let (mut live, mut candles) = loaded("1h", vec![bar(0, 100.0, 105.0, 99.0, 104.0, 5.0)]);
        // The feed's first message repeats the last trade: same price, no new size, no change.
        assert!(!live.apply(&tick(1, 0.2, 104.0, 1.0), 10.0, &mut candles));
        assert!(live.apply(&tick(2, 0.3, 110.0, 2.0), 11.0, &mut candles));
        assert_eq!(candles.len(), 1);
        let forming = &candles[0];
        assert_eq!(
            (forming.open, forming.high, forming.low, forming.close),
            (100.0, 110.0, 99.0, 110.0)
        );
        // Only the second trade's size is added.
        assert_eq!(forming.volume, 7.0);

        assert!(live.apply(&tick(3, 1.1, 108.0, 0.5), 12.0, &mut candles));
        assert_eq!(candles.len(), 2);
        assert_eq!(
            (candles[1].open, candles[1].close, candles[1].volume),
            (108.0, 108.0, 0.5)
        );
    }

    #[test]
    fn a_folded_candle_keeps_what_the_feed_missed_and_never_counts_a_trade_twice() {
        let base = vec![
            bar(0, 100.0, 101.0, 99.0, 100.5, 4.0),
            bar(1, 100.5, 103.0, 100.0, 102.0, 3.0),
            bar(2, 102.0, 104.0, 101.0, 103.0, 2.0),
        ];
        let (mut live, mut candles) = loaded("4h", base);
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].volume, 9.0);

        live.apply(&tick(1, 2.5, 103.0, 1.0), 10.0, &mut candles);
        live.apply(&tick(2, 2.6, 106.0, 1.5), 11.0, &mut candles);
        let forming = &candles[0];
        assert_eq!(
            (forming.open, forming.high, forming.low, forming.close),
            (100.0, 106.0, 99.0, 106.0)
        );
        assert_eq!(forming.volume, 10.5);
        // The same trade again changes nothing.
        assert!(!live.apply(&tick(2, 2.6, 106.0, 1.5), 12.0, &mut candles));
        assert_eq!(candles[0].volume, 10.5);
    }

    #[test]
    fn a_trade_older_than_the_forming_bar_is_ignored() {
        let (mut live, mut candles) = loaded("1h", vec![bar(1, 100.0, 105.0, 99.0, 104.0, 5.0)]);
        assert!(!live.apply(&tick(1, 0.5, 90.0, 1.0), 10.0, &mut candles));
        assert_eq!(candles[0].low, 99.0);
    }

    #[test]
    fn a_rest_snapshot_keeps_the_bars_the_feed_delivered_after_it_began() {
        let (mut live, mut candles) = loaded("1h", vec![bar(0, 100.0, 105.0, 99.0, 104.0, 5.0)]);
        live.apply(&tick(1, 0.2, 104.0, 0.0), 100.0, &mut candles);
        live.apply(&tick(2, 0.3, 111.0, 1.0), 200.0, &mut candles);
        let plan = live.plan;
        let snapshot = |volume: f64| {
            let base = vec![bar(0, 100.0, 108.0, 99.0, 107.0, volume)];
            Fetched {
                plan,
                limit: 50,
                candles: base.clone(),
                base,
            }
        };
        // Requested at 150, answered after the 200 trade: the feed's bar wins.
        live.merge(&snapshot(6.0), 150.0, &mut candles);
        assert_eq!(candles[0].close, 111.0);
        // Requested after the feed went quiet: REST's account replaces the bar.
        live.merge(&snapshot(6.0), 300.0, &mut candles);
        assert_eq!((candles[0].close, candles[0].high), (107.0, 108.0));
        // REST with more volume than the running bar lifts it.
        live.merge(&snapshot(9.0), 300.0, &mut candles);
        let next = live.bar.as_ref().unwrap();
        assert_eq!(next.volume, 9.0);
    }
}
