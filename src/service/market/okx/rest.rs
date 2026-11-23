//! OKX's candle requests over HTTP: one page, the paging around it.

use super::parse::parse_candles;
use super::{BASE_URL, bar};
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;

pub(super) fn fetch_page(
    endpoint: &str,
    id: &str,
    base: Timeframe,
    limit: usize,
    after: Option<i64>,
) -> Result<Vec<Kline>, ProviderError> {
    let bar = bar(base).ok_or_else(|| ProviderError::unsupported(DataSource::Okx, base))?;
    let mut url = format!("{BASE_URL}/market/{endpoint}?instId={id}&bar={bar}&limit={limit}");
    if let Some(after) = after {
        url.push_str(&format!("&after={after}"));
    }
    let body = http::get_text(&url, &[], http::Retry::Transient)?;
    parse_candles(&body, base)
}
