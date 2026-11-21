//! Bitstamp's requests over HTTP. Public endpoints, no key.
//!
//! `/ohlc/<market>/?step=&limit=&end=` answers up to 1,000 bars of `step` seconds, oldest first,
//! that open at or before `end` (seconds). `limit` is required, 1 to 1,000. A fetch pages back by
//! moving `end`. Measured on 2026-11-21: `start` is no lower bound once `end` is given (it only
//! applies without one, and then counts forward), so it is not sent; a minute nobody traded in has a
//! flat bar of no volume, so a short page is the start of the market's history and the page before
//! it is empty; an unknown market is HTTP 404 with no body, a bad `step`, `limit` or `end` HTTP 400;
//! and 160 requests from 40 threads in two seconds were all answered, against a documented 400 a
//! second, so candle requests have no schedule.

use super::parse::parse_candles;
use super::step;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::market::paging::Paging;
use crate::service::net::http;
use std::time::Duration;

pub(super) const BASE_URL: &str = "https://www.bitstamp.net/api/v2";

/// The most bars one request holds.
const PAGE: usize = 1_000;

/// The latest `end` Bitstamp takes (the year 9999, in seconds).
const LAST_END_S: i64 = 253_402_297_199;

/// A chart reaches back at most 30,000 base candles in one request (thirty pages); older ones come
/// as the chart scrolls to them.
pub(super) const PAGING: Paging = Paging {
    page: PAGE,
    max_candles: 30 * PAGE,
    pause: Duration::ZERO,
};

/// The answer to a GET of `path` (with its query), with the refusal of a country told apart from a
/// network failure.
pub(super) fn get(path: &str) -> Result<String, ProviderError> {
    match http::get_text(&format!("{BASE_URL}{path}"), &[], http::Retry::Transient) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(403)) => {
            Err(ProviderError::refused_by_region(DataSource::Bitstamp))
        }
        Err(error) => Err(error.into()),
    }
}

/// The path that asks for the `wanted` newest bars of `step` seconds that open before `after_ms`.
pub(super) fn candles_path(market: &str, step: i64, after_ms: i64, wanted: usize) -> String {
    // The last bar of the page opens at or before `end`, and a bar opens on a whole second.
    let end_s = (after_ms - 1).div_euclid(1_000).clamp(0, LAST_END_S);
    format!(
        "/ohlc/{market}/?step={step}&limit={}&end={end_s}",
        wanted.clamp(1, PAGE)
    )
}

/// The newest `wanted` candles of `base` that open before `after_ms`, oldest first; fewer only when
/// the market began later. `ask` answers a GET of the path it is given. A market Bitstamp does not
/// list is told so.
pub(super) fn page_before_with(
    ask: impl FnOnce(&str) -> Result<String, ProviderError>,
    market: &str,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let step = step(base).ok_or_else(|| ProviderError::unsupported(DataSource::Bitstamp, base))?;
    match ask(&candles_path(market, step, after_ms, wanted)) {
        Err(ProviderError::Network(http::Error::Status(404))) => Err(
            ProviderError::unknown_market(DataSource::Bitstamp, market.to_uppercase(), ""),
        ),
        answer => parse_candles(&answer?, base),
    }
}

/// [`page_before_with`] over HTTP.
pub(super) fn page_before(
    market: &str,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    page_before_with(get, market, base, after_ms, wanted)
}
