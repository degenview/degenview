use crate::model::Kline;
use crate::service::{
    EventInfo, EventStats, MarketSource, SearchResult, downsample, effective_span_days,
    replace_last_price, whole_dollars,
};
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const BASE_URL: &str = "https://external-api.kalshi.com/trade-api/v2";

pub struct Kalshi;

pub const KALSHI: Kalshi = Kalshi;

#[derive(Clone)]
struct Series {
    ticker: String,
    title: String,
    category: String,
    tags: Vec<String>,
}

/// Kalshi prices are fixed-point strings ("0.5600") in 0…1.
fn probability(raw: &Value) -> Option<f64> {
    raw.as_str()
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
}

/// (period minutes, span seconds) for the candlestick endpoint.
fn kalshi_window(interval: &str) -> (i64, i64) {
    let span = effective_span_days(interval);
    let period = if span <= 2 {
        1
    } else if span <= 120 {
        60
    } else {
        1_440
    };
    (period, span * 86_400)
}

type SeriesStore = Mutex<Option<(Instant, Vec<Series>)>>;

fn series_cache() -> &'static SeriesStore {
    static CACHE: OnceLock<SeriesStore> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// The series list is one unpaginated request (~18 MB), held in memory for 12 hours.
fn series_list() -> Result<Vec<Series>, String> {
    if let Some((fetched, list)) = series_cache().lock().ok().and_then(|guard| guard.clone())
        && fetched.elapsed() < Duration::from_secs(12 * 3_600)
    {
        return Ok(list);
    }

    let url = format!("{BASE_URL}/series");
    let body = crate::service::net::http::get_text_with_limit(
        &url,
        &[],
        crate::service::net::http::Retry::Transient,
        32 * 1024 * 1024,
    )
    .map_err(|error| format!("Network error: {error}"))?;
    let value: Value =
        serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;

    let list: Vec<Series> = value
        .get("series")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|series| {
            let ticker = series.get("ticker").and_then(Value::as_str)?.to_owned();
            let title = series
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if title.is_empty() {
                return None;
            }
            let category = series
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let tags = series
                .get("tags")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            Some(Series {
                ticker,
                title,
                category,
                tags,
            })
        })
        .collect();

    if let Ok(mut guard) = series_cache().lock() {
        *guard = Some((Instant::now(), list.clone()));
    }
    Ok(list)
}

fn rank_series(series: &[Series], query: &str, limit: usize) -> Vec<Series> {
    let words: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let phrase = words.join(" ");

    let mut scored: Vec<(i64, Series)> = Vec::new();
    for item in series {
        let title = item.title.to_lowercase();
        let ticker = item.ticker.to_lowercase();
        let mut haystack = format!("{title} {ticker} {}", item.category.to_lowercase());
        for tag in &item.tags {
            haystack.push(' ');
            haystack.push_str(&tag.to_lowercase());
        }
        if !words.iter().all(|word| haystack.contains(word)) {
            continue;
        }
        let score = if title.starts_with(&phrase) {
            0
        } else if title.contains(&format!(" {phrase}")) {
            1
        } else if ticker.starts_with(&phrase) || ticker.starts_with(&format!("kx{phrase}")) {
            2
        } else if title.contains(&phrase) {
            3
        } else {
            4
        };
        scored.push((score, item.clone()));
    }
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.title.len().cmp(&b.1.title.len()))
            .then_with(|| a.1.ticker.cmp(&b.1.ticker))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, series)| series)
        .collect()
}

impl MarketSource for Kalshi {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        let (series, market) = symbol
            .split_once('/')
            .ok_or_else(|| "Unrecognized Kalshi market id".to_owned())?;
        let (period, span) = kalshi_window(interval);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "System clock is before the epoch".to_owned())?
            .as_secs() as i64;
        let url = format!(
            "{BASE_URL}/series/{series}/markets/{market}/candlesticks?start_ts={}&end_ts={now}&period_interval={period}",
            now - span
        );
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Transient,
        )
        .map_err(|error| format!("Network error: {error}"))?;
        let value: Value =
            serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;

        let mut rows: Vec<(i64, &Value)> = value
            .get("candlesticks")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|candle| {
                candle
                    .get("end_period_ts")
                    .and_then(Value::as_f64)
                    .map(|t| ((t * 1_000.0) as i64, candle))
            })
            .collect();
        rows.sort_by_key(|(t, _)| *t);

        let mut last_known: Option<f64> = None;
        let mut candles = Vec::new();
        for (t, candle) in rows {
            let price = probability(&candle["price"]["close_dollars"])
                .or_else(|| probability(&candle["price"]["previous_dollars"]))
                .or(last_known);
            let Some(price) = price else {
                continue;
            };
            last_known = Some(price);
            candles.push(Kline {
                open_time: t,
                close_time: t,
                open: price,
                high: price,
                low: price,
                close: price,
                volume: 0.0,
            });
        }
        if candles.is_empty() {
            return Err("No price history for this market in this range".to_owned());
        }

        let thinned = downsample(&candles, limit as usize);
        Ok(match fetch_current_ask(market) {
            Some(price) => replace_last_price(thinned, price),
            None => thinned,
        })
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let q = query.trim().to_owned();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let series = series_list()?;
        let matches = rank_series(&series, &q, 8);

        let mut results = Vec::new();
        for matched in matches {
            for event in open_events(&matched.ticker)? {
                let series_ticker = event
                    .get("series_ticker")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| matched.ticker.clone());
                let event_title = event
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.is_empty())
                    .unwrap_or(&matched.title)
                    .to_owned();
                let markets = event
                    .get("markets")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let tradable: Vec<&Value> = markets
                    .iter()
                    .filter(|market| market.get("status").and_then(Value::as_str) == Some("active"))
                    .collect();
                let is_single = tradable.len() == 1;
                let stats = event_stats(&event, &tradable);

                for market in tradable {
                    let Some(market_ticker) = market.get("ticker").and_then(Value::as_str) else {
                        continue;
                    };
                    let short = market
                        .get("yes_sub_title")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());
                    let label = if is_single {
                        market.get("title").and_then(Value::as_str).or(short)
                    } else {
                        short.or_else(|| market.get("title").and_then(Value::as_str))
                    }
                    .unwrap_or_default();
                    if label.is_empty() {
                        continue;
                    }
                    let ask = probability(&market["yes_ask_dollars"]).filter(|p| *p > 0.0);
                    let price = ask.or_else(|| probability(&market["last_price_dollars"]));
                    results.push(SearchResult {
                        symbol: label.to_owned(),
                        full_symbol: format!("{series_ticker}/{market_ticker}"),
                        source: crate::model::DataSource::Kalshi,
                        price,
                        event: Some(EventInfo {
                            title: event_title.clone(),
                            image_url: None,
                            series: Vec::new(),
                            stats: Box::new(stats.clone()),
                        }),
                        base: None,
                        quote: None,
                    });
                }
            }
        }
        Ok(results)
    }
}

/// What Kalshi says about an event, summed over its open markets: the contracts traded and held,
/// and the first close.
fn event_stats(event: &Value, markets: &[&Value]) -> EventStats {
    let total = |key: &str| {
        markets
            .iter()
            .filter_map(|market| whole_dollars(&market[key]))
            .reduce(|sum, each| sum + each)
    };
    EventStats {
        volume: total("volume_fp"),
        volume_24h: total("volume_24h_fp"),
        liquidity: None,
        open_interest: total("open_interest_fp"),
        ends_at_ms: markets
            .iter()
            .filter_map(|market| market.get("close_time").and_then(Value::as_str))
            .filter_map(crate::format::parse_timestamp)
            .min(),
        category: event
            .get("category")
            .and_then(Value::as_str)
            .filter(|category| !category.is_empty())
            .map(str::to_owned),
        about: None,
    }
}

fn open_events(series: &str) -> Result<Vec<Value>, String> {
    let url = format!(
        "{BASE_URL}/events?series_ticker={series}&status=open&with_nested_markets=true&limit=20"
    );
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .map_err(|error| format!("Network error: {error}"))?;
    let value: Value =
        serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;
    Ok(value
        .get("events")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .to_vec())
}

/// The executable YES ask, matching the market page's headline.
fn fetch_current_ask(market: &str) -> Option<f64> {
    let url = format!("{BASE_URL}/markets/{market}");
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .ok()?;
    let value: Value = serde_json::from_str(&body).ok()?;
    let market = &value["market"];
    probability(&market["yes_ask_dollars"])
        .filter(|p| *p > 0.0)
        .or_else(|| probability(&market["last_price_dollars"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_uses_period_steps() {
        assert_eq!(kalshi_window("1h"), (60, 30 * 86_400));
        assert_eq!(kalshi_window("1d"), (60, 60 * 86_400));
        assert_eq!(kalshi_window("1Y"), (1_440, 364 * 86_400));
    }

    #[test]
    fn ranks_series_by_prefix_then_length() {
        let series = vec![
            Series {
                ticker: "KXFEDDEC26".into(),
                title: "Fed decision".into(),
                category: "Economics".into(),
                tags: vec![],
            },
            Series {
                ticker: "KXBTCDEC26".into(),
                title: "Bitcoin above $100k".into(),
                category: "Crypto".into(),
                tags: vec![],
            },
        ];
        let ranked = rank_series(&series, "fed", 8);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].ticker, "KXFEDDEC26");
    }

    #[test]
    fn an_events_figures_add_up_over_its_open_markets_and_end_at_the_first_close() {
        let event = serde_json::json!({"category": "Economics"});
        let a = serde_json::json!({"volume_fp": "100.5", "volume_24h_fp": "10", "open_interest_fp": "40",
            "close_time": "2026-12-10T04:59:00Z"});
        let b = serde_json::json!({"volume_fp": "200", "close_time": "2026-11-30T00:00:00Z"});
        let stats = event_stats(&event, &[&a, &b]);
        assert_eq!(stats.volume, Some(301));
        assert_eq!(stats.volume_24h, Some(10));
        assert_eq!(stats.open_interest, Some(40));
        assert_eq!(stats.liquidity, None);
        assert_eq!(stats.category.as_deref(), Some("Economics"));
        assert_eq!(
            stats.ends_at_ms,
            crate::format::parse_timestamp("2026-11-30 00:00")
        );
    }
}
