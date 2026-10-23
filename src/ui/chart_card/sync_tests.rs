//! Linked navigation through real pointer events: several charts drawn in one frame, the way the
//! workspace draws them, with the user dragging, scrolling and zooming one of them.

use super::*;
use crate::model::chart_sync::{ChartSyncSettings, SyncHub, TimeAxis};
use crate::model::viewport::{DEFAULT_VISIBLE, HorizontalViewport};
use crate::model::{ChartType, IndicatorConfig, Tool};
use crate::ui::replay_overlay::ReplayChartFrame;
use eframe::egui::Modifiers;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const T0: i64 = 1_700_000_000_000;

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

fn series(start: i64, step: i64, count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let close = 100.0 + (index as f64 * 0.3).sin() * 20.0 + index as f64 * 0.1;
            Kline {
                open_time: start + index as i64 * step,
                close_time: start + (index as i64 + 1) * step - 1,
                open: close - 1.0,
                high: close + 2.0,
                low: close - 3.0,
                close,
                volume: 10.0,
            }
        })
        .collect()
}

/// One chart of the layout: its card, its bars and where it sits on the window.
struct Member {
    card: ChartCard,
    candles: Vec<Kline>,
    source: DataSource,
    /// Where the chart is drawn, from its left edge and how wide.
    left: f32,
    width: f32,
    rect: Rect,
}

impl Member {
    fn new(candles: Vec<Kline>, left: f32, width: f32) -> Self {
        let mut card = ChartCard::default();
        card.set_timeframe(Timeframe::Hour);
        Self {
            card,
            candles,
            source: DataSource::Binance,
            left,
            width,
            rect: Rect::NOTHING,
        }
    }
}

/// Charts side by side in one window, linked as the workspace links them.
struct Linked {
    ctx: egui::Context,
    members: Vec<Member>,
    hub: SyncHub,
    settings: ChartSyncSettings,
    time: f64,
}

impl Linked {
    fn new(settings: ChartSyncSettings, members: Vec<Member>) -> Self {
        let mut linked = Self {
            ctx: egui::Context::default(),
            members,
            hub: SyncHub::default(),
            settings,
            time: 0.0,
        };
        for _ in 0..3 {
            linked.frame(Vec::new());
        }
        linked
    }

    /// Two charts of the same bars, one wide and one narrow.
    fn pair(settings: ChartSyncSettings) -> Self {
        let bars = series(T0, HOUR, 800);
        Self::new(
            settings,
            vec![
                Member::new(bars.clone(), 0.0, 700.0),
                Member::new(bars, 710.0, 420.0),
            ],
        )
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.016;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1140.0, 520.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (members, hub, settings) = (&mut self.members, &mut self.hub, self.settings);
        let mut output = self.ctx.run_ui(input, |ui| {
            for (index, member) in members.iter_mut().enumerate() {
                let rect = Rect::from_min_size(
                    Pos2::new(member.left, 0.0),
                    Vec2::new(member.width, 520.0),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    let id = index as u64 + 1;
                    // The workspace's turn for a chart: hear of the others, draw, tell them.
                    member
                        .card
                        .follow_sync(hub, id, settings, member.source, &member.candles);
                    let mut frame = DrawingFrame::new(&[], Tool::Cursor, None, 100);
                    let mut replay = ReplayChartFrame {
                        selecting: false,
                        active: false,
                        picked_ms: None,
                    };
                    let pointer = member.card.body(
                        ui,
                        "BTCUSDT",
                        &member.candles,
                        false,
                        None,
                        PriceScaleMode::Regular,
                        member.source,
                        &IndicatorConfig::default(),
                        None,
                        id,
                        &mut frame,
                        &mut replay,
                    );
                    member.rect = pointer.rect.unwrap_or(Rect::NOTHING);
                    member.card.publish_sync(hub, id, settings);
                });
            }
            hub.end_frame();
        });
        output.textures_delta.clear();
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.frame(Vec::new());
        }
    }

    fn button(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    /// A drag by `delta` from `from`, over a few frames, with the button released at the end.
    fn drag(&mut self, from: Pos2, delta: Vec2) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.frame(vec![Self::button(from, true)]);
        for step in 1..=6 {
            let at = from + delta * (step as f32 / 6.0);
            self.frame(vec![egui::Event::PointerMoved(at)]);
        }
        self.frame(vec![Self::button(from + delta, false)]);
        self.frame(Vec::new());
    }

    fn wheel(&mut self, at: Pos2, delta: Vec2) {
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        }]);
        self.frames(4);
    }

    fn in_plot(&self, chart: usize) -> Pos2 {
        let rect = self.members[chart].rect;
        Pos2::new(rect.left() + rect.width() * 0.4, rect.top() + 180.0)
    }

    fn on_price_axis(&self, chart: usize) -> Pos2 {
        let rect = self.members[chart].rect;
        Pos2::new(rect.right() - 20.0, rect.top() + 180.0)
    }

    fn on_time_axis(&self, chart: usize) -> Pos2 {
        let rect = self.members[chart].rect;
        Pos2::new(rect.left() + rect.width() * 0.4, rect.bottom() - 6.0)
    }

    fn view(&self, chart: usize) -> HorizontalViewport {
        self.members[chart].card.horizontal()
    }

    fn set_view(&mut self, chart: usize, visible: usize, right_offset: f64) {
        self.members[chart].card.viewport.horizontal = HorizontalViewport {
            visible,
            right_offset,
        };
    }

    fn range(&self, chart: usize) -> crate::model::chart_sync::TimeRange {
        let member = &self.members[chart];
        TimeAxis::new(&member.candles)
            .range_of(&member.card.horizontal())
            .expect("bars")
    }

    fn slot_width(&self, chart: usize) -> f32 {
        self.members[chart].card.probe.expect("drawn").slot_width
    }

    fn vertical_is_default(&self, chart: usize) -> bool {
        self.members[chart].card.viewport.vertical.is_default()
    }
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

// MARK: Position

#[test]
fn dragging_a_chart_moves_the_others_to_the_same_time_and_keeps_their_zoom() {
    let mut linked = Linked::pair(POSITION);
    linked.set_view(1, 300, 0.0);
    let own_zoom = linked.view(1).visible;
    linked.drag(linked.in_plot(0), Vec2::new(150.0, 0.0));

    assert!(linked.view(0).right_offset > 0.0, "the dragged chart moved");
    assert_eq!(linked.range(0).right, linked.range(1).right);
    assert_eq!(linked.view(1).visible, own_zoom, "zoom stays its own");
    assert_eq!(linked.view(0).visible, DEFAULT_VISIBLE);
}

#[test]
fn any_chart_can_lead() {
    let mut linked = Linked::pair(POSITION);
    linked.drag(linked.in_plot(0), Vec2::new(150.0, 0.0));
    let after_first = linked.view(0);
    linked.drag(linked.in_plot(1), Vec2::new(-60.0, 0.0));

    assert!(linked.view(1).right_offset < after_first.right_offset);
    assert_eq!(linked.range(0).right, linked.range(1).right);
    assert_ne!(
        linked.view(0),
        after_first,
        "the first chart followed the second"
    );
}

#[test]
fn dragging_right_shows_older_history_everywhere_and_left_newer() {
    let mut linked = Linked::pair(POSITION);
    linked.drag(linked.in_plot(0), Vec2::new(200.0, 0.0));
    let back = linked.view(1).right_offset;
    assert!(back > 0.0);
    linked.drag(linked.in_plot(0), Vec2::new(-80.0, 0.0));
    assert!(linked.view(1).right_offset < back);
}

#[test]
fn followers_move_while_the_drag_is_under_way_in_the_same_frame() {
    let mut linked = Linked::pair(POSITION);
    let from = linked.in_plot(0);
    linked.frame(vec![egui::Event::PointerMoved(from)]);
    linked.frame(vec![Linked::button(from, true)]);
    let mut previous = linked.view(1).right_offset;
    for step in 1..=8 {
        let at = from + Vec2::new(25.0 * step as f32, 0.0);
        linked.frame(vec![egui::Event::PointerMoved(at)]);
        // The follower is drawn after the leader, so one frame is enough: it never trails.
        assert_eq!(
            linked.range(0).right,
            linked.range(1).right,
            "step {step}: the charts disagree"
        );
        let now = linked.view(1).right_offset;
        assert!(now >= previous, "the follower turned back");
        previous = now;
    }
    assert!(previous > 0.0);
}

#[test]
fn a_chart_drawn_before_the_leader_catches_up_with_the_next_frame() {
    let mut linked = Linked::pair(POSITION);
    linked.drag(linked.in_plot(1), Vec2::new(120.0, 0.0));
    assert_eq!(linked.range(0).right, linked.range(1).right);
}

#[test]
fn with_position_off_charts_pan_alone() {
    let mut linked = Linked::pair(ZOOM);
    linked.drag(linked.in_plot(0), Vec2::new(150.0, 0.0));
    assert!(linked.view(0).right_offset > 0.0);
    assert_eq!(linked.view(1).right_offset, 0.0);

    let mut off = Linked::pair(ChartSyncSettings::UNLINKED);
    off.drag(off.in_plot(0), Vec2::new(150.0, 0.0));
    assert_eq!(off.view(1), HorizontalViewport::default());
}

#[test]
fn a_dragged_chart_is_not_moved_by_the_others() {
    let mut linked = Linked::pair(POSITION);
    let from = linked.in_plot(1);
    linked.frame(vec![egui::Event::PointerMoved(from)]);
    linked.frame(vec![Linked::button(from, true)]);
    for step in 1..=5 {
        linked.frame(vec![egui::Event::PointerMoved(
            from + Vec2::new(30.0 * step as f32, 0.0),
        )]);
    }
    let held = linked.view(1).right_offset;
    // Frames with no new input must not send the leader's own state back to it.
    linked.frames(5);
    assert_eq!(linked.view(1).right_offset, held);
}

// MARK: Zoom

#[test]
fn zooming_with_the_wheel_zooms_the_others_and_keeps_their_position() {
    let mut linked = Linked::pair(ZOOM);
    linked.set_view(1, 200, 25.0);
    linked.frames(2);
    linked.wheel(linked.in_plot(0), Vec2::new(0.0, 80.0));

    let leader = linked.view(0).visible;
    assert_ne!(leader, DEFAULT_VISIBLE, "the wheel zoomed the chart");
    let expected = (200.0 * leader as f64 / DEFAULT_VISIBLE as f64).round() as usize;
    assert!(
        linked.view(1).visible.abs_diff(expected) <= 1,
        "{} bars, expected about {expected}",
        linked.view(1).visible
    );
    assert_eq!(
        linked.view(1).right_offset,
        25.0,
        "no jump to the leader's date"
    );
}

#[test]
fn dragging_the_time_axis_zooms_the_others_too() {
    let mut linked = Linked::pair(ZOOM);
    linked.drag(linked.on_time_axis(0), Vec2::new(80.0, 0.0));
    assert!(linked.view(0).visible < DEFAULT_VISIBLE);
    assert!(linked.view(1).visible < DEFAULT_VISIBLE);
    assert_eq!(linked.view(1).right_offset, 0.0);
}

#[test]
fn zoom_sync_off_leaves_zoom_alone() {
    let mut linked = Linked::pair(POSITION);
    linked.wheel(linked.in_plot(0), Vec2::new(0.0, 80.0));
    assert_ne!(linked.view(0).visible, DEFAULT_VISIBLE);
    assert_eq!(linked.view(1).visible, DEFAULT_VISIBLE);
}

// MARK: Both

#[test]
fn both_linked_the_charts_show_the_same_time_range_whatever_their_width() {
    let mut linked = Linked::pair(BOTH);
    linked.set_view(1, 300, 90.0);
    linked.drag(linked.in_plot(0), Vec2::new(100.0, 0.0));
    linked.wheel(linked.in_plot(0), Vec2::new(0.0, 100.0));

    let (a, b) = (linked.range(0), linked.range(1));
    assert_eq!(a.right, b.right);
    // Equal up to the bar the follower's whole-bar count rounds to.
    assert!(near(a.left, b.left, HOUR as f64));
    assert_eq!(linked.view(0).visible, linked.view(1).visible);
}

#[test]
fn both_linked_equal_charts_have_the_same_perceived_zoom() {
    // Same bars and the same width: matching the range must also match how the bars look.
    let bars = series(T0, HOUR, 800);
    let mut linked = Linked::new(
        BOTH,
        vec![
            Member::new(bars.clone(), 0.0, 560.0),
            Member::new(bars, 570.0, 560.0),
        ],
    );
    linked.wheel(linked.in_plot(0), Vec2::new(0.0, 120.0));
    linked.drag(linked.in_plot(0), Vec2::new(90.0, 0.0));
    linked.frames(2);
    assert!(
        (linked.slot_width(0) - linked.slot_width(1)).abs() < 0.01,
        "{} against {}",
        linked.slot_width(0),
        linked.slot_width(1)
    );
}

#[test]
fn both_linked_a_narrower_chart_shows_the_same_bars_thinner() {
    let mut linked = Linked::pair(BOTH);
    linked.wheel(linked.in_plot(0), Vec2::new(0.0, 100.0));
    linked.frames(2);
    let ratio = linked.slot_width(1) / linked.slot_width(0);
    let widths = 420.0 / 700.0;
    assert!(
        (f64::from(ratio) - widths).abs() < 0.08,
        "bar widths follow the plots: {ratio}"
    );
}

#[test]
fn charts_of_different_timeframes_show_the_same_calendar_period() {
    let hourly = series(T0, HOUR, 400);
    let quarters = series(T0, 15 * MINUTE, 1600);
    let mut linked = Linked::new(
        BOTH,
        vec![
            Member::new(hourly, 0.0, 700.0),
            Member::new(quarters, 710.0, 420.0),
        ],
    );
    linked.drag(linked.in_plot(0), Vec2::new(120.0, 0.0));
    linked.wheel(linked.in_plot(0), Vec2::new(0.0, 60.0));

    let (a, b) = (linked.range(0), linked.range(1));
    assert_eq!(a.right, b.right);
    assert!(near(a.left, b.left, 15.0 * MINUTE as f64));
    assert!(
        linked.view(1).visible > linked.view(0).visible * 3,
        "four times the bars for a quarter of the interval"
    );
}

// MARK: Vertical

#[test]
fn vertical_dragging_stays_on_its_own_chart() {
    let mut linked = Linked::pair(BOTH);
    linked.drag(linked.in_plot(0), Vec2::new(0.0, 80.0));
    assert!(!linked.vertical_is_default(0));
    assert!(linked.vertical_is_default(1));
    assert_eq!(linked.view(1), HorizontalViewport::default());
}

#[test]
fn a_diagonal_drag_carries_only_its_horizontal_part() {
    let mut linked = Linked::pair(BOTH);
    linked.drag(linked.in_plot(0), Vec2::new(120.0, 70.0));
    assert!(
        linked.view(1).right_offset > 0.0,
        "the horizontal part followed"
    );
    assert!(!linked.vertical_is_default(0));
    assert!(linked.vertical_is_default(1), "the vertical part did not");
}

#[test]
fn the_price_axis_never_links() {
    let mut linked = Linked::pair(BOTH);
    linked.drag(linked.on_price_axis(1), Vec2::new(0.0, -80.0));
    assert!(linked.members[1].card.viewport.vertical.zoom > 1.0);
    assert!(linked.vertical_is_default(0));
    assert_eq!(linked.view(0), HorizontalViewport::default());
    assert_eq!(linked.view(1), HorizontalViewport::default());
}

// MARK: Loops, live data and eligibility

#[test]
fn followers_do_not_echo_and_the_sequence_stops_with_the_user() {
    let mut linked = Linked::pair(BOTH);
    linked.drag(linked.in_plot(0), Vec2::new(150.0, 0.0));
    let settled = linked.hub.latest_event().expect("an event").sequence;
    linked.frames(20);
    assert_eq!(
        linked.hub.latest_event().expect("an event").sequence,
        settled,
        "nothing published without input"
    );
    assert_eq!(linked.hub.latest_event().expect("an event").origin, 1);
}

#[test]
fn a_new_bar_is_not_navigation() {
    let mut linked = Linked::pair(BOTH);
    linked.drag(linked.in_plot(0), Vec2::new(150.0, 0.0));
    let settled = linked.hub.latest_event().expect("an event").sequence;
    let (view_a, view_b) = (linked.view(0), linked.view(1));
    for member in &mut linked.members {
        let last = member.candles.last().expect("bars").clone();
        member
            .candles
            .push(series(last.open_time + HOUR, HOUR, 1).remove(0));
    }
    linked.frames(3);
    assert_eq!(
        linked.hub.latest_event().expect("an event").sequence,
        settled
    );
    // The scrolled-back charts keep their bars on screen, as they always did.
    assert_eq!(linked.view(0).right_offset, view_a.right_offset + 1.0);
    assert_eq!(linked.view(1).right_offset, view_b.right_offset + 1.0);
}

#[test]
fn a_chart_following_the_newest_bar_does_not_drag_the_others() {
    let mut linked = Linked::pair(BOTH);
    linked.set_view(1, DEFAULT_VISIBLE, 40.0);
    linked.frames(2);
    for member in &mut linked.members {
        let last = member.candles.last().expect("bars").clone();
        member
            .candles
            .push(series(last.open_time + HOUR, HOUR, 1).remove(0));
    }
    linked.frames(3);
    assert!(linked.hub.latest_event().is_none());
    assert_eq!(linked.view(0).right_offset, 0.0);
}

#[test]
fn a_price_based_chart_neither_leads_nor_follows() {
    let bars = series(T0, HOUR, 800);
    let mut linked = Linked::new(
        BOTH,
        vec![
            Member::new(bars.clone(), 0.0, 560.0),
            Member::new(bars.clone(), 570.0, 560.0),
        ],
    );
    linked.members[1].card.appearance_mut().style.chart_type = ChartType::Renko;
    assert!(
        !linked.members[1]
            .card
            .takes_part_in_sync(DataSource::Binance)
    );
    assert!(
        linked.members[0]
            .card
            .takes_part_in_sync(DataSource::Binance)
    );

    linked.drag(linked.in_plot(0), Vec2::new(150.0, 0.0));
    assert!(linked.view(0).right_offset > 0.0);
    assert_eq!(linked.view(1).right_offset, 0.0, "it did not follow");

    let before = linked.hub.latest_event().expect("an event").sequence;
    linked.drag(linked.in_plot(1), Vec2::new(-60.0, 0.0));
    assert_eq!(
        linked.hub.latest_event().expect("an event").sequence,
        before,
        "and it did not lead"
    );
}

#[test]
fn a_chart_with_less_history_follows_as_far_as_it_can() {
    let long = series(T0, HOUR, 2000);
    let short = series(T0 + 1900 * HOUR, HOUR, 100);
    let mut linked = Linked::new(
        POSITION,
        vec![
            Member::new(long, 0.0, 700.0),
            Member::new(short, 710.0, 420.0),
        ],
    );
    // Far back, before the short chart begins.
    linked.drag(linked.in_plot(0), Vec2::new(700.0, 0.0));
    linked.drag(linked.in_plot(0), Vec2::new(700.0, 0.0));
    assert!(linked.view(0).right_offset > 100.0);
    assert_eq!(
        linked.view(1).right_offset,
        80.0,
        "held at its oldest bars, with a screenful of them still in view"
    );
    // The leader was not pushed back to where the follower's data ends.
    assert!(linked.view(0).right_offset > 100.0);
}

/// Frame cost of dragging one chart of a linked layout, against the same layout unlinked:
/// `cargo test --release --lib linked_drag_frame_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn linked_drag_frame_cost_report() {
    for (charts, count) in [(2, 10_000), (4, 10_000), (4, 100_000)] {
        for settings in [ChartSyncSettings::UNLINKED, BOTH] {
            let bars = series(T0, HOUR, count);
            let width = 1100.0 / charts as f32;
            let mut linked = Linked::new(
                settings,
                (0..charts)
                    .map(|index| Member::new(bars.clone(), index as f32 * width, width - 6.0))
                    .collect(),
            );
            linked.frames(3);
            let from = linked.in_plot(0);
            linked.frame(vec![egui::Event::PointerMoved(from)]);
            linked.frame(vec![Linked::button(from, true)]);
            let frames = 60;
            let started = std::time::Instant::now();
            for step in 1..=frames {
                let at = from + Vec2::new(step as f32 * 2.0, step as f32 * 0.3);
                linked.frame(vec![egui::Event::PointerMoved(at)]);
            }
            let per_frame = started.elapsed() / frames;
            let events = linked.hub.latest_event().map_or(0, |event| event.sequence);
            println!(
                "{charts} charts, {count:>7} candles, {}: {per_frame:?} per drag frame, {events} events for {frames} frames",
                if settings.is_linked() {
                    "linked  "
                } else {
                    "unlinked"
                }
            );
        }
    }
}
