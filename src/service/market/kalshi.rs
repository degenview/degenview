use super::ports::clock::Clock;
use super::ports::context::MarketContext;
use super::ports::http::{Http, Request};
use crate::model::market::line;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::market::error::ErrorKind;
use crate::service::{
    EventInfo, EventStats, MarketSource, ProviderError, SearchResult, downsample,
    replace_last_price, whole_dollars,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BASE_URL: &str = "https://external-api.kalshi.com/trade-api/v2";

pub struct Kalshi {
    http: Arc<dyn Http>,
    clock: Arc<dyn Clock>,
    /// The series list, which is one unpaginated request (~18 MB), held in memory for 12 hours.
    series: SeriesStore,
}

impl Kalshi {
    pub fn new(context: &MarketContext) -> Self {
        Self {
            http: context.http.clone(),
            clock: context.clock.clone(),
            series: Mutex::new(None),
        }
    }
}

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

/// The candlestick periods Kalshi serves, in minutes.
const PERIODS: [i64; 3] = [1, 60, 1_440];

/// `(period minutes, span seconds)` for the candlestick endpoint at `timeframe`: the longest
/// period that is no longer than a point of the line (`line::window`), and the line's span, cut
/// to the points that period gives a line of a thousand.
fn kalshi_window(timeframe: Timeframe) -> (i64, i64) {
    let window = line::window(timeframe);
    let period = PERIODS
        .into_iter()
        .rev()
        .find(|period| *period <= window.step_minutes())
        .unwrap_or(PERIODS[0]);
    let span_ms = window.span_ms.min(period * 60_000 * line::MAX_POINTS);
    (period, span_ms / 1_000)
}

type SeriesStore = Mutex<Option<(Instant, Vec<Series>)>>;

impl Kalshi {
    /// The series list is one unpaginated request (~18 MB), held in memory for 12 hours.
    fn series_list(&self) -> Result<Vec<Series>, ProviderError> {
        if let Some((fetched, list)) = self.series.lock().ok().and_then(|guard| guard.clone())
            && fetched.elapsed() < Duration::from_secs(12 * 3_600)
        {
            return Ok(list);
        }

        let url = format!("{BASE_URL}/series");
        let body = self.http.fetch(
            DataSource::Kalshi,
            &Request::get(&url).max_body_bytes(32 * 1024 * 1024),
        )?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| ProviderError::malformed(DataSource::Kalshi, error))?;

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

        if let Ok(mut guard) = self.series.lock() {
            *guard = Some((Instant::now(), list.clone()));
        }
        Ok(list)
    }
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
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let (series, market) = symbol
            .split_once('/')
            .ok_or_else(|| ProviderError::new(DataSource::Kalshi, ErrorKind::UnknownSymbol))?;
        let (period, span) = kalshi_window(timeframe);
        let now = self.clock.now_ms() / 1_000;
        let url = format!(
            "{BASE_URL}/series/{series}/markets/{market}/candlesticks?start_ts={}&end_ts={now}&period_interval={period}",
            now - span
        );
        let body = self.http.fetch(DataSource::Kalshi, &Request::get(&url))?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| ProviderError::malformed(DataSource::Kalshi, error))?;

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
            return Err(ProviderError::new(DataSource::Kalshi, ErrorKind::NoData));
        }

        let thinned = downsample(&candles, limit as usize);
        Ok(match self.fetch_current_ask(market) {
            Some(price) => replace_last_price(thinned, price),
            None => thinned,
        })
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
            DataSource::Kalshi,
            ErrorKind::NoTimeRange,
        ))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let q = query.trim().to_owned();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let series = self.series_list()?;
        let matches = rank_series(&series, &q, 8);

        let mut results = Vec::new();
        for matched in matches {
            for event in self.open_events(&matched.ticker)? {
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
                        names: Default::default(),
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

impl Kalshi {
    fn open_events(&self, series: &str) -> Result<Vec<Value>, ProviderError> {
        let url = format!(
            "{BASE_URL}/events?series_ticker={series}&status=open&with_nested_markets=true&limit=20"
        );
        let body = self.http.fetch(DataSource::Kalshi, &Request::get(&url))?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| ProviderError::malformed(DataSource::Kalshi, error))?;
        Ok(value
            .get("events")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .to_vec())
    }

    /// The executable YES ask, matching the market page's headline.
    fn fetch_current_ask(&self, market: &str) -> Option<f64> {
        let url = format!("{BASE_URL}/markets/{market}");
        let body = self
            .http
            .fetch(DataSource::Kalshi, &Request::get(&url))
            .ok()?;
        let value: Value = serde_json::from_str(&body).ok()?;
        let market = &value["market"];
        probability(&market["yes_ask_dollars"])
            .filter(|p| *p > 0.0)
            .or_else(|| probability(&market["last_price_dollars"]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::market::ports::fakes::Rig;
    use crate::service::market::ports::http::{self, Retry};

    fn window_of(timeframe: &str) -> (i64, i64) {
        kalshi_window(Timeframe::parse(timeframe).unwrap())
    }

    fn venue(rig: &Rig) -> Kalshi {
        Kalshi::new(&rig.context())
    }

    const MARKET: &str = "KXBTC/KXBTC-26NOV16-T100";

    #[test]
    fn a_line_is_the_candlesticks_since_the_span_ago_ending_on_the_executable_ask() {
        let rig = Rig::new();
        let now = Rig::NOON / 1_000;
        rig.http.on(
            "/candlesticks?",
            &format!(
                r#"{{"candlesticks":[
                    {{"end_period_ts":{},"price":{{"close_dollars":"0.5600"}}}},
                    {{"end_period_ts":{},"price":{{"previous_dollars":"0.5400"}}}},
                    {{"end_period_ts":{},"price":{{"close_dollars":"0.5500"}}}}
                ]}}"#,
                now - 600,
                now - 1_200,
                now - 1_800
            ),
        );
        rig.http.on(
            "/markets/KXBTC-26NOV16-T100",
            r#"{"market":{"yes_ask_dollars":"0.5800"}}"#,
        );
        let candles = venue(&rig)
            .fetch_klines(MARKET, Timeframe::HOUR_1, 10)
            .expect("a line");
        let (period, span) = kalshi_window(Timeframe::HOUR_1);
        let sent = rig.http.sent();
        assert_eq!(
            sent.iter()
                .map(|sent| sent.url.as_str())
                .collect::<Vec<_>>(),
            [
                format!(
                    "https://external-api.kalshi.com/trade-api/v2/series/KXBTC/markets/KXBTC-26NOV16-T100/candlesticks?start_ts={}&end_ts={now}&period_interval={period}",
                    now - span
                ),
                "https://external-api.kalshi.com/trade-api/v2/markets/KXBTC-26NOV16-T100"
                    .to_owned(),
            ]
        );
        assert!(sent.iter().all(|sent| sent.retry == Retry::Transient));
        // Oldest first, a point with no close taking the one before it, the last on the ask.
        let closes: Vec<f64> = candles.iter().map(|candle| candle.close).collect();
        assert_eq!(closes, [0.55, 0.54, 0.58]);
    }

    #[test]
    fn a_line_without_points_or_a_market_name_is_an_error_before_it_asks_for_more() {
        let rig = Rig::new();
        rig.http.on("/candlesticks?", r#"{"candlesticks":[]}"#);
        let kalshi = venue(&rig);
        assert_eq!(
            kalshi
                .fetch_klines(MARKET, Timeframe::HOUR_1, 10)
                .unwrap_err()
                .kind,
            ErrorKind::NoData
        );
        assert_eq!(rig.http.urls().len(), 1, "no ask is looked up for nothing");
        assert_eq!(
            kalshi
                .fetch_klines("KXBTC", Timeframe::HOUR_1, 10)
                .unwrap_err()
                .kind,
            ErrorKind::UnknownSymbol
        );
        let rig = Rig::new();
        rig.http.fail("/candlesticks?", http::Error::Status(429));
        assert_eq!(
            venue(&rig)
                .fetch_klines(MARKET, Timeframe::HOUR_1, 10)
                .unwrap_err()
                .kind,
            ErrorKind::RateLimited
        );
    }

    #[test]
    fn search_reads_the_series_list_once_and_the_open_events_of_each_match() {
        let rig = Rig::new();
        rig.http.on(
            "/series",
            r#"{"series":[{"ticker":"KXBTC","title":"Bitcoin price","category":"Crypto","tags":["BTC"]},
                          {"ticker":"KXRAIN","title":"Rain in Paris","category":"Weather"}]}"#,
        );
        let events = r#"{"events":[{"series_ticker":"KXBTC","title":"Bitcoin on Nov 16","category":"Crypto","markets":[
            {"ticker":"KXBTC-26NOV16-T100","status":"active","title":"Above 100k?","yes_sub_title":"100k","yes_ask_dollars":"0.5600"}
        ]}]}"#;
        rig.http.on("/events?series_ticker=KXBTC", events);
        rig.http.on("/events?series_ticker=KXBTC", events);
        let kalshi = venue(&rig);
        let found = kalshi.search_tickers("bitcoin").expect("hits");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].full_symbol, MARKET);
        assert_eq!(found[0].price, Some(0.56));
        kalshi.search_tickers("btc").expect("hits");
        let sent = rig.http.sent();
        let series: Vec<_> = sent
            .iter()
            .filter(|sent| sent.url.ends_with("/series"))
            .collect();
        assert_eq!(series.len(), 1, "{:?}", rig.http.urls());
        assert_eq!(series[0].max_body_bytes, Some(32 * 1024 * 1024));
        assert_eq!(sent.len(), 3);
    }

    #[test]
    fn the_period_is_the_longest_that_fits_a_point_of_the_line() {
        assert_eq!(window_of("1h"), (60, 30 * 86_400));
        assert_eq!(window_of("3h"), (60, 30 * 86_400));
        assert_eq!(window_of("1d"), (1_440, 60 * 86_400));
        assert_eq!(window_of("1Y"), (1_440, 364 * 86_400));
        // Between the periods the finer one serves, over fewer hours.
        assert_eq!(window_of("22m"), (1, 1_000 * 60));
        assert_eq!(window_of("1m"), (1, 1_000 * 60));
    }

    #[test]
    fn every_timeframe_has_a_period_and_a_span() {
        for timeframe in Timeframe::PRESETS {
            let (period, span) = kalshi_window(timeframe);
            assert!(PERIODS.contains(&period), "{timeframe}");
            assert!(span >= period * 60, "{timeframe}");
        }
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
