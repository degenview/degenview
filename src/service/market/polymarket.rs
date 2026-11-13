use crate::model::Kline;
use crate::service::{
    EventInfo, EventStats, MarketSource, SearchResult, downsample, effective_span_days,
    line_chart_point_count, replace_last_price, whole_dollars,
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

/// (interval token, fidelity) for the CLOB `/prices-history` endpoint.
fn pm_window(interval: &str) -> (String, i64) {
    let span = effective_span_days(interval);
    let needed = line_chart_point_count(interval) * 2;

    let token = if span <= 1 {
        "1h"
    } else if span <= 2 {
        "1d"
    } else if span <= 8 {
        "1w"
    } else if span <= 31 {
        "1m"
    } else {
        "max"
    };
    let interval_minutes: i64 = match token {
        "max" | "1m" => 43_200,
        "1w" => 10_080,
        "1d" => 1_440,
        _ => 60,
    };
    let fidelity = (interval_minutes / needed as i64).max(1);
    (token.to_owned(), fidelity)
}

impl MarketSource for Polymarket {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        let (window, fidelity) = pm_window(interval);
        let url = format!(
            "{CLOB_BASE}/prices-history?market={symbol}&interval={window}&fidelity={fidelity}"
        );
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Transient,
        )
        .map_err(|error| format!("Network error: {error}"))?;
        let value: Value =
            serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;

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
            return Err("No price history for this market in this range".to_owned());
        }

        let thinned = downsample(&candles, limit as usize);
        Ok(match fetch_current_ask(symbol) {
            Some(price) => replace_last_price(thinned, price),
            None => thinned,
        })
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let q = query.trim().to_owned();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url =
            format!("{GAMMA_BASE}/public-search?q={q}&limit_per_type=10&events_status=active");
        let body = crate::service::net::http::get_text(
            &url,
            &[],
            crate::service::net::http::Retry::Transient,
        )
        .map_err(|error| format!("Network error: {error}"))?;
        let value: Value =
            serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;

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
pub fn trending(limit: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "{GAMMA_BASE}/events?active=true&closed=false&order=volume24hr&ascending=false&limit={limit}"
    );
    let body =
        crate::service::net::http::get_text(&url, &[], crate::service::net::http::Retry::Transient)
            .map_err(|error| format!("Network error: {error}"))?;
    let events: Vec<Value> =
        serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;
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
    fn artwork_lookup_refuses_ids_that_are_not_tokens() {
        assert_eq!(artwork_for_token(""), Ok(None));
        assert_eq!(artwork_for_token("1&limit=500"), Ok(None));
    }

    #[test]
    fn window_matches_span() {
        assert_eq!(
            pm_window("1h"),
            ("1m".to_owned(), 60),
            "a month of hourly points"
        );
        assert_eq!(pm_window("1d").0, "max");
        assert_eq!(pm_window("1w").0, "max");
        assert_eq!(pm_window("1M").0, "max");
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
