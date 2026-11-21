use super::*;

#[test]
fn every_native_has_its_interval_and_nothing_else_does() {
    let intervals: Vec<String> = DataSource::Bybit
        .natives()
        .iter()
        .map(|native| interval(*native).expect("a native has an interval"))
        .collect();
    assert_eq!(
        intervals,
        [
            "1", "3", "5", "15", "30", "60", "120", "240", "360", "720", "D", "W", "M"
        ]
    );
    for other in ["2m", "10m", "45m", "8h", "2D", "2W", "3M", "1Y"] {
        assert_eq!(interval(Timeframe::parse(other).unwrap()), None, "{other}");
    }
}

fn base_of(timeframe: &str) -> (String, usize) {
    let plan = plan(DataSource::Bybit, Timeframe::parse(timeframe).unwrap()).unwrap();
    (interval(plan.base()).unwrap(), plan.base_count(500))
}

#[test]
fn other_lengths_fold_the_coarsest_native_that_nests() {
    // A request for 500 candles also asks for the one the start cuts.
    assert_eq!(base_of("1h"), ("60".to_owned(), 500));
    assert_eq!(base_of("22m"), ("1".to_owned(), 501 * 22));
    assert_eq!(base_of("8h"), ("240".to_owned(), 501 * 2));
    assert_eq!(base_of("3D"), ("D".to_owned(), 501 * 3));
    assert_eq!(base_of("3M"), ("M".to_owned(), 501 * 3));
    assert_eq!(base_of("1Y"), ("M".to_owned(), 501 * 12));
}

#[test]
fn a_request_wider_than_the_paging_is_capped_before_it_is_folded() {
    // 500 candles of 1,439 minutes (nothing larger folds from one-minute bars) is 720,939
    // base candles; paging reaches 60,000.
    let plan = plan(DataSource::Bybit, Timeframe::parse("1439m").unwrap()).unwrap();
    assert_eq!(PAGING.requested(plan, 500), PAGING.max_candles);
    let hour = super::plan(DataSource::Bybit, Timeframe::HOUR_1).unwrap();
    assert_eq!(PAGING.requested(hour, 500), 500);
}

#[test]
fn symbols_are_bybits_however_they_are_spelled() {
    assert_eq!(symbol_id("btc/usdt"), "BTCUSDT");
    assert_eq!(symbol_id("ETH-USDC"), "ETHUSDC");
    assert_eq!(symbol_id("sol"), "SOLUSDT");
    assert_eq!(symbol_id("WBTC"), "WBTCUSDT");
}

const ROWS: &str = r#"{"retCode":0,"retMsg":"OK","result":{"category":"spot","symbol":"BTCUSDT","list":[
    ["1700002800000","11","13","10","12","5.5","60"],
    ["1699999200000","10","12","9","11","42.5","450"]
]}}"#;

#[test]
fn rows_come_newest_first_and_leave_oldest_first_with_the_close_of_their_bucket() {
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
    let row = serde_json::json!(["1769904000000", "1", "2", "0.5", "1.5", "3", "4"]);
    let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
    assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
}

#[test]
fn a_refusal_is_bybits_message_and_the_limits_are_rate_limits() {
    let unknown = r#"{"retCode":10001,"retMsg":"Not supported symbols","result":{}}"#;
    assert_eq!(
        parse_candles(unknown, Timeframe::HOUR_1),
        Err(ProviderError::other("Bybit: Not supported symbols"))
    );
    for code in [10_006, 10_018] {
        let limited = format!(r#"{{"retCode":{code},"retMsg":"Too many visits!","result":{{}}}}"#);
        let error = parse_candles(&limited, Timeframe::HOUR_1).unwrap_err();
        assert!(error.is_rate_limited(), "{code}: {error}");
    }
    assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
    assert!(parse_candles(r#"{"retCode":0,"result":{}}"#, Timeframe::HOUR_1).is_err());
    assert!(parse_candles(r#"{"result":{"list":[]}}"#, Timeframe::HOUR_1).is_err());
    let bad_field =
        r#"{"retCode":0,"result":{"list":[["1700000000000","x","1","1","1","1","1"]]}}"#;
    assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
}

#[test]
fn the_catalog_keeps_only_trading_markets_and_follows_its_cursor() {
    let body = r#"{"retCode":0,"result":{"category":"spot","nextPageCursor":"page2","list":[
        {"symbol":"BTCUSDT","baseCoin":"BTC","quoteCoin":"USDT","status":"Trading"},
        {"symbol":"OLDUSDT","baseCoin":"OLD","quoteCoin":"USDT","status":"Closed"},
        {"symbol":"NOQUOTE","baseCoin":"X","quoteCoin":"","status":"Trading"}
    ]}}"#;
    let (pairs, next) = parse_pairs(body).expect("pairs");
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        (
            pairs[0].id.as_str(),
            pairs[0].base.as_str(),
            pairs[0].quote.as_str()
        ),
        ("BTCUSDT", "BTC", "USDT")
    );
    assert_eq!(next.as_deref(), Some("page2"));
    let last = r#"{"retCode":0,"result":{"nextPageCursor":"","list":[]}}"#;
    assert_eq!(parse_pairs(last).unwrap().1, None);
    assert!(parse_pairs("not json").is_err());
}

#[test]
fn a_pair_is_found_however_it_is_separated() {
    let pairs = vec![
        Pair {
            id: "BTCUSDT".into(),
            base: "BTC".into(),
            quote: "USDT".into(),
        },
        Pair {
            id: "ETHBTC".into(),
            base: "ETH".into(),
            quote: "BTC".into(),
        },
    ];
    for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
        let found = matching_pairs(&pairs, &normalize(query));
        assert_eq!(found.len(), 1, "{query}");
        assert_eq!(found[0].symbol, "BTCUSDT");
    }
    assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
}

/// Run with `cargo test --lib bybit::tests::live -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_bybit_serves_candles_pairs_and_every_native() {
    let hits = BYBIT.search_tickers("btc-usdt").expect("search");
    assert_eq!(hits[0].symbol, "BTCUSDT");
    assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
    for native in DataSource::Bybit.natives() {
        // More than one page of the shortest bars, so paging is exercised.
        let wanted = if *native == Timeframe::MINUTE_1 {
            1_500
        } else {
            350
        };
        let candles = BYBIT
            .fetch_klines("BTCUSDT", *native, wanted)
            .expect("candles");
        // Weeks and months reach back only to the market's listing.
        if *native >= Timeframe::WEEK_1 {
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
    // A 22-minute chart of 500 candles is eleven thousand one-minute ones: 12 pages.
    let folded = BYBIT
        .fetch_klines("BTCUSDT", Timeframe::parse("22m").unwrap(), 500)
        .expect("folded candles");
    assert_eq!(folded.len(), 500);
    let error = BYBIT
        .fetch_klines("NOPEUSDT", Timeframe::HOUR_1, 5)
        .unwrap_err();
    assert_eq!(error.to_string(), "Bybit: Not supported symbols");
}
