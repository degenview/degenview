use crate::Kline;

/// What every provider's candles pass through: ascending by open time, one candle per open time
/// (the later one wins), no non-finite prices, and a range that holds its own open and close.
/// Zero and negative prices stay: a prediction market's price reaches 0.
pub fn normalize(mut candles: Vec<Kline>) -> Vec<Kline> {
    candles.retain(|candle| {
        candle.open_time > 0
            && [candle.open, candle.high, candle.low, candle.close]
                .iter()
                .all(|price| price.is_finite())
    });
    // A stable sort keeps the later duplicate last, and `dedup_by` keeps the earlier of a pair, so
    // walk back to keep the later one.
    candles.sort_by_key(|candle| candle.open_time);
    candles.reverse();
    candles.dedup_by_key(|candle| candle.open_time);
    candles.reverse();
    for candle in &mut candles {
        candle.high = candle.high.max(candle.open).max(candle.close);
        candle.low = candle.low.min(candle.open).min(candle.close);
        if !candle.volume.is_finite() {
            candle.volume = 0.0;
        }
    }
    candles
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 86_400_000;
    /// 2024-01-01T00:00:00Z.
    const START: i64 = 1_704_067_200_000;

    fn daily(count: usize) -> Vec<Kline> {
        (0..count)
            .map(|index| {
                let open_time = START + index as i64 * DAY_MS;
                let price = 100.0 + index as f64;
                Kline {
                    open_time,
                    close_time: open_time + DAY_MS - 1,
                    open: price,
                    high: price + 5.0,
                    low: price - 5.0,
                    close: price + 1.0,
                    volume: 2.0,
                }
            })
            .collect()
    }

    #[test]
    fn normalizing_sorts_dedupes_and_repairs() {
        let mut a = daily(3);
        a.reverse();
        a.push(Kline {
            open: 500.0,
            ..a[0].clone()
        });
        a[1].high = f64::NAN;
        let mut broken = a[2].clone();
        broken.open_time = START + 31 * DAY_MS;
        broken.high = 1.0;
        a.push(broken);
        let out = normalize(a);
        // The NaN candle is gone; the duplicate of the newest keeps the later copy.
        let opens: Vec<i64> = out.iter().map(|c| c.open_time).collect();
        assert_eq!(opens, [START, START + 2 * DAY_MS, START + 31 * DAY_MS]);
        assert_eq!(out[1].open, 500.0);
        // The range holds the open and close.
        let repaired = &out[2];
        assert!(repaired.high >= repaired.open && repaired.high >= repaired.close);
    }
}
