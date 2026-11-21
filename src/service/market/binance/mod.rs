pub(in crate::service) mod live;
mod markets;
mod parse;
mod ticker;

use super::candles::plan;
use super::live::CandleProtocol;
use super::search_rank::normalize;
use super::venue::Venue;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::watchlist_quotes::provider::rejected;
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use markets::clear_catalog;
use markets::{CATALOG, download_pairs, matching_pairs};
#[cfg(test)]
use markets::{Pair, parse_pairs};
use parse::parse_klines;

const BASE_URL: &str = "https://api.binance.com/api/v3/klines";

pub struct Binance;

pub const BINANCE: Binance = Binance;

impl Venue for Binance {
    fn clear_catalog(&self) {
        clear_catalog();
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

/// The most klines one Binance request returns.
const MAX_PER_REQUEST: usize = 1_000;

impl MarketSource for Binance {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Binance, timeframe)?;
        let requested = plan.base_count(limit as usize);
        if requested > MAX_PER_REQUEST {
            // More finer candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let url = format!(
            "{BASE_URL}?symbol={}&interval={}&limit={requested}",
            symbol.to_uppercase(),
            plan.base().interval()
        );
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Transient,
        )?;
        Ok(plan.shape(parse_klines(&body)?, requested, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        const PAGE: usize = MAX_PER_REQUEST;
        // Pages are fetched side by side: their start times follow from the interval alone.
        const PARALLEL: usize = 8;
        let plan = plan(DataSource::Binance, timeframe)?;
        let interval = plan.base().interval();
        let requested = plan.base_count(max);
        let step = plan.base().nominal_ms();
        let page_ms = step * PAGE as i64;
        let first = (end_ms - step * requested as i64).max(start_ms);
        let starts: Vec<i64> = (0..)
            .map(|index| first + index * page_ms)
            .take_while(|start| *start < end_ms)
            .collect();
        let mut candles: Vec<Kline> = Vec::new();
        for batch in starts.chunks(PARALLEL) {
            let pages: Vec<Result<Vec<Kline>, ProviderError>> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|&from| {
                        let interval = &interval;
                        scope.spawn(move || {
                            let to = (from + page_ms).min(end_ms) - 1;
                            let url = format!(
                                "{BASE_URL}?symbol={}&interval={interval}&startTime={from}&endTime={to}&limit={PAGE}",
                                symbol.to_uppercase()
                            );
                            let body =
                                crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)?;
                            parse_klines(&body)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().unwrap_or_else(|_| {
                            Err(ProviderError::other("The candle download stopped."))
                        })
                    })
                    .collect()
            });
            for page in pages {
                candles.extend(page?);
            }
        }
        candles.sort_by_key(|candle| candle.open_time);
        candles.dedup_by_key(|candle| candle.open_time);
        if candles.len() > requested {
            candles.drain(..candles.len() - requested);
        }
        Ok(plan.shape(candles, requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC-USDT` and `btc/usdt` find `BTCUSDT`: Binance writes no separator.
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
