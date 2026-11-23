use super::*;
use crate::service::market::candles::plan;

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
fn the_symbols_and_interval_are_in_the_url() {
    let sessions = SHAPE.kline_sessions(&["BTCUSDT".into(), "ETHUSDT".into()], Timeframe::HOUR_1);
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].url,
        "wss://stream.binance.com/stream?streams=btcusdt@kline_1h/ethusdt@kline_1h"
    );
    assert!(sessions[0].subscribe.is_empty());
}
