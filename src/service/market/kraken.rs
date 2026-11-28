//! Kraken spot candles and its market list. Public endpoints, no key.
//!
//! `OHLC` answers the newest 720 candles of an interval and cannot page back: `since` only moves
//! the end this side of them. Rows are `[open time s, open, high, low, close, vwap, volume,
//! trades]`, the last one still forming. Failures arrive as HTTP 200 with an `error` list.

use super::candles::plan;
use super::catalog::Catalog;
use super::catalog::search::{self, ListedMarket};
use super::search_rank::normalize;
use crate::model::market::bucket::{FoldPlan, bucket_start_ms, next_bucket_start_ms};
use crate::model::market::natives::KRAKEN_BARS;
use crate::model::{DataSource, InstrumentId, Kline, Listing, Timeframe};
use crate::service::market::error::ErrorKind;
use crate::service::net::{http, rate_limit};
use crate::service::{MarketSource, ProviderError, SearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

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
    InstrumentId::new(DataSource::Kraken, symbol).canonical_symbol()
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
        let rows = fetch_ohlc(symbol, plan.base(), None)?;
        Ok(window(plan, rows, start_ms, end_ms, max))
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        search_pairs(query, || CATALOG.get(download_pairs))
    }
}

/// The hits for `query` among the pairs `list` brings. `BTC/USD`, `btc-usd`, `BTCUSD` and the
/// legacy `XBTUSD` are one query.
fn search_pairs(
    query: &str,
    list: impl FnOnce() -> Result<Arc<Vec<Pair>>, ProviderError>,
) -> Result<Vec<SearchResult>, ProviderError> {
    search::find(&alias(&normalize(query)), list)
}

/// The candles of `plan`'s target that open in `[start_ms, end_ms)` out of everything Kraken holds
/// of the base candle (`rows`), the newest `max`. A full window of `rows` was cut short of the
/// market's beginning, so its first fold is a fragment and goes, even when the range leaves fewer
/// candles than that: otherwise the page before what a chart shows would bring that fragment back
/// as a candle of its own.
fn window(plan: FoldPlan, rows: Vec<Kline>, start_ms: i64, end_ms: i64, max: usize) -> Vec<Kline> {
    let cut = rows.len() >= KRAKEN_BARS;
    let requested = plan.base_count(max).min(KRAKEN_BARS);
    let mut candles = rows;
    candles.retain(|candle| candle.open_time >= start_ms && candle.open_time < end_ms);
    let candles = newest(candles, requested);
    let first_open = candles.first().map(|candle| candle.open_time);
    let mut shown = plan.fold(candles, cut);
    // A fold keeps a lone bucket, whatever it is: a lone fragment is not a candle.
    if cut
        && shown.len() == 1
        && first_open.is_some_and(|open| open > bucket_start_ms(plan.target(), open))
    {
        shown.clear();
    }
    let skip = shown.len().saturating_sub(max);
    shown.drain(..skip);
    shown
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
        let since = crate::clock::now_ms() / 1_000 - (count as i64 + 1) * i64::from(minutes) * 60;
        url.push_str(&format!("&since={since}"));
    }
    parse_ohlc(&get(&url)?, base)
}

/// A request on Kraken's one-a-second schedule.
fn get(url: &str) -> Result<String, ProviderError> {
    rate_limit::kraken().wait_for_slot();
    http::get_text(url, &[], http::Retry::Transient)
        .map_err(|error| ProviderError::http(DataSource::Kraken, error))
}

/// The `result` of an answer, or the provider's refusal.
pub(crate) fn result_of(body: &str) -> Result<Value, ProviderError> {
    let mut value: Value = serde_json::from_str(body)
        .map_err(|error| ProviderError::malformed(DataSource::Kraken, error))?;
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
        .ok_or_else(|| ProviderError::malformed(DataSource::Kraken, "expected a result"))
}

/// What a Kraken error code (`EAPI:Rate limit exceeded`, `EQuery:Unknown asset pair`) means. The
/// rate limits are `EAPI:Rate limit…`, `EService:Throttled`, `EGeneral:Temporary lockout` and
/// `EGeneral:Too many requests`; an `EQuery` is a refusal of the request's own arguments, which
/// is a pair it does not know.
fn refusal(code: &str) -> ProviderError {
    let kind = if code.starts_with("EAPI:Rate limit")
        || code.starts_with("EService:Throttled")
        || code.starts_with("EGeneral:Temporary lockout")
        || code.starts_with("EGeneral:Too many requests")
    {
        ErrorKind::RateLimited
    } else if code.starts_with("EQuery:") {
        ErrorKind::UnknownSymbol
    } else {
        let text = code.split_once(':').map_or(code, |(_, text)| text);
        ErrorKind::Refused(text.to_owned())
    };
    ProviderError::new(DataSource::Kraken, kind)
}

/// Candles of length `base`, oldest first.
fn parse_ohlc(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let result = result_of(body)?;
    let rows = result
        .as_object()
        .and_then(|result| result.iter().find(|(key, _)| key.as_str() != "last"))
        .and_then(|(_, rows)| rows.as_array())
        .ok_or_else(|| ProviderError::malformed(DataSource::Kraken, "expected a candle array"))?;
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
            .ok_or_else(|| {
                ProviderError::malformed(DataSource::Kraken, format!("invalid field {index}"))
            })
    };
    let open_time = row
        .get(0)
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::malformed(DataSource::Kraken, "invalid timestamp"))?
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

/// One tradable market from `AssetPairs`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    id: String,
    base: String,
    quote: String,
}

impl ListedMarket for Pair {
    const SOURCE: DataSource = DataSource::Kraken;

    fn id(&self) -> &str {
        &self.id
    }

    fn sides(&self) -> Option<(&str, &str)> {
        Some((&self.base, &self.quote))
    }
}

/// Every market that is online, downloaded at most once a day.
static CATALOG: Catalog<Pair> = Catalog::new(Listing::spot(DataSource::Kraken), "catalog_kraken");

fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get(&format!("{BASE_URL}/AssetPairs"))?)
}

/// `wsname` is `XBT/USD`: the market as `BTC/USD`, which is how every Kraken API spells it.
fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let result = result_of(body)?;
    let pairs = result
        .as_object()
        .ok_or_else(|| ProviderError::malformed(DataSource::Kraken, "expected a pair list"))?;
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
            Err(ProviderError::new(
                DataSource::Kraken,
                ErrorKind::UnknownSymbol
            ))
        );
        let refused = r#"{"error":["EService:Unavailable"]}"#;
        assert_eq!(
            parse_ohlc(refused, Timeframe::HOUR_1),
            Err(ProviderError::refused(DataSource::Kraken, "Unavailable"))
        );
        for code in [
            "EAPI:Rate limit exceeded",
            "EGeneral:Too many requests",
            "EService:Throttled: 1700000000",
        ] {
            let limited = format!(r#"{{"error":["{code}"]}}"#);
            let error = parse_ohlc(&limited, Timeframe::HOUR_1).unwrap_err();
            assert!(error.is_rate_limited(), "{code}: {error:?}");
        }
        assert!(parse_ohlc("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_ohlc(r#"{"error":[]}"#, Timeframe::HOUR_1).is_err());
        assert!(parse_ohlc(r#"{"error":[],"result":{"last":1}}"#, Timeframe::HOUR_1).is_err());
        let bad_field =
            r#"{"error":[],"result":{"BTC/USD":[[1699999200,"x","1","1","1","1","1",1]]}}"#;
        assert!(parse_ohlc(bad_field, Timeframe::HOUR_1).is_err());
    }

    const DAY: i64 = 86_400_000;
    /// 2026-11-17 00:00 UTC.
    const TODAY: i64 = 1_794_873_600_000;

    /// `count` daily candles of volume 1 with the last one opening at `last_open`.
    fn days(count: usize, last_open: i64) -> Vec<Kline> {
        (0..count as i64)
            .rev()
            .map(|back| {
                let open_time = last_open - back * DAY;
                Kline {
                    open_time,
                    close_time: open_time + DAY - 1,
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 1.0,
                }
            })
            .collect()
    }

    #[test]
    fn the_first_older_page_brings_the_rest_of_the_candles_kraken_holds() {
        // 721 hourly candles: the chart showed the newest 500, the page before them is the rest.
        let plan = plan(DataSource::Kraken, Timeframe::HOUR_1).unwrap();
        let hour = 3_600_000;
        let rows: Vec<Kline> = (0..721)
            .map(|index| Kline {
                open_time: index * hour,
                close_time: index * hour + hour - 1,
                ..days(1, 0)[0].clone()
            })
            .collect();
        let end = rows[721 - 500].open_time;
        let page = window(plan, rows, 0, end, 1_000);
        assert_eq!(page.len(), 221);
        assert_eq!(page[0].open_time, 0);
        assert_eq!(page.last().unwrap().open_time, end - hour);
    }

    #[test]
    fn an_older_page_of_a_fold_does_not_bring_back_the_fragment_the_window_cut() {
        // Kraken's 720 days begin mid-week: the first weekly candle the chart showed is the first
        // whole one, and what lies before it is only that fragment, which is not a week.
        let plan = plan(DataSource::Kraken, Timeframe::WEEK_1).unwrap();
        let rows = days(721, TODAY);
        let shown = plan.shape(newest(rows.clone(), 720), 720, 500);
        let end = shown.first().unwrap().open_time;
        assert!(window(plan, rows, 0, end, 1_000).is_empty());
    }

    #[test]
    fn a_market_that_began_inside_the_window_keeps_its_first_partial_week() {
        let plan = plan(DataSource::Kraken, Timeframe::WEEK_1).unwrap();
        // Ten days of history that start on a Wednesday: the first week genuinely began then.
        let rows = days(10, TODAY);
        let weeks = window(plan, rows, 0, TODAY + DAY, 500);
        assert!(weeks.first().is_some_and(|week| week.volume < 7.0));
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
        let pairs = Arc::new(pairs);
        let ask = |query: &str| search_pairs(query, || Ok(Arc::clone(&pairs))).unwrap();
        for query in ["btcusd", "BTC/USD", "btc-usd", "BTC USD", "xbtusd"] {
            let found = ask(query);
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].source, DataSource::Kraken);
            assert_eq!(found[0].symbol, "BTC/USD");
            assert_eq!(found[0].base.as_deref(), Some("BTC"));
            assert_eq!(found[0].quote.as_deref(), Some("USD"));
        }
        assert_eq!(ask("BTC").len(), 2);
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
        // The chart's first 500 leave the rest of the 720 for one older page, then nothing.
        let shown = KRAKEN
            .fetch_klines("BTC/USD", Timeframe::HOUR_1, 500)
            .expect("hourly");
        let end = shown[0].open_time;
        let older = KRAKEN
            .fetch_klines_range("BTC/USD", Timeframe::HOUR_1, 0, end, 1_000)
            .expect("older page");
        assert!((200..=225).contains(&older.len()), "{}", older.len());
        let end = older[0].open_time;
        let none = KRAKEN
            .fetch_klines_range("BTC/USD", Timeframe::HOUR_1, 0, end, 1_000)
            .expect("second older page");
        assert!(none.is_empty());
        // A tail refresh asks for little.
        let tail = KRAKEN
            .fetch_klines("BTC/USD", Timeframe::MINUTE_1, 4)
            .expect("tail");
        assert_eq!(tail.len(), 4);
        let error = KRAKEN
            .fetch_klines("NOPE/USD", Timeframe::HOUR_1, 5)
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownSymbol);
    }
}
