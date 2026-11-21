//! Gate.io's candle requests over HTTP: one page, the paging around it.

use super::parse::parse_candles;
use super::{BASE_URL, PAGING, interval};
use crate::model::market::bucket::FoldPlan;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::net::http;

/// How many more candles of `base` a request ending just before `after_ms` may hold and stay
/// within what Gate.io reaches back to; none once the request would pass it.
pub(super) fn reach_left(now_ms: i64, after_ms: i64, base: Timeframe) -> usize {
    let step = base.nominal_ms().max(1);
    let points_ago = ((now_ms - after_ms).max(0) / step) as usize + 1;
    PAGING.max_candles.saturating_sub(points_ago)
}

/// The newest `max` candles of `plan`'s target that open in `[start_ms, end_ms)`, paged back from
/// `end_ms`. `fetch(to, limit)` answers up to `limit` base candles that open at `to` (seconds) or
/// before, oldest first.
pub(super) fn collect(
    plan: FoldPlan,
    now_ms: i64,
    start_ms: i64,
    end_ms: i64,
    max: usize,
    mut fetch: impl FnMut(i64, usize) -> Result<Vec<Kline>, ProviderError>,
) -> Result<Vec<Kline>, ProviderError> {
    // What cannot be reached is not asked for: an answer that fills this is one the reach cut
    // short, so its oldest fold is dropped as a fragment; a shorter one is the market's start.
    let requested = PAGING
        .requested(plan, max)
        .min(reach_left(now_ms, end_ms, plan.base()));
    if requested == 0 {
        return Ok(Vec::new());
    }
    let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
        // A gap in the candles moves `after` back past what was counted: never ask Gate.io for
        // more than it reaches (HTTP 400), the end of history is an empty answer.
        let wanted = wanted.min(reach_left(now_ms, after, plan.base()));
        if wanted == 0 {
            return Ok(Vec::new());
        }
        // `to` includes the candle that opens at it, in seconds; `after` is exclusive.
        fetch((after - 1) / 1_000, wanted)
    })?;
    Ok(plan.shape(candles, requested, max))
}

pub(super) fn fetch_page(
    id: &str,
    base: Timeframe,
    limit: usize,
    to_s: Option<i64>,
) -> Result<Vec<Kline>, ProviderError> {
    let interval =
        interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Gate, base))?;
    let mut url =
        format!("{BASE_URL}/candlesticks?currency_pair={id}&interval={interval}&limit={limit}");
    if let Some(to_s) = to_s {
        url.push_str(&format!("&to={to_s}"));
    }
    parse_candles(&get(&url)?, base)
}

/// A request, with Gate.io's refusals told apart from a network failure: a 400 names a market it
/// does not list, a 403 a country it does not serve (or a client that asks too often).
pub(in crate::service) fn get(url: &str) -> Result<String, ProviderError> {
    match http::get_text(url, &[], http::Retry::Transient) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(400)) => Err(ProviderError::other(
            "Gate.io refused the request (HTTP 400): it does not list this market",
        )),
        Err(http::Error::Status(403)) => Err(ProviderError::refused_by_region(DataSource::Gate)),
        Err(error) => Err(error.into()),
    }
}
