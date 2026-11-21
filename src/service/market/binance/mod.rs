//! Binance spot: the Binance-shaped API configured with its own hosts. Everything that parses or
//! pages is `service::market::shape::binance`; what is Binance's alone is here.

pub(in crate::service) mod live;
mod markets;
mod ticker;

use super::live::CandleProtocol;
use super::shape::binance::{Config, trading};
use super::venue::Venue;
use crate::model::DataSource;
use crate::model::{Kline, Timeframe};
use crate::service::watchlist_quotes::provider::rejected;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult};
use markets::CATALOG;
use std::time::Duration;

/// The most klines one Binance request returns.
const MAX_PER_REQUEST: usize = 1_000;

static SHAPE: Config = Config {
    source: DataSource::Binance,
    klines_url: "https://api.binance.com/api/v3/klines",
    max_per_request: MAX_PER_REQUEST,
    exchange_info_url: "https://api.binance.com/api/v3/exchangeInfo",
    serves: trading,
    catalog: &CATALOG,
    stream_host: "stream.binance.com",
    // Binance takes 1,024 topics a connection.
    max_streams: 1_024,
    max_silence: Duration::from_secs(60),
};

pub struct Binance;

pub const BINANCE: Binance = Binance;

impl Venue for Binance {
    fn clear_catalog(&self) {
        SHAPE.clear_catalog();
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
            400 => Some(rejected(400)),
            _ => None,
        }
    }
}

impl MarketSource for Binance {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        SHAPE.fetch_klines(symbol, timeframe, limit)
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        SHAPE.fetch_klines_range(symbol, timeframe, start_ms, end_ms, max)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        SHAPE.search(query)
    }
}

#[cfg(test)]
mod tests;
