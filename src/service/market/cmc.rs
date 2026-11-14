use crate::model::market::cmc::{
    ALTCOIN_HISTORICAL, ALTCOIN_LATEST, CHART_TYPES, FEAR_GREED_HISTORICAL, FEAR_GREED_LATEST,
};
use crate::model::{DataSource, Kline};
use crate::service::{MarketSource, SearchResult, downsample};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const BASE: &str = "https://pro-api.coinmarketcap.com";

/// The most recent reading behind a "Latest" chart.
#[derive(Clone, Debug, PartialEq)]
pub struct LatestReading {
    pub value: f64,
    pub classification: String,
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

/// Readings by chart symbol, filled by `fetch_klines` for the chart body to draw.
static LATEST: LazyLock<Mutex<HashMap<String, LatestReading>>> = LazyLock::new(Default::default);

/// The last reading fetched for a "Latest" chart, if any.
pub fn latest_reading(symbol: &str) -> Option<LatestReading> {
    LATEST.lock().ok()?.get(symbol).cloned()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// Bitcoin Season up to 25, Altcoin Season from 75.
fn season_classification(index: f64) -> &'static str {
    if index <= 25.0 {
        "Bitcoin Season"
    } else if index >= 75.0 {
        "Altcoin Season"
    } else {
        "Neutral"
    }
}

fn parse_latest(symbol: &str, value: &Value) -> Option<(i64, LatestReading)> {
    let data = value.get("data")?;
    let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);
    if symbol == ALTCOIN_LATEST {
        let index = data.get("altcoin_index")?.as_f64()?;
        let extreme = |value: &str, date: &str| {
            Some((data.get(value)?.as_f64()?, text(date).unwrap_or_default()))
        };
        let time = text("snapshot_time").and_then(|time| parse_date(&time));
        Some((
            time.unwrap_or_else(now_ms),
            LatestReading {
                value: index,
                classification: season_classification(index).to_owned(),
                yearly_high: extreme("yearly_high", "yearly_high_date"),
                yearly_low: extreme("yearly_low", "yearly_low_date"),
                altcoin_market_cap: data.get("altcoin_marketcap").and_then(Value::as_f64),
                history: Vec::new(),
            },
        ))
    } else {
        let time = text("update_time").and_then(|time| parse_date(&time));
        Some((
            time.unwrap_or_else(now_ms),
            LatestReading {
                value: data.get("value")?.as_f64()?,
                classification: text("value_classification").unwrap_or_default(),
                yearly_high: None,
                yearly_low: None,
                altcoin_market_cap: None,
                history: Vec::new(),
            },
        ))
    }
}

pub struct CoinMarketCap;

pub const COIN_MARKET_CAP: CoinMarketCap = CoinMarketCap;

/// The saved (or environment) API key. Without one the public endpoints still answer, at lower
/// rate limits.
fn api_key() -> Option<String> {
    crate::service::keychain::api_keys::cmc_api_key()
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
    failures: HashMap<String, (Instant, String)>,
}

static RESPONSES: LazyLock<Mutex<ResponseCache>> = LazyLock::new(Default::default);

const LATEST_TTL: Duration = Duration::from_secs(3_600);
const HISTORY_TTL: Duration = Duration::from_secs(3_600);
/// How long a failed request is remembered before the network is tried again.
const FAILURE_BACKOFF: Duration = Duration::from_secs(120);
/// The cache only ever sees a handful of URLs; this just bounds the worst case.
const MAX_CACHED: usize = 32;

fn get(path: &str, key: Option<&str>, ttl: Duration) -> Result<Value, String> {
    let url = format!("{}{path}", base_url(key));
    cached_get(&RESPONSES, &url, ttl, FAILURE_BACKOFF, || fetch(path, key))
}

/// `url` from the cache while fresh, else from `fetch`. The lock is not held across `fetch`.
fn cached_get(
    cache: &Mutex<ResponseCache>,
    url: &str,
    ttl: Duration,
    backoff: Duration,
    fetch: impl FnOnce() -> Result<Value, String>,
) -> Result<Value, String> {
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

fn fetch(path: &str, key: Option<&str>) -> Result<Value, String> {
    let url = format!("{}{path}", base_url(key));
    let mut headers = vec![("Accept", "application/json")];
    if let Some(key) = key {
        headers.push(("X-CMC_PRO_API_KEY", key));
    }
    let body = crate::service::net::http::get_text(
        &url,
        &headers,
        crate::service::net::http::Retry::Transient,
    )
    .map_err(|error| format!("CoinMarketCap error: {error}"))?;
    let value: Value =
        serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;
    if value["status"]["error_code"].as_i64().unwrap_or(0) != 0 {
        return Err(value["status"]["error_message"]
            .as_str()
            .unwrap_or("CoinMarketCap API error")
            .to_owned());
    }
    Ok(value)
}

fn to_klines(points: Vec<(i64, f64)>, limit: u32) -> Result<Vec<Kline>, String> {
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
        return Err("No CoinMarketCap data".to_owned());
    }
    Ok(downsample(&candles, limit as usize))
}

impl MarketSource for CoinMarketCap {
    fn fetch_klines(
        &self,
        symbol: &str,
        _interval: &str,
        limit: u32,
    ) -> Result<Vec<Kline>, String> {
        let key = api_key();
        match symbol {
            ALTCOIN_HISTORICAL => {
                let value = get(
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
            FEAR_GREED_HISTORICAL => to_klines(fear_greed_series(key.as_deref())?, limit),
            ALTCOIN_LATEST | FEAR_GREED_LATEST => {
                let path = if symbol == ALTCOIN_LATEST {
                    "/v1/altcoin-season-index/latest"
                } else {
                    "/v3/fear-and-greed/latest"
                };
                let (time, mut reading) =
                    parse_latest(symbol, &get(path, key.as_deref(), LATEST_TTL)?)
                        .ok_or_else(|| "No CoinMarketCap data".to_owned())?;
                if symbol == FEAR_GREED_LATEST {
                    // The history only decorates the gauge, so a failure here is not an error.
                    // On a failure the previous history stays rather than the gauge losing it.
                    reading.history = fear_greed_history(key.as_deref())
                        .ok()
                        .or_else(|| latest_reading(symbol).map(|previous| previous.history))
                        .unwrap_or_default();
                }
                let value = reading.value;
                if let Ok(mut latest) = LATEST.lock() {
                    latest.insert(symbol.to_owned(), reading);
                }
                // One point keeps the candle pipeline (loading, errors, alerts) working; the
                // chart body draws the cached reading as a gauge instead of a line.
                to_klines(vec![(time, value)], limit)
            }
            _ => Err("Unknown CoinMarketCap chart".to_owned()),
        }
    }

    fn search_tickers(&self, _query: &str) -> Result<Vec<SearchResult>, String> {
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
            })
            .collect())
    }
}

/// Readings older than this behind the newest one can't be the current end of the series.
const FRESH_WITHIN_MS: i64 = 3 * 86_400_000;
const HISTORY_PAGE: usize = 500;
const MAX_HISTORY_PAGES: usize = 4;

/// Every Fear & Greed reading CMC will page out, oldest first. The endpoint doesn't document its
/// order, so pages are followed until the newest reading is current (one request when the
/// newest come first).
fn fear_greed_series(key: Option<&str>) -> Result<Vec<(i64, f64)>, String> {
    let mut points: Vec<(i64, f64)> = Vec::new();
    for page in 0..MAX_HISTORY_PAGES {
        let path = format!(
            "/v3/fear-and-greed/historical?start={}&limit={HISTORY_PAGE}",
            page * HISTORY_PAGE + 1
        );
        let value = get(&path, key, HISTORY_TTL)?;
        let fetched = parse_series(&value);
        let full = fetched.len() >= HISTORY_PAGE;
        points.extend(fetched);
        let newest = points.iter().map(|(time, _)| *time).max().unwrap_or(0);
        if !full || newest >= now_ms() - FRESH_WITHIN_MS {
            break;
        }
    }
    points.sort_by_key(|(time, _)| *time);
    points.dedup_by_key(|(time, _)| *time);
    Ok(points)
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

/// The most recent 90 daily readings, oldest first, for the gauge.
fn fear_greed_history(key: Option<&str>) -> Result<Vec<(i64, f64)>, String> {
    let mut points = fear_greed_series(key)?;
    let keep = points.len().saturating_sub(90);
    points.drain(..keep);
    Ok(points)
}

/// Settings ▸ CoinMarketCap ▸ Test Connection: one cheap authenticated-if-configured request.
pub fn test_connection() -> Result<(), String> {
    fetch("/v3/fear-and-greed/latest", api_key().as_deref()).map(|_| ())
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

    #[test]
    fn parses_the_latest_readings() {
        let altcoin: Value = serde_json::from_str(
            r#"{"data":{"altcoin_index":60,"altcoin_marketcap":1.5e12,"snapshot_time":"2026-10-11T00:00:00.000Z","yearly_high":64,"yearly_high_date":"2026-09-27","yearly_low":14,"yearly_low_date":"2025-12-19"}}"#,
        )
        .unwrap();
        let (_, reading) = parse_latest(ALTCOIN_LATEST, &altcoin).unwrap();
        assert_eq!(reading.value, 60.0);
        assert_eq!(reading.classification, "Neutral");
        assert_eq!(reading.yearly_high, Some((64.0, "2026-09-27".to_owned())));
        assert_eq!(reading.yearly_low, Some((14.0, "2025-12-19".to_owned())));
        assert_eq!(reading.altcoin_market_cap, Some(1.5e12));

        let fear: Value = serde_json::from_str(
            r#"{"data":{"value":56,"update_time":"2026-10-11T08:00:00.000Z","value_classification":"Neutral"}}"#,
        )
        .unwrap();
        let (time, reading) = parse_latest(FEAR_GREED_LATEST, &fear).unwrap();
        assert!(time > 0);
        assert_eq!(reading.value, 56.0);
        assert_eq!(reading.classification, "Neutral");
        assert_eq!(reading.yearly_high, None);
        assert_eq!(season_classification(80.0), "Altcoin Season");
        assert_eq!(season_classification(25.0), "Bitcoin Season");
    }

    #[test]
    fn history_lookbacks_and_range() {
        let day = 86_400_000;
        let reading = LatestReading {
            value: 50.0,
            classification: String::new(),
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
        result: Result<Value, String>,
    ) -> impl FnOnce() -> Result<Value, String> + '_ {
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
        let failure = || Err("HTTP 429".to_owned());
        assert_eq!(
            cached_get(&cache, "u", minute, minute, counted(&calls, failure())),
            Err("HTTP 429".to_owned())
        );
        assert_eq!(
            cached_get(
                &cache,
                "u",
                minute,
                minute,
                counted(&calls, Ok(Value::Null))
            ),
            Err("HTTP 429".to_owned())
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
}
