use super::candles::plan;
use super::catalog::Catalog;
use super::search_rank::normalize;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BASE_URL: &str = "https://api.binance.com/api/v3/klines";

pub struct Binance;

pub const BINANCE: Binance = Binance;

/// The most klines one Binance request returns.
const MAX_PER_REQUEST: usize = 1_000;

impl MarketSource for Binance {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Binance, timeframe)?;
        let requested = plan.base_count(limit as usize);
        if requested > MAX_PER_REQUEST {
            // More finer candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let url = format!(
            "{BASE_URL}?symbol={}&interval={}&limit={requested}",
            symbol.to_uppercase(),
            plan.base().interval()
        );
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Transient,
        )?;
        Ok(plan.shape(parse_klines(&body)?, requested, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        const PAGE: usize = MAX_PER_REQUEST;
        // Pages are fetched side by side: their start times follow from the interval alone.
        const PARALLEL: usize = 8;
        let plan = plan(DataSource::Binance, timeframe)?;
        let interval = plan.base().interval();
        let requested = plan.base_count(max);
        let step = plan.base().nominal_ms();
        let page_ms = step * PAGE as i64;
        let first = (end_ms - step * requested as i64).max(start_ms);
        let starts: Vec<i64> = (0..)
            .map(|index| first + index * page_ms)
            .take_while(|start| *start < end_ms)
            .collect();
        let mut candles: Vec<Kline> = Vec::new();
        for batch in starts.chunks(PARALLEL) {
            let pages: Vec<Result<Vec<Kline>, ProviderError>> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|&from| {
                        let interval = &interval;
                        scope.spawn(move || {
                            let to = (from + page_ms).min(end_ms) - 1;
                            let url = format!(
                                "{BASE_URL}?symbol={}&interval={interval}&startTime={from}&endTime={to}&limit={PAGE}",
                                symbol.to_uppercase()
                            );
                            let body =
                                crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)?;
                            parse_klines(&body)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().unwrap_or_else(|_| {
                            Err(ProviderError::other("The candle download stopped."))
                        })
                    })
                    .collect()
            });
            for page in pages {
                candles.extend(page?);
            }
        }
        candles.sort_by_key(|candle| candle.open_time);
        candles.dedup_by_key(|candle| candle.open_time);
        if candles.len() > requested {
            candles.drain(..candles.len() - requested);
        }
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC-USDT` and `btc/usdt` find `BTCUSDT`: Binance writes no separator.
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = CATALOG.get(download_pairs)?;
        let results = matching_pairs(&pairs, &needle);
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}

/// The listed markets whose name contains `needle` (letters and digits, upper case).
fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| pair.symbol.contains(needle))
        .map(|pair| {
            SearchResult::new(
                crate::model::DataSource::Binance,
                pair.symbol.clone(),
                pair.symbol.clone(),
            )
            .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable market from `exchangeInfo`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    symbol: String,
    base: String,
    quote: String,
}

/// Every market that is trading, downloaded at most once a day.
static CATALOG: Catalog<Pair> = Catalog::new("catalog_binance");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

/// The market list, asking only for what the search reads. Plain `exchangeInfo` is 18 MB (every
/// market ever listed, 3,700 of them, each with its filters and its permission sets); the
/// `TRADING` ones without permission sets are 2.6 MB, the same pairs `parse_pairs` keeps.
const EXCHANGE_INFO_URL: &str =
    "https://api.binance.com/api/v3/exchangeInfo?symbolStatus=TRADING&showPermissionSets=false";

/// What this one download may read: six times the 2.6 MB it is today, so listings can grow
/// without breaking search, and far under the plain list. The shared 8 MiB limit stays for
/// every other call.
const EXCHANGE_INFO_MAX_BYTES: u64 = 16 * 1024 * 1024;

fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    download_pairs_with(|url, max_bytes| {
        crate::service::net::http::get_text_with_limit(
            url,
            &[],
            crate::service::net::http::Retry::Transient,
            max_bytes,
        )
    })
}

/// `download_pairs` with the transport passed in, so a test sees the request without a network.
fn download_pairs_with(
    get: impl FnOnce(&str, u64) -> Result<String, crate::service::net::http::Error>,
) -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get(EXCHANGE_INFO_URL, EXCHANGE_INFO_MAX_BYTES)?)
}

fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .get("symbols")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|symbol| symbol.get("status").and_then(Value::as_str) == Some("TRADING"))
        .filter_map(|symbol| {
            Some(Pair {
                symbol: symbol.get("symbol")?.as_str()?.to_owned(),
                base: symbol.get("baseAsset")?.as_str()?.to_owned(),
                quote: symbol.get("quoteAsset")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

fn parse_klines(body: &str) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let rows = value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a candle array"))?;
    rows.iter().map(parse_row).collect()
}

fn parse_row(row: &Value) -> Result<Kline, ProviderError> {
    let value = |index: usize| -> Result<f64, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| ProviderError::data(format!("invalid field {index}")))
    };
    let time = |index: usize| -> Result<i64, ProviderError> {
        row.get(index)
            .and_then(Value::as_i64)
            .ok_or_else(|| ProviderError::data(format!("invalid timestamp {index}")))
    };

    Ok(Kline {
        open_time: time(0)?,
        open: value(1)?,
        high: value(2)?,
        low: value(3)?,
        close: value(4)?,
        volume: value(5)?,
        close_time: time(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_is_found_however_it_is_separated() {
        let pairs = vec![
            Pair {
                symbol: "BTCUSDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
            },
            Pair {
                symbol: "ETHBTC".into(),
                base: "ETH".into(),
                quote: "BTC".into(),
            },
        ];
        for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
            let found = matching_pairs(&pairs, &normalize(query));
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].symbol, "BTCUSDT");
        }
        assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
    }

    #[test]
    fn parses_binance_rows() {
        let body = r#"[[1700000000000,"10.0","12.0","9.0","11.0","42.5",1700003599999]]"#;
        let candles = parse_klines(body).expect("valid response");
        assert_eq!(candles.len(), 1);
        assert_eq!(
            candles[0],
            Kline {
                open_time: 1_700_000_000_000,
                close_time: 1_700_003_599_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }
        );
    }

    fn base_of(timeframe: &str) -> (String, usize) {
        let plan = plan(DataSource::Binance, Timeframe::parse(timeframe).unwrap()).unwrap();
        (plan.base().interval(), plan.base_count(500))
    }

    #[test]
    fn a_native_length_is_asked_for_as_it_is() {
        for token in [
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h",
        ] {
            assert_eq!(base_of(token), (token.to_owned(), 500), "{token}");
        }
        assert_eq!(base_of("1d"), ("1d".to_owned(), 500));
        assert_eq!(base_of("1w"), ("1w".to_owned(), 500));
        assert_eq!(base_of("1M"), ("1M".to_owned(), 500));
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
        assert_eq!(base_of("6m"), ("3m".to_owned(), 501 * 2));
        assert_eq!(base_of("3h"), ("1h".to_owned(), 501 * 3));
        assert_eq!(base_of("10h"), ("2h".to_owned(), 501 * 5));
        assert_eq!(base_of("24h"), ("12h".to_owned(), 501 * 2));
        // Binance's own 3d is not on the grid the other sources cut, so days are folded.
        assert_eq!(base_of("3d"), ("1d".to_owned(), 501 * 3));
        assert_eq!(base_of("2w"), ("1w".to_owned(), 501 * 2));
        assert_eq!(base_of("3M"), ("1M".to_owned(), 501 * 3));
        assert_eq!(base_of("1Y"), ("1M".to_owned(), 501 * 12));
    }

    #[test]
    fn a_series_of_more_than_one_requests_candles_is_paged_but_its_tail_is_not() {
        assert!(base_of("5m").1 <= MAX_PER_REQUEST);
        assert!(base_of("22m").1 > MAX_PER_REQUEST);
        let tail = plan(DataSource::Binance, Timeframe::parse("22m").unwrap())
            .unwrap()
            .base_count(crate::service::cache::TAIL_LIMIT as usize);
        assert!(tail <= MAX_PER_REQUEST);
    }

    #[test]
    fn rejects_error_objects() {
        let error = parse_klines(r#"{"code":-1121,"msg":"Invalid symbol."}"#)
            .expect_err("object must fail");
        assert_eq!(error, ProviderError::data("expected a candle array"));
    }

    #[test]
    fn the_market_list_asks_for_trading_pairs_only_under_its_own_limit() {
        use crate::service::net::http::Error;
        let body = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT"}
        ]}"#;
        let mut asked = None;
        let pairs = download_pairs_with(|url, max_bytes| {
            asked = Some((url.to_owned(), max_bytes));
            Ok(body.to_owned())
        })
        .expect("pairs");
        assert_eq!(pairs.len(), 1);

        let (url, max_bytes) = asked.expect("one request");
        assert!(url.starts_with("https://api.binance.com/api/v3/exchangeInfo?"));
        assert!(url.contains("symbolStatus=TRADING"));
        assert!(url.contains("showPermissionSets=false"));
        // Room for the list to grow past today's 2.6 MB, but not the shared limit's job.
        assert!(max_bytes > 8 * 1024 * 1024);
        assert!(max_bytes <= 32 * 1024 * 1024);

        // A too-large answer is still an error and not a partial list.
        let too_big = download_pairs_with(|_, max_bytes| Err(Error::BodyTooLarge(max_bytes)));
        assert_eq!(
            too_big.expect_err("fails"),
            ProviderError::from(Error::BodyTooLarge(EXCHANGE_INFO_MAX_BYTES))
        );
    }

    #[test]
    fn the_catalog_keeps_only_trading_markets_with_all_their_fields() {
        let body = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT"},
            {"symbol":"LUNAUSDT","status":"BREAK","baseAsset":"LUNA","quoteAsset":"USDT"},
            {"symbol":"NOQUOTE","status":"TRADING","baseAsset":"X"}
        ]}"#;
        let pairs = parse_pairs(body).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].symbol.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTCUSDT", "BTC", "USDT")
        );
        assert!(parse_pairs("not json").is_err());
        assert!(parse_pairs("{}").expect("no symbols").is_empty());
    }
}
