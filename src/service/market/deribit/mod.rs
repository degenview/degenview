//! Deribit's perpetuals: the inverse `BTC-PERPETUAL` and `ETH-PERPETUAL` (quoted in dollars,
//! settled in the coin) and the linear `<ASSET>_USDC-PERPETUAL`, which are coins and also stocks,
//! ETFs, indices and commodities (`SPY_USDC-PERPETUAL`, `GOLD_USDC-PERPETUAL`). Public API, no key.
//!
//! A market is named, asked for and subscribed to by one name, in upper case. A candle request
//! answers the bars that open inside a window, so history is paged back by moving the window's
//! end. Deribit's own days, weeks and months open at 08:00 UTC or not at all, so everything from
//! three hours up folds from its two-hour bars (`rest.rs`, `resolution.rs`).

mod catalog;
pub(in crate::service) mod live;
mod markets;
mod parse;
pub(in crate::service) mod quotes;
mod resolution;
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
use crate::service::watchlist_quotes::provider::rejected;
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use catalog::{CATALOG, download};
use markets::matching;
use rest::PAGING;

pub struct Deribit;

pub const DERIBIT: Deribit = Deribit;

impl Venue for Deribit {
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
        ticker::fetch_quotes(requests)
    }

    /// An HTTP 400 on a price call is a currency Deribit does not have, so a market named after it.
    fn classify_status(&self, code: u16) -> Option<QuoteFailure> {
        (code == 400).then(|| rejected(400))
    }

    fn clear_catalog(&self) {
        catalog::clear();
    }
}

impl MarketSource for Deribit {
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
        let plan = plan(DataSource::Deribit, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            rest::page_before(symbol, plan.base(), after, wanted)
        })?;
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let names = CATALOG.get(download)?;
        let results = matching(&names, &needle);
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}
