//! Bybit spot candles and its market list. Public v5 endpoints, no key.
//!
//! `/market/kline` answers up to 1,000 rows newest first, `[start ms, open, high, low, close,
//! volume, turnover]`, and pages back with an `end` cursor. Failures arrive as HTTP 200 with a
//! `retCode` other than 0; some regions are refused with HTTP 403.

use super::catalog::Catalog;
use super::catalog::search::{self, ListedMarket};
use super::kline_endpoint::{self, KlineEndpoint};
use super::paging::Paging;
use super::ports::clock::Clock;
use super::ports::context::MarketContext;
use super::ports::http::{Http, Request};
use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::{DataSource, InstrumentId, Kline, Listing, Timeframe};
use crate::service::market::error::{ErrorKind, Statuses};
use crate::service::net::ws::Keepalive;
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const BASE_URL: &str = "https://api.bybit.com/v5/market";

pub struct Bybit {
    http: Arc<dyn Http>,
    clock: Arc<dyn Clock>,
    /// Every spot market that is trading, downloaded at most once a day.
    pub(super) catalog: Catalog<Pair>,
}

impl Bybit {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            clock: context.clock.clone(),
            catalog: Catalog::new(
                Listing::spot(DataSource::Bybit),
                "catalog_bybit",
                context.cache.clone(),
            ),
        }
    }
}

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
    InstrumentId::new(DataSource::Bybit, symbol).canonical_symbol()
}

/// Bybit's `/market/kline` endpoint.
impl KlineEndpoint for Bybit {
    const SOURCE: DataSource = DataSource::Bybit;
    const PAGING: Paging = PAGING;

    fn id(&self, symbol: &str) -> String {
        symbol_id(symbol)
    }

    fn page(
        &self,
        id: &str,
        base: Timeframe,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<Kline>, ProviderError> {
        // `end` includes the candle that opens at it; `before` is exclusive.
        self.fetch_page(id, base, limit, before.map(|before| before - 1))
    }
}

impl MarketSource for Bybit {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        kline_endpoint::latest(self, self.clock.now_ms(), symbol, timeframe, limit as usize)
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        kline_endpoint::range(self, symbol, timeframe, start_ms, end_ms, max)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        search::find(query, || self.catalog.get(|| self.download_pairs()))
    }
}

impl Bybit {
    fn fetch_page(
        &self,
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
        parse_candles(&self.get(&url)?, base)
    }

    /// A request, with Bybit's regional refusal told apart from a network failure.
    pub fn get(&self, url: &str) -> Result<String, ProviderError> {
        self.http
            .fetch(DataSource::Bybit, &Request::get(url).statuses(STATUSES))
    }
}

/// What Bybit's HTTP statuses mean beyond a rate limit, at every endpoint (none of them names a
/// market by status): some regions are refused with a 403, as is a client that asks too often.
pub(in crate::service) const STATUSES: Statuses = &[(403, ErrorKind::Blocked)];

/// Whether a `retCode` says to slow down: 10006 (too many visits) and 10018 (the IP limit).
fn is_rate_limit(code: i64) -> bool {
    matches!(code, 10_006 | 10_018)
}

/// The `result` of an answer, or the provider's refusal. Failures arrive as HTTP 200 with a
/// `retCode` other than 0.
pub(crate) fn result_of(body: &str) -> Result<Value, ProviderError> {
    let mut value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Bybit, error))?;
    let code = value
        .get("retCode")
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::malformed(DataSource::Bybit, "expected a retCode"))?;
    if code != 0 {
        if is_rate_limit(code) {
            return Err(ProviderError::new(
                DataSource::Bybit,
                ErrorKind::RateLimited,
            ));
        }
        return Err(ProviderError::refused_because(
            DataSource::Bybit,
            value.get("retMsg").and_then(Value::as_str),
        ));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| ProviderError::malformed(DataSource::Bybit, "expected a result"))
}

/// Candles of length `base`, oldest first.
fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let result = result_of(body)?;
    let mut candles = result
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::malformed(DataSource::Bybit, "expected a candle list"))?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// One kline of a push (`start`, `open`, …) is the same row as an object: see `live/bybit.rs`.
fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let text = |index: usize| -> Result<&str, ProviderError> {
        row.get(index).and_then(Value::as_str).ok_or_else(|| {
            ProviderError::malformed(DataSource::Bybit, format!("invalid field {index}"))
        })
    };
    let number = |index: usize| -> Result<f64, ProviderError> {
        text(index)?.parse().map_err(|_| {
            ProviderError::malformed(DataSource::Bybit, format!("invalid field {index}"))
        })
    };
    let open_time: i64 = text(0)?
        .parse()
        .map_err(|_| ProviderError::malformed(DataSource::Bybit, "invalid timestamp"))?;
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

/// One tradable spot market from `instruments-info`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pair {
    id: String,
    base: String,
    quote: String,
}

impl ListedMarket for Pair {
    const SOURCE: DataSource = DataSource::Bybit;

    fn id(&self) -> &str {
        &self.id
    }

    fn sides(&self) -> Option<(&str, &str)> {
        Some((&self.base, &self.quote))
    }
}

/// The most pages of the market list followed: 509 markets fit one page of 1,000.
const CATALOG_PAGES: usize = 10;

impl Bybit {
    fn download_pairs(&self) -> Result<Vec<Pair>, ProviderError> {
        let mut pairs = Vec::new();
        let mut cursor = String::new();
        for _ in 0..CATALOG_PAGES {
            let url = format!("{BASE_URL}/instruments-info?category=spot&limit=1000{cursor}");
            let (page, next) = parse_pairs(&self.get(&url)?)?;
            pairs.extend(page);
            match next {
                Some(next) => cursor = format!("&cursor={next}"),
                None => break,
            }
        }
        Ok(pairs)
    }
}

/// The trading markets of a page and the cursor of the next one, if there is one.
fn parse_pairs(body: &str) -> Result<(Vec<Pair>, Option<String>), ProviderError> {
    let result = result_of(body)?;
    let pairs = result
        .get("list")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::malformed(DataSource::Bybit, "expected a market list"))?
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
    use crate::service::market::candles::plan;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::net::http::{self, Retry};
    use std::sync::Arc;

    const MINUTE: i64 = 60_000;
    const NOON: i64 = Rig::NOON;

    fn venue(rig: &Rig) -> Bybit {
        Bybit::new(&rig.context())
    }

    /// An answer of `count` one-minute rows, newest first, the newest opening at `newest`.
    fn minute_rows(newest: i64, count: usize) -> String {
        let rows: Vec<String> = (0..count as i64)
            .map(|back| {
                format!(
                    r#"["{}","10","12","9","11","1.5","15"]"#,
                    newest - back * MINUTE
                )
            })
            .collect();
        format!(
            r#"{{"retCode":0,"retMsg":"OK","result":{{"category":"spot","symbol":"BTCUSDT","list":[{}]}}}}"#,
            rows.join(",")
        )
    }

    fn opens(candles: &[Kline]) -> Vec<i64> {
        candles.iter().map(|candle| candle.open_time).collect()
    }

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
            Err(ProviderError::refused(
                DataSource::Bybit,
                "Not supported symbols"
            ))
        );
        for code in [10_006, 10_018] {
            let limited =
                format!(r#"{{"retCode":{code},"retMsg":"Too many visits!","result":{{}}}}"#);
            let error = parse_candles(&limited, Timeframe::HOUR_1).unwrap_err();
            assert_eq!(
                error,
                ProviderError::new(DataSource::Bybit, ErrorKind::RateLimited),
                "{code}"
            );
        }
        let region =
            ProviderError::http_with(DataSource::Bybit, STATUSES, http::Error::Status(403));
        assert_eq!(region.kind, ErrorKind::Blocked);
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
        let pairs = Arc::new(pairs);
        let ask = |query: &str| search::find(query, || Ok(Arc::clone(&pairs))).unwrap();
        for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
            let found = ask(query);
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].source, DataSource::Bybit);
            assert_eq!(found[0].symbol, "BTCUSDT");
            assert_eq!(found[0].base.as_deref(), Some("BTC"));
            assert_eq!(found[0].quote.as_deref(), Some("USDT"));
        }
        assert_eq!(ask("BTC").len(), 2);
    }

    #[test]
    fn the_newest_candles_are_one_request_and_come_back_oldest_first() {
        let rig = Rig::new();
        rig.http.on("/kline?", ROWS);
        let candles = venue(&rig)
            .fetch_klines("btc/usdt", Timeframe::HOUR_1, 2)
            .expect("candles");
        assert_eq!(
            rig.http
                .sent()
                .iter()
                .map(|sent| (sent.url.as_str(), sent.retry))
                .collect::<Vec<_>>(),
            [(
                "https://api.bybit.com/v5/market/kline?category=spot&symbol=BTCUSDT&interval=60&limit=2",
                Retry::Transient
            )]
        );
        assert_eq!(opens(&candles), [1_699_999_200_000, 1_700_002_800_000]);
    }

    #[test]
    fn a_range_is_paged_back_by_the_end_cursor_of_each_page() {
        let rig = Rig::new();
        let end = NOON;
        rig.http
            .on("&limit=1000&end=", &minute_rows(end - MINUTE, 1_000));
        rig.http
            .on("&limit=500&end=", &minute_rows(end - 1_001 * MINUTE, 500));
        let candles = venue(&rig)
            .fetch_klines_range("BTCUSDT", Timeframe::MINUTE_1, 0, end, 1_500)
            .expect("candles");
        // `end` includes the candle that opens at it, so each cursor is a millisecond before the
        // oldest candle held.
        let base = "https://api.bybit.com/v5/market/kline?category=spot&symbol=BTCUSDT&interval=1";
        assert_eq!(
            rig.http.urls(),
            [
                format!("{base}&limit=1000&end={}", end - 1),
                format!("{base}&limit=500&end={}", end - 1_000 * MINUTE - 1),
            ]
        );
        assert_eq!(candles.len(), 1_500);
        assert_eq!(candles[0].open_time, end - 1_500 * MINUTE);
        assert_eq!(candles[1_499].open_time, end - MINUTE);
    }

    #[test]
    fn a_long_request_for_the_newest_pages_back_from_the_clock() {
        let rig = Rig::new();
        rig.http
            .on("&limit=1000&end=", &minute_rows(NOON - MINUTE, 1_000));
        rig.http
            .on("&limit=200&end=", &minute_rows(NOON - 1_001 * MINUTE, 200));
        let candles = venue(&rig)
            .fetch_klines("BTCUSDT", Timeframe::MINUTE_1, 1_200)
            .expect("candles");
        let urls = rig.http.urls();
        assert!(
            urls[0].ends_with(&format!("&limit=1000&end={NOON}")),
            "{urls:?}"
        );
        assert_eq!(urls.len(), 2);
        assert_eq!(candles.len(), 1_200);
    }

    #[test]
    fn a_length_bybit_cannot_serve_is_refused_before_any_request() {
        let rig = Rig::new();
        let error = venue(&rig)
            .fetch_klines("BTCUSDT", Timeframe::SECOND_1, 10)
            .unwrap_err();
        assert!(error.is_unsupported_interval());
        assert!(rig.http.urls().is_empty());
    }

    #[test]
    fn what_bybit_refuses_is_told_by_kind() {
        let kind_of = |answer: Result<&str, http::Error>| {
            let rig = Rig::new();
            match answer {
                Ok(body) => rig.http.on("/kline?", body),
                Err(error) => rig.http.fail("/kline?", error),
            };
            venue(&rig)
                .fetch_klines("BTCUSDT", Timeframe::HOUR_1, 5)
                .unwrap_err()
                .kind
        };
        assert_eq!(
            kind_of(Ok(
                r#"{"retCode":10001,"retMsg":"Not supported symbols","result":{}}"#
            )),
            ErrorKind::Refused(Some("Not supported symbols".to_owned()))
        );
        assert_eq!(
            kind_of(Ok(
                r#"{"retCode":10006,"retMsg":"Too many visits!","result":{}}"#
            )),
            ErrorKind::RateLimited
        );
        assert_eq!(kind_of(Err(http::Error::Status(403))), ErrorKind::Blocked);
        assert_eq!(
            kind_of(Err(http::Error::Status(500))),
            ErrorKind::Transport(http::Error::Status(500))
        );
    }

    #[test]
    fn the_market_list_follows_its_cursor_once_a_day_and_is_searched_locally() {
        let rig = Rig::new();
        rig.http.on(
            "instruments-info?category=spot&limit=1000&cursor=page2",
            r#"{"retCode":0,"result":{"nextPageCursor":"","list":[
                {"symbol":"ETHBTC","baseCoin":"ETH","quoteCoin":"BTC","status":"Trading"}]}}"#,
        );
        rig.http.on(
            "instruments-info?category=spot&limit=1000",
            r#"{"retCode":0,"result":{"nextPageCursor":"page2","list":[
                {"symbol":"BTCUSDT","baseCoin":"BTC","quoteCoin":"USDT","status":"Trading"}]}}"#,
        );
        let bybit = venue(&rig);
        let found = bybit.search_tickers("btc").expect("hits");
        assert_eq!(
            found
                .iter()
                .map(|hit| hit.symbol.as_str())
                .collect::<Vec<_>>(),
            ["BTCUSDT", "ETHBTC"]
        );
        assert_eq!(rig.http.urls().len(), 2);
        // A second search, and one after a relaunch, are answered from what was kept.
        bybit.search_tickers("eth").expect("hits");
        venue(&rig).search_tickers("eth").expect("hits");
        assert_eq!(rig.http.urls().len(), 2);
    }

    /// Run with `cargo test --lib bybit::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_bybit_serves_candles_pairs_and_every_native() {
        let hits = crate::service::wiring::venues()
            .bybit
            .search_tickers("btc-usdt")
            .expect("search");
        assert_eq!(hits[0].symbol, "BTCUSDT");
        assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
        for native in DataSource::Bybit.natives() {
            // More than one page of the shortest bars, so paging is exercised.
            let wanted = if *native == Timeframe::MINUTE_1 {
                1_500
            } else {
                350
            };
            let candles = crate::service::wiring::venues()
                .bybit
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
        let folded = crate::service::wiring::venues()
            .bybit
            .fetch_klines("BTCUSDT", Timeframe::parse("22m").unwrap(), 500)
            .expect("folded candles");
        assert_eq!(folded.len(), 500);
        let error = crate::service::wiring::venues()
            .bybit
            .fetch_klines("NOPEUSDT", Timeframe::HOUR_1, 5)
            .unwrap_err();
        assert_eq!(
            error,
            ProviderError::refused(DataSource::Bybit, "Not supported symbols")
        );
    }
}
