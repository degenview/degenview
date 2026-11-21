//! Deribit's requests over HTTP.
//!
//! `get_tradingview_chart_data` answers the candles that open in `[start_timestamp, end_timestamp]`
//! (both ends inclusive, in milliseconds), the newest 5,001 of them when the window holds more, and
//! a bar for every bucket (a market nobody traded has flat zero-volume bars), so a window of `n`
//! lengths holds `n` bars until the market began. History is paged by moving the end back.
//!
//! The allowance is credits: measured on 2026-11-21, a burst of about 45 requests is let through,
//! then HTTP 429 (code 10028) until the credits refill, and 30 requests a second for 20 seconds were
//! all let through. A chart pages one request after another, so no schedule is needed.

use super::markets::{not_listed, require_name};
use super::parse::parse_candles;
use super::resolution::resolution;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::market::paging::Paging;
use crate::service::net::http;
use std::time::Duration;

pub(super) const BASE_URL: &str = "https://www.deribit.com/api/v2/public";

/// What one candle request answers, at most (it holds one more).
const PAGE: usize = 5_000;

/// A chart reaches back at most ten pages in one request (50,000 two-hour bars: 500 weeks of them
/// are 42,000); the pages older than that come as the chart scrolls to them.
pub(super) const PAGING: Paging = Paging {
    page: PAGE,
    max_candles: 10 * PAGE,
    pause: Duration::ZERO,
};

/// The answer to a GET of `path` (with its query).
pub(super) fn get(path: &str) -> Result<String, ProviderError> {
    Ok(http::get_text(
        &format!("{BASE_URL}{path}"),
        &[],
        http::Retry::Transient,
    )?)
}

/// The newest `wanted` candles of `base` that open before `after_ms`, oldest first; fewer only when
/// the market began later. `ask` answers a GET of the path it is given.
///
/// The window starts `wanted` lengths before `after_ms`, so a refresh of four candles is a small
/// answer, not 5,001 bars. A 400 is the instrument it names: a market Deribit does not list.
pub(super) fn page_before_with(
    ask: impl FnOnce(&str) -> Result<String, ProviderError>,
    symbol: &str,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let resolution =
        resolution(base).ok_or_else(|| ProviderError::unsupported(DataSource::Deribit, base))?;
    let name = require_name(symbol)?;
    let start = (after_ms - wanted as i64 * base.nominal_ms()).max(0);
    let body = ask(&format!(
        "/get_tradingview_chart_data?instrument_name={name}&resolution={resolution}&start_timestamp={start}&end_timestamp={}",
        after_ms - 1
    ))
    .map_err(|error| match error {
        ProviderError::Network(http::Error::Status(400)) => not_listed(symbol),
        other => other,
    })?;
    let mut candles = parse_candles(&body, base)?;
    if candles.len() > wanted {
        candles.drain(..candles.len() - wanted);
    }
    Ok(candles)
}

/// [`page_before_with`] over HTTP.
pub(super) fn page_before(
    symbol: &str,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    page_before_with(get, symbol, base, after_ms, wanted)
}
