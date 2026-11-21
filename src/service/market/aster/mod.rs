//! Aster's perpetual futures: the Binance-shaped API (`service::market::shape::binance`) on its own
//! hosts. Public, no key and no region block for market data.
//!
//! Verified against the API on 2026-11-21: klines are the Binance rows, 1,500 at most (2,000 is
//! error -1130), minutes and hours from UTC midnight, days at midnight, weeks from Monday and
//! months from the first. Its `3d` opens on a day that is not a multiple of three since
//! 1970-01-01 (the same as Binance's), so it folds from days. Request weight is 2,400 a minute
//! (`x-mbx-used-weight-1m`): a 1,500-row page costs 10, the whole 24 h ticker 40. The socket takes
//! 200 topics a connection ("subscribed channels exceeds limit") and drops a topic it does not
//! list without a word; a stock perpetual is quiet outside its session. A bar is stored some
//! seconds (once over a minute) after it opens, so an answer lacks the newest bar until then; the
//! candle socket has it at once.

pub(in crate::service) mod live;
pub(in crate::service) mod quotes;
#[cfg(test)]
mod tests;
mod ticker;

use super::live::CandleProtocol;
use super::shape::binance::{Config, Pair, trading_perpetual};
use super::venue::Venue;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::market::catalog::Catalog;
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult};
use std::time::Duration;

/// The perpetuals Aster lists, held for a day.
static CATALOG: Catalog<Pair> = Catalog::new("catalog_aster");

static SHAPE: Config = Config {
    source: DataSource::Aster,
    klines_url: "https://fapi.asterdex.com/fapi/v1/klines",
    max_per_request: 1_500,
    exchange_info_url: "https://fapi.asterdex.com/fapi/v1/exchangeInfo",
    serves: trading_perpetual,
    catalog: &CATALOG,
    stream_host: "fstream.asterdex.com",
    max_streams: 200,
    // A stock perpetual makes no trade for a weekend; the heartbeat says the socket is there.
    max_silence: Duration::from_secs(600),
};

pub struct Aster;

pub const ASTER: Aster = Aster;

impl Venue for Aster {
    fn clear_catalog(&self) {
        SHAPE.clear_catalog();
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

impl MarketSource for Aster {
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
