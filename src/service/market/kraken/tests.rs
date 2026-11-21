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
        Err(ProviderError::other("Kraken: Unknown asset pair"))
    );
    for code in [
        "EAPI:Rate limit exceeded",
        "EGeneral:Too many requests",
        "EService:Throttled: 1700000000",
    ] {
        let limited = format!(r#"{{"error":["{code}"]}}"#);
        let error = parse_ohlc(&limited, Timeframe::HOUR_1).unwrap_err();
        assert!(error.is_rate_limited(), "{code}: {error}");
    }
    assert!(parse_ohlc("not json", Timeframe::HOUR_1).is_err());
    assert!(parse_ohlc(r#"{"error":[]}"#, Timeframe::HOUR_1).is_err());
    assert!(parse_ohlc(r#"{"error":[],"result":{"last":1}}"#, Timeframe::HOUR_1).is_err());
    let bad_field = r#"{"error":[],"result":{"BTC/USD":[[1699999200,"x","1","1","1","1","1",1]]}}"#;
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
    for query in ["btcusd", "BTC/USD", "btc-usd", "BTC USD", "xbtusd"] {
        let found = matching_pairs(&pairs, &alias(&normalize(query)));
        assert_eq!(found.len(), 1, "{query}");
        assert_eq!(found[0].symbol, "BTC/USD");
    }
    assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
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
    assert_eq!(error.to_string(), "Kraken: Unknown asset pair");
}
