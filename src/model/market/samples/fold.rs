//! Candles made of price samples: one candle per calendar bucket of the target length.

use crate::model::market::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::{Kline, Timeframe};

/// Fold a price series, oldest first, into candles of `target`.
///
/// Each candle opens at the previous one's close, so the series is continuous and a bucket holding
/// a single sample still gets a body (and a colour); only the first opens at its own first sample.
pub fn fold_samples(prices: &[(i64, f64)], target: Timeframe) -> Vec<Kline> {
    let mut out: Vec<Kline> = Vec::new();
    for &(ms, price) in prices {
        let bucket = bucket_start_ms(target, ms);
        match out.last_mut() {
            Some(last) if last.open_time == bucket => {
                last.high = last.high.max(price);
                last.low = last.low.min(price);
                last.close = price;
            }
            _ => {
                let open = out.last().map_or(price, |previous| previous.close);
                out.push(Kline {
                    open_time: bucket,
                    close_time: next_bucket_start_ms(target, bucket) - 1,
                    open,
                    high: open.max(price),
                    low: open.min(price),
                    close: price,
                    volume: 0.0,
                });
            }
        }
    }
    out
}
