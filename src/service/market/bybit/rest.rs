//! Bybit's candle requests over HTTP: one page, the paging around it.

use super::parse::parse_candles;
use super::{BASE_URL, interval};
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;

pub(super) fn fetch_page(
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
    parse_candles(&get(&url)?, base)
}

/// A request, with Bybit's regional refusal told apart from a network failure.
pub(in crate::service) fn get(url: &str) -> Result<String, ProviderError> {
    match http::get_text(url, &[], http::Retry::Transient) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(403)) => Err(ProviderError::other(
            "Bybit refused the request (HTTP 403): it is not available in every country, and \
             refuses a client that asks too often",
        )),
        Err(error) => Err(error.into()),
    }
}
