//! Bitfinex spot candles, its market list and its sockets. Public endpoints, no key.
//!
//! A market is a pair name as Bitfinex lists it, `BTCUSD` or, once an asset has more than three
//! letters, `AVAX:USD`; every API takes it with a `t` in front (`tBTCUSD`), which the app does not
//! keep. A candle request answers up to 10,000 bars, newest first, and pages back by `end`; it is
//! limited hard and answers `[]` when it is, which an empty range does too, so requests are
//! scheduled and an empty page is checked before it is believed (`rest.rs`).

mod channels;
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
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::watchlist_quotes::provider::refused_by_region;
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use markets::{CATALOG, clear_catalog, download_pairs, matching_pairs};
use rest::PAGING;

pub(crate) const BASE_URL: &str = "https://api-pub.bitfinex.com/v2";

pub struct Bitfinex;

pub const BITFINEX: Bitfinex = Bitfinex;

impl Venue for Bitfinex {
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
            403 => Some(refused_by_region(DataSource::Bitfinex)),
            _ => None,
        }
    }
}

/// Bitfinex's name of a candle length it serves itself: `1m`, `3h`, `1D`, `1W`, `1M`.
fn period(timeframe: Timeframe) -> Option<String> {
    DataSource::Bitfinex
        .natives()
        .contains(&timeframe)
        .then(|| timeframe.interval().replace('d', "D").replace('w', "W"))
}

/// The name of a market as the app keeps it: `BTCUSD`, `AVAX:USD`. `BTC/USD`, `btc-usd`,
/// `avaxusd` and a bare `BTC` (USD) are accepted; text that cannot name a market is `None`, so it is
/// never put in a URL.
fn market(symbol: &str) -> Option<String> {
    // A name with no letter or digit would be read as a bare asset of nothing and get a quote.
    if !symbol.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    let id = InstrumentId::new(DataSource::Bitfinex, symbol).api_symbol();
    id.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == ':')
        .then_some(id)
}

/// What every API calls a market: its name with a `t` in front, `tBTCUSD`.
fn symbol(text: &str) -> Option<String> {
    market(text).map(|market| format!("t{market}"))
}

impl MarketSource for Bitfinex {
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
        let plan = plan(DataSource::Bitfinex, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let market = market(symbol)
            .ok_or_else(|| ProviderError::unknown_market(DataSource::Bitfinex, symbol, ""))?;
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            rest::page_before(&market, plan.base(), after, wanted)
        })?;
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC-USD`, `btc/usd` and `BTCUSD` are one query.
        let needle = normalize(query);
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
