use super::markets::{Pair, parse_pairs};
use super::parse::{history_reply, parse_klines};
use super::socket::{history_request, kline_topic, subscribe, ticker_topic};
use super::ticker::parse_tickers;
use super::*;
use crate::model::market::bucket::bucket_start_ms;
use crate::model::watchlist::quote::VolumeKind;
use crate::service::net::ws::Decode;
use crate::service::watchlist_quotes::provider::request;
use serde_json::{Value, json};

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

fn tf(text: &str) -> Timeframe {
    Timeframe::parse(text).unwrap()
}

#[test]
fn every_native_has_its_period_and_nothing_else_does() {
    let periods: Vec<&str> = DataSource::Htx
        .natives()
        .iter()
        .map(|native| period(*native).expect("a native has a period"))
        .collect();
    assert_eq!(
        periods,
        ["1min", "5min", "15min", "30min", "60min", "4hour"]
    );
    for other in ["2m", "3m", "2h", "8h", "1D", "1W", "1M", "3M", "1Y", "22m"] {
        assert_eq!(period(tf(other)), None, "{other}");
    }
}

#[test]
fn everything_from_four_hours_up_folds_from_the_four_hour_bars() {
    let base = |timeframe: &str| {
        let plan = plan(DataSource::Htx, tf(timeframe)).unwrap();
        (period(plan.base()).unwrap(), plan.base_count(500))
    };
    assert_eq!(base("1h"), ("60min", 500));
    assert_eq!(base("22m"), ("1min", 501 * 22));
    assert_eq!(base("8h"), ("4hour", 501 * 2));
    assert_eq!(base("1D"), ("4hour", 501 * 6));
    assert_eq!(base("1W"), ("4hour", 501 * 42));
    assert_eq!(base("1M"), ("4hour", 501 * 186));
}

/// HTX's own days, weeks, months and years open at 16:00 UTC on the day before this app's: the
/// reason they are not natives. Opens as the API answered them on 2026-11-21.
#[test]
fn its_own_days_weeks_and_months_do_not_open_where_this_app_cuts() {
    for (timeframe, open_s) in [
        ("1D", 1_795_104_000_i64), // 2026-11-19 16:00 UTC, a Thursday
        ("1W", 1_794_758_400),     // 2026-11-15 16:00 UTC, a Sunday
        ("1M", 1_793_462_400),     // 2026-10-31 16:00 UTC
        ("1Y", 1_767_196_800),     // 2025-12-31 16:00 UTC
    ] {
        let open = open_s * 1_000;
        assert_eq!(open % DAY, 16 * HOUR, "{timeframe}");
        assert_ne!(bucket_start_ms(tf(timeframe), open), open, "{timeframe}");
        assert!(!DataSource::Htx.natives().contains(&tf(timeframe)));
    }
    // Its own four-hour bars (and every shorter one) do.
    for native in DataSource::Htx.natives() {
        let open = 1_795_233_600_000_i64;
        assert_eq!(bucket_start_ms(*native, open), open, "{native}");
    }
}

#[test]
fn symbols_are_htxs_however_they_are_spelled() {
    assert_eq!(symbol_id("BTCUSDT").as_deref(), Some("btcusdt"));
    assert_eq!(symbol_id("btc/usdt").as_deref(), Some("btcusdt"));
    assert_eq!(symbol_id("ETH-BTC").as_deref(), Some("ethbtc"));
    assert_eq!(symbol_id("sol").as_deref(), Some("solusdt"));
    assert_eq!(symbol_id("WBTC").as_deref(), Some("wbtcusdt"));
    // Text that could change a URL is not a market.
    for text in ["BTCUSDT&size=1", "btc?usdt", "a b%20", "é", ""] {
        assert_eq!(symbol_id(text), None, "{text:?}");
    }
}

#[test]
fn messages_name_the_topic_in_htxs_own_words() {
    assert_eq!(
        kline_topic("btcusdt", "4hour"),
        "market.btcusdt.kline.4hour"
    );
    assert_eq!(ticker_topic("btcusdt"), "market.btcusdt.ticker");
    assert_eq!(
        serde_json::from_str::<Value>(&subscribe("market.btcusdt.ticker")).unwrap(),
        json!({"sub": "market.btcusdt.ticker", "id": "degenview"})
    );
    assert_eq!(
        serde_json::from_str::<Value>(&history_request("btcusdt", "4hour", 100, 900)).unwrap(),
        json!({"req": "market.btcusdt.kline.4hour", "id": "history", "from": 100, "to": 900})
    );
}

/// What `/market/history/kline?symbol=btcusdt&period=4hour&size=3` answered on 2026-11-21: the
/// newest candle first, `amount` the base asset and `vol` the quote asset.
const REST_ROWS: &str = r#"{"ch":"market.btcusdt.kline.4hour","status":"ok","ts":1795239169181,"data":[
    {"id":1795233600,"open":56748.09,"close":56017.2,"low":55555.55,"high":56835.44,"amount":891.9829671600257,"vol":5.000276051887151E7,"count":14089},
    {"id":1795219200,"open":58949.63,"close":56743.0,"low":56394.13,"high":59249.7,"amount":1291.8946267578406,"vol":7.525178261784765E7,"count":81992},
    {"id":1795204800,"open":58562.0,"close":58966.05,"low":58211.0,"high":59401.08,"amount":800.7214110817212,"vol":4.7017664601738155E7,"count":8751}
]}"#;

#[test]
fn rows_come_newest_first_and_leave_oldest_first_with_the_close_of_their_bucket() {
    let candles = parse_klines(REST_ROWS, Timeframe::HOUR_4).expect("valid response");
    assert_eq!(candles.len(), 3);
    assert_eq!(
        candles[2],
        Kline {
            open_time: 1_795_233_600_000,
            close_time: 1_795_233_600_000 + 4 * HOUR - 1,
            open: 56_748.09,
            high: 56_835.44,
            low: 55_555.55,
            close: 56_017.2,
            // The base asset, not the quote turnover.
            volume: 891.9829671600257,
        }
    );
    assert!(
        candles
            .windows(2)
            .all(|pair| pair[0].open_time < pair[1].open_time)
    );
}

#[test]
fn a_refusal_is_htxs_message_and_a_429_is_a_rate_limit() {
    let unknown = r#"{"ts":1795239554030,"status":"error","err-code":"invalid-parameter","err-msg":"invalid symbol"}"#;
    assert_eq!(
        parse_klines(unknown, Timeframe::HOUR_1),
        Err(ProviderError::other("HTX: invalid symbol"))
    );
    let limited = r#"{"status":"error","ts":1795239219401,"id":"history","err-code":"bad-request","err-msg":"429 too many request topic is market.btcusdt.kline.1min"}"#;
    let error = parse_klines(limited, Timeframe::HOUR_1).unwrap_err();
    assert!(error.is_rate_limited(), "{error}");
    // A message that merely mentions a number is no rate limit.
    let other = r#"{"status":"error","err-code":"bad-request","err-msg":"invalid size,valid range: [1, 2000]"}"#;
    assert!(
        !parse_klines(other, Timeframe::HOUR_1)
            .unwrap_err()
            .is_rate_limited()
    );
    let nameless = r#"{"status":"error"}"#;
    assert_eq!(
        parse_klines(nameless, Timeframe::HOUR_1),
        Err(ProviderError::other("HTX: The request failed"))
    );
    assert!(parse_klines("not json", Timeframe::HOUR_1).is_err());
    assert!(parse_klines(r#"{"data":[]}"#, Timeframe::HOUR_1).is_err());
    assert!(parse_klines(r#"{"status":"ok"}"#, Timeframe::HOUR_1).is_err());
    let bad_field = r#"{"status":"ok","data":[{"id":1795233600,"open":"x","close":1,"low":1,"high":1,"amount":1}]}"#;
    assert!(parse_klines(bad_field, Timeframe::HOUR_1).is_err());
}

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

#[test]
fn a_gzip_history_answer_is_read_by_its_id_oldest_first() {
    let frame = on_the_wire(
        r#"{"id":"history","status":"ok","ts":1795239220084,"rep":"market.btcusdt.kline.4hour","data":[
            {"id":1795219200,"open":58949.63,"close":56743.0,"low":56394.13,"high":59249.7,"amount":1291.89,"vol":7.5E7,"count":81992},
            {"id":1795233600,"open":56748.09,"close":56017.2,"low":55555.55,"high":56835.44,"amount":891.98,"vol":5.0E7,"count":14089}
        ]}"#,
    );
    let candles = history_reply(&frame, Timeframe::HOUR_4)
        .expect("the answer to the history request")
        .expect("candles");
    assert_eq!(
        candles.iter().map(|bar| bar.open_time).collect::<Vec<_>>(),
        [1_795_219_200_000, 1_795_233_600_000]
    );
    // An empty window is a market that began later, not an error.
    let empty = r#"{"id":"history","status":"ok","rep":"market.btcusdt.kline.4hour","data":[]}"#;
    assert_eq!(
        history_reply(empty, Timeframe::HOUR_4),
        Some(Ok(Vec::new()))
    );
}

#[test]
fn history_refusals_are_told_apart_and_other_messages_are_not_answers() {
    let refused = |text: &str| history_reply(text, Timeframe::HOUR_4).expect("an answer");
    let window = r#"{"status":"error","ts":1,"id":"history","err-code":"bad-request","err-msg":"req topic market.btcusdt.kline.4hour with invalid from to, from: 1 to: 2"}"#;
    assert!(matches!(refused(window), Err(ProviderError::Other(_))));
    let limited = r#"{"status":"error","ts":1,"id":"history","err-code":"bad-request","err-msg":"429 too many request topic is market.btcusdt.kline.4hour"}"#;
    assert!(refused(limited).unwrap_err().is_rate_limited());
    let unknown = r#"{"status":"error","ts":1,"id":"history","err-code":"bad-request","err-msg":"invalid symbol nopexyz"}"#;
    assert_eq!(
        refused(unknown),
        Err(ProviderError::other("HTX: invalid symbol nopexyz"))
    );

    for ignored in [
        r#"{"ping":1795239220084}"#,
        r#"{"id":"degenview","status":"ok","subbed":"market.btcusdt.kline.1min","ts":1}"#,
        r#"{"status":"error","ts":1,"id":"degenview","err-code":"bad-request","err-msg":"invalid symbol nopexyz"}"#,
        r#"{"ch":"market.btcusdt.kline.1min","ts":1,"tick":{"id":1795239300,"open":1,"close":1,"low":1,"high":1,"amount":1}}"#,
        r#"{"status":"ok","data":[]}"#,
        "not json",
    ] {
        assert!(
            history_reply(ignored, Timeframe::HOUR_4).is_none(),
            "{ignored}"
        );
    }
}

#[test]
fn the_servers_ping_is_echoed_and_never_data() {
    let session = socket::session(Vec::new());
    assert_eq!(session.decode, Decode::Gzip);
    let ping = on_the_wire(r#"{"ping":1795239308084}"#);
    // The session answers the ping itself: `{"pong":…}` with the same value.
    assert_eq!(
        format!("{:?}", session.keepalive.inspect(&ping)),
        r#"Answer("{\"pong\":1795239308084}")"#
    );
    // A data message that merely holds the key is data.
    let data = r#"{"ch":"market.btcusdt.kline.1min","ping":1}"#;
    assert_eq!(format!("{:?}", session.keepalive.inspect(data)), "Data");
}

const SYMBOLS: &str = r#"{"status":"ok","ts":1795239000000,"full":1,"data":[
    {"sc":"btcusdt","bc":"btc","qc":"usdt","bcdn":"BTC","qcdn":"USDT","state":"online","tags":"activities"},
    {"sc":"ethbtc","bc":"eth","qc":"btc","bcdn":"ETH","qcdn":"BTC","state":"online","tags":""},
    {"sc":"cnneth","bc":"cnn","qc":"eth","bcdn":"CNN","qcdn":"ETH","state":"offline","tags":"hadax"},
    {"sc":"oldusdt","bc":"old","qc":"usdt","state":"suspend"},
    {"sc":"susdt","bc":"s","qc":"usdt","state":"online"},
    {"sc":"","bc":"x","qc":"usdt","state":"online"},
    {"bc":"x","qc":"usdt","state":"online"}
]}"#;

#[test]
fn the_catalog_keeps_only_online_markets_in_upper_case() {
    let pairs = parse_pairs(SYMBOLS).expect("pairs");
    assert_eq!(
        pairs,
        [
            Pair {
                id: "BTCUSDT".into(),
                base: "BTC".into(),
                quote: "USDT".into()
            },
            Pair {
                id: "ETHBTC".into(),
                base: "ETH".into(),
                quote: "BTC".into()
            },
            Pair {
                id: "SUSDT".into(),
                base: "S".into(),
                quote: "USDT".into()
            },
        ]
    );
    assert!(parse_pairs("not json").is_err());
    assert!(parse_pairs(r#"{"status":"ok"}"#).is_err());
    assert_eq!(
        parse_pairs(r#"{"status":"error","err-msg":"down"}"#),
        Err(ProviderError::other("HTX: down"))
    );
}

#[test]
fn a_pair_is_found_however_it_is_separated() {
    let pairs = parse_pairs(SYMBOLS).unwrap();
    for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
        let found = matching_pairs(&pairs, &normalize(query));
        assert_eq!(found.len(), 1, "{query}");
        assert_eq!(found[0].symbol, "BTCUSDT");
        assert_eq!(found[0].quote.as_deref(), Some("USDT"));
    }
    assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
}

const TICKERS: &str = r#"{"status":"ok","ts":1795239300000,"data":[
    {"symbol":"btcusdt","open":59481.46,"high":59830.77,"low":55555.55,"close":55946.5,"amount":7265.5,"vol":4.3E8,"count":158019},
    {"symbol":"ethbtc","open":0.0279,"close":0.028,"amount":1200.0,"vol":33.5,"count":10},
    {"symbol":"deadusdt","open":0.0,"close":0.0,"amount":0.0,"vol":0.0,"count":0},
    {"symbol":"zigusdt","open":0.037,"close":0.037,"amount":0.0,"vol":0.0,"count":0}
]}"#;

#[test]
fn tickers_answer_by_the_markets_asked_for_with_the_24h_volume_and_no_change() {
    let quotes = parse_tickers(
        TICKERS,
        &[
            request("BTCUSDT"),
            request("eth/btc"),
            request("DEADUSDT"),
            request("MISSINGUSDT"),
            request("BTCUSDT&"),
        ],
    )
    .unwrap();
    assert_eq!(
        quotes.len(),
        2,
        "priceless, unlisted and malformed markets are left out"
    );
    let btc = &quotes["BTCUSDT"];
    assert_eq!(btc.price, 55_946.5);
    // `open` here is the open of HTX's own day, not what the price was 24 hours ago: no change.
    assert_eq!(btc.previous_day_price, None);
    assert_eq!(
        (btc.volume_24h, btc.volume_kind),
        (Some(4.3E8), Some(VolumeKind::QuoteCurrency))
    );
    // A pair quoted in a coin reports the units traded, and keeps the symbol as it was asked for.
    let eth = &quotes["eth/btc"];
    assert_eq!(
        (eth.volume_24h, eth.volume_kind),
        (Some(1_200.0), Some(VolumeKind::Base))
    );
}

#[test]
fn a_refused_price_call_is_told_apart_by_what_it_says() {
    use crate::service::watchlist_quotes::QuoteFailure;
    let limited = r#"{"status":"error","err-code":"bad-request","err-msg":"429 too many request"}"#;
    assert_eq!(parse_tickers(limited, &[]), Err(QuoteFailure::RateLimited));
    let refused = r#"{"status":"error","err-code":"x","err-msg":"maintenance"}"#;
    assert_eq!(
        parse_tickers(refused, &[]),
        Err(QuoteFailure::Failed("HTX: maintenance".to_owned()))
    );
    assert!(parse_tickers("not json", &[]).is_err());
    assert!(parse_tickers(r#"{"status":"ok"}"#, &[]).is_err());
}

#[test]
fn a_country_refusal_is_the_one_message() {
    assert_eq!(
        ProviderError::refused_by_region(DataSource::Htx).to_string(),
        "HTX refused the request (HTTP 403): it is not available in every country"
    );
    assert!(matches!(
        Htx.classify_status(403),
        Some(QuoteFailure::Failed(_))
    ));
    assert!(Htx.classify_status(400).is_none());
}

#[test]
fn a_market_nothing_can_name_is_not_listed() {
    let error = Htx
        .fetch_klines_range("BTC&USDT", Timeframe::HOUR_1, 0, 1, 10)
        .unwrap_err();
    assert_eq!(
        error,
        ProviderError::unknown_market(DataSource::Htx, "BTC&USDT", "")
    );
}

/// Run with `cargo test --lib htx::tests::live_htx -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_htx_serves_candles_pages_back_and_streams() {
    use crate::service::market::live::LiveFeed;
    use crate::service::watchlist_quotes::quote_stream::QuoteSocket;
    use std::time::{Duration, Instant};

    let consecutive = |candles: &[Kline], step: i64| {
        candles
            .windows(2)
            .all(|pair| pair[1].open_time - pair[0].open_time == step)
    };

    // Inside REST's reach: one request.
    let hourly = HTX.fetch_klines("BTCUSDT", Timeframe::HOUR_1, 500).unwrap();
    println!(
        "BTCUSDT 1h: {} candles, newest opens {}",
        hourly.len(),
        hourly.last().unwrap().open_time
    );
    assert_eq!(hourly.len(), 500);
    assert!(consecutive(&hourly, HOUR));
    assert!(
        hourly
            .iter()
            .all(|candle| candle.volume >= 0.0 && candle.close > 0.0)
    );

    // The page before, joined to the first at the hour its oldest candle opens.
    let oldest = hourly[0].open_time;
    let older = HTX
        .fetch_klines_range("BTCUSDT", Timeframe::HOUR_1, 0, oldest, 500)
        .unwrap();
    assert_eq!(older.len(), 500);
    assert_eq!(older.last().unwrap().open_time, oldest - HOUR);

    // Past REST's 2,000 candles (83 days of hours): the rest comes over the socket, joined.
    let long = HTX
        .fetch_klines("BTCUSDT", Timeframe::HOUR_1, 6_000)
        .unwrap();
    println!(
        "BTCUSDT 1h long: {} candles from {}",
        long.len(),
        long[0].open_time
    );
    assert_eq!(long.len(), 6_000);
    assert!(consecutive(&long, HOUR));

    // The whole history of a market: everything it has, then nothing before it.
    let everything = HTX
        .fetch_klines("BTCUSDT", Timeframe::HOUR_4, 30_000)
        .unwrap();
    println!(
        "BTCUSDT 4h everything: {} candles from {}",
        everything.len(),
        everything[0].open_time
    );
    assert!(everything.len() > 15_000 && everything.len() < 30_000);
    assert!(consecutive(&everything, 4 * HOUR));
    let before = HTX
        .fetch_klines_range(
            "BTCUSDT",
            Timeframe::HOUR_4,
            0,
            everything[0].open_time,
            500,
        )
        .unwrap();
    assert!(before.is_empty());

    // Every native opens where this app cuts it.
    for native in DataSource::Htx.natives() {
        let candles = HTX.fetch_klines("BTCUSDT", *native, 30).unwrap();
        assert_eq!(candles.len(), 30, "{native}");
        for candle in &candles {
            assert_eq!(
                bucket_start_ms(*native, candle.open_time),
                candle.open_time,
                "{native}"
            );
        }
    }

    // Days fold from four-hour bars: 1,000 of them reach back past what REST holds of any period
    // (about 333 days of four-hour bars), and each opens on this app's midnight.
    for (timeframe, count) in [
        ("1D", 1_000),
        ("1W", 200),
        ("1M", 60),
        ("3m", 100),
        ("8h", 400),
    ] {
        let folded = HTX.fetch_klines("BTCUSDT", tf(timeframe), count).unwrap();
        println!(
            "BTCUSDT {timeframe} folded: {} candles from {}",
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

    // Where HTX's own day and the fold cover the same hours they are the same bar: its day of
    // 16:00 UTC is six of the four-hour bars from 16:00 (open, high, low, close and volume).
    let four_hours = HTX.fetch_klines("BTCUSDT", Timeframe::HOUR_4, 400).unwrap();
    let own_body = rest::get("/market/history/kline?symbol=btcusdt&period=1day&size=60").unwrap();
    let own = parse_klines(&own_body, Timeframe::DAY_1).unwrap();
    let bar = own
        .iter()
        .rev()
        .skip(1)
        .find(|bar| bar.open_time % DAY == 16 * HOUR && bar.open_time > four_hours[0].open_time)
        .expect("a finished 16:00 day inside the four-hour bars");
    let parts: Vec<&Kline> = four_hours
        .iter()
        .filter(|part| (bar.open_time..bar.open_time + DAY).contains(&part.open_time))
        .collect();
    assert_eq!(parts.len(), 6);
    let volume: f64 = parts.iter().map(|part| part.volume).sum();
    println!(
        "day of {}: own volume {}, six 4h bars {volume}",
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

    let unknown = HTX.fetch_klines("NOPEUSDT", Timeframe::HOUR_1, 5);
    println!("NOPEUSDT: {unknown:?}");
    assert_eq!(unknown, Err(ProviderError::other("HTX: invalid symbol")));
    // The same market asked past REST's reach is refused over the socket.
    let unknown = HTX.fetch_klines("NOPEUSDT", Timeframe::HOUR_1, 6_000);
    println!("NOPEUSDT long: {unknown:?}");
    assert!(unknown.is_err());

    let hits = HTX.search_tickers("btc").unwrap();
    println!(
        "search btc: {:?}",
        hits.iter()
            .map(|hit| hit.symbol.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(hits[0].full_symbol, "BTCUSDT");
    assert_eq!(hits[0].quote.as_deref(), Some("USDT"));

    let requests: Vec<QuoteRequest> = ["BTCUSDT", "ETHBTC", "NOPEUSDT"]
        .into_iter()
        .map(request)
        .collect();
    let outcome = HTX.rest_quotes(&requests).unwrap();
    for symbol in ["BTCUSDT", "ETHBTC"] {
        let quote = outcome
            .quotes
            .get(symbol)
            .unwrap_or_else(|| panic!("no REST quote for {symbol}"));
        println!("REST quote {symbol}: {quote:?}");
        assert!(quote.price > 0.0 && quote.previous_day_price.is_none());
    }
    assert!(!outcome.quotes.contains_key("NOPEUSDT") && outcome.failed.is_empty());

    // A market HTX does not list must not silence the others on the socket.
    let feed = LiveFeed::connect(
        DataSource::Htx,
        vec![
            "NOPEUSDT".to_owned(),
            "BTCUSDT".to_owned(),
            "ETHUSDT".to_owned(),
        ],
        Timeframe::MINUTE_1,
        eframe::egui::Context::default(),
    )
    .expect("HTX streams one-minute candles");
    let (sender, receiver) = std::sync::mpsc::channel();
    let _quotes = QuoteSocket::connect(
        DataSource::Htx,
        vec!["NOPEUSDT".to_owned(), "BTCUSDT".to_owned()],
        sender,
    )
    .expect("HTX has a ticker socket");
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

/// A socket that answers the server's pings is still the same connection after 65 seconds, which
/// hold a dozen of them (the server drops a client that misses two). Run with
/// `cargo test --lib htx::tests::live_htx_pings -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_htx_pings_are_answered_for_sixty_five_seconds() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let session = socket::session(vec![subscribe(&kline_topic("btcusdt", "1min"))]);
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

/// HTX cuts the same buckets as Binance at every timeframe the chart offers, the ones it folds
/// (days, weeks, months, years among them) included. Run with
/// `cargo test --lib htx::tests::live_htx_buckets -- --ignored --nocapture`; the all-venues
/// comparison is `live_sources_cut_the_same_buckets`.
#[test]
#[ignore = "needs the network"]
fn live_htx_buckets_match_binances_at_every_timeframe() {
    let odd = ["5m", "22m", "3h", "4h", "6h", "12h", "2D", "3D", "2W", "6M"].map(tf);
    for interval in Timeframe::LEGACY.into_iter().chain(odd) {
        let opens = |source| -> std::collections::BTreeSet<i64> {
            crate::service::fetch_klines(source, "ETHUSDT", interval, 500)
                .unwrap_or_else(|error| panic!("{source:?} {interval}: {error}"))
                .iter()
                .map(|bar| bar.open_time)
                .collect()
        };
        let (binance, htx) = (opens(DataSource::Binance), opens(DataSource::Htx));
        // Compare inside the range both cover.
        let from = binance.first().max(htx.first()).copied().unwrap();
        let within = |set: &std::collections::BTreeSet<i64>| -> Vec<i64> {
            set.iter().copied().filter(|time| *time >= from).collect()
        };
        println!(
            "{interval:>3}: binance {:>4} htx {:>4} | shared range {:>4} bars",
            binance.len(),
            htx.len(),
            within(&binance).len()
        );
        assert!(within(&htx).len() > 2, "{interval}");
        assert_eq!(within(&binance), within(&htx), "{interval} binance vs htx");
    }
}
