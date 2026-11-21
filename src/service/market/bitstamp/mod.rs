//! Bitstamp spot candles, its market list and its sockets. Public endpoints, no key.
//!
//! A market is `btcusd`, in lower case on every API; the app keeps the upper-case spelling the other
//! exchanges use and lowers it for Bitstamp. Bitstamp's socket has no candle or ticker channel, only
//! trades, so the live candle and the live price are built from those (`live.rs`, `quotes.rs`); a
//! candle request answers up to 1,000 bars of a step and pages back by `end` (`rest.rs`). It has no
//! week, month or year, which fold from its days.

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
pub(in crate::service::market) use parse::candle_rows;
use rest::PAGING;

pub struct Bitstamp;

pub const BITSTAMP: Bitstamp = Bitstamp;

impl Venue for Bitstamp {
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
            403 => Some(refused_by_region(DataSource::Bitstamp)),
            _ => None,
        }
    }
}

/// Bitstamp's `step` for a candle length it serves itself, in seconds (`60`, `3600`, `259200`).
fn step(timeframe: Timeframe) -> Option<i64> {
    DataSource::Bitstamp
        .natives()
        .contains(&timeframe)
        .then(|| timeframe.nominal_ms() / 1_000)
}

/// Bitstamp's name of a market, `btcusd`: lower case, which is the only spelling its APIs take.
/// `BTC/USD`, `btc-usd` and a bare `BTC` (USD) are accepted; text that cannot name a market is
/// `None`, so it is never put in a URL.
fn market_id(symbol: &str) -> Option<String> {
    // A name with no letter or digit would be read as a bare asset of nothing and get a quote.
    if !symbol.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    let id = InstrumentId::new(DataSource::Bitstamp, symbol)
        .api_symbol()
        .to_lowercase();
    id.chars()
        .all(|ch| ch.is_ascii_alphanumeric())
        .then_some(id)
}

impl MarketSource for Bitstamp {
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
        let plan = plan(DataSource::Bitstamp, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let id = market_id(symbol)
            .ok_or_else(|| ProviderError::unknown_market(DataSource::Bitstamp, symbol, ""))?;
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            rest::page_before(&id, plan.base(), after, wanted)
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
