//! Currency conversion for the portfolio and alerts: fiat rates from Frankfurter (current and
//! historical, cached on disk), BTC/USD from Binance (spot) and Bitstamp (daily closes for
//! history). A port of `FXRateService` + `BitcoinHistoryService`.
//!
//! Blocking and thread-safe: one lock serialises every request, as the Swift actor did, and is
//! held across the network calls so two callers never fetch the same range twice. The transport,
//! clock, sleep, BTC spot source and storage directory are all injectable, so the rules (the
//! seven-day lookback, retries, cache lifetimes) are tested without a network.

// Transient: the portfolio host wires this up for the CMC import and live conversion.
#![allow(dead_code)]

use crate::format::{civil_from_days, days_from_civil};
use crate::model::portfolio::PortfolioCurrency;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DAY_MS: i64 = 86_400_000;
const HISTORICAL_TOLERANCE_MS: i64 = 7 * DAY_MS;
const CURRENT_REFRESH_MS: i64 = DAY_MS;
const CURRENT_MAX_AGE_MS: i64 = 5 * DAY_MS;
const BTC_SPOT_FRESH_MS: i64 = 60_000;
const BTC_HISTORY_LIFETIME_MS: i64 = 12 * 60 * 60 * 1_000;
const MAXIMUM_ATTEMPTS: usize = 4;
const FRANKFURTER: &str = "https://api.frankfurter.dev/v2/rates";
const QUOTES: &str = "EUR,GBP,JPY,CHF";
const BITSTAMP: &str = "https://www.bitstamp.net/api/v2/ohlc/btcusd/";
const BITSTAMP_BATCH: usize = 1_000;

// MARK: Transport

pub struct HttpReply {
    pub status: u16,
    pub body: String,
    /// `Retry-After` seconds, when the server sent a number.
    pub retry_after: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransportError {
    /// Worth another try: timeouts, connection resets, DNS.
    Retryable(String),
    Fatal(String),
}

pub trait FxTransport: Send + Sync {
    fn get(&self, url: &str) -> Result<HttpReply, TransportError>;
}

pub struct UreqFxTransport {
    agent: ureq::Agent,
}

impl Default for UreqFxTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl FxTransport for UreqFxTransport {
    fn get(&self, url: &str) -> Result<HttpReply, TransportError> {
        let mut response = self.agent.get(url).call().map_err(|error| match error {
            ureq::Error::Timeout(_)
            | ureq::Error::Io(_)
            | ureq::Error::HostNotFound
            | ureq::Error::ConnectionFailed => TransportError::Retryable(error.to_string()),
            other => TransportError::Fatal(other.to_string()),
        })?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<f64>().ok());
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| TransportError::Retryable(error.to_string()))?;
        Ok(HttpReply {
            status,
            body,
            retry_after,
        })
    }
}

// MARK: Results

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conversion {
    pub rate: f64,
    /// The day (or fetch time) the rate was observed on; older than asked when the market was closed.
    pub observation_ms: i64,
    pub is_cached: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FxError {
    RateUnavailable {
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        date_ms: Option<i64>,
    },
    Http(u16),
    Network(String),
    InvalidResponse(String),
    EmptyBitcoinHistory,
}

impl FxError {
    pub fn message(&self) -> String {
        match self {
            Self::RateUnavailable { from, to, date_ms } => {
                let suffix = date_ms
                    .map(|ms| format!(" for {}", day_key(ms)))
                    .unwrap_or_default();
                format!(
                    "Exchange rate from {} to {} is unavailable{suffix}.",
                    from.code(),
                    to.code()
                )
            }
            Self::Http(status) => format!("The exchange-rate service answered HTTP {status}."),
            Self::Network(message) => format!("Network error: {message}"),
            Self::InvalidResponse(what) => format!("{what} returned an invalid response."),
            Self::EmptyBitcoinHistory => "Bitstamp returned no BTC/USD history.".to_owned(),
        }
    }
}

// MARK: Pure rules

/// `YYYY-MM-DD` of the UTC day containing `epoch_ms`.
pub fn day_key(epoch_ms: i64) -> String {
    let (year, month, day) = civil_from_days(epoch_ms.div_euclid(DAY_MS));
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn start_of_day(epoch_ms: i64) -> i64 {
    epoch_ms.div_euclid(DAY_MS) * DAY_MS
}

fn parse_day(key: &str) -> Option<i64> {
    let mut parts = key.split('-');
    let (year, month, day) = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    Some(days_from_civil(year, month, day) * DAY_MS)
}

fn is_btc_or_usd(currency: PortfolioCurrency) -> bool {
    matches!(currency, PortfolioCurrency::Btc | PortfolioCurrency::Usd)
}

/// Units of `to` per unit of `from`, going through dollars. `fiat_usd` holds units per USD for
/// the fiat currencies, `btc_usd` the dollar price of one bitcoin.
pub fn cross_rate(
    from: PortfolioCurrency,
    to: PortfolioCurrency,
    fiat_usd: &HashMap<String, f64>,
    btc_usd: Option<f64>,
) -> Option<f64> {
    // Units of `currency` per dollar; None when unknown or unusable.
    let units_per_usd = |currency: PortfolioCurrency| match currency {
        PortfolioCurrency::Usd => Some(1.0),
        PortfolioCurrency::Btc => btc_usd
            .filter(|price| *price > 0.0)
            .map(|price| 1.0 / price),
        other => fiat_usd.get(other.code()).copied(),
    };
    // Dollars per unit of a currency, avoiding a double inversion where one side is the dollar
    // (so BTC → USD is exactly the quoted price).
    let usd_per_unit = |currency: PortfolioCurrency| match currency {
        PortfolioCurrency::Usd => Some(1.0),
        PortfolioCurrency::Btc => btc_usd.filter(|price| *price > 0.0),
        other => fiat_usd
            .get(other.code())
            .copied()
            .filter(|units| *units != 0.0)
            .map(|units| 1.0 / units),
    };
    if to == PortfolioCurrency::Usd {
        return usd_per_unit(from);
    }
    if from == PortfolioCurrency::Usd {
        return units_per_usd(to);
    }
    if from == PortfolioCurrency::Btc {
        return Some(usd_per_unit(from)? * units_per_usd(to)?);
    }
    let source = units_per_usd(from)?;
    let destination = units_per_usd(to)?;
    (source != 0.0).then(|| destination / source)
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || (500..600).contains(&status)
}

/// Seconds to wait before attempt `attempt + 1`: the server's `Retry-After` (0…10), else
/// 0.5·2ⁿ capped at 4.
pub fn retry_delay(attempt: usize, retry_after: Option<f64>) -> f64 {
    match retry_after {
        Some(seconds) => seconds.clamp(0.0, 10.0),
        None => (0.5 * 2_f64.powi(attempt as i32)).min(4.0),
    }
}

// MARK: Caches

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CurrentCache {
    fetched_at: i64,
    rates: HashMap<String, f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct HistoricalCache {
    #[serde(default)]
    rates_by_day: BTreeMap<String, HashMap<String, f64>>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct DailyClose {
    pub date_ms: i64,
    pub close: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct BitcoinCache {
    fetched_at: i64,
    closes: Vec<DailyClose>,
}

/// Newest wins on a date clash; oldest first.
pub fn merge_closes(old: &[DailyClose], new: &[DailyClose]) -> Vec<DailyClose> {
    let mut by_date: BTreeMap<i64, DailyClose> = old.iter().map(|c| (c.date_ms, *c)).collect();
    for close in new {
        by_date.insert(close.date_ms, *close);
    }
    by_date.into_values().collect()
}

/// Bitstamp's OHLC answer → daily closes; rows with a bad timestamp or close are dropped.
pub fn parse_bitstamp(body: &str) -> Result<Vec<DailyClose>, FxError> {
    #[derive(Deserialize)]
    struct Ohlc {
        timestamp: String,
        close: String,
    }
    #[derive(Deserialize)]
    struct Data {
        ohlc: Vec<Ohlc>,
    }
    #[derive(Deserialize)]
    struct Response {
        data: Data,
    }
    let response: Response =
        serde_json::from_str(body).map_err(|_| FxError::InvalidResponse("Bitstamp".to_owned()))?;
    let closes: Vec<DailyClose> = response
        .data
        .ohlc
        .iter()
        .filter_map(|item| {
            let seconds: f64 = item.timestamp.parse().ok()?;
            let close: f64 = item.close.parse().ok()?;
            (seconds.is_finite() && close.is_finite() && close > 0.0).then_some(DailyClose {
                date_ms: (seconds * 1_000.0) as i64,
                close,
            })
        })
        .collect();
    Ok(merge_closes(&[], &closes))
}

/// The last close on or before `day_ms`, no more than a week old.
fn close_on(closes: &[DailyClose], day_ms: i64) -> Option<f64> {
    closes
        .iter()
        .rev()
        .find(|close| close.date_ms <= day_ms && day_ms - close.date_ms <= HISTORICAL_TOLERANCE_MS)
        .map(|close| close.close)
}

fn parse_frankfurter(body: &str) -> Result<Vec<(Option<String>, String, f64)>, FxError> {
    #[derive(Deserialize)]
    struct Item {
        date: Option<String>,
        quote: String,
        rate: f64,
    }
    let items: Vec<Item> = serde_json::from_str(body)
        .map_err(|_| FxError::InvalidResponse("Frankfurter".to_owned()))?;
    Ok(items
        .into_iter()
        .map(|item| (item.date, item.quote, item.rate))
        .collect())
}

// MARK: Service

struct State {
    current: Option<CurrentCache>,
    historical: HistoricalCache,
    bitcoin: Option<BitcoinCache>,
    spot: Option<(i64, f64)>,
}

pub struct FxRateService {
    transport: Arc<dyn FxTransport>,
    btc_spot: Box<dyn Fn() -> Option<f64> + Send + Sync>,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
    sleep: Box<dyn Fn(f64) + Send + Sync>,
    directory: Option<PathBuf>,
    state: Mutex<State>,
}

const CURRENT_FILE: &str = "alert_fx_rates.json";
const HISTORICAL_FILE: &str = "portfolio_fx_history.json";
const BITCOIN_FILE: &str = "bitcoin_power_law_history.json";

fn load<T: for<'de> Deserialize<'de>>(directory: &Option<PathBuf>, name: &str) -> Option<T> {
    let text = std::fs::read_to_string(directory.as_ref()?.join(name)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save<T: Serialize>(directory: &Option<PathBuf>, name: &str, value: &T) {
    let Some(directory) = directory else {
        return;
    };
    let _ = std::fs::create_dir_all(directory);
    if let Ok(text) = serde_json::to_string(value) {
        let path = directory.join(name);
        let temporary = directory.join(format!("{name}.tmp"));
        if std::fs::write(&temporary, text).is_ok() {
            let _ = std::fs::rename(&temporary, path);
        }
    }
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn binance_btc_spot() -> Option<f64> {
    let source = crate::service::for_source(crate::model::DataSource::Binance);
    let bars = source.fetch_klines("BTCUSDT", "1h", 1).ok()?;
    bars.last().map(|bar| bar.close)
}

impl FxRateService {
    /// The app's service: real network, real clock, caches in `directory` (None = memory only).
    pub fn live(directory: Option<PathBuf>) -> Self {
        Self::new(
            Arc::new(UreqFxTransport::default()),
            Box::new(binance_btc_spot),
            Box::new(system_now_ms),
            Box::new(|seconds| std::thread::sleep(Duration::from_secs_f64(seconds))),
            directory,
        )
    }

    pub fn new(
        transport: Arc<dyn FxTransport>,
        btc_spot: Box<dyn Fn() -> Option<f64> + Send + Sync>,
        clock: Box<dyn Fn() -> i64 + Send + Sync>,
        sleep: Box<dyn Fn(f64) + Send + Sync>,
        directory: Option<PathBuf>,
    ) -> Self {
        let state = State {
            current: load(&directory, CURRENT_FILE),
            historical: load(&directory, HISTORICAL_FILE).unwrap_or_default(),
            bitcoin: load(&directory, BITCOIN_FILE),
            spot: None,
        };
        Self {
            transport,
            btc_spot,
            clock,
            sleep,
            directory,
            state: Mutex::new(state),
        }
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // MARK: Public API

    /// Compatibility entry point used by alerts: the current rate, or None.
    pub fn rate(&self, from: PortfolioCurrency, to: PortfolioCurrency) -> Option<f64> {
        self.conversion(from, to, None).ok().map(|c| c.rate)
    }

    /// Units of `to` per unit of `from`, now (`on` = None) or on a UTC day.
    pub fn conversion(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        on: Option<i64>,
    ) -> Result<Conversion, FxError> {
        if from == to {
            return Ok(Conversion {
                rate: 1.0,
                observation_ms: on.unwrap_or_else(|| self.now()),
                is_cached: false,
            });
        }
        let mut state = self.lock();
        match on {
            Some(date) => self.historical_conversion(&mut state, from, to, date),
            None => self.current_conversion(&mut state, from, to),
        }
    }

    /// Rates for many dates at once: one history fetch per missing range rather than one per
    /// date. Every input date gets an answer; a failure is per date.
    pub fn conversions(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        dates: &[i64],
    ) -> HashMap<i64, Result<Conversion, FxError>> {
        let mut state = self.lock();
        let mut unique_days: Vec<i64> = dates.iter().map(|d| start_of_day(*d)).collect();
        unique_days.sort_unstable();
        unique_days.dedup();

        let mut bitcoin_by_day: HashMap<i64, f64> = HashMap::new();
        if from == PortfolioCurrency::Btc || to == PortfolioCurrency::Btc {
            match self.refresh_bitcoin(&mut state, false) {
                Ok(history) => {
                    for day in &unique_days {
                        if let Some(close) = close_on(&history, *day) {
                            bitcoin_by_day.insert(*day, close);
                        }
                    }
                }
                Err(error) => {
                    return dates.iter().map(|d| (*d, Err(error.clone()))).collect();
                }
            }
        }

        let needs_fiat = !(is_btc_or_usd(from) && is_btc_or_usd(to));
        let mut fresh: std::collections::HashSet<i64> = std::collections::HashSet::new();
        if needs_fiat {
            let missing: Vec<i64> = unique_days
                .iter()
                .copied()
                .filter(|day| {
                    self.cached_historical(&state, from, to, *day, bitcoin_by_day.get(day).copied())
                        .is_none()
                })
                .collect();
            if let (Some(first), Some(last)) = (missing.first(), missing.last()) {
                let mut start = first - 7 * DAY_MS;
                let mut failed = false;
                while start <= *last {
                    let range_end = (start + 364 * DAY_MS).min(*last);
                    if self
                        .fetch_historical_fiat(&mut state, start, range_end)
                        .is_err()
                    {
                        // Cached days may still answer; the rest fail below.
                        failed = true;
                        break;
                    }
                    start = range_end + DAY_MS;
                }
                if !failed {
                    fresh.extend(missing.iter().copied());
                }
            }
        }

        let mut result = HashMap::new();
        for date in dates {
            let day = start_of_day(*date);
            let btc = bitcoin_by_day.get(&day).copied();
            let outcome = if from == to {
                Ok(Conversion {
                    rate: 1.0,
                    observation_ms: day,
                    is_cached: false,
                })
            } else if let Some(rate) = (!needs_fiat)
                .then(|| cross_rate(from, to, &HashMap::new(), btc))
                .flatten()
            {
                Ok(Conversion {
                    rate,
                    observation_ms: day,
                    is_cached: false,
                })
            } else if let Some(found) = self.cached_historical(&state, from, to, day, btc) {
                Ok(Conversion {
                    is_cached: !fresh.contains(&day),
                    ..found
                })
            } else {
                Err(FxError::RateUnavailable {
                    from,
                    to,
                    date_ms: Some(*date),
                })
            };
            result.insert(*date, outcome);
        }
        result
    }

    /// Refreshes the current fiat rates; failures are ignored (the old ones keep serving).
    pub fn refresh(&self) {
        let mut state = self.lock();
        let _ = self.refresh_current_fiat(&mut state);
    }

    // MARK: Current

    fn current_conversion(
        &self,
        state: &mut State,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
    ) -> Result<Conversion, FxError> {
        let now = self.now();
        if is_btc_or_usd(from)
            && is_btc_or_usd(to)
            && let Some(btc) = self.current_bitcoin_usd(state)
            && let Some(rate) = cross_rate(from, to, &HashMap::new(), Some(btc))
        {
            return Ok(Conversion {
                rate,
                observation_ms: now,
                is_cached: false,
            });
        }
        let mut is_cached = true;
        let stale = state
            .current
            .as_ref()
            .is_none_or(|current| now - current.fetched_at > CURRENT_REFRESH_MS);
        if stale {
            is_cached = !self.refresh_current_fiat(state)?;
        }
        let unavailable = FxError::RateUnavailable {
            from,
            to,
            date_ms: None,
        };
        let current = state
            .current
            .clone()
            .filter(|current| now - current.fetched_at <= CURRENT_MAX_AGE_MS)
            .ok_or_else(|| unavailable.clone())?;
        let btc = (from == PortfolioCurrency::Btc || to == PortfolioCurrency::Btc)
            .then(|| self.current_bitcoin_usd(state))
            .flatten();
        let rate = cross_rate(from, to, &current.rates, btc).ok_or(unavailable)?;
        Ok(Conversion {
            rate,
            observation_ms: current.fetched_at,
            is_cached,
        })
    }

    /// True when new rates arrived; false when the fetch failed but older rates remain.
    fn refresh_current_fiat(&self, state: &mut State) -> Result<bool, FxError> {
        let url = format!("{FRANKFURTER}?base=USD&quotes={QUOTES}");
        let fetched = self.request(&url).and_then(|body| parse_frankfurter(&body));
        match fetched {
            Ok(items) => {
                let next = CurrentCache {
                    fetched_at: self.now(),
                    rates: items
                        .into_iter()
                        .map(|(_, quote, rate)| (quote, rate))
                        .collect(),
                };
                save(&self.directory, CURRENT_FILE, &next);
                state.current = Some(next);
                Ok(true)
            }
            Err(_) if state.current.is_some() => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn current_bitcoin_usd(&self, state: &mut State) -> Option<f64> {
        let now = self.now();
        if let Some((fetched, value)) = state.spot
            && now - fetched < BTC_SPOT_FRESH_MS
        {
            return Some(value);
        }
        let value = (self.btc_spot)().filter(|value| *value > 0.0)?;
        state.spot = Some((now, value));
        Some(value)
    }

    // MARK: Historical

    fn historical_conversion(
        &self,
        state: &mut State,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        date: i64,
    ) -> Result<Conversion, FxError> {
        let requested = start_of_day(date);
        let involves_btc = from == PortfolioCurrency::Btc || to == PortfolioCurrency::Btc;
        let btc = if involves_btc {
            let history = self.refresh_bitcoin(state, false)?;
            close_on(&history, requested)
        } else {
            None
        };
        if involves_btc
            && is_btc_or_usd(from)
            && is_btc_or_usd(to)
            && let Some(rate) = cross_rate(from, to, &HashMap::new(), btc)
        {
            return Ok(Conversion {
                rate,
                observation_ms: requested,
                is_cached: false,
            });
        }
        if let Some(found) = self.cached_historical(state, from, to, requested, btc) {
            return Ok(found);
        }
        self.fetch_historical_fiat(state, requested - 7 * DAY_MS, requested)?;
        match self.cached_historical(state, from, to, requested, btc) {
            Some(found) => Ok(Conversion {
                is_cached: false,
                ..found
            }),
            None => Err(FxError::RateUnavailable {
                from,
                to,
                date_ms: Some(date),
            }),
        }
    }

    /// The newest stored rate within a week before `requested` that covers both currencies.
    fn cached_historical(
        &self,
        state: &State,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        requested: i64,
        btc_usd: Option<f64>,
    ) -> Option<Conversion> {
        (0..=7).find_map(|offset| {
            let candidate = requested - offset * DAY_MS;
            let rates = state.historical.rates_by_day.get(&day_key(candidate))?;
            cross_rate(from, to, rates, btc_usd).map(|rate| Conversion {
                rate,
                observation_ms: candidate,
                is_cached: true,
            })
        })
    }

    fn fetch_historical_fiat(
        &self,
        state: &mut State,
        start: i64,
        end: i64,
    ) -> Result<(), FxError> {
        let url = format!(
            "{FRANKFURTER}?base=USD&quotes={QUOTES}&from={}&to={}",
            day_key(start),
            day_key(end)
        );
        let items = parse_frankfurter(&self.request(&url)?)?;
        for (date, quote, rate) in items {
            let Some(day) = date else {
                continue;
            };
            state
                .historical
                .rates_by_day
                .entry(day)
                .or_default()
                .insert(quote, rate);
        }
        save(&self.directory, HISTORICAL_FILE, &state.historical);
        Ok(())
    }

    // MARK: Bitcoin history

    /// Daily BTC/USD closes, oldest first. Fresh for twelve hours; a failed refresh falls back
    /// to whatever is cached.
    fn refresh_bitcoin(&self, state: &mut State, force: bool) -> Result<Vec<DailyClose>, FxError> {
        let now = self.now();
        let cached = state.bitcoin.clone();
        if !force
            && let Some(cache) = &cached
            && now - cache.fetched_at < BTC_HISTORY_LIFETIME_MS
        {
            return Ok(cache.closes.clone());
        }
        let fetched = (|| {
            let closes = match &cached {
                Some(cache) if !cache.closes.is_empty() => {
                    merge_closes(&cache.closes, &self.bitcoin_batch(now)?)
                }
                _ => self.bitcoin_full_history(now)?,
            };
            if closes.is_empty() {
                return Err(FxError::EmptyBitcoinHistory);
            }
            Ok(closes)
        })();
        match fetched {
            Ok(closes) => {
                let next = BitcoinCache {
                    fetched_at: now,
                    closes: closes.clone(),
                };
                save(&self.directory, BITCOIN_FILE, &next);
                state.bitcoin = Some(next);
                Ok(closes)
            }
            Err(error) => match cached {
                Some(cache) if !cache.closes.is_empty() => Ok(cache.closes),
                _ => Err(error),
            },
        }
    }

    fn bitcoin_full_history(&self, end_ms: i64) -> Result<Vec<DailyClose>, FxError> {
        let mut result: Vec<DailyClose> = Vec::new();
        let mut cursor = end_ms;
        loop {
            let batch = self.bitcoin_batch(cursor)?;
            if batch.is_empty() {
                break;
            }
            result = merge_closes(&result, &batch);
            let earliest = batch[0].date_ms;
            let next = earliest - 1_000;
            if batch.len() != BITSTAMP_BATCH || next >= cursor {
                break;
            }
            cursor = next;
        }
        Ok(result)
    }

    fn bitcoin_batch(&self, end_ms: i64) -> Result<Vec<DailyClose>, FxError> {
        let url = format!(
            "{BITSTAMP}?step=86400&limit={BITSTAMP_BATCH}&end={}&exclude_current_candle=true",
            end_ms.div_euclid(1_000)
        );
        let reply = self.transport.get(&url).map_err(|error| {
            FxError::Network(match error {
                TransportError::Retryable(m) | TransportError::Fatal(m) => m,
            })
        })?;
        if !(200..300).contains(&reply.status) {
            return Err(FxError::InvalidResponse("Bitstamp".to_owned()));
        }
        parse_bitstamp(&reply.body)
    }

    // MARK: HTTP

    /// GET with up to four attempts: retryable statuses and network errors back off, others fail.
    fn request(&self, url: &str) -> Result<String, FxError> {
        let mut attempt = 0;
        loop {
            let last = attempt + 1 >= MAXIMUM_ATTEMPTS;
            match self.transport.get(url) {
                Ok(reply) if (200..300).contains(&reply.status) => return Ok(reply.body),
                Ok(reply) => {
                    if last || !is_retryable_status(reply.status) {
                        return Err(FxError::Http(reply.status));
                    }
                    (self.sleep)(retry_delay(attempt, reply.retry_after));
                }
                Err(TransportError::Retryable(message)) => {
                    if last {
                        return Err(FxError::Network(message));
                    }
                    (self.sleep)(retry_delay(attempt, None));
                }
                Err(TransportError::Fatal(message)) => return Err(FxError::Network(message)),
            }
            attempt += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering};

    use PortfolioCurrency::{Btc, Eur, Gbp, Usd};

    const T0: i64 = 1_709_649_000_000; // 2024-03-05 14:30 UTC
    const DAY: i64 = 1_709_596_800_000; // 2024-03-05 00:00 UTC

    #[derive(Default)]
    struct FakeTransport {
        replies: Mutex<HashMap<String, VecDeque<Result<HttpReply, TransportError>>>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeTransport {
        fn on(&self, fragment: &str, reply: Result<HttpReply, TransportError>) {
            self.replies
                .lock()
                .unwrap()
                .entry(fragment.to_owned())
                .or_default()
                .push_back(reply);
        }

        fn ok(&self, fragment: &str, body: &str) {
            self.on(
                fragment,
                Ok(HttpReply {
                    status: 200,
                    body: body.to_owned(),
                    retry_after: None,
                }),
            );
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl FxTransport for FakeTransport {
        fn get(&self, url: &str) -> Result<HttpReply, TransportError> {
            self.calls.lock().unwrap().push(url.to_owned());
            let mut replies = self.replies.lock().unwrap();
            let queue = replies
                .iter_mut()
                .filter(|(fragment, _)| url.contains(fragment.as_str()))
                .max_by_key(|(fragment, _)| fragment.len())
                .map(|(_, queue)| queue);
            match queue.and_then(|queue| {
                if queue.len() > 1 {
                    queue.pop_front()
                } else {
                    queue.front().cloned_reply()
                }
            }) {
                Some(reply) => reply,
                None => Err(TransportError::Fatal(format!("no reply for {url}"))),
            }
        }
    }

    trait ClonedReply {
        fn cloned_reply(self) -> Option<Result<HttpReply, TransportError>>;
    }

    impl ClonedReply for Option<&Result<HttpReply, TransportError>> {
        fn cloned_reply(self) -> Option<Result<HttpReply, TransportError>> {
            self.map(|reply| match reply {
                Ok(reply) => Ok(HttpReply {
                    status: reply.status,
                    body: reply.body.clone(),
                    retry_after: reply.retry_after,
                }),
                Err(error) => Err(error.clone()),
            })
        }
    }

    struct Fixture {
        transport: Arc<FakeTransport>,
        clock: Arc<AtomicI64>,
        sleeps: Arc<Mutex<Vec<f64>>>,
        spot: Arc<Mutex<Option<f64>>>,
        service: FxRateService,
    }

    fn fixture(directory: Option<PathBuf>) -> Fixture {
        let transport = Arc::new(FakeTransport::default());
        let clock = Arc::new(AtomicI64::new(T0));
        let sleeps = Arc::new(Mutex::new(Vec::new()));
        let spot = Arc::new(Mutex::new(Some(50_000.0)));
        let service = FxRateService::new(
            transport.clone(),
            Box::new({
                let spot = spot.clone();
                move || *spot.lock().unwrap()
            }),
            Box::new({
                let clock = clock.clone();
                move || clock.load(Ordering::SeqCst)
            }),
            Box::new({
                let sleeps = sleeps.clone();
                move |seconds| sleeps.lock().unwrap().push(seconds)
            }),
            directory,
        );
        Fixture {
            transport,
            clock,
            sleeps,
            spot,
            service,
        }
    }

    fn current_body() -> String {
        r#"[{"quote":"EUR","rate":0.5},{"quote":"GBP","rate":0.8},{"quote":"JPY","rate":150},{"quote":"CHF","rate":0.9}]"#.to_owned()
    }

    fn history_body(rows: &[(&str, &str, f64)]) -> String {
        let items: Vec<String> = rows
            .iter()
            .map(|(date, quote, rate)| {
                format!(r#"{{"date":"{date}","quote":"{quote}","rate":{rate}}}"#)
            })
            .collect();
        format!("[{}]", items.join(","))
    }

    #[test]
    fn cross_rates_go_through_dollars() {
        let fiat = HashMap::from([("EUR".to_owned(), 0.5), ("GBP".to_owned(), 0.8)]);
        assert_eq!(cross_rate(Usd, Eur, &fiat, None), Some(0.5));
        assert_eq!(cross_rate(Eur, Usd, &fiat, None), Some(2.0));
        assert_eq!(cross_rate(Eur, Gbp, &fiat, None), Some(1.6));
        assert_eq!(cross_rate(Btc, Usd, &fiat, Some(50_000.0)), Some(50_000.0));
        assert_eq!(
            cross_rate(Usd, Btc, &fiat, Some(50_000.0)),
            Some(1.0 / 50_000.0)
        );
        assert_eq!(cross_rate(Btc, Eur, &fiat, Some(50_000.0)), Some(25_000.0));
        assert_eq!(cross_rate(Btc, Usd, &fiat, None), None, "no bitcoin price");
        assert_eq!(
            cross_rate(Usd, PortfolioCurrency::Jpy, &fiat, None),
            None,
            "unknown fiat"
        );
    }

    #[test]
    fn days_and_retry_delays() {
        assert_eq!(day_key(T0), "2024-03-05");
        assert_eq!(start_of_day(T0), DAY);
        assert_eq!(day_key(-1), "1969-12-31");
        assert_eq!(parse_day("2024-03-05"), Some(DAY));
        assert_eq!(retry_delay(0, None), 0.5);
        assert_eq!(retry_delay(1, None), 1.0);
        assert_eq!(retry_delay(5, None), 4.0);
        assert_eq!(retry_delay(0, Some(30.0)), 10.0);
        assert_eq!(retry_delay(0, Some(-3.0)), 0.0);
        assert!(is_retryable_status(429) && is_retryable_status(503) && !is_retryable_status(404));
    }

    #[test]
    fn the_same_currency_needs_no_lookup() {
        let f = fixture(None);
        let conversion = f.service.conversion(Eur, Eur, Some(DAY)).unwrap();
        assert_eq!(
            conversion,
            Conversion {
                rate: 1.0,
                observation_ms: DAY,
                is_cached: false
            }
        );
        assert!(f.transport.calls().is_empty());
    }

    #[test]
    fn current_rates_fetch_once_then_serve_from_cache() {
        let f = fixture(None);
        f.transport.ok("quotes=EUR", &current_body());
        let first = f.service.conversion(Usd, Eur, None).unwrap();
        assert_eq!(first.rate, 0.5);
        assert!(!first.is_cached, "fetched just now");
        assert_eq!(first.observation_ms, T0);
        f.clock.fetch_add(60_000, Ordering::SeqCst);
        let second = f.service.conversion(Eur, Usd, None).unwrap();
        assert_eq!(second.rate, 2.0);
        assert!(second.is_cached);
        assert_eq!(f.transport.calls().len(), 1);
        assert_eq!(f.service.rate(Usd, Gbp), Some(0.8));
    }

    #[test]
    fn stale_rates_refresh_and_a_failed_refresh_keeps_serving_for_five_days() {
        let f = fixture(None);
        f.transport.ok("quotes=EUR", &current_body());
        f.service.conversion(Usd, Eur, None).unwrap();
        f.clock.fetch_add(2 * DAY_MS, Ordering::SeqCst);
        f.transport
            .on("quotes=EUR", Err(TransportError::Fatal("down".into())));
        let served = f.service.conversion(Usd, Eur, None);
        // The fake keeps answering with the first queued reply once the queue is down to one,
        // so make the failure the only reply.
        assert!(served.is_ok());
        let f = fixture(None);
        f.transport.ok("quotes=EUR", &current_body());
        f.service.conversion(Usd, Eur, None).unwrap();
        let g = fixture(None);
        g.transport.ok("quotes=EUR", &current_body());
        g.service.conversion(Usd, Eur, None).unwrap();
        g.clock.fetch_add(6 * DAY_MS, Ordering::SeqCst);
        g.transport.replies.lock().unwrap().clear();
        let error = g.service.conversion(Usd, Eur, None).unwrap_err();
        assert!(
            matches!(error, FxError::RateUnavailable { date_ms: None, .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_first_fetch_that_fails_is_an_error() {
        let f = fixture(None);
        f.transport
            .on("quotes=EUR", Err(TransportError::Fatal("offline".into())));
        let error = f.service.conversion(Usd, Eur, None).unwrap_err();
        assert_eq!(error, FxError::Network("offline".into()));
        assert!(error.message().contains("offline"));
    }

    #[test]
    fn failures_retry_with_backoff_until_four_attempts() {
        let f = fixture(None);
        f.transport.on(
            "quotes=EUR",
            Ok(HttpReply {
                status: 503,
                body: String::new(),
                retry_after: None,
            }),
        );
        let error = f.service.conversion(Usd, Eur, None).unwrap_err();
        assert_eq!(error, FxError::Http(503));
        assert_eq!(f.transport.calls().len(), 4);
        assert_eq!(*f.sleeps.lock().unwrap(), [0.5, 1.0, 2.0]);

        let f = fixture(None);
        f.transport.on(
            "quotes=EUR",
            Ok(HttpReply {
                status: 404,
                body: String::new(),
                retry_after: None,
            }),
        );
        assert_eq!(
            f.service.conversion(Usd, Eur, None).unwrap_err(),
            FxError::Http(404)
        );
        assert_eq!(f.transport.calls().len(), 1, "a 404 is not retried");

        let f = fixture(None);
        f.transport.on(
            "quotes=EUR",
            Ok(HttpReply {
                status: 429,
                body: String::new(),
                retry_after: Some(2.0),
            }),
        );
        f.transport.ok("quotes=EUR", &current_body());
        assert_eq!(f.service.conversion(Usd, Eur, None).unwrap().rate, 0.5);
        assert_eq!(
            *f.sleeps.lock().unwrap(),
            [2.0],
            "Retry-After wins over the backoff"
        );

        let f = fixture(None);
        f.transport
            .on("quotes=EUR", Err(TransportError::Retryable("reset".into())));
        f.transport.ok("quotes=EUR", &current_body());
        assert_eq!(f.service.conversion(Usd, Eur, None).unwrap().rate, 0.5);
    }

    #[test]
    fn a_historical_rate_falls_back_up_to_a_week_and_is_cached() {
        let f = fixture(None);
        // The 5th is a Tuesday with no row; the 1st has one.
        f.transport.ok(
            "from=2024-02-27&to=2024-03-05",
            &history_body(&[("2024-03-01", "EUR", 0.9), ("2024-03-01", "GBP", 0.8)]),
        );
        let first = f.service.conversion(Usd, Eur, Some(T0)).unwrap();
        assert_eq!(first.rate, 0.9);
        assert_eq!(first.observation_ms, parse_day("2024-03-01").unwrap());
        assert!(!first.is_cached);
        let again = f.service.conversion(Usd, Eur, Some(T0)).unwrap();
        assert!(again.is_cached);
        assert_eq!(f.transport.calls().len(), 1);
        assert_eq!(
            f.service.conversion(Eur, Gbp, Some(DAY)).unwrap().rate,
            0.8 / 0.9
        );
    }

    #[test]
    fn a_date_with_nothing_within_a_week_is_unavailable() {
        let f = fixture(None);
        f.transport.ok(
            "from=2024-02-27",
            &history_body(&[("2024-02-20", "EUR", 0.9)]),
        );
        let error = f.service.conversion(Usd, Eur, Some(T0)).unwrap_err();
        assert_eq!(
            error,
            FxError::RateUnavailable {
                from: Usd,
                to: Eur,
                date_ms: Some(T0)
            }
        );
        assert_eq!(
            error.message(),
            "Exchange rate from USD to EUR is unavailable for 2024-03-05."
        );
    }

    #[test]
    fn many_dates_cost_one_request_and_report_per_date() {
        let f = fixture(None);
        f.transport.ok(
            "from=2024-02-24&to=2024-03-05",
            &history_body(&[
                ("2024-03-01", "EUR", 0.9),
                ("2024-03-04", "EUR", 0.92),
                ("2024-03-05", "EUR", 0.93),
            ]),
        );
        let dates = [
            DAY - 3 * DAY_MS,
            DAY - DAY_MS,
            DAY + 3_600_000,
            DAY + 7_200_000,
        ];
        let results = f.service.conversions(Usd, Eur, &dates);
        assert_eq!(f.transport.calls().len(), 1);
        assert_eq!(results.len(), 4);
        assert_eq!(
            results[&dates[0]].as_ref().unwrap().rate,
            0.9,
            "the 2nd falls back to the 1st"
        );
        assert_eq!(results[&dates[1]].as_ref().unwrap().rate, 0.92);
        assert_eq!(results[&dates[2]].as_ref().unwrap().rate, 0.93);
        assert!(
            !results[&dates[2]].as_ref().unwrap().is_cached,
            "just fetched"
        );
        // Everything is cached now: no new request.
        let again = f.service.conversions(Usd, Eur, &dates);
        assert!(again[&dates[1]].as_ref().unwrap().is_cached);
        assert_eq!(f.transport.calls().len(), 1);
    }

    #[test]
    fn a_failed_range_fetch_still_answers_from_the_cache() {
        let f = fixture(None);
        f.transport
            .ok("from=", &history_body(&[("2024-03-04", "EUR", 0.92)]));
        f.service.conversions(Usd, Eur, &[DAY - DAY_MS]);
        f.transport.replies.lock().unwrap().clear();
        let far = DAY + 30 * DAY_MS;
        let results = f.service.conversions(Usd, Eur, &[DAY - DAY_MS, far]);
        assert!(results[&(DAY - DAY_MS)].is_ok());
        assert!(results[&far].is_err());
    }

    fn bitstamp(rows: &[(i64, f64)]) -> String {
        let items: Vec<String> = rows
            .iter()
            .map(|(seconds, close)| format!(r#"{{"timestamp":"{seconds}","close":"{close}"}}"#))
            .collect();
        format!(
            r#"{{"data":{{"pair":"BTC/USD","ohlc":[{}]}}}}"#,
            items.join(",")
        )
    }

    #[test]
    fn bitcoin_history_closes_convert_btc_on_a_day() {
        let f = fixture(None);
        let day = DAY / 1_000;
        f.transport.ok(
            "bitstamp.net",
            &bitstamp(&[(day - 86_400, 60_000.0), (day, 62_000.0)]),
        );
        let usd = f.service.conversion(Btc, Usd, Some(T0)).unwrap();
        assert_eq!(usd.rate, 62_000.0);
        assert_eq!(usd.observation_ms, DAY);
        let back = f.service.conversion(Usd, Btc, Some(T0)).unwrap();
        assert_eq!(back.rate, 1.0 / 62_000.0);
        assert_eq!(
            f.transport.calls().len(),
            1,
            "the history is fresh for twelve hours"
        );
        // A day after the last close still uses it, up to a week.
        assert_eq!(
            f.service
                .conversion(Btc, Usd, Some(DAY + 3 * DAY_MS))
                .unwrap()
                .rate,
            62_000.0
        );
        let too_far = f.service.conversion(Btc, Usd, Some(DAY + 9 * DAY_MS));
        assert!(too_far.is_err(), "no close within a week");
    }

    #[test]
    fn bitcoin_history_pages_back_when_a_batch_is_full() {
        let f = fixture(None);
        let rows: Vec<(i64, f64)> = (0..BITSTAMP_BATCH as i64)
            .map(|i| (1_000_000 + i * 86_400, 100.0 + i as f64))
            .collect();
        f.transport.on(
            "end=1709649000",
            Ok(HttpReply {
                status: 200,
                body: bitstamp(&rows),
                retry_after: None,
            }),
        );
        f.transport.on(
            "end=999999",
            Ok(HttpReply {
                status: 200,
                body: bitstamp(&[(900_000, 50.0)]),
                retry_after: None,
            }),
        );
        let history = f
            .service
            .refresh_bitcoin(&mut f.service.lock(), false)
            .unwrap();
        assert_eq!(history.len(), BITSTAMP_BATCH + 1);
        assert_eq!(
            history[0],
            DailyClose {
                date_ms: 900_000_000,
                close: 50.0
            }
        );
        assert_eq!(f.transport.calls().len(), 2);
    }

    #[test]
    fn a_failed_bitcoin_refresh_serves_the_old_history() {
        let f = fixture(None);
        f.transport
            .ok("bitstamp.net", &bitstamp(&[(DAY / 1_000, 62_000.0)]));
        f.service.conversion(Btc, Usd, Some(DAY)).unwrap();
        f.clock.fetch_add(13 * 60 * 60 * 1_000, Ordering::SeqCst);
        f.transport.replies.lock().unwrap().clear();
        let rate = f
            .service
            .conversion(Btc, Usd, Some(DAY + 3_600_000))
            .unwrap();
        assert_eq!(rate.rate, 62_000.0, "stale history beats no history");
        let fresh = fixture(None);
        fresh
            .transport
            .on("bitstamp.net", Err(TransportError::Fatal("down".into())));
        assert!(fresh.service.conversion(Btc, Usd, Some(DAY)).is_err());
        fresh.transport.ok("bitstamp.net", &bitstamp(&[]));
        let empty = fixture(None);
        empty.transport.ok("bitstamp.net", &bitstamp(&[]));
        assert_eq!(
            empty.service.conversion(Btc, Usd, Some(DAY)).unwrap_err(),
            FxError::EmptyBitcoinHistory
        );
    }

    #[test]
    fn the_current_bitcoin_price_comes_from_the_spot_source_and_is_briefly_cached() {
        let f = fixture(None);
        assert_eq!(f.service.conversion(Btc, Usd, None).unwrap().rate, 50_000.0);
        *f.spot.lock().unwrap() = Some(60_000.0);
        f.clock.fetch_add(30_000, Ordering::SeqCst);
        assert_eq!(
            f.service.rate(Btc, Usd),
            Some(50_000.0),
            "inside the minute"
        );
        f.clock.fetch_add(40_000, Ordering::SeqCst);
        assert_eq!(f.service.rate(Btc, Usd), Some(60_000.0));
        assert!(
            f.transport.calls().is_empty(),
            "BTC/USD needs no fiat request"
        );

        f.transport.ok("quotes=EUR", &current_body());
        f.clock.fetch_add(120_000, Ordering::SeqCst);
        let euro = f.service.conversion(Btc, Eur, None).unwrap();
        assert_eq!(euro.rate, 30_000.0, "60,000 USD × 0.5 EUR per USD");
    }

    #[test]
    fn caches_survive_a_restart() {
        let directory = std::env::temp_dir().join(format!("degenview-fx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        {
            let f = fixture(Some(directory.clone()));
            f.transport.ok("quotes=EUR", &current_body());
            f.service.conversion(Usd, Eur, None).unwrap();
            f.transport.ok(
                "&from=2024-02-27",
                &history_body(&[("2024-03-05", "EUR", 0.93)]),
            );
            f.service.conversion(Usd, Eur, Some(DAY)).unwrap();
        }
        let again = fixture(Some(directory.clone()));
        assert_eq!(again.service.conversion(Usd, Eur, None).unwrap().rate, 0.5);
        let historical = again.service.conversion(Usd, Eur, Some(DAY)).unwrap();
        assert_eq!(historical.rate, 0.93);
        assert!(historical.is_cached);
        assert!(again.transport.calls().is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn bitstamp_rows_with_bad_numbers_are_dropped() {
        let body = r#"{"data":{"ohlc":[{"timestamp":"86400","close":"10.5"},{"timestamp":"x","close":"1"},{"timestamp":"172800","close":"-3"},{"timestamp":"259200","close":"0"}]}}"#;
        assert_eq!(
            parse_bitstamp(body).unwrap(),
            vec![DailyClose {
                date_ms: 86_400_000,
                close: 10.5
            }]
        );
        assert_eq!(
            parse_bitstamp("nope"),
            Err(FxError::InvalidResponse("Bitstamp".into()))
        );
        let merged = merge_closes(
            &[
                DailyClose {
                    date_ms: 1,
                    close: 1.0,
                },
                DailyClose {
                    date_ms: 2,
                    close: 2.0,
                },
            ],
            &[
                DailyClose {
                    date_ms: 2,
                    close: 9.0,
                },
                DailyClose {
                    date_ms: 0,
                    close: 0.5,
                },
            ],
        );
        assert_eq!(
            merged.iter().map(|c| c.close).collect::<Vec<_>>(),
            [0.5, 1.0, 9.0]
        );
    }
}
