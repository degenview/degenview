//! BingX spot candles, its market list and its sockets. Public endpoints, no key.
//!
//! A market is `BTC-USDT`. Two endpoints serve candles, a recent one and a history one, each for the
//! pages it has (`history.rs`); the history's prices carry a digit beyond the tick, cut off to be
//! the recent endpoint's (`precision.rs`). BingX cuts its days, weeks, months, `6h` and `12h` at
//! UTC+8 midnight, not where this app cuts them, so those fold from its finer bars.

mod history;
pub(in crate::service) mod live;
mod markets;
mod parse;
mod precision;
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
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::watchlist_quotes::provider::refused_by_region;
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use history::{PAGING, page_before};
use markets::{CATALOG, clear_catalog, download_pairs, find, matching_pairs};

pub struct Bingx;

pub const BINGX: Bingx = Bingx;

impl Venue for Bingx {
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

    fn classify_status(&self, code: u16) -> Option<QuoteFailure> {
        match code {
            403 => Some(refused_by_region(DataSource::Bingx)),
            _ => None,
        }
    }
}

/// BingX's name of a market, `BTC-USDT`: `BTC/USDT`, `btc-usdt` and a bare `BTC` (USDT) are
/// accepted; text that cannot name a market is `None`, so it is never put in a URL.
fn market_id(symbol: &str) -> Option<String> {
    // A name with no letter or digit would be read as a bare asset of nothing and get a quote.
    if !symbol.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    let id = InstrumentId::new(DataSource::Bingx, symbol)
        .api_symbol()
        .to_uppercase();
    id.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        .then_some(id)
}

impl MarketSource for Bingx {
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
        let plan = plan(DataSource::Bingx, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let not_listed = || ProviderError::unknown_market(DataSource::Bingx, symbol, "");
        let id = market_id(symbol).ok_or_else(not_listed)?;
        // The market list also says how many digits its prices have.
        let pairs = CATALOG.get(download_pairs)?;
        let market = find(&pairs, &id).ok_or_else(not_listed)?;
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            page_before(market, plan.base(), after, wanted)
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
