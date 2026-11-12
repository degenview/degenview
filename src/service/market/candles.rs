//! Candle shaping shared by every provider, so one market looks the same whichever source serves
//! it: calendar folds onto Binance's bucket boundaries, gap filling for sources that skip empty
//! intervals, and one clean-up pass over what a provider returned.

use crate::model::Kline;
use crate::service::bucket_start_ms;

const DAY_MS: i64 = 86_400_000;
/// The most candles a gap fill may insert in one go; a longer silence is left as it is.
const MAX_FILLED: i64 = 20_000;

/// Open time of the calendar bucket after the one that opens at `bucket`.
pub(crate) fn next_bucket_start_ms(bucket: i64, interval_secs: i64) -> i64 {
    // Any instant inside the next bucket lands in it: the shortest span past a bucket's start that
    // is certainly over its end is 8 days for a week, 32 for a month, 93 for a quarter, 367 for a
    // year.
    let probe = match interval_secs {
        s if s < 604_800 => return bucket + s * 1_000,
        s if s < 2_419_200 => 8,
        s if s < 7_000_000 => 32,
        s if s < 30_000_000 => 93,
        _ => 367,
    };
    bucket_start_ms(bucket + probe * DAY_MS, interval_secs)
}

/// Fold ascending candles into calendar buckets of `target_secs` (week from Monday, month,
/// quarter, year), the way Binance cuts them. Each bucket opens at its calendar start and closes a
/// millisecond before the next; the forming last bucket is kept.
///
/// `truncated` says the series starts where a request stopped rather than where the market
/// began: its first bucket is then a fragment and is dropped, so no bar shows a range the data
/// did not cover.
pub(crate) fn fold_calendar(candles: &[Kline], target_secs: i64, truncated: bool) -> Vec<Kline> {
    let mut out: Vec<Kline> = Vec::new();
    for candle in candles {
        let bucket = bucket_start_ms(candle.open_time, target_secs);
        match out.last_mut() {
            Some(last) if last.open_time == bucket => {
                last.high = last.high.max(candle.high);
                last.low = last.low.min(candle.low);
                last.close = candle.close;
                last.volume += candle.volume;
            }
            _ => out.push(Kline {
                open_time: bucket,
                close_time: next_bucket_start_ms(bucket, target_secs) - 1,
                open: candle.open,
                high: candle.high,
                low: candle.low,
                close: candle.close,
                volume: candle.volume,
            }),
        }
    }
    if truncated
        && let (Some(first), Some(bucket)) = (candles.first(), out.first())
        && first.open_time > bucket.open_time
        && out.len() > 1
    {
        out.remove(0);
    }
    out
}

/// Insert a flat, volume-less candle (at the previous close) for every interval a source skipped
/// between candles, and up to the interval holding `now_ms`, so bars are evenly spaced like an
/// exchange's. Intervals are `step_ms` wide and aligned to the epoch (hours, days).
pub(crate) fn fill_gaps(candles: Vec<Kline>, step_ms: i64, now_ms: i64) -> Vec<Kline> {
    if step_ms <= 0 || candles.is_empty() {
        return candles;
    }
    let mut out = Vec::with_capacity(candles.len());
    let mut filled = 0;
    for candle in candles {
        if let Some(previous) = out.last().cloned() {
            fill_between(&mut out, &previous, candle.open_time, step_ms, &mut filled);
        }
        out.push(candle);
    }
    let current = now_ms.div_euclid(step_ms) * step_ms;
    if let Some(last) = out.last().cloned() {
        fill_between(&mut out, &last, current + step_ms, step_ms, &mut filled);
    }
    out
}

/// Flat candles after `previous`, one per `step_ms`, strictly before `until`.
fn fill_between(
    out: &mut Vec<Kline>,
    previous: &Kline,
    until: i64,
    step_ms: i64,
    filled: &mut i64,
) {
    let missing = (until - previous.open_time) / step_ms - 1;
    if missing <= 0 || *filled + missing > MAX_FILLED {
        return;
    }
    *filled += missing;
    for index in 1..=missing {
        let open_time = previous.open_time + index * step_ms;
        out.push(Kline {
            open_time,
            close_time: open_time + step_ms - 1,
            open: previous.close,
            high: previous.close,
            low: previous.close,
            close: previous.close,
            volume: 0.0,
        });
    }
}

/// What every provider's candles pass through: ascending by open time, one candle per open time
/// (the later one wins), no non-finite or non-positive prices, and a range that holds its own
/// open and close.
pub(crate) fn normalize(mut candles: Vec<Kline>) -> Vec<Kline> {
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

    fn day(year: i64, month: i64, date: i64) -> i64 {
        crate::format::days_from_civil(year, month, date) * DAY_MS
    }

    fn daily(start: i64, count: usize) -> Vec<Kline> {
        (0..count)
            .map(|index| {
                let open_time = start + index as i64 * DAY_MS;
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
    fn weeks_open_on_monday_and_close_before_the_next() {
        // 2024-01-01 was a Monday.
        let weeks = fold_calendar(&daily(day(2024, 1, 1), 15), 604_800, false);
        let opens: Vec<i64> = weeks.iter().map(|week| week.open_time).collect();
        assert_eq!(opens, [day(2024, 1, 1), day(2024, 1, 8), day(2024, 1, 15)]);
        assert_eq!(weeks[0].close_time, day(2024, 1, 8) - 1);
        assert_eq!(weeks[0].open, 100.0);
        assert_eq!(weeks[0].close, 107.0);
        assert_eq!(weeks[0].high, 111.0);
        assert_eq!(weeks[0].low, 95.0);
        assert_eq!(weeks[0].volume, 14.0);
    }

    #[test]
    fn months_close_on_their_real_last_day() {
        let months = fold_calendar(&daily(day(2024, 1, 1), 70), 2_592_000, false);
        assert_eq!(months[0].close_time, day(2024, 2, 1) - 1);
        assert_eq!(months[1].open_time, day(2024, 2, 1));
        // 2024 is a leap year.
        assert_eq!(months[1].close_time, day(2024, 3, 1) - 1);
    }

    #[test]
    fn quarters_and_years_follow_the_calendar() {
        let quarters = fold_calendar(&daily(day(2023, 11, 20), 200), 7_776_000, false);
        let opens: Vec<i64> = quarters.iter().map(|q| q.open_time).collect();
        assert_eq!(opens, [day(2023, 10, 1), day(2024, 1, 1), day(2024, 4, 1)]);
        assert_eq!(quarters[0].close_time, day(2024, 1, 1) - 1);

        let years = fold_calendar(&daily(day(2023, 11, 20), 200), 31_536_000, false);
        let opens: Vec<i64> = years.iter().map(|y| y.open_time).collect();
        assert_eq!(opens, [day(2023, 1, 1), day(2024, 1, 1)]);
        assert_eq!(years[0].close_time, day(2024, 1, 1) - 1);
    }

    #[test]
    fn folding_days_or_months_cuts_the_same_buckets() {
        // Binance folds months, Coinbase and GeckoTerminal fold days: both must agree.
        let days = daily(day(2022, 1, 1), 900);
        let months = fold_calendar(&days, 2_592_000, false);
        for target in [7_776_000, 31_536_000] {
            let from_days = fold_calendar(&days, target, false);
            let from_months = fold_calendar(&months, target, false);
            let opens = |bars: &[Kline]| -> Vec<(i64, i64)> {
                bars.iter()
                    .map(|bar| (bar.open_time, bar.close_time))
                    .collect()
            };
            assert_eq!(opens(&from_days), opens(&from_months), "{target}");
        }
    }

    #[test]
    fn a_truncated_series_loses_its_fragment_of_a_first_bucket() {
        // Starts on a Wednesday: the first week is a fragment.
        let candles = daily(day(2024, 1, 3), 14);
        let kept = fold_calendar(&candles, 604_800, false);
        let dropped = fold_calendar(&candles, 604_800, true);
        assert_eq!(kept.len(), dropped.len() + 1);
        assert_eq!(dropped[0].open_time, day(2024, 1, 8));
        // A whole first bucket stays, and so does a lone fragment.
        assert_eq!(
            fold_calendar(&daily(day(2024, 1, 1), 7), 604_800, true).len(),
            1
        );
        assert_eq!(
            fold_calendar(&daily(day(2024, 1, 3), 3), 604_800, true).len(),
            1
        );
    }

    #[test]
    fn skipped_intervals_are_filled_flat_at_the_previous_close() {
        let mut candles = daily(day(2024, 1, 1), 5);
        candles.remove(2);
        let filled = fill_gaps(candles, DAY_MS, day(2024, 1, 5) + 1);
        assert_eq!(filled.len(), 5);
        assert_eq!(filled[2].open_time, day(2024, 1, 3));
        assert_eq!(filled[2].open, filled[1].close);
        assert_eq!(filled[2].high, filled[2].low);
        assert_eq!(filled[2].volume, 0.0);
    }

    #[test]
    fn the_filled_series_reaches_the_current_interval() {
        let filled = fill_gaps(daily(day(2024, 1, 1), 2), DAY_MS, day(2024, 1, 5) + 1_000);
        assert_eq!(filled.len(), 5);
        assert_eq!(filled[4].open_time, day(2024, 1, 5));
    }

    #[test]
    fn an_endless_silence_is_not_filled() {
        let candles = vec![daily(0, 1)[0].clone(), daily(day(2024, 1, 1), 1)[0].clone()];
        let before = candles.len();
        assert_eq!(fill_gaps(candles, 3_600_000, day(2024, 1, 1)).len(), before);
    }

    #[test]
    fn normalizing_sorts_dedupes_and_repairs() {
        let mut a = daily(day(2024, 1, 1), 3);
        a.reverse();
        a.push(Kline {
            open: 500.0,
            ..a[0].clone()
        });
        a[1].high = f64::NAN;
        let mut broken = a[2].clone();
        broken.open_time = day(2024, 2, 1);
        broken.high = 1.0;
        a.push(broken);
        let out = normalize(a);
        // The NaN candle is gone; the duplicate of the newest keeps the later copy.
        let opens: Vec<i64> = out.iter().map(|c| c.open_time).collect();
        assert_eq!(opens, [day(2024, 1, 1), day(2024, 1, 3), day(2024, 2, 1)]);
        assert_eq!(out[1].open, 500.0);
        // The range holds the open and close.
        let repaired = &out[2];
        assert!(repaired.high >= repaired.open && repaired.high >= repaired.close);
    }

    /// Fetches ETH from Binance, Coinbase and a Uniswap pool and compares their buckets. Run with
    /// `cargo test --release --lib live_sources_cut -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_sources_cut_the_same_buckets() {
        use crate::model::DataSource::{Binance, Coinbase, DexScreener};
        let pool = "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640";
        for interval in ["1h", "1d", "1w", "1M", "3M", "1Y"] {
            let fetch = |source, symbol: &str| {
                crate::service::fetch_klines(source, symbol, interval, 500)
                    .map_err(|error| format!("{source:?} {interval}: {error}"))
            };
            let binance = fetch(Binance, "ETHUSDT").unwrap();
            let coinbase = fetch(Coinbase, "ETH-USD").unwrap();
            // GeckoTerminal's free quota is small; a refusal skips it rather than failing the rest.
            let dex = fetch(DexScreener, pool)
                .map_err(|error| println!("skipped: {error}"))
                .ok();
            let opens = |bars: &[Kline]| -> std::collections::BTreeSet<i64> {
                bars.iter().map(|bar| bar.open_time).collect()
            };
            let (b, c) = (opens(&binance), opens(&coinbase));
            let d = dex.as_deref().map(opens);
            // Compare inside the range every source covers.
            let from = [b.first(), c.first(), d.as_ref().and_then(|d| d.first())]
                .into_iter()
                .flatten()
                .max()
                .copied()
                .unwrap();
            let within = |set: &std::collections::BTreeSet<i64>| -> Vec<i64> {
                set.iter().copied().filter(|time| *time >= from).collect()
            };
            println!(
                "{interval:>3}: binance {:>4} coinbase {:>4} dex {:>4} | shared range {:>4} bars",
                b.len(),
                c.len(),
                d.as_ref().map_or(0, |d| d.len()),
                within(&b).len(),
            );
            assert_eq!(within(&b), within(&c), "{interval} binance vs coinbase");
            if let Some(d) = &d {
                assert_eq!(within(&b), within(d), "{interval} binance vs dexscreener");
            }
        }
    }
}
