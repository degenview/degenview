//! HTX's requests over HTTP. Public endpoints, no key.
//!
//! `/market/history/kline` answers the newest `size` candles of a period, 1 to 2,000, and ignores
//! `from` and `to`: REST reaches back no further than that, and the rest of the history is asked
//! over the socket (`history.rs`). Failures arrive as HTTP 200 with `status: error`.

use super::parse::parse_klines;
use super::period;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;

pub(super) const BASE_URL: &str = "https://api.huobi.pro";

/// The most candles one request holds, and so all REST reaches.
pub(super) const REST_BARS: usize = 2_000;

/// The answer to a GET of `path` (with its query), with the refusal of a country told apart from a
/// network failure.
pub(super) fn get(path: &str) -> Result<String, ProviderError> {
    match http::get_text(&format!("{BASE_URL}{path}"), &[], http::Retry::Transient) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(403)) => Err(ProviderError::refused_by_region(DataSource::Htx)),
        Err(error) => Err(error.into()),
    }
}

/// The newest `count` candles of `base` (at most [`REST_BARS`]), oldest first, the forming one last.
pub(super) fn recent(id: &str, base: Timeframe, count: usize) -> Result<Vec<Kline>, ProviderError> {
    let period = period(base).ok_or_else(|| ProviderError::unsupported(DataSource::Htx, base))?;
    let size = count.clamp(1, REST_BARS);
    parse_klines(
        &get(&format!(
            "/market/history/kline?symbol={id}&period={period}&size={size}"
        ))?,
        base,
    )
}
