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
    // With no time known for the held candle, the bar open when the socket connected is taken
    // to be in it (10 includes the 3 it had traded): only what it trades from here counts.
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

/// A fold told the held bucket was requested at `cutoff`.
fn fold_as_of(cutoff: i64) -> LiveFold {
    let mut fold = LiveFold::default();
    let _ = fold.merge_fetched(Vec::new(), &[], None, Some(cutoff), 500);
    fold
}

#[test]
fn a_bar_that_opened_after_the_held_candle_was_requested_counts_in_full() {
    let mut fold = fold_as_of(2 * MINUTE);
    let mut candles = vec![held_bucket()];
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    assert_eq!(candles[0].volume, 13.0, "the held 10 and the new bar's 3");
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 8.0));
    assert_eq!(candles[0].volume, 18.0);
    fold_into(&mut fold, &mut candles, bar(4 * MINUTE, 114.0, 2.0));
    assert_eq!(candles[0].volume, 20.0);
}

#[test]
fn a_bar_that_opened_before_the_held_candle_was_requested_counts_only_since() {
    let mut fold = fold_as_of(4 * MINUTE);
    let mut candles = vec![held_bucket()];
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    assert_eq!(candles[0].volume, 10.0);
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 8.0));
    assert_eq!(candles[0].volume, 15.0);
    fold_into(&mut fold, &mut candles, bar(4 * MINUTE, 114.0, 2.0));
    assert_eq!(
        candles[0].volume, 17.0,
        "the later bar is not in the held candle"
    );
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

fn guarded_merge(
    fold: &mut LiveFold,
    fetched: Vec<Kline>,
    current: &[Kline],
    as_of: i64,
) -> Vec<Kline> {
    fold.merge_fetched(fetched, current, Some(0), Some(as_of), 500)
}

#[test]
fn an_answer_that_comes_after_the_first_kline_gives_the_bucket_what_the_socket_missed() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    // The socket was first: the bucket holds only the bars it saw.
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    assert_eq!(candles[0].volume, 3.0);
    // The answer was requested at minute 2.5 and has the bucket as it was then.
    let merged = guarded_merge(
        &mut fold,
        vec![held_bucket()],
        &candles,
        2 * MINUTE + 30_000,
    );
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].open, 100.0, "the answer's open");
    assert_eq!((merged[0].high, merged[0].low), (120.0, 90.0));
    assert_eq!(merged[0].volume, 13.0);
    assert_eq!(merged[0].close, 112.0, "the socket's latest close");
    candles = merged;
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 8.0));
    assert_eq!(candles[0].volume, 18.0);
    fold_into(&mut fold, &mut candles, bar(4 * MINUTE, 114.0, 2.0));
    assert_eq!(candles[0].volume, 20.0);
}

#[test]
fn a_bar_open_before_the_race_answer_was_requested_is_not_counted_twice() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    // Requested at minute 3.5: the answer already counts the bar's first 3.
    let merged = guarded_merge(
        &mut fold,
        vec![held_bucket()],
        &candles,
        3 * MINUTE + 30_000,
    );
    assert_eq!(merged[0].volume, 10.0);
    candles = merged;
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 5.0));
    assert_eq!(candles[0].volume, 12.0);
}

#[test]
fn an_answer_that_has_no_candle_of_the_bucket_leaves_the_socket_bars_as_they_are() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    let earlier = Kline {
        open_time: -5 * MINUTE,
        close_time: -1,
        ..held_bucket()
    };
    let merged = guarded_merge(&mut fold, vec![earlier.clone()], &candles, 2 * MINUTE);
    assert_eq!(merged, vec![earlier, candles[0].clone()]);
}

#[test]
fn a_guarded_answer_does_not_move_a_bucket_that_already_has_its_held_candle() {
    let mut fold = fold_as_of(2 * MINUTE);
    let mut candles = vec![held_bucket()];
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    let refreshed = Kline {
        volume: 40.0,
        ..held_bucket()
    };
    let merged = guarded_merge(&mut fold, vec![refreshed], &candles, 3 * MINUTE + 30_000);
    assert_eq!(merged, candles);
    candles = merged;
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 8.0));
    assert_eq!(candles[0].volume, 18.0);
}

#[test]
fn an_answer_with_nothing_newer_than_it_starts_the_bucket_again() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    let merged = fold.merge_fetched(vec![held_bucket()], &candles, None, Some(4 * MINUTE), 500);
    assert_eq!(merged, vec![held_bucket()]);
    candles = merged;
    // The answer is the account of the bucket, requested at minute 4: the bar open since 3 is in it.
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 113.0, 5.0));
    assert_eq!(candles[0].volume, 10.0);
}

#[test]
fn an_unknown_request_time_adopts_nothing() {
    let (mut fold, mut candles) = (LiveFold::default(), Vec::new());
    fold_into(&mut fold, &mut candles, bar(3 * MINUTE, 112.0, 3.0));
    let merged = fold.merge_fetched(vec![held_bucket()], &candles, Some(0), None, 500);
    assert_eq!(merged, candles);
}
