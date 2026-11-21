//! MEXC spot candles and its market list. Public v3 endpoints, no key.
//!
//! `/klines` answers up to 500 rows oldest first, `[open ms, open, high, low, close, volume,
//! close ms, quote volume]`, and takes either a `limit` (the newest rows) or a `startTime` and
//! `endTime` window; an `endTime` alone is ignored. Failures are HTTP 400 with a JSON code, and
//! some regions get 403.

pub(in crate::service) mod live;
mod markets;
mod parse;
mod rest;
mod ticker;

use super::candles::plan;
use super::live::CandleProtocol;
use super::paging::Paging;
use super::search_rank::normalize;
use super::venue::Venue;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::watchlist_quotes::provider::refused_by_region;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use markets::clear_catalog;
use markets::{CATALOG, download_pairs, matching_pairs};
#[cfg(test)]
use markets::{Pair, parse_pairs};
#[cfg(test)]
use parse::{parse_candles, parse_row};
use rest::{fetch_page, window_before};
use std::time::Duration;

pub(crate) const BASE_URL: &str = "https://api.mexc.com/api/v3";

pub struct Mexc;

pub const MEXC: Mexc = Mexc;

impl Venue for Mexc {
    fn clear_catalog(&self) {
        clear_catalog();
    }

    fn candles(&self) -> &dyn MarketSource {
        self
    }

    fn live(&self) -> Option<&dyn CandleProtocol> {
        Some(self)
    }

    fn rest_quotes(&self, requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
        ticker::fetch_quotes(requests).map(Outcome::from_quotes)
    }

    fn classify_status(&self, code: u16) -> Option<QuoteFailure> {
        match code {
            403 => Some(refused_by_region(DataSource::Mexc)),
            _ => None,
        }
    }
}

/// 500 rows a request and 20,000 base candles (a 22-minute chart of 500 candles is 22 pages, and
/// that is two years of hours). Its one-minute candles reach back about a month. Nothing limits
/// the pages that is worth spacing them for.
const PAGING: Paging = Paging {
    page: 500,
    max_candles: 20_000,
    pause: Duration::ZERO,
};

/// The oldest start a window may have: a `startTime` of 0 answers nothing.
const EARLIEST_MS: i64 = 1_500_000_000_000;

/// MEXC's `interval` for a candle length it serves itself: `1m`, `60m`, `4h`, `1d`, `1W`, `1M`.
pub(crate) fn interval(timeframe: Timeframe) -> Option<&'static str> {
    table(timeframe).map(|(rest, _)| rest)
}

/// The name of that interval in its websocket topics: `Min1`, `Min60`, `Hour4`, `Day1`, `Week1`,
/// `Month1`.
pub(crate) fn socket_interval(timeframe: Timeframe) -> Option<&'static str> {
    table(timeframe).map(|(_, socket)| socket)
}

fn table(timeframe: Timeframe) -> Option<(&'static str, &'static str)> {
    if !DataSource::Mexc.natives().contains(&timeframe) {
        return None;
    }
    [
        (Timeframe::MINUTE_1, ("1m", "Min1")),
        (Timeframe::MINUTE_5, ("5m", "Min5")),
        (Timeframe::MINUTE_15, ("15m", "Min15")),
        (Timeframe::MINUTE_30, ("30m", "Min30")),
        (Timeframe::HOUR_1, ("60m", "Min60")),
        (Timeframe::HOUR_4, ("4h", "Hour4")),
        (Timeframe::HOUR_8, ("8h", "Hour8")),
        (Timeframe::DAY_1, ("1d", "Day1")),
        (Timeframe::WEEK_1, ("1W", "Week1")),
        (Timeframe::MONTH_1, ("1M", "Month1")),
    ]
    .into_iter()
    .find_map(|(native, names)| (native == timeframe).then_some(names))
}

/// MEXC's name of a market, `BTCUSDT`. `BTC/USDT`, `btc-usdt` and a bare `BTC` (USDT) are
/// accepted.
pub(crate) fn symbol_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Mexc, symbol).api_symbol()
}

impl MarketSource for Mexc {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Mexc, timeframe)?;
        let requested = PAGING.requested(plan, limit as usize);
        if requested > PAGING.page {
            // More base candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let rows = fetch_page(&symbol_id(symbol), plan.base(), requested, None)?;
        Ok(plan.shape(rows, requested, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Mexc, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let id = symbol_id(symbol);
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            let window = window_before(after, wanted, plan.base());
            if window.0 >= window.1 {
                return Ok(Vec::new());
            }
            fetch_page(&id, plan.base(), wanted, Some(window))
        })?;
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC-USDT`, `btc/usdt` and `BTCUSDT` are one query.
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = CATALOG.get(download_pairs)?;
        let results = matching_pairs(&pairs, &needle);
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}

#[cfg(test)]
mod tests;
