//! Hyperliquid's requests over HTTP: every public query is a POST to `/info`, and a candle
//! request answers a window of time, not a count.

use super::interval;
use super::parse::parse_candles;
use crate::lock::lock;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

pub(super) const INFO_URL: &str = "https://api.hyperliquid.xyz/info";

/// Room a window leaves for bars the market did not trade in, which have no row.
const GAP_ROOM_MS: i64 = 6 * 3_600_000;

/// A page of this many candles or fewer is a refresh of the newest bars (`cache::TAIL_LIMIT`).
const REFRESH_BARS: usize = crate::service::cache::TAIL_LIMIT as usize;

/// A market whose whole history holds fewer candles than this at an interval is thin: its history
/// weighs no more than a window, and a window of it may well hold nothing.
const THIN_BARS: usize = 100;

/// How many thin markets are remembered.
const MAX_THIN_MARKETS: usize = 512;

/// The answer to one `/info` query.
pub(super) fn info(query: &Value) -> Result<String, ProviderError> {
    Ok(http::post_json(
        INFO_URL,
        &query.to_string(),
        http::Retry::Transient,
    )?)
}

/// The candles of `id` that open in `[start_ms, end_ms]`, oldest first. Hyperliquid keeps the
/// newest ~5,000 candles of an interval, so a window that reaches further back answers from the
/// oldest it has.
fn window(
    id: &str,
    base: Timeframe,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<Kline>, ProviderError> {
    let interval =
        interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Hyperliquid, base))?;
    let query = json!({
        "type": "candleSnapshot",
        "req": {"coin": id, "interval": interval, "startTime": start_ms, "endTime": end_ms},
    });
    candles_of(
        http::post_json_reading_refusals(INFO_URL, &query.to_string(), http::Retry::Never),
        id,
        base,
    )
}

/// The candles of an answer to a window request. Hyperliquid answers `null` with an HTTP 500 for a
/// name it does not list; any other refusal is an outage or a limit, and the chart asks again at its
/// next refresh, so the request is not retried.
pub(super) fn candles_of(
    answer: Result<String, http::Error>,
    id: &str,
    base: Timeframe,
) -> Result<Vec<Kline>, ProviderError> {
    match answer {
        Ok(body) => parse_candles(&body, base),
        Err(http::Error::Refused { status: 500, body }) if body.trim() == "null" => {
            Err(ProviderError::unknown_market(
                DataSource::Hyperliquid,
                id,
                "names are case-sensitive, like BTC or kPEPE",
            ))
        }
        Err(error) => Err(error.into()),
    }
}

/// The span of time that holds `wanted` candles of `base` with room for gaps.
fn span_ms(base: Timeframe, wanted: usize) -> i64 {
    base.nominal_ms()
        .saturating_mul(wanted as i64)
        .saturating_mul(2)
        .saturating_add(GAP_ROOM_MS)
}

/// The newest `wanted` candles that open before `after_ms` and not before `start_ms`, oldest
/// first; fewer only when the market has no more. `ask(start_ms, end_ms)` answers the candles of a
/// window.
///
/// A window of the span `wanted` candles take is asked first, which is all a refresh needs. When it
/// holds fewer and the page is more than a refresh (a chart's first fetch, an older page: the market
/// missed bars or began later) everything Hyperliquid keeps from `start_ms` is asked. A refresh of
/// a few bars never is: a market quiet for hours has no bar to find, and asking for its history
/// every few seconds would spend the allowance. A market whose history held fewer than
/// [`THIN_BARS`] candles (`thin`) is asked for in one request, window or not.
pub(super) fn page_before_with(
    mut ask: impl FnMut(i64, i64) -> Result<Vec<Kline>, ProviderError>,
    thin: bool,
    base: Timeframe,
    start_ms: i64,
    after_ms: i64,
    wanted: usize,
) -> Result<Page, ProviderError> {
    let end_ms = after_ms - 1;
    let window_start = if thin {
        start_ms
    } else {
        (end_ms - span_ms(base, wanted)).max(start_ms)
    };
    let mut candles = ask(window_start, end_ms)?;
    let mut whole = window_start <= start_ms;
    if candles.len() < wanted && !whole && wanted > REFRESH_BARS {
        candles = ask(start_ms, end_ms)?;
        whole = true;
    }
    let thin = (start_ms <= 0 && whole).then_some(candles.len() < THIN_BARS);
    if candles.len() > wanted {
        candles.drain(..candles.len() - wanted);
    }
    Ok(Page { candles, thin })
}

/// What [`page_before_with`] found.
pub(super) struct Page {
    pub candles: Vec<Kline>,
    /// Whether the market's whole history holds fewer than [`THIN_BARS`] candles, when it was asked
    /// for whole.
    pub thin: Option<bool>,
}

/// The markets (`id/interval`) whose whole history is thin, so that [`page_before`] asks for it in
/// one request instead of a window and then everything.
fn thin_markets() -> &'static Mutex<HashSet<String>> {
    static THIN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    THIN.get_or_init(Mutex::default)
}

/// [`page_before_with`] over HTTP.
pub(super) fn page_before(
    id: &str,
    base: Timeframe,
    start_ms: i64,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let key = format!("{id}/{}", base.interval());
    let thin = lock(thin_markets()).contains(&key);
    let page = page_before_with(
        |start_ms, end_ms| window(id, base, start_ms, end_ms),
        thin,
        base,
        start_ms,
        after_ms,
        wanted,
    )?;
    let mut markets = lock(thin_markets());
    match page.thin {
        Some(true) if markets.len() < MAX_THIN_MARKETS || markets.contains(&key) => {
            markets.insert(key);
        }
        Some(false) => {
            markets.remove(&key);
        }
        _ => {}
    }
    Ok(page.candles)
}
