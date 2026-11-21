//! Lighter's requests over HTTP, every one on the one-a-second schedule its allowance asks for.
//!
//! `/candles` answers the newest 500 bars that open before `end_timestamp`, however `start_timestamp`
//! and `count_back` are set (the latter is required all the same), and the end is compared in whole
//! seconds. So history is paged by moving the end back to where the oldest bar opens.

use super::interval;
use super::parse::parse_candles;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::market::paging::Paging;
use crate::service::net::{http, rate_limit};
use std::time::Duration;

pub(super) const BASE_URL: &str = "https://mainnet.zklighter.elliot.ai/api/v1";

/// What one candle request answers.
const PAGE: usize = 500;

/// A chart reaches back at most ten pages (ten seconds) in one request; the pages older than that
/// come as the chart scrolls to them. The schedule of `get` paces the pages, so there is no pause.
pub(super) const PAGING: Paging = Paging {
    page: PAGE,
    max_candles: 10 * PAGE,
    pause: Duration::ZERO,
};

/// The answer to a GET of `path` (with its query), once the schedule has a slot for it.
pub(super) fn get(path: &str) -> Result<String, ProviderError> {
    rate_limit::lighter().wait_for_slot();
    Ok(http::get_text(
        &format!("{BASE_URL}{path}"),
        &[],
        http::Retry::Transient,
    )?)
}

/// The `end_timestamp` that leaves out the bars that open at `after_ms` or later: the API keeps the
/// bars that open before the end's whole second, and a bar opens on a whole second.
fn end_param(after_ms: i64) -> i64 {
    (after_ms + 999).div_euclid(1_000) * 1_000
}

/// The newest `wanted` candles of `base` that open before `after_ms`, oldest first; fewer only when
/// the market has no more. `ask` answers a GET of the path it is given.
pub(super) fn page_before_with(
    ask: impl FnOnce(&str) -> Result<String, ProviderError>,
    market_id: u32,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    let resolution =
        interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Lighter, base))?;
    let body = ask(&format!(
        "/candles?market_id={market_id}&resolution={resolution}&start_timestamp=0&end_timestamp={}&count_back={PAGE}",
        end_param(after_ms)
    ))?;
    let mut candles = parse_candles(&body, base)?;
    if candles.len() > wanted {
        candles.drain(..candles.len() - wanted);
    }
    Ok(candles)
}

/// [`page_before_with`] over HTTP.
pub(super) fn page_before(
    market_id: u32,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    page_before_with(get, market_id, base, after_ms, wanted)
}
