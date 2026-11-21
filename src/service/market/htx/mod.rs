//! HTX (formerly Huobi) spot candles, its market list and its sockets. Public endpoints, no key.
//!
//! A market is `btcusdt`, in lower case on every API; the app keeps the upper-case spelling the
//! other exchanges use and lowers it for HTX. REST answers only the newest 2,000 candles of a
//! period, so older history is asked over the socket (`history.rs`). Its days, weeks and months open
//! at 16:00 UTC (UTC+8 midnight), not where this app cuts them, so everything from four hours up
//! folds from its four-hour bars.

mod history;
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
use history::{History, PAGING};
use markets::{CATALOG, clear_catalog, download_pairs, matching_pairs};

pub struct Htx;

pub const HTX: Htx = Htx;

impl Venue for Htx {
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
            403 => Some(refused_by_region(DataSource::Htx)),
            _ => None,
        }
    }
}

/// The periods HTX serves itself and what it calls them.
const PERIODS: [(Timeframe, &str); 6] = [
    (Timeframe::MINUTE_1, "1min"),
    (Timeframe::MINUTE_5, "5min"),
    (Timeframe::MINUTE_15, "15min"),
    (Timeframe::MINUTE_30, "30min"),
    (Timeframe::HOUR_1, "60min"),
    (Timeframe::HOUR_4, "4hour"),
];

/// HTX's `period` for a candle length it serves itself.
fn period(timeframe: Timeframe) -> Option<&'static str> {
    PERIODS
        .iter()
        .find(|(native, _)| *native == timeframe)
        .map(|(_, period)| *period)
}

/// HTX's name of a market, `btcusdt`: lower case, which is the only spelling its APIs take.
/// `BTC/USDT`, `btc-usdt` and a bare `BTC` (USDT) are accepted; text that cannot name a market is
/// `None`, so it is never put in a URL.
fn symbol_id(symbol: &str) -> Option<String> {
    // A name with no letter or digit would be read as a bare asset of nothing and get a quote.
    if !symbol.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    let id = InstrumentId::new(DataSource::Htx, symbol)
        .api_symbol()
        .to_lowercase();
    id.chars()
        .all(|ch| ch.is_ascii_alphanumeric())
        .then_some(id)
}

impl MarketSource for Htx {
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
        let plan = plan(DataSource::Htx, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let id = symbol_id(symbol)
            .ok_or_else(|| ProviderError::unknown_market(DataSource::Htx, symbol, ""))?;
        let mut history = History::new();
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            history.page_before(&id, plan.base(), after, wanted)
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
