use super::catalog::Catalog;
use super::{MarketSource, SearchResult, fold_calendar, rank_results};
use crate::model::Kline;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BASE_URL: &str = "https://api.binance.com/api/v3/klines";

pub struct Binance;

pub const BINANCE: Binance = Binance;

/// The most klines one Binance request returns.
const MAX_PER_REQUEST: usize = 1_000;

/// How an interval is served: the token Binance knows and, for quarters and years, which calendar
/// fold of monthly klines makes them, as `(seconds, months per bar)`.
fn native_interval(interval: &str) -> (&str, Option<(i64, usize)>) {
    match interval {
        "3M" => ("1M", Some((7_776_000, 3))),
        "1Y" => ("1M", Some((31_536_000, 12))),
        other => (other, None),
    }
}

impl MarketSource for Binance {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        let (native, fold) = native_interval(interval);
        // A fold asks for one extra bar's worth of months so the oldest bar is whole.
        let requested = match fold {
            Some((_, months)) => (limit as usize * months + months).min(MAX_PER_REQUEST),
            None => limit as usize,
        };
        let url = format!(
            "{BASE_URL}?symbol={}&interval={native}&limit={requested}",
            symbol.to_uppercase()
        );
        let body = super::http::get_text(&url, &[], super::http::Retry::Transient)
            .map_err(|error| format!("Network error: {error}"))?;
        let candles = parse_klines(&body)?;
        let Some((seconds, _)) = fold else {
            return Ok(candles);
        };
        let folded = fold_calendar(&candles, seconds, candles.len() >= requested);
        let start = folded.len().saturating_sub(limit as usize);
        Ok(folded[start..].to_vec())
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, String> {
        const PAGE: usize = MAX_PER_REQUEST;
        // Pages are fetched side by side: their start times follow from the interval alone.
        const PARALLEL: usize = 8;
        let (interval, fold) = native_interval(interval);
        let output_max = max;
        let max = max * fold.map_or(1, |(_, months)| months);
        let step = super::interval_seconds(interval)
            .ok_or_else(|| format!("Unknown interval {interval}"))?
            * 1_000;
        let page_ms = step * PAGE as i64;
        let first = (end_ms - step * max as i64).max(start_ms);
        let starts: Vec<i64> = (0..)
            .map(|index| first + index * page_ms)
            .take_while(|start| *start < end_ms)
            .collect();
        let mut candles: Vec<Kline> = Vec::new();
        for batch in starts.chunks(PARALLEL) {
            let pages: Vec<Result<Vec<Kline>, String>> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|&from| {
                        scope.spawn(move || {
                            let to = (from + page_ms).min(end_ms) - 1;
                            let url = format!(
                                "{BASE_URL}?symbol={}&interval={interval}&startTime={from}&endTime={to}&limit={PAGE}",
                                symbol.to_uppercase()
                            );
                            let body =
                                super::http::get_text(&url, &[], super::http::Retry::Transient)
                                    .map_err(|error| format!("Network error: {error}"))?;
                            parse_klines(&body)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err("The candle download stopped.".to_owned()))
                    })
                    .collect()
            });
            for page in pages {
                candles.extend(page?);
            }
        }
        candles.sort_by_key(|candle| candle.open_time);
        candles.dedup_by_key(|candle| candle.open_time);
        if candles.len() > max {
            candles.drain(..candles.len() - max);
        }
        if let Some((seconds, _)) = fold {
            let folded = fold_calendar(&candles, seconds, candles.len() >= max);
            let start = folded.len().saturating_sub(output_max);
            return Ok(folded[start..].to_vec());
        }
        Ok(candles)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let needle = query.trim().to_uppercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = CATALOG.get(download_pairs)?;
        let results: Vec<SearchResult> = pairs
            .iter()
            .filter(|pair| pair.symbol.contains(&needle))
            .map(|pair| {
                SearchResult::new(
                    crate::model::DataSource::Binance,
                    pair.symbol.clone(),
                    pair.symbol.clone(),
                )
                .pair(pair.base.clone(), pair.quote.clone())
            })
            .collect();
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}

/// One tradable market from `exchangeInfo`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    symbol: String,
    base: String,
    quote: String,
}

/// Every market that is trading, downloaded at most once a day.
static CATALOG: Catalog<Pair> = Catalog::new("catalog_binance");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear_catalog() {
    CATALOG.clear_memory();
}

fn download_pairs() -> Result<Vec<Pair>, String> {
    let body = super::http::get_text(
        "https://api.binance.com/api/v3/exchangeInfo",
        &[],
        super::http::Retry::Transient,
    )
    .map_err(|error| format!("Network error: {error}"))?;
    parse_pairs(&body)
}

fn parse_pairs(body: &str) -> Result<Vec<Pair>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    Ok(value
        .get("symbols")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|symbol| symbol.get("status").and_then(Value::as_str) == Some("TRADING"))
        .filter_map(|symbol| {
            Some(Pair {
                symbol: symbol.get("symbol")?.as_str()?.to_owned(),
                base: symbol.get("baseAsset")?.as_str()?.to_owned(),
                quote: symbol.get("quoteAsset")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

fn parse_klines(body: &str) -> Result<Vec<Kline>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    let rows = value
        .as_array()
        .ok_or_else(|| "Data error: expected a candle array".to_owned())?;
    rows.iter().map(parse_row).collect()
}

fn parse_row(row: &Value) -> Result<Kline, String> {
    let value = |index: usize| -> Result<f64, String> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| format!("Data error: invalid field {index}"))
    };
    let time = |index: usize| -> Result<i64, String> {
        row.get(index)
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("Data error: invalid timestamp {index}"))
    };

    Ok(Kline {
        open_time: time(0)?,
        open: value(1)?,
        high: value(2)?,
        low: value(3)?,
        close: value(4)?,
        volume: value(5)?,
        close_time: time(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binance_rows() {
        let body = r#"[[1700000000000,"10.0","12.0","9.0","11.0","42.5",1700003599999]]"#;
        let candles = parse_klines(body).expect("valid response");
        assert_eq!(candles.len(), 1);
        assert_eq!(
            candles[0],
            Kline {
                open_time: 1_700_000_000_000,
                close_time: 1_700_003_599_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }
        );
    }

    #[test]
    fn quarters_and_years_are_folded_from_monthly_klines() {
        assert_eq!(native_interval("1d"), ("1d", None));
        assert_eq!(native_interval("1M"), ("1M", None));
        assert_eq!(native_interval("3M"), ("1M", Some((7_776_000, 3))));
        assert_eq!(native_interval("1Y"), ("1M", Some((31_536_000, 12))));
    }

    #[test]
    fn rejects_error_objects() {
        let error = parse_klines(r#"{"code":-1121,"msg":"Invalid symbol."}"#)
            .expect_err("object must fail");
        assert!(error.contains("expected a candle array"));
    }

    #[test]
    fn the_catalog_keeps_only_trading_markets_with_all_their_fields() {
        let body = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT"},
            {"symbol":"LUNAUSDT","status":"BREAK","baseAsset":"LUNA","quoteAsset":"USDT"},
            {"symbol":"NOQUOTE","status":"TRADING","baseAsset":"X"}
        ]}"#;
        let pairs = parse_pairs(body).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].symbol.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTCUSDT", "BTC", "USDT")
        );
        assert!(parse_pairs("not json").is_err());
        assert!(parse_pairs("{}").expect("no symbols").is_empty());
    }
}
