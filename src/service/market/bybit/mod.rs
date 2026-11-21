//! Bybit spot candles and its market list. Public v5 endpoints, no key.
//!
//! `/market/kline` answers up to 1,000 rows newest first, `[start ms, open, high, low, close,
//! volume, turnover]`, and pages back with an `end` cursor. Failures arrive as HTTP 200 with a
//! `retCode` other than 0; some regions are refused with HTTP 403.

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
use crate::service::net::ws::Keepalive;
use crate::service::watchlist_quotes::provider::refused_by_region;
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
pub(in crate::service) use markets::clear_catalog;
use markets::{CATALOG, download_pairs, matching_pairs};
#[cfg(test)]
use markets::{Pair, parse_pairs};
use parse::is_rate_limit;
#[cfg(test)]
use parse::{parse_candles, parse_row};
use rest::fetch_page;
pub(in crate::service) use rest::get;
use std::time::Duration;

pub(crate) const BASE_URL: &str = "https://api.bybit.com/v5/market";

pub struct Bybit;

pub const BYBIT: Bybit = Bybit;

impl Venue for Bybit {
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
            403 => Some(refused_by_region(DataSource::Bybit)),
            _ => None,
        }
    }
}

/// Bybit's heartbeat on every public socket: `{"op":"ping"}`, answered by a reply that starts
/// `{"success":true,"ret_msg":"pong"`.
pub const KEEPALIVE: Keepalive = Keepalive::Text {
    ping: r#"{"op":"ping"}"#,
    pong: r#"{"success":true,"ret_msg":"pong""#,
};

/// 1,000 rows a request and 60 pages (a 22-minute chart of 500 candles is 12). Bybit allows 600
/// requests per 5 seconds per IP, far more than a chart asks for, so the pages are not spaced.
const PAGING: Paging = Paging {
    page: 1_000,
    max_candles: 60_000,
    pause: Duration::ZERO,
};

/// Bybit's `interval` for a candle length it serves itself: minutes (`"1"`, `"60"`, `"720"`), or
/// `"D"`, `"W"` and `"M"`.
pub(crate) fn interval(timeframe: Timeframe) -> Option<String> {
    if !DataSource::Bybit.natives().contains(&timeframe) {
        return None;
    }
    Some(if timeframe == Timeframe::DAY_1 {
        "D".to_owned()
    } else if timeframe == Timeframe::WEEK_1 {
        "W".to_owned()
    } else if timeframe == Timeframe::MONTH_1 {
        "M".to_owned()
    } else {
        (timeframe.nominal_ms() / 60_000).to_string()
    })
}

/// Bybit's name of a market, `BTCUSDT`, which its REST API and its websocket take. `BTC/USDT`,
/// `btc-usdt` and a bare `BTC` (USDT) are accepted.
pub(crate) fn symbol_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Bybit, symbol).api_symbol()
}

impl MarketSource for Bybit {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Bybit, timeframe)?;
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
        let plan = plan(DataSource::Bybit, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let id = symbol_id(symbol);
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            // `end` includes the candle that opens at it; `after` is exclusive.
            fetch_page(&id, plan.base(), wanted, Some(after - 1))
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
