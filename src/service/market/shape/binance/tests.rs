//! The shape's parsers over inline samples, and its sockets over a configuration of their own.

use super::*;
use crate::model::watchlist::quote::VolumeKind;
use crate::model::{DataSource, Kline, Timeframe};
use crate::service::ProviderError;
use crate::service::market::search_rank::normalize;
use crate::service::net::http;
use crate::service::watchlist_quotes::QuoteFailure;
use crate::service::watchlist_quotes::provider::request;
use exchange_info::{matching_pairs, parse_pairs};
use klines::parse_klines;
use rest::refusal;

static CATALOG: Catalog<Pair> = Catalog::new("catalog_shape_test");

static CONFIG: Config = Config {
    source: DataSource::Binance,
    klines_url: "https://klines.test/v1/klines",
    max_per_request: 1_500,
    exchange_info_url: "https://klines.test/v1/exchangeInfo",
    serves: trading,
    catalog: &CATALOG,
    stream_host: "stream.test",
    max_streams: 2,
    max_silence: Duration::from_secs(90),
};

#[test]
fn parses_binance_rows() {
    let body = r#"[[1700000000000,"10.0","12.0","9.0","11.0","42.5",1700003599999]]"#;
    let candles = parse_klines(body).expect("valid response");
    assert_eq!(candles.len(), 1);
    assert_eq!(
        candles[0],
        Kline {
            open_time: 1_700_000_000_000,
            close_time: 1_700_003_599_999,
            open: 10.0,
            high: 12.0,
            low: 9.0,
            close: 11.0,
            volume: 42.5,
        }
    );
}

#[test]
fn rows_keep_their_first_seven_fields_and_ignore_the_rest() {
    let body = r#"[[1,"1","2","0.5","1.5","3",2,"4.5",10,"1","2","0"]]"#;
    assert_eq!(parse_klines(body).expect("row").len(), 1);
}

#[test]
fn rejects_error_objects() {
    let error =
        parse_klines(r#"{"code":-1121,"msg":"Invalid symbol."}"#).expect_err("object must fail");
    assert_eq!(error, ProviderError::data("expected a candle array"));
}

#[test]
fn only_the_invalid_symbol_code_is_an_unknown_market() {
    let refused = |status, body: &str| http::Error::Refused {
        status,
        body: body.to_owned(),
    };
    let unknown = refusal(
        DataSource::Binance,
        "btcuusdt",
        refused(400, r#"{"code":-1121,"msg":"Invalid symbol."}"#),
    );
    assert_eq!(
        unknown,
        ProviderError::unknown_market(DataSource::Binance, "BTCUUSDT", "")
    );
    assert_eq!(unknown.to_string(), "Binance does not list BTCUUSDT");
    // Any other refusal keeps what it was: a bad parameter, a limit, a ban.
    for error in [
        refused(
            400,
            r#"{"code":-1130,"msg":"Invalid data sent for a parameter."}"#,
        ),
        refused(400, "not json"),
        refused(429, r#"{"code":-1121}"#),
        http::Error::Status(400),
    ] {
        let kept = refusal(DataSource::Binance, "BTCUSDT", error.clone());
        assert_eq!(kept, ProviderError::Network(error));
    }
}

#[test]
fn a_malformed_row_is_an_error_not_a_panic() {
    for body in [
        r#"[[1700000000000,"x","12","9","11","1",1700003599999]]"#,
        r#"[[1700000000000,"10","12","9","11","1"]]"#,
        r#"[["1700000000000","10","12","9","11","1",1]]"#,
        r#"[[]]"#,
        "not json",
    ] {
        assert!(parse_klines(body).is_err(), "{body}");
    }
}

#[test]
fn a_pair_is_found_however_it_is_separated() {
    let pairs = vec![
        Pair {
            symbol: "BTCUSDT".into(),
            base: "BTC".into(),
            quote: "USDT".into(),
        },
        Pair {
            symbol: "ETHBTC".into(),
            base: "ETH".into(),
            quote: "BTC".into(),
        },
    ];
    for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
        let found = matching_pairs(DataSource::Binance, &pairs, &normalize(query));
        assert_eq!(found.len(), 1, "{query}");
        assert_eq!(found[0].symbol, "BTCUSDT");
    }
    assert_eq!(matching_pairs(DataSource::Binance, &pairs, "BTC").len(), 2);
}

#[test]
fn the_catalog_keeps_only_trading_markets_with_all_their_fields() {
    let body = r#"{"symbols":[
        {"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT"},
        {"symbol":"LUNAUSDT","status":"BREAK","baseAsset":"LUNA","quoteAsset":"USDT"},
        {"symbol":"NOQUOTE","status":"TRADING","baseAsset":"X"}
    ]}"#;
    let pairs = parse_pairs(body, trading).expect("pairs");
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        (
            pairs[0].symbol.as_str(),
            pairs[0].base.as_str(),
            pairs[0].quote.as_str()
        ),
        ("BTCUSDT", "BTC", "USDT")
    );
    assert!(parse_pairs("not json", trading).is_err());
    assert!(parse_pairs("{}", trading).expect("no symbols").is_empty());
}

#[test]
fn binance_mini_ticker_with_dollar_and_base_volume() {
    let body = r#"[
        {"symbol":"BTCUSDT","lastPrice":"67000.5","openPrice":"66000","volume":"100","quoteVolume":"6700000","closeTime":1700000000000},
        {"symbol":"ETHBTC","lastPrice":"0.05","openPrice":"0.049","volume":"2000","quoteVolume":"100","closeTime":1700000000001}
    ]"#;
    let quotes = parse_quotes(
        DataSource::Binance,
        body,
        &[request("BTCUSDT"), request("ETHBTC"), request("GONEUSDT")],
    )
    .unwrap();
    assert_eq!(quotes.len(), 2, "an unanswered symbol is omitted");
    let btc = &quotes["BTCUSDT"];
    assert_eq!(
        (btc.price, btc.previous_day_price),
        (67_000.5, Some(66_000.0))
    );
    assert_eq!(
        (btc.volume_24h, btc.volume_kind),
        (Some(6_700_000.0), Some(VolumeKind::QuoteCurrency))
    );
    assert_eq!(btc.timestamp_ms, Some(1_700_000_000_000));
    let eth = &quotes["ETHBTC"];
    assert_eq!(
        (eth.volume_24h, eth.volume_kind),
        (Some(2_000.0), Some(VolumeKind::Base)),
        "no dollar figure on a non-dollar pair"
    );
}

#[test]
fn quotes_reject_non_arrays_and_zero_prices() {
    let failure = parse_quotes(DataSource::Binance, r#"{"code":-1121}"#, &[request("X")]);
    assert_eq!(
        failure,
        Err(QuoteFailure::Failed("Unexpected Binance response".into()))
    );
    let zero = r#"[{"symbol":"AUSDT","lastPrice":"0","openPrice":"1"}]"#;
    assert!(
        parse_quotes(DataSource::Binance, zero, &[request("AUSDT")])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_symbols_and_interval_are_in_the_url_and_a_connection_takes_max_streams_of_them() {
    let symbols: Vec<String> = ["BTCUSDT", "ETHUSDT", "SOLUSDT"].map(String::from).into();
    let sessions = CONFIG.kline_sessions(&symbols, Timeframe::HOUR_1);
    let urls: Vec<_> = sessions
        .iter()
        .map(|session| session.url.as_str())
        .collect();
    assert_eq!(
        urls,
        [
            "wss://stream.test/stream?streams=btcusdt@kline_1h/ethusdt@kline_1h",
            "wss://stream.test/stream?streams=solusdt@kline_1h",
        ]
    );
    for session in &sessions {
        assert!(session.subscribe.is_empty());
        assert_eq!(session.host, "stream.test");
        assert_eq!(session.max_data_silence, Duration::from_secs(90));
    }
}

#[test]
fn parses_kline_stream_message() {
    let text = r#"{"stream":"btcusdt@kline_1h","data":{"k":{"t":1700000000000,"T":1700003599999,"o":"10","h":"12","l":"9","c":"11","v":"42.5","q":"0","x":false}}}"#;
    let (symbol, kline) = parse_kline(text).expect("parse");
    assert_eq!(symbol, "BTCUSDT");
    assert_eq!(kline.open_time, 1_700_000_000_000);
    assert_eq!(kline.close_time, 1_700_003_599_999);
    assert_eq!(kline.high, 12.0);
    assert_eq!(kline.low, 9.0);
    assert_eq!(kline.close, 11.0);
    assert_eq!(kline.volume, 42.5);
}

#[test]
fn a_message_without_a_close_time_falls_back_to_the_open_time() {
    let text = r#"{"stream":"btcusdt@kline_1h","data":{"k":{"t":1700000000000,"o":"10","h":"12","l":"9","c":"11","v":"1"}}}"#;
    let (_, kline) = parse_kline(text).expect("parse");
    assert_eq!(kline.close_time, 1_700_000_000_000);
}

#[test]
fn acknowledgements_other_topics_and_broken_messages_are_not_klines() {
    for ignored in [
        r#"{"id":1,"result":null}"#,
        r#"{"stream":"btcusdt@ticker","data":{"s":"BTCUSDT","c":"1","o":"1"}}"#,
        r#"{"stream":"btcusdt@depth","data":{}}"#,
        r#"{"stream":"btcusdt@kline_1m"}"#,
        r#"{"stream":"btcusdt@kline_1m","data":{"k":{"t":1,"o":"x"}}}"#,
        "not json",
    ] {
        assert!(parse_kline(ignored).is_none(), "{ignored}");
    }
}
