use crate::model::market::line;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::market::error::ErrorKind;
use crate::service::net::url::encode_component;
use crate::service::{
    EventInfo, EventStats, MarketSource, ProviderError, SearchResult, downsample,
    replace_last_price, whole_dollars,
};
use serde_json::Value;

const GAMMA_BASE: &str = "https://gamma-api.polymarket.com";
const CLOB_BASE: &str = "https://clob.polymarket.com";

pub struct Polymarket;

pub const POLYMARKET: Polymarket = Polymarket;

/// Gamma serializes `outcomes`/`outcomePrices`/`clobTokenIds` as JSON strings that
/// themselves contain arrays; they need a second decode pass.
fn decode_string_array(raw: &Value) -> Vec<String> {
    let text = raw.as_str().unwrap_or("");
    serde_json::from_str::<Vec<String>>(text).unwrap_or_default()
}

/// A price in 0…1, either as a number or a string.
fn probability(raw: &Value) -> Option<f64> {
    raw.as_f64()
        .or_else(|| raw.as_str().and_then(|text| text.parse().ok()))
        .filter(|value| (0.0..=1.0).contains(value))
}

/// The history windows the CLOB `/prices-history` endpoint takes, shortest first: its token, the
/// minutes it covers and the finest `fidelity` (minutes between points) it allows. `max` is the
/// whole history, but only for a fidelity of twelve hours or more; finer, it is the last month.
const WINDOWS: [(&str, i64, i64); 6] = [
    ("1h", 60, 1),
    ("6h", 360, 1),
    ("1d", 1_440, 1),
    ("1w", 10_080, 5),
    ("1m", 43_200, 10),
    ("max", i64::MAX, 720),
];

/// What a request for a line asks the endpoint, and how far back the line then reaches.
#[derive(Debug, PartialEq)]
struct PmWindow {
    token: &'static str,
    fidelity: i64,
    span_ms: i64,
}

/// The window of a line at `timeframe`: a point every timeframe (`line::window` keeps that to a
/// week), in the shortest window that holds the line's span and allows that fidelity. When none
/// both holds the span and allows the fidelity, the longest that allows it, and the line is
/// shorter.
/// The search address for a trimmed query.
fn search_url(query: &str) -> String {
    format!(
        "{GAMMA_BASE}/public-search?q={}&limit_per_type=10&events_status=active",
        encode_component(query)
    )
}

fn pm_window(timeframe: Timeframe) -> PmWindow {
    let line = line::window(timeframe);
    let step = line.step_minutes();
    let allowed = || WINDOWS.iter().filter(|(_, _, floor)| *floor <= step);
    let (token, minutes, _) = allowed()
        .find(|(_, minutes, _)| *minutes >= line.span_ms / 60_000)
        .or_else(|| allowed().next_back())
        .unwrap_or(&WINDOWS[0]);
    PmWindow {
        token,
        fidelity: step,
        span_ms: line.span_ms.min(minutes.saturating_mul(60_000)),
    }
}

impl MarketSource for Polymarket {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let window = pm_window(timeframe);
        let url = format!(
            "{CLOB_BASE}/prices-history?market={symbol}&interval={}&fidelity={}",
            window.token, window.fidelity
        );
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Transient,
        )
        .map_err(|error| ProviderError::http(DataSource::Polymarket, error))?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| ProviderError::malformed(DataSource::Polymarket, error))?;

        let mut points: Vec<(i64, f64)> = value
            .get("history")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|point| {
                let t = point.get("t").and_then(Value::as_f64)?;
                let p = point.get("p").and_then(Value::as_f64)?;
                Some(((t * 1_000.0) as i64, p))
            })
            .collect();
        points.sort_by_key(|(t, _)| *t);
        // A window can reach back past the line (`max` is the whole history).
        let since = crate::clock::now_ms() - window.span_ms;
        points.retain(|(t, _)| *t >= since);

        let candles: Vec<Kline> = points
            .into_iter()
            .map(|(t, p)| Kline {
                open_time: t,
                close_time: t,
                open: p,
                high: p,
                low: p,
                close: p,
                volume: 0.0,
            })
            .collect();
        if candles.is_empty() {
            return Err(ProviderError::new(
                DataSource::Polymarket,
                ErrorKind::NoData,
            ));
        }

        let thinned = downsample(&candles, limit as usize);
        Ok(match fetch_current_ask(symbol) {
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
            DataSource::Polymarket,
            ErrorKind::NoTimeRange,
        ))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        let q = query.trim().to_owned();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = search_url(&q);
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Transient,
        )
        .map_err(|error| ProviderError::http(DataSource::Polymarket, error))?;
        let value: Value = serde_json::from_str(&body)
            .map_err(|error| ProviderError::malformed(DataSource::Polymarket, error))?;

        let events = value
            .get("events")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let results = events.iter().flat_map(event_results).collect();
        Ok(results)
    }
}

/// The first non-empty artwork URL among a Gamma object's `icon` and `image`.
fn artwork(value: &Value) -> Option<String> {
    ["icon", "image"]
        .iter()
        .filter_map(|key| value.get(*key).and_then(Value::as_str))
        .find(|url| !url.is_empty())
        .map(str::to_owned)
}

/// The artwork of the market behind a YES token, for rows and charts saved without any. `Err`
/// means the lookup failed (retry soon); `Ok(None)` means the market has no artwork.
pub fn artwork_for_token(token_id: &str) -> Result<Option<String>, ()> {
    if token_id.is_empty() || !token_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Ok(None);
    }
    let url = format!("{GAMMA_BASE}/markets?clob_token_ids={token_id}");
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .map_err(|_| ())?;
    let markets: Vec<Value> = serde_json::from_str(&body).map_err(|_| ())?;
    Ok(markets.first().and_then(artwork))
}

/// The open markets of one Gamma event, one result per tradable YES token.
fn event_results(event: &Value) -> Vec<SearchResult> {
    let event_title = event
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let event_artwork = artwork(event);
    let stats = event_stats(event);
    let mut results = Vec::new();
    for market in event
        .get("markets")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        if market.get("closed").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(token_id) = decode_string_array(&market["clobTokenIds"])
            .into_iter()
            .next()
        else {
            continue;
        };
        let label = market
            .get("groupItemTitle")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| market.get("question").and_then(Value::as_str))
            .unwrap_or_default();
        if label.is_empty() {
            continue;
        }
        let price = probability(&market["bestAsk"]).or_else(|| {
            decode_string_array(&market["outcomePrices"])
                .into_iter()
                .next()
                .and_then(|text| text.parse().ok())
                .filter(|value| (0.0..=1.0).contains(value))
        });
        results.push(SearchResult {
            symbol: label.to_owned(),
            full_symbol: token_id,
            source: crate::model::DataSource::Polymarket,
            price,
            event: Some(EventInfo {
                title: event_title.to_owned(),
                image_url: artwork(market).or_else(|| event_artwork.clone()),
                series: Vec::new(),
                stats: Box::new(stats.clone()),
            }),
            base: None,
            quote: None,
            names: Default::default(),
        });
    }
    results
}

/// What Gamma says about an event: the money traded, when it ends and how it resolves.
fn event_stats(event: &Value) -> EventStats {
    let text = |key: &str| {
        event
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    EventStats {
        volume: whole_dollars(&event["volume"]),
        volume_24h: whole_dollars(&event["volume24hr"]),
        liquidity: whole_dollars(&event["liquidity"]),
        open_interest: whole_dollars(&event["openInterest"]),
        ends_at_ms: text("endDate").and_then(|date| crate::format::parse_timestamp(&date)),
        category: None,
        about: text("description"),
    }
}

/// The most-traded live events of the last 24 hours, flattened like search results.
pub fn trending(limit: usize) -> Result<Vec<SearchResult>, ProviderError> {
    let url = format!(
        "{GAMMA_BASE}/events?active=true&closed=false&order=volume24hr&ascending=false&limit={limit}"
    );
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .map_err(|error| ProviderError::http(DataSource::Polymarket, error))?;
    let events: Vec<Value> = serde_json::from_str(&body)
        .map_err(|error| ProviderError::malformed(DataSource::Polymarket, error))?;
    Ok(events.iter().flat_map(event_results).collect())
}

/// The executable YES ask, matching the market page's headline.
fn fetch_current_ask(token_id: &str) -> Option<f64> {
    let url = format!("{CLOB_BASE}/price?token_id={token_id}&side=SELL");
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .ok()?;
    let value: Value = serde_json::from_str(&body).ok()?;
    probability(&value["price"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_outside_ascii_is_escaped_in_the_search_address() {
        assert_eq!(
            search_url("比特币 & co"),
            format!(
                "{GAMMA_BASE}/public-search?q=%E6%AF%94%E7%89%B9%E5%B8%81%20%26%20co&limit_per_type=10&events_status=active"
            )
        );
    }

    #[test]
    fn artwork_lookup_refuses_ids_that_are_not_tokens() {
        assert_eq!(artwork_for_token(""), Ok(None));
        assert_eq!(artwork_for_token("1&limit=500"), Ok(None));
    }

    fn window_of(timeframe: &str) -> (&'static str, i64) {
        let window = pm_window(Timeframe::parse(timeframe).unwrap());
        (window.token, window.fidelity)
    }

    #[test]
    fn the_window_holds_the_span_and_the_fidelity_is_the_timeframe() {
        assert_eq!(window_of("1h"), ("1m", 60), "a month of hourly points");
        assert_eq!(window_of("3h"), ("1m", 180));
        assert_eq!(window_of("12h"), ("1m", 720));
        assert_eq!(window_of("1d"), ("max", 1_440));
        assert_eq!(window_of("2d"), ("max", 2_880));
        // A point is never a week and more apart.
        assert_eq!(window_of("1w"), ("max", 10_080));
        assert_eq!(window_of("1Y"), ("max", 10_080));
    }

    #[test]
    fn a_short_timeframe_reaches_back_hours_or_days_at_its_own_fidelity() {
        assert_eq!(window_of("1m"), ("1d", 1));
        assert_eq!(window_of("5m"), ("1w", 5));
        // The month window allows no finer than ten minutes: a 15-minute line fits it.
        assert_eq!(window_of("15m"), ("1m", 15));
    }

    #[test]
    fn a_fidelity_the_long_window_forbids_gets_a_shorter_line() {
        // Two-minute points are not allowed over a week, so the line stops at a day.
        let window = pm_window(Timeframe::parse("2m").unwrap());
        assert_eq!((window.token, window.fidelity), ("1d", 2));
        assert_eq!(window.span_ms, 86_400_000);
    }

    #[test]
    fn every_timeframe_has_a_window_that_allows_its_fidelity() {
        for timeframe in Timeframe::PRESETS {
            let window = pm_window(timeframe);
            let floor = WINDOWS
                .iter()
                .find(|(token, ..)| *token == window.token)
                .map(|(_, _, floor)| *floor);
            assert!(
                floor.is_some_and(|floor| window.fidelity >= floor),
                "{timeframe}"
            );
            assert!(window.span_ms > 0);
        }
    }

    #[test]
    fn an_event_lists_its_open_markets_with_yes_tokens() {
        let event = serde_json::json!({"markets": [
            {"groupItemTitle": "Yes wins", "clobTokenIds": "[\"11\",\"12\"]", "bestAsk": 0.62},
            {"question": "Closed one", "closed": true, "clobTokenIds": "[\"21\"]"},
            {"question": "No tokens"},
        ]});
        let results = super::event_results(&event);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol, "Yes wins");
        assert_eq!(results[0].full_symbol, "11");
        assert_eq!(results[0].price, Some(0.62));
    }

    #[test]
    fn a_choice_carries_its_event_title_and_nearest_artwork() {
        let event = serde_json::json!({
            "title": "Fed decision",
            "icon": "https://img/event.png",
            "markets": [
                {"groupItemTitle": "Cut", "clobTokenIds": "[\"1\"]", "image": "https://img/cut.png"},
                {"groupItemTitle": "Hold", "clobTokenIds": "[\"2\"]", "icon": ""},
            ],
        });
        let results = super::event_results(&event);
        let event_of = |index: usize| results[index].event.clone().expect("event");
        assert_eq!(event_of(0).title, "Fed decision");
        assert_eq!(
            event_of(0).image_url.as_deref(),
            Some("https://img/cut.png")
        );
        assert_eq!(
            event_of(1).image_url.as_deref(),
            Some("https://img/event.png")
        );
    }

    #[test]
    fn decodes_string_arrays() {
        let value: Value = serde_json::from_str(r#""[\"Yes\",\"No\"]""#).expect("string");
        assert_eq!(decode_string_array(&value), vec!["Yes", "No"]);
    }

    #[test]
    fn an_event_carries_what_gamma_says_about_it() {
        let event = serde_json::json!({
            "title": "Fed Decision",
            "volume": 31625166.69, "volume24hr": "4615987.7", "liquidity": 1834558.05,
            "openInterest": 5149829.2, "endDate": "2026-12-10T04:59:00Z",
            "description": "  How the FED decides.  ",
            "markets": [{"groupItemTitle": "Hold", "clobTokenIds": "[\"11\"]", "bestAsk": 0.5}],
        });
        let results = super::event_results(&event);
        let stats = &results[0].event.as_ref().expect("event").stats;
        assert_eq!(stats.volume, Some(31_625_167));
        assert_eq!(stats.volume_24h, Some(4_615_988));
        assert_eq!(stats.liquidity, Some(1_834_558));
        assert_eq!(stats.open_interest, Some(5_149_829));
        assert_eq!(stats.about.as_deref(), Some("How the FED decides."));
        assert_eq!(
            stats.ends_at_ms,
            crate::format::parse_timestamp("2026-12-10 04:59")
        );
        // An event that says nothing has nothing.
        let bare = super::event_results(&serde_json::json!({"markets": [
            {"groupItemTitle": "Yes", "clobTokenIds": "[\"1\"]"}
        ]}));
        assert!(bare[0].event.as_ref().expect("event").stats.is_empty());
    }
}
