//! Gate.io spot candles and its market list. Public v4 endpoints, no key.
//!
//! `/spot/candlesticks` answers up to 1,000 rows oldest first, `[start s, quote volume, close,
//! high, low, open, base volume, window closed]`, all strings, and pages back with a `to` cursor
//! (inclusive, in seconds). Failures are HTTP 4xx with a JSON label; some regions get 403.

pub(in crate::service) mod live;
mod markets;
mod parse;
pub(in crate::service) mod quotes;
mod rest;
mod ticker;

use super::candles::plan;
use super::live::CandleProtocol;
use super::paging::Paging;
use super::search_rank::normalize;
use super::venue::Venue;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::watchlist_quotes::provider::refused_by_region;
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
pub(in crate::service) use markets::clear_catalog;
use markets::{CATALOG, download_pairs, matching_pairs};
#[cfg(test)]
use markets::{Pair, parse_pairs};
#[cfg(test)]
use parse::{parse_candles, parse_row};
#[cfg(test)]
use rest::reach_left;
use rest::{collect, fetch_page};
use std::time::Duration;

pub(crate) const BASE_URL: &str = "https://api.gateio.ws/api/v4/spot";

pub struct Gate;

pub const GATE: Gate = Gate;

impl Venue for Gate {
    fn candles(&self) -> &dyn MarketSource {
        self
    }

    fn live(&self) -> Option<&dyn CandleProtocol> {
        Some(self)
    }

    fn quotes(&self) -> Option<&dyn QuoteProtocol> {
        Some(self)
    }

    fn rest_quotes(&self, requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
        ticker::fetch_quotes(requests).map(Outcome::from_quotes)
    }

    fn classify_status(&self, code: u16) -> Option<QuoteFailure> {
        match code {
            403 => Some(refused_by_region(DataSource::Gate)),
            _ => None,
        }
    }
}

/// 1,000 rows a request. Gate.io refuses (HTTP 400, "Candlestick too long ago") a request that
/// reaches 10,000 points back from now in the interval asked for, and counts the rows the
/// request holds: the newest 8,900 keep clear of it, which is a week of 1-minute candles, a
/// month of 5-minute ones, a year of hours and 24 years of days. It allows 200 requests per 10
/// seconds per IP, far more than a chart asks for, so the pages are not spaced.
const PAGING: Paging = Paging {
    page: 1_000,
    max_candles: 8_900,
    pause: Duration::ZERO,
};

/// Gate.io's `interval` for a candle length it serves itself: `1m`, `4h`, `1d`, `7d` (weeks,
/// from Monday) and `30d` (the calendar month).
pub(crate) fn interval(timeframe: Timeframe) -> Option<&'static str> {
    if !DataSource::Gate.natives().contains(&timeframe) {
        return None;
    }
    [
        (Timeframe::MINUTE_1, "1m"),
        (Timeframe::MINUTE_5, "5m"),
        (Timeframe::MINUTE_15, "15m"),
        (Timeframe::MINUTE_30, "30m"),
        (Timeframe::HOUR_1, "1h"),
        (Timeframe::HOUR_4, "4h"),
        (Timeframe::HOUR_8, "8h"),
        (Timeframe::DAY_1, "1d"),
        (Timeframe::WEEK_1, "7d"),
        (Timeframe::MONTH_1, "30d"),
    ]
    .into_iter()
    .find_map(|(native, token)| (native == timeframe).then_some(token))
}

/// Gate.io's name of a market, `BTC_USDT`. `BTC/USDT`, `btc-usdt` and a bare `BTC` (USDT) are
/// accepted.
pub(crate) fn pair_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Gate, symbol).api_symbol()
}

impl MarketSource for Gate {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Gate, timeframe)?;
        let requested = PAGING.requested(plan, limit as usize);
        if requested > PAGING.page {
            // More base candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let rows = fetch_page(&pair_id(symbol), plan.base(), requested, None)?;
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
        let plan = plan(DataSource::Gate, timeframe)?;
        let id = pair_id(symbol);
        collect(
            plan,
            crate::clock::now_ms(),
            start_ms,
            end_ms,
            max,
            |to_s, limit| fetch_page(&id, plan.base(), limit, Some(to_s)),
        )
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC_USDT`, `btc/usdt` and `BTCUSDT` are one query.
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
