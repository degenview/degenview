use super::markets::{Pair, matching_pairs, parse_pairs};
use super::parse::{parse_candles, trade};
use super::rest::{PAGING, candles_path, page_before_with};
use super::socket::trades_session;
use super::ticker::parse_tickers;
use super::*;
use crate::model::market::bucket::{bucket_start_ms, next_bucket_start_ms};
use crate::model::market::live::Trade;
use crate::model::watchlist::quote::VolumeKind;
use crate::service::market::live::FrameReader;
use crate::service::net::http;
use crate::service::net::ws::{Decode, Frame, Keepalive};
use crate::service::watchlist_quotes::provider::request;
use crate::service::watchlist_quotes::quote_stream::TickReader;
use serde_json::Value;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 3_600_000;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

/// UTC milliseconds of a calendar time.
fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    crate::format::days_from_civil(year, month, day) * 86_400_000 + hour * HOUR + minute * MINUTE
}

// MARK: Names

#[test]
fn a_market_is_named_in_lower_case_whatever_way_it_was_typed() {
    for (typed, name) in [
        ("BTCUSD", "btcusd"),
        ("btcusd", "btcusd"),
        ("btc/usd", "btcusd"),
        ("BTC-USD", "btcusd"),
        ("BTC", "btcusd"),
        ("XRPRLUSD", "xrprlusd"),
        ("SUSD", "susd"),
    ] {
        assert_eq!(market_id(typed).as_deref(), Some(name), "{typed}");
    }
    for refused in [
        "",
        "  ",
        "/",
        "BTC USD&limit=1",
        "BTC?x",
        "../BTC",
        "BTC:USD",
    ] {
        assert_eq!(market_id(refused), None, "{refused}");
    }
}

#[test]
fn every_native_length_is_a_step_bitstamp_accepts_and_no_other_is() {
    let steps: Vec<_> = DataSource::Bitstamp
        .natives()
        .iter()
        .map(|native| step(*native).unwrap())
        .collect();
    assert_eq!(
        steps,
        [
            60, 180, 300, 900, 1_800, 3_600, 7_200, 14_400, 21_600, 43_200, 86_400, 259_200
        ]
    );
    // Its week (`604800`) and its odd lengths are refused: those are folded, not asked for.
    for other in ["10m", "8h", "2D", "1W", "1M", "22m"] {
        assert_eq!(step(tf(other)), None, "{other}");
    }
}

/// A bar of each step as the API answered on 2026-11-21: every one opens where this app's bucket
/// of the same length does, so none needs shifting and the `3D` of day numbers divisible by three
/// is one of them.
#[test]
fn every_native_opens_where_this_app_cuts_its_buckets() {
    for (length, open) in [
        ("1m", utc(2026, 11, 21, 6, 46)),
        ("3m", utc(2026, 11, 21, 6, 6)),
        ("5m", utc(2026, 11, 21, 5, 30)),
        ("15m", utc(2026, 11, 21, 2, 15)),
        ("30m", utc(2026, 11, 20, 21, 30)),
        ("1h", utc(2026, 11, 20, 12, 0)),
        ("2h", utc(2026, 11, 19, 16, 0)),
        ("4h", utc(2026, 11, 18, 0, 0)),
        ("6h", utc(2026, 11, 16, 12, 0)),
        ("12h", utc(2026, 11, 11, 12, 0)),
        ("1D", utc(2026, 11, 2, 0, 0)),
        ("3D", utc(2026, 9, 25, 0, 0)),
    ] {
        let timeframe = tf(length);
        assert!(DataSource::Bitstamp.natives().contains(&timeframe));
        assert_eq!(bucket_start_ms(timeframe, open), open, "{length}");
        assert_eq!(open % (step(timeframe).unwrap() * 1_000), 0, "{length}");
    }
}

// MARK: Candles

const OHLC: &str = r#"{"data": {"pair": "BTC/USD", "ohlc": [{"timestamp": "1795233600", "open": "56638.28", "high": "56743.30", "low": "55483.28", "close": "56217.24", "volume": "201.54859942"}, {"timestamp": "1795230000", "open": "58486.80", "high": "58488.35", "low": "56299.00", "close": "56655.36", "volume": "198.62932163"}]}}"#;

#[test]
fn a_candle_has_its_numbers_as_strings_and_closes_a_millisecond_before_the_next_opens() {
    let candles = parse_candles(OHLC, Timeframe::HOUR_1).unwrap();
    // Sorted, whatever order the rows came in.
    assert_eq!(candles.len(), 2);
    assert_eq!(candles[0].open_time, 1_795_230_000_000);
    assert_eq!(candles[1].open_time, 1_795_233_600_000);
    assert_eq!(candles[0].close_time, 1_795_233_599_999);
    assert_eq!(
        (
            candles[0].open,
            candles[0].high,
            candles[0].low,
            candles[0].close,
            candles[0].volume
        ),
        (58_486.80, 58_488.35, 56_299.00, 56_655.36, 198.62932163)
    );
    assert_eq!(
        candles[1].close_time,
        next_bucket_start_ms(Timeframe::HOUR_1, candles[1].open_time) - 1
    );
}

#[test]
fn an_empty_range_is_no_candles_and_a_shape_that_is_not_one_is_an_error() {
    assert_eq!(
        parse_candles(
            r#"{"data":{"pair":"BTC/USD","ohlc":[]}}"#,
            Timeframe::HOUR_1
        ),
        Ok(Vec::new())
    );
    for body in [
        "not json",
        "[]",
        r#"{"data":{}}"#,
        r#"{"data":{"ohlc":[{"timestamp":"1795233600","open":"1"}]}}"#,
        r#"{"data":{"ohlc":[{"timestamp":"soon","open":"1","high":"1","low":"1","close":"1","volume":"0"}]}}"#,
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

#[test]
fn a_page_ends_at_the_whole_second_before_where_the_last_candle_began() {
    // The bar before the one that opens at 1795233600 s opens at or before 1795233599.
    assert_eq!(
        candles_path("btcusd", 3_600, 1_795_233_600_000, 500),
        "/ohlc/btcusd/?step=3600&limit=500&end=1795233599"
    );
    // "Now" is not on a second: the forming bar is included.
    assert_eq!(
        candles_path("btcusd", 60, 1_795_244_712_501, 5),
        "/ohlc/btcusd/?step=60&limit=5&end=1795244712"
    );
}

#[test]
fn a_request_never_asks_for_more_than_a_page_holds_or_an_end_bitstamp_refuses() {
    assert!(candles_path("btcusd", 60, 1_000_000, 5_000).contains("limit=1000&"));
    assert!(candles_path("btcusd", 60, 1_000_000, 0).contains("limit=1&"));
    assert!(candles_path("btcusd", 60, i64::MAX / 2, 5).ends_with("end=253402297199"));
    assert!(candles_path("btcusd", 60, -5, 5).ends_with("end=0"));
    // 30,000 base candles in one request, a page at a time.
    assert_eq!(PAGING.page, 1_000);
    assert_eq!(PAGING.max_candles, 30_000);
}

#[test]
fn a_page_is_asked_in_steps_of_the_length_and_read_as_candles() {
    let mut asked = String::new();
    let page = page_before_with(
        |path| {
            asked = path.to_owned();
            Ok(OHLC.to_owned())
        },
        "btcusd",
        Timeframe::HOUR_1,
        1_795_237_200_000,
        2,
    )
    .unwrap();
    assert_eq!(asked, "/ohlc/btcusd/?step=3600&limit=2&end=1795237199");
    assert_eq!(page.len(), 2);
}

#[test]
fn a_market_bitstamp_does_not_list_is_told_so_and_other_failures_are_what_they_are() {
    let ask = |status: u16| {
        move |_: &str| -> Result<String, ProviderError> {
            Err(ProviderError::Network(http::Error::Status(status)))
        }
    };
    assert_eq!(
        page_before_with(ask(404), "nopeusd", Timeframe::HOUR_1, 1_000_000, 10),
        Err(ProviderError::unknown_market(
            DataSource::Bitstamp,
            "NOPEUSD",
            ""
        ))
    );
    for status in [400, 429, 500] {
        assert_eq!(
            page_before_with(ask(status), "btcusd", Timeframe::HOUR_1, 1_000_000, 10),
            Err(ProviderError::Network(http::Error::Status(status)))
        );
    }
    // A length it has no step for is not asked.
    let unasked = |_: &str| -> Result<String, ProviderError> { panic!("asked") };
    assert!(
        page_before_with(unasked, "btcusd", Timeframe::WEEK_1, 1_000_000, 10)
            .unwrap_err()
            .is_unsupported_interval()
    );
}

#[test]
fn weeks_months_and_years_are_made_from_days() {
    for folded in ["1W", "1M", "3M", "1Y"] {
        let plan = DataSource::Bitstamp.plan(tf(folded)).unwrap();
        assert_eq!(plan.base(), Timeframe::DAY_1, "{folded}");
    }
    // Hours of the day it has no step for come from the coarsest finer one that nests in them.
    assert_eq!(
        DataSource::Bitstamp.plan(tf("8h")).unwrap().base(),
        Timeframe::HOUR_4
    );
}

// MARK: Markets

const MARKETS: &str = r#"[
 {"name":"BTC/USD","market_symbol":"btcusd","base_currency":"BTC","base_decimals":8,"counter_currency":"USD","counter_decimals":2,"trading":"Enabled","description":"Bitcoin / U.S. dollar","market_type":"SPOT","matching":"Enabled"},
 {"name":"XRP/RLUSD","market_symbol":"xrprlusd","base_currency":"XRP","counter_currency":"RLUSD","trading":"Enabled","market_type":"SPOT","matching":"Enabled"},
 {"name":"S/USD","market_symbol":"susd","base_currency":"S","counter_currency":"USD","trading":"Enabled","market_type":"SPOT","matching":"Enabled"},
 {"name":"ZK/EUR","market_symbol":"zkeur","base_currency":"ZK","counter_currency":"EUR","trading":"Enabled","market_type":"SPOT","matching":"Disabled"},
 {"name":"OLD/USD","market_symbol":"oldusd","base_currency":"OLD","counter_currency":"USD","trading":"Disabled","market_type":"SPOT","matching":"Enabled"},
 {"name":"BTC/USD-PERP","market_symbol":"btcusd-perp","base_currency":"BTC","counter_currency":"USD","trading":"Enabled","market_type":"PERPETUAL","matching":"Enabled"},
 {"name":"","market_symbol":"","base_currency":"X","counter_currency":"USD","trading":"Enabled","market_type":"SPOT","matching":"Enabled"}
]"#;

fn pairs() -> Vec<Pair> {
    parse_pairs(MARKETS).unwrap()
}

#[test]
fn the_market_list_holds_the_spot_markets_that_trade() {
    let ids: Vec<_> = pairs().into_iter().map(|pair| pair.id).collect();
    assert_eq!(ids, ["BTCUSD", "XRPRLUSD", "SUSD"]);
    assert!(matches!(parse_pairs("{}"), Err(ProviderError::Data(_))));
    assert!(matches!(parse_pairs("nope"), Err(ProviderError::Data(_))));
}

#[test]
fn the_assets_the_list_names_are_the_ones_the_profile_reads_from_the_id() {
    for pair in pairs() {
        assert_eq!(DataSource::Bitstamp.base_asset(&pair.id), pair.base);
        assert_eq!(
            DataSource::Bitstamp.quote_asset(&pair.id).as_deref(),
            Some(pair.quote.as_str())
        );
    }
}

#[test]
fn a_hit_names_its_two_assets_and_a_query_runs_the_assets_together() {
    let hits = matching_pairs(&pairs(), &normalize("btc/usd"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].symbol, "BTCUSD");
    assert_eq!(hits[0].source, DataSource::Bitstamp);
    assert_eq!(
        (hits[0].base.as_deref(), hits[0].quote.as_deref()),
        (Some("BTC"), Some("USD"))
    );
    let all = matching_pairs(&pairs(), "USD");
    assert_eq!(all.len(), 3, "BTCUSD, XRPRLUSD and SUSD contain it");
    assert!(matching_pairs(&pairs(), "DOGE").is_empty());
}

// MARK: Prices

const TICKERS: &str = r#"[
 {"timestamp":"1795244712","open":"58808.00","high":"62272.30","low":"55081.31","last":"57823.73","volume":"2485.14446965","vwap":"58958.24","bid":"57823.80","ask":"57828.39","side":"0","open_24":"61862.87","percent_change_24":"-6.53","market_type":"SPOT","pair":"BTC/USD","market":"BTC/USD"},
 {"timestamp":"1795244712","open":"0.000000","high":"0.000000","low":"0.000000","last":"0.030040","volume":"0.0","vwap":"0.000000","bid":"0.026510","ask":"0.026645","side":"1","open_24":"0.000000","percent_change_24":"","market_type":"SPOT","pair":"STRK/EUR","market":"STRK/EUR"},
 {"timestamp":"1795244712","open":"1","high":"1","low":"1","last":"0.00000000","volume":"0.0","open_24":"0","percent_change_24":"","market_type":"SPOT","pair":"NEW/USD"},
 {"timestamp":"1795244712","open":"58833","high":"62281","low":"55095","last":"57826","volume":"1987.02208","vwap":"58609","open_24":"61868","percent_change_24":"-6.53","market_type":"PERPETUAL","pair":"BTC/USD-PERP","market":"BTC/USD-PERP","mark_price":"57846.24291132"}
]"#;

#[test]
fn a_ticker_row_prices_a_market_against_the_price_24_hours_ago_not_the_utc_days_open() {
    let quotes = parse_tickers(TICKERS, &[request("BTCUSD")]).unwrap();
    let btc = quotes["BTCUSD"];
    assert_eq!(btc.price, 57_823.73);
    // `open_24`, not `open` (the open of the UTC day, 58,808).
    assert_eq!(btc.previous_day_price, Some(61_862.87));
    assert_eq!(
        (btc.volume_24h, btc.volume_kind),
        (Some(2_485.14446965), Some(VolumeKind::Base))
    );
    assert_eq!(btc.timestamp_ms, Some(1_795_244_712_000));
}

#[test]
fn a_market_that_did_not_trade_in_24_hours_has_a_price_and_no_reference() {
    let quotes = parse_tickers(TICKERS, &[request("STRKEUR")]).unwrap();
    assert_eq!(quotes["STRKEUR"].price, 0.03004);
    assert_eq!(quotes["STRKEUR"].previous_day_price, None);
}

#[test]
fn only_the_markets_asked_for_are_priced_under_the_names_they_were_asked_as() {
    let asked = [
        request("btcusd"),
        request("BTC/USD"),
        // No price yet, a perpetual (which shares the answer) and a market it does not list.
        request("NEWUSD"),
        request("BTCUSD-PERP"),
        request("NOPEUSD"),
    ];
    let quotes = parse_tickers(TICKERS, &asked).unwrap();
    let mut names: Vec<_> = quotes.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["BTC/USD", "btcusd"]);
}

#[test]
fn a_ticker_answer_that_is_not_a_list_is_a_failure() {
    for body in ["not json", "{}", r#"{"code":"validation-error"}"#] {
        assert!(
            matches!(
                parse_tickers(body, &[request("BTCUSD")]),
                Err(QuoteFailure::Failed(_))
            ),
            "{body}"
        );
    }
}

// MARK: Socket

const TRADE: &str = r#"{"data":{"id":678461211,"timestamp":"1795244813","amount":0.00172369,"amount_str":"0.00172369","price":58029.52,"price_str":"58029.52","type":0,"microtimestamp":"1795244813629000","buy_order_id":2074254700429313,"sell_order_id":2074254699409411},"channel":"live_trades_btcusd","event":"trade"}"#;

/// What Bitstamp's socket says that is not a trade: the acknowledgement of a subscription, the
/// refusal of a channel it has not, the answer to a heartbeat, a request to reconnect, a trade of a
/// market that cannot be read and a trade that is not a trade.
const NOT_TRADES: [&str; 12] = [
    r#"{"event":"bts:subscription_succeeded","channel":"live_trades_btcusd","data":{}}"#,
    r#"{"event":"bts:error","channel":"","data":{"code":null,"message":"Invalid channel provided."}}"#,
    r#"{"event":"bts:error","channel":"","data":{"code":null,"message":"Incorrect JSON format."}}"#,
    r#"{"event":"bts:heartbeat","channel":"","data":{"status":"success"}}"#,
    r#"{"event":"bts:request_reconnect","channel":"","data":""}"#,
    r#"{"event":"bts:unsubscription_succeeded","channel":"live_trades_btcusd","data":{}}"#,
    r#"{"data":{"price":1.0,"amount":1.0,"microtimestamp":"1795244813629000"},"channel":"order_book_btcusd","event":"trade"}"#,
    r#"{"data":{"price":0,"amount":1.0,"microtimestamp":"1795244813629000"},"channel":"live_trades_btcusd","event":"trade"}"#,
    r#"{"data":{"price":1.0,"amount":-1.0,"microtimestamp":"1795244813629000"},"channel":"live_trades_btcusd","event":"trade"}"#,
    r#"{"data":{"price":1.0,"amount":1.0},"channel":"live_trades_btcusd","event":"trade"}"#,
    r#"{"data":{"price":1.0,"amount":1.0,"microtimestamp":"1795244813629000"},"channel":"live_trades_btcusd","event":"other"}"#,
    "not json",
];

#[test]
fn a_trade_says_its_market_price_size_and_time_in_milliseconds() {
    let (market, trade) = trade(TRADE).expect("a trade");
    assert_eq!(market, "BTCUSD");
    assert_eq!(
        trade,
        Trade {
            time_ms: 1_795_244_813_629,
            price: 58_029.52,
            size: 0.00172369,
        }
    );
}

#[test]
fn nothing_else_the_socket_says_is_a_trade() {
    for ignored in NOT_TRADES {
        assert!(trade(ignored).is_none(), "{ignored}");
    }
}

#[test]
fn every_market_is_its_own_subscription_on_one_connection() {
    let session = trades_session(&[
        "BTCUSD".into(),
        "eth/usd".into(),
        "btc-usd".into(),
        "BTC&x".into(),
    ]);
    assert_eq!(session.url, "wss://ws.bitstamp.net");
    assert_eq!(
        (session.host.as_str(), session.port),
        ("ws.bitstamp.net", 443)
    );
    assert_eq!(session.decode, Decode::None);
    let channels: Vec<Value> = session
        .subscribe
        .iter()
        .map(|message| serde_json::from_str(message).unwrap())
        .collect();
    // The same market spelled twice is subscribed once, and what is no market not at all.
    assert_eq!(
        channels
            .iter()
            .map(|message| (
                message["event"].as_str().unwrap(),
                message["data"]["channel"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            ("bts:subscribe", "live_trades_btcusd"),
            ("bts:subscribe", "live_trades_ethusd")
        ]
    );
}

#[test]
fn the_connection_is_probed_with_a_heartbeat_whose_answer_is_no_data() {
    let keepalive = trades_session(&[]).keepalive;
    assert!(matches!(
        keepalive,
        Keepalive::QuietText { ping, pong }
            if ping == r#"{"event":"bts:heartbeat"}"#
                && NOT_TRADES[3].starts_with(pong)
                && !TRADE.starts_with(pong)
                && !NOT_TRADES[0].starts_with(pong)
    ));
    assert!(matches!(
        keepalive.probe(),
        Some(tungstenite::Message::Text(text)) if text.as_str() == r#"{"event":"bts:heartbeat"}"#
    ));
    // A thin market is quiet for hours: the answer is all the socket has to show.
    assert!(keepalive.heard_is_alive());
}

// MARK: Candles from trades

fn trade_message(market: &str, time_ms: i64, price: f64, amount: f64) -> String {
    format!(
        r#"{{"data":{{"price":{price},"amount":{amount},"microtimestamp":"{}"}},"channel":"live_trades_{market}","event":"trade"}}"#,
        time_ms * 1_000
    )
}

fn candle_reader(base: Timeframe) -> Box<dyn FrameReader> {
    CandleProtocol::reader(&Bitstamp, base).expect("Bitstamp builds its candles in a reader")
}

fn read(reader: &mut dyn FrameReader, text: &str) -> Option<(String, Kline)> {
    reader.read(Frame::Text(text))
}

/// 2026-11-21T07:05:00Z, on a minute.
const T0: i64 = 1_795_244_700_000;

#[test]
fn trades_make_the_forming_candle_of_the_length_the_feed_asks() {
    let mut reader = candle_reader(Timeframe::MINUTE_1);
    let (market, first) = read(
        &mut *reader,
        &trade_message("btcusd", T0 + 5_000, 100.0, 1.0),
    )
    .expect("a candle");
    assert_eq!(market, "BTCUSD");
    assert_eq!((first.open_time, first.close_time), (T0, T0 + MINUTE - 1));
    read(
        &mut *reader,
        &trade_message("btcusd", T0 + 20_000, 105.0, 2.0),
    );
    read(
        &mut *reader,
        &trade_message("btcusd", T0 + 30_000, 98.0, 0.5),
    );
    let (_, last) = read(
        &mut *reader,
        &trade_message("btcusd", T0 + 59_000, 101.0, 0.5),
    )
    .unwrap();
    assert_eq!(
        (last.open, last.high, last.low, last.close, last.volume),
        (100.0, 105.0, 98.0, 101.0, 4.0)
    );
    // The next minute opens a candle of its own.
    let (_, next) = read(
        &mut *reader,
        &trade_message("btcusd", T0 + MINUTE + 1_000, 90.0, 1.0),
    )
    .unwrap();
    assert_eq!(
        (next.open_time, next.open, next.volume),
        (T0 + MINUTE, 90.0, 1.0)
    );
}

#[test]
fn each_market_of_a_connection_has_its_own_candle() {
    let mut reader = candle_reader(Timeframe::HOUR_1);
    read(&mut *reader, &trade_message("btcusd", T0, 100.0, 1.0));
    read(
        &mut *reader,
        &trade_message("ethusd", T0 + 1_000, 10.0, 5.0),
    );
    let (market, btc) = read(
        &mut *reader,
        &trade_message("btcusd", T0 + 2_000, 102.0, 1.0),
    )
    .unwrap();
    assert_eq!(market, "BTCUSD");
    assert_eq!((btc.open, btc.close, btc.volume), (100.0, 102.0, 2.0));
    let (market, eth) = read(
        &mut *reader,
        &trade_message("ethusd", T0 + 3_000, 11.0, 5.0),
    )
    .unwrap();
    assert_eq!(market, "ETHUSD");
    assert_eq!((eth.open, eth.close, eth.volume), (10.0, 11.0, 10.0));
    assert_eq!(eth.open_time, bucket_start_ms(Timeframe::HOUR_1, T0));
}

#[test]
fn a_new_connection_goes_on_with_the_candle_in_progress() {
    let mut reader = candle_reader(Timeframe::MINUTE_1);
    read(&mut *reader, &trade_message("btcusd", T0, 100.0, 1.0));
    let (_, before) = read(
        &mut *reader,
        &trade_message("btcusd", T0 + 1_000, 110.0, 1.0),
    )
    .unwrap();
    reader.reset();
    // The trades missed while the link was down are not in the candle, but a candle that started
    // over would hold less volume than the last, which the chart takes for an older account of the
    // bar and drops: it goes on, and the next account of the bar from REST restores the rest.
    let (_, candle) = read(
        &mut *reader,
        &trade_message("btcusd", T0 + 30_000, 90.0, 2.0),
    )
    .unwrap();
    assert_eq!(
        (candle.open, candle.high, candle.low, candle.volume),
        (100.0, 110.0, 90.0, 4.0)
    );
    assert!(candle.volume >= before.volume);
}

#[test]
fn the_feed_says_its_klines_are_trades_to_add_to_the_held_bar() {
    use crate::model::market::live::Klines;
    assert_eq!(CandleProtocol::klines(&Bitstamp), Klines::Trades);
}

#[test]
fn no_message_but_a_trade_is_a_candle() {
    let mut reader = candle_reader(Timeframe::MINUTE_1);
    for ignored in NOT_TRADES {
        assert!(read(&mut *reader, ignored).is_none(), "{ignored}");
    }
    assert!(reader.read(Frame::Binary(&[1, 2, 3])).is_none());
}

#[test]
fn a_feed_opens_one_connection_for_every_market_and_only_for_a_length_it_serves() {
    let sessions = Bitstamp
        .sessions(&["BTCUSD".into(), "ETHUSD".into()], Timeframe::MINUTE_1)
        .expect("a session");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].subscribe.len(), 2);
    // Every length the feed can be asked for is a native one.
    for native in DataSource::Bitstamp.natives() {
        assert!(Bitstamp.sessions(&["BTCUSD".into()], *native).is_some());
    }
    for other in ["8h", "1W", "1M"] {
        assert!(
            Bitstamp.sessions(&["BTCUSD".into()], tf(other)).is_none(),
            "{other}"
        );
    }
    assert!(Bitstamp.sessions(&[], Timeframe::MINUTE_1).is_none());
    assert!(
        Bitstamp
            .sessions(&["BTC&x".into()], Timeframe::MINUTE_1)
            .is_none()
    );
}

// MARK: Prices from trades

fn tick_reader() -> Box<dyn TickReader> {
    QuoteProtocol::reader(&Bitstamp).expect("Bitstamp reads its trades through a reader")
}

#[test]
fn a_trade_is_the_last_price_and_says_nothing_of_the_day() {
    let tick = tick_reader().read(TRADE).expect("a tick");
    assert_eq!(tick.source, DataSource::Bitstamp);
    assert_eq!(tick.product_id, "BTCUSD");
    assert_eq!(tick.quote.price, 58_029.52);
    assert_eq!(tick.quote.timestamp_ms, Some(1_795_244_813_629));
    assert_eq!(tick.quote.previous_day_price, None);
    assert_eq!(tick.quote.volume_24h, None);
    // So the engine is told to take the day from REST.
    assert!(!QuoteProtocol::ticks_carry_reference(&Bitstamp));
    let mut reader = tick_reader();
    for ignored in NOT_TRADES {
        assert!(reader.read(ignored).is_none(), "{ignored}");
    }
}

const REQUEST_RECONNECT: &str = r#"{"event":"bts:request_reconnect","channel":"","data":""}"#;

#[test]
fn a_request_to_reconnect_is_told_once_by_the_candle_feed_and_the_ticker() {
    let mut candles = candle_reader(Timeframe::MINUTE_1);
    let mut prices = tick_reader();
    assert!(!candles.reconnect() && !prices.reconnect());
    for text in [TRADE, NOT_TRADES[3], NOT_TRADES[0]] {
        read(&mut *candles, text);
        prices.read(text);
        assert!(!candles.reconnect() && !prices.reconnect(), "{text}");
    }
    assert!(read(&mut *candles, REQUEST_RECONNECT).is_none());
    assert!(prices.read(REQUEST_RECONNECT).is_none());
    assert!(candles.reconnect() && prices.reconnect());
    assert!(
        !candles.reconnect() && !prices.reconnect(),
        "asked for once"
    );

    // Another message saying the words is not the request.
    let mention =
        r#"{"event":"bts:error","channel":"","data":{"message":"bts:request_reconnect"}}"#;
    read(&mut *candles, mention);
    prices.read(mention);
    assert!(!candles.reconnect() && !prices.reconnect());

    // A request that came before the new connection is not the new connection's.
    read(&mut *candles, REQUEST_RECONNECT);
    prices.read(REQUEST_RECONNECT);
    candles.reset();
    prices.reset();
    assert!(!candles.reconnect() && !prices.reconnect());
}

#[test]
fn the_ticker_socket_subscribes_every_market_once() {
    let session = QuoteProtocol::session(&Bitstamp, &["BTCUSD".into(), "ETHBTC".into()]);
    assert_eq!(session.url, "wss://ws.bitstamp.net");
    assert_eq!(session.subscribe.len(), 2);
    assert!(session.subscribe[1].contains("live_trades_ethbtc"));
}

// MARK: Live

/// Run with `cargo test --release --lib bitstamp::tests::live_bitstamp_serves -- --ignored
/// --nocapture`. It waits for a one-minute candle to close (up to about three minutes).
#[test]
#[ignore = "needs the network"]
fn live_bitstamp_serves_candles_pages_back_and_streams() {
    use crate::service::market::live::LiveFeed;
    use crate::service::watchlist_quotes::quote_stream::QuoteSocket;
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    let consecutive = |candles: &[Kline], step: i64| {
        candles
            .windows(2)
            .all(|pair| pair[1].open_time - pair[0].open_time == step)
    };

    let hourly = BITSTAMP
        .fetch_klines("BTCUSD", Timeframe::HOUR_1, 2_500)
        .unwrap();
    println!(
        "BTCUSD 1h: {} candles, newest opens {}",
        hourly.len(),
        hourly.last().unwrap().open_time
    );
    assert_eq!(hourly.len(), 2_500, "three pages of up to 1,000");
    assert!(consecutive(&hourly, HOUR));
    assert!(
        hourly
            .iter()
            .all(|c| c.high >= c.low && c.volume >= 0.0 && c.close > 0.0)
    );

    // The page before, joined at the hour the oldest candle opens.
    let oldest = hourly[0].open_time;
    let older = BITSTAMP
        .fetch_klines_range("BTCUSD", Timeframe::HOUR_1, 0, oldest, 500)
        .unwrap();
    assert_eq!(older.len(), 500);
    assert_eq!(older.last().unwrap().open_time, oldest - HOUR);

    // A short page is the start of the history: the page before it is empty.
    let daily = BITSTAMP
        .fetch_klines("BTCUSD", Timeframe::DAY_1, 6_000)
        .unwrap();
    println!(
        "BTCUSD 1D everything: {} candles from {}",
        daily.len(),
        daily[0].open_time
    );
    assert!(daily.len() > 4_000 && consecutive(&daily, 86_400_000));
    assert!(
        BITSTAMP
            .fetch_klines_range("BTCUSD", Timeframe::DAY_1, 0, daily[0].open_time, 500)
            .unwrap()
            .is_empty()
    );

    // Weeks fold from days and open on Mondays.
    let weekly = BITSTAMP
        .fetch_klines("BTCUSD", Timeframe::WEEK_1, 100)
        .unwrap();
    assert!(weekly.len() >= 99);
    assert!(
        weekly
            .iter()
            .all(|c| bucket_start_ms(Timeframe::WEEK_1, c.open_time) == c.open_time)
    );

    // A market Bitstamp does not list is told so.
    let error = BITSTAMP
        .fetch_klines("NOPEUSD", Timeframe::HOUR_1, 10)
        .unwrap_err();
    println!("NOPEUSD: {error}");
    assert_eq!(
        error,
        ProviderError::unknown_market(DataSource::Bitstamp, "NOPEUSD", "")
    );

    let hits = BITSTAMP.search_tickers("btc usd").unwrap();
    println!(
        "search btc usd: {:?}",
        hits.iter().map(|h| &h.symbol).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].symbol, "BTCUSD");

    let quotes = ticker::fetch_quotes(&[request("BTCUSD"), request("ETHEUR")]).unwrap();
    println!("REST quotes: {quotes:?}");
    assert!(quotes["BTCUSD"].price > 0.0 && quotes["BTCUSD"].previous_day_price.is_some());

    // The sockets: a price from the first trade, and a one-minute candle built from the trades that
    // closes and is then compared with the bar Bitstamp itself made of the minute.
    let feed = LiveFeed::connect(
        DataSource::Bitstamp,
        vec!["BTCUSD".to_owned()],
        Timeframe::MINUTE_1,
        eframe::egui::Context::default(),
    )
    .expect("Bitstamp streams one-minute candles");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _socket = QuoteSocket::connect(DataSource::Bitstamp, vec!["BTCUSD".to_owned()], sender)
        .expect("Bitstamp has a ticker socket");
    let tick = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a tick in 30 s");
    println!("live tick: {tick:?}");
    assert_eq!(tick.product_id, "BTCUSD");
    assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_none());

    let started = Instant::now();
    let mut built: BTreeMap<i64, Kline> = BTreeMap::new();
    let first_candle = loop {
        for (symbol, kline) in feed.inbox.drain() {
            assert_eq!(symbol, "BTCUSD");
            assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
            built.insert(kline.open_time, kline);
        }
        if !built.is_empty() {
            break started.elapsed();
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "no candle in 60 s"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    println!("first trade-built candle after {first_candle:?}");
    // Three bars seen means the second one is closed and was built from every trade of its minute.
    while built.len() < 3 {
        for (_, kline) in feed.inbox.drain() {
            built.insert(kline.open_time, kline);
        }
        assert!(
            started.elapsed() < Duration::from_secs(200),
            "no minute closed in 200 s"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let closed = built.values().nth(1).unwrap().clone();
    // The candle holds every trade of its minute (the feed began before it), so it is the whole bar.
    // Bitstamp's REST account of a minute lags the trades by a few seconds, and its own bar of the
    // minute that has just closed can be missing or short of its last trades: ask again until it
    // agrees, for as long as the lag lasts.
    let ask = || {
        BITSTAMP
            .fetch_klines_range(
                "BTCUSD",
                Timeframe::MINUTE_1,
                closed.open_time,
                closed.open_time + MINUTE,
                5,
            )
            .unwrap()
            .first()
            .cloned()
    };
    let same = |theirs: &Kline| {
        (theirs.open, theirs.high, theirs.low, theirs.close)
            == (closed.open, closed.high, closed.low, closed.close)
            && (theirs.volume - closed.volume).abs() < 1e-6 * theirs.volume.max(1.0)
    };
    let mut theirs = ask();
    for _ in 0..15 {
        if theirs.as_ref().is_some_and(same) {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
        theirs = ask();
    }
    let theirs = theirs.expect("Bitstamp's bar of that minute");
    println!("trade-built {closed:?}\nbitstamp    {theirs:?}");
    assert_eq!(theirs.open_time, closed.open_time);
    assert_eq!(
        (theirs.open, theirs.high, theirs.low, theirs.close),
        (closed.open, closed.high, closed.low, closed.close)
    );
    assert!((theirs.volume - closed.volume).abs() < 1e-6 * theirs.volume.max(1.0));
}

/// Every candle length the chart offers opens where Binance's does, the folded ones included. Run
/// with `cargo test --release --lib bitstamp::tests::live_bitstamp_buckets -- --ignored
/// --nocapture`; the all-venues comparison is `live_sources_cut_the_same_buckets`.
#[test]
#[ignore = "needs the network"]
fn live_bitstamp_buckets_match_binances_at_every_timeframe() {
    let odd = [
        "3m", "5m", "22m", "2h", "3h", "4h", "6h", "8h", "12h", "2D", "3D", "2W", "6M",
    ]
    .map(tf);
    for interval in Timeframe::LEGACY.into_iter().chain(odd) {
        let opens = |source, symbol| -> std::collections::BTreeSet<i64> {
            crate::service::fetch_klines(source, symbol, interval, 500)
                .unwrap_or_else(|error| panic!("{source:?} {interval}: {error}"))
                .iter()
                .map(|bar| bar.open_time)
                .collect()
        };
        let binance = opens(DataSource::Binance, "ETHUSDT");
        let bitstamp = opens(DataSource::Bitstamp, "ETHUSD");
        let from = binance.first().max(bitstamp.first()).copied().unwrap();
        let within = |set: &std::collections::BTreeSet<i64>| -> Vec<i64> {
            set.iter().copied().filter(|time| *time >= from).collect()
        };
        println!(
            "{interval:>3}: binance {:>4} bitstamp {:>4} | shared range {:>4} bars",
            binance.len(),
            bitstamp.len(),
            within(&binance).len()
        );
        assert!(within(&bitstamp).len() > 2, "{interval}");
        assert_eq!(
            within(&binance),
            within(&bitstamp),
            "{interval} binance vs bitstamp"
        );
    }
}
