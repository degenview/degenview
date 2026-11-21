//! Candle clean-up shared by every provider, so one market looks the same whichever source serves
//! it. Bucket boundaries, folds and gap filling are `model::market::bucket`.

use crate::model::market::bucket::FoldPlan;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;

/// How `source` makes `timeframe` from the candles it has, or the error that it cannot.
pub(crate) fn plan(source: DataSource, timeframe: Timeframe) -> Result<FoldPlan, ProviderError> {
    source
        .plan(timeframe)
        .ok_or_else(|| ProviderError::unsupported(source, timeframe))
}

/// What every provider's candles pass through: ascending by open time, one candle per open time
/// (the later one wins), no non-finite prices, and a range that holds its own open and close.
/// Zero and negative prices stay: a prediction market's price reaches 0.
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

    const DAY_MS: i64 = 86_400_000;

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

    /// Lengths a user may pick, short to long, odd ones among them.
    const TABLE: [&str; 15] = [
        "1m", "2m", "3m", "5m", "7m", "22m", "30m", "1h", "3h", "24h", "1D", "3D", "1W", "1M", "1Y",
    ];

    #[test]
    fn a_candle_sources_planner_agrees_with_what_it_serves() {
        for source in [
            DataSource::Binance,
            DataSource::Coinbase,
            DataSource::Okx,
            DataSource::Kraken,
            DataSource::Bybit,
            DataSource::Gate,
            DataSource::Mexc,
            DataSource::Alpaca,
            DataSource::DexScreener,
        ] {
            for text in TABLE {
                let timeframe = Timeframe::parse(text).unwrap();
                match plan(source, timeframe) {
                    Ok(plan) => assert!(source.serves(timeframe), "{source:?} {text}: {plan:?}"),
                    Err(error) => {
                        assert!(!source.serves(timeframe), "{source:?} {text}");
                        assert_eq!(error, ProviderError::unsupported(source, timeframe));
                    }
                }
            }
        }
    }

    #[test]
    fn coingecko_refuses_what_it_does_not_serve_before_any_request() {
        for text in TABLE {
            let timeframe = Timeframe::parse(text).unwrap();
            if DataSource::CoinGecko.serves(timeframe) {
                continue;
            }
            let error = crate::service::for_source(DataSource::CoinGecko)
                .fetch_klines("bitcoin", timeframe, 10)
                .unwrap_err();
            assert!(error.is_unsupported_interval(), "{text}: {error}");
        }
    }

    /// Fetches ETH from Binance, Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC, Hyperliquid and a Uniswap pool and compares their buckets. Run with
    /// `cargo test --release --lib live_sources_cut -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_sources_cut_the_same_buckets() {
        use crate::model::DataSource::{
            Binance, Bybit, Coinbase, DexScreener, Gate, Hyperliquid, Kraken, Mexc, Okx,
        };
        let pool = "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640";
        let odd = ["5m", "22m", "3h", "4h", "6h", "12h", "2D", "3D", "2W", "6M"]
            .map(|text| Timeframe::parse(text).expect("a timeframe"));
        for interval in Timeframe::LEGACY.into_iter().chain(odd) {
            let fetch = |source, symbol: &str| {
                crate::service::fetch_klines(source, symbol, interval, 500)
                    .map_err(|error| format!("{source:?} {interval}: {error}"))
            };
            let binance = fetch(Binance, "ETHUSDT").unwrap();
            let coinbase = fetch(Coinbase, "ETH-USD").unwrap();
            let okx = fetch(Okx, "ETH-USDT").unwrap();
            let kraken = fetch(Kraken, "ETH/USD").unwrap();
            let bybit = fetch(Bybit, "ETHUSDT").unwrap();
            let gate = fetch(Gate, "ETH_USDT").unwrap();
            let mexc = fetch(Mexc, "ETHUSDT").unwrap();
            let hyperliquid = fetch(Hyperliquid, "ETH").unwrap();
            // GeckoTerminal's free quota is small; a refusal skips it rather than failing the rest.
            let dex = fetch(DexScreener, pool)
                .map_err(|error| println!("skipped: {error}"))
                .ok();
            let opens = |bars: &[Kline]| -> std::collections::BTreeSet<i64> {
                bars.iter().map(|bar| bar.open_time).collect()
            };
            let (b, c, o, k, y, g, m, h) = (
                opens(&binance),
                opens(&coinbase),
                opens(&okx),
                opens(&kraken),
                opens(&bybit),
                opens(&gate),
                opens(&mexc),
                opens(&hyperliquid),
            );
            let d = dex.as_deref().map(opens);
            // Compare inside the range every source covers.
            let from = [
                b.first(),
                c.first(),
                o.first(),
                k.first(),
                y.first(),
                g.first(),
                m.first(),
                h.first(),
                d.as_ref().and_then(|d| d.first()),
            ]
            .into_iter()
            .flatten()
            .max()
            .copied()
            .unwrap();
            let within = |set: &std::collections::BTreeSet<i64>| -> Vec<i64> {
                set.iter().copied().filter(|time| *time >= from).collect()
            };
            println!(
                "{interval:>3}: binance {:>4} coinbase {:>4} okx {:>4} kraken {:>4} bybit {:>4} gate {:>4} mexc {:>4} hyperliquid {:>4} dex {:>4} | shared range {:>4} bars",
                b.len(),
                c.len(),
                o.len(),
                k.len(),
                y.len(),
                g.len(),
                m.len(),
                h.len(),
                d.as_ref().map_or(0, |d| d.len()),
                within(&b).len(),
            );
            assert_eq!(within(&b), within(&c), "{interval} binance vs coinbase");
            assert_eq!(within(&b), within(&o), "{interval} binance vs okx");
            assert_eq!(within(&b), within(&k), "{interval} binance vs kraken");
            assert_eq!(within(&b), within(&y), "{interval} binance vs bybit");
            assert_eq!(within(&b), within(&g), "{interval} binance vs gate");
            assert_eq!(within(&b), within(&m), "{interval} binance vs mexc");
            assert_eq!(within(&b), within(&h), "{interval} binance vs hyperliquid");
            if let Some(d) = &d {
                assert_eq!(within(&b), within(d), "{interval} binance vs dexscreener");
            }
        }
    }
}
