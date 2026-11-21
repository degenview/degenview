//! Bitfinex's requests over HTTP. Public endpoints, no key.
//!
//! `/candles/trade:<period>:t<PAIR>/hist` answers up to 10,000 candles (10,001 is HTTP 500,
//! `["error",10020,"limit: invalid"]`), newest first, that open at or before `end`. A fetch pages
//! back by moving `end`. Candle requests go through the one-at-a-time schedule of
//! `net::rate_limit::bitfinex`, which keeps them inside the allowance: past it (measured on
//! 2026-11-21: the 34th request of a run at one a second) Bitfinex answers HTTP 429,
//! `["error",11010,"ratelimit: error"]`, with a `Retry-After` of about five minutes, and answers
//! nothing until then. An answer of `[]` is not a limit that was seen (it is what a name Bitfinex
//! does not know, a range before a market's first candle and a market with no candle of the length
//! get), so it is told from the end of a market's history, or from a name Bitfinex does not list,
//! by asking for the market's newest candle ([`page_before_with`]); a limit is the failure of a
//! request, whose HTTP 429 or `11010` answer comes back as it is.

use super::markets::is_listed;
use super::parse::parse_candles;
use super::{BASE_URL, period, symbol};
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::market::paging::Paging;
use crate::service::net::{http, rate_limit};
use std::time::Duration;

/// The most candles one request holds.
const PAGE: usize = 10_000;

/// A chart reaches back at most 30,000 base candles in one request (three requests); older ones come
/// as the chart scrolls to them. The schedule of [`get`] paces the pages, so there is no pause.
pub(super) const PAGING: Paging = Paging {
    page: PAGE,
    max_candles: 3 * PAGE,
    pause: Duration::ZERO,
};

/// The answer to a GET of `path` (with its query) with no wait, for a call that has an allowance of
/// its own: the market list. A refusal of a country is told from a network failure.
pub(super) fn get_now(path: &str) -> Result<String, ProviderError> {
    match http::get_text(&format!("{BASE_URL}{path}"), &[], http::Retry::Never) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(403)) => {
            Err(ProviderError::refused_by_region(DataSource::Bitfinex))
        }
        Err(error) => Err(error.into()),
    }
}

/// [`get_now`] for a candle request, once the candle schedule has a slot for it. A 429 is not
/// retried: the wait is minutes, and asking again within it only fails again.
pub(super) fn get(path: &str) -> Result<String, ProviderError> {
    rate_limit::bitfinex().wait_for_slot();
    get_now(path)
}

fn candles_path(
    market: &str,
    base: Timeframe,
    end_ms: i64,
    count: usize,
) -> Result<String, ProviderError> {
    let period =
        period(base).ok_or_else(|| ProviderError::unsupported(DataSource::Bitfinex, base))?;
    let symbol = symbol(market)
        .ok_or_else(|| ProviderError::unknown_market(DataSource::Bitfinex, market, ""))?;
    Ok(format!(
        "/candles/trade:{period}:{symbol}/hist?limit={}&end={end_ms}",
        count.clamp(1, PAGE)
    ))
}

/// The newest `wanted` candles of `base` that open before `after_ms`, oldest first; fewer only when
/// the market began later. `ask` answers a GET of the path it is given, `listed` whether Bitfinex
/// lists the market (asked only when it matters) and `now_ms` is the time.
///
/// An empty answer is the market's first candle passed, a market Bitfinex does not list, or a
/// market with no candle of this length, and these look alike. So after one the market's newest
/// candle is asked for, with an `end` that no earlier request used so that nothing in between can
/// answer it from memory: if that comes back, the empty page was the start of the history; if it
/// is empty too, the list tells whether the market is unknown to Bitfinex or only has no candle at
/// this length (there is nothing before `after_ms`, which ends the history). A client asking too
/// fast is a failed request, not an empty answer: it ends this with the limit's own error.
pub(super) fn page_before_with(
    mut ask: impl FnMut(&str) -> Result<String, ProviderError>,
    listed: impl FnOnce() -> bool,
    market: &str,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
    now_ms: i64,
) -> Result<Vec<Kline>, ProviderError> {
    let page = parse_candles(
        &ask(&candles_path(market, base, after_ms - 1, wanted)?)?,
        base,
    )?;
    if !page.is_empty() {
        return Ok(page);
    }
    let newest = parse_candles(&ask(&candles_path(market, base, now_ms, 1)?)?, base)?;
    if !newest.is_empty() {
        return Ok(page);
    }
    if listed() {
        Ok(page)
    } else {
        Err(ProviderError::unknown_market(
            DataSource::Bitfinex,
            market,
            "",
        ))
    }
}

/// [`page_before_with`] over HTTP.
pub(super) fn page_before(
    market: &str,
    base: Timeframe,
    after_ms: i64,
    wanted: usize,
) -> Result<Vec<Kline>, ProviderError> {
    page_before_with(
        get,
        || is_listed(market),
        market,
        base,
        after_ms,
        wanted,
        crate::clock::now_ms(),
    )
}
