use crate::model::market::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::market::samples;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::market::error::ErrorKind;
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
        let body =
            crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Never)
                .map_err(|error| ProviderError::http(DataSource::CoinGecko, error))?;
        let prices = parse_prices(&body)?;
        Ok(newest_candles(
            &prices,
            timeframe,
            limit,
            samples::spacing_ms(days),
        ))
    }

    fn fetch_klines_range(
        &self,
        _symbol: &str,
        _timeframe: Timeframe,
        _start_ms: i64,
        _end_ms: i64,
        _max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        Err(ProviderError::new(
            DataSource::CoinGecko,
            ErrorKind::NoTimeRange,
        ))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = search_url(&q);
        let body =
            crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Never)
                .map_err(|error| ProviderError::http(DataSource::CoinGecko, error))?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| ProviderError::malformed(DataSource::CoinGecko, error))?;
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
    let value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::CoinGecko, error))?;
    let rows = value
        .get("prices")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProviderError::malformed(DataSource::CoinGecko, "expected a prices array")
        })?;
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

/// The last sample is the price now, off the sampling grid. It can have passed a boundary whose own
/// sample is not published yet; until that one is, this price stands in for it. Filed past it, the
/// candle ending there would be missing, or closed without its last price.
fn stand_in_for_unpublished(prices: &mut [(i64, f64)], spacing: i64) {
    if let [.., (published, _), (now, _)] = prices
        && *now - *published > spacing
    {
        *now = *published + spacing;
    }
}

/// The newest `limit` candles of a window of price samples. A candle opens where the one before it
/// closed, which the window does not say of its first bucket: that one is left out, unless it is all
/// there is.
fn newest_candles(
    prices: &[(i64, f64)],
    timeframe: Timeframe,
    limit: u32,
    spacing: i64,
) -> Vec<Kline> {
    let mut prices = prices.to_vec();
    stand_in_for_unpublished(&mut prices, spacing);
    let candles = fold_prices(&prices, timeframe);
    let unanchored = usize::from(candles.len() > 1);
    let start = candles.len().saturating_sub(limit as usize).max(unanchored);
    candles[start..].to_vec()
}

/// Fold a price series into candles of `target`, one candle per calendar bucket. A sample is the
/// price at its instant, so it belongs to the bucket that was running just before it: the one on a
/// boundary closes the candle that ends there.
///
/// Each candle opens at the previous one's close, so the series is continuous and a bucket holding
/// a single sample still gets a body (and a colour); only the first opens at its own first sample.
fn fold_prices(prices: &[(i64, f64)], target: Timeframe) -> Vec<Kline> {
    let mut out: Vec<Kline> = Vec::new();
    for &(ms, price) in prices {
        let bucket = bucket_start_ms(target, ms - 1);
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
        let before = newest_candles(&samples[..5], Timeframe::MINUTE_5, 500, 300_000);
        let after = newest_candles(&samples[1..], Timeframe::MINUTE_5, 500, 300_000);
        let shared: Vec<&Kline> = after
            .iter()
            .filter(|candle| before.iter().any(|old| old.open_time == candle.open_time))
            .collect();
        assert_eq!(shared.len(), 3);
        for candle in shared {
            assert!(before.contains(candle), "{candle:?}");
        }
    }

    /// A sample is the price at its instant, so the one at a bucket's end is that candle's close,
    /// not the first of the next.
    #[test]
    fn the_sample_on_a_boundary_closes_the_candle_that_ends_there() {
        let samples = [(0, 10.0), (300_000, 11.0), (600_000, 12.0), (660_000, 13.0)];
        let candles = newest_candles(&samples, Timeframe::MINUTE_5, 500, 300_000);
        let seen: Vec<(i64, f64, f64)> = candles
            .iter()
            .map(|candle| (candle.open_time, candle.open, candle.close))
            .collect();
        // The window's first sample closes a candle it does not hold the start of.
        assert_eq!(
            seen,
            [
                (0, 10.0, 11.0),
                (300_000, 11.0, 12.0),
                (600_000, 12.0, 13.0)
            ][..]
        );
    }

    /// The price now can pass a boundary before CoinGecko publishes that boundary's sample
    /// (seen 2026-11-27: 16:00 and 17:00:50, no 17:00). The candle ending there is still the
    /// newest one, and no candle but the newest changes once the sample arrives.
    #[test]
    fn a_boundary_not_yet_published_leaves_its_candle_forming() {
        const HOUR: i64 = 3_600_000;
        let early = [
            (14 * HOUR, 10.0),
            (15 * HOUR, 11.0),
            (16 * HOUR, 12.0),
            (17 * HOUR + 50_000, 13.0),
        ];
        let answer = |samples: &[(i64, f64)]| newest_candles(samples, Timeframe::HOUR_1, 500, HOUR);
        let before = answer(&early);
        assert_eq!(
            before
                .iter()
                .map(|bar| bar.open_time / HOUR)
                .collect::<Vec<_>>(),
            [14, 15, 16]
        );
        assert_eq!(before[2].close, 13.0, "the price now stands in");

        let mut published = early[..3].to_vec();
        published.extend([(17 * HOUR, 12.5), (17 * HOUR + 120_000, 14.0)]);
        let after = answer(&published);
        assert_eq!(after[..2], before[..2]);
        assert_eq!((after[2].open_time, after[2].close), (16 * HOUR, 12.5));
        assert_eq!((after[3].open_time, after[3].close), (17 * HOUR, 14.0));
    }

    #[test]
    fn a_history_of_one_candle_is_that_candle() {
        let candles = newest_candles(&[(0, 10.0)], Timeframe::MINUTE_5, 500, 300_000);
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
