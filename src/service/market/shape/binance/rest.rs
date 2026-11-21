//! Candles over the klines endpoint: one request, or pages fetched side by side.

use super::Config;
use super::klines::parse_klines;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::market::candles::plan;
use crate::service::net::http;

/// Pages are fetched side by side: their start times follow from the interval alone.
const PARALLEL: usize = 8;

impl Config {
    /// The answer to a klines request, or why there is none: a market the venue does not list is
    /// said so, not left as a bare HTTP 400.
    fn get_klines(&self, url: &str, symbol: &str) -> Result<String, ProviderError> {
        http::get_text_reading_refusals(url, &[], http::Retry::Transient)
            .map_err(|error| refusal(self.source, symbol, error))
    }

    pub(in crate::service) fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(self.source, timeframe)?;
        let requested = plan.base_count(limit as usize);
        if requested > self.max_per_request {
            // More finer candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let url = format!(
            "{}?symbol={}&interval={}&limit={requested}",
            self.klines_url,
            symbol.to_uppercase(),
            plan.base().interval()
        );
        let body = self.get_klines(&url, symbol)?;
        Ok(plan.shape(parse_klines(&body)?, requested, limit as usize))
    }

    pub(in crate::service) fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let page = self.max_per_request;
        let plan = plan(self.source, timeframe)?;
        let interval = plan.base().interval();
        let requested = plan.base_count(max);
        let step = plan.base().nominal_ms();
        let page_ms = step * page as i64;
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
                                "{}?symbol={}&interval={interval}&startTime={from}&endTime={to}&limit={page}",
                                self.klines_url,
                                symbol.to_uppercase()
                            );
                            let body = self.get_klines(&url, symbol)?;
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
            for fetched in pages {
                candles.extend(fetched?);
            }
        }
        candles.sort_by_key(|candle| candle.open_time);
        candles.dedup_by_key(|candle| candle.open_time);
        if candles.len() > requested {
            candles.drain(..candles.len() - requested);
        }
        Ok(plan.shape(candles, requested, max))
    }
}

/// Binance's error code for a symbol it does not list (`{"code":-1121,"msg":"Invalid symbol."}`
/// with HTTP 400), which its clones answer alike.
const INVALID_SYMBOL: i64 = -1121;

/// `error` as the provider's error: [`ProviderError::UnknownMarket`] for a refusal that says the
/// symbol is invalid, else the error as it came.
pub(super) fn refusal(source: DataSource, symbol: &str, error: http::Error) -> ProviderError {
    if let http::Error::Refused { status: 400, body } = &error
        && serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value.get("code")?.as_i64())
            == Some(INVALID_SYMBOL)
    {
        return ProviderError::unknown_market(source, symbol.to_uppercase(), "");
    }
    error.into()
}
