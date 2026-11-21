use super::*;

#[test]
fn every_native_has_its_bar_and_nothing_else_does() {
    let bars: Vec<String> = DataSource::Okx
        .natives()
        .iter()
        .map(|native| bar(*native).expect("a native has a bar"))
        .collect();
    assert_eq!(
        bars,
        [
            "1m", "3m", "5m", "15m", "30m", "1H", "2H", "4H", "6Hutc", "12Hutc", "1Dutc", "1Wutc",
            "1Mutc", "3Mutc"
        ]
    );
    for other in ["2m", "8h", "2D", "3D", "2W", "2M", "6M", "1Y"] {
        assert_eq!(bar(Timeframe::parse(other).unwrap()), None, "{other}");
    }
}

fn base_of(timeframe: &str) -> (String, usize) {
    let plan = plan(DataSource::Okx, Timeframe::parse(timeframe).unwrap()).unwrap();
    (bar(plan.base()).unwrap(), plan.base_count(500))
}

#[test]
fn other_lengths_fold_the_coarsest_native_that_nests() {
    // A request for 500 candles also asks for the one the start cuts.
    assert_eq!(base_of("1h"), ("1H".to_owned(), 500));
    assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
    assert_eq!(base_of("3h"), ("1H".to_owned(), 501 * 3));
    assert_eq!(base_of("8h"), ("4H".to_owned(), 501 * 2));
    assert_eq!(base_of("3D"), ("1Dutc".to_owned(), 501 * 3));
    assert_eq!(base_of("1Y"), ("3Mutc".to_owned(), 501 * 4));
}

#[test]
fn symbols_are_okx_ids() {
    assert_eq!(instrument_id("btc-usdt"), "BTC-USDT");
    assert_eq!(instrument_id("ETH/USDC"), "ETH-USDC");
    assert_eq!(instrument_id("sol"), "SOL-USDT");
}

const ROWS: &str = r#"{"code":"0","msg":"","data":[
    ["1700002800000","11","13","10","12","5.5","60","60","0"],
    ["1699999200000","10","12","9","11","42.5","450","450","1"]
]}"#;

#[test]
fn rows_come_newest_first_and_leave_oldest_first() {
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
    let row = serde_json::json!(["1769904000000", "1", "2", "0.5", "1.5", "3", "4", "4", "1"]);
    let candle = parse_row(&row, Timeframe::MONTH_1).expect("row");
    // 2026-03-01 00:00 UTC, less a millisecond: February has 28 days.
    assert_eq!(candle.close_time, 1_772_323_200_000 - 1);
}

#[test]
fn an_error_answer_is_okxs_message() {
    let body = r#"{"code":"51001","msg":"Instrument ID doesn't exist.","data":[]}"#;
    assert_eq!(
        parse_candles(body, Timeframe::HOUR_1),
        Err(ProviderError::other("OKX: Instrument ID doesn't exist."))
    );
    assert!(parse_candles("not json", Timeframe::HOUR_1).is_err());
    assert!(parse_candles(r#"{"code":"0"}"#, Timeframe::HOUR_1).is_err());
    let bad_field = r#"{"code":"0","data":[["1700000000000","x","1","1","1","1"]]}"#;
    assert!(parse_candles(bad_field, Timeframe::HOUR_1).is_err());
}

/// Run with `cargo test --lib okx::tests::live -- --ignored --nocapture`.
#[test]
#[ignore = "needs the network"]
fn live_okx_serves_candles_pairs_and_a_chart_of_every_native() {
    let hits = OKX.search_tickers("btc-usdt").expect("search");
    assert_eq!(hits[0].symbol, "BTC-USDT");
    assert_eq!(hits[0].quote.as_deref(), Some("USDT"));
    for native in DataSource::Okx.natives() {
        let candles = OKX.fetch_klines("BTC-USDT", *native, 350).expect("candles");
        // Months and quarters reach back only to the market's listing.
        if *native >= Timeframe::MONTH_1 {
            assert!(candles.len() > 30, "{native}");
        } else {
            assert_eq!(candles.len(), 350, "{native}");
        }
        assert!(
            candles
                .windows(2)
                .all(|pair| pair[0].open_time < pair[1].open_time),
            "{native}"
        );
    }
    // A 22-minute chart of 500 candles is eleven thousand one-minute ones: 37 pages.
    let folded = OKX
        .fetch_klines("BTC-USDT", Timeframe::parse("22m").unwrap(), 500)
        .expect("folded candles");
    assert_eq!(folded.len(), 500);
    let error = OKX
        .fetch_klines("NOPE-USDT", Timeframe::HOUR_1, 5)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "OKX: Instrument ID, Instrument ID code, or Spread ID doesn't exist."
    );
}

#[test]
fn the_catalog_keeps_only_live_markets_with_all_their_fields() {
    let body = r#"{"code":"0","data":[
        {"instId":"BTC-USDT","state":"live","baseCcy":"BTC","quoteCcy":"USDT"},
        {"instId":"OLD-USDT","state":"suspend","baseCcy":"OLD","quoteCcy":"USDT"},
        {"instId":"NOQUOTE","state":"live","baseCcy":"X","quoteCcy":""}
    ]}"#;
    let pairs = parse_pairs(body).expect("pairs");
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        (
            pairs[0].id.as_str(),
            pairs[0].base.as_str(),
            pairs[0].quote.as_str()
        ),
        ("BTC-USDT", "BTC", "USDT")
    );
    assert!(parse_pairs("not json").is_err());
}

#[test]
fn a_pair_is_found_however_it_is_separated() {
    let pairs = vec![
        Pair {
            id: "BTC-USDT".into(),
            base: "BTC".into(),
            quote: "USDT".into(),
        },
        Pair {
            id: "ETH-BTC".into(),
            base: "ETH".into(),
            quote: "BTC".into(),
        },
    ];
    for query in ["btcusdt", "BTC-USDT", "btc/usdt", "BTC USDT"] {
        let found = matching_pairs(&pairs, &normalize(query));
        assert_eq!(found.len(), 1, "{query}");
        assert_eq!(found[0].symbol, "BTC-USDT");
    }
    assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
}
