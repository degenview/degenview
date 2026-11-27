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
    let now = Instant::now();
    assert!(history.should_request(true, true, FULL_CANDLES - 1, now));
    assert!(
        !history.should_request(true, true, FULL_CANDLES, now),
        "what a trim leaves is not paged back again"
    );
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

fn refresh(held: &[Kline], fresh: Vec<Kline>) -> Refreshed {
    refreshed(DataSource::Binance, held, fresh)
}

#[test]
fn a_refresh_keeps_the_older_candles_it_did_not_bring() {
    let merged = refresh(&bars(50..120), bars(80..125));
    assert_eq!(times(&merged.candles), (50..125).collect::<Vec<_>>());
    assert!(!merged.restarted);
}

#[test]
fn a_sliding_window_keeps_the_first_candle() {
    let mut held = bars(0..10);
    for start in 1..4 {
        held = refresh(&held, bars(start..start + 10)).candles;
    }
    assert_eq!(times(&held), (0..13).collect::<Vec<_>>());
}

#[test]
fn a_refresh_that_starts_after_the_newest_candle_held_starts_the_series_again() {
    let merged = refresh(&bars(10..20), bars(30..40));
    assert_eq!(times(&merged.candles), (30..40).collect::<Vec<_>>());
    assert!(merged.restarted);
}

#[test]
fn the_first_answer_has_nothing_to_restart() {
    let merged = refresh(&[], bars(0..10));
    assert_eq!(merged.candles.len(), 10);
    assert!(!merged.restarted);
}

#[test]
fn a_line_chart_is_what_the_latest_answer_drew() {
    let merged = refreshed(DataSource::Polymarket, &bars(0..10), bars(2..12));
    assert_eq!(times(&merged.candles), (2..12).collect::<Vec<_>>());
    assert!(!merged.restarted);
}

#[test]
fn a_candle_added_after_an_outage_does_not_join_the_two_sides_of_it() {
    let mut held = bars(0..100);
    held.extend(bars(700..701));
    let merged = refresh(&held, bars(201..701));
    assert_eq!(times(&merged.candles), (201..701).collect::<Vec<_>>());
    assert!(merged.restarted);
}

#[test]
fn a_refresh_that_reaches_further_back_than_what_is_held_is_the_series() {
    let merged = refresh(&bars(50..51), bars(0..51));
    assert_eq!(times(&merged.candles), (0..51).collect::<Vec<_>>());
    assert!(!merged.restarted);
}

/// One of a provider's servers can be behind the others: its answer is an older view.
#[test]
fn an_answer_that_ends_before_the_newest_candle_held_changes_nothing() {
    let merged = refresh(&bars(0..10), bars(0..8));
    assert_eq!(times(&merged.candles), (0..10).collect::<Vec<_>>());
    assert!(!merged.restarted);
}

#[test]
fn a_refresh_with_nothing_changes_nothing() {
    let merged = refresh(&bars(0..10), Vec::new());
    assert_eq!(times(&merged.candles), (0..10).collect::<Vec<_>>());
    assert!(!merged.restarted);
}

#[test]
fn a_full_series_makes_room_for_a_page_of_candles_at_once() {
    let full = MAX_CANDLES as i64;
    let kept = refresh(&bars(0..full), bars(full - 10..full + 1)).candles;
    assert_eq!(kept.len(), FULL_CANDLES);
    assert_eq!(
        kept.last().map(|bar| bar.open_time),
        Some(full * 3_600_000),
        "the newest candles are the ones that stay"
    );
    let next = refresh(&kept, bars(full - 10..full + 2)).candles;
    assert_eq!(
        next.first(),
        kept.first(),
        "the next candle does not move the series' start"
    );
}

#[test]
fn a_refresh_during_the_first_page_does_not_drop_the_bar_the_page_ends_before() {
    let mut history = OlderHistory::default();
    let mut held = bars(10..20);
    let end = page_end(&held).unwrap();
    history.started();

    // A bar opened while the page was out: the refresh brings the newest ten, 11..21.
    held = refresh(&held, bars(11..21)).candles;
    assert_eq!(times(&held), (10..21).collect::<Vec<_>>());

    // The page was asked for with the old end and stops before hour 10.
    let page: Vec<Kline> = bars(5..10)
        .into_iter()
        .filter(|b| b.open_time < end)
        .collect();
    let added = prepend(&mut held, page);
    history.arrived(added);
    assert_eq!(
        times(&held),
        (5..21).collect::<Vec<_>>(),
        "hour 10 is still there"
    );
}

#[test]
fn a_chart_with_a_script_pages_back_until_it_holds_what_scripts_run_over() {
    let history = OlderHistory::default();
    let now = Instant::now();
    assert!(short_for_scripts(true, 500));
    assert!(short_for_scripts(true, SCRIPT_DEPTH - 1));
    assert!(!short_for_scripts(true, SCRIPT_DEPTH));
    assert!(
        !short_for_scripts(false, 500),
        "a chart with no script keeps to what is on screen"
    );
    // The page is asked for with the view nowhere near the oldest candle.
    assert!(history.should_request(true, short_for_scripts(true, 500), 500, now));
    // But never past what the chart may keep, from a provider with nothing older, or twice at once.
    let mut paging = OlderHistory::default();
    paging.started();
    assert!(!paging.should_request(true, true, 500, now));
    paging.arrived(0);
    assert!(!paging.should_request(true, true, 500, now));
}
