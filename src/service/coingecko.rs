use super::{MarketSource, SearchResult, bucket_start_ms, interval_seconds, next_bucket_start_ms};
use crate::model::Kline;
use serde_json::Value;

const BASE_URL: &str = "https://api.coingecko.com/api/v3";

pub struct CoinGecko;

pub const COINGECKO: CoinGecko = CoinGecko;

impl MarketSource for CoinGecko {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        let coin_id = symbol.to_lowercase();
        let target = interval_seconds(interval).unwrap_or(3_600);
        let days = market_chart_days(target, limit);
        super::rate_limit::coingecko().wait_for_slot();
        let url = format!("{BASE_URL}/coins/{coin_id}/market_chart?vs_currency=usd&days={days}");
        let body = super::http::get_text(&url, &[], super::http::Retry::Never)
            .map_err(|error| format!("Network error: {error}"))?;
        let prices = parse_prices(&body)?;
        let candles = fold_prices(&prices, target);
        let start = candles.len().saturating_sub(limit as usize);
        Ok(candles[start..].to_vec())
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{BASE_URL}/search?query={q}");
        let body = super::http::get_text(&url, &[], super::http::Retry::Never)
            .map_err(|error| format!("Network error: {error}"))?;
        let value: Value =
            serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;
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

/// Days of `/market_chart` history that can build the requested candles. Bounded
/// by the deepest tier whose samples are still fine enough to fold into a candle.
fn market_chart_days(target_secs: i64, limit: u32) -> i64 {
    let deepest = (i64::from(limit) * target_secs + 86_399) / 86_400;
    let tier = [1, 90, 365]
        .into_iter()
        .filter(|days| sample_step(*days) <= target_secs)
        .max()
        .unwrap_or(1);
    deepest.min(tier).clamp(1, 365)
}

fn sample_step(days: i64) -> i64 {
    if days <= 1 {
        300
    } else if days <= 90 {
        3_600
    } else {
        86_400
    }
}

/// `/market_chart` returns `{"prices": [[timestamp_ms, price], …]}`, oldest first.
fn parse_prices(body: &str) -> Result<Vec<(i64, f64)>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    let rows = value
        .get("prices")
        .and_then(Value::as_array)
        .ok_or_else(|| "Data error: expected a prices array".to_owned())?;
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

/// Fold a price series into candles of `target_secs`, one candle per calendar bucket.
///
/// Each candle opens at the previous one's close, so the series is continuous and a bucket holding
/// a single sample still gets a body (and a colour); only the first opens at its own first sample.
fn fold_prices(prices: &[(i64, f64)], target_secs: i64) -> Vec<Kline> {
    let mut out: Vec<Kline> = Vec::new();
    for &(ms, price) in prices {
        let bucket = bucket_start_ms(ms, target_secs);
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
                    close_time: next_bucket_start_ms(bucket, target_secs) - 1,
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
    fn window_tracks_interval_and_limit() {
        assert_eq!(market_chart_days(3_600, 500), 21);
        assert_eq!(market_chart_days(86_400, 500), 365);
    }

    #[test]
    fn folds_prices_into_buckets() {
        let prices = vec![
            (1_700_000_000_000, 10.0),
            (1_700_001_800_000, 12.0), // same hour bucket as above
            (1_700_003_600_000, 8.0),  // next hour bucket
        ];
        let candles = fold_prices(&prices, 3_600);
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
        let candles = fold_prices(&[(1_700_000_000_000, 10.0)], 3_600);
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
        let candles = fold_prices(&prices, 3_600);
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
