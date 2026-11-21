use super::channels::{Channels, Message, Subscription, message};
use super::markets::{matching_pairs, parse_pairs};
use super::parse::too_fast;
use super::parse::{latest, parse_candles};
use super::rest::page_before_with;
use super::socket::{CHANNELS_PER_CONNECTION, subscribe_candles, subscribe_ticker};
use super::ticker::parse_tickers;
use super::*;
use crate::model::market::bucket::bucket_start_ms;
use crate::model::watchlist::quote::VolumeKind;
use crate::service::market::live::FrameReader;
use crate::service::net::ws::{Frame, Keepalive};
use crate::service::watchlist_quotes::provider::request;
use crate::service::watchlist_quotes::quote_stream::TickReader;
use serde_json::Value;
use std::cell::RefCell;

const HOUR: i64 = 3_600_000;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

// MARK: Names

#[test]
fn a_market_is_named_as_bitfinex_does_whatever_way_it_was_typed() {
    for (typed, name) in [
        ("BTCUSD", "BTCUSD"),
        ("btc/usd", "BTCUSD"),
        ("BTC-USD", "BTCUSD"),
        ("BTC", "BTCUSD"),
        ("ETHBTC", "ETHBTC"),
        ("AVAX:USD", "AVAX:USD"),
        ("avaxusd", "AVAX:USD"),
        ("AVAX/USD", "AVAX:USD"),
        ("avax", "AVAX:USD"),
        ("AVAX:UST", "AVAX:UST"),
        ("btc:xaut", "BTC:XAUT"),
        ("ETH2X:USD", "ETH2X:USD"),
    ] {
        assert_eq!(market(typed).as_deref(), Some(name), "{typed}");
    }
    assert_eq!(symbol("avax/usd").as_deref(), Some("tAVAX:USD"));
    for refused in ["", "  ", "/", "BTC USD&limit=1", "BTC?x", "../BTC"] {
        assert_eq!(market(refused), None, "{refused}");
    }
}

#[test]
fn the_two_assets_of_a_pair_are_told_apart_whether_or_not_a_colon_separates_them() {
    let bitfinex = DataSource::Bitfinex;
    for (pair, base, quote) in [
        ("BTCUSD", "BTC", "USD"),
        ("ADAUST", "ADA", "UST"),
        ("ETHBTC", "ETH", "BTC"),
        ("AVAX:USD", "AVAX", "USD"),
        ("AVAX:BTC", "AVAX", "BTC"),
        ("BTC:XAUT", "BTC", "XAUT"),
        ("ALT2612:UST", "ALT2612", "UST"),
    ] {
        assert_eq!(bitfinex.base_asset(pair), base, "{pair}");
        assert_eq!(bitfinex.quote_asset(pair).as_deref(), Some(quote), "{pair}");
    }
}

// MARK: Candle lengths

#[test]
fn every_native_has_its_period_and_nothing_else_does() {
    let periods: Vec<String> = DataSource::Bitfinex
        .natives()
        .iter()
        .map(|native| period(*native).expect("a native has a period"))
        .collect();
    assert_eq!(
        periods,
        [
            "1m", "5m", "15m", "30m", "1h", "3h", "6h", "12h", "1D", "1W", "14D", "1M"
        ]
    );
    for other in ["2m", "3m", "2h", "4h", "8h", "7D", "2W", "3M", "1Y", "22m"] {
        assert_eq!(period(tf(other)), None, "{other}");
    }
}

#[test]
fn every_native_opens_where_this_app_cuts() {
    // The first bars of BTCUSD the API answered from 2020-01-01 00:00 UTC (a Wednesday).
    let opens = |native: &str, times: &[i64]| {
        for time in times {
            assert_eq!(bucket_start_ms(tf(native), *time), *time, "{native}");
        }
    };
    opens(
        "3h",
        &[1_577_836_800_000, 1_577_847_600_000, 1_577_858_400_000],
    );
    opens(
        "6h",
        &[1_577_836_800_000, 1_577_858_400_000, 1_577_880_000_000],
    );
    opens(
        "12h",
        &[1_577_836_800_000, 1_577_880_000_000, 1_577_923_200_000],
    );
    opens("1D", &[1_577_836_800_000, 1_577_923_200_000]);
    // Mondays: 2020-01-06, -13, -20.
    opens(
        "1W",
        &[1_578_268_800_000, 1_578_873_600_000, 1_579_478_400_000],
    );
    // The first of January, February, March, April.
    opens(
        "1M",
        &[
            1_577_836_800_000,
            1_580_515_200_000,
            1_583_020_800_000,
            1_585_699_200_000,
        ],
    );
    // Thursdays 2020-01-09, -23, 02-06, -20: days 18270, 18284... since 1970, multiples of 14.
    opens(
        "14D",
        &[
            1_578_528_000_000,
            1_579_737_600_000,
            1_580_947_200_000,
            1_582_156_800_000,
        ],
    );
}

// MARK: Candles

/// What `candles/trade:1h:tBTCUSD/hist?limit=3` answered: newest first, `[MTS, OPEN, CLOSE, HIGH,
/// LOW, VOLUME]`.
const HOURLY: &str = "[[1795240800000,56054,55952,56130,55930,45.53082668],[1795237200000,56572,56054,56721,55932,114.3192368],[1795233600000,56931,56513,57083,55782,528.04075879]]";

#[test]
fn a_candle_row_has_its_close_before_its_high_and_comes_oldest_first() {
    let candles = parse_candles(HOURLY, Timeframe::HOUR_1).unwrap();
    assert_eq!(
        candles.iter().map(|c| c.open_time).collect::<Vec<_>>(),
        [1_795_233_600_000, 1_795_237_200_000, 1_795_240_800_000]
    );
    let newest = &candles[2];
    assert_eq!(
        (
            newest.open,
            newest.high,
            newest.low,
            newest.close,
            newest.volume
        ),
        (56_054.0, 56_130.0, 55_930.0, 55_952.0, 45.53082668)
    );
    assert_eq!(newest.close_time, 1_795_240_800_000 + HOUR - 1);
}

#[test]
fn an_empty_answer_is_no_candles_and_a_refusal_is_an_error() {
    assert_eq!(parse_candles("[]", Timeframe::HOUR_1), Ok(Vec::new()));
    for (body, text) in [
        (
            r#"["error",10020,"limit: invalid"]"#,
            "Bitfinex: limit: invalid",
        ),
        (r#"["error",20000]"#, "Bitfinex: The request failed"),
    ] {
        assert_eq!(
            parse_candles(body, Timeframe::HOUR_1),
            Err(ProviderError::other(text)),
            "{body}"
        );
    }
    // The limit's own answer is a rate limit, so the chart reads it as one and waits.
    let limited = parse_candles(r#"["error", 11010, "ratelimit: error"]"#, Timeframe::HOUR_1);
    assert_eq!(limited, Err(too_fast()));
    assert!(limited.unwrap_err().is_rate_limited());
    for broken in ["not json", "{}", "[[1,2,3]]", r#"[["x",1,2,3,4,5]]"#] {
        assert!(
            matches!(
                parse_candles(broken, Timeframe::HOUR_1),
                Err(ProviderError::Data(_))
            ),
            "{broken}"
        );
    }
}

// MARK: Market list

#[test]
fn the_market_list_leaves_out_the_paper_markets() {
    let body =
        r#"[["AAVE:USD","ADABTC","BTCUSD","TESTBTC:TESTUSD","TESTBTC:TESTUSDT","AVAX:UST"]]"#;
    assert_eq!(
        parse_pairs(body).unwrap(),
        ["AAVE:USD", "ADABTC", "BTCUSD", "AVAX:UST"]
    );
    assert!(parse_pairs("[]").is_err());
    assert!(parse_pairs(r#"["error",10020,"x"]"#).is_err());
    assert!(parse_pairs("nope").is_err());
}

fn listed() -> Vec<String> {
    [
        "AVAX:USD", "AVAX:UST", "BTCUSD", "BTCUST", "BTC:XAUT", "ETHBTC",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn search_reads_any_spelling_of_a_pair_and_tether_as_bitfinex_calls_it() {
    let found = |needle: &str| -> Vec<String> {
        matching_pairs(&listed(), needle)
            .into_iter()
            .map(|hit| hit.symbol)
            .collect()
    };
    assert_eq!(found("BTCUSD"), ["BTCUSD"]);
    assert_eq!(found("AVAXUSD"), ["AVAX:USD"]);
    assert_eq!(
        found("BTC"),
        ["BTCUSD", "BTCUST", "BTC:XAUT", "ETHBTC", "AVAX:BTC"][..4]
    );
    // Tether is `UST` there.
    assert_eq!(found("BTCUSDT"), ["BTCUST"]);
    assert_eq!(found("AVAXUSDT"), ["AVAX:UST"]);
    assert!(found("NOPE").is_empty());
}

#[test]
fn a_hit_names_its_two_assets() {
    let hits = matching_pairs(&listed(), "AVAXUSD");
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.source, DataSource::Bitfinex);
    assert_eq!(
        (hit.full_symbol.as_str(), hit.kind),
        ("AVAX:USD", Default::default())
    );
    assert_eq!(
        (hit.base.as_deref(), hit.quote.as_deref()),
        (Some("AVAX"), Some("USD"))
    );
}

// MARK: Requests

/// Records the paths asked and answers each with the next of `answers`.
struct Replies {
    asked: RefCell<Vec<String>>,
    answers: RefCell<Vec<Result<String, ProviderError>>>,
}

impl Replies {
    fn new(answers: Vec<Result<String, ProviderError>>) -> Self {
        let mut answers = answers;
        answers.reverse();
        Self {
            asked: RefCell::default(),
            answers: RefCell::new(answers),
        }
    }

    fn ask(&self, path: &str) -> Result<String, ProviderError> {
        self.asked.borrow_mut().push(path.to_owned());
        self.answers.borrow_mut().pop().expect("an answer left")
    }
}

#[test]
fn a_page_is_the_candles_up_to_a_millisecond_before_where_the_last_one_began() {
    let replies = Replies::new(vec![Ok(HOURLY.to_owned())]);
    let page = page_before_with(
        |path| replies.ask(path),
        || unreachable!("a page with candles is not doubted"),
        "AVAX:USD",
        Timeframe::HOUR_1,
        1_795_244_400_000,
        3,
        0,
    )
    .unwrap();
    assert_eq!(page.len(), 3);
    assert_eq!(
        *replies.asked.borrow(),
        ["/candles/trade:1h:tAVAX:USD/hist?limit=3&end=1795244399999"]
    );
}

#[test]
fn a_request_never_asks_for_more_than_a_page_holds() {
    let replies = Replies::new(vec![Ok(HOURLY.to_owned())]);
    page_before_with(
        |path| replies.ask(path),
        || true,
        "BTCUSD",
        Timeframe::DAY_1,
        10,
        50_000,
        0,
    )
    .unwrap();
    assert_eq!(
        *replies.asked.borrow(),
        ["/candles/trade:1D:tBTCUSD/hist?limit=10000&end=9"]
    );
}

#[test]
fn an_empty_page_is_the_start_of_the_history_only_when_the_newest_candle_is_still_answered() {
    let replies = Replies::new(vec![Ok("[]".to_owned()), Ok(HOURLY.to_owned())]);
    let page = page_before_with(
        |path| replies.ask(path),
        || unreachable!("the newest candle answered"),
        "BTCUSD",
        Timeframe::HOUR_1,
        1_000,
        500,
        1_795_244_400_000,
    );
    assert_eq!(page, Ok(Vec::new()));
    // The second request is for one candle, at an end no other request of the fetch used.
    assert_eq!(
        replies.asked.borrow()[1],
        "/candles/trade:1h:tBTCUSD/hist?limit=1&end=1795244400000"
    );
}

#[test]
fn an_empty_page_beside_an_empty_newest_candle_is_no_candles_at_this_length_for_a_listed_market() {
    let replies = Replies::new(vec![Ok("[]".to_owned()), Ok("[]".to_owned())]);
    let page = page_before_with(
        |path| replies.ask(path),
        || true,
        "BTCUSD",
        Timeframe::HOUR_1,
        1_000,
        500,
        2_000,
    );
    assert_eq!(
        page,
        Ok(Vec::new()),
        "a listed market with no candle of the length has nothing before, which is no limit"
    );
    assert_eq!(replies.asked.borrow().len(), 2);
}

#[test]
fn a_limit_on_the_newest_candle_request_is_a_rate_limit() {
    for limited in [
        Err(too_fast()),
        Ok(r#"["error",11010,"ratelimit: error"]"#.to_owned()),
    ] {
        let replies = Replies::new(vec![Ok("[]".to_owned()), limited]);
        let error = page_before_with(
            |path| replies.ask(path),
            || unreachable!("the limit answers before the list is asked"),
            "BTCUSD",
            Timeframe::HOUR_1,
            1_000,
            500,
            2_000,
        );
        let error = error.unwrap_err();
        assert!(error.is_rate_limited(), "{error:?}");
    }
}

#[test]
fn an_empty_page_beside_an_empty_newest_candle_is_an_unknown_market_for_an_unlisted_one() {
    let replies = Replies::new(vec![Ok("[]".to_owned()), Ok("[]".to_owned())]);
    let error = page_before_with(
        |path| replies.ask(path),
        || false,
        "NOPEUSD",
        Timeframe::HOUR_1,
        1_000,
        500,
        2_000,
    )
    .unwrap_err();
    assert_eq!(
        error,
        ProviderError::unknown_market(DataSource::Bitfinex, "NOPEUSD", "")
    );
    assert!(!error.is_rate_limited());
}

#[test]
fn a_failed_request_is_the_answer_and_asks_nothing_more() {
    let replies = Replies::new(vec![Err(ProviderError::Network(
        crate::service::net::http::Error::Status(429),
    ))]);
    let error = page_before_with(
        |path| replies.ask(path),
        || true,
        "BTCUSD",
        Timeframe::HOUR_1,
        1_000,
        500,
        2_000,
    )
    .unwrap_err();
    assert!(error.is_rate_limited());
    assert_eq!(replies.asked.borrow().len(), 1);
}

#[test]
fn text_that_names_no_market_is_refused_before_a_request() {
    let error = Bitfinex
        .fetch_klines_range("BTC&USD", Timeframe::HOUR_1, 0, 1, 10)
        .unwrap_err();
    assert_eq!(
        error,
        ProviderError::unknown_market(DataSource::Bitfinex, "BTC&USD", "")
    );
}

// MARK: Socket messages

const BTC_CANDLES_SUBSCRIBED: &str =
    r#"{"event":"subscribed","channel":"candles","chanId":526037,"key":"trade:1m:tBTCUSD"}"#;
const AVAX_CANDLES_SUBSCRIBED: &str =
    r#"{"event":"subscribed","channel":"candles","chanId":526038,"key":"trade:1m:tAVAX:USD"}"#;
const BTC_TICKER_SUBSCRIBED: &str = r#"{"event":"subscribed","channel":"ticker","chanId":497842,"symbol":"tBTCUSD","pair":"BTCUSD"}"#;
const SNAPSHOT: &str = "[526037,[[1795242480000,56242,56325,56365,56242,0.79727488],[1795242420000,56248,56241,56350,56241,0.6492354],[1795242360000,56278,56251,56286,56199,0.5690327000000001]]]";
const UPDATE: &str = "[526037,[1795242480000,56242,56366,56366,56242,0.80231289]]";
const TICKER_PUSH: &str = "[497842,[56267,5.82147853,56330,55.21766269,-5657,-0.09132442,56287,3862.78416576,62340,55501,1358182043000]]";

#[test]
fn a_subscribed_event_says_what_its_channel_number_is() {
    let value = json(BTC_CANDLES_SUBSCRIBED);
    assert_eq!(
        message(&value),
        Message::Subscribed(
            526_037,
            Subscription::Candles {
                market: "BTCUSD".to_owned(),
                period: "1m".to_owned()
            }
        )
    );
    // A market with a colon keeps it: the key splits at its first two colons only.
    let value = json(AVAX_CANDLES_SUBSCRIBED);
    assert_eq!(
        message(&value),
        Message::Subscribed(
            526_038,
            Subscription::Candles {
                market: "AVAX:USD".to_owned(),
                period: "1m".to_owned()
            }
        )
    );
    let value = json(BTC_TICKER_SUBSCRIBED);
    assert_eq!(
        message(&value),
        Message::Subscribed(
            497_842,
            Subscription::Ticker {
                market: "BTCUSD".to_owned()
            }
        )
    );
}

#[test]
fn heartbeats_refusals_notices_and_the_unreadable_are_not_data() {
    for text in [
        r#"[526037,"hb"]"#,
        r#"{"event":"info","version":2,"serverId":"x","platform":{"status":1}}"#,
        r#"{"channel":"ticker","symbol":"tNOPEUSD","event":"error","msg":"symbol: invalid","code":10300,"pair":"NOPEUSD"}"#,
        r#"{"channel":"ticker","symbol":"tZECBTC","event":"error","msg":"subscribe: limit","code":10305}"#,
        r#"{"channel":"candles","key":"trade:1m:tBTCUSD","event":"error","msg":"subscribe: dup","code":10301,"chanId":549070}"#,
        r#"{"event":"pong","cid":7,"ts":1795242609767}"#,
        // A channel this app does not read, and a subscription with no key.
        r#"{"event":"subscribed","channel":"trades","chanId":1,"symbol":"tBTCUSD"}"#,
        r#"{"event":"subscribed","channel":"candles","chanId":1}"#,
        "[]",
        "{}",
        "3",
    ] {
        assert_eq!(message(&json(text)), Message::Other, "{text}");
    }
}

#[test]
fn a_push_is_known_only_by_what_its_connection_was_told() {
    let mut channels = Channels::default();
    let value = json(UPDATE);
    // Before the connection says what 526037 is, a push on it is nobody's.
    assert!(channels.track(message(&value)).is_none());

    assert!(
        channels
            .track(message(&json(BTC_CANDLES_SUBSCRIBED)))
            .is_none()
    );
    assert!(
        channels
            .track(message(&json(BTC_TICKER_SUBSCRIBED)))
            .is_none()
    );
    let (subscription, _) = channels.track(message(&value)).expect("a known channel");
    assert!(matches!(subscription, Subscription::Candles { market, .. } if market == "BTCUSD"));
    let ticker = json(TICKER_PUSH);
    let (subscription, _) = channels.track(message(&ticker)).unwrap();
    assert!(matches!(subscription, Subscription::Ticker { .. }));
    // Another channel number is nobody's.
    assert!(channels.track(message(&json("[1,[2]]"))).is_none());

    // A new connection starts from nothing.
    channels.clear();
    assert!(channels.track(message(&value)).is_none());
}

// MARK: Live candles

fn read(reader: &mut dyn FrameReader, text: &str) -> Option<(String, Kline)> {
    reader.read(Frame::Text(text))
}

fn candle_reader(base: Timeframe) -> Box<dyn FrameReader> {
    CandleProtocol::reader(&Bitfinex, base).expect("Bitfinex reads its candles through a reader")
}

const RESTARTING: &str =
    r#"{"event":"info","code":20051,"msg":"Stop/Restart Websocket Server (please reconnect)"}"#;
const MAINTENANCE_BEGINS: &str = r#"{"event":"info","code":20060,"msg":"Entering in Maintenance mode. Please pause any activity and resume after receiving the info message 20061 (it should take 120 seconds at most)."}"#;
const MAINTENANCE_ENDS: &str = r#"{"event":"info","code":20061,"msg":"Maintenance ended. You can resume normal activity. It is advised to unsubscribe/subscribe again all channels."}"#;

#[test]
fn the_server_asking_for_a_new_connection_is_a_message_of_its_own() {
    for (text, asks) in [
        (RESTARTING, true),
        (MAINTENANCE_ENDS, true),
        // Pausing is what a client that sends nothing does.
        (MAINTENANCE_BEGINS, false),
        (
            r#"{"event":"info","version":2,"platform":{"status":1}}"#,
            false,
        ),
        (r#"{"event":"info","code":"20051"}"#, false),
    ] {
        assert_eq!(message(&json(text)) == Message::Reconnect, asks, "{text}");
    }
}

#[test]
fn a_candle_feed_is_told_to_reconnect_once_for_each_request_of_the_server() {
    let mut reader = candle_reader(Timeframe::MINUTE_1);
    assert!(!reader.reconnect());
    assert!(read(&mut *reader, BTC_CANDLES_SUBSCRIBED).is_none());
    assert!(!reader.reconnect(), "a subscription asks for nothing");
    assert!(read(&mut *reader, MAINTENANCE_BEGINS).is_none());
    assert!(!reader.reconnect());
    for text in [RESTARTING, MAINTENANCE_ENDS] {
        assert!(read(&mut *reader, text).is_none());
        assert!(reader.reconnect(), "{text}");
        assert!(!reader.reconnect(), "asked for once");
    }
    // A request that came before the new connection is not the new connection's.
    read(&mut *reader, RESTARTING);
    reader.reset();
    assert!(!reader.reconnect());
}

#[test]
fn a_ticker_feed_is_told_to_reconnect_too() {
    let mut reader = tick_reader();
    assert!(!reader.reconnect());
    assert!(reader.read(RESTARTING).is_none());
    assert!(reader.reconnect());
    assert!(!reader.reconnect());
}

#[test]
fn a_candle_feed_reads_the_snapshot_and_the_updates_after_its_subscription() {
    let mut reader = candle_reader(Timeframe::MINUTE_1);
    assert!(
        read(
            &mut *reader,
            r#"{"event":"info","version":2,"platform":{"status":1}}"#
        )
        .is_none()
    );
    assert!(read(&mut *reader, BTC_CANDLES_SUBSCRIBED).is_none());

    // The snapshot's newest candle (the forming one) is the first of its rows.
    let (market, kline) = read(&mut *reader, SNAPSHOT).expect("the snapshot's newest candle");
    assert_eq!(market, "BTCUSD");
    assert_eq!(kline.open_time, 1_795_242_480_000);
    assert_eq!(
        (kline.high, kline.low, kline.close),
        (56_365.0, 56_242.0, 56_325.0)
    );

    let (market, kline) = read(&mut *reader, UPDATE).expect("an update");
    assert_eq!(market, "BTCUSD");
    assert_eq!((kline.close, kline.volume), (56_366.0, 0.80231289));
    assert_eq!(kline.close_time, 1_795_242_480_000 + 59_999);

    // The previous candle comes with an update too, and is read like any other.
    let again = "[526037,[1795242420000,56248,56241,56350,56241,0.6492354]]";
    assert_eq!(
        read(&mut *reader, again).unwrap().1.open_time,
        1_795_242_420_000
    );

    for ignored in [
        r#"[526037,"hb"]"#,
        // A market Bitfinex does not list is subscribed, and answers an empty snapshot.
        "[526037,[]]",
        // Its ticker channel is another feed's.
        TICKER_PUSH,
        r#"{"channel":"candles","key":"trade:1m:tBTCUSD","event":"error","msg":"subscribe: dup","code":10301,"chanId":549070}"#,
        "not json",
    ] {
        assert!(read(&mut *reader, ignored).is_none(), "{ignored}");
    }
}

#[test]
fn a_candle_feed_names_a_market_with_a_colon_as_the_app_does() {
    let mut reader = candle_reader(Timeframe::MINUTE_1);
    read(&mut *reader, AVAX_CANDLES_SUBSCRIBED);
    let (market, kline) = read(
        &mut *reader,
        "[526038,[1795242480000,6.1249,6.1231,6.1257,6.1141,118.49432632]]",
    )
    .unwrap();
    assert_eq!(market, "AVAX:USD");
    assert_eq!(kline.close, 6.1231);
    // The chart reads the name through the profile, so any spelling of it matches.
    assert!(
        InstrumentId::new(DataSource::Bitfinex, "avax/usd")
            .is_same_market(&InstrumentId::new(DataSource::Bitfinex, &market))
    );
}

#[test]
fn a_candle_feed_of_one_length_ignores_the_channels_of_another() {
    let mut reader = candle_reader(Timeframe::HOUR_1);
    read(&mut *reader, BTC_CANDLES_SUBSCRIBED);
    assert!(
        read(&mut *reader, UPDATE).is_none(),
        "a 1m channel in a 1h feed"
    );
    read(
        &mut *reader,
        r#"{"event":"subscribed","channel":"candles","chanId":9,"key":"trade:1h:tBTCUSD"}"#,
    );
    assert!(
        read(
            &mut *reader,
            "[9,[1795240800000,56054,55952,56130,55930,45.5]]"
        )
        .is_some()
    );
}

#[test]
fn a_candle_feed_forgets_its_channels_when_the_connection_is_made_again() {
    let mut reader = candle_reader(Timeframe::MINUTE_1);
    read(&mut *reader, BTC_CANDLES_SUBSCRIBED);
    assert!(read(&mut *reader, UPDATE).is_some());

    reader.reset();
    // The new connection may number its channels as it pleases: 526037 is nobody until it says.
    assert!(read(&mut *reader, UPDATE).is_none());
    read(
        &mut *reader,
        AVAX_CANDLES_SUBSCRIBED.replace("526038", "526037").as_str(),
    );
    assert_eq!(read(&mut *reader, UPDATE).unwrap().0, "AVAX:USD");
}

#[test]
fn the_newest_candle_of_a_snapshot_is_found_wherever_it_stands() {
    let value = json("[[1000,1,2,3,0.5,9],[3000,1,2,3,0.5,9],[2000,1,2,3,0.5,9]]");
    assert_eq!(
        latest(&value, Timeframe::MINUTE_1).unwrap().open_time,
        3_000
    );
    assert!(latest(&json("[]"), Timeframe::MINUTE_1).is_none());
    assert!(latest(&json("[1,2]"), Timeframe::MINUTE_1).is_none());
}

#[test]
fn markets_are_shared_out_over_connections_of_thirty_subscriptions() {
    let symbols: Vec<String> = (0..70)
        .map(|index| {
            format!(
                "{}USD",
                ["AAA", "BBB", "CCC", "DDD", "EEE", "FFF", "GGG"][index % 7]
            )
        })
        .chain((0..70).map(|index| format!("X{index}ZZ:USD")))
        .collect();
    // 7 distinct names among the first 70, then 70 more.
    let sessions = Bitfinex
        .sessions(&symbols, Timeframe::MINUTE_1)
        .expect("a session");
    let sizes: Vec<usize> = sessions
        .iter()
        .map(|session| session.subscribe.len())
        .collect();
    assert_eq!(sizes, [30, 30, 17]);
    assert_eq!(CHANNELS_PER_CONNECTION, 30);
    let first = &sessions[0];
    assert_eq!(first.url, "wss://api-pub.bitfinex.com/ws/2");
    assert_eq!(first.subscribe[0], subscribe_candles("1m", "tAAAUSD"));
    assert_eq!(
        first.subscribe[0],
        r#"{"channel":"candles","event":"subscribe","key":"trade:1m:tAAAUSD"}"#
    );
    assert!(matches!(first.keepalive, Keepalive::Text { .. }));
}

#[test]
fn a_feed_of_no_market_or_a_length_bitfinex_lacks_opens_nothing() {
    assert!(Bitfinex.sessions(&[], Timeframe::MINUTE_1).is_none());
    assert!(
        Bitfinex
            .sessions(&["???".to_owned()], Timeframe::MINUTE_1)
            .is_none()
    );
    assert!(
        Bitfinex
            .sessions(&["BTCUSD".to_owned()], tf("2m"))
            .is_none()
    );
}

#[test]
fn the_keepalive_asks_with_a_ping_event_and_takes_its_pong_for_no_data() {
    let keepalive = socket::session(Vec::new()).keepalive;
    assert!(matches!(
        keepalive,
        Keepalive::Text { ping, pong }
            if ping == r#"{"event":"ping"}"# && r#"{"event":"pong","cid":7,"ts":1795242609767}"#.starts_with(pong)
    ));
    assert!(matches!(
        keepalive.probe(),
        Some(tungstenite::Message::Text(text)) if text.as_str() == r#"{"event":"ping"}"#
    ));
}

// MARK: Prices

fn tick_reader() -> Box<dyn TickReader> {
    QuoteProtocol::reader(&Bitfinex).expect("Bitfinex reads its tickers through a reader")
}

#[test]
fn the_ticker_channel_prices_a_market_with_the_price_a_day_ago() {
    let mut reader = tick_reader();
    assert!(
        reader.read(TICKER_PUSH).is_none(),
        "a channel it was not told about"
    );
    assert!(reader.read(BTC_TICKER_SUBSCRIBED).is_none());
    let tick = reader.read(TICKER_PUSH).expect("a tick");
    assert_eq!(tick.source, DataSource::Bitfinex);
    assert_eq!(tick.product_id, "BTCUSD");
    assert_eq!(tick.quote.price, 56_287.0);
    // The last trade less the change over the last 24 hours (-5657).
    assert_eq!(tick.quote.previous_day_price, Some(61_944.0));
    assert_eq!(
        (tick.quote.volume_24h, tick.quote.volume_kind),
        (Some(3862.78416576), Some(VolumeKind::Base))
    );

    for ignored in [
        r#"[497842,"hb"]"#,
        r#"{"channel":"ticker","symbol":"tNOPEUSD","event":"error","msg":"symbol: invalid","code":10300}"#,
        // A candle channel is another feed's.
        "[526037,[1795242480000,56242,56366,56366,56242,0.8]]",
        "[497842,[]]",
        "not json",
    ] {
        assert!(reader.read(ignored).is_none(), "{ignored}");
    }
    reader.reset();
    assert!(
        reader.read(TICKER_PUSH).is_none(),
        "the old connection's numbers are void"
    );
}

#[test]
fn the_ticker_socket_subscribes_every_market_once_up_to_what_a_connection_takes() {
    let products: Vec<String> = ["BTCUSD", "BTC/USD", "AVAX:USD", "bad?"]
        .iter()
        .map(|product| (*product).to_owned())
        .chain((0..40).map(|index| format!("C{index}XX:USD")))
        .collect();
    let session = QuoteProtocol::session(&Bitfinex, &products);
    assert_eq!(session.subscribe.len(), CHANNELS_PER_CONNECTION);
    assert_eq!(session.subscribe[0], subscribe_ticker("tBTCUSD"));
    assert_eq!(
        session.subscribe[1],
        r#"{"channel":"ticker","event":"subscribe","symbol":"tAVAX:USD"}"#
    );
}

/// What `/v2/tickers?symbols=tBTCUSD,tAVAX:USD,tNOPEUSD,tETHBTC` answered: a row per listed market.
const TICKERS: &str = r#"[["tBTCUSD",56247,7.99515427,56308,109.17220986,-5652,-0.0912437,56292,3862.78416576,62340,55501,1358182043000],["tAVAX:USD",6.1137,13536.05157551,6.1256,5607.34922576,-0.7836,-0.11360309,6.1141,13363.02241427,6.946,6.0721,1600862465185],["tETHBTC",0.027968,527.82470098,0.027979,468.41440286,-0.000222,-0.00787234,0.027978,5779.44350998,0.028372,0.027222,1457539859000]]"#;

#[test]
fn a_ticker_answer_prices_the_markets_asked_for_under_the_names_they_were_asked_as() {
    let requests = [
        request("BTCUSD"),
        request("AVAX:USD"),
        request("NOPEUSD"),
        request("ETHBTC"),
    ];
    let quotes = parse_tickers(TICKERS, &requests).unwrap();
    assert_eq!(quotes.len(), 3, "a market Bitfinex left out has no price");
    assert_eq!(quotes["BTCUSD"].price, 56_292.0);
    assert_eq!(quotes["BTCUSD"].previous_day_price, Some(61_944.0));
    let avax = quotes["AVAX:USD"];
    assert_eq!(avax.price, 6.1141);
    assert!((avax.previous_day_price.unwrap() - 6.8977).abs() < 1e-9);
    assert_eq!(quotes["ETHBTC"].volume_kind, Some(VolumeKind::Base));
}

#[test]
fn a_ticker_with_no_trade_has_no_price_and_an_error_answer_is_a_failure() {
    let silent = r#"[["tBTCUSD",0,0,0,0,0,0,0,0,0,0,0]]"#;
    assert!(
        parse_tickers(silent, &[request("BTCUSD")])
            .unwrap()
            .is_empty()
    );
    for body in [r#"["error",10020,"x"]"#, "nope", "{}"] {
        assert!(
            matches!(
                parse_tickers(body, &[request("BTCUSD")]),
                Err(QuoteFailure::Failed(_))
            ),
            "{body}"
        );
    }
}

// MARK: Live

/// Run with `cargo test --release --lib bitfinex::tests::live_bitfinex_serves -- --ignored
/// --nocapture`. Candle requests are answered a few seconds apart by the schedule, so it takes a
/// minute.
#[test]
#[ignore = "needs the network"]
fn live_bitfinex_serves_candles_pages_back_and_streams() {
    use crate::service::market::live::LiveFeed;
    use crate::service::watchlist_quotes::quote_stream::QuoteSocket;
    use std::time::{Duration, Instant};

    let consecutive = |candles: &[Kline], step: i64| {
        candles
            .windows(2)
            .all(|pair| pair[1].open_time - pair[0].open_time == step)
    };

    let hourly = BITFINEX
        .fetch_klines("BTCUSD", Timeframe::HOUR_1, 500)
        .unwrap();
    println!(
        "BTCUSD 1h: {} candles, newest opens {}",
        hourly.len(),
        hourly.last().unwrap().open_time
    );
    assert_eq!(hourly.len(), 500);
    assert!(consecutive(&hourly, HOUR));
    assert!(
        hourly
            .iter()
            .all(|c| c.high >= c.low && c.volume >= 0.0 && c.close > 0.0)
    );

    // The page before, joined at the hour the oldest candle opens.
    let oldest = hourly[0].open_time;
    let older = BITFINEX
        .fetch_klines_range("BTCUSD", Timeframe::HOUR_1, 0, oldest, 500)
        .unwrap();
    assert_eq!(older.len(), 500);
    assert_eq!(older.last().unwrap().open_time, oldest - HOUR);

    // A pair with a colon in its name.
    let avax = BITFINEX
        .fetch_klines("AVAX:USD", Timeframe::HOUR_1, 100)
        .unwrap();
    println!("AVAX:USD 1h: {} candles", avax.len());
    assert!(avax.len() > 50 && avax.iter().all(|c| c.close > 0.0));

    // The page before a market's first candle is empty, which the newest candle's answer confirms.
    let daily = BITFINEX
        .fetch_klines("BTCUSD", Timeframe::DAY_1, 6_000)
        .unwrap();
    println!(
        "BTCUSD 1D everything: {} candles from {}",
        daily.len(),
        daily[0].open_time
    );
    let before = BITFINEX
        .fetch_klines_range("BTCUSD", Timeframe::DAY_1, 0, daily[0].open_time, 500)
        .unwrap();
    assert!(before.is_empty());

    // A market Bitfinex does not list is told so, not mistaken for a limit.
    let error = BITFINEX
        .fetch_klines("NOPEUSD", Timeframe::HOUR_1, 10)
        .unwrap_err();
    println!("NOPEUSD: {error}");
    assert_eq!(
        error,
        ProviderError::unknown_market(DataSource::Bitfinex, "NOPE:USD", "")
    );

    // Search.
    let hits = BITFINEX.search_tickers("avax usdt").unwrap();
    println!(
        "search avax usdt: {:?}",
        hits.iter().map(|h| &h.symbol).collect::<Vec<_>>()
    );
    assert!(hits.iter().any(|hit| hit.symbol == "AVAX:UST"));

    // Prices over REST, then the sockets.
    let quotes = ticker::fetch_quotes(&[request("BTCUSD"), request("AVAX:USD")]).unwrap();
    println!("REST quotes: {quotes:?}");
    assert!(quotes["BTCUSD"].price > 0.0 && quotes["AVAX:USD"].previous_day_price.is_some());

    let feed = LiveFeed::connect(
        DataSource::Bitfinex,
        vec!["BTCUSD".to_owned(), "AVAX:USD".to_owned()],
        Timeframe::MINUTE_1,
        eframe::egui::Context::default(),
    )
    .expect("Bitfinex streams one-minute candles");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _socket = QuoteSocket::connect(
        DataSource::Bitfinex,
        vec!["BTCUSD".to_owned(), "AVAX:USD".to_owned()],
        sender,
    )
    .expect("Bitfinex has a ticker socket");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = std::collections::BTreeSet::new();
    while seen.len() < 2 {
        for (symbol, kline) in feed.inbox.drain() {
            assert!(kline.close > 0.0 && kline.close_time > kline.open_time);
            seen.insert(symbol);
        }
        assert!(Instant::now() < deadline, "only {seen:?} in 30 s");
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("live candles: {seen:?}");
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["AVAX:USD", "BTCUSD"]);
    let tick = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a tick in 30 s");
    println!("live tick: {tick:?}");
    assert_eq!(tick.source, DataSource::Bitfinex);
    assert!(["BTCUSD", "AVAX:USD"].contains(&tick.product_id.as_str()));
    assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
}

/// Every candle length the chart offers opens where Binance's does, the folded ones included. Run
/// with `cargo test --release --lib bitfinex::tests::live_bitfinex_buckets -- --ignored
/// --nocapture`; the all-venues comparison is `live_sources_cut_the_same_buckets`. Bitfinex answers
/// a request every 2.5 seconds, so this takes a few minutes.
#[test]
#[ignore = "needs the network"]
fn live_bitfinex_buckets_match_binances_at_every_timeframe() {
    let odd = ["5m", "22m", "3h", "4h", "6h", "12h", "2D", "3D", "2W", "6M"].map(tf);
    for interval in Timeframe::LEGACY.into_iter().chain(odd) {
        let opens = |source, symbol| -> std::collections::BTreeSet<i64> {
            crate::service::fetch_klines(source, symbol, interval, 500)
                .unwrap_or_else(|error| panic!("{source:?} {interval}: {error}"))
                .iter()
                .map(|bar| bar.open_time)
                .collect()
        };
        let binance = opens(DataSource::Binance, "ETHUSDT");
        let bitfinex = opens(DataSource::Bitfinex, "ETHUSD");
        let from = binance.first().max(bitfinex.first()).copied().unwrap();
        let within = |set: &std::collections::BTreeSet<i64>| -> Vec<i64> {
            set.iter().copied().filter(|time| *time >= from).collect()
        };
        println!(
            "{interval:>3}: binance {:>4} bitfinex {:>4} | shared range {:>4} bars",
            binance.len(),
            bitfinex.len(),
            within(&binance).len()
        );
        assert!(within(&bitfinex).len() > 2, "{interval}");
        assert_eq!(
            within(&binance),
            within(&bitfinex),
            "{interval} binance vs bitfinex"
        );
    }
}
