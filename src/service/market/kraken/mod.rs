//! Kraken spot candles and its market list. Public endpoints, no key.
//!
//! `OHLC` answers the newest 720 candles of an interval and cannot page back: `since` only moves
//! the end this side of them. Rows are `[open time s, open, high, low, close, vwap, volume,
//! trades]`, the last one still forming. Failures arrive as HTTP 200 with an `error` list.

pub(in crate::service) mod live;
mod markets;
mod parse;
pub(in crate::service) mod quotes;
mod rest;
mod ticker;

use super::candles::plan;
use super::live::CandleProtocol;
use super::search_rank::normalize;
use super::venue::Venue;
use crate::model::market::natives::KRAKEN_BARS;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use markets::clear_catalog;
use markets::{CATALOG, download_pairs, matching_pairs};
#[cfg(test)]
use markets::{Pair, parse_pairs};
use parse::is_rate_limit;
#[cfg(test)]
use parse::parse_ohlc;
use rest::{alias, fetch_ohlc, newest, window};

pub(crate) const BASE_URL: &str = "https://api.kraken.com/0/public";

pub struct Kraken;

pub const KRAKEN: Kraken = Kraken;

impl Venue for Kraken {
    fn clear_catalog(&self) {
        clear_catalog();
    }

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
}

/// Kraken's `interval` (minutes) for a candle length it serves itself.
pub(crate) fn interval(timeframe: Timeframe) -> Option<u32> {
    DataSource::Kraken
        .natives()
        .contains(&timeframe)
        .then(|| (timeframe.nominal_ms() / 60_000) as u32)
}

/// Kraken's name of a market, `BTC/USD`, which both its REST API and its websocket take.
/// `BTC-USD`, a bare `BTC` (USD) and the legacy `XBT/USD` are accepted.
pub(crate) fn pair(symbol: &str) -> String {
    InstrumentId::new(DataSource::Kraken, symbol).api_symbol()
}

impl MarketSource for Kraken {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Kraken, timeframe)?;
        let requested = plan.base_count(limit as usize).min(KRAKEN_BARS);
        let candles = fetch_ohlc(symbol, plan.base(), Some(requested))?;
        Ok(plan.shape(newest(candles, requested), requested, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Kraken, timeframe)?;
        let rows = fetch_ohlc(symbol, plan.base(), None)?;
        Ok(window(plan, rows, start_ms, end_ms, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC/USD`, `btc-usd`, `BTCUSD` and the legacy `XBTUSD` are one query.
        let needle = alias(&normalize(query));
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = CATALOG.get(download_pairs)?;
        let results = matching_pairs(&pairs, &needle);
        Ok(rank_results(results, &needle)
            .into_iter()
            .take(20)
            .collect())
    }
}

#[cfg(test)]
mod tests;
