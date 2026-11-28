use super::ports::clock::Clock;
use super::ports::context::MarketContext;
use super::ports::credentials::Credentials;
use super::ports::http::{Http, Request};
use crate::model::market::cmc::{
    ALTCOIN_HISTORICAL, ALTCOIN_LATEST, CHART_TYPES, FEAR_GREED_HISTORICAL, FEAR_GREED_LATEST,
};
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::market::error::{ErrorKind, Statuses};
use crate::service::{MarketSource, ProviderError, SearchResult, downsample};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BASE: &str = "https://pro-api.coinmarketcap.com";

/// What a reading is called. A value rather than the provider's English word, so the card can name
/// it in the interface language.
#[derive(Clone, Debug, PartialEq)]
pub enum Classification {
    ExtremeFear,
    Fear,
    Neutral,
    Greed,
    ExtremeGreed,
    BitcoinSeason,
    AltcoinSeason,
    /// A word CoinMarketCap added that this version does not know, shown as it came.
    Unlisted(String),
}

impl Classification {
    /// Reads Fear & Greed's `value_classification`, whose capitals vary.
    fn from_fear_greed(word: &str) -> Self {
        match word.to_ascii_lowercase().as_str() {
            "extreme fear" => Self::ExtremeFear,
            "fear" => Self::Fear,
            "neutral" => Self::Neutral,
            "greed" => Self::Greed,
            "extreme greed" => Self::ExtremeGreed,
            _ => Self::Unlisted(word.to_owned()),
        }
    }
}

/// The most recent reading behind a "Latest" chart.
#[derive(Clone, Debug, PartialEq)]
pub struct LatestReading {
    pub value: f64,
    pub classification: Classification,
    /// Altcoin Season only: the yearly extremes with their dates, and the altcoin market cap.
    pub yearly_high: Option<(f64, String)>,
    pub yearly_low: Option<(f64, String)>,
    pub altcoin_market_cap: Option<f64>,
    /// Fear & Greed only: the last ~90 daily readings, oldest first, as (epoch ms, value).
    pub history: Vec<(i64, f64)>,
}

impl LatestReading {
    /// The reading `days` before the newest history point (the closest one at or before it).
    pub fn days_ago(&self, days: i64) -> Option<(i64, f64)> {
        let newest = self.history.last()?.0;
        let target = newest - days * 86_400_000;
        self.history
            .iter()
            .rev()
            .find(|(time, _)| *time <= target + 43_200_000)
            .copied()
    }

    /// The (low, high) readings over the trailing `days` of history, with their timestamps.
    pub fn range(&self, days: i64) -> Option<((i64, f64), (i64, f64))> {
        let newest = self.history.last()?.0;
        let window = self
            .history
            .iter()
            .filter(|(time, _)| *time >= newest - days * 86_400_000);
        let low = window.clone().min_by(|a, b| a.1.total_cmp(&b.1))?;
        let high = window.max_by(|a, b| a.1.total_cmp(&b.1))?;
        Some((*low, *high))
    }
}

/// Bitcoin Season up to 25, Altcoin Season from 75.
fn season_classification(index: f64) -> Classification {
    if index <= 25.0 {
        Classification::BitcoinSeason
    } else if index >= 75.0 {
        Classification::AltcoinSeason
    } else {
        Classification::Neutral
    }
}

/// The reading in a "latest" answer and when it was taken, `now_ms` for one that says nothing of it.
fn parse_latest(symbol: &str, value: &Value, now_ms: i64) -> Option<(i64, LatestReading)> {
    let data = value.get("data")?;
    let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);
    if symbol == ALTCOIN_LATEST {
        let index = data.get("altcoin_index")?.as_f64()?;
        let extreme = |value: &str, date: &str| {
            Some((data.get(value)?.as_f64()?, text(date).unwrap_or_default()))
        };
        let time = text("snapshot_time").and_then(|time| parse_date(&time));
        Some((
            time.unwrap_or(now_ms),
            LatestReading {
                value: index,
                classification: season_classification(index),
                yearly_high: extreme("yearly_high", "yearly_high_date"),
                yearly_low: extreme("yearly_low", "yearly_low_date"),
                altcoin_market_cap: data.get("altcoin_marketcap").and_then(Value::as_f64),
                history: Vec::new(),
            },
        ))
    } else {
        let time = text("update_time").and_then(|time| parse_date(&time));
        Some((
            time.unwrap_or(now_ms),
            LatestReading {
                value: data.get("value")?.as_f64()?,
                classification: Classification::from_fear_greed(
                    &text("value_classification").unwrap_or_default(),
                ),
                yearly_high: None,
                yearly_low: None,
                altcoin_market_cap: None,
                history: Vec::new(),
            },
        ))
    }
}

pub struct CoinMarketCap {
    http: Arc<dyn Http>,
    clock: Arc<dyn Clock>,
    credentials: Arc<dyn Credentials>,
    /// Readings by chart symbol, filled by `fetch_klines` for the chart body to draw.
    latest: Mutex<HashMap<String, LatestReading>>,
    responses: Mutex<ResponseCache>,
}

impl CoinMarketCap {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            clock: context.clock.clone(),
            credentials: context.credentials.clone(),
            latest: Mutex::default(),
            responses: Mutex::default(),
        }
    }

    /// The last reading fetched for a "Latest" chart, if any.
    pub fn latest_reading(&self, symbol: &str) -> Option<LatestReading> {
        self.latest.lock().ok()?.get(symbol).cloned()
    }

    /// The saved (or environment) API key. Without one the public endpoints still answer, at
    /// lower rate limits.
    fn api_key(&self) -> Option<String> {
        self.credentials.cmc()
    }
}

/// The base URL: the authenticated API with a key, the public one without.
fn base_url(key: Option<&str>) -> String {
    if key.is_some() {
        BASE.to_owned()
    } else {
        format!("{BASE}/public-api")
    }
}

/// Responses by URL, plus recent failures. CMC's rate limit is tight (and the keyless public API
/// tighter) while these indices move at most daily, so repeat refreshes reuse a response for a
/// while, and after a failure (HTTP 429 included) the same request isn't repeated straight away.
#[derive(Default)]
struct ResponseCache {
    responses: HashMap<String, (Instant, Value)>,
    failures: HashMap<String, (Instant, ProviderError)>,
}

const LATEST_TTL: Duration = Duration::from_secs(3_600);
const HISTORY_TTL: Duration = Duration::from_secs(3_600);
/// How long a failed request is remembered before the network is tried again.
const FAILURE_BACKOFF: Duration = Duration::from_secs(120);
/// The cache only ever sees a handful of URLs; this just bounds the worst case.
const MAX_CACHED: usize = 32;

impl CoinMarketCap {
    fn get(&self, path: &str, key: Option<&str>, ttl: Duration) -> Result<Value, ProviderError> {
        let url = format!("{}{path}", base_url(key));
        cached_get(&self.responses, &url, ttl, FAILURE_BACKOFF, || {
            self.fetch(path, key)
        })
    }
}

/// `url` from the cache while fresh, else from `fetch`. The lock is not held across `fetch`.
fn cached_get(
    cache: &Mutex<ResponseCache>,
    url: &str,
    ttl: Duration,
    backoff: Duration,
    fetch: impl FnOnce() -> Result<Value, ProviderError>,
) -> Result<Value, ProviderError> {
    if let Ok(cache) = cache.lock() {
        if let Some((fetched, value)) = cache.responses.get(url)
            && fetched.elapsed() < ttl
        {
            return Ok(value.clone());
        }
        if let Some((failed, error)) = cache.failures.get(url)
            && failed.elapsed() < backoff
        {
            return Err(error.clone());
        }
    }
    let result = fetch();
    if let Ok(mut cache) = cache.lock() {
        match &result {
            Ok(value) => {
                cache.failures.remove(url);
                if cache.responses.len() >= MAX_CACHED
                    && let Some(oldest) = cache
                        .responses
                        .iter()
                        .min_by_key(|(_, (fetched, _))| *fetched)
                        .map(|(url, _)| url.clone())
                {
                    cache.responses.remove(&oldest);
                }
                cache
                    .responses
                    .insert(url.to_owned(), (Instant::now(), value.clone()));
            }
            Err(error) => {
                cache
                    .failures
                    .insert(url.to_owned(), (Instant::now(), error.clone()));
            }
        }
    }
    result
}

/// What CoinMarketCap's HTTP statuses mean with a key: a 401 refuses the key, a 403 the plan it
/// belongs to.
const KEYED_STATUSES: Statuses = &[(401, ErrorKind::Unauthorized), (403, ErrorKind::NotInPlan)];

/// What they mean without one, on the keyless public API: it refuses, and a key is what it asks
/// for.
const KEYLESS_STATUSES: Statuses = &[(401, ErrorKind::MissingKey), (403, ErrorKind::MissingKey)];

/// What a status of a request means, which depends on whether it was made with a key.
fn statuses(key: Option<&str>) -> Statuses {
    if key.is_some() {
        KEYED_STATUSES
    } else {
        KEYLESS_STATUSES
    }
}

impl CoinMarketCap {
    fn fetch(&self, path: &str, key: Option<&str>) -> Result<Value, ProviderError> {
        let url = format!("{}{path}", base_url(key));
        let mut headers = vec![("Accept", "application/json")];
        if let Some(key) = key {
            headers.push(("X-CMC_PRO_API_KEY", key));
        }
        let body = self.http.fetch(
            DataSource::CoinMarketCap,
            &Request::get(&url).headers(&headers).statuses(statuses(key)),
        )?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| ProviderError::malformed(DataSource::CoinMarketCap, error))?;
        if value["status"]["error_code"].as_i64().unwrap_or(0) != 0 {
            return Err(ProviderError::refused_because(
                DataSource::CoinMarketCap,
                value["status"]["error_message"].as_str(),
            ));
        }
        Ok(value)
    }
}

fn to_klines(points: Vec<(i64, f64)>, limit: u32) -> Result<Vec<Kline>, ProviderError> {
    let candles: Vec<Kline> = points
        .into_iter()
        .map(|(t, value)| Kline {
            open_time: t,
            close_time: t,
            open: value,
            high: value,
            low: value,
            close: value,
            volume: 0.0,
        })
        .collect();
    if candles.is_empty() {
        return Err(ProviderError::new(
            DataSource::CoinMarketCap,
            ErrorKind::NoData,
        ));
    }
    Ok(downsample(&candles, limit as usize))
}

impl MarketSource for CoinMarketCap {
    fn fetch_klines(
        &self,
        symbol: &str,
        _timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let key = self.api_key();
        match symbol {
            ALTCOIN_HISTORICAL => {
                let value = self.get(
                    "/v1/altcoin-season-index/historical?timeframe=7d",
                    key.as_deref(),
                    HISTORY_TTL,
                )?;
                let points = value
                    .pointer("/data/points")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|point| {
                        let t = parse_date(point.get("timestamp")?.as_str()?)?;
                        let v = point.get("altcoin_index")?.as_f64()?;
                        Some((t, v))
                    })
                    .collect();
                to_klines(points, limit)
            }
            FEAR_GREED_HISTORICAL => to_klines(self.fear_greed_series(key.as_deref())?, limit),
            ALTCOIN_LATEST | FEAR_GREED_LATEST => {
                let path = if symbol == ALTCOIN_LATEST {
                    "/v1/altcoin-season-index/latest"
                } else {
                    "/v3/fear-and-greed/latest"
                };
                let (time, mut reading) = parse_latest(
                    symbol,
                    &self.get(path, key.as_deref(), LATEST_TTL)?,
                    self.clock.now_ms(),
                )
                .ok_or_else(|| ProviderError::new(DataSource::CoinMarketCap, ErrorKind::NoData))?;
                if symbol == FEAR_GREED_LATEST {
                    // The history only decorates the gauge, so a failure here is not an error.
                    // On a failure the previous history stays rather than the gauge losing it.
                    reading.history = self
                        .fear_greed_history(key.as_deref())
                        .ok()
                        .or_else(|| self.latest_reading(symbol).map(|previous| previous.history))
                        .unwrap_or_default();
                }
                let value = reading.value;
                if let Ok(mut latest) = self.latest.lock() {
                    latest.insert(symbol.to_owned(), reading);
                }
                // One point keeps the candle pipeline (loading, errors, alerts) working; the
                // chart body draws the cached reading as a gauge instead of a line.
                to_klines(vec![(time, value)], limit)
            }
            _ => Err(ProviderError::new(
                DataSource::CoinMarketCap,
                ErrorKind::UnknownSymbol,
            )),
        }
    }

    fn fetch_klines_range(
        &self,
        _symbol: &str,
        _timeframe: Timeframe,
        _start_ms: i64,
        _end_ms: i64,
        _max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        Err(ProviderError::new(
            DataSource::CoinMarketCap,
            ErrorKind::NoTimeRange,
        ))
    }

    fn search_tickers(&self, _query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        Ok(CHART_TYPES
            .iter()
            .map(|(key, title)| SearchResult {
                symbol: (*title).to_owned(),
                full_symbol: (*key).to_owned(),
                source: DataSource::CoinMarketCap,
                price: None,
                event: None,
                base: None,
                quote: None,
                names: Default::default(),
            })
            .collect())
    }
}

/// Readings older than this behind the newest one can't be the current end of the series.
const FRESH_WITHIN_MS: i64 = 3 * 86_400_000;
const HISTORY_PAGE: usize = 500;
const MAX_HISTORY_PAGES: usize = 4;

impl CoinMarketCap {
    /// Every Fear & Greed reading CMC will page out, oldest first. The endpoint doesn't document
    /// its order, so pages are followed until the newest reading is current (one request when the
    /// newest come first).
    fn fear_greed_series(&self, key: Option<&str>) -> Result<Vec<(i64, f64)>, ProviderError> {
        let mut points: Vec<(i64, f64)> = Vec::new();
        for page in 0..MAX_HISTORY_PAGES {
            let path = format!(
                "/v3/fear-and-greed/historical?start={}&limit={HISTORY_PAGE}",
                page * HISTORY_PAGE + 1
            );
            let value = self.get(&path, key, HISTORY_TTL)?;
            let fetched = parse_series(&value);
            let full = fetched.len() >= HISTORY_PAGE;
            points.extend(fetched);
            let newest = points.iter().map(|(time, _)| *time).max().unwrap_or(0);
            if !full || newest >= self.clock.now_ms() - FRESH_WITHIN_MS {
                break;
            }
        }
        points.sort_by_key(|(time, _)| *time);
        points.dedup_by_key(|(time, _)| *time);
        Ok(points)
    }

    /// The most recent 90 daily readings, oldest first, for the gauge.
    fn fear_greed_history(&self, key: Option<&str>) -> Result<Vec<(i64, f64)>, ProviderError> {
        let mut points = self.fear_greed_series(key)?;
        let keep = points.len().saturating_sub(90);
        points.drain(..keep);
        Ok(points)
    }

    /// Settings ▸ CoinMarketCap ▸ Test Connection: one cheap authenticated-if-configured request.
    pub fn test_connection(&self) -> Result<(), ProviderError> {
        self.fetch("/v3/fear-and-greed/latest", self.api_key().as_deref())
            .map(|_| ())
    }
}

fn parse_series(value: &Value) -> Vec<(i64, f64)> {
    value
        .get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|point| {
            let t = parse_date(point.get("timestamp")?.as_str()?)?;
            Some((t, point.get("value")?.as_f64()?))
        })
        .collect()
}

/// CMC timestamps arrive as epoch seconds, ISO-8601, or a bare date.
fn parse_date(text: &str) -> Option<i64> {
    if let Ok(seconds) = text.parse::<f64>() {
        return Some((seconds * 1_000.0) as i64);
    }
    let (date, time) = if let Some((date, time)) = text.split_once('T') {
        (date, time.trim_end_matches('Z'))
    } else if let Some((date, time)) = text.split_once(' ') {
        (date, time)
    } else {
        (text, "00:00:00")
    };
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts
        .next()
        .unwrap_or("0")
        .split('.')
        .next()?
        .parse()
        .ok()?;
    let days = crate::format::days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::net::http;

    const LATEST_FEAR: &str = r#"{"data":{"value":56,"update_time":"2026-11-15T08:00:00.000Z","value_classification":"Neutral"},"status":{"error_code":0}}"#;
    const FEAR_HISTORY: &str = r#"{"data":[
        {"timestamp":"2026-11-14T00:00:00.000Z","value":50},
        {"timestamp":"2026-11-13T00:00:00.000Z","value":40}
    ],"status":{"error_code":0}}"#;

    fn venue(rig: &Rig) -> CoinMarketCap {
        CoinMarketCap::new(&rig.context())
    }

    #[test]
    fn a_latest_reading_is_one_point_and_keeps_the_history_for_the_gauge() {
        let rig = Rig::new();
        rig.credentials.save_cmc("cmc-key");
        rig.http.on("/v3/fear-and-greed/latest", LATEST_FEAR);
        rig.http.on(
            "/v3/fear-and-greed/historical?start=1&limit=500",
            FEAR_HISTORY,
        );
        let cmc = venue(&rig);
        let candles = cmc
            .fetch_klines(FEAR_GREED_LATEST, Timeframe::DAY_1, 1)
            .expect("one point");
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].close, 56.0);
        let sent = rig.http.sent();
        assert_eq!(
            sent.iter()
                .map(|sent| sent.url.as_str())
                .collect::<Vec<_>>(),
            [
                "https://pro-api.coinmarketcap.com/v3/fear-and-greed/latest",
                "https://pro-api.coinmarketcap.com/v3/fear-and-greed/historical?start=1&limit=500",
            ]
        );
        assert_eq!(
            sent[0].headers,
            [
                ("Accept".to_owned(), "application/json".to_owned()),
                ("X-CMC_PRO_API_KEY".to_owned(), "cmc-key".to_owned())
            ]
        );
        let reading = cmc.latest_reading(FEAR_GREED_LATEST).expect("a reading");
        assert_eq!(reading.value, 56.0);
        // Oldest first.
        assert_eq!(
            reading
                .history
                .iter()
                .map(|(_, value)| *value)
                .collect::<Vec<_>>(),
            [40.0, 50.0]
        );
        assert_eq!(rig.credentials.cmc_lookups(), 1);
    }

    #[test]
    fn without_a_key_the_public_api_answers_and_a_response_is_reused_within_the_hour() {
        let rig = Rig::new();
        rig.http.on("/v3/fear-and-greed/latest", LATEST_FEAR);
        rig.http.on("/v3/fear-and-greed/historical", FEAR_HISTORY);
        let cmc = venue(&rig);
        cmc.fetch_klines(FEAR_GREED_LATEST, Timeframe::DAY_1, 1)
            .expect("one point");
        cmc.fetch_klines(FEAR_GREED_LATEST, Timeframe::DAY_1, 1)
            .expect("one point");
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 2, "the second answer is the first, kept");
        assert!(
            sent[0]
                .url
                .starts_with("https://pro-api.coinmarketcap.com/public-api/")
        );
        assert_eq!(
            sent[0].headers,
            [("Accept".to_owned(), "application/json".to_owned())]
        );
    }

    #[test]
    fn what_coinmarketcap_refuses_is_told_by_kind_and_a_failure_is_not_asked_again_at_once() {
        let rig = Rig::new();
        rig.http.on(
            "/v3/fear-and-greed/latest",
            r#"{"status":{"error_code":1008,"error_message":"You've exceeded your API Key's rate limit."}}"#,
        );
        let cmc = venue(&rig);
        let first = cmc
            .fetch_klines(FEAR_GREED_LATEST, Timeframe::DAY_1, 1)
            .unwrap_err();
        assert_eq!(
            first.kind,
            ErrorKind::Refused(Some(
                "You've exceeded your API Key's rate limit.".to_owned()
            ))
        );
        let again = cmc
            .fetch_klines(FEAR_GREED_LATEST, Timeframe::DAY_1, 1)
            .unwrap_err();
        assert_eq!(again, first);
        assert_eq!(rig.http.urls().len(), 1);

        // A refusal means what it was asked with: no key asks for one, a key is refused.
        for (key, kind) in [
            (None, ErrorKind::MissingKey),
            (Some("cmc-key"), ErrorKind::Unauthorized),
        ] {
            let rig = Rig::new();
            if let Some(key) = key {
                rig.credentials.save_cmc(key);
            }
            rig.http
                .fail("/v3/fear-and-greed/latest", http::Error::Status(401));
            assert_eq!(
                venue(&rig)
                    .fetch_klines(FEAR_GREED_LATEST, Timeframe::DAY_1, 1)
                    .unwrap_err()
                    .kind,
                kind
            );
        }
    }

    #[test]
    fn a_chart_cmc_does_not_have_is_unknown_without_a_request() {
        let rig = Rig::new();
        let error = venue(&rig)
            .fetch_klines("cmc.nope", Timeframe::DAY_1, 1)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownSymbol);
        assert!(rig.http.urls().is_empty());
    }

    #[test]
    fn a_connection_test_is_one_request_with_the_saved_key() {
        let rig = Rig::new();
        rig.credentials.save_cmc("cmc-key");
        rig.http.on("/v3/fear-and-greed/latest", LATEST_FEAR);
        venue(&rig).test_connection().expect("a connection");
        assert_eq!(rig.http.urls().len(), 1);
    }

    #[test]
    fn parses_the_latest_readings() {
        let altcoin: Value = serde_json::from_str(
            r#"{"data":{"altcoin_index":60,"altcoin_marketcap":1.5e12,"snapshot_time":"2026-10-11T00:00:00.000Z","yearly_high":64,"yearly_high_date":"2026-09-27","yearly_low":14,"yearly_low_date":"2025-12-19"}}"#,
        )
        .unwrap();
        let (_, reading) = parse_latest(ALTCOIN_LATEST, &altcoin, 1_000).unwrap();
        assert_eq!(reading.value, 60.0);
        assert_eq!(reading.classification, Classification::Neutral);
        assert_eq!(reading.yearly_high, Some((64.0, "2026-09-27".to_owned())));
        assert_eq!(reading.yearly_low, Some((14.0, "2025-12-19".to_owned())));
        assert_eq!(reading.altcoin_market_cap, Some(1.5e12));

        let fear: Value = serde_json::from_str(
            r#"{"data":{"value":56,"update_time":"2026-10-11T08:00:00.000Z","value_classification":"Neutral"}}"#,
        )
        .unwrap();
        let (time, reading) = parse_latest(FEAR_GREED_LATEST, &fear, 1_000).unwrap();
        assert!(time > 0);
        assert_eq!(reading.value, 56.0);
        assert_eq!(reading.classification, Classification::Neutral);
        assert_eq!(reading.yearly_high, None);
        assert_eq!(season_classification(80.0), Classification::AltcoinSeason);
        assert_eq!(season_classification(25.0), Classification::BitcoinSeason);
    }

    #[test]
    fn provider_words_become_classifications() {
        for (word, classification) in [
            ("Extreme fear", Classification::ExtremeFear),
            ("Extreme Greed", Classification::ExtremeGreed),
            ("greed", Classification::Greed),
            ("Panic", Classification::Unlisted("Panic".to_owned())),
        ] {
            assert_eq!(Classification::from_fear_greed(word), classification);
        }
    }

    #[test]
    fn history_lookbacks_and_range() {
        let day = 86_400_000;
        let reading = LatestReading {
            value: 50.0,
            classification: Classification::Neutral,
            yearly_high: None,
            yearly_low: None,
            altcoin_market_cap: None,
            history: (0..40).map(|i| (i * day, i as f64)).collect(),
        };
        assert_eq!(reading.days_ago(1), Some((38 * day, 38.0)));
        assert_eq!(reading.days_ago(30), Some((9 * day, 9.0)));
        assert_eq!(reading.days_ago(90), None);
        assert_eq!(
            reading.range(7).map(|(low, high)| (low.1, high.1)),
            Some((32.0, 39.0))
        );
    }

    fn counted(
        calls: &std::cell::Cell<u32>,
        result: Result<Value, ProviderError>,
    ) -> impl FnOnce() -> Result<Value, ProviderError> + '_ {
        move || {
            calls.set(calls.get() + 1);
            result
        }
    }

    #[test]
    fn a_fresh_response_is_reused_without_a_request() {
        let cache = Mutex::new(ResponseCache::default());
        let calls = std::cell::Cell::new(0);
        let hour = Duration::from_secs(3_600);
        let ok = || Ok(serde_json::json!({"v": 1}));
        let first = cached_get(&cache, "u", hour, hour, counted(&calls, ok()));
        let second = cached_get(&cache, "u", hour, hour, counted(&calls, ok()));
        assert_eq!(first, second);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn an_expired_response_is_fetched_again() {
        let cache = Mutex::new(ResponseCache::default());
        let calls = std::cell::Cell::new(0);
        for _ in 0..2 {
            let _ = cached_get(
                &cache,
                "u",
                Duration::ZERO,
                Duration::ZERO,
                counted(&calls, Ok(Value::Null)),
            );
        }
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn a_failure_backs_off_instead_of_retrying_at_once() {
        let cache = Mutex::new(ResponseCache::default());
        let calls = std::cell::Cell::new(0);
        let minute = Duration::from_secs(60);
        let limited = ProviderError::new(DataSource::CoinMarketCap, ErrorKind::RateLimited);
        let failure = || Err(limited.clone());
        assert_eq!(
            cached_get(&cache, "u", minute, minute, counted(&calls, failure())),
            Err(limited.clone())
        );
        assert_eq!(
            cached_get(
                &cache,
                "u",
                minute,
                minute,
                counted(&calls, Ok(Value::Null))
            ),
            Err(limited)
        );
        assert_eq!(calls.get(), 1);
        // Once the backoff has passed the request goes out again, and a success clears it.
        assert!(
            cached_get(
                &cache,
                "u",
                minute,
                Duration::ZERO,
                counted(&calls, Ok(Value::Null))
            )
            .is_ok()
        );
        assert_eq!(calls.get(), 2);
        assert!(cache.lock().unwrap().failures.is_empty());
    }

    #[test]
    fn the_cache_stays_bounded() {
        let cache = Mutex::new(ResponseCache::default());
        let hour = Duration::from_secs(3_600);
        for index in 0..MAX_CACHED * 2 {
            let _ = cached_get(&cache, &format!("u{index}"), hour, hour, || Ok(Value::Null));
        }
        assert!(cache.lock().unwrap().responses.len() <= MAX_CACHED);
    }

    #[test]
    fn a_series_parses_in_either_order() {
        let value: Value = serde_json::from_str(
            r#"{"data":[{"timestamp":"2026-10-10T00:00:00.000Z","value":60},{"timestamp":"2026-10-09T00:00:00.000Z","value":55},{"timestamp":"bad","value":1}]}"#,
        )
        .unwrap();
        let points = parse_series(&value);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].1, 60.0);
    }

    #[test]
    fn parses_a_live_historical_payload() {
        // Captured from the public endpoint: newest first, epoch-second strings.
        let value: Value = serde_json::from_str(
            r#"{"data":[{"timestamp":"1791590400","value":57,"value_classification":"Neutral"},{"timestamp":"1791504000","value":56,"value_classification":"Neutral"}],"status":{"error_code":"0"}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_series(&value),
            vec![(1_791_590_400_000, 57.0), (1_791_504_000_000, 56.0)]
        );
    }

    #[test]
    fn parses_cmc_dates() {
        assert_eq!(parse_date("2026-10-09"), Some(1_791_504_000_000));
        assert_eq!(parse_date("2026-10-09T12:00:00Z"), Some(1_791_547_200_000));
        assert_eq!(parse_date("1791504000"), Some(1_791_504_000_000));
    }

    #[test]
    fn a_key_selects_the_authenticated_api_and_none_the_public_one() {
        assert_eq!(base_url(Some("k")), "https://pro-api.coinmarketcap.com");
        assert_eq!(
            base_url(None),
            "https://pro-api.coinmarketcap.com/public-api"
        );
    }

    #[test]
    fn a_refusal_without_a_key_asks_for_one_and_with_one_blames_the_key_or_the_plan() {
        use crate::service::net::http::Error;
        let kind = |key, code| {
            ProviderError::http_with(
                DataSource::CoinMarketCap,
                statuses(key),
                Error::Status(code),
            )
            .kind
        };
        for code in [401, 403] {
            assert_eq!(kind(None, code), ErrorKind::MissingKey, "{code}");
        }
        assert_eq!(kind(Some("key"), 401), ErrorKind::Unauthorized);
        assert_eq!(kind(Some("key"), 403), ErrorKind::NotInPlan);
        assert_eq!(kind(None, 429), ErrorKind::RateLimited);
    }

    #[test]
    fn an_envelope_error_without_a_message_is_a_refusal_without_words() {
        let error = ProviderError::refused_because(DataSource::CoinMarketCap, None);
        assert_eq!(error.kind, ErrorKind::Refused(None));
        let empty = ProviderError::refused_because(DataSource::CoinMarketCap, Some(""));
        assert_eq!(empty, error, "an empty message says nothing either");
    }
}
