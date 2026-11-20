//! Request planning shared by every provider, so one market is cut the same whichever source serves
//! it. Bucket boundaries, folds and gap filling are `degenview_market::bucket`; the clean-up every
//! answer passes through is `degenview_market::normalize`.

use crate::model::{DataSource, Timeframe};
use crate::service::ProviderError;
use degenview_market::bucket::FoldPlan;

/// How `source` makes `timeframe` from the candles it has, or the error that it cannot.
pub(crate) fn plan(source: DataSource, timeframe: Timeframe) -> Result<FoldPlan, ProviderError> {
    source
        .plan(timeframe)
        .ok_or_else(|| ProviderError::unsupported(source, timeframe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;

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

    /// Fetches ETH from Binance, Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC and a Uniswap pool and compares their buckets. Run with
    /// `cargo test --release --lib live_sources_cut -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_sources_cut_the_same_buckets() {
        use crate::model::DataSource::{
            Binance, Bybit, Coinbase, DexScreener, Gate, Kraken, Mexc, Okx,
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
            // GeckoTerminal's free quota is small; a refusal skips it rather than failing the rest.
            let dex = fetch(DexScreener, pool)
                .map_err(|error| println!("skipped: {error}"))
                .ok();
            let opens = |bars: &[Kline]| -> std::collections::BTreeSet<i64> {
                bars.iter().map(|bar| bar.open_time).collect()
            };
            let (b, c, o, k, y, g, m) = (
                opens(&binance),
                opens(&coinbase),
                opens(&okx),
                opens(&kraken),
                opens(&bybit),
                opens(&gate),
                opens(&mexc),
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
                "{interval:>3}: binance {:>4} coinbase {:>4} okx {:>4} kraken {:>4} bybit {:>4} gate {:>4} mexc {:>4} dex {:>4} | shared range {:>4} bars",
                b.len(),
                c.len(),
                o.len(),
                k.len(),
                y.len(),
                g.len(),
                m.len(),
                d.as_ref().map_or(0, |d| d.len()),
                within(&b).len(),
            );
            assert_eq!(within(&b), within(&c), "{interval} binance vs coinbase");
            assert_eq!(within(&b), within(&o), "{interval} binance vs okx");
            assert_eq!(within(&b), within(&k), "{interval} binance vs kraken");
            assert_eq!(within(&b), within(&y), "{interval} binance vs bybit");
            assert_eq!(within(&b), within(&g), "{interval} binance vs gate");
            assert_eq!(within(&b), within(&m), "{interval} binance vs mexc");
            if let Some(d) = &d {
                assert_eq!(within(&b), within(d), "{interval} binance vs dexscreener");
            }
        }
    }
}
