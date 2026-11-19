//! Gate.io spot candles and its market list. Public v4 endpoints, no key.
//!
//! `/spot/candlesticks` answers up to 1,000 rows oldest first, `[start s, quote volume, close,
//! high, low, open, base volume, window closed]`, all strings, and pages back with a `to` cursor
//! (inclusive, in seconds). Failures are HTTP 4xx with a JSON label; some regions get 403.

use super::candles::plan;
use super::catalog::Catalog;
use super::paging::Paging;
use super::search_rank::normalize;
use crate::model::{DataSource, InstrumentId, Kline, Timeframe};
use crate::service::net::http;
use crate::service::{MarketSource, ProviderError, SearchResult, rank_results};
use degenview_market::bucket::{FoldPlan, next_bucket_start_ms};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub(crate) const BASE_URL: &str = "https://api.gateio.ws/api/v4/spot";

pub struct Gate;

pub const GATE: Gate = Gate;

/// 1,000 rows a request. Gate.io refuses (HTTP 400, "Candlestick too long ago") a request that
/// reaches 10,000 points back from now in the interval asked for, and counts the rows the
/// request holds: the newest 8,900 keep clear of it, which is a week of 1-minute candles, a
/// month of 5-minute ones, a year of hours and 24 years of days. It allows 200 requests per 10
/// seconds per IP, far more than a chart asks for, so the pages are not spaced.
const PAGING: Paging = Paging {
    page: 1_000,
    max_candles: 8_900,
    pause: Duration::ZERO,
};

/// How many more candles of `base` a request ending just before `after_ms` may hold and stay
/// within what Gate.io reaches back to; none once the request would pass it.
fn reach_left(now_ms: i64, after_ms: i64, base: Timeframe) -> usize {
    let step = base.nominal_ms().max(1);
    let points_ago = ((now_ms - after_ms).max(0) / step) as usize + 1;
    PAGING.max_candles.saturating_sub(points_ago)
}

/// The newest `max` candles of `plan`'s target that open in `[start_ms, end_ms)`, paged back from
/// `end_ms`. `fetch(to, limit)` answers up to `limit` base candles that open at `to` (seconds) or
/// before, oldest first.
fn collect(
    plan: FoldPlan,
    now_ms: i64,
    start_ms: i64,
    end_ms: i64,
    max: usize,
    mut fetch: impl FnMut(i64, usize) -> Result<Vec<Kline>, ProviderError>,
) -> Result<Vec<Kline>, ProviderError> {
    // What cannot be reached is not asked for: an answer that fills this is one the reach cut
    // short, so its oldest fold is dropped as a fragment; a shorter one is the market's start.
    let requested = PAGING
        .requested(plan, max)
        .min(reach_left(now_ms, end_ms, plan.base()));
    if requested == 0 {
        return Ok(Vec::new());
    }
    let candles = PAGING.back(start_ms, end_ms, requested, |after, wanted| {
        // A gap in the candles moves `after` back past what was counted: never ask Gate.io for
        // more than it reaches (HTTP 400), the end of history is an empty answer.
        let wanted = wanted.min(reach_left(now_ms, after, plan.base()));
        if wanted == 0 {
            return Ok(Vec::new());
        }
        // `to` includes the candle that opens at it, in seconds; `after` is exclusive.
        fetch((after - 1) / 1_000, wanted)
    })?;
    Ok(plan.shape(candles, requested, max))
}

/// Gate.io's `interval` for a candle length it serves itself: `1m`, `4h`, `1d`, `7d` (weeks,
/// from Monday) and `30d` (the calendar month).
pub(crate) fn interval(timeframe: Timeframe) -> Option<&'static str> {
    if !DataSource::Gate.natives().contains(&timeframe) {
        return None;
    }
    [
        (Timeframe::MINUTE_1, "1m"),
        (Timeframe::MINUTE_5, "5m"),
        (Timeframe::MINUTE_15, "15m"),
        (Timeframe::MINUTE_30, "30m"),
        (Timeframe::HOUR_1, "1h"),
        (Timeframe::HOUR_4, "4h"),
        (Timeframe::HOUR_8, "8h"),
        (Timeframe::DAY_1, "1d"),
        (Timeframe::WEEK_1, "7d"),
        (Timeframe::MONTH_1, "30d"),
    ]
    .into_iter()
    .find_map(|(native, token)| (native == timeframe).then_some(token))
}

/// Gate.io's name of a market, `BTC_USDT`. `BTC/USDT`, `btc-usdt` and a bare `BTC` (USDT) are
/// accepted.
pub(crate) fn pair_id(symbol: &str) -> String {
    InstrumentId::new(DataSource::Gate, symbol).api_symbol()
}

impl MarketSource for Gate {
    fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        limit: u32,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Gate, timeframe)?;
        let requested = PAGING.requested(plan, limit as usize);
        if requested > PAGING.page {
            // More base candles than one request holds: page back from now.
            let end_ms = crate::clock::now_ms() + 1;
            return self.fetch_klines_range(symbol, timeframe, 0, end_ms, limit as usize);
        }
        let rows = fetch_page(&pair_id(symbol), plan.base(), requested, None)?;
        Ok(plan.shape(rows, requested, limit as usize))
    }

    fn fetch_klines_range(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_ms: i64,
        end_ms: i64,
        max: usize,
    ) -> Result<Vec<Kline>, ProviderError> {
        let plan = plan(DataSource::Gate, timeframe)?;
        let id = pair_id(symbol);
        collect(
            plan,
            crate::clock::now_ms(),
            start_ms,
            end_ms,
            max,
            |to_s, limit| fetch_page(&id, plan.base(), limit, Some(to_s)),
        )
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, ProviderError> {
        // `BTC_USDT`, `btc/usdt` and `BTCUSDT` are one query.
        let needle = normalize(query);
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = CATALOG.get(download_pairs)?;
        let results = matching_pairs(&pairs, &needle);
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}

fn fetch_page(
    id: &str,
    base: Timeframe,
    limit: usize,
    to_s: Option<i64>,
) -> Result<Vec<Kline>, ProviderError> {
    let interval =
        interval(base).ok_or_else(|| ProviderError::unsupported(DataSource::Gate, base))?;
    let mut url =
        format!("{BASE_URL}/candlesticks?currency_pair={id}&interval={interval}&limit={limit}");
    if let Some(to_s) = to_s {
        url.push_str(&format!("&to={to_s}"));
    }
    parse_candles(&get(&url)?, base)
}

/// A request, with Gate.io's refusals told apart from a network failure: a 400 names a market it
/// does not list, a 403 a country it does not serve (or a client that asks too often).
pub(crate) fn get(url: &str) -> Result<String, ProviderError> {
    match http::get_text(url, &[], http::Retry::Transient) {
        Ok(body) => Ok(body),
        Err(http::Error::Status(400)) => Err(ProviderError::other(
            "Gate.io refused the request (HTTP 400): it does not list this market",
        )),
        Err(http::Error::Status(403)) => Err(ProviderError::other(
            "Gate.io refused the request (HTTP 403): it is not available in every country, and \
             refuses a client that asks too often",
        )),
        Err(error) => Err(error.into()),
    }
}

/// Candles of length `base`, oldest first.
fn parse_candles(body: &str, base: Timeframe) -> Result<Vec<Kline>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    let mut candles = value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a candle array"))?
        .iter()
        .map(|row| parse_row(row, base))
        .collect::<Result<Vec<_>, _>>()?;
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

fn parse_row(row: &Value, base: Timeframe) -> Result<Kline, ProviderError> {
    let text = |index: usize| -> Result<&str, ProviderError> {
        row.get(index)
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::data(format!("invalid field {index}")))
    };
    let number = |index: usize| -> Result<f64, ProviderError> {
        text(index)?
            .parse()
            .map_err(|_| ProviderError::data(format!("invalid field {index}")))
    };
    let seconds: i64 = text(0)?
        .parse()
        .map_err(|_| ProviderError::data("invalid timestamp"))?;
    let open_time = seconds * 1_000;
    Ok(Kline {
        open_time,
        close_time: next_bucket_start_ms(base, open_time) - 1,
        open: number(5)?,
        high: number(3)?,
        low: number(4)?,
        close: number(2)?,
        volume: number(6)?,
    })
}

/// The listed markets whose name, without its underscore, contains `needle` (letters and digits,
/// upper case).
fn matching_pairs(pairs: &[Pair], needle: &str) -> Vec<SearchResult> {
    pairs
        .iter()
        .filter(|pair| normalize(&pair.id).contains(needle))
        .map(|pair| {
            SearchResult::new(DataSource::Gate, pair.id.clone(), pair.id.clone())
                .pair(pair.base.clone(), pair.quote.clone())
        })
        .collect()
}

/// One tradable spot market from `currency_pairs`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    id: String,
    base: String,
    quote: String,
}

/// Every spot market that is tradable, downloaded at most once a day.
static CATALOG: Catalog<Pair> = Catalog::new("catalog_gateio");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(in crate::service) fn clear_catalog() {
    CATALOG.clear_memory();
}

fn download_pairs() -> Result<Vec<Pair>, ProviderError> {
    parse_pairs(&get(&format!("{BASE_URL}/currency_pairs"))?)
}

fn parse_pairs(body: &str) -> Result<Vec<Pair>, ProviderError> {
    let value: Value = serde_json::from_str(body).map_err(ProviderError::data)?;
    Ok(value
        .as_array()
        .ok_or_else(|| ProviderError::data("expected a market list"))?
        .iter()
        .filter(|row| row.get("trade_status").and_then(Value::as_str) == Some("tradable"))
        .filter_map(|row| {
            let field = |key: &str| row.get(key)?.as_str().filter(|text| !text.is_empty());
            Some(Pair {
                id: field("id")?.to_owned(),
                base: field("base")?.to_owned(),
                quote: field("quote")?.to_owned(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_has_its_interval_and_nothing_else_does() {
        let intervals: Vec<&str> = DataSource::Gate
            .natives()
            .iter()
            .map(|native| interval(*native).expect("a native has an interval"))
            .collect();
        assert_eq!(
            intervals,
            [
                "1m", "5m", "15m", "30m", "1h", "4h", "8h", "1d", "7d", "30d"
            ]
        );
        // Gate.io also answers 3m, 2h, 6h and 12h, but documents none of them.
        for other in ["3m", "2h", "6h", "12h", "2D", "2W", "3M", "1Y"] {
            assert_eq!(interval(Timeframe::parse(other).unwrap()), None, "{other}");
        }
    }

    fn base_of(timeframe: &str) -> (&'static str, usize) {
        let plan = plan(DataSource::Gate, Timeframe::parse(timeframe).unwrap()).unwrap();
        (interval(plan.base()).unwrap(), plan.base_count(500))
    }

    #[test]
    fn other_lengths_fold_the_coarsest_native_that_nests() {
        // A request for 500 candles also asks for the one the start cuts.
        assert_eq!(base_of("1h"), ("1h", 500));
        assert_eq!(base_of("22m"), ("1m", 501 * 22));
        assert_eq!(base_of("2h"), ("1h", 501 * 2));
        assert_eq!(base_of("12h"), ("4h", 501 * 3));
        assert_eq!(base_of("3D"), ("1d", 501 * 3));
        assert_eq!(base_of("3M"), ("30d", 501 * 3));
        assert_eq!(base_of("1Y"), ("30d", 501 * 12));
    }

    #[test]
    fn a_request_wider_than_the_paging_is_capped_before_it_is_folded() {
        // 500 candles of 1,439 minutes is 720,939 one-minute candles; paging reaches 8,900.
        let wide = plan(DataSource::Gate, Timeframe::parse("1439m").unwrap()).unwrap();
        assert_eq!(PAGING.requested(wide, 500), PAGING.max_candles);
        let hour = plan(DataSource::Gate, Timeframe::HOUR_1).unwrap();
        assert_eq!(PAGING.requested(hour, 500), 500);
    }

    #[test]
    fn a_request_stops_where_gate_stops_reaching_back() {
        let hour = 3_600_000;
        let now = 100_000 * hour;
        let base = Timeframe::HOUR_1;
        // The page right behind the newest candles has nearly all the reach left.
        assert_eq!(reach_left(now, now - 500 * hour, base), 8_399);
        // Near the end only what is left may be asked for, and past it nothing.
        assert_eq!(reach_left(now, now - 8_000 * hour, base), 899);
        assert_eq!(reach_left(now, now - 8_899 * hour, base), 0);
        assert_eq!(reach_left(now, now - 20_000 * hour, base), 0);
        // A cursor in the future (a clock that is behind) has all of it.
        assert_eq!(reach_left(now, now + hour, base), 8_899);
    }

    /// 12:00:30 UTC on 2026-11-16: the forming one-minute candle opened at 12:00:00.
    const NOW_MS: i64 = 1_794_830_430_000;
    const MINUTE: i64 = 60_000;

    /// A fake Gate.io: a market whose one-minute candles (volume 1) reach back `depth` minutes
    /// from the forming one, answering `fetch(to, limit)` as the API does and refusing, as the
    /// real service does with a 400, a request that reaches 9,000 points back from now.
    struct Fake {
        depth: i64,
        now: i64,
        asked: std::cell::Cell<usize>,
    }

    impl Fake {
        fn new(depth: i64) -> Self {
            Self::at(depth, NOW_MS)
        }

        /// The same market `now` milliseconds into the day: where the reach ends moves with it.
        fn at(depth: i64, now: i64) -> Self {
            Self {
                depth,
                now,
                asked: std::cell::Cell::new(0),
            }
        }

        fn answer(&self, to_s: i64, limit: usize) -> Result<Vec<Kline>, ProviderError> {
            self.asked.set(self.asked.get() + 1);
            let newest = self.now / MINUTE * MINUTE;
            let oldest = newest - (self.depth - 1) * MINUTE;
            let points_ago = (self.now - to_s * 1_000) / MINUTE + limit as i64;
            assert!(
                points_ago <= 9_000,
                "Gate.io refuses: too long ago ({points_ago})"
            );
            let last = (to_s * 1_000 / MINUTE * MINUTE).min(newest);
            Ok((0..limit as i64)
                .rev()
                .map(|back| last - back * MINUTE)
                .filter(|open| *open >= oldest)
                .map(|open_time| Kline {
                    open_time,
                    close_time: open_time + MINUTE - 1,
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 1.0,
                })
                .collect())
        }
    }

    /// Whether `candle` holds every minute of its bucket: nothing was cut off it.
    fn is_whole(candle: &Kline, base: Timeframe) -> bool {
        (candle.volume * MINUTE as f64) as i64
            == next_bucket_start_ms(base, candle.open_time) - candle.open_time
    }

    #[test]
    fn a_22_minute_chart_cut_short_by_the_reach_drops_its_partial_first_candle() {
        let target = Timeframe::parse("22m").unwrap();
        let plan = plan(DataSource::Gate, target).unwrap();
        // Where the reach ends against the 22-minute grid depends on the minute it is asked in.
        for minute in 0..22 {
            let now = NOW_MS + minute * MINUTE;
            let market = Fake::at(20_000, now);
            let shown = collect(plan, now, 0, now + 1, 500, |to, limit| {
                market.answer(to, limit)
            })
            .expect("candles");
            assert!(shown.len() > 300, "{}", shown.len());
            assert!(
                is_whole(&shown[0], target),
                "minute {minute}: the oldest candle holds {} minutes of {}",
                shown[0].volume,
                (next_bucket_start_ms(target, shown[0].open_time) - shown[0].open_time) / MINUTE
            );
        }
    }

    #[test]
    fn an_older_page_near_the_reach_is_cut_short_and_the_one_after_it_is_empty() {
        let target = Timeframe::parse("22m").unwrap();
        let plan = plan(DataSource::Gate, target).unwrap();
        let market = Fake::new(20_000);
        let newest = NOW_MS / MINUTE * MINUTE;
        let end = newest - 8_400 * MINUTE;
        let page = collect(plan, NOW_MS, 0, end, 1_000, |to, limit| {
            market.answer(to, limit)
        })
        .expect("an older page");
        assert!(!page.is_empty());
        assert!(is_whole(&page[0], target), "{}", page[0].volume);

        let asked = market.asked.get();
        let end = newest - 8_899 * MINUTE;
        let past = collect(plan, NOW_MS, 0, end, 1_000, |to, limit| {
            market.answer(to, limit)
        })
        .expect("nothing past the reach");
        assert!(past.is_empty());
        assert_eq!(
            market.asked.get(),
            asked,
            "no request is made past the reach"
        );
    }

    #[test]
    fn a_market_that_began_inside_the_reach_keeps_its_first_candle_as_it_is() {
        let target = Timeframe::parse("22m").unwrap();
        let plan = plan(DataSource::Gate, target).unwrap();
        let market = Fake::new(3_000);
        let shown = collect(plan, NOW_MS, 0, NOW_MS + 1, 500, |to, limit| {
            market.answer(to, limit)
        })
        .expect("candles");
        // Listed 3,000 minutes ago: its first bucket is genuinely short.
        assert!(!is_whole(&shown[0], target), "{}", shown[0].volume);
    }

    #[test]
    fn a_native_length_reaches_as_far_back_as_gate_lets_it() {
        let one = plan(DataSource::Gate, Timeframe::MINUTE_1).unwrap();
        let market = Fake::new(20_000);
        let all = collect(one, NOW_MS, 0, NOW_MS + 1, 8_900, |to, limit| {
            market.answer(to, limit)
        })
        .expect("candles");
        assert_eq!(all.len(), 8_899);
        assert!(
            all.windows(2)
                .all(|pair| pair[0].open_time < pair[1].open_time)
        );
    }

    #[test]
    fn symbols_are_gateios_however_they_are_spelled() {
        assert_eq!(pair_id("btc/usdt"), "BTC_USDT");
        assert_eq!(pair_id("eth-usdc"), "ETH_USDC");
        assert_eq!(pair_id("ETH_BTC"), "ETH_BTC");
        assert_eq!(pair_id("sol"), "SOL_USDT");
    }

    // The close is the third field and the open the sixth; the base volume is the seventh.
    const ROWS: &str = r#"[
        ["1699999200","450","11","12","9","10","42.5","true"],
        ["1700002800","60","12","13","10","11","5.5","false"]
    ]"#;

    #[test]
    fn rows_are_candles_with_the_close_of_their_bucket() {
        let candles = parse_candles(ROWS, Timeframe::HOUR_1).expect("valid response");
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
    fn a_month_closes_when_the_next_calendar_month_opens() {
        // 2026-02-01 00:00 UTC.
        let row = serde_json::json!(["1769904000", "4", "1.5", "2", "0.5", "1", "3", "true"]);
        let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
        assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
    }

    #[test]
    fn odd_answers_are_errors() {
        assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
        assert!(parse_candles(r#"{"label":"INVALID_PARAM_VALUE"}"#, Timeframe::HOUR_1).is_err());
        assert_eq!(parse_candles("[]", Timeframe::HOUR_1), Ok(Vec::new()));
        let bad_field = r#"[["1700000000","1","x","1","1","1","1","true"]]"#;
        assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
        assert!(
            parse_candles(
                r#"[["x","1","1","1","1","1","1","true"]]"#,
                Timeframe::HOUR_1
            )
            .is_err()
        );
    }

    #[test]
    fn the_catalog_keeps_only_tradable_markets() {
        let body = r#"[
            {"id":"BTC_USDT","base":"BTC","quote":"USDT","trade_status":"tradable"},
            {"id":"OLD_USDT","base":"OLD","quote":"USDT","trade_status":"untradable"},
            {"id":"SELL_USDT","base":"SELL","quote":"USDT","trade_status":"sellable"},
            {"id":"NOQUOTE","base":"X","quote":"","trade_status":"tradable"}
        ]"#;
        let pairs = parse_pairs(body).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].id.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTC_USDT", "BTC", "USDT")
        );
        assert!(parse_pairs("not json").is_err());
        assert!(parse_pairs("{}").is_err());
    }

    #[test]
    fn a_pair_is_found_however_it_is_separated() {
        let pairs = vec![
            Pair {
                id: "BTC_USDT".into(),
                base: "BTC".into(),
                quote: "USDT".into(),
            },
            Pair {
                id: "ETH_BTC".into(),
                base: "ETH".into(),
                quote: "BTC".into(),
            },
        ];
        for query in ["btcusdt", "BTC_USDT", "btc/usdt", "BTC USDT", "btc-usdt"] {
            let found = matching_pairs(&pairs, &normalize(query));
            assert_eq!(found.len(), 1, "{query}");
            assert_eq!(found[0].symbol, "BTC_USDT");
        }
        assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
    }

    /// Run with `cargo test --lib gateio::tests::live -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the network"]
    fn live_gate_serves_candles_pairs_and_every_native() {
        let hits = GATE.search_tickers("btc-usdt").expect("search");
        assert_eq!(hits[0].symbol, "BTC_USDT");
        assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
        for native in DataSource::Gate.natives() {
            // More than one page of the shortest bars, so paging is exercised.
            let wanted = if *native == Timeframe::MINUTE_1 {
                1_500
            } else {
                350
            };
            let candles = GATE
                .fetch_klines("BTC_USDT", *native, wanted)
                .expect("candles");
            // Months reach back only to the market's listing.
            if *native == Timeframe::MONTH_1 {
                assert!(candles.len() > 30, "{native}");
            } else {
                assert_eq!(candles.len(), wanted as usize, "{native}");
            }
            assert!(
                candles
                    .windows(2)
                    .all(|pair| pair[0].open_time < pair[1].open_time),
                "{native}"
            );
        }
        // A 22-minute chart wants eleven thousand one-minute candles and Gate.io reaches back to
        // 8,900 of them: about 408 candles (days restart the buckets), none of them a fragment.
        let folded = GATE
            .fetch_klines("BTC_USDT", Timeframe::parse("22m").unwrap(), 500)
            .expect("folded candles");
        assert!((395..=420).contains(&folded.len()), "{}", folded.len());
        // Paging back from the first 500 reaches the end of what Gate.io holds without an error,
        // and the page after it is empty.
        let hour = GATE
            .fetch_klines("BTC_USDT", Timeframe::MINUTE_1, 500)
            .expect("minutes");
        let mut end = hour[0].open_time;
        let mut total = hour.len();
        loop {
            let page = GATE
                .fetch_klines_range("BTC_USDT", Timeframe::MINUTE_1, 0, end, 1_000)
                .expect("an older page, or an empty one at the end");
            let Some(first) = page.first() else { break };
            total += page.len();
            end = first.open_time;
        }
        assert!((8_800..=8_900).contains(&total), "{total}");
        let error = GATE
            .fetch_klines("NOPE_USDT", Timeframe::HOUR_1, 5)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Gate.io refused the request (HTTP 400): it does not list this market"
        );
    }
}
