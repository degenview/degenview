//! MEXC's candle requests over HTTP: one page, the paging around it.

use super::parse::parse_candles;
use super::{BASE_URL, EARLIEST_MS, interval};
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;

/// The window `[start, end]` in milliseconds that holds `wanted` candles of `base` just before
/// `after_ms`: a window, because the end alone is ignored. A month is taken at its shortest, so
/// the window never holds more than the 500 a request returns.
pub(super) fn window_before(after_ms: i64, wanted: usize, base: Timeframe) -> (i64, i64) {
    let step = if base == Timeframe::MONTH_1 {
        28 * 86_400_000
    } else {
        base.nominal_ms()
    };
    let end = after_ms - 1;
    let start = (after_ms - wanted as i64 * step).max(EARLIEST_MS);
    (start, end)
}

pub(super) fn fetch_page(
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
    parse_candles(&get(&url)?, base)
}

/// A request, with MEXC's refusals told apart from a network failure: a 400 names a market it
/// does not list, a 403 a country it does not serve.
pub(in crate::service) fn get(url: &str) -> Result<String, ProviderError> {
    match http::get_text(url, &[], http::Retry::Transient) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(400)) => Err(ProviderError::other(
            "MEXC refused the request (HTTP 400): it does not list this market",
        )),
        Err(http::Error::Status(403)) => Err(ProviderError::other(
            "MEXC refused the request (HTTP 403): it is not available in every country",
        )),
        Err(error) => Err(error.into()),
    }
}
