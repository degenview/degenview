use super::*;

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
        let found = matching_pairs(&pairs, &normalize(query));
        assert_eq!(found.len(), 1, "{query}");
        assert_eq!(found[0].symbol, "BTCUSDT");
    }
    assert_eq!(matching_pairs(&pairs, "BTC").len(), 2);
}

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

fn base_of(timeframe: &str) -> (String, usize) {
    let plan = plan(DataSource::Binance, Timeframe::parse(timeframe).unwrap()).unwrap();
    (plan.base().interval(), plan.base_count(500))
}

#[test]
fn a_native_length_is_asked_for_as_it_is() {
    for token in [
        "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h",
    ] {
        assert_eq!(base_of(token), (token.to_owned(), 500), "{token}");
    }
    assert_eq!(base_of("1d"), ("1d".to_owned(), 500));
    assert_eq!(base_of("1w"), ("1w".to_owned(), 500));
    assert_eq!(base_of("1M"), ("1M".to_owned(), 500));
}

#[test]
fn other_lengths_fold_the_coarsest_native_that_nests() {
    // A request for 500 candles also asks for the one the start cuts.
    assert_eq!(base_of("22m"), ("1m".to_owned(), 501 * 22));
    assert_eq!(base_of("6m"), ("3m".to_owned(), 501 * 2));
    assert_eq!(base_of("3h"), ("1h".to_owned(), 501 * 3));
    assert_eq!(base_of("10h"), ("2h".to_owned(), 501 * 5));
    assert_eq!(base_of("24h"), ("12h".to_owned(), 501 * 2));
    // Binance's own 3d is not on the grid the other sources cut, so days are folded.
    assert_eq!(base_of("3d"), ("1d".to_owned(), 501 * 3));
    assert_eq!(base_of("2w"), ("1w".to_owned(), 501 * 2));
    assert_eq!(base_of("3M"), ("1M".to_owned(), 501 * 3));
    assert_eq!(base_of("1Y"), ("1M".to_owned(), 501 * 12));
}

#[test]
fn a_series_of_more_than_one_requests_candles_is_paged_but_its_tail_is_not() {
    assert!(base_of("5m").1 <= MAX_PER_REQUEST);
    assert!(base_of("22m").1 > MAX_PER_REQUEST);
    let tail = plan(DataSource::Binance, Timeframe::parse("22m").unwrap())
        .unwrap()
        .base_count(crate::service::cache::TAIL_LIMIT as usize);
    assert!(tail <= MAX_PER_REQUEST);
}

#[test]
fn rejects_error_objects() {
    let error =
        parse_klines(r#"{"code":-1121,"msg":"Invalid symbol."}"#).expect_err("object must fail");
    assert_eq!(error, ProviderError::data("expected a candle array"));
}

#[test]
fn the_catalog_keeps_only_trading_markets_with_all_their_fields() {
    let body = r#"{"symbols":[
        {"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT"},
        {"symbol":"LUNAUSDT","status":"BREAK","baseAsset":"LUNA","quoteAsset":"USDT"},
        {"symbol":"NOQUOTE","status":"TRADING","baseAsset":"X"}
    ]}"#;
    let pairs = parse_pairs(body).expect("pairs");
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        (
            pairs[0].symbol.as_str(),
            pairs[0].base.as_str(),
            pairs[0].quote.as_str()
        ),
        ("BTCUSDT", "BTC", "USDT")
    );
    assert!(parse_pairs("not json").is_err());
    assert!(parse_pairs("{}").expect("no symbols").is_empty());
}
