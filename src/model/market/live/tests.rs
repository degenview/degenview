use super::*;
use crate::format::days_from_civil;
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

const MINUTE: i64 = 60_000;
const DAY: i64 = 86_400_000;

fn bar(open_time: i64, close: f64, volume: f64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + MINUTE - 1,
        open: close - 1.0,
        high: close + 2.0,
        low: close - 3.0,
        close,
        volume,
    }
}

fn five_minutes() -> Timeframe {
    Timeframe::parse("5m").unwrap()
}

fn fold_into(fold: &mut LiveFold, candles: &mut Vec<Kline>, kline: Kline) -> Option<i64> {
    fold.apply(five_minutes(), Timeframe::MINUTE_1, candles, kline)
}

#[test]
fn a_base_bar_resent_as_it_grows_is_counted_once() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    for volume in [1.0, 2.5, 4.0] {
        fold_into(&mut fold, &mut candles, bar(0, 10.0, volume));
    }
    assert_eq!(candles.len(), 1);
    assert_eq!(candles[0].volume, 4.0);
}

#[test]
fn the_bars_of_a_bucket_make_one_candle() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(0, 10.0, 2.0));
    fold_into(&mut fold, &mut candles, bar(MINUTE, 14.0, 3.0));
    fold_into(&mut fold, &mut candles, bar(2 * MINUTE, 8.0, 1.0));
    fold_into(&mut fold, &mut candles, bar(2 * MINUTE, 9.0, 1.5));
    assert_eq!(candles.len(), 1);
    let candle = &candles[0];
    assert_eq!((candle.open_time, candle.close_time), (0, 5 * MINUTE - 1));
    assert_eq!(candle.open, 9.0, "the first bar's open");
    assert_eq!((candle.high, candle.low), (16.0, 6.0));
    assert_eq!(candle.close, 9.0, "the last bar's close");
    assert_eq!(candle.volume, 6.5);
}

#[test]
fn the_next_bucket_follows_the_last_and_the_old_one_stays() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(4 * MINUTE, 10.0, 2.0));
    let changed = fold_into(&mut fold, &mut candles, bar(5 * MINUTE, 20.0, 3.0));
    assert_eq!(changed, Some(5 * MINUTE));
    assert_eq!(candles.len(), 2);
    assert_eq!((candles[0].open_time, candles[0].volume), (0, 2.0));
    assert_eq!((candles[1].open_time, candles[1].volume), (5 * MINUTE, 3.0));
}

#[test]
fn a_late_bar_of_an_older_bucket_is_ignored() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(5 * MINUTE, 20.0, 3.0));
    let before = candles.clone();
    assert_eq!(
        fold_into(&mut fold, &mut candles, bar(MINUTE, 10.0, 9.0)),
        None
    );
    assert_eq!(candles, before);
}

#[test]
fn an_unchanged_kline_changes_nothing() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    assert!(fold_into(&mut fold, &mut candles, bar(0, 10.0, 2.0)).is_some());
    assert_eq!(fold_into(&mut fold, &mut candles, bar(0, 10.0, 2.0)), None);
}

#[test]
fn a_twenty_two_minute_day_ends_in_a_shorter_bucket_that_midnight_cuts() {
    let midnight = days_from_civil(2026, 10, 5) * DAY;
    let target = Timeframe::parse("22m").unwrap();
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    // 23:59 belongs to the 23:50 bucket, which closes at midnight.
    fold.apply(
        target,
        Timeframe::MINUTE_1,
        &mut candles,
        bar(midnight - MINUTE, 10.0, 1.0),
    );
    assert_eq!(candles[0].open_time, midnight - 10 * MINUTE);
    assert_eq!(candles[0].close_time, midnight - 1);
    // 00:00 starts a bucket of its own, not a continuation of it.
    fold.apply(
        target,
        Timeframe::MINUTE_1,
        &mut candles,
        bar(midnight, 11.0, 1.0),
    );
    assert_eq!(candles.len(), 2);
    assert_eq!(candles[1].open_time, midnight);
    assert_eq!(candles[1].close_time, midnight + 22 * MINUTE - 1);
    assert_eq!(candles[0].volume, 1.0);
}

fn held_bucket() -> Kline {
    Kline {
        open_time: 0,
        close_time: 5 * MINUTE - 1,
        open: 100.0,
        high: 120.0,
        low: 90.0,
        close: 110.0,
        volume: 10.0,
    }
}

#[test]
fn what_the_series_held_of_the_bucket_is_kept_and_its_open_bar_is_not_counted_twice() {
    let (mut fold, mut candles) = (LiveFold::default(), vec![held_bucket()]);
    // The bar open when the socket connected is already in the held volume (10 includes the 3
    // it had traded): only what it trades from here counts.
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    assert_eq!(candles[0].volume, 10.0);
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 5.0));
    assert_eq!(candles[0].volume, 12.0);
    fold_into(&mut fold, &mut candles, bar(4 * MINUTE, 114.0, 2.0));
    let candle = &candles[0];
    assert_eq!(candle.volume, 14.0);
    assert_eq!(candle.open, 100.0, "the held open");
    assert_eq!((candle.high, candle.low), (120.0, 90.0));
    assert_eq!(candle.close, 114.0);
}

#[test]
fn a_reset_starts_the_bucket_again_from_what_the_series_holds() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(0, 10.0, 4.0));
    fold.reset();
    // A refresh replaced the candle with its own account, which has the bar at 6.
    candles = vec![Kline {
        volume: 6.0,
        ..candles[0].clone()
    }];
    fold_into(&mut fold, &mut candles, bar(0, 10.0, 7.0));
    assert_eq!(candles[0].volume, 6.0);
    fold_into(&mut fold, &mut candles, bar(0, 10.0, 8.0));
    assert_eq!(candles[0].volume, 7.0);
}

#[test]
fn a_new_timeframe_starts_a_new_bucket() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(0, 10.0, 2.0));
    let ten = Timeframe::parse("10m").unwrap();
    let mut other = Vec::new();
    fold.apply(ten, Timeframe::MINUTE_1, &mut other, bar(MINUTE, 11.0, 5.0));
    assert_eq!(
        other[0].volume, 5.0,
        "nothing of the five-minute bucket carries over"
    );
}

#[test]
fn a_kline_of_the_timeframe_itself_replaces_the_candle() {
    let hour = Timeframe::HOUR_1;
    let (mut fold, mut candles) = (LiveFold::default(), vec![candle(1, 10.0)]);
    assert_eq!(
        fold.apply(hour, hour, &mut candles, candle(1, 12.0)),
        Some(1)
    );
    assert_eq!(candles, vec![candle(1, 12.0)]);
    assert_eq!(
        fold.apply(hour, hour, &mut candles, candle(2, 13.0)),
        Some(2)
    );
    assert_eq!(candles.len(), 2);
}

/// The base bars of the first bucket as REST reports them: minutes 0 to 2 trade 2, 3 and 2.
fn rest_bars() -> Vec<Kline> {
    vec![
        bar(0, 100.0, 2.0),
        bar(MINUTE, 101.0, 3.0),
        bar(2 * MINUTE, 102.0, 2.0),
    ]
}

/// A REST answer for the first bucket through `fold`, over `candles`.
fn merge_rest(
    fold: &mut LiveFold,
    fetched: Vec<Kline>,
    bars: Vec<Kline>,
    candles: &[Kline],
    live_open: Option<i64>,
) -> Vec<Kline> {
    fold.merge_fetched(five_minutes(), fetched, bars, candles, live_open, 500)
}

#[test]
fn a_bar_the_socket_has_further_along_than_rest_keeps_the_socket_volume() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 8.0));
    // The answer was cut before the bar traded more than 3.
    let mut bars = rest_bars();
    bars.push(bar(3 * MINUTE, 103.0, 3.0));
    candles = merge_rest(&mut fold, vec![held_bucket()], bars, &candles, Some(0));
    assert_eq!(candles.len(), 1);
    assert_eq!(candles[0].volume, 15.0, "2 + 3 + 2 and the socket's 8");
    assert_eq!(candles[0].close, 112.0);
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 9.0));
    assert_eq!(candles[0].volume, 16.0);
    fold_into(&mut fold, &mut candles, bar(4 * MINUTE, 114.0, 2.0));
    assert_eq!(candles[0].volume, 18.0);
}

#[test]
fn a_minute_the_answer_already_has_is_not_counted_again_when_the_socket_reaches_it() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    // The answer came after the minute began: its candle and its bars both hold the 3 it traded.
    let mut bars = rest_bars();
    bars.push(bar(3 * MINUTE, 103.0, 3.0));
    let fetched = Kline {
        volume: 10.0,
        ..held_bucket()
    };
    candles = merge_rest(&mut fold, vec![fetched], bars, &candles, None);
    assert_eq!(candles[0].volume, 10.0);
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 103.0, 3.0));
    assert_eq!(candles[0].volume, 10.0, "not 13");
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 104.0, 4.0));
    assert_eq!(candles[0].volume, 11.0);
}

#[test]
fn the_socket_first_and_the_answer_after_make_the_whole_bucket() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    assert_eq!(candles[0].volume, 3.0);
    candles = merge_rest(
        &mut fold,
        vec![held_bucket()],
        rest_bars(),
        &candles,
        Some(0),
    );
    assert_eq!(candles[0].volume, 10.0);
    assert_eq!(candles[0].open, 99.0, "the first bar's open");
    assert_eq!(candles[0].close, 112.0, "the socket's latest close");
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 5.0));
    assert_eq!(candles[0].volume, 12.0);
}

#[test]
fn a_finished_bucket_the_answer_holds_is_not_replaced_by_the_sockets_account_of_it() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(2 * MINUTE, 20.0, 1.0));
    let finished = Kline {
        close: 25.0,
        ..held_bucket()
    };
    let next = Kline {
        open_time: 5 * MINUTE,
        close_time: 10 * MINUTE - 1,
        ..held_bucket()
    };
    let merged = merge_rest(
        &mut fold,
        vec![finished.clone(), next.clone()],
        vec![bar(5 * MINUTE, 105.0, 4.0)],
        &candles,
        Some(0),
    );
    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged[0], finished,
        "REST's close of 25, not the socket's 20"
    );
    assert_eq!(merged[1].open_time, 5 * MINUTE);
    assert_eq!(merged[1].volume, 4.0, "the new bucket is its bars");
}

#[test]
fn an_answer_with_no_bars_leaves_the_held_candle_rule_as_it_was() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    candles = merge_rest(&mut fold, vec![held_bucket()], Vec::new(), &candles, None);
    assert_eq!(candles, vec![held_bucket()]);
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    assert_eq!(candles[0].volume, 10.0);
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 5.0));
    assert_eq!(candles[0].volume, 12.0);
}

#[test]
fn an_answer_with_no_bars_and_no_newer_socket_bar_starts_the_bucket_again() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(0, 10.0, 4.0));
    let refreshed = Kline {
        volume: 6.0,
        ..candles[0].clone()
    };
    candles = merge_rest(&mut fold, vec![refreshed], Vec::new(), &candles, None);
    fold_into(&mut fold, &mut candles, bar(0, 10.0, 7.0));
    assert_eq!(candles[0].volume, 6.0);
}

#[test]
fn a_bar_state_with_less_volume_than_the_stored_one_is_dropped_and_a_tie_replaces_it() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(0, 10.0, 5.0));
    fold_into(&mut fold, &mut candles, bar(0, 11.0, 4.0));
    assert_eq!((candles[0].close, candles[0].volume), (10.0, 5.0));
    fold_into(&mut fold, &mut candles, bar(0, 12.0, 5.0));
    assert_eq!((candles[0].close, candles[0].volume), (12.0, 5.0));
}

#[test]
fn bars_of_a_bucket_older_than_the_forming_one_are_ignored() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(5 * MINUTE, 20.0, 3.0));
    let merged = merge_rest(
        &mut fold,
        vec![held_bucket()],
        rest_bars(),
        &candles,
        Some(5 * MINUTE),
    );
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0], held_bucket());
    assert_eq!(merged[1], candles[0]);
}
