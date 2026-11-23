use super::*;
use crate::model::InstrumentId;
use crate::model::market::MarketKind;
use crate::model::market::bucket::bucket_start_ms;
use crate::model::watchlist::quote::VolumeKind;
use crate::service::market::candles::plan;
use crate::service::market::search_rank::normalize;
use crate::service::market::shape::binance::{
    matching_pairs, parse_klines, parse_pairs, parse_quotes,
};
use crate::service::net::ws::Frame;
use crate::service::watchlist_quotes::provider::request;
use crate::service::watchlist_quotes::quote_stream::StreamTick;
use serde_json::{Value, json};

const DAY: i64 = 24 * 3_600_000;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

fn base_of(timeframe: &str) -> (String, usize) {
    let plan = plan(DataSource::Aster, tf(timeframe)).unwrap();
    (plan.base().interval(), plan.base_count(500))
}

/// A trimmed `exchangeInfo` as Aster answers it: a coin, a stock and a commodity perpetual, a
/// `USD1` and a `USDC` market, one that is settling, one awaiting its listing.
const EXCHANGE_INFO: &str = r#"{"timezone":"UTC","rateLimits":[],"symbols":[
    {"symbol":"BTCUSDT","status":"TRADING","contractType":"PERPETUAL","baseAsset":"BTC","quoteAsset":"USDT","pair":"BTCUSDT","filters":[]},
    {"symbol":"QQQUSDT","status":"TRADING","contractType":"PERPETUAL","baseAsset":"QQQ","quoteAsset":"USDT","pair":"QQQUSDT"},
    {"symbol":"XAUUSDT","status":"TRADING","contractType":"PERPETUAL","baseAsset":"XAU","quoteAsset":"USDT","pair":"XAUUSDT"},
    {"symbol":"SPXUSDT","status":"TRADING","contractType":"PERPETUAL","baseAsset":"SPX","quoteAsset":"USDT","pair":"SPXUSDT"},
    {"symbol":"BTCUSD1","status":"TRADING","contractType":"PERPETUAL","baseAsset":"BTC","quoteAsset":"USD1","pair":"BTCUSD1"},
    {"symbol":"ETHUSDC","status":"TRADING","contractType":"PERPETUAL","baseAsset":"ETH","quoteAsset":"USDC","pair":"ETHUSDC"},
    {"symbol":"OLDUSDT","status":"SETTLING","contractType":"PERPETUAL","baseAsset":"OLD","quoteAsset":"USDT","pair":"OLDUSDT"},
    {"symbol":"MBLUSDT","status":"PENDING_TRADING","contractType":"","baseAsset":"MBL","quoteAsset":"USDT","pair":"MBLUSDT"}
]}"#;

#[test]
fn only_trading_perpetuals_are_listed_with_their_base_and_quote() {
    let pairs = parse_pairs(EXCHANGE_INFO, SHAPE.serves).expect("pairs");
    let symbols: Vec<_> = pairs.iter().map(|pair| pair.symbol.as_str()).collect();
    assert_eq!(
        symbols,
        [
            "BTCUSDT", "QQQUSDT", "XAUUSDT", "SPXUSDT", "BTCUSD1", "ETHUSDC"
        ]
    );
    let usd1 = pairs.iter().find(|pair| pair.symbol == "BTCUSD1").unwrap();
    assert_eq!((usd1.base.as_str(), usd1.quote.as_str()), ("BTC", "USD1"));
}

#[test]
fn every_hit_is_a_perpetual_found_however_it_is_written() {
    let pairs = parse_pairs(EXCHANGE_INFO, SHAPE.serves).unwrap();
    for query in ["btcusdt", "BTC-USDT", "btc/usdt"] {
        let hits = matching_pairs(DataSource::Aster, &pairs, &normalize(query));
        assert_eq!(hits.len(), 1, "{query}");
        assert_eq!(hits[0].full_symbol, "BTCUSDT");
        assert_eq!(hits[0].kind, MarketKind::Perp);
    }
    // The S&P 500 is not aliased onto `SPX`: on Aster that is a memecoin.
    let spx = matching_pairs(DataSource::Aster, &pairs, "SPX");
    assert_eq!(spx.len(), 1);
    assert_eq!(
        (spx[0].symbol.as_str(), spx[0].base.as_deref()),
        ("SPXUSDT", Some("SPX"))
    );
}

#[test]
fn parses_the_rows_aster_answers() {
    let body = r#"[[1795226400000,"59003.9","59059.0","58550.0","58582.6","192.304",1795229999999,"11303459.5154",3023,"88.172","5181478.9652","0"],[1795230000000,"58582.5","58582.5","56526.9","56827.6","1924.508",1795233599999,"110363873.3068",9422,"703.959","40394762.2947","0"]]"#;
    let candles = parse_klines(body).expect("rows");
    assert_eq!(candles.len(), 2);
    assert_eq!(candles[1].open_time, 1_795_230_000_000);
    assert_eq!(candles[1].close_time, 1_795_233_599_999);
    assert_eq!(
        (
            candles[1].open,
            candles[1].high,
            candles[1].low,
            candles[1].close,
            candles[1].volume
        ),
        (58_582.5, 58_582.5, 56_526.9, 56_827.6, 1_924.508)
    );
}

#[test]
fn an_error_body_is_a_data_error_not_candles() {
    let body = r#"{"code":-1130,"msg":"Data sent for parameter 'limit' is not valid."}"#;
    assert!(matches!(parse_klines(body), Err(ProviderError::Data(_))));
}

#[test]
fn a_native_length_is_asked_for_as_it_is() {
    for native in DataSource::Aster.natives() {
        let token = native.interval();
        assert_eq!(base_of(&token), (token.clone(), 500), "{token}");
    }
    assert_eq!(DataSource::Aster.natives().len(), 14);
}

#[test]
fn a_3d_bar_is_folded_because_aster_counts_its_own_from_another_day() {
    // Aster's `3d` opens on a day number that is 1 modulo 3 since 1970-01-01 (BTC: 2021-09-01).
    assert_eq!(1_630_454_400_000 / DAY % 3, 1);
    assert_ne!(
        bucket_start_ms(tf("3d"), 1_630_454_400_000),
        1_630_454_400_000
    );
    assert!(!DataSource::Aster.natives().contains(&tf("3d")));
    assert_eq!(base_of("3d"), ("1d".to_owned(), 501 * 3));
    assert_eq!(base_of("2w"), ("1w".to_owned(), 501 * 2));
    assert_eq!(base_of("3M"), ("1M".to_owned(), 501 * 3));
    assert_eq!(base_of("1Y"), ("1M".to_owned(), 501 * 12));
    assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
    assert_eq!(base_of("10h"), ("2h".to_owned(), 501 * 5));
}

#[test]
fn the_weeks_and_months_aster_cuts_are_the_ones_this_app_cuts() {
    // 2026-11-16 was a Monday, 2026-11-01 the first of the month: Aster's own 1w and 1M opens.
    for (native, opens) in [
        (Timeframe::WEEK_1, 1_794_787_200_000),
        (Timeframe::MONTH_1, 1_793_491_200_000),
    ] {
        assert_eq!(bucket_start_ms(native, opens), opens, "{native}");
    }
}

#[test]
fn a_series_of_more_than_1500_candles_is_paged_but_its_tail_is_not() {
    assert!(base_of("5m").1 <= SHAPE.max_per_request);
    assert!(base_of("22m").1 > SHAPE.max_per_request);
    let tail = plan(DataSource::Aster, tf("22m"))
        .unwrap()
        .base_count(crate::service::cache::TAIL_LIMIT as usize);
    assert!(tail <= SHAPE.max_per_request);
}

#[test]
fn the_rest_ticker_prices_a_stock_perpetual_and_a_usd1_market() {
    let body = r#"[
        {"symbol":"QQQUSDT","priceChange":"-30.62","priceChangePercent":"-4.752","lastPrice":"613.800000","openPrice":"644.420000","volume":"2368.09","quoteVolume":"1490061.870000","openTime":1795146540000,"closeTime":1795232973000},
        {"symbol":"BTCUSD1","lastPrice":"56972.20","openPrice":"62256.60","volume":"155.670","quoteVolume":"9255433.73","closeTime":1795232999350}
    ]"#;
    let quotes = parse_quotes(
        DataSource::Aster,
        body,
        &[request("QQQUSDT"), request("BTCUSD1"), request("GONEUSDT")],
    )
    .unwrap();
    assert_eq!(quotes.len(), 2);
    let qqq = &quotes["QQQUSDT"];
    assert_eq!((qqq.price, qqq.previous_day_price), (613.8, Some(644.42)));
    assert_eq!(
        (qqq.volume_24h, qqq.volume_kind),
        (Some(1_490_061.87), Some(VolumeKind::QuoteCurrency))
    );
    assert_eq!(qqq.timestamp_ms, Some(1_795_232_973_000));
    let usd1 = &quotes["BTCUSD1"];
    assert_eq!(
        (usd1.volume_24h, usd1.volume_kind),
        (Some(155.67), Some(VolumeKind::Base)),
        "units, not a dollar figure: USD1 is not one of the shared dollar quotes"
    );
}

#[test]
fn a_candle_is_asked_over_the_perpetual_hosts() {
    let sessions = Aster
        .sessions(
            &["BTCUSDT".to_owned(), "QQQUSDT".to_owned()],
            Timeframe::MINUTE_1,
        )
        .expect("Aster streams candles");
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].url,
        "wss://fstream.asterdex.com/stream?streams=btcusdt@kline_1m/qqqusdt@kline_1m"
    );
    assert_eq!(sessions[0].max_data_silence, Duration::from_secs(600));
}

#[test]
fn a_connection_takes_200_topics_so_more_markets_get_more_connections() {
    let symbols: Vec<String> = (0..450).map(|index| format!("M{index}USDT")).collect();
    let sessions = Aster.sessions(&symbols, Timeframe::HOUR_1).unwrap();
    assert_eq!(sessions.len(), 3);
    let topics = |session: &crate::service::net::ws::Session| {
        session
            .url
            .split_once("streams=")
            .unwrap()
            .1
            .split('/')
            .count()
    };
    assert_eq!(
        sessions.iter().map(topics).collect::<Vec<_>>(),
        [200, 200, 50]
    );
}

#[test]
fn watchlist_prices_are_one_subscribe_message_for_the_first_200_markets() {
    let products: Vec<String> = (0..250).map(|index| format!("M{index}USDT")).collect();
    let session = QuoteProtocol::session(&Aster, &products);
    assert_eq!(session.url, "wss://fstream.asterdex.com/stream");
    assert_eq!(session.subscribe.len(), 1);
    let message: Value = serde_json::from_str(&session.subscribe[0]).unwrap();
    assert_eq!(message["method"], "SUBSCRIBE");
    let topics = message["params"].as_array().unwrap();
    assert_eq!(topics.len(), 200);
    assert_eq!(topics[0], json!("m0usdt@ticker"));
    assert_eq!(topics[199], json!("m199usdt@ticker"));
}

/// The candle a socket message of the candle protocol says.
fn candle(text: &str) -> Option<(String, Kline)> {
    CandleProtocol::parse(&Aster, Frame::Text(text), Timeframe::MINUTE_1)
}

/// The tick a socket message of the quote protocol says.
fn tick(text: &str) -> Option<StreamTick> {
    QuoteProtocol::parse(&Aster, text)
}

#[test]
fn parses_the_candle_and_the_ticker_aster_pushes() {
    let kline = r#"{"stream":"btcusdt@kline_1m","data":{"e":"kline","E":1795232871036,"s":"BTCUSDT","k":{"t":1795232820000,"T":1795232879999,"s":"BTCUSDT","i":"1m","f":150361114,"L":150361207,"o":"56921.5","c":"56904.4","h":"56944.0","l":"56887.6","v":"29.238","n":94,"x":false,"q":"1664115.3353","V":"6.228","Q":"354515.7299","B":"0"}}}"#;
    let (symbol, candle) = candle(kline).expect("candle");
    assert_eq!(symbol, "BTCUSDT");
    assert_eq!(
        (candle.open_time, candle.close_time),
        (1_795_232_820_000, 1_795_232_879_999)
    );
    assert_eq!(
        (
            candle.open,
            candle.high,
            candle.low,
            candle.close,
            candle.volume
        ),
        (56_921.5, 56_944.0, 56_887.6, 56_904.4, 29.238)
    );

    let ticker = r#"{"stream":"btcusdt@ticker","data":{"e":"24hrTicker","E":1795232871035,"s":"BTCUSDT","p":"-5202.1","P":"-8.376","w":"59457.6","c":"56904.4","Q":"0.001","o":"62106.5","h":"62309.5","l":"56526.9","v":"15850.680","q":"942444142.15","O":1795146420000,"C":1795232871000,"F":150266448,"L":150361207,"n":94695}}"#;
    let tick = tick(ticker).expect("tick");
    assert_eq!(tick.source, DataSource::Aster);
    assert_eq!(tick.product_id, "BTCUSDT");
    assert_eq!(tick.quote.price, 56_904.4);
    assert_eq!(tick.quote.previous_day_price, Some(62_106.5));
    assert_eq!(
        (tick.quote.volume_24h, tick.quote.volume_kind),
        (Some(942_444_142.15), Some(VolumeKind::QuoteCurrency))
    );
}

#[test]
fn acknowledgements_and_the_other_kind_of_topic_are_ignored() {
    let kline = r#"{"stream":"btcusdt@kline_1m","data":{"k":{"t":1,"T":2,"o":"1","h":"1","l":"1","c":"1","v":"1"}}}"#;
    let ticker = r#"{"stream":"btcusdt@ticker","data":{"s":"BTCUSDT","c":"1","o":"1","v":"1","q":"1","C":1}}"#;
    assert!(tick(kline).is_none());
    assert!(candle(ticker).is_none());
    for ignored in [
        r#"{"id":1,"result":null}"#,
        r#"{"stream":"btcusdt@ticker","data":{"s":"BTCUSDT","c":"0","o":"1"}}"#,
        "illegal request",
    ] {
        assert!(tick(ignored).is_none(), "{ignored}");
        assert!(candle(ignored).is_none(), "{ignored}");
    }
    let binary = Frame::Binary(b"\x00");
    assert!(CandleProtocol::parse(&Aster, binary, Timeframe::MINUTE_1).is_none());
}

/// Every market is a perpetual, whatever its id.
#[test]
fn every_market_is_a_perpetual() {
    for id in ["BTCUSDT", "QQQUSDT", "BTCUSD1"] {
        assert_eq!(
            Aster.kind_of(&InstrumentId::new(DataSource::Aster, id)),
            MarketKind::Perp,
            "{id}"
        );
    }
}

/// Run with `cargo test --lib aster::tests::live_aster -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_aster_serves_candles_pages_back_and_streams() {
    use crate::service::market::live::LiveFeed;
    use crate::service::net::http;
    use crate::service::watchlist_quotes::quote_stream::QuoteSocket;
    use std::time::Instant;

    let hourly = ASTER
        .fetch_klines("BTCUSDT", Timeframe::HOUR_1, 500)
        .unwrap();
    println!(
        "BTCUSDT 1h: {} candles, newest opens {}",
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

    let oldest = hourly[0].open_time;
    let older = ASTER
        .fetch_klines_range("BTCUSDT", Timeframe::HOUR_1, 0, oldest, 500)
        .unwrap();
    println!(
        "BTCUSDT 1h one page back: {} candles ending {}",
        older.len(),
        older.last().unwrap().open_time
    );
    assert_eq!(older.len(), 500);
    assert!(older.last().unwrap().open_time < oldest);

    // More than one request holds is paged: 3,000 one-minute candles are two pages.
    let minutes = ASTER
        .fetch_klines("BTCUSDT", Timeframe::MINUTE_1, 3_000)
        .unwrap();
    println!("BTCUSDT 1m x 3000: {} candles", minutes.len());
    // Aster stores a bar some seconds after its minute opens, so the newest may still be missing.
    assert!((2_999..=3_000).contains(&minutes.len()));

    // Every native opens on this app's grid.
    for native in DataSource::Aster.natives() {
        let candles = ASTER.fetch_klines("BTCUSDT", *native, 30).unwrap();
        assert!(!candles.is_empty(), "{native}");
        for candle in &candles {
            assert_eq!(
                bucket_start_ms(*native, candle.open_time),
                candle.open_time,
                "{native}"
            );
        }
    }
    // Its own 3d does not, so days are folded: the folded 3d bars are on the grid.
    let url = format!("{}?symbol=BTCUSDT&interval=3d&limit=20", SHAPE.klines_url);
    let own: Vec<i64> = parse_klines(&http::get_text(&url, &[], http::Retry::Never).unwrap())
        .unwrap()
        .iter()
        .map(|bar| bar.open_time)
        .collect();
    println!(
        "own 3d opens, days since 1970 modulo 3: {:?}",
        own.iter().map(|open| open / DAY % 3).collect::<Vec<_>>()
    );
    assert!(own.iter().all(|open| open / DAY % 3 == 1));
    let folded = ASTER.fetch_klines("BTCUSDT", tf("3d"), 10).unwrap();
    assert!(
        folded
            .iter()
            .all(|bar| bucket_start_ms(tf("3d"), bar.open_time) == bar.open_time)
    );
    // BTC trades since 2021-09-01.
    let first = ASTER
        .fetch_klines("BTCUSDT", Timeframe::DAY_1, 3_000)
        .unwrap();
    println!(
        "BTCUSDT 1d: {} candles from {}",
        first.len(),
        first[0].open_time
    );
    assert!(first[0].open_time < 1_640_995_200_000);

    let stock = ASTER
        .fetch_klines("QQQUSDT", Timeframe::HOUR_1, 200)
        .unwrap();
    println!(
        "QQQUSDT 1h: {} candles, last close {}",
        stock.len(),
        stock.last().unwrap().close
    );
    assert!(stock.len() >= 100);
    let unknown = ASTER.fetch_klines("NOPEUSDT", Timeframe::HOUR_1, 5);
    println!("NOPEUSDT: {unknown:?}");
    assert!(unknown.is_err());

    let hits = ASTER.search_tickers("BTC-USDT").unwrap();
    println!(
        "search BTC-USDT: {:?}",
        hits.iter()
            .map(|hit| hit.symbol.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(hits[0].full_symbol, "BTCUSDT");
    assert!(hits.iter().all(|hit| hit.kind == MarketKind::Perp));
    let stocks = ASTER.search_tickers("qqq").unwrap();
    assert!(stocks.iter().any(|hit| hit.full_symbol == "QQQUSDT"));

    let requests: Vec<QuoteRequest> = ["BTCUSDT", "QQQUSDT", "BTCUSD1", "NOPEUSDT"]
        .iter()
        .map(|symbol| request(symbol))
        .collect();
    let outcome = ASTER.rest_quotes(&requests).unwrap();
    for symbol in ["BTCUSDT", "QQQUSDT", "BTCUSD1"] {
        let quote = outcome
            .quotes
            .get(symbol)
            .unwrap_or_else(|| panic!("no REST quote for {symbol}"));
        println!("REST quote {symbol}: {quote:?}");
        assert!(quote.price > 0.0 && quote.previous_day_price.is_some());
    }
    assert!(!outcome.quotes.contains_key("NOPEUSDT") && outcome.failed.is_empty());

    // A market Aster does not list is dropped without closing the socket on its neighbours.
    let feed = LiveFeed::connect(
        DataSource::Aster,
        vec![
            "NOPEUSDT".to_owned(),
            "BTCUSDT".to_owned(),
            "ETHUSDT".to_owned(),
        ],
        Timeframe::MINUTE_1,
        eframe::egui::Context::default(),
    )
    .expect("Aster streams one-minute candles");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _quotes = QuoteSocket::connect(
        DataSource::Aster,
        vec!["NOPEUSDT".to_owned(), "BTCUSDT".to_owned()],
        sender,
    )
    .expect("Aster has a ticker socket");
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
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["BTCUSDT", "ETHUSDT"]);
    let tick = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a quote tick in 30 s");
    println!("socket quote {}: {:?}", tick.product_id, tick.quote);
    assert_eq!(tick.product_id, "BTCUSDT");
    assert!(tick.quote.price > 0.0 && tick.quote.previous_day_price.is_some());
}
