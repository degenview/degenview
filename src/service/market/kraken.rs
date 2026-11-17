//! Kraken spot candles and its market list. Public endpoints, no key.
//!
//! `OHLC` answers the newest 720 candles of an interval and cannot page back: `since` only moves
//! the end this side of them. Rows are `[open time s, open, high, low, close, vwap, volume,
//! trades]`, the last one still forming. Failures arrive as HTTP 200 with an `error` list.

use super::candles::plan;
use super::catalog::Catalog;
use super::search_rank::normalize;
use crate::model::market::bucket::next_bucket_start_ms;
use crate::model::market::natives::KRAKEN_BARS;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::net::{http, rate_limit};
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const BASE_URL: &str = "https://api.kraken.com/0/public";

pub struct Kraken;

pub const KRAKEN: Kraken = Kraken;

/// Kraken's `interval` (minutes) for a candle length it serves itself.
pub(crate) fn interval(timeframe: Timeframe) -> Option<u32> {
    DataSource::Kraken
        .natives()
        .contains(&timeframe)
        .then(|| (timeframe.nominal_ms() / 60_000) as u32)
}

/// Kraken's name of a market, `BTC/USD`, which both its REST API and its websocket take.
/// `BTC-USD`, a bare `BTC` (USD) and the legacy `XBT/USD` are accepted.
pub(crate) fn pair(symbol: &str) -> String {
    InstrumentId::new(DataSource::Kraken, symbol).api_symbol()
}

impl MarketSource for Kraken {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Kraken, timeframe)?;
        let requested = plan.base_count(limit as usize).min(KRAKEN_BARS);
        let candles = fetch_ohlc(symbol, plan.base(), Some(requested))?;
        Ok(plan.shape(newest(candles, requested), requested, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Kraken, timeframe)?;
        let requested = plan.base_count(max).min(KRAKEN_BARS);
        let mut candles = fetch_ohlc(symbol, plan.base(), None)?;
        candles.retain(|candle| candle.open_time >= start_ms && candle.open_time < end_ms);
        Ok(plan.shape(newest(candles, requested), requested, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC/USD`, `btc-usd`, `BTCUSD` and the legacy `XBTUSD` are one query.
        let needle = alias(&normalize(query));
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = CATALOG.get(download_pairs)?;
        let results = matching_pairs(&pairs, &needle);
        Ok(rank_results(results, &needle)
            .into_iter()
            .take(20)
            .collect())
    }
}

/// `text` with Kraken's legacy asset names in the common ones.
fn alias(text: &str) -> String {
    text.replace("XBT", "BTC").replace("XDG", "DOGE")
}

/// The last `count` candles.
fn newest(mut candles: Vec<Kline>, count: usize) -> Vec<Kline> {
    if candles.len() > count {
        candles.drain(..candles.len() - count);
    }
    candles
}

/// The candles Kraken holds for `symbol`, oldest first. `recent` asks only for that many of the
/// newest (and a little more), which keeps a tail refresh small; `None` asks for all 720.
fn fetch_ohlc(
    symbol: &str,
    base: Timeframe,
    recent: Option<usize>,
) -> Result<Vec<Kline>, ProviderError> {
    let minutes =
        interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Kraken, base))?;
    let mut url = format!("{BASE_URL}/OHLC?pair={}&interval={minutes}", pair(symbol));
    if let Some(count) = recent.filter(|count| *count < KRAKEN_BARS) {
        let since = crate::service::now_ms() / 1_000 - (count as i64 + 1) * i64::from(minutes) * 60;
        url.push_str(&format!("&since={since}"));
    }
    parse_ohlc(&get(&url)?, base)
}

/// A request on Kraken's one-a-second schedule.
fn get(url: &str) -> Result<String, ProviderError> {
    rate_limit::kraken().wait_for_slot();
    Ok(http::get_text(url, &[], http::Retry::Transient)?)
}

/// Whether a Kraken error code says to slow down: `EAPI:Rate limit exceeded`,
/// `EGeneral:Too many requests`, `EService:Throttled`, `EGeneral:Temporary lockout`.
pub(crate) fn is_rate_limit(code: &str) -> bool {
    code.starts_with("EAPI:Rate limit")
        || code.starts_with("EService:Throttled")
        || code.starts_with("EGeneral:Temporary lockout")
        || code.starts_with("EGeneral:Too many requests")
}

/// The `result` of an answer, or the provider's refusal.
pub(crate) fn result_of(body: &str) -> Result<Value, ProviderError> {
    let mut value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    if let Some(error) = value
        .get("error")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(Value::as_str)
    {
        return Err(refusal(error));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| ProviderError::data("expected a result"))
}

/// What a Kraken error code (`EAPI:Rate limit exceeded`, `EQuery:Unknown asset pair`) means.
fn refusal(code: &str) -> ProviderError {
    if is_rate_limit(code) {
        return ProviderError::Network(http::Error::Status(429));
    }
    let text = code.split_once(':').map_or(code, |(_, text)| text);
    ProviderError::other(format!("Kraken: {text}"))
}

/// Candles of length `base`, oldest first.
fn parse_ohlc(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let result = result_of(body)?;
    let rows = result
        .as_object()
        .and_then(|result| result.iter().find(|(key, _)| key.as_str() != "last"))
        .and_then(|(_, rows)| rows.as_array())
        .ok_or_else(|| ProviderError::data("expected a candle array"))?;
    let mut candles = rows
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let number = |index: usize| -> Result<f64, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| ProviderError::data(format!("invalid field {index}")))
    };
    let open_time = row
        .get(0)
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::data("invalid timestamp"))?
        * 1_000;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(1)?,
        high: number(2)?,
        low: number(3)?,
        close: number(4)?,
        volume: number(6)?,
    })
}

/// The listed markets whose name, without its slash, contains `needle` (letters and digits,
/// upper case).
fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| normalize(&pair.id).contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Kraken, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable market from `AssetPairs`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    id: String,
    base: String,
    quote: String,
}

/// Every market that is online, downloaded at most once a day.
static CATALOG: Catalog<Pair> = Catalog::new("catalog_kraken");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get(&format!("{BASE_URL}/AssetPairs"))?)
}

/// `wsname` is `XBT/USD`: the market as `BTC/USD`, which is how every Kraken API spells it.
fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let result = result_of(body)?;
    let pairs = result
        .as_object()
        .ok_or_else(|| ProviderError::data("expected a pair list"))?;
    Ok(pairs
        .values()
        .filter(|pair| pair.get("status").and_then(Value::as_str) == Some("online"))
        .filter_map(|pair| {
            let id = self::pair(pair.get("wsname")?.as_str()?);
            let (base, quote) = id.split_once('/')?;
            let (base, quote) = (base.to_owned(), quote.to_owned());
            (!base.is_empty() && !quote.is_empty() && !quote.contains('/')).then_some(Pair {
                id,
                base,
                quote,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_has_its_interval_in_minutes_and_nothing_else_does() {
        let minutes: Vec<u32> = DataSource::Kraken
            .natives()
            .iter()
            .map(|native| interval(*native).expect("a native has an interval"))
            .collect();
        assert_eq!(minutes, [1, 5, 15, 30, 60, 240, 1440]);
        for other in ["2m", "3m", "2h", "8h", "1W", "1M", "1Y"] {
            assert_eq!(interval(Timeframe::parse(other).unwrap()), None, "{other}");
        }
    }

    fn base_of(timeframe: &str) -> (u32, usize) {
        let plan = plan(DataSource::Kraken, Timeframe::parse(timeframe).unwrap()).unwrap();
        (interval(plan.base()).unwrap(), plan.base_count(500))
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("1h"), (60, 500));
        assert_eq!(base_of("22m"), (1, 501 * 22));
        assert_eq!(base_of("3h"), (60, 501 * 3));
        assert_eq!(base_of("12h"), (240, 501 * 3));
        // Kraken's own week starts on a Thursday and its month is not offered.
        assert_eq!(base_of("1W"), (1440, 501 * 7));
        assert_eq!(base_of("1M"), (1440, 501 * 31));
    }

    #[test]
    fn a_request_is_capped_at_the_candles_kraken_answers_with() {
        let plan = plan(DataSource::Kraken, Timeframe::parse("1W").unwrap()).unwrap();
        assert!(plan.base_count(500).min(KRAKEN_BARS) == KRAKEN_BARS);
        // A tail refresh stays small.
        assert!(plan.base_count(crate::service::cache::TAIL_LIMIT as usize) < KRAKEN_BARS);
    }

    #[test]
    fn symbols_are_krakens_common_names() {
        assert_eq!(pair("btc/usd"), "BTC/USD");
        assert_eq!(pair("XBT/USD"), "BTC/USD");
        assert_eq!(pair("xdg-eur"), "DOGE/EUR");
        assert_eq!(pair("sol"), "SOL/USD");
    }

    const OHLC: &str = r#"{"error":[],"result":{"BTC/USD":[
        [1699999200,"10.0","12.0","9.0","11.0","10.5","42.5",7],
        [1700002800,"11.0","13.0","10.0","12.0","11.5","5.5",3]
    ],"last":1699999200}}"#;

    #[test]
    fn rows_are_candles_with_the_close_of_their_bucket_and_the_last_key_is_not_a_pair() {
        let candles = parse_ohlc(OHLC, Timeframe::HOUR_1).expect("valid response");
        assert_eq!(candles.len(), 2);
        assert_eq!(
            candles[0],
            Kline {
                open_time: 1_699_999_200_000,
                close_time: 1_700_002_799_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }
        );
        assert_eq!(candles[1].open_time, 1_700_002_800_000);
    }

    #[test]
    fn a_refusal_is_krakens_message_and_a_rate_limit_is_one() {
        let unknown = r#"{"error":["EQuery:Unknown asset pair"]}"#;
        assert_eq!(
            parse_ohlc(unknown, Timeframe::HOUR_1),
            Err(ProviderError::other("Kraken: Unknown asset pair"))
        );
        for code in [
            "EAPI:Rate limit exceeded",
            "EGeneral:Too many requests",
            "EService:Throttled: 1700000000",
        ] {
            let limited = format!(r#"{{"error":["{code}"]}}"#);
            let error = parse_ohlc(&limited, Timeframe::HOUR_1).unwrap_err();
            assert!(error.is_rate_limited(), "{code}: {error}");
        }
        assert!(parse_ohlc("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_ohlc(r#"{"error":[]}"#, Timeframe::HOUR_1).is_err());
        assert!(parse_ohlc(r#"{"error":[],"result":{"last":1}}"#, Timeframe::HOUR_1).is_err());
        let bad_field =
            r#"{"error":[],"result":{"BTC/USD":[[1699999200,"x","1","1","1","1","1",1]]}}"#;
        assert!(parse_ohlc(bad_field, Timeframe::HOUR_1).is_err());
    }

    #[test]
    fn a_range_keeps_the_candles_that_open_in_it() {
        let candles = parse_ohlc(OHLC, Timeframe::HOUR_1).unwrap();
        let mut inside = candles.clone();
        inside.retain(|candle| candle.open_time >= 1_700_000_000_000);
        assert_eq!(inside.len(), 1);
        assert_eq!(newest(candles, 1)[0].open_time, 1_700_002_800_000);
    }

    #[test]
    fn the_catalog_keeps_only_online_markets_under_their_common_names() {
        let body = r#"{"error":[],"result":{
            "XXBTZUSD":{"wsname":"XBT/USD","status":"online"},
            "XDGUSD":{"wsname":"XDG/USD","status":"online"},
            "OLDUSD":{"wsname":"OLD/USD","status":"cancel_only"},
            "NOWS":{"status":"online"}
        }}"#;
        let mut ids: Vec<String> = parse_pairs(body)
            .expect("pairs")
            .into_iter()
            .map(|pair| format!("{} {} {}", pair.id, pair.base, pair.quote))
            .collect();
        ids.sort();
        assert_eq!(ids, ["BTC/USD BTC USD", "DOGE/USD DOGE USD"]);
        assert!(parse_pairs("not json").is_err());
    }

    #[test]
    fn a_pair_is_found_however_it_is_separated_or_spelled() {
        let pairs = vec![
            Pair {
                id: "BTC/USD".into(),
                base: "BTC".into(),
                quote: "USD".into(),
            },
            Pair {
                id: "ETH/BTC".into(),
                base: "ETH".into(),
                quote: "BTC".into(),
            },
        ];
        for query in ["btcusd", "BTC/USD", "btc-usd", "BTC USD", "xbtusd"] {
            let found = matching_pairs(&pairs, &alias(&normalize(query)));
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].symbol, "BTC/USD");
        }
        assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
    }

    /// Run with `cargo test --lib kraken::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_kraken_serves_candles_pairs_and_every_native() {
        let hits = KRAKEN.search_tickers("xbt/usd").expect("search");
        assert_eq!(hits[0].symbol, "BTC/USD");
        assert_eq!(hits[0].quote.as_deref(), Some("USD"));
        for native in DataSource::Kraken.natives() {
            let candles = KRAKEN
                .fetch_klines("BTC/USD", *native, 100)
                .expect("candles");
            assert_eq!(candles.len(), 100, "{native}");
            assert!(
                candles
                    .windows(2)
                    .all(|pair| pair[0].open_time < pair[1].open_time),
                "{native}"
            );
        }
        // A weekly chart is folded from the 720 days Kraken holds.
        let weekly = KRAKEN
            .fetch_klines("BTC/USD", Timeframe::WEEK_1, 500)
            .expect("weekly");
        assert!((90..=105).contains(&weekly.len()), "{}", weekly.len());
        // A tail refresh asks for little.
        let tail = KRAKEN
            .fetch_klines("BTC/USD", Timeframe::MINUTE_1, 4)
            .expect("tail");
        assert_eq!(tail.len(), 4);
        let error = KRAKEN
            .fetch_klines("NOPE/USD", Timeframe::HOUR_1, 5)
            .unwrap_err();
        assert_eq!(error.to_string(), "Kraken: Unknown asset pair");
    }
}
