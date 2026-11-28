//! MEXC spot candles and its market list. Public v3 endpoints, no key.
//!
//! `/klines` answers up to 500 rows oldest first, `[open ms, open, high, low, close, volume,
//! close ms, quote volume]`, and takes either a `limit` (the newest rows) or a `startTime` and
//! `endTime` window; an `endTime` alone is ignored. Failures are HTTP 400 with a JSON code, and
//! some regions get 403.

use super::catalog::Catalog;
use super::catalog::search::{self, ListedMarket};
use super::kline_endpoint::{self, KlineEndpoint};
use super::paging::Paging;
use super::ports::clock::Clock;
use super::ports::context::MarketContext;
use super::ports::http::{Http, Request};
use crate::model::market::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::{DataSource, InstrumentId, Kline, Listing, Timeframe};
use crate::service::market::error::{Asked, ErrorKind, Statuses};
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const BASE_URL: &str = "https://api.mexc.com/api/v3";

pub struct Mexc {
    http: Arc<dyn Http>,
    clock: Arc<dyn Clock>,
    /// Every spot market that is enabled, downloaded at most once a day.
    pub(super) catalog: Catalog<Pair>,
}

impl Mexc {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            clock: context.clock.clone(),
            catalog: Catalog::new(
                Listing::spot(DataSource::Mexc),
                "catalog_mexc_v2",
                context.cache.clone(),
            ),
        }
    }
}

/// 500 rows a request and 20,000 base candles (a 22-minute chart of 500 candles is 22 pages, and
/// that is two years of hours). Its one-minute candles reach back about a month. Nothing limits
/// the pages that is worth spacing them for.
const PAGING: Paging = Paging {
    page: 500,
    max_candles: 20_000,
    pause: Duration::ZERO,
};

/// The oldest start a window may have: a `startTime` of 0 answers nothing.
const EARLIEST_MS: i64 = 1_500_000_000_000;

/// MEXC's `interval` for a candle length it serves itself: `1m`, `60m`, `4h`, `1d`, `1W`, `1M`.
pub(crate) fn interval(timeframe: Timeframe) -> Option<&'static str> {
    table(timeframe).map(|(rest, _)| rest)
}

/// The name of that interval in its websocket topics: `Min1`, `Min60`, `Hour4`, `Day1`, `Week1`,
/// `Month1`.
pub(crate) fn socket_interval(timeframe: Timeframe) -> Option<&'static str> {
    table(timeframe).map(|(_, socket)| socket)
}

fn table(timeframe: Timeframe) -> Option<(&'static str, &'static str)> {
    if !DataSource::Mexc.natives().contains(&timeframe) {
        return None;
    }
    [
        (Timeframe::MINUTE_1, ("1m", "Min1")),
        (Timeframe::MINUTE_5, ("5m", "Min5")),
        (Timeframe::MINUTE_15, ("15m", "Min15")),
        (Timeframe::MINUTE_30, ("30m", "Min30")),
        (Timeframe::HOUR_1, ("60m", "Min60")),
        (Timeframe::HOUR_4, ("4h", "Hour4")),
        (Timeframe::HOUR_8, ("8h", "Hour8")),
        (Timeframe::DAY_1, ("1d", "Day1")),
        (Timeframe::WEEK_1, ("1W", "Week1")),
        (Timeframe::MONTH_1, ("1M", "Month1")),
    ]
    .into_iter()
    .find_map(|(native, names)| (native == timeframe).then_some(names))
}

/// MEXC's name of a market, `BTCUSDT`. `BTC/USDT`, `btc-usdt` and a bare `BTC` (USDT) are
/// accepted.
pub(crate) fn symbol_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Mexc, symbol).canonical_symbol()
}

/// MEXC's `/klines` endpoint.
impl KlineEndpoint for Mexc {
    const SOURCE: DataSource = DataSource::Mexc;
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
        let Some(before) = before else {
            return self.fetch_page(id, base, limit, None);
        };
        let window = window_before(before, limit, base);
        if window.0 >= window.1 {
            return Ok(Vec::new());
        }
        self.fetch_page(id, base, limit, Some(window))
    }
}

impl MarketSource for Mexc {
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

    fn asset_names(&self) -> Vec<(String, String)> {
        search::asset_names(|| self.catalog.get(|| self.download_pairs()))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        search::find(query, || self.catalog.get(|| self.download_pairs()))
    }
}

/// The window `[start, end]` in milliseconds that holds `wanted` candles of `base` just before
/// `after_ms`: a window, because the end alone is ignored. A month is taken at its shortest, so
/// the window never holds more than the 500 a request returns.
fn window_before(after_ms: i64, wanted: usize, base: Timeframe) -> (i64, i64) {
    let step = if base == Timeframe::MONTH_1 {
        28 * 86_400_000
    } else {
        base.nominal_ms()
    };
    let end = after_ms - 1;
    let start = (after_ms - wanted as i64 * step).max(EARLIEST_MS);
    (start, end)
}

impl Mexc {
    fn fetch_page(
        &self,
        id: &str,
        base: Timeframe,
        limit: usize,
        window: Option<(i64, i64)>,
    ) -> Result<Vec<Kline>, ProviderError> {
        let interval =
            interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Mexc, base))?;
        let mut url = format!("{BASE_URL}/klines?symbol={id}&interval={interval}&limit={limit}");
        if let Some((start, end)) = window {
            url.push_str(&format!("&startTime={start}&endTime={end}"));
        }
        parse_candles(&self.get(&url, MARKET_STATUSES)?, base)
    }

    /// A request, with MEXC's refusals told apart from a network failure: its statuses mean what
    /// `statuses` says.
    fn get(&self, url: &str, statuses: Statuses) -> Result<String, ProviderError> {
        self.http
            .fetch(DataSource::Mexc, &Request::get(url).statuses(statuses))
    }
}

/// What the statuses of a request that names a market mean at MEXC (klines): a 400 names a market
/// it does not list, a 403 refuses a country it does not serve.
pub(in crate::service) const MARKET_STATUSES: Statuses =
    &[(400, ErrorKind::UnknownSymbol), (403, ErrorKind::Blocked)];

/// The same for a request that names none (the market list, every ticker): a 400 there says
/// nothing about any market.
pub(in crate::service) const LIST_STATUSES: Statuses = &[(403, ErrorKind::Blocked)];

/// Candles of length `base`, oldest first. The days, weeks and months MEXC cut at UTC+8 midnight
/// (its oldest history) are not on this app's grid and are left out.
fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let mut candles = rows_of(body, Asked::Markets)?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.retain(|candle| bucket_start_ms(base, candle.open_time) == candle.open_time);
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// The rows of an answer that is a list (klines, tickers) to a request that `asked` about markets
/// or a list, or what MEXC's envelope says when it is not: `{"code": -1121, "msg": "Invalid
/// symbol."}`, which only a request that named a symbol can mean. `-1003` is "too many requests",
/// the code of Binance's API that MEXC's v3 shares; no other rate-limit code is documented.
pub(crate) fn rows_of(body: &str, asked: Asked) -> Result<Vec<Value>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Mexc, error))?;
    let Value::Array(rows) = value else {
        return Err(match value.get("code").and_then(Value::as_i64) {
            Some(-1121) if asked == Asked::Markets => {
                ProviderError::new(DataSource::Mexc, ErrorKind::UnknownSymbol)
            }
            Some(-1003) => ProviderError::new(DataSource::Mexc, ErrorKind::RateLimited),
            _ => match value.get("msg").and_then(Value::as_str) {
                Some(message) => ProviderError::refused(DataSource::Mexc, message),
                None => ProviderError::malformed(DataSource::Mexc, "expected a list"),
            },
        });
    };
    Ok(rows)
}

fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let number = |index: usize| -> Result<f64, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| {
                ProviderError::malformed(DataSource::Mexc, format!("invalid field {index}"))
            })
    };
    let open_time = row
        .get(0)
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::malformed(DataSource::Mexc, "invalid timestamp"))?;
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

/// One tradable spot market from `exchangeInfo`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pair {
    id: String,
    base: String,
    quote: String,
    /// The base asset's full name.
    #[serde(default)]
    name: Option<String>,
}

impl ListedMarket for Pair {
    const SOURCE: DataSource = DataSource::Mexc;

    fn id(&self) -> &str {
        &self.id
    }

    fn sides(&self) -> Option<(&str, &str)> {
        Some((&self.base, &self.quote))
    }

    fn names(&self) -> (Option<&str>, Option<&str>) {
        (self.name.as_deref(), None)
    }
}

impl Mexc {
    fn download_pairs(&self) -> Result<Vec<Pair>, ProviderError> {
        parse_pairs(&self.get(&format!("{BASE_URL}/exchangeInfo"), LIST_STATUSES)?)
    }
}

/// The enabled spot markets. Its tokenised stocks and commodities (`GOLD(XAUT)USDC`) are not
/// letters and digits and are left out, like a symbol the app cannot name.
fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Mexc, error))?;
    Ok(value
        .get("symbols")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::malformed(DataSource::Mexc, "expected a symbol list"))?
        .iter()
        .filter(|row| {
            row.get("status").and_then(Value::as_str) == Some("1")
                && row.get("isSpotTradingAllowed").and_then(Value::as_bool) == Some(true)
        })
        .filter_map(|row| {
            let field = |key: &str| row.get(key)?.as_str().filter(|text| !text.is_empty());
            let id = field("symbol")?;
            if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
                return None;
            }
            Some(Pair {
                id: id.to_owned(),
                base: field("baseAsset")?.to_owned(),
                quote: field("quoteAsset")?.to_owned(),
                name: field("fullName").map(str::to_owned),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::market::candles::plan;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::market::ports::http::{self, Retry};
    use std::sync::Arc;

    const MINUTE: i64 = 60_000;
    const NOON: i64 = Rig::NOON;

    fn venue(rig: &Rig) -> Mexc {
        Mexc::new(&rig.context())
    }

    /// An answer of the `count` one-minute rows that open from `first`, oldest first.
    fn minute_rows(first: i64, count: usize) -> String {
        let rows: Vec<String> = (0..count as i64)
            .map(|index| {
                let open = first + index * MINUTE;
                format!(
                    r#"[{open},"10","12","9","11","1.5",{},"15"]"#,
                    open + MINUTE
                )
            })
            .collect();
        format!("[{}]", rows.join(","))
    }

    #[test]
    fn every_native_has_its_interval_and_socket_name_and_nothing_else_does() {
        let names: Vec<(&str, &str)> = DataSource::Mexc
            .natives()
            .iter()
            .map(|native| {
                (
                    interval(*native).expect("a native has an interval"),
                    socket_interval(*native).expect("and a socket name"),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("1m", "Min1"),
                ("5m", "Min5"),
                ("15m", "Min15"),
                ("30m", "Min30"),
                ("60m", "Min60"),
                ("4h", "Hour4"),
                ("8h", "Hour8"),
                ("1d", "Day1"),
                ("1W", "Week1"),
                ("1M", "Month1"),
            ]
        );
        for other in ["3m", "2h", "6h", "12h", "2D", "2W", "3M", "1Y"] {
            assert_eq!(interval(Timeframe::parse(other).unwrap()), None, "{other}");
        }
    }

    fn base_of(timeframe: &str) -> (&'static str, usize) {
        let plan = plan(DataSource::Mexc, Timeframe::parse(timeframe).unwrap()).unwrap();
        (interval(plan.base()).unwrap(), plan.base_count(500))
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("1h"), ("60m", 500));
        assert_eq!(base_of("22m"), ("1m", 501 * 22));
        assert_eq!(base_of("2h"), ("60m", 501 * 2));
        assert_eq!(base_of("12h"), ("4h", 501 * 3));
        assert_eq!(base_of("3D"), ("1d", 501 * 3));
        assert_eq!(base_of("3M"), ("1M", 501 * 3));
        assert_eq!(base_of("1Y"), ("1M", 501 * 12));
    }

    #[test]
    fn symbols_are_mexcs_however_they_are_spelled() {
        assert_eq!(symbol_id("btc/usdt"), "BTCUSDT");
        assert_eq!(symbol_id("ETH-USDC"), "ETHUSDC");
        assert_eq!(symbol_id("sol"), "SOLUSDT");
    }

    #[test]
    fn a_page_is_asked_for_as_a_window_that_holds_what_is_wanted_and_no_more() {
        let hour = 3_600_000;
        let after = 1_794_916_800_000;
        // The end is the millisecond before `after`; the start is `wanted` candles back.
        assert_eq!(
            window_before(after, 500, Timeframe::HOUR_1),
            (after - 500 * hour, after - 1)
        );
        // A month is taken at its shortest: 100 of them never hold more than 100 candles.
        let (start, end) = window_before(after, 100, Timeframe::MONTH_1);
        assert_eq!((end - start + 1) / (28 * 86_400_000), 100);
        // A window never starts before the first date MEXC answers for.
        assert_eq!(
            window_before(EARLIEST_MS + 1, 500, Timeframe::HOUR_1).0,
            EARLIEST_MS
        );
    }

    // A one-minute candle: the volume is the sixth field, the close time (the next open) the
    // seventh and the quote volume the eighth.
    const ROWS: &str = r#"[
        [1699999200000,"10.0","12.0","9.0","11.0","42.5",1700002800000,"450"],
        [1700002800000,"11.0","13.0","10.0","12.0","5.5",1700006400000,"60"]
    ]"#;

    #[test]
    fn rows_are_candles_with_the_close_of_their_bucket() {
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
    }

    #[test]
    fn candles_cut_at_utc_plus_8_midnight_are_not_on_the_grid_and_are_left_out() {
        // 2018-06-01 16:00 UTC, which MEXC's oldest days open at, and 2018-12-04 00:00 UTC.
        let body = r#"[
            [1527868800000,"1","2","0.5","1.5","3",1527955200000,"4"],
            [1543881600000,"1","2","0.5","1.5","3",1543968000000,"4"]
        ]"#;
        let candles = parse_candles(body, Timeframe::DAY_1).unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open_time, 1_543_881_600_000);
        // The same for months: one that opens at 16:00 on the last day of the month before.
        let months = r#"[
            [1506787200000,"1","2","0.5","1.5","3",1509465600000,"4"],
            [1509494400000,"1","2","0.5","1.5","3",1512086400000,"4"]
        ]"#;
        let candles = parse_candles(months, Timeframe::MONTH_1).unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open_time, 1_509_494_400_000);
    }

    #[test]
    fn a_month_closes_when_the_next_calendar_month_opens() {
        // 2026-02-01 00:00 UTC.
        let row = serde_json::json!([
            1769904000000_i64,
            "1",
            "2",
            "0.5",
            "1.5",
            "3",
            1772323200000_i64,
            "4"
        ]);
        let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
        assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
    }

    #[test]
    fn odd_answers_are_errors() {
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        assert_eq!(
            parse_candles(
                r#"{"code":-1121,"msg":"Invalid symbol."}"#,
                Timeframe::HOUR_1
            ),
            Err(ProviderError::new(
                DataSource::Mexc,
                ErrorKind::UnknownSymbol
            ))
        );
        assert_eq!(
            parse_candles(
                r#"{"code":-1003,"msg":"Too many requests"}"#,
                Timeframe::HOUR_1
            ),
            Err(ProviderError::new(DataSource::Mexc, ErrorKind::RateLimited))
        );
        assert_eq!(
            parse_candles(
                r#"{"code":-1100,"msg":"Illegal characters found in parameter."}"#,
                Timeframe::HOUR_1
            ),
            Err(ProviderError::refused(
                DataSource::Mexc,
                "Illegal characters found in parameter."
            ))
        );
        assert!(parse_candles(r#"{"x":1}"#, Timeframe::HOUR_1).is_err());
        assert_eq!(parse_candles("[]", Timeframe::HOUR_1), Ok(Vec::new()));
        let bad_field = r#"[[1700000000000,"x","1","1","1","1",1,"1"]]"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"[["x","1","1","1","1","1",1,"1"]]"#, Timeframe::HOUR_1).is_err());
    }

    #[test]
    fn the_newest_candles_are_one_request_and_come_back_oldest_first() {
        let rig = Rig::new();
        rig.http.on("/klines?", ROWS);
        let candles = venue(&rig)
            .fetch_klines("btc/usdt", Timeframe::HOUR_1, 2)
            .expect("candles");
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].url,
            "https://api.mexc.com/api/v3/klines?symbol=BTCUSDT&interval=60m&limit=2"
        );
        assert_eq!(sent[0].retry, Retry::Transient);
        let opens: Vec<i64> = candles.iter().map(|candle| candle.open_time).collect();
        assert_eq!(opens, [1_699_999_200_000, 1_700_002_800_000]);
    }

    #[test]
    fn a_range_is_paged_back_as_windows_because_an_end_alone_is_ignored() {
        let rig = Rig::new();
        let end = NOON;
        let (first, second) = (end - 500 * MINUTE, end - 700 * MINUTE);
        rig.http
            .on("&limit=500&startTime=", &minute_rows(first, 500));
        rig.http
            .on("&limit=200&startTime=", &minute_rows(second, 200));
        let candles = venue(&rig)
            .fetch_klines_range("BTCUSDT", Timeframe::MINUTE_1, 0, end, 700)
            .expect("candles");
        let base = "https://api.mexc.com/api/v3/klines?symbol=BTCUSDT&interval=1m";
        assert_eq!(
            rig.http.urls(),
            [
                format!("{base}&limit=500&startTime={first}&endTime={}", end - 1),
                format!("{base}&limit=200&startTime={second}&endTime={}", first - 1),
            ]
        );
        assert_eq!(candles.len(), 700);
        assert_eq!(candles[0].open_time, second);
        assert_eq!(candles[699].open_time, end - MINUTE);
    }

    #[test]
    fn what_mexc_refuses_is_told_by_kind() {
        let kind_of = |answer: Result<&str, http::Error>| {
            let rig = Rig::new();
            match answer {
                Ok(body) => rig.http.on("/klines?", body),
                Err(error) => rig.http.fail("/klines?", error),
            };
            venue(&rig)
                .fetch_klines("BTCUSDT", Timeframe::HOUR_1, 5)
                .unwrap_err()
                .kind
        };
        assert_eq!(
            kind_of(Ok(r#"{"code":-1121,"msg":"Invalid symbol."}"#)),
            ErrorKind::UnknownSymbol
        );
        assert_eq!(
            kind_of(Ok(r#"{"code":-1003,"msg":"Too many requests"}"#)),
            ErrorKind::RateLimited
        );
        assert_eq!(
            kind_of(Err(http::Error::Status(400))),
            ErrorKind::UnknownSymbol
        );
        assert_eq!(kind_of(Err(http::Error::Status(403))), ErrorKind::Blocked);
        assert_eq!(
            kind_of(Err(http::Error::Status(429))),
            ErrorKind::RateLimited
        );
    }

    #[test]
    fn the_market_list_is_downloaded_once_a_day_and_names_the_assets() {
        let rig = Rig::new();
        rig.http.on(
            "/exchangeInfo",
            r#"{"symbols":[
                {"symbol":"BTCUSDT","status":"1","baseAsset":"BTC","quoteAsset":"USDT","isSpotTradingAllowed":true,"fullName":"Bitcoin"},
                {"symbol":"OLDUSDT","status":"2","baseAsset":"OLD","quoteAsset":"USDT","isSpotTradingAllowed":true}]}"#,
        );
        let mexc = venue(&rig);
        let found = mexc.search_tickers("btc-usdt").expect("hits");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbol, "BTCUSDT");
        assert_eq!(
            venue(&rig).asset_names(),
            [("BTC".to_owned(), "Bitcoin".to_owned())]
        );
        assert_eq!(rig.http.urls().len(), 1);
    }

    #[test]
    fn the_catalog_keeps_only_enabled_spot_markets_with_a_plain_name() {
        let body = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"1","baseAsset":"BTC","quoteAsset":"USDT","isSpotTradingAllowed":true},
            {"symbol":"OLDUSDT","status":"2","baseAsset":"OLD","quoteAsset":"USDT","isSpotTradingAllowed":true},
            {"symbol":"NOSPOTUSDT","status":"1","baseAsset":"NOSPOT","quoteAsset":"USDT","isSpotTradingAllowed":false},
            {"symbol":"GOLD(XAUT)USDC","status":"1","baseAsset":"GOLD(XAUT)","quoteAsset":"USDC","isSpotTradingAllowed":true}
        ]}"#;
        let pairs = parse_pairs(body).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].id.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTCUSDT", "BTC", "USDT")
        );
        assert!(parse_pairs("not json").is_err());
        assert!(parse_pairs("{}").is_err());
    }

    #[test]
    fn a_pair_is_found_however_it_is_separated() {
        let pairs = vec![
            Pair {
                id: "BTCUSDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
                name: None,
            },
            Pair {
                id: "ETHBTC".into(),
                base: "ETH".into(),
                quote: "BTC".into(),
                name: None,
            },
        ];
        let pairs = Arc::new(pairs);
        let ask = |query: &str| search::find(query, || Ok(Arc::clone(&pairs))).unwrap();
        for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
            let found = ask(query);
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].source, DataSource::Mexc);
            assert_eq!(found[0].symbol, "BTCUSDT");
            assert_eq!(found[0].base.as_deref(), Some("BTC"));
            assert_eq!(found[0].quote.as_deref(), Some("USDT"));
        }
        assert_eq!(ask("BTC").len(), 2);
    }

    /// Run with `cargo test --lib mexc::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_mexc_serves_candles_pairs_and_every_native() {
        let mexc = &crate::service::wiring::venues().mexc;
        let hits = mexc.search_tickers("btc-usdt").expect("search");
        assert_eq!(hits[0].symbol, "BTCUSDT");
        assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
        for native in DataSource::Mexc.natives() {
            // More than one page of the shortest bars, so paging is exercised.
            let wanted = if *native == Timeframe::MINUTE_1 {
                1_200
            } else {
                350
            };
            let candles = mexc
                .fetch_klines("BTCUSDT", *native, wanted)
                .expect("candles");
            // Weeks and months reach back only as far as the grid does.
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
        // A 22-minute chart of 500 candles is eleven thousand one-minute ones: 22 pages.
        let folded = mexc
            .fetch_klines("BTCUSDT", Timeframe::parse("22m").unwrap(), 500)
            .expect("folded candles");
        assert_eq!(folded.len(), 500);
        // Daily history is on the grid all the way: no candle cut at UTC+8 midnight remains.
        let mut days = mexc
            .fetch_klines("BTCUSDT", Timeframe::DAY_1, 500)
            .expect("days");
        loop {
            let end = days[0].open_time;
            let page = mexc
                .fetch_klines_range("BTCUSDT", Timeframe::DAY_1, 0, end, 1_000)
                .expect("an older page");
            if page.is_empty() {
                break;
            }
            days.splice(0..0, page);
        }
        assert!(days.len() > 2_500, "{}", days.len());
        assert!(days.iter().all(|day| day.open_time % 86_400_000 == 0));
        let error = mexc
            .fetch_klines("NOPEUSDT", Timeframe::HOUR_1, 5)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownSymbol);
    }

    #[test]
    fn a_400_names_an_unknown_market_only_where_one_was_asked_for() {
        use crate::service::market::ports::http::Error;
        let kind = |statuses, code| {
            ProviderError::http_with(DataSource::Mexc, statuses, Error::Status(code)).kind
        };
        assert_eq!(kind(MARKET_STATUSES, 400), ErrorKind::UnknownSymbol);
        // The market list and the whole ticker name none.
        assert_eq!(
            kind(LIST_STATUSES, 400),
            ErrorKind::Transport(Error::Status(400))
        );
        for statuses in [MARKET_STATUSES, LIST_STATUSES] {
            assert_eq!(kind(statuses, 403), ErrorKind::Blocked);
        }
    }

    #[test]
    fn an_invalid_symbol_is_an_unknown_market_only_to_a_request_that_named_one() {
        let body = r#"{"code":-1121,"msg":"Invalid symbol."}"#;
        assert_eq!(
            rows_of(body, Asked::Markets),
            Err(ProviderError::new(
                DataSource::Mexc,
                ErrorKind::UnknownSymbol
            ))
        );
        assert_eq!(
            rows_of(body, Asked::List),
            Err(ProviderError::refused(DataSource::Mexc, "Invalid symbol."))
        );
        // A rate limit is one whatever was asked.
        let limited = r#"{"code":-1003,"msg":"Too many requests"}"#;
        for asked in [Asked::Markets, Asked::List] {
            assert!(rows_of(limited, asked).unwrap_err().is_rate_limited());
        }
    }
}
