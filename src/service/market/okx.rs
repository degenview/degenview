//! OKX spot candles and its market list. Public endpoints, no key.
//!
//! Rows are `[open time ms, open, high, low, close, base volume, quote volume, quote volume,
//! confirm]`, all strings, newest first. `/market/candles` serves the newest 300; older ones are
//! paged back with `/market/history-candles` and an `after` cursor. Failures arrive as HTTP 200
//! with a `code` other than `"0"`.

use super::candles::plan;
use super::catalog::Catalog;
use super::paging::Paging;
use super::search_rank::normalize;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::net::http;
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const BASE_URL: &str = "https://www.okx.com/api/v5";

pub struct Okx;

pub const OKX: Okx = Okx;

/// 300 rows a request, 120 pages (a 22-minute chart of 500 candles is 37), and a pause between
/// them for its 20 requests per 2 seconds per IP.
const PAGING: Paging = Paging {
    page: 300,
    max_candles: 36_000,
    pause: Duration::from_millis(100),
};

/// OKX's bar token for a candle length it serves itself: `5m`, `4H`, and the `utc` bars from six
/// hours up (`6Hutc`, `1Dutc`, `1Wutc`, `1Mutc`, `3Mutc`), because its plain ones are cut at UTC+8
/// and the app's buckets are UTC.
pub(crate) fn bar(timeframe: Timeframe) -> Option<String> {
    if !DataSource::Okx.natives().contains(&timeframe) {
        return None;
    }
    let token = timeframe.interval();
    let (count, unit) = token.split_at(token.len() - 1);
    Some(match unit {
        "m" => token,
        "h" if count.parse::<u16>().ok()? < 6 => format!("{count}H"),
        _ => format!("{count}{}utc", unit.to_uppercase()),
    })
}

/// OKX's id of a market: `BTC-USDT`. `BTC/USDT` and a bare `BTC` (USDT) are accepted.
pub(crate) fn instrument_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Okx, symbol).api_symbol()
}

impl MarketSource for Okx {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Okx, timeframe)?;
        let requested = PAGING.requested(plan, limit as usize);
        if requested > PAGING.page {
            // More base candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let rows = fetch_page(
            "candles",
            &instrument_id(symbol),
            plan.base(),
            requested,
            None,
        )?;
        Ok(plan.shape(rows, requested, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Okx, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let id = instrument_id(symbol);
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            fetch_page("history-candles", &id, plan.base(), wanted, Some(after))
        })?;
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC-USDT`, `btc/usdt` and `BTCUSDT` are one query.
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = CATALOG.get(download_pairs)?;
        let results = matching_pairs(&pairs, &needle);
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}

fn fetch_page(
    endpoint: &str,
    id: &str,
    base: Timeframe,
    limit: usize,
    after: Option<i64>,
) -> Result<Vec<Kline>, ProviderError> {
    let bar = bar(base).ok_or_else(|| ProviderError::unsupported(DataSource::Okx, base))?;
    let mut url = format!("{BASE_URL}/market/{endpoint}?instId={id}&bar={bar}&limit={limit}");
    if let Some(after) = after {
        url.push_str(&format!("&after={after}"));
    }
    let body = http::get_text(&url, &[], http::Retry::Transient)?;
    degenview_market::okx::parse_candles(&body, base).map_err(ProviderError::from)
}

/// The listed markets whose name, without its dash, contains `needle` (letters and digits, upper
/// case).
fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| normalize(&pair.id).contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Okx, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable spot market from `/public/instruments`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    id: String,
    base: String,
    quote: String,
}

/// Every spot market that is live, downloaded at most once a day.
static CATALOG: Catalog<Pair> = Catalog::new("catalog_okx");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    let body = http::get_text(
        &format!("{BASE_URL}/public/instruments?instType=SPOT"),
        &[],
        http::Retry::Transient,
    )?;
    parse_pairs(&body)
}

fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    Ok(degenview_market::okx::data_rows(body)?
        .iter()
        .filter(|row| row.get("state").and_then(Value::as_str) == Some("live"))
        .filter_map(|row| {
            let field = |key: &str| row.get(key)?.as_str().filter(|text| !text.is_empty());
            Some(Pair {
                id: field("instId")?.to_owned(),
                base: field("baseCcy")?.to_owned(),
                quote: field("quoteCcy")?.to_owned(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_has_its_bar_and_nothing_else_does() {
        let bars: Vec<String> = DataSource::Okx
            .natives()
            .iter()
            .map(|native| bar(*native).expect("a native has a bar"))
            .collect();
        assert_eq!(
            bars,
            [
                "1m", "3m", "5m", "15m", "30m", "1H", "2H", "4H", "6Hutc", "12Hutc", "1Dutc",
                "1Wutc", "1Mutc", "3Mutc"
            ]
        );
        for other in ["2m", "8h", "2D", "3D", "2W", "2M", "6M", "1Y"] {
            assert_eq!(bar(Timeframe::parse(other).unwrap()), None, "{other}");
        }
    }

    fn base_of(timeframe: &str) -> (String, usize) {
        let plan = plan(DataSource::Okx, Timeframe::parse(timeframe).unwrap()).unwrap();
        (bar(plan.base()).unwrap(), plan.base_count(500))
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("1h"), ("1H".to_owned(), 500));
        assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
        assert_eq!(base_of("3h"), ("1H".to_owned(), 501 * 3));
        assert_eq!(base_of("8h"), ("4H".to_owned(), 501 * 2));
        assert_eq!(base_of("3D"), ("1Dutc".to_owned(), 501 * 3));
        assert_eq!(base_of("1Y"), ("3Mutc".to_owned(), 501 * 4));
    }

    #[test]
    fn symbols_are_okx_ids() {
        assert_eq!(instrument_id("btc-usdt"), "BTC-USDT");
        assert_eq!(instrument_id("ETH/USDC"), "ETH-USDC");
        assert_eq!(instrument_id("sol"), "SOL-USDT");
    }

    /// Run with `cargo test --lib okx::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_okx_serves_candles_pairs_and_a_chart_of_every_native() {
        let hits = OKX.search_tickers("btc-usdt").expect("search");
        assert_eq!(hits[0].symbol, "BTC-USDT");
        assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
        for native in DataSource::Okx.natives() {
            let candles = OKX.fetch_klines("BTC-USDT", *native, 350).expect("candles");
            // Months and quarters reach back only to the market's listing.
            if *native >= Timeframe::MONTH_1 {
                assert!(candles.len() > 30, "{native}");
            } else {
                assert_eq!(candles.len(), 350, "{native}");
            }
            assert!(
                candles
                    .windows(2)
                    .all(|pair| pair[0].open_time < pair[1].open_time),
                "{native}"
            );
        }
        // A 22-minute chart of 500 candles is eleven thousand one-minute ones: 37 pages.
        let folded = OKX
            .fetch_klines("BTC-USDT", Timeframe::parse("22m").unwrap(), 500)
            .expect("folded candles");
        assert_eq!(folded.len(), 500);
        let error = OKX
            .fetch_klines("NOPE-USDT", Timeframe::HOUR_1, 5)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "OKX: Instrument ID, Instrument ID code, or Spread ID doesn't exist."
        );
    }

    #[test]
    fn the_catalog_keeps_only_live_markets_with_all_their_fields() {
        let body = r#"{"code":"0","data":[
            {"instId":"BTC-USDT","state":"live","baseCcy":"BTC","quoteCcy":"USDT"},
            {"instId":"OLD-USDT","state":"suspend","baseCcy":"OLD","quoteCcy":"USDT"},
            {"instId":"NOQUOTE","state":"live","baseCcy":"X","quoteCcy":""}
        ]}"#;
        let pairs = parse_pairs(body).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].id.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTC-USDT", "BTC", "USDT")
        );
        assert!(parse_pairs("not json").is_err());
    }

    #[test]
    fn a_pair_is_found_however_it_is_separated() {
        let pairs = vec![
            Pair {
                id: "BTC-USDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
            },
            Pair {
                id: "ETH-BTC".into(),
                base: "ETH".into(),
                quote: "BTC".into(),
            },
        ];
        for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
            let found = matching_pairs(&pairs, &normalize(query));
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].symbol, "BTC-USDT");
        }
        assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
    }
}
