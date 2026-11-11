use super::*;
use crate::model::chart::viewport::MIN_VISIBLE;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
/// A Monday midnight, UTC.
const T0: i64 = 1_704_067_200_000;

/// `count` bars `step` long starting at `start`.
fn bars(start: i64, step: i64, count: usize) -> Vec<Kline> {
    (0..count as i64)
        .map(|index| bar(start + index * step, step))
        .collect()
}

fn bar(open_time: i64, step: i64) -> Kline {
    Kline {
        open_time,
        close_time: open_time + step - 1,
        open: 1.0,
        high: 2.0,
        low: 0.5,
        close: 1.5,
        volume: 1.0,
    }
}

/// `count` daily bars that skip every Saturday and Sunday.
fn weekdays(count: usize) -> Vec<Kline> {
    (0..)
        .map(|day| T0 + day * DAY)
        .filter(|time| ((time - T0) / DAY) % 7 < 5)
        .take(count)
        .map(|time| bar(time, DAY))
        .collect()
}

fn view(visible: usize, right_offset: f64) -> HorizontalViewport {
    HorizontalViewport {
        visible,
        right_offset,
    }
}

const POSITION: ChartSyncSettings = ChartSyncSettings {
    horizontal_position: true,
    horizontal_zoom: false,
};
const ZOOM: ChartSyncSettings = ChartSyncSettings {
    horizontal_position: false,
    horizontal_zoom: true,
};
const BOTH: ChartSyncSettings = ChartSyncSettings {
    horizontal_position: true,
    horizontal_zoom: true,
};

/// The change a leader's navigation from `before` to `after` makes.
fn change(
    data: &[Kline],
    before: &HorizontalViewport,
    after: &HorizontalViewport,
) -> HorizontalChange {
    HorizontalChange::between(before, after, &TimeAxis::new(data)).expect("something changed")
}

fn follower(
    settings: ChartSyncSettings,
    change: &HorizontalChange,
    data: &[Kline],
    current: &HorizontalViewport,
) -> Followed {
    let zoom = change.zoom.map(|zoom| ZoomFollow {
        factor: zoom.to as f64 / zoom.from as f64,
        start_visible: current.visible,
    });
    follow(settings, change, zoom, &TimeAxis::new(data), current)
}

fn right_time(data: &[Kline], view: &HorizontalViewport) -> f64 {
    TimeAxis::new(data).range_of(view).expect("bars").right
}

// MARK: Time axis

#[test]
fn the_time_scale_round_trips_inside_and_beyond_the_bars() {
    let data = bars(T0, HOUR, 50);
    let axis = TimeAxis::new(&data);
    for x in [-30.0, -0.5, 0.0, 0.25, 7.0, 31.75, 49.99, 50.0, 53.5, 90.0] {
        let time = axis.time_at(x).expect("time");
        let back = axis.x_at(time).expect("x");
        assert!((back - x).abs() < 1e-9, "{x} came back as {back}");
    }
}

#[test]
fn the_end_of_the_newest_bar_is_the_edge_of_the_data() {
    let data = bars(T0, HOUR, 10);
    let axis = TimeAxis::new(&data);
    assert_eq!(axis.time_at(10.0), Some((T0 + 10 * HOUR) as f64));
    assert_eq!(axis.time_at(0.0), Some(T0 as f64));
}

#[test]
fn empty_space_continues_at_the_length_of_the_edge_bars() {
    let data = bars(T0, HOUR, 10);
    let axis = TimeAxis::new(&data);
    assert_eq!(axis.time_at(12.0), Some((T0 + 12 * HOUR) as f64));
    assert_eq!(axis.time_at(-3.0), Some((T0 - 3 * HOUR) as f64));
}

#[test]
fn a_gap_in_the_data_is_one_slot_and_time_never_runs_backwards() {
    let data = weekdays(30);
    let axis = TimeAxis::new(&data);
    let mut last = f64::NEG_INFINITY;
    let mut x = -2.0;
    while x < 34.0 {
        let time = axis.time_at(x).expect("time");
        assert!(time > last, "time stalled or reversed at {x}");
        let back = axis.x_at(time).expect("x");
        assert!((back - x).abs() < 1e-9);
        last = time;
        x += 0.125;
    }
    // Friday's slot covers the weekend: the next bar is Monday.
    assert_eq!(axis.time_at(5.0), Some((T0 + 7 * DAY) as f64));
}

#[test]
fn an_empty_chart_has_no_time_scale() {
    let axis = TimeAxis::new(&[]);
    assert_eq!(axis.time_at(0.0), None);
    assert_eq!(axis.x_at(1.0), None);
    assert_eq!(axis.range_of(&view(120, 0.0)), None);
}

#[test]
fn charts_of_different_timeframes_agree_on_the_present() {
    let hourly = bars(T0, HOUR, 200);
    let quarter = bars(T0 + 100 * HOUR, 15 * MINUTE, 400);
    // Both end at the same instant: T0 + 200 h.
    let a = TimeAxis::new(&hourly)
        .range_of(&view(50, 0.0))
        .expect("range");
    let b = TimeAxis::new(&quarter)
        .range_of(&view(50, 0.0))
        .expect("range");
    assert_eq!(a.right, b.right);
}

// MARK: Detecting a change

#[test]
fn nothing_shared_changed_when_the_viewport_did_not() {
    let data = bars(T0, HOUR, 200);
    let same = view(120, 4.0);
    assert!(HorizontalChange::between(&same, &same, &TimeAxis::new(&data)).is_none());
}

#[test]
fn a_zoom_keeps_the_right_edge_and_is_not_a_move() {
    let data = bars(T0, HOUR, 500);
    let change = change(&data, &view(120, 10.0), &view(60, 10.0));
    assert!(!change.moved);
    assert_eq!(change.zoom, Some(Zoom { from: 120, to: 60 }));
}

#[test]
fn a_pan_is_a_move_without_a_zoom() {
    let data = bars(T0, HOUR, 500);
    let change = change(&data, &view(120, 0.0), &view(120, 3.5));
    assert!(change.moved);
    assert_eq!(change.zoom, None);
    assert!(!change.latest);
}

#[test]
fn the_newest_bar_is_flagged() {
    let data = bars(T0, HOUR, 500);
    assert!(change(&data, &view(120, 8.0), &view(120, 0.0)).latest);
}

// MARK: Position

#[test]
fn position_sync_puts_followers_at_the_same_time_and_keeps_their_zoom() {
    let leader = bars(T0, HOUR, 400);
    let other = bars(T0, 15 * MINUTE, 1600);
    let panned = change(&leader, &view(100, 0.0), &view(100, 24.0));

    let followed = follower(POSITION, &panned, &other, &view(300, 0.0));
    assert_eq!(followed.viewport.visible, 300, "zoom is the follower's own");
    assert!(followed.reached);
    assert_eq!(
        right_time(&other, &followed.viewport),
        right_time(&leader, &view(100, 24.0)),
        "24 hourly bars back is 96 quarter-hour bars back"
    );
    assert!((followed.viewport.right_offset - 96.0).abs() < 1e-9);
}

#[test]
fn position_sync_works_in_both_directions() {
    let a = bars(T0, HOUR, 400);
    let b = bars(T0, 15 * MINUTE, 1600);
    let from_b = change(&b, &view(300, 0.0), &view(300, 40.0));
    let on_a = follower(POSITION, &from_b, &a, &view(100, 0.0));
    assert!((on_a.viewport.right_offset - 10.0).abs() < 1e-9);
}

#[test]
fn dragging_right_pulls_followers_back_in_time_and_left_forward() {
    let data = bars(T0, HOUR, 400);
    let target = view(100, 20.0);
    let back = follower(
        POSITION,
        &change(&data, &view(100, 0.0), &view(100, 30.0)),
        &data,
        &target,
    );
    let forward = follower(
        POSITION,
        &change(&data, &view(100, 30.0), &view(100, 10.0)),
        &data,
        &target,
    );
    assert!(back.viewport.right_offset > target.right_offset);
    assert!(forward.viewport.right_offset < target.right_offset);
}

#[test]
fn followers_keep_a_fraction_of_a_bar() {
    let leader = bars(T0, HOUR, 400);
    let other = bars(T0, 30 * MINUTE, 800);
    let panned = change(&leader, &view(100, 0.0), &view(100, 3.3));
    let followed = follower(POSITION, &panned, &other, &view(100, 0.0));
    assert!((followed.viewport.right_offset - 6.6).abs() < 1e-9);
}

#[test]
fn a_smooth_pan_moves_every_follower_smoothly_even_across_a_weekend() {
    let leader = bars(T0, HOUR, 24 * 40);
    let other = weekdays(30);
    let mut last = f64::NEG_INFINITY;
    let mut previous = view(100, 0.0);
    for step in 1..=600 {
        let next = view(100, f64::from(step) * 0.25);
        let panned = change(&leader, &previous, &next);
        let followed = follower(POSITION, &panned, &other, &view(20, 0.0));
        assert!(
            followed.viewport.right_offset >= last,
            "the follower turned back at step {step}"
        );
        last = followed.viewport.right_offset;
        previous = next;
    }
}

#[test]
fn position_sync_off_leaves_followers_alone() {
    let data = bars(T0, HOUR, 400);
    let panned = change(&data, &view(100, 0.0), &view(100, 24.0));
    let current = view(100, 3.0);
    for settings in [ChartSyncSettings::UNLINKED, ZOOM] {
        let followed = follower(settings, &panned, &data, &current);
        assert_eq!(followed.viewport, current);
    }
}

#[test]
fn the_newest_bar_sends_followers_to_theirs() {
    let leader = bars(T0, HOUR, 400);
    // Ends earlier: its newest bar is not at the leader's time.
    let other = bars(T0, HOUR, 380);
    let home = change(&leader, &view(100, 30.0), &view(100, 0.0));
    let followed = follower(POSITION, &home, &other, &view(100, 25.0));
    assert_eq!(followed.viewport.right_offset, 0.0);
}

#[test]
fn space_past_the_newest_bar_is_followed_in_time() {
    let leader = bars(T0, HOUR, 400);
    let other = bars(T0, 15 * MINUTE, 1600);
    let panned = change(&leader, &view(100, 0.0), &view(100, -5.0));
    let followed = follower(POSITION, &panned, &other, &view(100, 0.0));
    assert!((followed.viewport.right_offset + 20.0).abs() < 1e-9);
    assert!(followed.reached);
}

#[test]
fn a_chart_with_less_history_stops_at_its_oldest_bars_and_says_so() {
    let leader = bars(T0, HOUR, 1000);
    let short = bars(T0 + 900 * HOUR, HOUR, 100);
    let panned = change(&leader, &view(100, 0.0), &view(100, 700.0));
    let followed = follower(POSITION, &panned, &short, &view(60, 0.0));
    assert!(!followed.reached);
    assert_eq!(
        followed.viewport.right_offset,
        (100 - MIN_VISIBLE) as f64,
        "held at its oldest bars, never past them"
    );
    assert_eq!(followed.viewport.visible, 60);
}

#[test]
fn a_chart_that_was_short_catches_up_once_older_bars_arrive() {
    let leader = bars(T0, HOUR, 1000);
    let short = bars(T0 + 900 * HOUR, HOUR, 100);
    let longer = bars(T0 + 200 * HOUR, HOUR, 800);
    let mut hub = SyncHub::default();
    let mut other = view(60, 0.0);
    hub.publish(1, change(&leader, &view(100, 0.0), &view(100, 700.0)));
    assert!(hub.deliver(2, POSITION, &TimeAxis::new(&short), &mut other));
    // Nothing new to say, but the follower is still short of the leader's time.
    assert!(hub.deliver(2, POSITION, &TimeAxis::new(&longer), &mut other));
    assert_eq!(
        right_time(&longer, &other),
        right_time(&leader, &view(100, 700.0))
    );
    // Now it has arrived: no more work.
    assert!(!hub.deliver(2, POSITION, &TimeAxis::new(&longer), &mut other));
}

#[test]
fn a_chart_with_no_bars_is_left_alone() {
    let data = bars(T0, HOUR, 400);
    let panned = change(&data, &view(100, 0.0), &view(100, 24.0));
    let followed = follower(BOTH, &panned, &[], &view(100, 0.0));
    assert_eq!(followed.viewport, view(100, 0.0));
}

// MARK: Zoom

#[test]
fn zoom_sync_scales_followers_by_the_same_factor_and_keeps_their_position() {
    let a = bars(T0, HOUR, 600);
    let b = bars(T0 + 100 * HOUR, 15 * MINUTE, 2000);
    let zoomed = change(&a, &view(100, 40.0), &view(50, 40.0));
    let followed = follower(ZOOM, &zoomed, &b, &view(300, 12.5));
    assert_eq!(followed.viewport.visible, 150);
    assert_eq!(
        followed.viewport.right_offset, 12.5,
        "no jump to the leader's date"
    );
}

#[test]
fn zoom_sync_off_leaves_followers_alone() {
    let data = bars(T0, HOUR, 600);
    let zoomed = change(&data, &view(100, 0.0), &view(50, 0.0));
    let current = view(300, 12.0);
    for settings in [ChartSyncSettings::UNLINKED, POSITION] {
        assert_eq!(
            follower(settings, &zoomed, &data, &current).viewport,
            current
        );
    }
}

#[test]
fn zooming_out_widens_followers() {
    let data = bars(T0, HOUR, 900);
    let zoomed = change(&data, &view(100, 0.0), &view(200, 0.0));
    assert_eq!(
        follower(ZOOM, &zoomed, &data, &view(80, 0.0))
            .viewport
            .visible,
        160
    );
}

#[test]
fn zoom_stays_inside_what_a_follower_can_show() {
    let data = bars(T0, HOUR, 90);
    let zoomed = change(&data, &view(100, 0.0), &view(20, 0.0));
    let followed = follower(ZOOM, &zoomed, &data, &view(30, 0.0));
    assert_eq!(followed.viewport.visible, MIN_VISIBLE);
}

#[test]
fn both_linked_followers_show_the_leaders_time_range() {
    let leader = bars(T0, HOUR, 600);
    let other = bars(T0, 15 * MINUTE, 2400);
    let zoomed = change(&leader, &view(100, 30.0), &view(48, 30.0));
    let followed = follower(BOTH, &zoomed, &other, &view(300, 500.0));
    let shown = TimeAxis::new(&other)
        .range_of(&followed.viewport)
        .expect("range");
    let wanted = TimeAxis::new(&leader)
        .range_of(&view(48, 30.0))
        .expect("range");
    assert_eq!(shown.right, wanted.right);
    assert_eq!(shown.left, wanted.left, "48 hours is 192 quarter-hours");
    assert_eq!(followed.viewport.visible, 192);
}

#[test]
fn both_linked_the_range_matches_whatever_the_widths_since_it_is_in_time() {
    // The viewport has no pixels in it: a narrow follower shows the same bars, just thinner.
    let leader = bars(T0, HOUR, 600);
    let zoomed = change(&leader, &view(100, 0.0), &view(40, 0.0));
    let followed = follower(BOTH, &zoomed, &leader, &view(100, 0.0));
    assert_eq!(followed.viewport, view(40, 0.0));
}

#[test]
fn both_linked_a_range_over_a_weekend_is_matched_in_time_not_in_bars() {
    let leader = bars(T0, DAY, 400);
    let sessions = weekdays(300);
    let zoomed = change(&leader, &view(70, 0.0), &view(35, 0.0));
    let followed = follower(BOTH, &zoomed, &sessions, &view(100, 0.0));
    // 35 calendar days hold 25 weekdays.
    assert_eq!(followed.viewport.visible, 25);
}

#[test]
fn both_linked_a_pan_moves_position_without_touching_zoom() {
    let data = bars(T0, HOUR, 600);
    let panned = change(&data, &view(100, 0.0), &view(100, 10.0));
    let followed = follower(BOTH, &panned, &data, &view(300, 0.0));
    assert_eq!(followed.viewport, view(300, 10.0));
}

#[test]
fn a_reset_with_both_linked_returns_every_chart_to_the_newest_bars() {
    let data = bars(T0, HOUR, 600);
    let reset = change(&data, &view(30, 200.0), &HorizontalViewport::default());
    let followed = follower(BOTH, &reset, &data, &view(300, 70.0));
    assert_eq!(followed.viewport.right_offset, 0.0);
    assert_eq!(followed.viewport.visible, 120);
}

// MARK: Hub

fn pan_by(offset: f64) -> HorizontalChange {
    let data = bars(T0, HOUR, 600);
    change(&data, &view(100, 0.0), &view(100, offset))
}

#[test]
fn the_leader_never_follows_itself() {
    let data = bars(T0, HOUR, 600);
    let mut hub = SyncHub::default();
    hub.publish(1, pan_by(10.0));
    let mut leader = view(100, 10.0);
    assert!(!hub.deliver(1, BOTH, &TimeAxis::new(&data), &mut leader));
    assert_eq!(leader, view(100, 10.0));
}

#[test]
fn every_follower_applies_an_event_once() {
    let data = bars(T0, HOUR, 600);
    let mut hub = SyncHub::default();
    hub.publish(1, pan_by(10.0));
    let mut b = view(100, 0.0);
    assert!(hub.deliver(2, POSITION, &TimeAxis::new(&data), &mut b));
    // The user then drags the follower's chart by hand: the event is not replayed over it.
    b.right_offset = 55.0;
    assert!(!hub.deliver(2, POSITION, &TimeAxis::new(&data), &mut b));
    assert_eq!(b.right_offset, 55.0);
    let mut c = view(100, 0.0);
    assert!(hub.deliver(3, POSITION, &TimeAxis::new(&data), &mut c));
    assert_eq!(b.visible, c.visible);
}

#[test]
fn sequences_grow_and_the_newest_event_wins() {
    let mut hub = SyncHub::default();
    hub.publish(1, pan_by(10.0));
    let first = hub.latest_event().expect("event").sequence;
    hub.publish(3, pan_by(20.0));
    let second = *hub.latest_event().expect("event");
    assert!(second.sequence > first);
    assert_eq!(second.origin, 3, "any chart may lead next");
    let data = bars(T0, HOUR, 600);
    let mut a = view(100, 0.0);
    assert!(hub.deliver(1, POSITION, &TimeAxis::new(&data), &mut a));
    assert_eq!(a.right_offset, 20.0);
}

#[test]
fn a_follower_that_moved_because_it_followed_publishes_nothing() {
    // Following edits the viewport before the frame's own input is looked at, so the frame sees
    // no change: the chart's own diff stays empty.
    let data = bars(T0, HOUR, 600);
    let mut hub = SyncHub::default();
    hub.publish(1, pan_by(10.0));
    let mut b = view(100, 0.0);
    hub.deliver(2, POSITION, &TimeAxis::new(&data), &mut b);
    let at_frame_start = b;
    assert!(HorizontalChange::between(&at_frame_start, &b, &TimeAxis::new(&data)).is_none());
    assert_eq!(hub.latest_event().expect("event").origin, 1);
}

#[test]
fn unlinked_charts_do_not_follow() {
    let data = bars(T0, HOUR, 600);
    let mut hub = SyncHub::default();
    hub.publish(1, pan_by(10.0));
    let mut b = view(100, 0.0);
    assert!(!hub.deliver(
        2,
        ChartSyncSettings::UNLINKED,
        &TimeAxis::new(&data),
        &mut b
    ));
    assert_eq!(b, view(100, 0.0));
}

#[test]
fn turning_a_setting_on_changes_nothing_until_the_next_navigation() {
    let data = bars(T0, HOUR, 600);
    let mut hub = SyncHub::default();
    hub.publish(1, pan_by(10.0));
    let mut b = view(100, 0.0);
    // An event from before the link was made is dropped by the reset the toggle performs.
    hub.reset();
    assert!(!hub.deliver(2, BOTH, &TimeAxis::new(&data), &mut b));
    assert_eq!(b, view(100, 0.0));
}

#[test]
fn a_burst_of_zooming_does_not_accumulate_rounding() {
    let data = bars(T0, HOUR, 2000);
    let mut hub = SyncHub::default();
    let axis = TimeAxis::new(&data);
    let mut follower_view = view(333, 7.0);
    let mut leader = 120;
    // The leader zooms out a bar at a time over many frames.
    while leader < 240 {
        let before = view(leader, 0.0);
        leader += 1;
        hub.publish(1, change(&data, &before, &view(leader, 0.0)));
        hub.deliver(2, ZOOM, &axis, &mut follower_view);
        hub.end_frame();
    }
    // Exactly what one big zoom of the same factor gives.
    assert_eq!(
        follower_view.visible,
        (333.0_f64 * 240.0 / 120.0).round() as usize
    );
    assert_eq!(follower_view.right_offset, 7.0);
}

#[test]
fn a_zoom_burst_ends_with_the_first_frame_that_does_not_zoom() {
    let data = bars(T0, HOUR, 2000);
    let axis = TimeAxis::new(&data);
    let mut hub = SyncHub::default();
    let mut b = view(200, 0.0);
    hub.publish(1, change(&data, &view(100, 0.0), &view(110, 0.0)));
    hub.deliver(2, ZOOM, &axis, &mut b);
    hub.end_frame();
    hub.end_frame();
    // A new gesture starts from where the follower now is.
    hub.publish(1, change(&data, &view(110, 0.0), &view(55, 0.0)));
    hub.deliver(2, ZOOM, &axis, &mut b);
    assert_eq!(b.visible, 110, "half of the 220 it had");
}

#[test]
fn a_chart_that_leaves_is_forgotten() {
    let data = bars(T0, HOUR, 600);
    let mut hub = SyncHub::default();
    hub.publish(1, pan_by(10.0));
    hub.forget(1);
    let mut b = view(100, 0.0);
    assert!(!hub.deliver(2, POSITION, &TimeAxis::new(&data), &mut b));
}

// MARK: Settings

#[test]
fn settings_round_trip() {
    for settings in [ChartSyncSettings::UNLINKED, POSITION, ZOOM, BOTH] {
        let json = serde_json::to_string(&settings).expect("serialize");
        assert_eq!(
            serde_json::from_str::<ChartSyncSettings>(&json).expect("decode"),
            settings
        );
    }
}

#[test]
fn charts_are_linked_both_ways_unless_the_user_says_otherwise() {
    assert_eq!(ChartSyncSettings::default(), BOTH);
    let settings: ChartSyncSettings = serde_json::from_str("{}").expect("decode");
    assert_eq!(settings, BOTH, "data saved before linking opens linked");
    // A choice that was saved is kept, and what it did not mention is not guessed as off.
    let unlinked = serde_json::to_string(&ChartSyncSettings::UNLINKED).expect("encode");
    assert_eq!(
        serde_json::from_str::<ChartSyncSettings>(&unlinked).expect("decode"),
        ChartSyncSettings::UNLINKED
    );
    let partial: ChartSyncSettings =
        serde_json::from_str(r#"{"horizontal_position":false}"#).expect("decode");
    assert_eq!(partial, ZOOM);
    assert!(!ChartSyncSettings::UNLINKED.is_linked());
}
