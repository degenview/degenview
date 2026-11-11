use super::*;

fn bars(range: std::ops::Range<i64>) -> Vec<Kline> {
    range
        .map(|hour| Kline {
            open_time: hour * 3_600_000,
            close_time: (hour + 1) * 3_600_000 - 1,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 1.0,
        })
        .collect()
}

fn times(bars: &[Kline]) -> Vec<i64> {
    bars.iter().map(|bar| bar.open_time / 3_600_000).collect()
}

#[test]
fn a_page_is_asked_for_only_near_the_oldest_candle() {
    let history = OlderHistory::default();
    let now = Instant::now();
    assert!(history.should_request(true, true, 500, now));
    assert!(!history.should_request(true, false, 500, now));
    assert!(
        !history.should_request(false, true, 500, now),
        "a provider that cannot serve a range is never asked"
    );
}

#[test]
fn the_first_page_is_fetched_ahead_of_the_user_once() {
    let mut history = OlderHistory::default();
    let now = Instant::now();
    assert!(history.untouched());
    assert!(history.should_request(true, history.untouched(), 500, now));
    history.started();
    assert!(!history.untouched(), "while a page is out");
    history.arrived(40);
    assert!(!history.untouched(), "once a page is held");

    let mut failing = OlderHistory::default();
    failing.started();
    failing.failed(now);
    assert!(
        !failing.untouched(),
        "a failure waits for the user to get there"
    );

    let mut empty = OlderHistory::default();
    empty.started();
    empty.arrived(0);
    assert!(!empty.untouched(), "nothing older exists");
}

#[test]
fn only_one_request_is_out_at_a_time() {
    let mut history = OlderHistory::default();
    let now = Instant::now();
    history.started();
    assert!(history.is_loading());
    assert!(!history.should_request(true, true, 500, now));
    history.arrived(10);
    assert!(history.should_request(true, true, 510, now));
}

#[test]
fn an_empty_page_is_the_start_of_history() {
    let mut history = OlderHistory::default();
    history.started();
    history.arrived(0);
    assert!(!history.should_request(true, true, 500, Instant::now()));
    assert!(!history.keeps_older(), "nothing was added to keep");
}

#[test]
fn a_failure_waits_before_the_next_try() {
    let mut history = OlderHistory::default();
    let now = Instant::now();
    history.started();
    history.failed(now);
    assert!(!history.should_request(true, true, 500, now));
    assert!(!history.should_request(true, true, 500, now + RETRY_AFTER / 2));
    assert!(history.should_request(true, true, 500, now + RETRY_AFTER));
}

#[test]
fn a_dropped_request_can_be_made_again_at_once() {
    let mut history = OlderHistory::default();
    history.started();
    history.abandoned();
    assert!(history.should_request(true, true, 500, Instant::now()));
}

#[test]
fn paging_stops_at_the_most_a_chart_keeps() {
    let history = OlderHistory::default();
    assert!(!history.should_request(true, true, MAX_CANDLES, Instant::now()));
}

#[test]
fn a_page_goes_in_front_in_order_without_a_repeated_candle() {
    let mut held = bars(100..120);
    let mut page = bars(90..101);
    page.reverse();
    let added = prepend(&mut held, page);
    assert_eq!(added, 10, "the candle both have is kept once");
    assert_eq!(times(&held), (90..120).collect::<Vec<_>>());
}

#[test]
fn a_page_of_nothing_new_changes_nothing() {
    let mut held = bars(100..120);
    assert_eq!(prepend(&mut held, bars(105..110)), 0);
    assert_eq!(prepend(&mut held, Vec::new()), 0);
    assert_eq!(times(&held), (100..120).collect::<Vec<_>>());
}

#[test]
fn a_refresh_keeps_the_older_candles_it_did_not_bring() {
    let held = bars(50..120);
    let fresh = bars(80..125);
    let merged = with_older(&held, fresh, true);
    assert_eq!(times(&merged), (50..125).collect::<Vec<_>>());
}

#[test]
fn a_refresh_replaces_everything_while_no_history_was_loaded() {
    let held = bars(50..120);
    let merged = with_older(&held, bars(80..125), false);
    assert_eq!(times(&merged), (80..125).collect::<Vec<_>>());
}

#[test]
fn a_refresh_with_nothing_is_nothing() {
    assert!(with_older(&bars(0..10), Vec::new(), true).is_empty());
}

#[test]
fn kept_history_is_bounded() {
    let held = bars(0..(MAX_CANDLES as i64 + 50));
    let fresh = bars((MAX_CANDLES as i64 + 40)..(MAX_CANDLES as i64 + 60));
    let merged = with_older(&held, fresh, true);
    assert_eq!(merged.len(), MAX_CANDLES);
    assert_eq!(
        merged.last().map(|bar| bar.open_time),
        Some((MAX_CANDLES as i64 + 59) * 3_600_000),
        "the newest candles are the ones that stay"
    );
}
