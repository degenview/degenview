mod ticker;

use super::venue::Venue;
use crate::model::market::samples;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::watchlist_quotes::{Outcome, QuoteFailure, QuoteRequest};
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde_json::Value;

const BASE_URL: &str = "https://api.coingecko.com/api/v3";

pub struct CoinGecko;

pub const COINGECKO: CoinGecko = CoinGecko;

impl Venue for CoinGecko {
    fn candles(&self) -> &dyn MarketSource {
        self
    }

    fn rest_quotes(&self, requests: &[QuoteRequest]) -> Result<Outcome, QuoteFailure> {
        ticker::fetch_quotes(requests).map(Outcome::from_quotes)
    }
}

impl MarketSource for CoinGecko {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let coin_id = symbol.to_lowercase();
        let days = samples::chart_days(timeframe, limit)
            .ok_or_else(|| ProviderError::unsupported(DataSource::CoinGecko, timeframe))?;
        crate::service::net::rate_limit::coingecko().wait_for_slot();
        let url = format!("{BASE_URL}/coins/{coin_id}/market_chart?vs_currency=usd&days={days}");
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Never,
        )?;
        let prices = parse_prices(&body)?;
        let candles = samples::fold_samples(&prices, timeframe);
        let start = candles.len().saturating_sub(limit as usize);
        Ok(candles[start..].to_vec())
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{BASE_URL}/search?query={q}");
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Never,
        )?;
        let value: Value = serde_json::from_str(&body).map_err(ProviderError::data)?;
        let mut results = Vec::new();
        for coin in value
            .get("coins")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let (Some(id), Some(symbol)) = (
                coin.get("id").and_then(Value::as_str),
                coin.get("symbol").and_then(Value::as_str),
            ) else {
                continue;
            };
            results.push(SearchResult::new(
                crate::model::DataSource::CoinGecko,
                symbol.to_uppercase(),
                id,
            ));
        }
        Ok(results.into_iter().take(20).collect())
    }
}

/// `/market_chart` returns `{"prices": [[timestamp_ms, price], …]}`, oldest first.
fn parse_prices(body: &str) -> Result<Vec<(i64, f64)>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let rows = value
        .get("prices")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::data("expected a prices array"))?;
    let mut prices = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 2 {
            continue;
        }
        let (Some(ms), Some(price)) = (fields[0].as_f64(), fields[1].as_f64()) else {
            continue;
        };
        prices.push((ms as i64, price));
    }
    prices.sort_by_key(|(ms, _)| *ms);
    Ok(prices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_the_samples_cannot_build_is_refused_before_any_request() {
        for timeframe in [Timeframe::MINUTE_1, Timeframe::parse("22m").unwrap()] {
            let error = COINGECKO
                .fetch_klines("bitcoin", timeframe, 100)
                .unwrap_err();
            assert_eq!(
                error,
                ProviderError::unsupported(DataSource::CoinGecko, timeframe)
            );
        }
    }

    #[test]
    fn parses_price_series_and_skips_nulls() {
        let body = r#"{"prices":[[1700000000000,10.0],[1700003600000,null],[1700007200000,11.0]]}"#;
        let prices = parse_prices(body).expect("valid");
        assert_eq!(prices.len(), 2);
        assert_eq!(prices[1].1, 11.0);
    }
}
