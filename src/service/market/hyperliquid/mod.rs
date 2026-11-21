//! Hyperliquid's perpetuals (the main dex and the HIP-3 dexes such as trade.xyz's `xyz:`) and spot
//! pairs. Public `/info` queries, POSTed, no key and no region block.
//!
//! A candle request answers a window of time (`startTime`, `endTime`), at most ~5,000 candles, and
//! Hyperliquid keeps only the newest ~5,000 of each interval: a 1-minute chart reaches back about
//! three and a half days. Prices and volume are strings; volume is in the base asset. Rate limit:
//! 1,200 weight a minute per IP, a candle request weighing about 20 (measured: a burst of ~95
//! requests of 500 candles, or ~25 of 5,000, is answered with HTTP 429, and the allowance returns
//! at 20 a second).

mod catalog;
mod ids;
pub(in crate::service) mod live;
mod markets;
mod names;
mod parse;
pub(in crate::service) mod quotes;
mod rest;
mod socket;
#[cfg(test)]
mod tests;
mod ticker;

use super::candles::plan;
use super::live::CandleProtocol;
use super::paging::Paging;
use super::search_rank::normalize;
use super::venue::Venue;
use crate::model::market::MarketKind;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use catalog::{CATALOG, download};
use ids::{coin_id, is_spot};
use markets::matching;
use std::time::Duration;

pub struct Hyperliquid;

pub const HYPERLIQUID: Hyperliquid = Hyperliquid;

impl Venue for Hyperliquid {
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

    fn kind_of(&self, id: &InstrumentId) -> MarketKind {
        if is_spot(&id.symbol) {
            MarketKind::Spot
        } else {
            MarketKind::Perp
        }
    }

    fn clear_catalog(&self) {
        catalog::clear();
    }
}

/// One request answers every candle Hyperliquid keeps (~5,000), so a chart is one page: the
/// first fetch leaves the rest for its first older page, and the next one is empty.
const PAGING: Paging = Paging {
    page: 5_000,
    max_candles: 5_000,
    pause: Duration::ZERO,
};

/// Hyperliquid's `interval` for a candle length it serves itself: `1m`, `4h`, `1d`, `3d`.
fn interval(timeframe: Timeframe) -> Option<String> {
    DataSource::Hyperliquid
        .natives()
        .contains(&timeframe)
        .then(|| timeframe.interval())
}

impl MarketSource for Hyperliquid {
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
        let plan = plan(DataSource::Hyperliquid, timeframe)?;
        let requested = PAGING.requested(plan, max);
        // So the live feed of this chart can share a connection with the other listed markets.
        catalog::warm();
        let id = coin_id(symbol);
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            rest::page_before(&id, plan.base(), start_ms, after, wanted)
        })?;
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `xyz:SP500`, `xyz sp500` and `XYZSP500` are one query.
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let markets = CATALOG.get(download)?;
        let results = matching(&markets, &needle);
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}
