//! OKX spot candles and its market list. Public endpoints, no key.
//!
//! Rows are `[open time ms, open, high, low, close, base volume, quote volume, quote volume,
//! confirm]`, all strings, newest first. `/market/candles` serves the newest 300; older ones are
//! paged back with `/market/history-candles` and an `after` cursor. Failures arrive as HTTP 200
//! with a `code` other than `"0"`.

pub(in crate::service) mod live;
mod markets;
mod parse;
pub(in crate::service) mod quotes;
mod rest;
mod ticker;

use super::candles::plan;
use super::live::CandleProtocol;
use super::paging::Paging;
use super::search_rank::normalize;
use super::venue::Venue;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::watchlist_quotes::quote_stream::QuoteProtocol;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use markets::clear_catalog;
use markets::{CATALOG, download_pairs, matching_pairs};
#[cfg(test)]
use markets::{Pair, parse_pairs};
#[cfg(test)]
use parse::parse_candles;
use parse::parse_row;
use rest::fetch_page;
use std::time::Duration;

const BASE_URL: &str = "https://www.okx.com/api/v5";

pub struct Okx;

pub const OKX: Okx = Okx;

impl Venue for Okx {
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

/// 300 rows a request, 120 pages (a 22-minute chart of 500 candles is 37), and a pause between
/// them for its 20 requests per 2 seconds per IP.
const PAGING: Paging = Paging {
    page: 300,
    max_candles: 36_000,
    pause: Duration::from_millis(100),
};

/// OKX's bar token for a candle length it serves itself: `5m`, `4H`, and the `utc` bars from six
/// hours up (`6Hutc`, `1Dutc`, `1Wutc`, `1Mutc`, `3Mutc`), because its plain ones are cut at UTC+8
/// and the app's buckets are UTC.
pub(crate) fn bar(timeframe: Timeframe) -> Option<String> {
    if !DataSource::Okx.natives().contains(&timeframe) {
        return None;
    }
    let token = timeframe.interval();
    let (count, unit) = token.split_at(token.len() - 1);
    Some(match unit {
        "m" => token,
        "h" if count.parse::<u16>().ok()? < 6 => format!("{count}H"),
        _ => format!("{count}{}utc", unit.to_uppercase()),
    })
}

/// OKX's id of a market: `BTC-USDT`. `BTC/USDT` and a bare `BTC` (USDT) are accepted.
pub(crate) fn instrument_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Okx, symbol).api_symbol()
}

impl MarketSource for Okx {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Okx, timeframe)?;
        let requested = PAGING.requested(plan, limit as usize);
        if requested > PAGING.page {
            // More base candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let rows = fetch_page(
            "candles",
            &instrument_id(symbol),
            plan.base(),
            requested,
            None,
        )?;
        Ok(plan.shape(rows, requested, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Okx, timeframe)?;
        let requested = PAGING.requested(plan, max);
        let id = instrument_id(symbol);
        let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
            fetch_page("history-candles", &id, plan.base(), wanted, Some(after))
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

#[cfg(test)]
mod tests;
