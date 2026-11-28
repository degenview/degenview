//! Oanda's v3 API: OHLCV candles of any instrument of the account, and the account's instrument
//! list for search. Both need a login — a personal access token and the account id (Settings ▸
//! Oanda) — sent as a bearer token; the candles endpoint itself takes only the token. The login
//! also names its environment, live or practice, which decides the host (`api_request`). The login
//! is looked up once per answer and passed down, so the pages of one history all come from one
//! environment even if Settings changes while they are being fetched.
//!
//! Oanda aligns its hourly, daily and weekly bars to 5pm New York by default, which puts an H12
//! bar across two UTC days. Every request here instead asks for UTC alignment
//! (`alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday`), so the seconds, minutes,
//! hours, days and weeks it returns open exactly on the app's UTC grid. Its monthly candle is
//! stamped on the last day of the previous month even then, off that grid, so months are folded
//! from days rather than asked for.

use super::catalog::Catalog;
use super::catalog::search::{self, ListedMarket, WIDE_HITS};
use super::kline_endpoint::{self, KlineEndpoint};
use super::paging::Paging;
use super::ports::clock::Clock;
use super::ports::context::MarketContext;
use super::ports::credentials::Credentials;
use super::ports::http::{Http, Request};
use super::search_rank::normalize;
use crate::model::market::oanda::{OandaCredentials, OandaEnvironment};
use crate::model::{DataSource, Kline, Listing, TimeUnit, Timeframe};
use crate::service::market::error::{ErrorKind, Statuses};
use crate::service::market::ports::http::Retry;
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub struct Oanda {
    http: Arc<dyn Http>,
    clock: Arc<dyn Clock>,
    credentials: Arc<dyn Credentials>,
    /// The instrument list belongs to the account whose credentials downloaded it.
    pub(super) catalog: Catalog<Instrument>,
}

impl Oanda {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            clock: context.clock.clone(),
            credentials: context.credentials.clone(),
            catalog: Catalog::new(
                Listing::spot(DataSource::Oanda),
                "catalog_oanda_v2",
                context,
            ),
        }
    }
}

/// The most candles one Oanda request returns.
const MAX_PER_REQUEST: usize = 5_000;

// Reserve one row for a candle returned at the cursor boundary. Forty pages can
// fill a standard 500-bar chart even when a yearly view folds daily candles.
const PAGING: Paging = Paging {
    page: MAX_PER_REQUEST - 1,
    max_candles: 40 * (MAX_PER_REQUEST - 1),
    pause: Duration::ZERO,
};

/// What the statuses of a request that names instruments mean at Oanda (candles, pricing): a 400
/// names an instrument it does not know, a 401 or 403 refuses the access token.
pub(in crate::service) const MARKET_STATUSES: Statuses = &[
    (400, ErrorKind::UnknownSymbol),
    (401, ErrorKind::Unauthorized),
    (403, ErrorKind::Unauthorized),
];

/// A request about the account (its instrument list, Test Connection): a 401 or 403 refuses the
/// access token, and a 400 is a malformed account id, which no instrument is to blame for.
const ACCOUNT_STATUSES: Statuses = &[
    (401, ErrorKind::Unauthorized),
    (403, ErrorKind::Unauthorized),
];

impl Oanda {
    /// The saved login, once per answer: what is fetched next is fetched with it.
    fn login(&self) -> Result<OandaCredentials, ProviderError> {
        self.credentials
            .oanda()
            .ok_or(ProviderError::new(DataSource::Oanda, ErrorKind::MissingKey))
    }
}

/// The v3 API of an environment.
fn base_url(environment: OandaEnvironment) -> &'static str {
    match environment {
        OandaEnvironment::Live => "https://api-fxtrade.oanda.com/v3",
        OandaEnvironment::Practice => "https://api-fxpractice.oanda.com/v3",
    }
}

/// A v3 request of a login: the URL on its environment's host and the `Authorization` header
/// that signs it in. The two are only ever built together, since a token is refused anywhere
/// but on its own environment's host.
pub(in crate::service) struct ApiRequest {
    pub url: String,
    pub authorization: String,
}

pub(in crate::service) fn api_request(login: &OandaCredentials, path: &str) -> ApiRequest {
    ApiRequest {
        url: format!("{}{path}", base_url(login.environment)),
        authorization: format!("Bearer {}", login.access_token),
    }
}

impl Oanda {
    fn send(
        &self,
        request: &ApiRequest,
        statuses: Statuses,
        retry: Retry,
    ) -> Result<String, ProviderError> {
        self.http.fetch(
            DataSource::Oanda,
            &Request::get(&request.url)
                .headers(&[("Authorization", request.authorization.as_str())])
                .retry(retry)
                .statuses(statuses),
        )
    }

    /// A request about the account: its instrument list.
    fn get_account(
        &self,
        login: &OandaCredentials,
        path: &str,
        retry: Retry,
    ) -> Result<String, ProviderError> {
        self.send(&api_request(login, path), ACCOUNT_STATUSES, retry)
    }
}

/// The `granularity` value Oanda takes for a native length: `S5`, `M1`, `H4`, `D`, `W`, `M`.
fn granularity(bar: Timeframe) -> String {
    let count = bar.count();
    match bar.unit() {
        TimeUnit::Second => format!("S{count}"),
        TimeUnit::Minute => format!("M{count}"),
        TimeUnit::Hour => format!("H{count}"),
        TimeUnit::Day => "D".to_owned(),
        TimeUnit::Week => "W".to_owned(),
        TimeUnit::Month => "M".to_owned(),
    }
}

/// The account's instrument list, which is what search needs.
fn instruments_path(account_id: &str) -> String {
    format!("/accounts/{account_id}/instruments")
}

impl Oanda {
    /// Settings ▸ Oanda ▸ Test Connection: the account's instrument list, asked for with the given
    /// login (the one typed on the page, saved or not), which is what Oanda charts need it for.
    pub fn test_connection(&self, login: &OandaCredentials) -> Result<(), ProviderError> {
        let login = login.trimmed();
        self.get_account(&login, &instruments_path(&login.account_id), Retry::Never)
            .map(|_| ())
    }

    /// The candles of one login that open in `[start_ms, end_ms)`, at most `max` of them, paged
    /// back from the end.
    pub(in crate::service) fn klines_range(
        &self,
        login: &OandaCredentials,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        kline_endpoint::range(
            &self.klines(login),
            symbol,
            timeframe,
            start_ms,
            end_ms,
            max,
        )
    }

    /// Oanda's candles endpoint, asked as one login.
    fn klines<'a>(&'a self, login: &'a OandaCredentials) -> Klines<'a> {
        Klines { oanda: self, login }
    }
}

struct Klines<'a> {
    oanda: &'a Oanda,
    login: &'a OandaCredentials,
}

impl KlineEndpoint for Klines<'_> {
    const SOURCE: DataSource = DataSource::Oanda;
    const PAGING: Paging = PAGING;

    fn id(&self, symbol: &str) -> String {
        symbol.to_owned()
    }

    fn page(
        &self,
        id: &str,
        base: Timeframe,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<Kline>, ProviderError> {
        let fetch = |count| {
            let path = candles_path(id, base, before, count);
            let body = self.oanda.send(
                &api_request(self.login, &path),
                MARKET_STATUSES,
                Retry::Transient,
            )?;
            parse_candles(&body, base)
        };
        match before {
            Some(before) => candle_page(before, limit, fetch),
            None => fetch(limit),
        }
    }
}

impl MarketSource for Oanda {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let login = self.login()?;
        kline_endpoint::latest(
            &self.klines(&login),
            self.clock.now_ms(),
            symbol,
            timeframe,
            limit as usize,
        )
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        self.klines_range(&self.login()?, symbol, timeframe, start_ms, end_ms, max)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // Credentials first: with none saved, a cached list must not make search seem available.
        let login = self.login()?;
        search::find(query, || {
            self.catalog
                .get_for(&login.account_id, || self.download_instruments(&login))
        })
    }
}

/// Mid prices aligned to the UTC grid. An end cursor asks for a count of
/// candles leading up to that time, including when the market was closed.
fn candles_path(instrument: &str, bar: Timeframe, to_ms: Option<i64>, count: usize) -> String {
    let mut path = format!(
        "/instruments/{instrument}/candles?price=M&granularity={}&alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday&count={}",
        granularity(bar),
        count.clamp(1, MAX_PER_REQUEST),
    );
    if let Some(to) = to_ms {
        path.push_str(&format!("&to={}", crate::format::iso8601(to)));
    }
    path
}

/// Convert the provider's end cursor into the pager's exclusive cursor. Asking
/// for one additional row lets a candle at the boundary be discarded without
/// mistaking a full page for the start of history.
fn candle_page(
    before: i64,
    wanted: usize,
    fetch: impl FnOnce(usize) -> Result<Vec<Kline>, ProviderError>,
) -> Result<Vec<Kline>, ProviderError> {
    let mut candles = fetch(wanted + 1)?;
    candles.retain(|candle| candle.open_time < before);
    let extra = candles.len().saturating_sub(wanted);
    candles.drain(..extra);
    Ok(candles)
}

/// `{"candles": [{"time": ISO, "volume": n, "complete": bool, "mid": {"o","h","l","c"}}]}`, oldest
/// first. The forming (incomplete) last candle is kept, as Binance's is.
fn parse_candles(body: &str, bar: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Oanda, error))?;
    let rows = value
        .get("candles")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::malformed(DataSource::Oanda, "expected a candles object"))?;
    rows.iter().map(|row| parse_candle(row, bar)).collect()
}

fn parse_candle(row: &Value, bar: Timeframe) -> Result<Kline, ProviderError> {
    let open_time = row
        .get("time")
        .and_then(Value::as_str)
        .and_then(crate::format::parse_iso8601)
        .ok_or_else(|| ProviderError::malformed(DataSource::Oanda, "invalid candle time"))?;
    let mid = row
        .get("mid")
        .ok_or_else(|| ProviderError::malformed(DataSource::Oanda, "expected mid prices"))?;
    let price = |key: &str| -> Result<f64, ProviderError> {
        mid.get(key)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| {
                ProviderError::malformed(DataSource::Oanda, format!("invalid price {key}"))
            })
    };
    let volume = row.get("volume").and_then(Value::as_f64).unwrap_or(0.0);
    Ok(Kline {
        open_time,
        // The bars are asked for on the UTC grid, so a candle closes a millisecond before the
        // next one opens.
        close_time: open_time + bar.nominal_ms() - 1,
        open: price("o")?,
        high: price("h")?,
        low: price("l")?,
        close: price("c")?,
        volume,
    })
}

/// One instrument of the account from `/accounts/{id}/instruments`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Instrument {
    name: String,
    /// How Oanda writes it for people: `Germany 30`, `Gold`, or just the pair (`EUR/USD`).
    #[serde(default)]
    display_name: Option<String>,
}

impl Instrument {
    /// The display name, unless it only repeats the id with other punctuation.
    fn described(&self) -> Option<&str> {
        let name = self.display_name.as_deref()?;
        (normalize(name) != normalize(&self.name)).then_some(name)
    }
}

impl ListedMarket for Instrument {
    const SOURCE: DataSource = DataSource::Oanda;
    const HITS: usize = WIDE_HITS;

    fn id(&self) -> &str {
        &self.name
    }

    /// Oanda names a market `BASE_QUOTE`.
    fn sides(&self) -> Option<(&str, &str)> {
        self.name
            .split_once('_')
            .filter(|(base, quote)| !base.is_empty() && !quote.is_empty())
    }

    fn names(&self) -> (Option<&str>, Option<&str>) {
        (self.described(), None)
    }
}

impl Oanda {
    fn download_instruments(
        &self,
        login: &OandaCredentials,
    ) -> Result<Vec<Instrument>, ProviderError> {
        let body = self.get_account(
            login,
            &instruments_path(&login.account_id),
            Retry::Transient,
        )?;
        parse_instruments(&body)
    }
}

fn parse_instruments(body: &str) -> Result<Vec<Instrument>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Oanda, error))?;
    Ok(value
        .get("instruments")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|instrument| {
            Some(Instrument {
                name: instrument.get("name")?.as_str()?.to_owned(),
                display_name: instrument
                    .get("displayName")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::market::candles::plan;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::market::ports::http;
    use std::sync::Arc;

    const DAY_MS: i64 = 86_400_000;

    #[test]
    fn parses_oanda_candles_with_their_forming_one() {
        let body = r#"{
            "instrument": "EUR_USD",
            "granularity": "H1",
            "candles": [
                {"time":"2026-10-09T13:00:00.000000000Z","volume":812,"complete":true,
                 "mid":{"o":"1.10000","h":"1.10500","l":"1.09900","c":"1.10200"}},
                {"time":"2026-10-09T14:00:00.000000000Z","volume":120,"complete":false,
                 "mid":{"o":"1.10200","h":"1.10300","l":"1.10100","c":"1.10250"}}
            ]
        }"#;
        let candles = parse_candles(body, Timeframe::HOUR_1).expect("valid response");
        assert_eq!(candles.len(), 2, "the forming candle is kept");
        let open = crate::format::days_from_civil(2026, 10, 9) * DAY_MS + 13 * 3_600_000;
        assert_eq!(
            candles[0],
            Kline {
                open_time: open,
                close_time: open + 3_600_000 - 1,
                open: 1.1,
                high: 1.105,
                low: 1.099,
                close: 1.102,
                volume: 812.0,
            }
        );
        assert_eq!(candles[1].open_time, open + 3_600_000);
        assert_eq!(candles[1].close_time, candles[1].open_time + 3_600_000 - 1);
    }

    #[test]
    fn rejects_error_objects() {
        let error = parse_candles(
            r#"{"errorCode":"some","errorMessage":"bad"}"#,
            Timeframe::HOUR_1,
        )
        .expect_err("object must fail");
        assert_eq!(
            error,
            ProviderError::malformed(DataSource::Oanda, "expected a candles object")
        );
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
    }

    #[test]
    fn a_login_is_only_sent_to_the_host_of_its_own_environment() {
        for (environment, host) in [
            (
                OandaEnvironment::Live,
                "https://api-fxtrade.oanda.com/v3/accounts/101-001/instruments",
            ),
            (
                OandaEnvironment::Practice,
                "https://api-fxpractice.oanda.com/v3/accounts/101-001/instruments",
            ),
        ] {
            let login = OandaCredentials {
                account_id: "101-001".into(),
                access_token: "tok".into(),
                environment,
            };
            let request = api_request(&login, &instruments_path(&login.account_id));
            assert_eq!(request.url, host);
            assert_eq!(request.authorization, "Bearer tok");
        }
    }

    fn practice_login() -> OandaCredentials {
        OandaCredentials {
            account_id: "101-001".into(),
            access_token: "tok".into(),
            environment: OandaEnvironment::Practice,
        }
    }

    fn venue(rig: &Rig) -> Oanda {
        Oanda::new(&rig.context())
    }

    /// An answer of the `count` one-minute candles that open before `to`, oldest first.
    fn minute_candles(to: i64, count: i64) -> String {
        let rows: Vec<String> = (1..=count)
            .rev()
            .map(|back| {
                format!(
                    r#"{{"time":"{}","volume":1,"complete":true,"mid":{{"o":"1","h":"1","l":"1","c":"1"}}}}"#,
                    crate::format::iso8601(to - back * 60_000)
                )
            })
            .collect();
        format!(
            r#"{{"instrument":"EUR_USD","candles":[{}]}}"#,
            rows.join(",")
        )
    }

    #[test]
    fn every_page_of_a_history_is_asked_with_the_login_it_was_given_and_it_is_looked_up_once() {
        let rig = Rig::new();
        rig.credentials.save_oanda(practice_login());
        let end_ms = crate::format::days_from_civil(2026, 10, 9) * DAY_MS;
        // Every page is full (one row more than wanted, for the cursor), so the pager goes back
        // for a second one.
        let first_page = MAX_PER_REQUEST as i64;
        rig.http
            .on("&count=5000&", &minute_candles(end_ms, first_page));
        rig.http.on(
            "&count=102&",
            &minute_candles(end_ms - (first_page - 1) * 60_000, 102),
        );
        let candles = venue(&rig)
            .fetch_klines_range(
                "EUR_USD",
                Timeframe::MINUTE_1,
                0,
                end_ms,
                MAX_PER_REQUEST + 100,
            )
            .expect("history");

        assert_eq!(candles.len(), MAX_PER_REQUEST + 100);
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 2);
        for request in &sent {
            assert!(
                request
                    .url
                    .starts_with("https://api-fxpractice.oanda.com/v3/instruments/EUR_USD/candles"),
                "{}",
                request.url
            );
            assert_eq!(
                request.headers,
                [("Authorization".to_owned(), "Bearer tok".to_owned())]
            );
            assert_eq!(request.retry, Retry::Transient);
        }
        assert!(
            sent[0]
                .url
                .ends_with(&format!("&to={}", crate::format::iso8601(end_ms)))
        );
        assert_eq!(rig.credentials.oanda_lookups(), 1);
    }

    #[test]
    fn the_newest_candles_are_one_request_signed_with_the_saved_login() {
        let rig = Rig::new();
        rig.credentials.save_oanda(practice_login());
        rig.http.on("/candles?", &minute_candles(Rig::NOON, 3));
        let candles = venue(&rig)
            .fetch_klines("EUR_USD", Timeframe::MINUTE_1, 3)
            .expect("candles");
        assert_eq!(
            rig.http.urls(),
            [
                "https://api-fxpractice.oanda.com/v3/instruments/EUR_USD/candles?price=M&granularity=M1&alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday&count=3"
            ]
        );
        assert_eq!(candles.len(), 3);
        assert_eq!(candles[2].open_time, Rig::NOON - 60_000);
        assert_eq!(rig.credentials.oanda_lookups(), 1);
    }

    #[test]
    fn without_a_saved_login_nothing_is_asked_and_the_key_is_what_is_missing() {
        let rig = Rig::new();
        let oanda = venue(&rig);
        let error = oanda
            .fetch_klines("EUR_USD", Timeframe::MINUTE_1, 3)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::MissingKey);
        assert_eq!(
            oanda.search_tickers("eur").unwrap_err().kind,
            ErrorKind::MissingKey
        );
        assert!(rig.http.urls().is_empty());
    }

    #[test]
    fn what_oanda_refuses_is_told_by_kind() {
        let kind_of = |status| {
            let rig = Rig::new();
            rig.credentials.save_oanda(practice_login());
            rig.http.fail("/candles?", http::Error::Status(status));
            venue(&rig)
                .fetch_klines("EUR_USD", Timeframe::MINUTE_1, 3)
                .unwrap_err()
                .kind
        };
        assert_eq!(kind_of(400), ErrorKind::UnknownSymbol);
        assert_eq!(kind_of(401), ErrorKind::Unauthorized);
        assert_eq!(kind_of(403), ErrorKind::Unauthorized);
        assert_eq!(kind_of(429), ErrorKind::RateLimited);
    }

    #[test]
    fn each_account_has_its_own_instrument_list_downloaded_once() {
        let rig = Rig::new();
        rig.credentials.save_oanda(practice_login());
        rig.http.on(
            "/accounts/101-001/instruments",
            r#"{"instruments":[{"name":"EUR_USD","displayName":"EUR/USD"},{"name":"XAU_USD","displayName":"Gold"}]}"#,
        );
        let oanda = venue(&rig);
        let found = oanda.search_tickers("xau").expect("hits");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbol, "XAU_USD");
        assert_eq!(found[0].names.base.as_deref(), Some("Gold"));
        oanda.search_tickers("eur").expect("hits");
        assert_eq!(
            rig.http
                .sent()
                .iter()
                .map(|sent| sent.url.as_str())
                .collect::<Vec<_>>(),
            ["https://api-fxpractice.oanda.com/v3/accounts/101-001/instruments"]
        );
    }

    #[test]
    fn a_connection_test_sends_the_login_it_is_given_trimmed_and_asks_once() {
        let rig = Rig::new();
        rig.http
            .on("/accounts/101-001/instruments", r#"{"instruments":[]}"#);
        let pasted = OandaCredentials {
            account_id: " 101-001 ".into(),
            access_token: " tok\n".into(),
            environment: OandaEnvironment::Practice,
        };
        venue(&rig).test_connection(&pasted).expect("a connection");
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].url,
            "https://api-fxpractice.oanda.com/v3/accounts/101-001/instruments"
        );
        assert_eq!(
            sent[0].headers,
            [("Authorization".to_owned(), "Bearer tok".to_owned())]
        );
        assert_eq!(sent[0].retry, Retry::Never);
        assert_eq!(rig.credentials.oanda_lookups(), 0);
    }

    #[test]
    fn a_candles_request_asks_for_mid_prices_of_the_granularity_on_the_utc_grid() {
        let to = crate::format::days_from_civil(2026, 10, 9) * DAY_MS;
        assert_eq!(
            candles_path("EUR_USD", Timeframe::HOUR_4, None, 500),
            "/instruments/EUR_USD/candles?price=M&granularity=H4&alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday&count=500"
        );
        assert_eq!(
            candles_path("XAU_USD", Timeframe::DAY_1, Some(to), 5_000),
            format!(
                "/instruments/XAU_USD/candles?price=M&granularity=D&alignmentTimezone=UTC&dailyAlignment=0&weeklyAlignment=Monday&count=5000&to={}",
                crate::format::iso8601(to)
            )
        );
        // The count is capped at what one request holds.
        assert!(candles_path("EUR_USD", Timeframe::MINUTE_1, None, 9_999).ends_with("count=5000"));
    }

    #[test]
    fn natives_map_to_oandas_granularities() {
        let tokens: Vec<String> = DataSource::Oanda
            .natives()
            .iter()
            .map(|native| granularity(*native))
            .collect();
        assert_eq!(
            tokens,
            [
                "S5", "S10", "S15", "S30", "M1", "M2", "M4", "M5", "M10", "M15", "M30", "H1", "H2",
                "H4", "H6", "H8", "H12", "D", "W"
            ]
        );
    }

    fn base_of(timeframe: &str) -> (String, usize) {
        let plan = plan(DataSource::Oanda, Timeframe::parse(timeframe).unwrap()).unwrap();
        (granularity(plan.base()), plan.base_count(500))
    }

    #[test]
    fn a_native_length_is_asked_for_as_it_is() {
        for (token, granularity) in [
            ("5s", "S5"),
            ("30s", "S30"),
            ("1m", "M1"),
            ("4m", "M4"),
            ("15m", "M15"),
            ("30m", "M30"),
            ("1h", "H1"),
            ("4h", "H4"),
            ("12h", "H12"),
            ("1d", "D"),
            ("1w", "W"),
        ] {
            assert_eq!(base_of(token), (granularity.to_owned(), 500), "{token}");
        }
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("22m"), ("M2".to_owned(), 501 * 11));
        assert_eq!(base_of("45m"), ("M15".to_owned(), 501 * 3));
        assert_eq!(base_of("3h"), ("H1".to_owned(), 501 * 3));
        assert_eq!(base_of("8h"), ("H8".to_owned(), 500));
        assert_eq!(base_of("2d"), ("D".to_owned(), 501 * 2));
        assert_eq!(base_of("2w"), ("W".to_owned(), 501 * 2));
        // Oanda's monthly candle is stamped off the UTC grid, so months fold from days, sized
        // for the longest month.
        assert_eq!(base_of("1M"), ("D".to_owned(), 501 * 31));
        assert_eq!(base_of("3M"), ("D".to_owned(), 501 * 93));
        assert_eq!(base_of("1Y"), ("D".to_owned(), 501 * 372));
        // Oanda has no second candles, so 1s cannot be folded to.
        assert!(plan(DataSource::Oanda, Timeframe::SECOND_1).is_err());
    }

    fn candle_at(t: i64) -> Kline {
        Kline {
            open_time: t,
            close_time: t,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        }
    }

    #[test]
    fn count_pages_cross_a_closed_market_and_exclude_the_cursor() {
        let paging = Paging {
            page: 3,
            max_candles: 9,
            pause: Duration::ZERO,
        };
        for includes_cursor in [true, false] {
            let candles = paging
                .back(35, 61, 7, |before, wanted| {
                    candle_page(before, wanted, |count| {
                        Ok((0..70)
                            .filter(|&t| !(40..60).contains(&t))
                            .filter(|&t| t < before || (includes_cursor && t == before))
                            .map(candle_at)
                            .rev()
                            .take(count)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect())
                    })
                })
                .unwrap();
            let opens: Vec<_> = candles.iter().map(|candle| candle.open_time).collect();
            assert_eq!(opens, [35, 36, 37, 38, 39, 60]);
        }
    }

    #[test]
    fn a_series_of_more_than_one_requests_candles_is_paged_but_its_tail_is_not() {
        assert!(base_of("5m").1 <= MAX_PER_REQUEST);
        assert!(base_of("22m").1 > MAX_PER_REQUEST);
        let tail = plan(DataSource::Oanda, Timeframe::parse("22m").unwrap())
            .unwrap()
            .base_count(crate::service::cache::TAIL_LIMIT as usize);
        assert!(tail <= MAX_PER_REQUEST);
    }

    #[test]
    fn an_instrument_is_found_however_it_is_separated() {
        let instruments = vec![
            Instrument {
                name: "EUR_USD".into(),
                display_name: None,
            },
            Instrument {
                name: "XAU_USD".into(),
                display_name: None,
            },
            Instrument {
                name: "SPX500_USD".into(),
                display_name: None,
            },
        ];
        let instruments = Arc::new(instruments);
        let ask = |query: &str| search::find(query, || Ok(Arc::clone(&instruments))).unwrap();
        for query in ["eurusd", "EUR/USD", "eur_usd", "EUR USD"] {
            let found = ask(query);
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].source, DataSource::Oanda);
            assert_eq!(found[0].symbol, "EUR_USD");
            assert_eq!(found[0].base.as_deref(), Some("EUR"));
            assert_eq!(found[0].quote.as_deref(), Some("USD"));
        }
        assert_eq!(ask("USD").len(), 3);
        assert_eq!(ask("XAU").len(), 1);
    }

    #[test]
    fn the_instrument_list_keeps_only_named_ones() {
        let body = r#"{"instruments":[
            {"name":"EUR_USD","type":"CURRENCY"},
            {"name":"XAU_USD","type":"METAL"},
            {"type":"CFD"}
        ],"lastTransactionID":"1"}"#;
        let instruments = parse_instruments(body).expect("instruments");
        assert_eq!(instruments.len(), 2);
        assert_eq!(instruments[0].name, "EUR_USD");
        assert!(parse_instruments("not json").is_err());
        assert!(parse_instruments("{}").expect("no instruments").is_empty());
    }

    #[test]
    fn a_400_names_an_unknown_instrument_only_where_one_was_asked_for() {
        use crate::service::market::ports::http::Error;
        let kind = |statuses, code| {
            ProviderError::http_with(DataSource::Oanda, statuses, Error::Status(code)).kind
        };
        assert_eq!(kind(MARKET_STATUSES, 400), ErrorKind::UnknownSymbol);
        // A mistyped account id in Test Connection, or the instrument list, names no instrument.
        assert_eq!(
            kind(ACCOUNT_STATUSES, 400),
            ErrorKind::Transport(Error::Status(400))
        );
        for statuses in [MARKET_STATUSES, ACCOUNT_STATUSES] {
            assert_eq!(kind(statuses, 401), ErrorKind::Unauthorized);
            assert_eq!(kind(statuses, 403), ErrorKind::Unauthorized);
        }
    }
}
