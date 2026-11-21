use super::history::page_before_with;
use super::markets::{Pair, decimals_of, find, matching_pairs, parse_pairs};
use super::parse::{Answer, answer, parse_candles, pushed_candle};
use super::precision::{cut, cut_candle, decimals_of_tick};
use super::socket::{TOPICS_PER_CONNECTION, kline_topic, subscribe, ticker_topic, token};
use super::ticker::parse_tickers;
use super::*;
use crate::model::market::bucket::bucket_start_ms;
use crate::model::watchlist::quote::VolumeKind;
use crate::service::net::http;
use crate::service::net::ws::{Decode, Keepalive};
use crate::service::watchlist_quotes::provider::request;
use serde_json::{Value, json};
use std::collections::HashMap;

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

#[test]
fn every_native_has_its_socket_name_and_nothing_else_does() {
    let tokens: Vec<&str> = DataSource::Bingx
        .natives()
        .iter()
        .map(|native| token(*native).expect("a native has a socket name"))
        .collect();
    // The socket's names are not REST's: `1h` is `60min`.
    assert_eq!(
        tokens,
        [
            "1min", "3min", "5min", "15min", "30min", "60min", "2hour", "4hour", "8hour"
        ]
    );
    for other in ["2m", "6h", "12h", "1D", "3D", "1W", "1M", "1Y", "22m"] {
        assert_eq!(token(tf(other)), None, "{other}");
        assert_eq!(history::interval(tf(other)), None, "{other}");
    }
    for native in DataSource::Bingx.natives() {
        // REST takes the length as this app writes it.
        assert_eq!(history::interval(*native), Some(native.interval()));
    }
}

#[test]
fn what_it_cuts_at_utc_plus_eight_folds_from_finer_bars() {
    let base = |timeframe: &str| {
        let plan = plan(DataSource::Bingx, tf(timeframe)).unwrap();
        (plan.base().interval(), plan.base_count(500))
    };
    assert_eq!(base("1h"), ("1h".to_owned(), 500));
    assert_eq!(base("8h"), ("8h".to_owned(), 500));
    assert_eq!(base("22m"), ("1m".to_owned(), 501 * 22));
    for folded in ["6h", "12h", "1D", "3D", "1W", "1M", "1Y"] {
        let (interval, _) = base(folded);
        assert!(
            DataSource::Bingx
                .natives()
                .iter()
                .any(|native| native.interval() == interval),
            "{folded} folds from {interval}"
        );
    }
}

/// BingX's own 6h, 12h, days, 3-day and weekly bars open at UTC+8 midnight and the hours six and
/// twelve before and after it: the reason they are not natives. Opens as both endpoints answered
/// them on 2026-11-21.
#[test]
fn its_own_longer_bars_do_not_open_where_this_app_cuts() {
    for (timeframe, open_s) in [
        ("6h", 1_795_082_400_i64), // 2026-11-19 10:00 UTC
        ("12h", 1_795_060_800),    // 2026-11-19 04:00 UTC
        ("1D", 1_795_104_000),     // 2026-11-19 16:00 UTC, a Thursday
        ("3D", 1_794_844_800),     // 2026-11-16 16:00 UTC
        ("1W", 1_794_758_400),     // 2026-11-15 16:00 UTC, a Sunday
        ("1M", 1_793_462_400),     // 2026-10-31 16:00 UTC
    ] {
        let open = open_s * 1_000;
        assert_ne!(bucket_start_ms(tf(timeframe), open), open, "{timeframe}");
        assert!(!DataSource::Bingx.natives().contains(&tf(timeframe)));
    }
    // Its own bars of up to 8 hours, which are the natives, do.
    for native in DataSource::Bingx.natives() {
        let open = 1_795_104_000_000_i64; // 2026-11-19 16:00 UTC: on every one of their grids
        assert_eq!(bucket_start_ms(*native, open), open, "{native}");
    }
}

#[test]
fn markets_are_bingxs_however_they_are_spelled() {
    assert_eq!(market_id("BTC-USDT").as_deref(), Some("BTC-USDT"));
    assert_eq!(market_id("btc/usdt").as_deref(), Some("BTC-USDT"));
    assert_eq!(market_id("eth-btc").as_deref(), Some("ETH-BTC"));
    assert_eq!(market_id("sol").as_deref(), Some("SOL-USDT"));
    // Text that could change a URL is not a market.
    for text in ["BTC-USDT&limit=1", "btc?usdt", "a b%20", "é", "", "-"] {
        assert_eq!(market_id(text), None, "{text:?}");
    }
}

#[test]
fn messages_name_the_topic_in_bingxs_own_words() {
    assert_eq!(kline_topic("BTC-USDT", "60min"), "BTC-USDT@kline_60min");
    assert_eq!(ticker_topic("BTC-USDT"), "BTC-USDT@ticker");
    assert_eq!(
        serde_json::from_str::<Value>(&subscribe("BTC-USDT@ticker")).unwrap(),
        json!({"id": "degenview", "reqType": "sub", "dataType": "BTC-USDT@ticker"})
    );
}

// MARK: Answers

/// What `/openApi/spot/v2/market/kline?symbol=BTC-USDT&interval=1h&limit=3` answered on
/// 2026-11-21: the newest candle first, the prices on the tick.
const RECENT_ROWS: &str = r#"{"code":0,"timestamp":1795246229354,"data":[
    [1795244400000,58079.99,58665.93,57745.16,58190.35,202.413057,1795247999999,11781405.50],
    [1795240800000,55792.02,58106.44,55159.00,58079.99,325.239542,1795244399999,18405675.55],
    [1795237200000,56307.34,56503.47,55722.21,55792.02,242.431494,1795240799999,13600307.72]
]}"#;

/// What `/openApi/market/his/v1/kline` answered for the same market and length: a digit past the
/// tick, the float noise of it, and a volume written with eighteen decimals.
const HISTORY_ROWS: &str = r#"{"code":0,"timestamp":1795246229642,"data":[
    [1795158000000,62065.116,62065.116,61478.73,61859.024,237.657998000000000000,1795161599999,14691460.832319],
    [1795154400000,61937.992,62066.12,61702.823000000004,62065.116,104.101306000000000000,1795157999999,6441274.027146],
    [1795150800000,61517.715000000004,62015.07,61435.757000000005,61937.992,117.878423000000000000,1795154399999,7270550.706105]
]}"#;

#[test]
fn rows_come_newest_first_and_leave_oldest_first_with_the_close_of_their_bucket() {
    let candles = parse_candles(RECENT_ROWS, Timeframe::HOUR_1).unwrap();
    assert_eq!(
        candles.iter().map(|bar| bar.open_time).collect::<Vec<_>>(),
        [1_795_237_200_000, 1_795_240_800_000, 1_795_244_400_000]
    );
    let newest = &candles[2];
    assert_eq!(newest.close_time, 1_795_247_999_999);
    assert_eq!(
        (newest.open, newest.high, newest.low, newest.close),
        (58_079.99, 58_665.93, 57_745.16, 58_190.35)
    );
    // The base asset, not the quote turnover that follows the close time.
    assert_eq!(newest.volume, 202.413057);
}

#[test]
fn the_history_endpoint_answers_the_same_rows_with_a_digit_more() {
    let candles = parse_candles(HISTORY_ROWS, Timeframe::HOUR_1).unwrap();
    assert_eq!(candles.len(), 3);
    assert_eq!(candles[2].volume, 237.657998);
    // The digit beyond the tick, and the noise after it, as the text says them.
    assert_eq!(candles[1].low, 61_702.823000000004);
}

#[test]
fn an_answer_of_nothing_is_no_candles_and_a_refusal_is_bingxs_message() {
    // Before the market began, past what the endpoint keeps, or an unknown market.
    for nothing in [
        r#"{"code":100204,"msg":"no data found","timestamp":1}"#,
        r#"{"code":100204,"msg":"The maximum query range for BTC_USDT K-lines is 7 days and 0 hours.","timestamp":1}"#,
    ] {
        assert_eq!(parse_candles(nothing, Timeframe::MINUTE_1), Ok(Vec::new()));
        assert_eq!(answer(nothing), Ok(Answer::NoData));
    }
    assert_eq!(
        parse_candles(
            r#"{"code":100400,"msg":"limit should be less than or equal to 1000","timestamp":1}"#,
            Timeframe::MINUTE_1
        ),
        Err(ProviderError::other(
            "BingX: limit should be less than or equal to 1000"
        ))
    );
    assert_eq!(
        parse_candles(r#"{"code":100500}"#, Timeframe::MINUTE_1),
        Err(ProviderError::other("BingX: The request failed"))
    );
    // A call that has no use for nothing says so.
    assert_eq!(
        answer(r#"{"code":100204,"msg":"no data found"}"#)
            .unwrap()
            .data(),
        Err(ProviderError::other("BingX has no data for the request"))
    );
}

#[test]
fn a_rate_limit_is_told_by_its_code_and_nothing_else_is_one() {
    let limited = parse_candles(
        r#"{"code":100410,"msg":"rate limit exceeded","timestamp":1}"#,
        Timeframe::MINUTE_1,
    )
    .unwrap_err();
    assert_eq!(limited, ProviderError::Network(http::Error::Status(429)));
    assert!(limited.is_rate_limited());
    // A message that happens to say so is not.
    let other = parse_candles(
        r#"{"code":100400,"msg":"429 too many requests"}"#,
        Timeframe::MINUTE_1,
    )
    .unwrap_err();
    assert!(!other.is_rate_limited());
}

#[test]
fn unreadable_answers_are_data_errors() {
    for broken in [
        "not json",
        "{}",
        r#"{"code":"0"}"#,
        r#"{"code":0}"#,
        r#"{"code":0,"data":{}}"#,
        r#"{"code":0,"data":[[1795244400000,1.0]]}"#,
        r#"{"code":0,"data":[["x",1,1,1,1,1]]}"#,
    ] {
        let error = parse_candles(broken, Timeframe::HOUR_1).unwrap_err();
        assert!(matches!(error, ProviderError::Data(_)), "{broken}: {error}");
    }
}

// MARK: Prices

#[test]
fn a_prices_digits_are_the_ticks() {
    for (tick, digits) in [
        (1.0, 0),
        (0.1, 1),
        (0.01, 2),
        (0.0001, 4),
        (1e-8, 8),
        (1e-12, 12),
    ] {
        assert_eq!(decimals_of_tick(tick), Some(digits), "{tick}");
    }
    for tick in [0.0, -0.01, f64::NAN, f64::INFINITY] {
        assert_eq!(decimals_of_tick(tick), None, "{tick}");
    }
}

#[test]
fn a_price_is_cut_at_the_tick_never_rounded() {
    // What the history says, and what the recent endpoint made of it (measured on 2026-11-21).
    for (history, recent, digits) in [
        (64_655.599, 64_655.59, 2),
        (64_637.138, 64_637.13, 2),
        (62_065.116, 62_065.11, 2),
        (61_702.823000000004, 61_702.82, 2),
        (61_517.715000000004, 61_517.71, 2),
        (58_243.039000000004, 58_243.03, 2),
        // A double a hair under the tick it means, which BingX cuts a tick lower.
        (64_441.509999999995, 64_441.5, 2),
        // Already on the tick: a float that is a hair under it is not a tick lower.
        (64_655.59, 64_655.59, 2),
        (0.1 + 0.2, 0.3, 1),
        (3.33e-06, 3.33e-06, 8),
        (3.339e-06, 3.33e-06, 8),
        (0.0, 0.0, 2),
        (5.0, 5.0, 0),
        (5.9, 5.0, 0),
    ] {
        assert_eq!(cut(history, digits), recent, "{history} at {digits}");
    }
}

#[test]
fn a_candle_is_cut_at_the_tick_and_its_volume_is_left_alone() {
    let candles = parse_candles(HISTORY_ROWS, Timeframe::HOUR_1).unwrap();
    let cut = cut_candle(candles[1].clone(), 2);
    assert_eq!(
        (cut.open, cut.high, cut.low, cut.close),
        (61_937.99, 62_066.12, 61_702.82, 62_065.11)
    );
    assert_eq!(cut.volume, 104.101306);
    assert_eq!(
        (cut.open_time, cut.close_time),
        (1_795_154_400_000, 1_795_157_999_999)
    );
}

// MARK: Paging

fn market() -> Pair {
    Pair {
        id: "BTC-USDT".to_owned(),
        base: "BTC".to_owned(),
        quote: "USDT".to_owned(),
        decimals: 2,
    }
}

/// An answer with a candle at each of `opens` (hours), newest first, in the shape of the API.
fn rows_at(opens: &[i64]) -> String {
    let rows: Vec<Value> = opens
        .iter()
        .rev()
        .map(|hour| {
            json!([
                hour * HOUR,
                10.123,
                11.456,
                9.789,
                10.5,
                2.0,
                hour * HOUR + HOUR - 1,
                20.0
            ])
        })
        .collect();
    json!({"code": 0, "data": rows}).to_string()
}

/// The `endTime` and `limit` a path asks for.
fn asked(path: &str) -> (i64, i64) {
    let value = |key: &str| -> i64 {
        path.split_once(&format!("{key}="))
            .and_then(|(_, rest)| rest.split('&').next())
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("no {key} in {path}"))
    };
    (value("endTime"), value("limit"))
}

#[test]
fn a_page_in_the_last_day_and_a_half_is_the_recent_endpoints_and_an_older_one_the_historys() {
    let now = 1_795_246_229_354_i64;
    let mut paths = Vec::new();
    let mut ask = |path: &str| {
        paths.push(path.to_owned());
        Ok(r#"{"code":100204,"msg":"no data found"}"#.to_owned())
    };
    for after in [now + 1, now - 35 * HOUR, now - 37 * HOUR, now - 400 * DAY] {
        page_before_with(&mut ask, now, &market(), Timeframe::HOUR_1, after, 500).unwrap();
    }
    assert_eq!(
        paths,
        [
            format!(
                "/openApi/spot/v2/market/kline?symbol=BTC-USDT&interval=1h&limit=500&endTime={now}"
            ),
            format!(
                "/openApi/spot/v2/market/kline?symbol=BTC-USDT&interval=1h&limit=500&endTime={}",
                now - 35 * HOUR - 1
            ),
            format!(
                "/openApi/market/his/v1/kline?symbol=BTC-USDT&interval=1h&limit=500&endTime={}",
                now - 37 * HOUR - 1
            ),
            format!(
                "/openApi/market/his/v1/kline?symbol=BTC-USDT&interval=1h&limit=500&endTime={}",
                now - 400 * DAY - 1
            ),
        ]
    );
}

#[test]
fn only_the_historys_prices_are_cut() {
    // The newest of the rows opened a day and a bit before `answered`.
    let answered = 1_795_246_229_354_i64;
    let page = |now: i64| {
        page_before_with(
            |_: &str| Ok(HISTORY_ROWS.to_owned()),
            now,
            &market(),
            Timeframe::HOUR_1,
            answered,
            3,
        )
        .unwrap()
    };
    // Asked when the page is the last day and a half: the recent endpoint's, used as it is.
    let recent = page(answered);
    assert_eq!(recent[1].low, 61_702.823000000004);
    // Asked ten days later it is the history's, and cut at the tick.
    let history = page(answered + 10 * DAY);
    assert_eq!(history[1].low, 61_702.82);
    assert_eq!(history[2].close, 61_859.02);
}

#[test]
fn a_hole_in_the_market_is_stepped_over() {
    let now = 1_000 * DAY;
    let after = now - 100 * DAY;
    let after_hour = after / HOUR;
    // The hours 1 to 4 before `after` have candles, so do 10 to 40; BingX was down for 5 to 9.
    let exists = |back: i64| (1..=4).contains(&back) || (10..=40).contains(&back);
    let mut requests = 0;
    let candles = page_before_with(
        |path: &str| {
            requests += 1;
            let (end, limit) = asked(path);
            let end_hour = end / HOUR;
            let opens: Vec<i64> = (end_hour - limit + 1..=end_hour)
                .filter(|hour| exists(after_hour - hour))
                .collect();
            Ok(rows_at(&opens))
        },
        now,
        &market(),
        Timeframe::HOUR_1,
        after,
        12,
    )
    .unwrap();
    // The first page holds 7 of the 12 slots; the second, asked from before its oldest, the rest.
    assert_eq!(requests, 2);
    assert_eq!(
        candles
            .iter()
            .map(|candle| after_hour - candle.open_time / HOUR)
            .collect::<Vec<_>>(),
        [17, 16, 15, 14, 13, 12, 11, 10, 4, 3, 2, 1]
    );
}

#[test]
fn a_page_before_the_markets_first_candle_is_empty_and_costs_one_request() {
    let now = 1_000 * DAY;
    let mut requests = 0;
    let candles = page_before_with(
        |_: &str| {
            requests += 1;
            Ok(r#"{"code":100204,"msg":"no data found"}"#.to_owned())
        },
        now,
        &market(),
        Timeframe::HOUR_1,
        now - 500 * DAY,
        1_000,
    )
    .unwrap();
    assert!(candles.is_empty());
    assert_eq!(requests, 1);
}

#[test]
fn the_start_of_the_market_costs_one_request_past_its_first_candle_and_no_more() {
    let now = 1_000 * DAY;
    let after = now - 100 * DAY;
    let first_hour = after / HOUR - 6;
    let mut requests = 0;
    let candles = page_before_with(
        |path: &str| {
            requests += 1;
            let (end, limit) = asked(path);
            let end_hour = end / HOUR;
            let opens: Vec<i64> = (end_hour - limit + 1..=end_hour)
                .filter(|hour| *hour >= first_hour)
                .collect();
            Ok(rows_at(&opens))
        },
        now,
        &market(),
        Timeframe::HOUR_1,
        after,
        500,
    )
    .unwrap();
    assert_eq!(candles.len(), 6);
    assert_eq!(candles[0].open_time, first_hour * HOUR);
    assert_eq!(requests, 2);
}

#[test]
fn a_page_is_no_longer_than_wanted_and_holds_nothing_from_its_end_on() {
    let now = 1_000 * DAY;
    let after = now - 100 * DAY;
    let after_hour = after / HOUR;
    // The answer has 20 candles and one that opens at `after` itself, which is not in the page.
    let opens: Vec<i64> = (after_hour - 19..=after_hour).collect();
    let candles = page_before_with(
        |_: &str| Ok(rows_at(&opens)),
        now,
        &market(),
        Timeframe::HOUR_1,
        after,
        5,
    )
    .unwrap();
    assert_eq!(
        candles
            .iter()
            .map(|bar| bar.open_time / HOUR)
            .collect::<Vec<_>>(),
        (after_hour - 5..after_hour).collect::<Vec<_>>()
    );
}

#[test]
fn a_hole_the_size_of_a_page_ends_the_search_after_four_requests() {
    let now = 1_000 * DAY;
    let mut requests = 0;
    let candles = page_before_with(
        |path: &str| {
            requests += 1;
            let (end, _) = asked(path);
            // One candle a page, however far back it is asked.
            Ok(rows_at(&[end / HOUR]))
        },
        now,
        &market(),
        Timeframe::HOUR_1,
        now - 100 * DAY,
        500,
    )
    .unwrap();
    assert_eq!(requests, 4);
    assert_eq!(candles.len(), 4);
}

#[test]
fn a_length_bingx_has_no_bars_for_is_refused_before_any_request() {
    let error = page_before_with(
        |_: &str| panic!("no request"),
        0,
        &market(),
        tf("1D"),
        1,
        10,
    )
    .unwrap_err();
    assert!(error.is_unsupported_interval());
    // A chart's day folds from natives, so it is served.
    assert!(plan(DataSource::Bingx, tf("1D")).is_ok());
}

// MARK: Markets

const SYMBOLS: &str = r#"{"code":0,"msg":"","retryable":0,"data":{"symbols":[
    {"symbol":"BTC-USDT","minQty":0.0001826,"status":1,"tickSize":0.01,"stepSize":1e-06,"apiStateSell":true,"apiStateBuy":true,"displayName":"BTC-USDT"},
    {"symbol":"PEPE-USDT","status":1,"tickSize":1e-08,"stepSize":1,"displayName":"PEPE-USDT"},
    {"symbol":"BTC-USDC","status":1,"tickSize":0.01},
    {"symbol":"ETH-BTC","status":1,"tickSize":0.000001},
    {"symbol":"ERN_OLD-USDT","status":0,"tickSize":0.001},
    {"symbol":"TITA-USDT","status":10,"tickSize":0.000001},
    {"symbol":"SNDKB-USDT","status":25,"tickSize":0.01},
    {"symbol":"QRDO-OLD-USDT","status":1,"tickSize":0.01},
    {"symbol":"NOTICK-USDT","status":1},
    {"symbol":"NODASH","status":1,"tickSize":0.01},
    {"symbol":"-USDT","status":1,"tickSize":0.01}
]}}"#;

#[test]
fn the_catalog_keeps_the_markets_in_trading_with_the_digits_of_their_prices() {
    let pairs = parse_pairs(SYMBOLS).unwrap();
    assert_eq!(
        pairs
            .iter()
            .map(|pair| (
                pair.id.as_str(),
                pair.base.as_str(),
                pair.quote.as_str(),
                pair.decimals
            ))
            .collect::<Vec<_>>(),
        [
            ("BTC-USDT", "BTC", "USDT", 2),
            ("PEPE-USDT", "PEPE", "USDT", 8),
            ("BTC-USDC", "BTC", "USDC", 2),
            ("ETH-BTC", "ETH", "BTC", 6),
        ]
    );
    assert_eq!(find(&pairs, "PEPE-USDT").map(|pair| pair.decimals), Some(8));
    assert!(find(&pairs, "ERN_OLD-USDT").is_none());
    assert_eq!(
        parse_pairs(r#"{"code":0,"data":{"symbols":[]}}"#),
        Ok(Vec::new())
    );
    assert!(parse_pairs(r#"{"code":0,"data":{}}"#).is_err());
    assert!(parse_pairs("not json").is_err());
}

#[test]
fn a_pair_is_found_however_it_is_separated() {
    let pairs = parse_pairs(SYMBOLS).unwrap();
    for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
        let found = matching_pairs(&pairs, &normalize(query));
        assert_eq!(found.len(), 1, "{query}");
        assert_eq!(found[0].symbol, "BTC-USDT");
        assert_eq!(found[0].quote.as_deref(), Some("USDT"));
    }
    assert_eq!(matching_pairs(&pairs, "BTC").len(), 3);
}

// MARK: Prices

/// What `/openApi/spot/v1/ticker/24hr` answered on 2026-11-21 (the fields used).
const TICKERS: &str = r#"{"code":0,"timestamp":1795246568676,"data":[
    {"symbol":"BTC-USDT","openPrice":61542.79,"highPrice":62333.56,"lowPrice":55159.00,"lastPrice":58255.28,"priceChange":-3287.51,"priceChangePercent":"-5.34%","volume":5034.26,"quoteVolume":296070295.19,"openTime":1795160168676,"closeTime":1795246568676},
    {"symbol":"ETH-BTC","openPrice":0.0279,"lastPrice":0.028,"volume":1200.0,"quoteVolume":33.5,"openTime":1795160168676,"closeTime":1795246568676},
    {"symbol":"DEAD-USDT","openPrice":0.0,"lastPrice":0.0,"volume":0.0,"quoteVolume":0.0,"closeTime":1795246568676},
    {"symbol":"PEPE-USDT","openPrice":3.59e-06,"lastPrice":3.33e-06,"volume":420018173259.0,"quoteVolume":1424754.91,"closeTime":1795246568676}
]}"#;

#[test]
fn tickers_answer_by_the_markets_asked_for_with_the_rolling_reference_and_the_24h_volume() {
    let requests: Vec<QuoteRequest> = ["BTC-USDT", "eth/btc", "PEPE", "DEAD-USDT", "NOPE-USDT"]
        .into_iter()
        .map(request)
        .collect();
    let quotes = parse_tickers(TICKERS, &requests).unwrap();
    let mut asked: Vec<&String> = quotes.keys().collect();
    asked.sort();
    // A market with no price, and one BingX does not list, have no quote.
    assert_eq!(asked, ["BTC-USDT", "PEPE", "eth/btc"]);

    let btc = quotes["BTC-USDT"];
    assert_eq!(btc.price, 58_255.28);
    // `openPrice` is the price 24 hours ago: the window is rolling.
    assert_eq!(btc.previous_day_price, Some(61_542.79));
    assert_eq!(btc.timestamp_ms, Some(1_795_246_568_676));
    // A dollar-quoted pair reports the turnover, any other the units traded.
    assert_eq!(
        (btc.volume_24h, btc.volume_kind),
        (Some(296_070_295.19), Some(VolumeKind::QuoteCurrency))
    );
    let eth = quotes["eth/btc"];
    assert_eq!((eth.price, eth.previous_day_price), (0.028, Some(0.0279)));
    assert_eq!(
        (eth.volume_24h, eth.volume_kind),
        (Some(1_200.0), Some(VolumeKind::Base))
    );
    assert_eq!(quotes["PEPE"].price, 3.33e-06);
}

#[test]
fn a_refused_price_call_is_told_apart_by_its_code() {
    let requests = [request("BTC-USDT")];
    assert_eq!(
        parse_tickers(r#"{"code":100410,"msg":"limit"}"#, &requests),
        Err(QuoteFailure::RateLimited)
    );
    assert_eq!(
        parse_tickers(r#"{"code":100400,"msg":"bad"}"#, &requests),
        Err(QuoteFailure::Failed("BingX: bad".to_owned()))
    );
    assert!(parse_tickers(r#"{"code":0,"data":{}}"#, &requests).is_err());
}

#[test]
fn a_country_refusal_is_the_one_message() {
    assert_eq!(
        BINGX.classify_status(403),
        Some(refused_by_region(DataSource::Bingx))
    );
    assert_eq!(BINGX.classify_status(500), None);
    assert_eq!(
        ProviderError::refused_by_region(DataSource::Bingx).to_string(),
        "BingX refused the request (HTTP 403): it is not available in every country"
    );
}

#[test]
fn a_market_nothing_can_name_is_not_listed() {
    let error = Bingx
        .fetch_klines_range("BTC&USDT", Timeframe::HOUR_1, 0, 1, 10)
        .unwrap_err();
    assert_eq!(
        error,
        ProviderError::unknown_market(DataSource::Bingx, "BTC&USDT", "")
    );
}

// MARK: Sockets

fn gzip(text: &str) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

/// A socket message as the session hands it to a parser: gzip-compressed on the wire, unpacked.
fn on_the_wire(text: &str) -> String {
    Decode::Gzip
        .text(&gzip(text))
        .expect("gzip session")
        .expect("valid gzip")
}

/// What the `BTC-USDT@kline_1min` topic pushed on 2026-11-21.
const PUSH: &str = r#"{"code":0,"data":{"E":1795246660412,"K":{"T":1795246679999,"c":"58269.753000000004","h":"58299.54","i":"1min","l":"58220.221000000005","n":82,"o":"58299.54","q":"97179.584282","s":"BTC-USDT","t":1795246620000,"v":"1.668113000000000000"},"e":"kline","s":"BTC-USDT"},"dataType":"BTC-USDT@kline_1min","success":true,"timestamp":1795246660412}"#;

#[test]
fn a_gzip_push_is_a_candle_with_the_close_of_its_bucket() {
    let text = on_the_wire(PUSH);
    let (market, kline) = pushed_candle(&text, "1min", Timeframe::MINUTE_1).expect("candle");
    assert_eq!(market, "BTC-USDT");
    assert_eq!(kline.open_time, 1_795_246_620_000);
    assert_eq!(kline.close_time, 1_795_246_679_999);
    assert_eq!(
        (kline.open, kline.high, kline.low, kline.close),
        (
            58_299.54,
            58_299.54,
            58_220.221000000005,
            58_269.753000000004
        )
    );
    // The base asset, not the quote turnover in `q`.
    assert_eq!(kline.volume, 1.668113);
}

#[test]
fn acknowledgements_refusals_pings_and_other_topics_are_not_candles() {
    for ignored in [
        r#"{"code":0,"id":"degenview","msg":"SUCCESS","timestamp":1795246659175}"#,
        r#"{"code":100400,"id":"degenview","msg":"dataType is error: NOPE-USDT@kline_1min","timestamp":1795246659175}"#,
        r#"{"code":100416,"id":"degenview","msg":"Failed, one websocket can subscribe to a maximum of 200 data points."}"#,
        r#"{"ping":"5795035fdea241aa82008457e74b20c5","time":"2026-11-21T15:38:31.327+0800"}"#,
        r#"{"code":0,"data":{"e":"24hTicker","s":"BTC-USDT","c":1.0},"dataType":"BTC-USDT@ticker"}"#,
        // Another length than the one asked for.
        &PUSH.replace(r#""i":"1min""#, r#""i":"5min""#),
        &PUSH.replace(r#""o":"58299.54""#, r#""o":"x""#),
        r#"{"code":0,"data":{"e":"kline","s":"BTC-USDT"}}"#,
        "not json",
    ] {
        assert!(
            pushed_candle(ignored, "1min", Timeframe::MINUTE_1).is_none(),
            "{ignored}"
        );
    }
}

#[test]
fn the_reader_cuts_a_push_at_the_markets_tick() {
    use crate::service::market::live::FrameReader;
    use crate::service::net::ws::Frame;
    let decimals = HashMap::from([("BTC-USDT".to_owned(), 2)]);
    let mut candles = live::Candles::new(Timeframe::MINUTE_1, decimals).unwrap();
    let (_, kline) = candles.read(Frame::Text(PUSH)).expect("candle");
    assert_eq!(
        (kline.open, kline.high, kline.low, kline.close),
        (58_299.54, 58_299.54, 58_220.22, 58_269.75)
    );
    // A market the list does not have is used as it comes.
    let (_, other) = candles
        .read(Frame::Text(&PUSH.replace("BTC-USDT", "NEW-USDT")))
        .expect("candle");
    assert_eq!(other.close, 58_269.753000000004);
    // A length with no socket name has no reader.
    assert!(live::Candles::new(Timeframe::DAY_1, HashMap::new()).is_none());
}

#[test]
fn the_reader_asks_for_the_digits_only_while_it_has_none_and_a_reconnect_does_not() {
    use crate::service::market::live::FrameReader;
    use crate::service::net::ws::Frame;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ASKED: AtomicUsize = AtomicUsize::new(0);
    static LISTED: AtomicUsize = AtomicUsize::new(0);
    fn learn() -> HashMap<String, u32> {
        ASKED.fetch_add(1, Ordering::SeqCst);
        if LISTED.load(Ordering::SeqCst) == 0 {
            HashMap::new()
        } else {
            HashMap::from([("BTC-USDT".to_owned(), 2)])
        }
    }
    let mut candles = live::Candles::new(Timeframe::MINUTE_1, HashMap::new()).unwrap();
    candles.learn = learn;

    // The list is not held yet: the prices come as they are, and each push asks again.
    let (_, uncut) = candles.read(Frame::Text(PUSH)).expect("candle");
    assert_eq!(uncut.close, 58_269.753000000004);
    candles.reset();
    candles.read(Frame::Text(PUSH)).expect("candle");
    assert_eq!(ASKED.load(Ordering::SeqCst), 2);

    // Once it is, the digits are read once and used from then on, whatever the connection does.
    LISTED.store(1, Ordering::SeqCst);
    let (_, cut) = candles.read(Frame::Text(PUSH)).expect("candle");
    assert_eq!(cut.close, 58_269.75);
    candles.reset();
    candles.read(Frame::Text(PUSH)).expect("candle");
    candles.read(Frame::Text(PUSH)).expect("candle");
    assert_eq!(ASKED.load(Ordering::SeqCst), 3);
}

#[test]
fn the_digits_of_a_list_are_read_by_market() {
    let pairs = parse_pairs(SYMBOLS).unwrap();
    let decimals = decimals_of(&pairs);
    assert_eq!(decimals.len(), pairs.len());
    assert_eq!(decimals.get("PEPE-USDT"), Some(&8));
}

#[test]
fn every_market_is_its_own_topic_and_a_connection_takes_two_hundred() {
    let sessions = Bingx
        .sessions(
            &[
                "BTC-USDT".into(),
                "eth/usdt".into(),
                "sol".into(),
                "BTC-USDT".into(),
            ],
            Timeframe::HOUR_1,
        )
        .unwrap();
    assert_eq!(sessions.len(), 1);
    let session = &sessions[0];
    assert_eq!(session.url, "wss://open-api-ws.bingx.com/market");
    assert_eq!(session.decode, Decode::Gzip);
    let topics: Vec<String> = session
        .subscribe
        .iter()
        .map(|message| {
            serde_json::from_str::<Value>(message).unwrap()["dataType"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        topics,
        [
            "BTC-USDT@kline_60min",
            "ETH-USDT@kline_60min",
            "SOL-USDT@kline_60min"
        ]
    );

    let many: Vec<String> = (0..450).map(|index| format!("COIN{index}-USDT")).collect();
    let sessions = Bingx.sessions(&many, Timeframe::MINUTE_1).unwrap();
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.subscribe.len())
            .collect::<Vec<_>>(),
        [TOPICS_PER_CONNECTION, TOPICS_PER_CONNECTION, 50]
    );
    assert!(socket::sessions(Vec::new()).is_empty());
}

#[test]
fn a_length_bingx_has_no_socket_name_for_has_no_session_and_unnameable_text_is_not_subscribed() {
    for other in ["6h", "12h", "1D", "1W", "22m"] {
        assert!(
            Bingx.sessions(&["BTC-USDT".into()], tf(other)).is_none(),
            "{other}"
        );
    }
    assert!(
        Bingx
            .sessions(&["BTC&USDT".into(), "".into()], Timeframe::MINUTE_1)
            .is_none()
    );
}

#[test]
fn the_servers_json_ping_is_answered_with_pong_and_never_data() {
    let session = socket::session(Vec::new());
    assert_eq!(session.decode, Decode::Gzip);
    let ping = on_the_wire(
        r#"{"ping":"5795035fdea241aa82008457e74b20c5","time":"2026-11-21T15:38:31.327+0800"}"#,
    );
    assert_eq!(
        format!("{:?}", session.keepalive.inspect(&ping)),
        r#"Answer("Pong")"#
    );
    // Acknowledgements, refusals and data are data.
    for data in [
        r#"{"code":0,"id":"degenview","msg":"SUCCESS","timestamp":1795246659175}"#,
        PUSH,
    ] {
        assert_eq!(format!("{:?}", session.keepalive.inspect(data)), "Data");
    }
    assert!(matches!(session.keepalive, Keepalive::AnswerJson { .. }));
}

// MARK: Network

/// Run with `cargo test --lib bingx::tests::live_bingx_serves -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_bingx_serves_candles_pages_back_and_streams() {
    use crate::service::market::live::LiveFeed;
    use crate::service::watchlist_quotes::quote_stream::QuoteSocket;
    use std::time::{Duration, Instant};

    let consecutive = |candles: &[Kline], step: i64| {
        candles
            .windows(2)
            .all(|pair| pair[1].open_time - pair[0].open_time == step)
    };

    // The newest page is the recent endpoint's; its newest candle is the forming one.
    let hourly = BINGX
        .fetch_klines("BTC-USDT", Timeframe::HOUR_1, 500)
        .unwrap();
    println!(
        "BTC-USDT 1h: {} candles, newest opens {}",
        hourly.len(),
        hourly.last().unwrap().open_time
    );
    assert_eq!(hourly.len(), 500);
    assert!(consecutive(&hourly, HOUR));
    assert!(hourly.last().unwrap().close_time > crate::clock::now_ms());
    assert!(
        hourly
            .iter()
            .all(|candle| candle.volume >= 0.0 && candle.close > 0.0)
    );

    // Past 400 days: pages that only the history endpoint has, joined to the recent ones.
    let long = BINGX
        .fetch_klines("BTC-USDT", Timeframe::HOUR_1, 10_000)
        .unwrap();
    println!(
        "BTC-USDT 1h long: {} candles from {} ({} days)",
        long.len(),
        long[0].open_time,
        (crate::clock::now_ms() - long[0].open_time) / DAY
    );
    assert_eq!(long.len(), 10_000);
    assert!(consecutive(&long, HOUR));
    assert!((crate::clock::now_ms() - long[0].open_time) / DAY > 400);
    // No price has a digit past the tick, the history's included.
    assert!(long.iter().all(|candle| {
        [candle.open, candle.high, candle.low, candle.close]
            .iter()
            .all(|price| cut(*price, 2) == *price)
    }));

    // The page before the oldest joins it.
    let oldest = long[0].open_time;
    let older = BINGX
        .fetch_klines_range("BTC-USDT", Timeframe::HOUR_1, 0, oldest, 500)
        .unwrap();
    assert_eq!(older.len(), 500);
    assert_eq!(older.last().unwrap().open_time, oldest - HOUR);

    // The two endpoints are one bar where they overlap: the history's prices, cut, are the recent
    // endpoint's.
    let now = crate::clock::now_ms();
    let page = |from: &'static str, to: &'static str| {
        page_before_with(
            |path: &str| rest::get(&path.replace(from, to)),
            // Far enough in the past that every page is asked of the endpoint named by `from`.
            now,
            &market(),
            Timeframe::MINUTE_1,
            now - 3 * DAY,
            600,
        )
        .unwrap()
    };
    let recent = page("/openApi/market/his/v1/", "/openApi/spot/v2/market/");
    let history = page("/openApi/spot/v2/market/", "/openApi/market/his/v1/");
    assert_eq!((recent.len(), history.len()), (600, 600));
    for (recent, history) in recent.iter().zip(&history) {
        assert_eq!(recent.open_time, history.open_time);
        assert_eq!(
            (recent.open, recent.high, recent.low, recent.close),
            (history.open, history.high, history.low, history.close),
            "{}",
            recent.open_time
        );
        assert!((recent.volume - history.volume).abs() <= recent.volume * 1e-9);
    }

    // One-minute candles reach back to the start of 2025 and no further.
    let minutes = BINGX
        .fetch_klines("BTC-USDT", Timeframe::MINUTE_1, 30_000)
        .unwrap();
    println!(
        "BTC-USDT 1m: {} candles from {}",
        minutes.len(),
        minutes[0].open_time
    );
    assert_eq!(minutes.len(), 30_000);
    assert!(consecutive(&minutes, 60_000));

    // Every native opens where this app cuts it.
    for native in DataSource::Bingx.natives() {
        let candles = BINGX.fetch_klines("BTC-USDT", *native, 30).unwrap();
        assert_eq!(candles.len(), 30, "{native}");
        for candle in &candles {
            assert_eq!(
                bucket_start_ms(*native, candle.open_time),
                candle.open_time,
                "{native}"
            );
        }
    }

    // What it cuts elsewhere folds: this app's day is the sum of the hours between its midnights.
    for (timeframe, count) in [
        ("1D", 1_000),
        ("1W", 100),
        ("1M", 24),
        ("6h", 800),
        ("12h", 400),
    ] {
        let folded = BINGX
            .fetch_klines("BTC-USDT", tf(timeframe), count)
            .unwrap();
        println!(
            "BTC-USDT {timeframe} folded: {} candles from {}",
            folded.len(),
            folded[0].open_time
        );
        assert_eq!(folded.len(), count as usize, "{timeframe}");
        assert!(
            folded
                .iter()
                .all(|bar| bucket_start_ms(tf(timeframe), bar.open_time) == bar.open_time),
            "{timeframe}"
        );
    }

    let unknown = BINGX.fetch_klines("NOPE-USDT", Timeframe::HOUR_1, 5);
    println!("NOPE-USDT: {unknown:?}");
    assert_eq!(
        unknown,
        Err(ProviderError::unknown_market(
            DataSource::Bingx,
            "NOPE-USDT",
            ""
        ))
    );

    let hits = BINGX.search_tickers("btc").unwrap();
    println!(
        "search btc: {:?}",
        hits.iter()
            .map(|hit| hit.symbol.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(hits[0].full_symbol, "BTC-USDT");
    assert_eq!(hits[0].quote.as_deref(), Some("USDT"));

    let requests: Vec<QuoteRequest> = ["BTC-USDT", "ETH-USDT", "NOPE-USDT"]
        .into_iter()
        .map(request)
        .collect();
    let outcome = BINGX.rest_quotes(&requests).unwrap();
    for symbol in ["BTC-USDT", "ETH-USDT"] {
        let quote = outcome
            .quotes
            .get(symbol)
            .unwrap_or_else(|| panic!("no REST quote for {symbol}"));
        println!("REST quote {symbol}: {quote:?}");
        assert!(quote.price > 0.0 && quote.previous_day_price.is_some());
    }
    assert!(!outcome.quotes.contains_key("NOPE-USDT") && outcome.failed.is_empty());

    // A market BingX does not list must not silence the others on the socket.
    let feed = LiveFeed::connect(
        DataSource::Bingx,
        vec![
            "NOPE-USDT".to_owned(),
            "BTC-USDT".to_owned(),
            "ETH-USDT".to_owned(),
        ],
        Timeframe::MINUTE_1,
        eframe::egui::Context::default(),
    )
    .expect("BingX streams one-minute candles");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _quotes = QuoteSocket::connect(
        DataSource::Bingx,
        vec!["NOPE-USDT".to_owned(), "BTC-USDT".to_owned()],
        sender,
    )
    .expect("BingX has a ticker socket");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = std::collections::BTreeSet::new();
    while seen.len() < 2 {
        for (symbol, kline) in feed.inbox.drain() {
            assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
            assert_eq!(
                cut(kline.close, 2),
                kline.close,
                "a push is cut at the tick"
            );
            if !seen.contains(&symbol) {
                println!("socket candle {symbol}: {kline:?}");
            }
            seen.insert(symbol);
        }
        assert!(Instant::now() < deadline, "only {seen:?} in 30 s");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        ["BTC-USDT", "ETH-USDT"]
    );
    let tick = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a quote tick in 30 s");
    println!("socket quote {}: {:?}", tick.product_id, tick.quote);
    assert_eq!(tick.product_id, "BTC-USDT");
    assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
}

/// A socket that answers the server's pings is still the same connection after 65 seconds, which
/// hold two of them (the server pings every 30 seconds, the first within 25). Run with
/// `cargo test --lib bingx::tests::live_bingx_pings -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_bingx_pings_are_answered_for_sixty_five_seconds() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let session = socket::session(vec![subscribe(&kline_topic("BTC-USDT", "1min"))]);
    let shutdown = Arc::new(AtomicBool::new(false));
    let links = Arc::new(Mutex::new(Vec::new()));
    let frames = Arc::new(Mutex::new(0_u32));
    let thread = {
        let (shutdown, links, frames) = (shutdown.clone(), links.clone(), frames.clone());
        std::thread::spawn(move || {
            session.run_linked(
                &shutdown,
                |up| crate::lock::lock(&links).push(up),
                |_| {
                    *crate::lock::lock(&frames) += 1;
                    true
                },
            );
        })
    };
    std::thread::sleep(Duration::from_secs(65));
    let links_after_65_seconds = crate::lock::lock(&links).clone();
    println!(
        "after 65 s: link events {links_after_65_seconds:?}, {} data frames",
        *crate::lock::lock(&frames)
    );
    shutdown.store(true, Ordering::Release);
    thread.join().unwrap();
    // Connected once and never dropped.
    assert_eq!(links_after_65_seconds, [true]);
}

/// BingX cuts the same buckets as Binance at every timeframe the chart offers, the ones it folds
/// (days, weeks, months, years among them) included. Run with
/// `cargo test --lib bingx::tests::live_bingx_buckets -- --ignored --nocapture`; the all-venues
/// comparison is `live_sources_cut_the_same_buckets`.
#[test]
#[ignore = "needs the network"]
fn live_bingx_buckets_match_binances_at_every_timeframe() {
    let odd = ["5m", "22m", "3h", "4h", "6h", "12h", "2D", "3D", "2W", "6M"].map(tf);
    for interval in Timeframe::LEGACY.into_iter().chain(odd) {
        let opens = |source, symbol: &str| -> std::collections::BTreeSet<i64> {
            crate::service::fetch_klines(source, symbol, interval, 500)
                .unwrap_or_else(|error| panic!("{source:?} {interval}: {error}"))
                .iter()
                .map(|bar| bar.open_time)
                .collect()
        };
        let (binance, bingx) = (
            opens(DataSource::Binance, "ETHUSDT"),
            opens(DataSource::Bingx, "ETH-USDT"),
        );
        // Compare inside the range both cover.
        let from = binance.first().max(bingx.first()).copied().unwrap();
        let within = |set: &std::collections::BTreeSet<i64>| -> Vec<i64> {
            set.iter().copied().filter(|time| *time >= from).collect()
        };
        println!(
            "{interval:>3}: binance {:>4} bingx {:>4} | shared range {:>4} bars",
            binance.len(),
            bingx.len(),
            within(&binance).len()
        );
        assert!(within(&bingx).len() > 2, "{interval}");
        assert_eq!(
            within(&binance),
            within(&bingx),
            "{interval} binance vs bingx"
        );
    }
}
