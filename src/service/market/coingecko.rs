use crate::model::market::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::market::samples;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::net::url::encode_component;
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde_json::Value;

pub(in crate::service) const BASE_URL: &str = "https://api.coingecko.com/api/v3";

pub struct CoinGecko;

pub const COINGECKO: CoinGecko = CoinGecko;

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
        Ok(newest_candles(&prices, timeframe, limit))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = search_url(&q);
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
            results.push(
                SearchResult::new(DataSource::CoinGecko, symbol.to_uppercase(), id)
                    .named(coin.get("name").and_then(Value::as_str), None),
            );
        }
        Ok(results.into_iter().take(20).collect())
    }
}

/// `/market_chart` returns `{"prices": [[timestamp_ms, price], …]}`, oldest first.
/// The search address for an already trimmed, lower-cased query.
fn search_url(query: &str) -> String {
    format!("{BASE_URL}/search?query={}", encode_component(query))
}

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

/// The newest `limit` candles of a window of price samples. A candle opens where the one before it
/// closed, which the window does not say of its first bucket: that one is left out, unless it is all
/// there is.
fn newest_candles(prices: &[(i64, f64)], timeframe: Timeframe, limit: u32) -> Vec<Kline> {
    let candles = fold_prices(prices, timeframe);
    let unanchored = usize::from(candles.len() > 1);
    let start = candles.len().saturating_sub(limit as usize).max(unanchored);
    candles[start..].to_vec()
}

/// Fold a price series into candles of `target`, one candle per calendar bucket.
///
/// Each candle opens at the previous one's close, so the series is continuous and a bucket holding
/// a single sample still gets a body (and a colour); only the first opens at its own first sample.
fn fold_prices(prices: &[(i64, f64)], target: Timeframe) -> Vec<Kline> {
    let mut out: Vec<Kline> = Vec::new();
    for &(ms, price) in prices {
        let bucket = bucket_start_ms(target, ms);
        match out.last_mut() {
            Some(last) if last.open_time == bucket => {
                last.high = last.high.max(price);
                last.low = last.low.min(price);
                last.close = price;
            }
            _ => {
                let open = out.last().map_or(price, |previous| previous.close);
                out.push(Kline {
                    open_time: bucket,
                    close_time: next_bucket_start_ms(target, bucket) - 1,
                    open,
                    high: open.max(price),
                    low: open.min(price),
                    close: price,
                    volume: 0.0,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_outside_ascii_is_escaped_in_the_search_address() {
        assert_eq!(
            search_url("比特币 & co"),
            format!("{BASE_URL}/search?query=%E6%AF%94%E7%89%B9%E5%B8%81%20%26%20co")
        );
    }

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

    /// The window the samples cover moves on with time: two answers a bucket apart must say the
    /// same of every candle both have, or a chart takes the older one for changed history.
    #[test]
    fn answers_a_bucket_apart_agree_on_the_candles_they_share() {
        let samples: Vec<(i64, f64)> = (0..6)
            .map(|index| (index * 300_000, 10.0 + index as f64))
            .collect();
        let before = newest_candles(&samples[..5], Timeframe::MINUTE_5, 500);
        let after = newest_candles(&samples[1..], Timeframe::MINUTE_5, 500);
        let shared: Vec<&Kline> = after
            .iter()
            .filter(|candle| before.iter().any(|old| old.open_time == candle.open_time))
            .collect();
        assert_eq!(shared.len(), 3);
        for candle in shared {
            assert!(before.contains(candle), "{candle:?}");
        }
    }

    #[test]
    fn a_history_of_one_candle_is_that_candle() {
        let candles = newest_candles(&[(0, 10.0)], Timeframe::MINUTE_5, 500);
        assert_eq!(candles.len(), 1);
    }

    #[test]
    fn folds_prices_into_buckets() {
        let prices = vec![
            (1_700_000_000_000, 10.0),
            (1_700_001_800_000, 12.0), // same hour bucket as above
            (1_700_003_600_000, 8.0),  // next hour bucket
        ];
        let candles = fold_prices(&prices, Timeframe::HOUR_1);
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].high, 12.0);
        assert_eq!(candles[0].low, 10.0);
        assert_eq!(candles[0].close, 12.0);
        assert_eq!(candles[1].close, 8.0);
        assert_eq!(candles[1].open, 12.0);
        assert_eq!(candles[1].high, 12.0);
        assert_eq!(candles[1].low, 8.0);
    }

    #[test]
    fn first_candle_opens_at_its_first_sample() {
        let candles = fold_prices(&[(1_700_000_000_000, 10.0)], Timeframe::HOUR_1);
        assert_eq!(candles[0].open, 10.0);
        assert_eq!(candles[0].close, 10.0);
    }

    #[test]
    fn single_sample_buckets_get_a_body_and_a_colour() {
        // One sample per hour, as `/market_chart` returns for 2–90 days.
        let prices = vec![
            (1_700_000_000_000, 10.0),
            (1_700_003_600_000, 9.0),
            (1_700_007_200_000, 11.0),
        ];
        let candles = fold_prices(&prices, Timeframe::HOUR_1);
        assert_eq!(candles.len(), 3);
        assert!(!candles[1].is_up());
        assert_eq!((candles[1].open, candles[1].close), (10.0, 9.0));
        assert_eq!((candles[1].high, candles[1].low), (10.0, 9.0));
        assert!(candles[2].is_up());
        assert_eq!((candles[2].open, candles[2].close), (9.0, 11.0));
    }

    #[test]
    fn parses_price_series_and_skips_nulls() {
        let body = r#"{"prices":[[1700000000000,10.0],[1700003600000,null],[1700007200000,11.0]]}"#;
        let prices = parse_prices(body).expect("valid");
        assert_eq!(prices.len(), 2);
        assert_eq!(prices[1].1, 11.0);
    }
}
