use super::live::parse_candle;
use super::markets::{Market, id_of, ids_of, matching, parse_markets, resolve, symbol_of};
use super::parse::{answer, parse_candles};
use super::quotes::parse_tick;
use super::rest::{PAGING, page_before_with};
use super::ticker::parse_stats;
use super::*;
use crate::model::InstrumentId;
use crate::model::market::MarketKind;
use crate::model::market::bucket::bucket_start_ms;
use crate::model::watchlist;
use crate::model::watchlist::quote::VolumeKind;
use crate::service::net::http;
use crate::service::watchlist_quotes::provider::request;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

#[test]
fn every_native_has_its_resolution_and_nothing_else_does() {
    let resolutions: Vec<String> = DataSource::Lighter
        .natives()
        .iter()
        .map(|native| interval(*native).expect("a native has a resolution"))
        .collect();
    assert_eq!(
        resolutions,
        ["1m", "5m", "15m", "30m", "1h", "4h", "12h", "1d"]
    );
    // Lighter answers `invalid param` to each of these: they fold from the bars below.
    for other in ["3m", "2h", "6h", "8h", "3d", "1w", "1M", "3M", "1Y", "22m"] {
        assert_eq!(interval(tf(other)), None, "{other}");
    }
}

/// The newest bar of each resolution the API answered for BTC on 2026-11-21 (04:2x UTC): it cuts
/// where this app does, so every native is a native.
#[test]
fn the_bars_lighter_opens_are_on_this_apps_grid() {
    for (resolution, open) in [
        ("1m", 1_795_234_560_000),
        ("5m", 1_795_234_500_000),
        ("15m", 1_795_234_500_000),
        ("30m", 1_795_233_600_000),
        ("1h", 1_795_233_600_000),
        ("4h", 1_795_233_600_000),
        ("12h", 1_795_219_200_000),
        ("1d", 1_795_219_200_000),
    ] {
        let native = tf(resolution);
        assert_eq!(bucket_start_ms(native, open), open, "{resolution}");
        assert!(DAY % native.nominal_ms() == 0 || native.nominal_ms() % DAY == 0);
    }
}

fn base_of(timeframe: &str) -> (String, usize) {
    let plan = plan(DataSource::Lighter, tf(timeframe)).unwrap();
    (interval(plan.base()).unwrap(), plan.base_count(500))
}

#[test]
fn what_it_cannot_cut_folds_the_coarsest_native_that_nests() {
    assert_eq!(base_of("1h"), ("1h".to_owned(), 500));
    assert_eq!(base_of("4h"), ("4h".to_owned(), 500));
    assert_eq!(base_of("3m"), ("1m".to_owned(), 501 * 3));
    assert_eq!(base_of("2h"), ("1h".to_owned(), 501 * 2));
    assert_eq!(base_of("6h"), ("1h".to_owned(), 501 * 6));
    assert_eq!(base_of("8h"), ("4h".to_owned(), 501 * 2));
    assert_eq!(base_of("3D"), ("1d".to_owned(), 501 * 3));
    assert_eq!(base_of("1W"), ("1d".to_owned(), 501 * 7));
    assert_eq!(base_of("1M"), ("1d".to_owned(), 501 * 31));
    assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
}

const CANDLES: &str = r#"{"code":200,"r":"1d","c":[
    {"t":1795132800000,"o":61989,"h":62641.8,"l":57726.9,"c":58927.5,"v":24912.16768,"V":1499116302.271221,"i":41894696524},
    {"t":1795046400000,"o":63076.8,"h":64086.1,"l":61435,"c":61989,"v":23460.55682,"V":1475465705.18709,"i":41577655545},
    {"t":1795219200000,"o":58927.5,"h":59225,"l":55510,"c":55720.1,"v":7153.22103,"V":411216867.640171,"i":41934526283}]}"#;

#[test]
fn parses_candles_oldest_first_each_closing_where_its_bucket_does() {
    let candles = parse_candles(CANDLES, Timeframe::DAY_1).unwrap();
    let opens: Vec<i64> = candles.iter().map(|candle| candle.open_time).collect();
    assert_eq!(
        opens,
        [1_795_046_400_000, 1_795_132_800_000, 1_795_219_200_000]
    );
    let first = &candles[0];
    assert_eq!(first.close_time, first.open_time + DAY - 1);
    assert_eq!(
        (first.open, first.high, first.low, first.close, first.volume),
        (63076.8, 64086.1, 61435.0, 61989.0, 23460.55682)
    );
}

#[test]
fn an_answer_that_is_a_refusal_or_not_candles_is_an_error_naming_it() {
    let refusal = parse_candles(
        r#"{"code":20001,"message":"invalid param "}"#,
        Timeframe::DAY_1,
    );
    assert_eq!(
        refusal,
        Err(ProviderError::other(
            "Lighter answered 20001: invalid param "
        ))
    );
    assert!(matches!(
        parse_candles(r#"{"code":200,"r":"1d"}"#, Timeframe::DAY_1),
        Err(ProviderError::Data(_))
    ));
    assert!(matches!(
        parse_candles(
            r#"{"code":200,"c":[{"t":1,"o":1,"h":1,"l":1,"c":1}]}"#,
            Timeframe::DAY_1
        ),
        Err(ProviderError::Data(message)) if message.contains('v')
    ));
    assert!(matches!(
        parse_candles("[]", Timeframe::DAY_1),
        Err(ProviderError::Data(_))
    ));
    assert!(matches!(
        parse_candles("not json", Timeframe::DAY_1),
        Err(ProviderError::Data(_))
    ));
    // No candle in range is an answer, not an error.
    assert_eq!(
        parse_candles(r#"{"code":200,"r":"1d","c":[]}"#, Timeframe::DAY_1),
        Ok(Vec::new())
    );
    assert!(answer(r#"{"code":200}"#).is_ok());
}

const ORDER_BOOKS: &str = r#"{"code":200,"order_books":[
    {"symbol":"ETH","market_id":0,"market_type":"perp","base_asset_id":0,"quote_asset_id":0,"status":"active","taker_fee":"0.0000"},
    {"symbol":"BTC","market_id":1,"market_type":"perp","status":"active"},
    {"symbol":"1000PEPE","market_id":4,"market_type":"perp","status":"active"},
    {"symbol":"US500","market_id":10001,"market_type":"perp","status":"active"},
    {"symbol":"SPX","market_id":42,"market_type":"perp","status":"active"},
    {"symbol":"WHEAT","market_id":99,"market_type":"perp","status":"inactive"},
    {"symbol":"ETH/USDC","market_id":2048,"market_type":"spot","status":"active"},
    {"symbol":"","market_id":7,"market_type":"perp","status":"active"},
    {"symbol":"NOID","market_type":"perp","status":"active"},
    {"symbol":"BIG","market_id":99999999999,"market_type":"perp","status":"active"}]}"#;

fn markets() -> Vec<Market> {
    parse_markets(ORDER_BOOKS).unwrap()
}

#[test]
fn only_the_active_perpetuals_with_a_symbol_and_a_number_are_listed() {
    let listed: Vec<(u32, String)> = markets()
        .into_iter()
        .map(|market| (market.id, market.symbol))
        .collect();
    assert_eq!(
        listed,
        [
            (0, "ETH".to_owned()),
            (1, "BTC".to_owned()),
            (4, "1000PEPE".to_owned()),
            (10001, "US500".to_owned()),
            (42, "SPX".to_owned()),
        ]
    );
    for body in [
        r#"{"code":200}"#,
        r#"{"code":200,"order_books":{}}"#,
        "null",
    ] {
        assert!(
            matches!(parse_markets(body), Err(ProviderError::Data(_))),
            "{body}"
        );
    }
}

#[test]
fn a_symbol_is_resolved_to_its_number_and_back_however_it_is_cased() {
    let markets = markets();
    assert_eq!(id_of(&markets, "BTC"), Some(1));
    assert_eq!(id_of(&markets, "btc"), Some(1));
    assert_eq!(id_of(&markets, " 1000pepe "), Some(4));
    assert_eq!(
        id_of(&markets, "WHEAT"),
        None,
        "an inactive market is not listed"
    );
    assert_eq!(symbol_of(&markets, 10001), Some("US500"));
    assert_eq!(symbol_of(&markets, 3), None);
    let asked = ["NOPE".to_owned(), "eth".to_owned(), "US500".to_owned()];
    assert_eq!(ids_of(&markets, &asked).collect::<Vec<_>>(), [0, 10001]);
}

#[test]
fn a_market_the_list_does_not_have_is_an_error_that_says_so_not_a_panic() {
    let markets = markets();
    assert_eq!(resolve(&markets, "btc"), Ok(1));
    let error = resolve(&markets, "NOPE").unwrap_err();
    assert!(
        matches!(&error, ProviderError::UnknownMarket { provider: DataSource::Lighter, symbol, .. } if symbol == "NOPE"),
        "{error}"
    );
    assert!(error.to_string().starts_with("Lighter does not list NOPE"));
    assert!(resolve(&[], "BTC").is_err());
}

#[test]
fn search_finds_a_market_by_the_letters_and_digits_of_its_name_and_tags_it_a_perpetual() {
    let markets = markets();
    let symbols = |needle: &str| -> Vec<String> {
        matching(&markets, needle)
            .into_iter()
            .map(|hit| hit.full_symbol)
            .collect()
    };
    assert_eq!(symbols("BTC"), ["BTC"]);
    assert_eq!(symbols("PEPE"), ["1000PEPE"]);
    assert_eq!(symbols("US"), ["US500"]);
    assert_eq!(symbols("500"), ["US500"]);
    assert!(symbols("WHEAT").is_empty());
    let hit = &matching(&markets, "BTC")[0];
    assert_eq!(
        (hit.source, hit.symbol.as_str(), hit.kind),
        (DataSource::Lighter, "BTC", MarketKind::Perp)
    );
    assert_eq!(hit.base_asset(), "BTC");
}

#[test]
fn a_market_is_a_perpetual() {
    assert_eq!(
        LIGHTER.kind_of(&InstrumentId::new(DataSource::Lighter, "US500")),
        MarketKind::Perp
    );
}

/// The number the query of a candle request gives `name`; a request lacking one is a failure.
fn query_param(path: &str, name: &str) -> i64 {
    path.strip_prefix("/candles?")
        .expect("a candle request")
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("{name} is required: {path}"))
        .parse()
        .unwrap()
}

/// What the API keeps of BTC's hours as the candle request answers them: the newest 500 that open
/// before the end.
fn lighter_hours(
    first_open: i64,
    last_open: i64,
    asked: &std::cell::RefCell<Vec<String>>,
) -> impl Fn(&str) -> Result<String, ProviderError> + '_ {
    move |path| {
        asked.borrow_mut().push(path.to_owned());
        let param = |name: &str| query_param(path, name);
        assert_eq!(param("market_id"), 1);
        assert_eq!(param("count_back"), 500);
        let end = param("end_timestamp");
        let rows: Vec<String> = (first_open / HOUR..=last_open / HOUR)
            .map(|hour| hour * HOUR)
            .filter(|open| *open < end)
            .map(|open| format!(r#"{{"t":{open},"o":1,"h":2,"l":0.5,"c":1.5,"v":3,"V":4,"i":5}}"#))
            .collect();
        let newest = &rows[rows.len().saturating_sub(500)..];
        Ok(format!(
            r#"{{"code":200,"r":"1h","c":[{}]}}"#,
            newest.join(",")
        ))
    }
}

#[test]
fn history_is_paged_back_by_moving_the_end_to_where_the_oldest_bar_opens() {
    let asked = std::cell::RefCell::new(Vec::new());
    let now = 5_000 * HOUR + 1_234;
    let history = lighter_hours(1_000 * HOUR, 5_000 * HOUR, &asked);
    let plan = plan(DataSource::Lighter, Timeframe::HOUR_1).unwrap();
    let candles = PAGING
        .back(0, now, 1_200, |after, wanted| {
            page_before_with(&history, 1, plan.base(), after, wanted)
        })
        .unwrap();
    assert_eq!(candles.len(), 1_200);
    assert_eq!(candles.last().unwrap().open_time, 5_000 * HOUR);
    assert!(
        candles
            .windows(2)
            .all(|pair| pair[1].open_time - pair[0].open_time == HOUR)
    );
    let ends: Vec<String> = asked
        .borrow()
        .iter()
        .map(|path| query_param(path, "end_timestamp").to_string())
        .collect();
    // The first question ends at the next whole second after `now`; the next two where the page
    // before opened (a bar that opens at the end is left out), the last one asking for the 200 left.
    assert_eq!(
        ends,
        [
            (5_000 * HOUR + 2_000).to_string(),
            (4_501 * HOUR).to_string(),
            (4_001 * HOUR).to_string()
        ]
    );
    assert!(asked.borrow()[0].contains("market_id=1&resolution=1h&start_timestamp=0&"));
}

#[test]
fn a_market_that_began_later_ends_the_paging_with_a_short_page() {
    let asked = std::cell::RefCell::new(Vec::new());
    let history = lighter_hours(4_800 * HOUR, 5_000 * HOUR, &asked);
    let candles = PAGING
        .back(0, 5_001 * HOUR, 1_500, |after, wanted| {
            page_before_with(&history, 1, Timeframe::HOUR_1, after, wanted)
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
fn a_refresh_asks_one_request_and_keeps_the_newest_few() {
    let asked = std::cell::RefCell::new(Vec::new());
    let history = lighter_hours(1_000 * HOUR, 5_000 * HOUR, &asked);
    let candles = page_before_with(&history, 1, Timeframe::HOUR_1, 5_001 * HOUR, 4).unwrap();
    assert_eq!(asked.borrow().len(), 1);
    assert_eq!(
        candles
            .iter()
            .map(|candle| candle.open_time)
            .collect::<Vec<_>>(),
        [4_997 * HOUR, 4_998 * HOUR, 4_999 * HOUR, 5_000 * HOUR]
    );
}

#[test]
fn a_length_lighter_has_no_candle_for_is_unsupported_not_asked() {
    let error = page_before_with(
        |_| panic!("no request for a length it cannot answer"),
        1,
        tf("3m"),
        HOUR,
        10,
    )
    .unwrap_err();
    assert!(error.is_unsupported_interval());
}

#[test]
fn a_refusal_or_a_failed_request_is_the_answer_of_the_page() {
    let limited = page_before_with(
        |_| Err(ProviderError::Network(http::Error::Status(429))),
        1,
        Timeframe::HOUR_1,
        HOUR,
        10,
    )
    .unwrap_err();
    assert!(limited.is_rate_limited());
    let refused = page_before_with(
        |_| Ok(r#"{"code":21602,"message":"invalid market index"}"#.to_owned()),
        1,
        Timeframe::HOUR_1,
        HOUR,
        10,
    );
    assert_eq!(
        refused,
        Err(ProviderError::other(
            "Lighter answered 21602: invalid market index"
        ))
    );
}

#[test]
fn a_chart_reaches_back_ten_pages_at_most() {
    let requested =
        |text: &str| PAGING.requested(plan(DataSource::Lighter, tf(text)).unwrap(), 500);
    assert_eq!(requested("1h"), 500);
    assert_eq!(requested("3h"), 501 * 3);
    assert_eq!(requested("22m"), 5_000);
    assert_eq!(requested("1M"), 5_000);
}

const CANDLE_UPDATE: &str = r#"{"candles":[{"t":1795235040000,"o":55777.5,"h":55811.4,"l":55755.1,"c":55769,"v":5.50013,"V":306769.365469,"i":41936942341}],"channel":"candle:1:1m","timestamp":1795235055661,"type":"update/candle"}"#;

#[test]
fn parses_the_forming_candle_of_a_market_named_by_its_number() {
    let (id, kline) = parse_candle(CANDLE_UPDATE, Timeframe::MINUTE_1).expect("a candle");
    assert_eq!(id, 1);
    assert_eq!(kline.open_time, 1_795_235_040_000);
    assert_eq!(kline.close_time, 1_795_235_099_999);
    assert_eq!(
        (kline.open, kline.high, kline.low, kline.close, kline.volume),
        (55777.5, 55811.4, 55755.1, 55769.0, 5.50013)
    );
    // The answer to the subscription is the same message under another type.
    let subscribed = CANDLE_UPDATE.replace("update/candle", "subscribed/candle");
    assert_eq!(
        parse_candle(&subscribed, Timeframe::MINUTE_1).map(|(id, _)| id),
        Some(1)
    );
    // Of several candles, the newest.
    let two = r#"{"candles":[{"t":1795234980000,"o":1,"h":1,"l":1,"c":1,"v":1},{"t":1795235040000,"o":2,"h":2,"l":2,"c":2,"v":2}],"channel":"candle:10001:1m","type":"update/candle"}"#;
    let (id, kline) = parse_candle(two, Timeframe::MINUTE_1).unwrap();
    assert_eq!((id, kline.close), (10001, 2.0));
}

#[test]
fn what_is_not_a_candle_is_ignored() {
    for ignored in [
        r#"{"session_id":"6946567b-bc0e-407f-9829-2ae692cd17fb","type":"connected"}"#,
        r#"{"type":"pong"}"#,
        r#"{"error":{"code":30005,"message":"Invalid Channel:  (marketId)"}}"#,
        r#"{"channel":"market_stats:1","market_stats":{"symbol":"BTC"},"type":"update/market_stats"}"#,
        r#"{"candles":[],"channel":"candle:1:1m","type":"update/candle"}"#,
        r#"{"candles":[{"t":1,"o":"x"}],"channel":"candle:1:1m","type":"update/candle"}"#,
        r#"{"candles":[{"t":1,"o":1,"h":1,"l":1,"c":1,"v":1}],"channel":"candle:BTC:1m","type":"update/candle"}"#,
        r#"{"candles":[{"t":1,"o":1,"h":1,"l":1,"c":1,"v":1}],"channel":"candle:1:1m","type":"unsubscribed/candle"}"#,
        "not json",
    ] {
        assert!(
            parse_candle(ignored, Timeframe::MINUTE_1).is_none(),
            "{ignored}"
        );
    }
}

#[test]
fn every_market_of_a_chart_is_a_subscription_of_one_connection() {
    let markets = markets();
    let symbols = ["BTC".to_owned(), "NOPE".to_owned(), "us500".to_owned()];
    let channels = live::candle_channels(&markets, &symbols, "4h");
    assert_eq!(channels, ["candle/1/4h", "candle/10001/4h"]);
    let session = socket::session(&channels);
    assert_eq!(session.url, "wss://mainnet.zklighter.elliot.ai/stream");
    assert_eq!(
        session.subscribe,
        [
            r#"{"channel":"candle/1/4h","type":"subscribe"}"#,
            r#"{"channel":"candle/10001/4h","type":"subscribe"}"#
        ]
    );
}

#[test]
fn a_length_lighter_has_no_candle_for_has_no_session() {
    for other in ["3m", "2h", "1w", "1M"] {
        assert!(
            LIGHTER.sessions(&["BTC".into()], tf(other)).is_none(),
            "{other}"
        );
    }
}

#[test]
fn the_socket_is_kept_alive_with_a_text_ping_it_answers() {
    use crate::service::net::ws::Keepalive;
    let session = socket::session(&[]);
    assert_eq!(
        session.keepalive,
        Keepalive::QuietText {
            ping: r#"{"type":"ping"}"#,
            pong: r#"{"type":"pong"}"#
        }
    );
}

const STATS_UPDATE: &str = r#"{"channel":"market_stats:1","market_stats":{"symbol":"BTC","market_id":1,"index_price":"55857.8","mark_price":"55823.5","last_trade_price":"55814.2","current_funding_rate":"-0.0006","funding_rate":"0.0010","funding_timestamp":1795233600001,"daily_base_token_volume":29104.09417,"daily_quote_token_volume":1726265842.303962,"daily_price_low":55510,"daily_price_high":62334.9,"daily_price_change":-9.862789394558405,"premium":"-0.0552"},"timestamp":1795235085178,"type":"update/market_stats"}"#;

#[test]
fn parses_a_markets_price_with_the_price_a_day_ago_from_its_change() {
    let tick = parse_tick(STATS_UPDATE).expect("a tick");
    assert_eq!(tick.source, DataSource::Lighter);
    assert_eq!(tick.product_id, "BTC");
    assert_eq!(tick.quote.price, 55814.2);
    let day_ago = tick.quote.previous_day_price.unwrap();
    assert!(
        (day_ago - 55814.2 / (1.0 - 0.09862789394558405)).abs() < 1e-6,
        "{day_ago}"
    );
    assert_eq!(
        (tick.quote.volume_24h, tick.quote.volume_kind),
        (Some(1_726_265_842.303962), Some(VolumeKind::QuoteCurrency))
    );
    assert_eq!(tick.quote.timestamp_ms, Some(1_795_235_085_178));
    let subscribed = STATS_UPDATE.replace("update/market_stats", "subscribed/market_stats");
    assert!(parse_tick(&subscribed).is_some());
}

#[test]
fn what_is_not_a_price_is_ignored() {
    for ignored in [
        r#"{"type":"pong"}"#,
        r#"{"session_id":"x","type":"connected"}"#,
        r#"{"error":{"code":30005,"message":"Invalid Channel:  (marketId)"}}"#,
        CANDLE_UPDATE,
        // Every market's stats in one message are keyed by number and carry no symbol of their own.
        r#"{"channel":"market_stats:all","market_stats":{"1":{"symbol":"BTC","last_trade_price":"1"}},"type":"update/market_stats"}"#,
        // A market that has not traded has no price.
        r#"{"channel":"market_stats:7","market_stats":{"symbol":"NEW","last_trade_price":"0.00","daily_price_change":0},"type":"update/market_stats"}"#,
        r#"{"channel":"market_stats:1","type":"update/market_stats"}"#,
        "not json",
    ] {
        assert!(parse_tick(ignored).is_none(), "{ignored}");
    }
}

#[test]
fn every_market_a_watchlist_shows_is_a_subscription_of_one_connection() {
    let products = ["BTC".to_owned(), "NOPE".to_owned(), "ETH".to_owned()];
    let channels = quotes::stats_channels(&markets(), &products);
    assert_eq!(channels, ["market_stats/1", "market_stats/0"]);
}

const EXCHANGE_STATS: &str = r#"{"code":200,"total":6,"order_book_stats":[
    {"symbol":"BTC-UPDOWN-15M-1795241700","last_trade_price":0,"daily_trades_count":0,"daily_base_token_volume":0,"daily_quote_token_volume":0,"daily_price_change":0},
    {"symbol":"BTC","last_trade_price":55780.7,"daily_trades_count":649262,"daily_base_token_volume":29147.17301,"daily_quote_token_volume":1728668420.531737,"daily_price_change":-9.89976859739014},
    {"symbol":"US500","last_trade_price":5000,"daily_quote_token_volume":1000.5,"daily_price_change":25},
    {"symbol":"ETH/USDC","last_trade_price":1551.6,"daily_quote_token_volume":1,"daily_price_change":1},
    {"symbol":"WILD","last_trade_price":1,"daily_quote_token_volume":1,"daily_price_change":-100},
    {"symbol":"NODAY","last_trade_price":2}]}"#;

#[test]
fn every_market_is_priced_by_one_call_and_the_ones_asked_for_are_kept() {
    let requests: Vec<_> = ["btc", "US500", "WILD", "NODAY", "NOPE"]
        .into_iter()
        .map(request)
        .collect();
    let quotes = parse_stats(EXCHANGE_STATS, &requests).unwrap();
    assert_eq!(quotes.len(), 4, "{quotes:?}");
    let btc = quotes["btc"];
    assert_eq!(btc.price, 55780.7);
    assert!((btc.previous_day_price.unwrap() - 61909.6).abs() < 0.1);
    assert_eq!(
        (btc.volume_24h, btc.volume_kind),
        (Some(1_728_668_420.531737), Some(VolumeKind::QuoteCurrency))
    );
    assert_eq!(quotes["US500"].previous_day_price, Some(4000.0));
    assert_eq!(
        quotes["WILD"].previous_day_price, None,
        "-100% has no reference"
    );
    assert_eq!(quotes["NODAY"].previous_day_price, None);
    assert_eq!(quotes["NODAY"].volume_24h, None);
    assert!(!quotes.contains_key("NOPE"));
}

#[test]
fn an_answer_that_is_a_refusal_or_another_shape_is_a_failure_not_an_empty_book() {
    let requests = [request("BTC")];
    for body in [
        r#"{"code":23000,"message":"Too Many Requests!"}"#,
        r#"{"code":200}"#,
        r#"{"code":200,"order_book_stats":{}}"#,
        "[]",
        "not json",
    ] {
        assert!(
            matches!(
                parse_stats(body, &requests),
                Err(crate::service::watchlist_quotes::QuoteFailure::Failed(_))
            ),
            "{body}"
        );
    }
}

#[test]
fn a_watchlist_text_names_a_market_and_reads_it_back() {
    let parsed =
        watchlist::text::parse("LIGHTER:BTC\nlighter:1000PEPE\nLIGHTER:US500\nLIGHTER:EURUSD");
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
            "LIGHTER:BTC",
            "LIGHTER:1000PEPE",
            "LIGHTER:US500",
            "LIGHTER:EURUSD"
        ]
    );
    assert_eq!(parsed.skipped.len(), 0);
    let skipped = watchlist::text::parse("LIGHTER:BTC USD\nLIGHTER:bad!\nLIGHTER:");
    assert_eq!(skipped.skipped.len(), 3);
    assert!(watchlist::text::validate_market(DataSource::Lighter, "US500").is_ok());
    // Another venue's market of the same name is another market.
    for other in DataSource::ALL
        .into_iter()
        .filter(|other| *other != DataSource::Lighter)
    {
        assert_ne!(
            InstrumentId::new(DataSource::Lighter, "BTC").key(),
            InstrumentId::new(other, "BTC").key(),
            "{other:?}"
        );
    }
    assert!(
        InstrumentId::new(DataSource::Lighter, "BTC")
            .is_same_market(&InstrumentId::new(DataSource::Lighter, "btc"))
    );
}

/// Run with `cargo test --lib lighter::tests::live_lighter -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_lighter_serves_candles_pages_back_and_streams() {
    use crate::service::market::live::LiveFeed;
    use crate::service::watchlist_quotes::quote_stream::QuoteSocket;
    use std::time::{Duration, Instant};

    let hourly = LIGHTER.fetch_klines("BTC", Timeframe::HOUR_1, 500).unwrap();
    println!(
        "BTC 1h: {} candles, newest opens {}",
        hourly.len(),
        hourly.last().unwrap().open_time
    );
    assert_eq!(hourly.len(), 500);
    assert!(
        hourly
            .windows(2)
            .all(|pair| pair[0].open_time < pair[1].open_time)
    );
    assert!(
        hourly
            .iter()
            .all(|candle| candle.volume >= 0.0 && candle.close > 0.0)
    );

    // The page before, joined to the first at the hour its oldest candle opens.
    let oldest = hourly[0].open_time;
    let older = LIGHTER
        .fetch_klines_range("BTC", Timeframe::HOUR_1, 0, oldest, 500)
        .unwrap();
    println!(
        "BTC 1h one page back: {} candles, {} to {}",
        older.len(),
        older[0].open_time,
        older.last().unwrap().open_time
    );
    assert_eq!(older.len(), 500);
    assert_eq!(older.last().unwrap().open_time, oldest - HOUR);

    // Every native opens where this app cuts it.
    for native in DataSource::Lighter.natives() {
        let candles = LIGHTER.fetch_klines("BTC", *native, 30).unwrap();
        assert_eq!(candles.len(), 30, "{native}");
        for candle in &candles {
            assert_eq!(
                bucket_start_ms(*native, candle.open_time),
                candle.open_time,
                "{native}"
            );
        }
    }
    // Weeks fold from days and open on Mondays.
    let weekly = LIGHTER.fetch_klines("BTC", Timeframe::WEEK_1, 10).unwrap();
    println!("BTC 1w folded from 1d: {} weeks", weekly.len());
    assert!(
        weekly
            .iter()
            .all(|week| bucket_start_ms(Timeframe::WEEK_1, week.open_time) == week.open_time)
    );

    // The start of a market's history: a page that holds the oldest bars is short, the next is empty.
    let daily = LIGHTER
        .fetch_klines("BTC", Timeframe::DAY_1, 5_000)
        .unwrap();
    println!(
        "BTC 1d everything: {} candles from {}",
        daily.len(),
        daily[0].open_time
    );
    assert!(daily.len() > 500 && daily.len() < 2_000);
    let before = LIGHTER
        .fetch_klines_range("BTC", Timeframe::DAY_1, 0, daily[0].open_time, 500)
        .unwrap();
    println!("BTC 1d before its first candle: {} candles", before.len());
    assert!(before.is_empty());

    let unknown = LIGHTER.fetch_klines("NOPE", Timeframe::HOUR_1, 5);
    println!("NOPE: {unknown:?}");
    assert!(matches!(unknown, Err(ProviderError::UnknownMarket { .. })));

    let hits = LIGHTER.search_tickers("us500").unwrap();
    println!(
        "search us500: {:?}",
        hits.iter()
            .map(|hit| hit.symbol.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(hits[0].full_symbol, "US500");
    assert!(
        LIGHTER
            .search_tickers("btc")
            .unwrap()
            .iter()
            .any(|hit| hit.full_symbol == "BTC")
    );

    let requests: Vec<QuoteRequest> = ["BTC", "US500", "NOPE"].into_iter().map(request).collect();
    let outcome = LIGHTER.rest_quotes(&requests).unwrap();
    for symbol in ["BTC", "US500"] {
        let quote = outcome
            .quotes
            .get(symbol)
            .unwrap_or_else(|| panic!("no REST quote for {symbol}"));
        println!("REST quote {symbol}: {quote:?}");
        assert!(quote.price > 0.0 && quote.previous_day_price.is_some());
    }
    assert!(!outcome.quotes.contains_key("NOPE") && outcome.failed.is_empty());

    // A market Lighter does not list must not silence the others on the socket.
    let feed = LiveFeed::connect(
        DataSource::Lighter,
        vec!["NOPE".to_owned(), "BTC".to_owned(), "ETH".to_owned()],
        Timeframe::MINUTE_1,
        eframe::egui::Context::default(),
    )
    .expect("Lighter streams one-minute candles once its list is held");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _quotes = QuoteSocket::connect(
        DataSource::Lighter,
        vec!["NOPE".to_owned(), "BTC".to_owned()],
        sender,
    )
    .expect("Lighter has a ticker socket");
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
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["BTC", "ETH"]);
    let tick = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a quote tick in 30 s");
    println!("socket quote {}: {:?}", tick.product_id, tick.quote);
    assert_eq!(tick.product_id, "BTC");
    assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
}
