//! Bybit spot candles and its market list. Public v5 endpoints, no key.
//!
//! `/market/kline` answers up to 1,000 rows newest first, `[start ms, open, high, low, close,
//! volume, turnover]`, and pages back with an `end` cursor. Failures arrive as HTTP 200 with a
//! `retCode` other than 0; some regions are refused with HTTP 403.

use super::candles::plan;
use super::catalog::Catalog;
use super::paging::Paging;
use super::search_rank::normalize;
use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::net::http;
use crate::service::net::ws::Keepalive;
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub(crate) const BASE_URL: &str = "https://api.bybit.com/v5/market";

pub struct Bybit;

pub const BYBIT: Bybit = Bybit;

/// Bybit's heartbeat on every public socket: `{"op":"ping"}`, answered by a reply that starts
/// `{"success":true,"ret_msg":"pong"`.
pub const KEEPALIVE: Keepalive = Keepalive::Text {
    ping: r#"{"op":"ping"}"#,
    pong: r#"{"success":true,"ret_msg":"pong""#,
};

/// 1,000 rows a request and 60 pages (a 22-minute chart of 500 candles is 12). Bybit allows 600
/// requests per 5 seconds per IP, far more than a chart asks for, so the pages are not spaced.
const PAGING: Paging = Paging {
    page: 1_000,
    max_candles: 60_000,
    pause: Duration::ZERO,
};

/// Bybit's `interval` for a candle length it serves itself: minutes (`"1"`, `"60"`, `"720"`), or
/// `"D"`, `"W"` and `"M"`.
pub(crate) fn interval(timeframe: Timeframe) -> Option<String> {
    if !DataSource::Bybit.natives().contains(&timeframe) {
        return None;
    }
    Some(if timeframe == Timeframe::DAY_1 {
        "D".to_owned()
    } else if timeframe == Timeframe::WEEK_1 {
        "W".to_owned()
    } else if timeframe == Timeframe::MONTH_1 {
        "M".to_owned()
    } else {
        (timeframe.nominal_ms() / 60_000).to_string()
    })
}

/// Bybit's name of a market, `BTCUSDT`, which its REST API and its websocket take. `BTC/USDT`,
/// `btc-usdt` and a bare `BTC` (USDT) are accepted.
pub(crate) fn symbol_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Bybit, symbol).api_symbol()
}

impl MarketSource for Bybit {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Bybit, timeframe)?;
        let requested = PAGING.requested(plan, limit as usize);
        if requested > PAGING.page {
            // More base candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let rows = fetch_page(&symbol_id(symbol), plan.base(), requested, None)?;
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
        let plan = plan(DataSource::Bybit, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let id = symbol_id(symbol);
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            // `end` includes the candle that opens at it; `after` is exclusive.
            fetch_page(&id, plan.base(), wanted, Some(after - 1))
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
    id: &str,
    base: Timeframe,
    limit: usize,
    end_ms: Option<i64>,
) -> Result<Vec<Kline>, ProviderError> {
    let interval =
        interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Bybit, base))?;
    let mut url =
        format!("{BASE_URL}/kline?category=spot&symbol={id}&interval={interval}&limit={limit}");
    if let Some(end_ms) = end_ms {
        url.push_str(&format!("&end={end_ms}"));
    }
    parse_candles(&get(&url)?, base)
}

/// A request, with Bybit's regional refusal told apart from a network failure.
pub(crate) fn get(url: &str) -> Result<String, ProviderError> {
    match http::get_text(url, &[], http::Retry::Transient) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(403)) => Err(ProviderError::other(
            "Bybit refused the request (HTTP 403): it is not available in every country, and \
             refuses a client that asks too often",
        )),
        Err(error) => Err(error.into()),
    }
}

/// Whether a `retCode` says to slow down: 10006 (too many visits) and 10018 (the IP limit).
pub(crate) fn is_rate_limit(code: i64) -> bool {
    matches!(code, 10_006 | 10_018)
}

/// The `result` of an answer, or the provider's refusal.
fn result_of(body: &str) -> Result<Value, ProviderError> {
    let mut value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let code = value
        .get("retCode")
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::data("expected a retCode"))?;
    if code != 0 {
        if is_rate_limit(code) {
            return Err(ProviderError::Network(http::Error::Status(429)));
        }
        let message = value
            .get("retMsg")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("The request failed");
        return Err(ProviderError::other(format!("Bybit: {message}")));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| ProviderError::data("expected a result"))
}

/// Candles of length `base`, oldest first.
fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let result = result_of(body)?;
    let mut candles = result
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a candle list"))?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// One kline of a push (`start`, `open`, …) is the same row as an object: see `live/bybit.rs`.
fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let text = |index: usize| -> Result<&str, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::data(format!("invalid field {index}")))
    };
    let number = |index: usize| -> Result<f64, ProviderError> {
        text(index)?
            .parse()
            .map_err(|_| ProviderError::data(format!("invalid field {index}")))
    };
    let open_time: i64 = text(0)?
        .parse()
        .map_err(|_| ProviderError::data("invalid timestamp"))?;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(1)?,
        high: number(2)?,
        low: number(3)?,
        close: number(4)?,
        volume: number(5)?,
    })
}

/// The listed markets whose name contains `needle` (letters and digits, upper case).
fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| pair.id.contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Bybit, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable spot market from `instruments-info`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    id: String,
    base: String,
    quote: String,
}

/// Every spot market that is trading, downloaded at most once a day.
static CATALOG: Catalog<Pair> = Catalog::new("catalog_bybit");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

/// The most pages of the market list followed: 509 markets fit one page of 1,000.
const CATALOG_PAGES: usize = 10;

fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    let mut pairs = Vec::new();
    let mut cursor = String::new();
    for _ in 0..CATALOG_PAGES {
        let url = format!("{BASE_URL}/instruments-info?category=spot&limit=1000{cursor}");
        let (page, next) = parse_pairs(&get(&url)?)?;
        pairs.extend(page);
        match next {
            Some(next) => cursor = format!("&cursor={next}"),
            None => break,
        }
    }
    Ok(pairs)
}

/// The trading markets of a page and the cursor of the next one, if there is one.
fn parse_pairs(body: &str) -> Result<(Vec<Pair>, Option<String>), ProviderError> {
    let result = result_of(body)?;
    let pairs = result
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter(|row| row.get("status").and_then(Value::as_str) == Some("Trading"))
        .filter_map(|row| {
            let field = |key: &str| row.get(key)?.as_str().filter(|text| !text.is_empty());
            Some(Pair {
                id: field("symbol")?.to_owned(),
                base: field("baseCoin")?.to_owned(),
                quote: field("quoteCoin")?.to_owned(),
            })
        })
        .collect();
    let next = result
        .get("nextPageCursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    Ok((pairs, next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_has_its_interval_and_nothing_else_does() {
        let intervals: Vec<String> = DataSource::Bybit
            .natives()
            .iter()
            .map(|native| interval(*native).expect("a native has an interval"))
            .collect();
        assert_eq!(
            intervals,
            [
                "1", "3", "5", "15", "30", "60", "120", "240", "360", "720", "D", "W", "M"
            ]
        );
        for other in ["2m", "10m", "45m", "8h", "2D", "2W", "3M", "1Y"] {
            assert_eq!(interval(Timeframe::parse(other).unwrap()), None, "{other}");
        }
    }

    fn base_of(timeframe: &str) -> (String, usize) {
        let plan = plan(DataSource::Bybit, Timeframe::parse(timeframe).unwrap()).unwrap();
        (interval(plan.base()).unwrap(), plan.base_count(500))
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("1h"), ("60".to_owned(), 500));
        assert_eq!(base_of("22m"), ("1".to_owned(), 501 * 22));
        assert_eq!(base_of("8h"), ("240".to_owned(), 501 * 2));
        assert_eq!(base_of("3D"), ("D".to_owned(), 501 * 3));
        assert_eq!(base_of("3M"), ("M".to_owned(), 501 * 3));
        assert_eq!(base_of("1Y"), ("M".to_owned(), 501 * 12));
    }

    #[test]
    fn a_request_wider_than_the_paging_is_capped_before_it_is_folded() {
        // 500 candles of 1,439 minutes (nothing larger folds from one-minute bars) is 720,939
        // base candles; paging reaches 60,000.
        let plan = plan(DataSource::Bybit, Timeframe::parse("1439m").unwrap()).unwrap();
        assert_eq!(PAGING.requested(plan, 500), PAGING.max_candles);
        let hour = super::plan(DataSource::Bybit, Timeframe::HOUR_1).unwrap();
        assert_eq!(PAGING.requested(hour, 500), 500);
    }

    #[test]
    fn symbols_are_bybits_however_they_are_spelled() {
        assert_eq!(symbol_id("btc/usdt"), "BTCUSDT");
        assert_eq!(symbol_id("ETH-USDC"), "ETHUSDC");
        assert_eq!(symbol_id("sol"), "SOLUSDT");
        assert_eq!(symbol_id("WBTC"), "WBTCUSDT");
    }

    const ROWS: &str = r#"{"retCode":0,"retMsg":"OK","result":{"category":"spot","symbol":"BTCUSDT","list":[
        ["1700002800000","11","13","10","12","5.5","60"],
        ["1699999200000","10","12","9","11","42.5","450"]
    ]}}"#;

    #[test]
    fn rows_come_newest_first_and_leave_oldest_first_with_the_close_of_their_bucket() {
        let candles = parse_candles(ROWS, Timeframe::HOUR_1).expect("valid response");
        assert_eq!(candles.len(), 2);
        assert_eq!(
            candles[0],
            Kline {
                open_time: 1_699_999_200_000,
                close_time: 1_700_002_799_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }
        );
        assert_eq!(candles[1].open_time, 1_700_002_800_000);
    }

    #[test]
    fn a_month_closes_when_the_next_calendar_month_opens() {
        // 2026-02-01 00:00 UTC.
        let row = serde_json::json!(["1769904000000", "1", "2", "0.5", "1.5", "3", "4"]);
        let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
        assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
    }

    #[test]
    fn a_refusal_is_bybits_message_and_the_limits_are_rate_limits() {
        let unknown = r#"{"retCode":10001,"retMsg":"Not supported symbols","result":{}}"#;
        assert_eq!(
            parse_candles(unknown, Timeframe::HOUR_1),
            Err(ProviderError::other("Bybit: Not supported symbols"))
        );
        for code in [10_006, 10_018] {
            let limited =
                format!(r#"{{"retCode":{code},"retMsg":"Too many visits!","result":{{}}}}"#);
            let error = parse_candles(&limited, Timeframe::HOUR_1).unwrap_err();
            assert!(error.is_rate_limited(), "{code}: {error}");
        }
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"retCode":0,"result":{}}"#, Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"result":{"list":[]}}"#, Timeframe::HOUR_1).is_err());
        let bad_field =
            r#"{"retCode":0,"result":{"list":[["1700000000000","x","1","1","1","1","1"]]}}"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
    }

    #[test]
    fn the_catalog_keeps_only_trading_markets_and_follows_its_cursor() {
        let body = r#"{"retCode":0,"result":{"category":"spot","nextPageCursor":"page2","list":[
            {"symbol":"BTCUSDT","baseCoin":"BTC","quoteCoin":"USDT","status":"Trading"},
            {"symbol":"OLDUSDT","baseCoin":"OLD","quoteCoin":"USDT","status":"Closed"},
            {"symbol":"NOQUOTE","baseCoin":"X","quoteCoin":"","status":"Trading"}
        ]}}"#;
        let (pairs, next) = parse_pairs(body).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].id.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTCUSDT", "BTC", "USDT")
        );
        assert_eq!(next.as_deref(), Some("page2"));
        let last = r#"{"retCode":0,"result":{"nextPageCursor":"","list":[]}}"#;
        assert_eq!(parse_pairs(last).unwrap().1, None);
        assert!(parse_pairs("not json").is_err());
    }

    #[test]
    fn a_pair_is_found_however_it_is_separated() {
        let pairs = vec![
            Pair {
                id: "BTCUSDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
            },
            Pair {
                id: "ETHBTC".into(),
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

    /// Run with `cargo test --lib bybit::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_bybit_serves_candles_pairs_and_every_native() {
        let hits = BYBIT.search_tickers("btc-usdt").expect("search");
        assert_eq!(hits[0].symbol, "BTCUSDT");
        assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
        for native in DataSource::Bybit.natives() {
            // More than one page of the shortest bars, so paging is exercised.
            let wanted = if *native == Timeframe::MINUTE_1 {
                1_500
            } else {
                350
            };
            let candles = BYBIT
                .fetch_klines("BTCUSDT", *native, wanted)
                .expect("candles");
            // Weeks and months reach back only to the market's listing.
            if *native >= Timeframe::WEEK_1 {
                assert!(candles.len() > 30, "{native}");
            } else {
                assert_eq!(candles.len(), wanted as usize, "{native}");
            }
            assert!(
                candles
                    .windows(2)
                    .all(|pair| pair[0].open_time < pair[1].open_time),
                "{native}"
            );
        }
        // A 22-minute chart of 500 candles is eleven thousand one-minute ones: 12 pages.
        let folded = BYBIT
            .fetch_klines("BTCUSDT", Timeframe::parse("22m").unwrap(), 500)
            .expect("folded candles");
        assert_eq!(folded.len(), 500);
        let error = BYBIT
            .fetch_klines("NOPEUSDT", Timeframe::HOUR_1, 5)
            .unwrap_err();
        assert_eq!(error.to_string(), "Bybit: Not supported symbols");
    }
}
