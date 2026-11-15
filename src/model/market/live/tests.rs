use super::*;
use std::time::Duration;

fn candle(open_time: i64, close: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + 59_999,
        open: close,
        high: close,
        low: close,
        close,
        volume: 1.0,
    }
}

#[test]
fn a_live_kline_replaces_the_forming_bar() {
    let mut candles = vec![candle(1, 10.0), candle(2, 20.0)];
    assert!(apply_live(&mut candles, candle(2, 25.0)));
    assert_eq!(candles, vec![candle(1, 10.0), candle(2, 25.0)]);
}

#[test]
fn a_live_kline_of_a_new_bar_follows_the_last() {
    let mut candles = vec![candle(1, 10.0)];
    assert!(apply_live(&mut candles, candle(2, 20.0)));
    assert_eq!(candles, vec![candle(1, 10.0), candle(2, 20.0)]);
    let mut empty = Vec::new();
    assert!(apply_live(&mut empty, candle(2, 20.0)));
    assert_eq!(empty, vec![candle(2, 20.0)]);
}

#[test]
fn a_late_kline_of_an_older_bar_is_ignored() {
    let mut candles = vec![candle(1, 10.0), candle(2, 20.0)];
    assert!(!apply_live(&mut candles, candle(1, 11.0)));
    assert_eq!(candles, vec![candle(1, 10.0), candle(2, 20.0)]);
}

#[test]
fn an_identical_kline_changes_nothing() {
    let mut candles = vec![candle(1, 10.0)];
    assert!(!apply_live(&mut candles, candle(1, 10.0)));
}

#[test]
fn live_bars_win_over_an_older_rest_snapshot() {
    let fetched = vec![candle(1, 10.0), candle(2, 20.0)];
    let live = vec![candle(1, 10.0), candle(2, 25.0), candle(3, 30.0)];
    assert_eq!(merge_fetched(fetched.clone(), &live, Some(2), 500), live);
}

#[test]
fn without_a_live_mark_the_snapshot_replaces_the_forming_bar() {
    let fetched = vec![candle(1, 10.0), candle(2, 20.0)];
    let live = vec![candle(1, 10.0), candle(2, 25.0), candle(3, 30.0)];
    assert_eq!(merge_fetched(fetched.clone(), &live, None, 500), fetched);
}

#[test]
fn the_merge_keeps_the_newest_candles_up_to_the_limit() {
    let fetched = vec![candle(1, 1.0), candle(2, 2.0), candle(3, 3.0)];
    assert_eq!(
        merge_fetched(fetched, &[], None, 2),
        vec![candle(2, 2.0), candle(3, 3.0)]
    );
}

#[test]
fn a_mark_guards_only_requests_begun_before_it_arrived() {
    let begun = Instant::now();
    let arrived = begun + Duration::from_secs(1);
    let mark = LiveMark::new(7, arrived);
    assert_eq!(mark.guard_for(begun), Some(7));
    assert_eq!(mark.guard_for(arrived), Some(7));
    assert_eq!(
        mark.guard_for(arrived + Duration::from_secs(1)),
        None,
        "the socket went quiet before this request began"
    );
}
