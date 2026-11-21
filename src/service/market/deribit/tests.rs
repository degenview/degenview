use super::live::{candle_channels, parse_candle};
use super::markets::{
    instrument_name, matching, not_listed, parse_markets, require_name, settlement_of,
};
use super::parse::{answer, parse_candles};
use super::quotes::{parse_tick, ticker_channels};
use super::resolution::resolution;
use super::rest::{PAGING, page_before_with};
use super::ticker::{fetch_quotes_with, parse_summaries};
use super::*;
use crate::model::InstrumentId;
use crate::model::market::MarketKind;
use crate::model::market::bucket::bucket_start_ms;
use crate::model::watchlist;
use crate::model::watchlist::quote::VolumeKind;
use crate::service::net::http;
use crate::service::net::ws::Keepalive;
use crate::service::watchlist_quotes::provider::request;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

#[test]
fn every_native_has_its_resolution_and_nothing_else_does() {
    let resolutions: Vec<String> = DataSource::Deribit
        .natives()
        .iter()
        .map(|native| resolution(*native).expect("a native has a resolution"))
        .collect();
    assert_eq!(resolutions, ["1", "3", "5", "10", "15", "30", "60", "120"]);
    // Deribit answers 180, 360, 720 and 1D, but they open off this app's grid, and `240`, `1W` and
    // `1M` are refused: these fold from the two-hour bars.
    for other in [
        "4m", "3h", "4h", "6h", "12h", "1d", "1w", "1M", "3M", "1Y", "22m",
    ] {
        assert_eq!(resolution(tf(other)), None, "{other}");
    }
}

/// The oldest bar of each resolution the API answered for BTC-PERPETUAL on 2026-11-21 (`ticks[0]`
/// of the newest 5,001): the natives open where this app cuts, the others do not (180 and 360 at
/// 02:00 UTC, 720 and 1D at 08:00 UTC).
#[test]
fn the_bars_deribit_opens_are_on_this_apps_grid_only_up_to_two_hours() {
    for (resolution, open) in [
        ("1m", 1_794_937_080_000),
        ("3m", 1_794_337_020_000),
        ("5m", 1_793_736_900_000),
        ("10m", 1_792_236_600_000),
        ("15m", 1_790_736_300_000),
        ("30m", 1_786_235_400_000),
        ("1h", 1_777_233_600_000),
        ("2h", 1_759_233_600_000),
    ] {
        let native = tf(resolution);
        assert!(DataSource::Deribit.natives().contains(&native));
        assert_eq!(bucket_start_ms(native, open), open, "{resolution}");
    }
    for (resolution, open) in [
        ("3h", 1_741_226_400_000),
        ("6h", 1_687_226_400_000),
        ("12h", 1_579_204_800_000),
        ("1d", 1_534_233_600_000),
    ] {
        let own = tf(resolution);
        assert_ne!(bucket_start_ms(own, open), open, "{resolution}");
    }
}

fn base_of(timeframe: &str) -> (String, usize) {
    let plan = plan(DataSource::Deribit, tf(timeframe)).unwrap();
    (resolution(plan.base()).unwrap(), plan.base_count(500))
}

#[test]
fn what_it_cannot_cut_folds_the_coarsest_native_that_nests() {
    assert_eq!(base_of("1h"), ("60".to_owned(), 500));
    assert_eq!(base_of("2h"), ("120".to_owned(), 500));
    assert_eq!(base_of("3h"), ("60".to_owned(), 501 * 3));
    assert_eq!(base_of("4h"), ("120".to_owned(), 501 * 2));
    assert_eq!(base_of("12h"), ("120".to_owned(), 501 * 6));
    assert_eq!(base_of("1D"), ("120".to_owned(), 501 * 12));
    assert_eq!(base_of("1W"), ("120".to_owned(), 501 * 84));
    assert_eq!(base_of("22m"), ("1".to_owned(), 501 * 22));
    assert_eq!(base_of("4m"), ("1".to_owned(), 501 * 4));
}

const CANDLES: &str = r#"{"usOut":1795237105303681,"usIn":1795237105301595,"usDiff":2086,"testnet":false,"result":{"volume":[487.74800917,1216.36945376,953.84095102],"ticks":[1795215600000,1795219200000,1795222800000],"status":"ok","open":[59032.0,58810.5,58913.0],"low":[58723.0,58742.0,58782.0],"high":[59167.5,59054.0,59130.0],"cost":[28740980.0,71693150.0,56208400.0],"close":[58815.5,58912.5,58914.0]},"jsonrpc":"2.0"}"#;

#[test]
fn parses_candles_oldest_first_each_closing_where_its_bucket_does() {
    let candles = parse_candles(CANDLES, Timeframe::HOUR_1).unwrap();
    let opens: Vec<i64> = candles.iter().map(|candle| candle.open_time).collect();
    assert_eq!(
        opens,
        [1_795_215_600_000, 1_795_219_200_000, 1_795_222_800_000]
    );
    let first = &candles[0];
    assert_eq!(first.close_time, first.open_time + HOUR - 1);
    // The volume is the base asset (the coins), not `cost`, which is dollars.
    assert_eq!(
        (first.open, first.high, first.low, first.close, first.volume),
        (59032.0, 59167.5, 58723.0, 58815.5, 487.74800917)
    );
}

#[test]
fn a_window_with_no_bar_is_no_candles_not_an_error() {
    let body = r#"{"jsonrpc":"2.0","result":{"volume":[],"ticks":[],"status":"no_data","open":[],"low":[],"high":[],"cost":[],"close":[]}}"#;
    assert_eq!(parse_candles(body, Timeframe::HOUR_1), Ok(Vec::new()));
}

#[test]
fn an_answer_that_is_a_refusal_or_not_candles_is_an_error_naming_it() {
    let unknown = r#"{"jsonrpc":"2.0","error":{"message":"Invalid params","data":{"reason":"instrument not found","param":"instrument_name"},"code":-32602}}"#;
    assert_eq!(
        answer(unknown),
        Err(ProviderError::other(
            "Deribit answered -32602: Invalid params (instrument not found)"
        ))
    );
    let bare = r#"{"jsonrpc":"2.0","error":{"message":"Internal error","code":11050}}"#;
    assert_eq!(
        answer(bare),
        Err(ProviderError::other(
            "Deribit answered 11050: Internal error"
        ))
    );
    let unsigned = r#"{"jsonrpc":"2.0","error":{"message":"gone"}}"#;
    assert_eq!(
        answer(unsigned),
        Err(ProviderError::other("Deribit answered: gone"))
    );
    // The allowance used up is a rate limit, as an HTTP 429 is: decided on the variant.
    let limited = r#"{"jsonrpc":"2.0","error":{"message":"too_many_requests","code":10028}}"#;
    assert!(answer(limited).unwrap_err().is_rate_limited());
    for body in [
        "",
        "<html>",
        r#"{"jsonrpc":"2.0"}"#,
        r#"{"jsonrpc":"2.0","result":{"status":"ok"}}"#,
        r#"{"jsonrpc":"2.0","result":{"ticks":[1],"open":[1],"high":[1],"low":[1],"close":[1]}}"#,
        r#"{"jsonrpc":"2.0","result":{"ticks":[1],"open":[1],"high":[1],"low":[1],"close":[1],"volume":[]}}"#,
        r#"{"jsonrpc":"2.0","result":{"ticks":["x"],"open":[1],"high":[1],"low":[1],"close":[1],"volume":[1]}}"#,
        r#"{"jsonrpc":"2.0","result":{"ticks":[1],"open":[null],"high":[1],"low":[1],"close":[1],"volume":[1]}}"#,
    ] {
        assert!(
            matches!(
                parse_candles(body, Timeframe::HOUR_1),
                Err(ProviderError::Data(_))
            ),
            "{body}"
        );
    }
}

const INSTRUMENTS: &str = r#"{"jsonrpc":"2.0","result":[
    {"instrument_name":"SOL_USDC-PERPETUAL","settlement_period":"perpetual","is_active":true,"base_currency":"SOL","quote_currency":"USDC","settlement_currency":"USDC","kind":"future"},
    {"instrument_name":"BTC-27MAR26","settlement_period":"month","is_active":true,"base_currency":"BTC","quote_currency":"USD","settlement_currency":"BTC","kind":"future"},
    {"instrument_name":"BTC_USDC-PERPETUAL","settlement_period":"perpetual","is_active":true,"base_currency":"BTC","quote_currency":"USDC","settlement_currency":"USDC","kind":"future"},
    {"instrument_name":"BTC-PERPETUAL","settlement_period":"perpetual","is_active":true,"base_currency":"BTC","quote_currency":"USD","settlement_currency":"BTC","kind":"future"},
    {"instrument_name":"OLD_USDC-PERPETUAL","settlement_period":"perpetual","is_active":false,"base_currency":"OLD","quote_currency":"USDC","settlement_currency":"USDC","kind":"future"},
    {"instrument_name":"ETH-3DEC26","settlement_period":"week","is_active":true,"kind":"future"},
    {"settlement_period":"perpetual","is_active":true},
    {"instrument_name":"SPY_USDC-PERPETUAL","settlement_period":"perpetual","is_active":true,"kind":"future"}]}"#;

fn names() -> Vec<String> {
    parse_markets(INSTRUMENTS).unwrap()
}

#[test]
fn only_the_active_perpetuals_with_a_name_are_listed_in_name_order() {
    assert_eq!(
        names(),
        [
            "BTC-PERPETUAL",
            "BTC_USDC-PERPETUAL",
            "SOL_USDC-PERPETUAL",
            "SPY_USDC-PERPETUAL"
        ]
    );
    for body in [
        "",
        r#"{"result":{}}"#,
        r#"{"error":{"code":1,"message":"x"}}"#,
    ] {
        assert!(parse_markets(body).is_err(), "{body}");
    }
}

#[test]
fn a_name_is_asked_for_in_upper_case_and_only_when_it_could_be_one() {
    assert_eq!(
        instrument_name(" sol_usdc-perpetual "),
        Some("SOL_USDC-PERPETUAL".to_owned())
    );
    assert_eq!(
        instrument_name("BTC-PERPETUAL"),
        Some("BTC-PERPETUAL".to_owned())
    );
    // Nothing that could change a request's query is ever put in one.
    for bad in [
        "",
        "  ",
        "BTC PERPETUAL",
        "BTC&resolution=1",
        "BTC/USD",
        "BTC-ПЕР",
        "a?b",
    ] {
        assert_eq!(instrument_name(bad), None, "{bad:?}");
        assert_eq!(require_name(bad), Err(not_listed(bad)), "{bad:?}");
    }
    assert!(
        not_listed("NOPE")
            .to_string()
            .starts_with("Deribit does not list NOPE;")
    );
}

#[test]
fn a_perpetual_is_settled_in_its_coin_or_in_its_quote() {
    assert_eq!(settlement_of("BTC-PERPETUAL"), Some("BTC"));
    assert_eq!(settlement_of("ETH-PERPETUAL"), Some("ETH"));
    assert_eq!(settlement_of("SOL_USDC-PERPETUAL"), Some("USDC"));
    assert_eq!(settlement_of("1000PEPE_USDC-PERPETUAL"), Some("USDC"));
    for other in [
        "BTC-27MAR26",
        "BTC",
        "-PERPETUAL",
        "_USDC",
        "BTC_-PERPETUAL",
        "",
    ] {
        assert_eq!(settlement_of(other), None, "{other}");
    }
}

#[test]
fn search_finds_a_market_by_the_letters_and_digits_of_its_name_and_tags_it_a_perpetual() {
    let names = names();
    let symbols = |needle: &str| -> Vec<String> {
        matching(&names, needle)
            .into_iter()
            .map(|hit| hit.symbol)
            .collect()
    };
    assert_eq!(symbols("BTC"), ["BTC-PERPETUAL", "BTC_USDC-PERPETUAL"]);
    assert_eq!(symbols("SOLUSDC"), ["SOL_USDC-PERPETUAL"]);
    assert_eq!(symbols("SPY"), ["SPY_USDC-PERPETUAL"]);
    assert!(symbols("NOPE").is_empty());
    let hit = &matching(&names, "SOL")[0];
    assert_eq!(hit.source, DataSource::Deribit);
    assert_eq!(hit.full_symbol, "SOL_USDC-PERPETUAL");
    assert_eq!(hit.kind, MarketKind::Perp);
}

#[test]
fn an_empty_query_searches_nothing_and_asks_nothing() {
    assert_eq!(DERIBIT.search_tickers("  /"), Ok(Vec::new()));
}

/// The number the query of a candle request gives `name`.
fn query_param(path: &str, name: &str) -> i64 {
    path.split_once('?')
        .expect("a query")
        .1
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("{name} is required: {path}"))
        .parse()
        .unwrap()
}

/// What the API holds of BTC-PERPETUAL's hours as the chart-data request answers them: the bars
/// that open inside `[start_timestamp, end_timestamp]` (both ends included), the newest 5,001.
fn deribit_hours(
    first_open: i64,
    last_open: i64,
    asked: &std::cell::RefCell<Vec<String>>,
) -> impl Fn(&str) -> Result<String, ProviderError> + '_ {
    move |path| {
        asked.borrow_mut().push(path.to_owned());
        assert!(path.starts_with(
            "/get_tradingview_chart_data?instrument_name=BTC-PERPETUAL&resolution=60&"
        ));
        let (start, end) = (
            query_param(path, "start_timestamp"),
            query_param(path, "end_timestamp"),
        );
        let opens: Vec<i64> = (first_open / HOUR..=last_open / HOUR)
            .map(|hour| hour * HOUR)
            .filter(|open| (start..=end).contains(open))
            .collect();
        let newest = &opens[opens.len().saturating_sub(5_001)..];
        let list = |value: &str| vec![value.to_owned(); newest.len()].join(",");
        Ok(format!(
            r#"{{"jsonrpc":"2.0","result":{{"ticks":[{}],"open":[{o}],"high":[{h}],"low":[{l}],"close":[{o}],"volume":[{o}],"cost":[{o}],"status":"ok"}}}}"#,
            newest
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(","),
            o = list("1.5"),
            h = list("2"),
            l = list("0.5"),
        ))
    }
}

#[test]
fn history_is_paged_back_by_moving_the_end_to_just_before_the_oldest_bar() {
    let asked = std::cell::RefCell::new(Vec::new());
    let now = 50_000 * HOUR + 1_234;
    let history = deribit_hours(1_000 * HOUR, 50_000 * HOUR, &asked);
    let plan = plan(DataSource::Deribit, Timeframe::HOUR_1).unwrap();
    let candles = PAGING
        .back(0, now, 12_000, |after, wanted| {
            page_before_with(&history, "btc-perpetual", plan.base(), after, wanted)
        })
        .unwrap();
    assert_eq!(candles.len(), 12_000);
    assert_eq!(candles.last().unwrap().open_time, 50_000 * HOUR);
    assert!(
        candles
            .windows(2)
            .all(|pair| pair[1].open_time - pair[0].open_time == HOUR)
    );
    let windows: Vec<(i64, i64)> = asked
        .borrow()
        .iter()
        .map(|path| {
            (
                query_param(path, "start_timestamp"),
                query_param(path, "end_timestamp"),
            )
        })
        .collect();
    // Each window ends a millisecond before the oldest bar of the page after it (the end is
    // inclusive) and starts as many lengths before that as the page is asked to hold.
    assert_eq!(
        windows,
        [
            (now - 5_000 * HOUR, now - 1),
            (45_001 * HOUR - 5_000 * HOUR, 45_001 * HOUR - 1),
            (40_001 * HOUR - 2_000 * HOUR, 40_001 * HOUR - 1)
        ]
    );
}

#[test]
fn a_market_that_began_later_ends_the_paging_with_a_short_page() {
    let asked = std::cell::RefCell::new(Vec::new());
    let history = deribit_hours(4_800 * HOUR, 5_000 * HOUR, &asked);
    let candles = PAGING
        .back(0, 5_001 * HOUR, 4_000, |after, wanted| {
            page_before_with(&history, "BTC-PERPETUAL", Timeframe::HOUR_1, after, wanted)
        })
        .unwrap();
    assert_eq!(candles.len(), 201);
    assert_eq!(candles[0].open_time, 4_800 * HOUR);
    assert_eq!(
        asked.borrow().len(),
        1,
        "a short page is the start of the market"
    );
}

#[test]
fn a_page_before_the_markets_first_bar_is_empty_not_an_error() {
    let no_data = |_: &str| {
        Ok(r#"{"jsonrpc":"2.0","result":{"volume":[],"ticks":[],"status":"no_data","open":[],"low":[],"high":[],"cost":[],"close":[]}}"#.to_owned())
    };
    assert_eq!(
        page_before_with(no_data, "BTC-PERPETUAL", Timeframe::HOUR_1, 1_000, 500),
        Ok(Vec::new())
    );
}

#[test]
fn a_refresh_asks_a_window_of_the_few_bars_it_wants() {
    let asked = std::cell::RefCell::new(Vec::new());
    let history = deribit_hours(1_000 * HOUR, 5_000 * HOUR, &asked);
    let candles = page_before_with(
        &history,
        "BTC-PERPETUAL",
        Timeframe::HOUR_1,
        5_001 * HOUR,
        4,
    )
    .unwrap();
    assert_eq!(asked.borrow().len(), 1);
    let path = &asked.borrow()[0];
    assert_eq!(
        query_param(path, "end_timestamp") - query_param(path, "start_timestamp"),
        4 * HOUR - 1
    );
    assert_eq!(
        candles
            .iter()
            .map(|candle| candle.open_time)
            .collect::<Vec<_>>(),
        [4_997 * HOUR, 4_998 * HOUR, 4_999 * HOUR, 5_000 * HOUR]
    );
}

#[test]
fn the_window_never_starts_before_the_epoch() {
    let asked = std::cell::RefCell::new(Vec::new());
    let history = deribit_hours(0, 10 * HOUR, &asked);
    page_before_with(&history, "BTC-PERPETUAL", Timeframe::HOUR_1, 3 * HOUR, 500).unwrap();
    assert_eq!(query_param(&asked.borrow()[0], "start_timestamp"), 0);
}

#[test]
fn a_length_deribit_has_no_candle_for_is_unsupported_not_asked() {
    let never = |path: &str| -> Result<String, ProviderError> { panic!("asked {path}") };
    for other in ["3h", "12h", "1d", "1w"] {
        assert_eq!(
            page_before_with(never, "BTC-PERPETUAL", tf(other), 1_000 * HOUR, 5),
            Err(ProviderError::unsupported(DataSource::Deribit, tf(other))),
            "{other}"
        );
    }
}

#[test]
fn text_that_is_no_name_is_not_a_market_and_is_never_asked() {
    let never = |path: &str| -> Result<String, ProviderError> { panic!("asked {path}") };
    assert_eq!(
        page_before_with(never, "BTC&x=1", Timeframe::HOUR_1, 1_000 * HOUR, 5),
        Err(not_listed("BTC&x=1"))
    );
}

#[test]
fn a_refusal_or_a_failed_request_is_the_answer_of_the_page() {
    let asked = |error: ProviderError| move |_: &str| Err(error.clone());
    let page = |ask: &dyn Fn(&str) -> Result<String, ProviderError>,
                symbol: &str|
     -> Result<Vec<Kline>, ProviderError> {
        page_before_with(ask, symbol, Timeframe::HOUR_1, 1_000 * HOUR, 5)
    };
    // An HTTP 400 is the instrument it names, which Deribit does not list.
    let unknown = page(
        &asked(ProviderError::Network(http::Error::Status(400))),
        "NOPE-PERPETUAL",
    );
    assert_eq!(unknown, Err(not_listed("NOPE-PERPETUAL")));
    assert!(!unknown.unwrap_err().is_rate_limited());
    // A 429 stays one, for the caller that backs off; so does a failed connection.
    let limited = page(
        &asked(ProviderError::Network(http::Error::Status(429))),
        "BTC-PERPETUAL",
    );
    assert!(limited.unwrap_err().is_rate_limited());
    let down = ProviderError::Network(http::Error::Network("connection reset".to_owned()));
    assert_eq!(page(&asked(down.clone()), "BTC-PERPETUAL"), Err(down));
    // The same refusals as an answer body of HTTP 200.
    let refused = |body: &'static str| move |_: &str| Ok(body.to_owned());
    let busy = page(
        &refused(r#"{"error":{"message":"too_many_requests","code":10028}}"#),
        "BTC-PERPETUAL",
    );
    assert!(busy.unwrap_err().is_rate_limited());
    assert!(matches!(
        page(&refused("not json"), "BTC-PERPETUAL"),
        Err(ProviderError::Data(_))
    ));
}

#[test]
fn a_chart_reaches_back_ten_pages_at_most() {
    let plan = plan(DataSource::Deribit, tf("1W")).unwrap();
    assert_eq!(PAGING.requested(plan, 500), 501 * 84);
    assert_eq!(PAGING.requested(plan, 5), 6 * 84);
    // Months would take more than that: ten pages is what a request reaches.
    let months = self::plan(DataSource::Deribit, tf("1M")).unwrap();
    assert_eq!(PAGING.requested(months, 500), 50_000);
    assert_eq!(
        PAGING.requested(
            self::plan(DataSource::Deribit, Timeframe::HOUR_1).unwrap(),
            500
        ),
        500
    );
}

const CHART_PUSH: &str = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"chart.trades.BTC-PERPETUAL.1","data":{"close":56186.5,"high":56350.5,"low":56160.5,"open":56343.0,"tick":1795237380000,"cost":1180980.0,"volume":20.99192082}}}"#;

#[test]
fn parses_the_forming_candle_of_a_market_named_by_its_channel() {
    let (name, candle) = parse_candle(CHART_PUSH, Timeframe::MINUTE_1).expect("a candle");
    assert_eq!(name, "BTC-PERPETUAL");
    assert_eq!(candle.open_time, 1_795_237_380_000);
    assert_eq!(candle.close_time, 1_795_237_380_000 + MINUTE - 1);
    assert_eq!(
        (
            candle.open,
            candle.high,
            candle.low,
            candle.close,
            candle.volume
        ),
        (56343.0, 56350.5, 56160.5, 56186.5, 20.99192082)
    );
    let linear = CHART_PUSH.replace("BTC-PERPETUAL", "SOL_USDC-PERPETUAL");
    assert_eq!(
        parse_candle(&linear, Timeframe::MINUTE_1).unwrap().0,
        "SOL_USDC-PERPETUAL"
    );
}

#[test]
fn what_is_not_a_candle_is_ignored() {
    for ignored in [
        // The answer to a subscribe, which names the channels it took.
        r#"{"jsonrpc":"2.0","id":2,"result":["chart.trades.BTC-PERPETUAL.1"],"usIn":1,"usOut":2,"usDiff":1,"testnet":false}"#,
        // The answer to the heartbeat's requests, and the heartbeat itself.
        r#"{"jsonrpc":"2.0","id":1000000001,"result":"ok"}"#,
        r#"{"params":{"type":"heartbeat"},"method":"heartbeat","jsonrpc":"2.0"}"#,
        r#"{"params":{"type":"test_request"},"method":"heartbeat","jsonrpc":"2.0"}"#,
        // An error.
        r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"Invalid params"}}"#,
        // A push with no candle in it.
        r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"chart.trades.BTC-PERPETUAL.1","data":{"close":1.0}}}"#,
        r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"chart.trades.BTC-PERPETUAL.1"}}"#,
        r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"chart.trades","data":{"close":1.0,"high":1.0,"low":1.0,"open":1.0,"tick":1,"volume":1.0}}}"#,
        "not json",
        "",
    ] {
        assert!(
            parse_candle(ignored, Timeframe::MINUTE_1).is_none(),
            "{ignored}"
        );
        assert!(parse_tick(ignored).is_none(), "{ignored}");
    }
    // Each channel's push is the other's to read.
    assert!(parse_candle(TICKER_PUSH, Timeframe::MINUTE_1).is_none());
    assert!(parse_tick(CHART_PUSH).is_none());
}

#[test]
fn every_market_of_a_chart_is_a_subscription_of_one_connection() {
    let symbols = [
        "btc-perpetual".to_owned(),
        "bad name".to_owned(),
        "SOL_USDC-PERPETUAL".to_owned(),
    ];
    let channels = candle_channels(&symbols, tf("2h")).unwrap();
    assert_eq!(
        channels,
        [
            "chart.trades.BTC-PERPETUAL.120",
            "chart.trades.SOL_USDC-PERPETUAL.120"
        ]
    );
    let sessions = DERIBIT.sessions(&symbols, tf("2h")).unwrap();
    assert_eq!(sessions.len(), 1);
    let session = &sessions[0];
    assert_eq!(session.url, "wss://www.deribit.com/ws/api/v2");
    let [subscribe] = &session.subscribe[..] else {
        panic!("one request: {:?}", session.subscribe);
    };
    assert_eq!(
        subscribe,
        r#"{"id":2,"jsonrpc":"2.0","method":"public/subscribe","params":{"channels":["chart.trades.BTC-PERPETUAL.120","chart.trades.SOL_USDC-PERPETUAL.120"]}}"#
    );
}

#[test]
fn a_length_deribit_has_no_candle_for_has_no_session() {
    for other in ["3h", "12h", "1d", "1w"] {
        assert!(
            DERIBIT
                .sessions(&["BTC-PERPETUAL".into()], tf(other))
                .is_none(),
            "{other}"
        );
    }
    assert!(DERIBIT.sessions(&["bad name".into()], tf("1m")).is_none());
}

#[test]
fn many_channels_are_subscribed_in_requests_of_a_hundred() {
    let channels: Vec<String> = (0..250)
        .map(|index| format!("ticker.M{index}.agg2"))
        .collect();
    let session = socket::session(&channels);
    assert_eq!(session.subscribe.len(), 3);
    let sizes: Vec<usize> = session
        .subscribe
        .iter()
        .map(|message| {
            let value: serde_json::Value = serde_json::from_str(message).unwrap();
            value["params"]["channels"].as_array().unwrap().len()
        })
        .collect();
    assert_eq!(sizes, [100, 100, 50]);
    // The ids are the subscribe requests' own, never the heartbeat's reserved ones.
    let ids: Vec<u64> = session
        .subscribe
        .iter()
        .map(|message| {
            serde_json::from_str::<serde_json::Value>(message).unwrap()["id"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert_eq!(ids, [2, 3, 4]);
    assert!(socket::session(&[]).subscribe.is_empty());
}

#[test]
fn the_socket_asks_for_a_heartbeat_and_answers_the_servers_test_request() {
    let session = socket::session(&["chart.trades.BTC-PERPETUAL.1".to_owned()]);
    assert_eq!(
        session.keepalive,
        Keepalive::JsonRpcHeartbeat { interval_secs: 30 }
    );
    let greeting: serde_json::Value =
        serde_json::from_str(&session.keepalive.greeting().expect("a greeting")).unwrap();
    assert_eq!(greeting["method"], "public/set_heartbeat");
    assert_eq!(greeting["params"]["interval"], 30);
    // The frames Deribit sends (measured on 2026-11-21): the answer to the greeting, then every 30
    // seconds a heartbeat and a test request; none of them is data, and the data is.
    let heartbeat = r#"{"params":{"type":"heartbeat"},"method":"heartbeat","jsonrpc":"2.0"}"#;
    let request = r#"{"params":{"type":"test_request"},"method":"heartbeat","jsonrpc":"2.0"}"#;
    let greeted = r#"{"jsonrpc":"2.0","id":1000000001,"result":"ok","usIn":1795237601724283,"usOut":1795237601724336,"usDiff":53,"testnet":false}"#;
    let tested = r#"{"jsonrpc":"2.0","id":1000000002,"result":{"version":"1.2.26"},"usIn":1795237632259600,"usOut":1795237632259657,"usDiff":57,"testnet":false}"#;
    let verdict = |text: &str| format!("{:?}", session.keepalive.inspect(text));
    assert_eq!(verdict(heartbeat), "Absorbed");
    assert_eq!(verdict(greeted), "Absorbed");
    assert_eq!(verdict(tested), "Absorbed");
    assert!(verdict(request).starts_with("Answer(") && verdict(request).contains("public/test"));
    assert_eq!(verdict(CHART_PUSH), "Data");
    assert_eq!(
        verdict(r#"{"jsonrpc":"2.0","id":2,"result":["chart.trades.BTC-PERPETUAL.1"]}"#),
        "Data"
    );
}

const TICKER_PUSH: &str = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"ticker.BTC-PERPETUAL.agg2","data":{"timestamp":1795237424233,"state":"open","stats":{"high":62267.0,"low":55410.5,"price_change":-8.6794,"volume":39576.00987446,"volume_usd":2364179910.0},"index_price":56235.03,"instrument_name":"BTC-PERPETUAL","last_price":56190.0,"mark_price":56191.57}}}"#;

#[test]
fn parses_a_markets_price_with_the_price_a_day_ago_from_its_change() {
    let tick = parse_tick(TICKER_PUSH).expect("a tick");
    assert_eq!(tick.source, DataSource::Deribit);
    assert_eq!(tick.product_id, "BTC-PERPETUAL");
    let quote = tick.quote;
    assert_eq!(quote.price, 56190.0);
    let day_ago = quote.previous_day_price.expect("a price a day ago");
    assert!(
        (day_ago - 56190.0 / (1.0 - 0.086794)).abs() < 1e-6,
        "{day_ago}"
    );
    assert_eq!(quote.volume_24h, Some(2_364_179_910.0));
    assert_eq!(quote.volume_kind, Some(VolumeKind::QuoteCurrency));
    assert_eq!(quote.timestamp_ms, Some(1_795_237_424_233));
}

#[test]
fn a_ticker_push_with_no_price_is_ignored() {
    for body in [
        r#"{"timestamp":1,"instrument_name":"BTC-PERPETUAL","last_price":null,"stats":{}}"#,
        r#"{"timestamp":1,"instrument_name":"BTC-PERPETUAL","last_price":0.0}"#,
        r#"{"timestamp":1,"last_price":5.0}"#,
    ] {
        let push = format!(
            r#"{{"jsonrpc":"2.0","method":"subscription","params":{{"channel":"ticker.BTC-PERPETUAL.agg2","data":{body}}}}}"#
        );
        assert!(parse_tick(&push).is_none(), "{body}");
    }
    // A price with no change is a price without a day-ago reference.
    let bare = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"ticker.X.agg2","data":{"instrument_name":"X-PERPETUAL","last_price":3.0}}}"#;
    let quote = parse_tick(bare).unwrap().quote;
    assert_eq!(
        (quote.price, quote.previous_day_price, quote.volume_24h),
        (3.0, None, None)
    );
}

#[test]
fn every_market_a_watchlist_shows_is_a_subscription_of_one_connection() {
    let products = [
        "BTC-PERPETUAL".to_owned(),
        "x y".to_owned(),
        "spy_usdc-perpetual".to_owned(),
    ];
    assert_eq!(
        ticker_channels(&products),
        [
            "ticker.BTC-PERPETUAL.agg2",
            "ticker.SPY_USDC-PERPETUAL.agg2"
        ]
    );
    let session = QuoteProtocol::session(&DERIBIT, &products);
    assert_eq!(session.url, "wss://www.deribit.com/ws/api/v2");
    assert_eq!(session.subscribe.len(), 1);
    assert!(session.subscribe[0].contains("ticker.SPY_USDC-PERPETUAL.agg2"));
    assert_eq!(
        QuoteProtocol::parse(&DERIBIT, TICKER_PUSH),
        parse_tick(TICKER_PUSH)
    );
}

const BTC_SUMMARY: &str = r#"{"jsonrpc":"2.0","result":[{"high":62267.0,"low":55410.5,"last":56326.0,"instrument_name":"BTC-PERPETUAL","bid_price":56317.5,"ask_price":56318.0,"mark_price":56314.05,"price_change":-8.3884,"volume":39588.54935433,"base_currency":"BTC","quote_currency":"USD","volume_usd":2365545770.0},
    {"last":null,"instrument_name":"BTC-27MAR26","price_change":null,"volume_usd":0.0}]}"#;

const USDC_SUMMARY: &str = r#"{"jsonrpc":"2.0","result":[{"high":72.889,"low":65.041,"last":65.985,"instrument_name":"SOL_USDC-PERPETUAL","price_change":-8.5676,"volume":8985427.342,"volume_usd":628104737.7},
    {"last":692.77,"instrument_name":"SPY_USDC-PERPETUAL","price_change":-2.8128,"volume_usd":13246901.36},
    {"last":null,"instrument_name":"QUIET_USDC-PERPETUAL","price_change":null,"volume_usd":0.0}]}"#;

#[test]
fn a_summary_prices_every_market_that_has_a_trade() {
    let quotes = parse_summaries(USDC_SUMMARY).unwrap();
    assert_eq!(quotes.len(), 2);
    let sol = quotes["SOL_USDC-PERPETUAL"];
    assert_eq!(sol.price, 65.985);
    assert!((sol.previous_day_price.unwrap() - 65.985 / (1.0 - 0.085676)).abs() < 1e-9);
    assert_eq!(sol.volume_24h, Some(628_104_737.7));
    assert!(!quotes.contains_key("QUIET_USDC-PERPETUAL"));
}

#[test]
fn an_answer_that_is_a_refusal_or_another_shape_is_a_failure_not_an_empty_book() {
    for body in [
        "",
        r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params","data":{"reason":"invalid currency"}}}"#,
        r#"{"jsonrpc":"2.0","result":{"not":"a list"}}"#,
        r#"{"jsonrpc":"2.0","error":{"code":10028,"message":"too_many_requests"}}"#,
    ] {
        assert!(
            matches!(
                parse_summaries(body),
                Err(crate::service::watchlist_quotes::QuoteFailure::Failed(_))
            ),
            "{body}"
        );
    }
}

#[test]
fn each_settlement_currency_is_one_call_and_the_markets_asked_for_are_kept() {
    let requests: Vec<_> = [
        "BTC-PERPETUAL",
        "sol_usdc-perpetual",
        "SPY_USDC-PERPETUAL",
        "NOPE_USDC-PERPETUAL",
        "bad name",
        "BTC-27MAR26",
    ]
    .into_iter()
    .map(request)
    .collect();
    let asked = std::cell::RefCell::new(Vec::new());
    let outcome = fetch_quotes_with(&requests, |url| {
        asked.borrow_mut().push(url.to_owned());
        Ok(if url.contains("currency=BTC&") {
            BTC_SUMMARY
        } else {
            USDC_SUMMARY
        }
        .to_owned())
    });
    assert_eq!(
        *asked.borrow(),
        [
            "https://www.deribit.com/api/v2/public/get_book_summary_by_currency?currency=BTC&kind=future",
            "https://www.deribit.com/api/v2/public/get_book_summary_by_currency?currency=USDC&kind=future"
        ]
    );
    let mut priced: Vec<_> = outcome.quotes.keys().map(String::as_str).collect();
    priced.sort();
    // Keyed as asked, whatever the case; a market not in the answer has no quote and is no failure.
    assert_eq!(
        priced,
        ["BTC-PERPETUAL", "SPY_USDC-PERPETUAL", "sol_usdc-perpetual"]
    );
    assert!(outcome.failed.is_empty() && outcome.rejected.is_empty());
}

#[test]
fn a_call_that_fails_fails_only_its_markets() {
    use crate::service::watchlist_quotes::QuoteFailure;
    let requests: Vec<_> = ["BTC-PERPETUAL", "SPY_USDC-PERPETUAL"]
        .into_iter()
        .map(request)
        .collect();
    let outcome = fetch_quotes_with(&requests, |url| {
        if url.contains("currency=BTC&") {
            Err(QuoteFailure::RateLimited)
        } else {
            Ok(USDC_SUMMARY.to_owned())
        }
    });
    assert_eq!(
        outcome.quotes.keys().collect::<Vec<_>>(),
        ["SPY_USDC-PERPETUAL"]
    );
    assert_eq!(
        outcome.failed,
        [(vec!["BTC-PERPETUAL".to_owned()], QuoteFailure::RateLimited)]
    );
}

#[test]
fn a_currency_deribit_does_not_have_rejects_the_markets_named_after_it() {
    use crate::service::watchlist_quotes::QuoteFailure;
    let requests: Vec<_> = ["NOPE-PERPETUAL", "SPY_USDC-PERPETUAL"]
        .into_iter()
        .map(request)
        .collect();
    let outcome = fetch_quotes_with(&requests, |url| {
        if url.contains("currency=NOPE&") {
            Err(DERIBIT.classify_status(400).expect("a 400 is classified"))
        } else {
            Ok(USDC_SUMMARY.to_owned())
        }
    });
    assert_eq!(
        outcome.rejected.into_iter().collect::<Vec<_>>(),
        ["NOPE-PERPETUAL"]
    );
    assert_eq!(
        outcome.quotes.keys().collect::<Vec<_>>(),
        ["SPY_USDC-PERPETUAL"]
    );
    assert!(outcome.failed.is_empty());
    // Any other status is the venue's, not a market's.
    assert!(DERIBIT.classify_status(500).is_none());
    assert!(DERIBIT.classify_status(403).is_none());
    assert!(matches!(
        DERIBIT.classify_status(400),
        Some(QuoteFailure::Rejected(_))
    ));
}

#[test]
fn a_watchlist_text_names_a_market_and_reads_it_back() {
    let parsed = watchlist::text::parse(
        "DERIBIT:BTC-PERPETUAL
deribit:SOL_USDC-PERPETUAL
DERIBIT:1000PEPE_USDC-PERPETUAL",
    );
    assert!(parsed.skipped.is_empty(), "{:?}", parsed.skipped);
    let keys: Vec<String> = parsed
        .rows
        .iter()
        .filter_map(|row| match row {
            watchlist::text::Row::Instrument(item) => Some(item.instrument.key()),
            watchlist::text::Row::Section(_) => None,
        })
        .collect();
    assert_eq!(
        keys,
        [
            "DERIBIT:BTC-PERPETUAL",
            "DERIBIT:SOL_USDC-PERPETUAL",
            "DERIBIT:1000PEPE_USDC-PERPETUAL"
        ]
    );
    let skipped = watchlist::text::parse(
        "DERIBIT:BTC PERPETUAL
DERIBIT:bad!
DERIBIT:",
    );
    assert_eq!(skipped.skipped.len(), 3);
    assert!(watchlist::text::validate_market(DataSource::Deribit, "ETH-PERPETUAL").is_ok());
    // Another venue's market of the same name is another market.
    for other in DataSource::ALL
        .into_iter()
        .filter(|other| *other != DataSource::Deribit)
    {
        assert_ne!(
            InstrumentId::new(DataSource::Deribit, "BTC-PERPETUAL").key(),
            InstrumentId::new(other, "BTC-PERPETUAL").key(),
            "{other:?}"
        );
    }
    assert!(
        InstrumentId::new(DataSource::Deribit, "BTC-PERPETUAL")
            .is_same_market(&InstrumentId::new(DataSource::Deribit, "btc-perpetual"))
    );
}

/// Run with `cargo test --lib deribit::tests::live_deribit -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_deribit_serves_candles_pages_back_and_streams() {
    use crate::service::market::live::LiveFeed;
    use crate::service::watchlist_quotes::quote_stream::QuoteSocket;
    use std::time::{Duration, Instant};

    let hourly = DERIBIT
        .fetch_klines("BTC-PERPETUAL", Timeframe::HOUR_1, 500)
        .unwrap();
    println!(
        "BTC-PERPETUAL 1h: {} candles, newest opens {}",
        hourly.len(),
        hourly.last().unwrap().open_time
    );
    assert_eq!(hourly.len(), 500);
    assert!(
        hourly
            .windows(2)
            .all(|pair| pair[1].open_time - pair[0].open_time == HOUR)
    );
    assert!(
        hourly
            .iter()
            .all(|candle| candle.volume >= 0.0 && candle.close > 0.0)
    );

    // The page before, joined to the first at the hour its oldest candle opens.
    let oldest = hourly[0].open_time;
    let older = DERIBIT
        .fetch_klines_range("BTC-PERPETUAL", Timeframe::HOUR_1, 0, oldest, 500)
        .unwrap();
    println!(
        "BTC-PERPETUAL 1h one page back: {} candles, {} to {}",
        older.len(),
        older[0].open_time,
        older.last().unwrap().open_time
    );
    assert_eq!(older.len(), 500);
    assert_eq!(older.last().unwrap().open_time, oldest - HOUR);

    // More than one request holds: 12,000 hours is three pages, all joined.
    let long = DERIBIT
        .fetch_klines("BTC-PERPETUAL", Timeframe::HOUR_1, 12_000)
        .unwrap();
    println!("BTC-PERPETUAL 1h long: {} candles", long.len());
    assert_eq!(long.len(), 12_000);
    assert!(
        long.windows(2)
            .all(|pair| pair[1].open_time - pair[0].open_time == HOUR)
    );

    // Every native opens where this app cuts it.
    for native in DataSource::Deribit.natives() {
        let candles = DERIBIT.fetch_klines("BTC-PERPETUAL", *native, 30).unwrap();
        assert_eq!(candles.len(), 30, "{native}");
        for candle in &candles {
            assert_eq!(
                bucket_start_ms(*native, candle.open_time),
                candle.open_time,
                "{native}"
            );
        }
    }

    // What it cannot cut folds from two-hour bars, and where the venue's own bar and the fold
    // cover the same hours they are the same bar: its 12-hour bar of 08:00 UTC is six of the
    // two-hour bars from 08:00.
    let two_hours = DERIBIT
        .fetch_klines("BTC-PERPETUAL", Timeframe::parse("2h").unwrap(), 400)
        .unwrap();
    let own_body = rest::get(&format!(
        "/get_tradingview_chart_data?instrument_name=BTC-PERPETUAL&resolution=720&start_timestamp=0&end_timestamp={}",
        crate::clock::now_ms()
    ))
    .unwrap();
    let own = parse_candles(&own_body, tf("12h")).unwrap();
    let bar = own
        .iter()
        .rev()
        .skip(1)
        .find(|bar| bar.open_time % DAY == 8 * HOUR && bar.open_time > two_hours[0].open_time)
        .expect("a finished 08:00 bar inside the two-hour bars");
    let parts: Vec<&Kline> = two_hours
        .iter()
        .filter(|part| (bar.open_time..bar.open_time + 12 * HOUR).contains(&part.open_time))
        .collect();
    assert_eq!(parts.len(), 6);
    let volume: f64 = parts.iter().map(|part| part.volume).sum();
    println!(
        "12h bar of {}: own volume {}, six 2h bars {volume}",
        bar.open_time, bar.volume
    );
    assert_eq!(parts[0].open, bar.open);
    assert_eq!(parts[5].close, bar.close);
    assert_eq!(
        parts.iter().map(|part| part.high).fold(f64::MIN, f64::max),
        bar.high
    );
    assert_eq!(
        parts.iter().map(|part| part.low).fold(f64::MAX, f64::min),
        bar.low
    );
    assert!((volume - bar.volume).abs() <= bar.volume * 1e-9);

    // Days, weeks and months fold and open on this app's grid.
    for (timeframe, count) in [
        ("3h", 30),
        ("4h", 30),
        ("12h", 30),
        ("1D", 30),
        ("1W", 10),
        ("1M", 6),
    ] {
        let folded = DERIBIT
            .fetch_klines("BTC-PERPETUAL", tf(timeframe), count)
            .unwrap();
        println!("BTC-PERPETUAL {timeframe} folded: {} candles", folded.len());
        assert_eq!(folded.len(), count as usize, "{timeframe}");
        assert!(
            folded
                .iter()
                .all(|bar| bucket_start_ms(tf(timeframe), bar.open_time) == bar.open_time),
            "{timeframe}"
        );
    }

    // The start of a market's history: a chart that asks for more than exists is short, and the
    // page before it is empty.
    let daily = DERIBIT
        .fetch_klines("BTC-PERPETUAL", Timeframe::DAY_1, 5_000)
        .unwrap();
    println!(
        "BTC-PERPETUAL 1d everything: {} candles from {}",
        daily.len(),
        daily[0].open_time
    );
    assert!(daily.len() > 2_500 && daily.len() < 5_000);
    let before = DERIBIT
        .fetch_klines_range(
            "BTC-PERPETUAL",
            Timeframe::DAY_1,
            0,
            daily[0].open_time,
            500,
        )
        .unwrap();
    println!(
        "BTC-PERPETUAL 1d before its first candle: {} candles",
        before.len()
    );
    assert!(before.is_empty());

    // A linear perpetual, and a stock's.
    for name in ["SOL_USDC-PERPETUAL", "SPY_USDC-PERPETUAL"] {
        let candles = DERIBIT.fetch_klines(name, Timeframe::HOUR_1, 200).unwrap();
        println!(
            "{name} 1h: {} candles, last close {}",
            candles.len(),
            candles.last().unwrap().close
        );
        assert_eq!(candles.len(), 200);
        assert!(candles.iter().all(|candle| candle.close > 0.0));
    }

    let unknown = DERIBIT.fetch_klines("NOPE-PERPETUAL", Timeframe::HOUR_1, 5);
    println!("NOPE-PERPETUAL: {unknown:?}");
    assert_eq!(unknown, Err(not_listed("NOPE-PERPETUAL")));

    let hits = DERIBIT.search_tickers("sol").unwrap();
    println!(
        "search sol: {:?}",
        hits.iter()
            .map(|hit| hit.symbol.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(hits[0].full_symbol, "SOL_USDC-PERPETUAL");
    let hits = DERIBIT.search_tickers("btc").unwrap();
    assert_eq!(hits[0].full_symbol, "BTC-PERPETUAL");
    assert!(
        hits.iter()
            .any(|hit| hit.full_symbol == "BTC_USDC-PERPETUAL")
    );
    assert!(hits.iter().all(|hit| hit.kind == MarketKind::Perp));

    let requests: Vec<QuoteRequest> = ["BTC-PERPETUAL", "SPY_USDC-PERPETUAL", "NOPE-PERPETUAL"]
        .into_iter()
        .map(request)
        .collect();
    let outcome = DERIBIT.rest_quotes(&requests).unwrap();
    for symbol in ["BTC-PERPETUAL", "SPY_USDC-PERPETUAL"] {
        let quote = outcome
            .quotes
            .get(symbol)
            .unwrap_or_else(|| panic!("no REST quote for {symbol}"));
        println!("REST quote {symbol}: {quote:?}");
        assert!(quote.price > 0.0 && quote.previous_day_price.is_some());
    }
    assert!(!outcome.quotes.contains_key("NOPE-PERPETUAL") && outcome.failed.is_empty());

    // A market Deribit does not list must not silence the others on the socket.
    let feed = LiveFeed::connect(
        DataSource::Deribit,
        vec![
            "NOPE-PERPETUAL".to_owned(),
            "BTC-PERPETUAL".to_owned(),
            "ETH-PERPETUAL".to_owned(),
        ],
        Timeframe::MINUTE_1,
        eframe::egui::Context::default(),
    )
    .expect("Deribit streams one-minute candles");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _quotes = QuoteSocket::connect(
        DataSource::Deribit,
        vec!["NOPE-PERPETUAL".to_owned(), "BTC-PERPETUAL".to_owned()],
        sender,
    )
    .expect("Deribit has a ticker socket");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = std::collections::BTreeSet::new();
    while seen.len() < 2 {
        for (symbol, kline) in feed.inbox.drain() {
            assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
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
        ["BTC-PERPETUAL", "ETH-PERPETUAL"]
    );
    let tick = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a quote tick in 30 s");
    println!("socket quote {}: {:?}", tick.product_id, tick.quote);
    assert_eq!(tick.product_id, "BTC-PERPETUAL");
    assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
}

/// A socket that answers the server's heartbeat is still the same connection after 72 seconds,
/// which hold two rounds of it (one every 30). Run with
/// `cargo test --lib deribit::tests::live_deribit_heartbeat -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_deribit_heartbeat_keeps_the_socket_up_for_seventy_seconds() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let session = socket::session(&["chart.trades.BTC-PERPETUAL.1".to_owned()]);
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
    std::thread::sleep(Duration::from_secs(72));
    let links_after_72_seconds = crate::lock::lock(&links).clone();
    println!(
        "after 72 s: link events {links_after_72_seconds:?}, {} data frames",
        *crate::lock::lock(&frames)
    );
    shutdown.store(true, Ordering::Release);
    thread.join().unwrap();
    // Connected once and never dropped: the two heartbeats in 72 s were answered.
    assert_eq!(links_after_72_seconds, [true]);
}
