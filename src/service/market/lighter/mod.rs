//! Lighter's perpetuals: coins, and the stocks, ETFs, indices, currencies and commodities it lists
//! beside them (`BTC`, `AAPL`, `US500`, `EURUSD`, `XAU`). Public API, no key.
//!
//! A market is named by its symbol and asked for by a number, which the cached market list
//! (`catalog.rs`) resolves. A candle request always answers the newest 500 bars before its
//! `end_timestamp`, so history is paged back by moving that end. The allowance is about one request a
//! second per IP (`net::rate_limit::lighter`), which every REST call here keeps to.

mod catalog;
pub(in crate::service) mod live;
mod markets;
mod parse;
pub(in crate::service) mod quotes;
mod rest;
mod socket;
#[cfg(test)]
mod tests;
mod ticker;

use super::candles::plan;
use super::live::CandleProtocol;
use super::search_rank::normalize;
use super::venue::Venue;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use catalog::{CATALOG, download, market_id};
use markets::matching;
use rest::PAGING;

pub struct Lighter;

pub const LIGHTER: Lighter = Lighter;

impl Venue for Lighter {
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

    fn clear_catalog(&self) {
        catalog::clear();
    }
}

/// Lighter's `resolution` for a candle length it serves itself: `1m`, `4h`, `1d`.
fn interval(timeframe: Timeframe) -> Option<String> {
    DataSource::Lighter
        .natives()
        .contains(&timeframe)
        .then(|| timeframe.interval())
}

impl MarketSource for Lighter {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let end_ms = crate::clock::now_ms() + 1;
        self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize)
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Lighter, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let id = market_id(symbol)?;
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            rest::page_before(id, plan.base(), after, wanted)
        })?;
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let markets = CATALOG.get(download)?;
        let results = matching(&markets, &needle);
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}
