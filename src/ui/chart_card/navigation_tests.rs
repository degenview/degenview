//! Navigation through real pointer events: that a drag follows the pointer, that each part of the
//! chart answers its own gesture, and that moving the view never edits a drawing or the data.

use super::*;
use crate::model::viewport::{DEFAULT_VISIBLE, MIN_VISIBLE};
use crate::model::{Anchor, Drawing, IndicatorConfig, Tool};
use crate::ui::replay_overlay::ReplayChartFrame;
use eframe::egui::Modifiers;

const HOUR: i64 = 3_600_000;
const SIZE: Vec2 = Vec2::new(760.0, 520.0);

fn series(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let close = 100.0 + (index as f64 * 0.7).sin() * 20.0 + index as f64 * 0.5;
            Kline {
                open_time: index as i64 * HOUR,
                close_time: (index as i64 + 1) * HOUR - 1,
                open: close - 1.0,
                high: close + 2.0,
                low: close - 3.0,
                close,
                volume: 10.0 + index as f64,
            }
        })
        .collect()
}

/// A chart on a window that takes pointer events.
struct Rig {
    ctx: egui::Context,
    card: ChartCard,
    candles: Vec<Kline>,
    drawings: Vec<Drawing>,
    scale_mode: PriceScaleMode,
    tool: Tool,
    selected: Vec<u64>,
    time: f64,
    rect: Rect,
    commands: usize,
    /// The cursor the last frame asked for.
    cursor: egui::CursorIcon,
}

impl Rig {
    fn new() -> Self {
        let mut card = ChartCard::default();
        card.set_timeframe(Timeframe::Hour);
        let mut rig = Self {
            ctx: egui::Context::default(),
            card,
            candles: series(400),
            drawings: Vec::new(),
            scale_mode: PriceScaleMode::Regular,
            tool: Tool::Cursor,
            selected: Vec::new(),
            time: 0.0,
            rect: Rect::NOTHING,
            commands: 0,
            cursor: egui::CursorIcon::Default,
        };
        rig.frames(&[]);
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.016;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (card, candles, drawings, scale_mode) = (
            &mut self.card,
            &self.candles,
            &self.drawings,
            self.scale_mode,
        );
        let (tool, selected) = (self.tool, self.selected.clone());
        let (mut rect, mut commands) = (Rect::NOTHING, 0);
        let mut output = self.ctx.run_ui(input, |ui| {
            let mut frame =
                DrawingFrame::new(drawings, tool, None, 100).with_selection(selected.clone());
            let mut replay = ReplayChartFrame {
                selecting: false,
                active: false,
                picked_ms: None,
            };
            let pointer = card.body(
                ui,
                "BTCUSDT",
                candles,
                false,
                None,
                scale_mode,
                DataSource::Binance,
                &IndicatorConfig::default(),
                None,
                1,
                &mut frame,
                &mut replay,
            );
            rect = pointer.rect.unwrap_or(Rect::NOTHING);
            commands = frame.commands.len();
        });
        output.textures_delta.clear();
        self.cursor = output.platform_output.cursor_icon;
        self.rect = rect;
        self.commands += commands;
    }

    fn frames(&mut self, events: &[egui::Event]) {
        for _ in 0..3 {
            self.frame(Vec::new());
        }
        self.frame(events.to_vec());
    }

    fn button(&self, pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn press(&mut self, at: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![self.button(at, true)]);
    }

    /// Moves the held pointer to `to` over a few frames, as a hand does.
    fn drag_to(&mut self, from: Pos2, to: Pos2) {
        for step in 1..=6 {
            let at = from + (to - from) * (step as f32 / 6.0);
            self.frame(vec![egui::Event::PointerMoved(at)]);
        }
    }

    fn release(&mut self, at: Pos2) {
        self.frame(vec![self.button(at, false)]);
        self.frame(Vec::new());
    }

    fn drag(&mut self, from: Pos2, delta: Vec2) {
        self.press(from);
        self.drag_to(from, from + delta);
        self.release(from + delta);
    }

    fn in_plot(&self) -> Pos2 {
        Pos2::new(self.rect.left() + 300.0, self.rect.top() + 180.0)
    }

    fn on_price_axis(&self) -> Pos2 {
        Pos2::new(self.rect.right() - 20.0, self.rect.top() + 180.0)
    }

    fn on_time_axis(&self) -> Pos2 {
        Pos2::new(self.rect.left() + 300.0, self.rect.bottom() - 6.0)
    }

    fn offset(&self) -> f64 {
        self.card.viewport.horizontal.right_offset
    }

    fn probe(&self) -> Probe {
        self.card.probe.expect("the chart was drawn")
    }

    fn slot_width(&self) -> f32 {
        self.probe().slot_width
    }

    /// The x a bar is drawn at.
    fn x_of(&self, index: usize) -> f32 {
        let probe = self.probe();
        probe.plot.left()
            + probe.slot_width * (index as f32 - probe.start as f32 + 0.5 + probe.lead)
    }

    /// The y a price is drawn at on a linear scale.
    fn y_of(&self, price: f64) -> f32 {
        let probe = self.probe();
        let (low, high) = (probe.minimum, probe.maximum);
        probe.plot.top() + ((high - price) / (high - low)) as f32 * probe.plot.height()
    }

    fn range(&self) -> (f64, f64) {
        let probe = self.probe();
        (probe.minimum, probe.maximum)
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn dragging_right_reveals_older_bars_following_the_pointer_exactly() {
    let mut rig = Rig::new();
    let from = rig.in_plot();
    rig.drag(from, Vec2::new(100.0, 0.0));
    let slot = rig.slot_width();
    assert!(
        close(rig.offset(), f64::from(100.0 / slot)),
        "{} bars for 100 points of {slot}",
        rig.offset()
    );
    assert!(rig.offset().fract() != 0.0, "a fraction of a bar is kept");
    assert!(rig.card.viewport.vertical.is_auto());
}

#[test]
fn dragging_left_moves_toward_newer_bars_and_then_into_empty_space() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 50.0;
    let from = rig.in_plot();
    rig.drag(from, Vec2::new(-100.0, 0.0));
    let slot = rig.slot_width();
    assert!(close(rig.offset(), 50.0 - f64::from(100.0 / slot)));

    rig.card.viewport.horizontal.right_offset = 0.0;
    rig.drag(from, Vec2::new(-150.0, 0.0));
    assert!(rig.offset() < 0.0, "past the newest bar: {}", rig.offset());
    assert!(!rig.card.viewport.horizontal.follows_latest());
}

#[test]
fn whitespace_past_the_newest_bar_is_bounded() {
    let mut rig = Rig::new();
    let from = Pos2::new(rig.rect.left() + 500.0, rig.rect.top() + 180.0);
    for _ in 0..6 {
        rig.drag(from, Vec2::new(-480.0, 0.0));
    }
    assert!(close(
        rig.offset(),
        -((DEFAULT_VISIBLE - MIN_VISIBLE) as f64)
    ));
}

#[test]
fn a_drag_has_no_dead_zone_and_no_jump_on_release() {
    let mut rig = Rig::new();
    let from = rig.in_plot();
    rig.press(from);
    rig.drag_to(from, from + Vec2::new(40.0, 0.0));
    let held = rig.offset();
    rig.release(from + Vec2::new(40.0, 0.0));
    assert_eq!(rig.offset(), held, "nothing moves on release");
    let slot = rig.slot_width();
    assert!(
        close(held, f64::from(40.0 / slot)),
        "the whole 40 points count, not the distance after a threshold: {held}"
    );
}

#[test]
fn a_vertical_drag_moves_the_prices_and_turns_autoscale_off() {
    let mut rig = Rig::new();
    let before = rig.range();
    let from = rig.in_plot();
    rig.drag(from, Vec2::new(0.0, 60.0));
    assert!(!rig.card.viewport.vertical.is_auto());
    rig.frames(&[]);
    let after = rig.range();
    assert!(after.0 > before.0 && after.1 > before.1);
    assert!(
        close(after.1 - after.0, before.1 - before.0),
        "the span is kept"
    );
    // The range stays where it was put as more frames pass and the data does not move.
    rig.frames(&[]);
    assert_eq!(rig.range().0, after.0);
}

#[test]
fn a_horizontal_drag_leaves_autoscale_on() {
    let mut rig = Rig::new();
    let from = rig.in_plot();
    rig.press(from);
    rig.drag_to(from, from + Vec2::new(120.0, 2.0));
    rig.release(from + Vec2::new(120.0, 2.0));
    assert!(rig.card.viewport.vertical.is_auto());
    assert!(rig.offset() > 0.0);
}

#[test]
fn a_diagonal_drag_moves_both_ways() {
    let mut rig = Rig::new();
    let from = rig.in_plot();
    rig.drag(from, Vec2::new(-60.0, -50.0));
    assert!(rig.offset() < 0.0);
    assert!(!rig.card.viewport.vertical.is_auto());
}

#[test]
fn the_price_axis_stretches_the_scale_and_never_pans_the_time() {
    let mut rig = Rig::new();
    let from = rig.on_price_axis();
    rig.drag(from, Vec2::new(-40.0, -60.0));
    assert!(
        rig.card.viewport.vertical.zoom > 1.0,
        "dragging up zooms in"
    );
    assert_eq!(rig.offset(), 0.0, "an axis drag does not pan sideways");
}

#[test]
fn the_time_axis_stretches_the_bars_and_leaves_the_prices() {
    let mut rig = Rig::new();
    let from = rig.on_time_axis();
    rig.drag(from, Vec2::new(80.0, 30.0));
    assert!(rig.card.viewport.horizontal.visible < DEFAULT_VISIBLE);
    assert!(rig.card.viewport.vertical.is_default());
    assert_eq!(rig.offset(), 0.0);
}

#[test]
fn double_clicking_the_price_axis_restores_autoscale() {
    let mut rig = Rig::new();
    rig.card.viewport.vertical.zoom = 3.0;
    rig.card.viewport.vertical.manual = Some(PriceRange::new(10.0, 20.0));
    rig.card.viewport.horizontal.right_offset = 9.0;
    let at = rig.on_price_axis();
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    for _ in 0..2 {
        rig.frame(vec![rig.button(at, true)]);
        rig.frame(vec![rig.button(at, false)]);
    }
    assert!(rig.card.viewport.vertical.is_default());
    assert_eq!(rig.offset(), 9.0, "the time position stays");
}

#[test]
fn double_clicking_the_time_axis_returns_to_the_newest_bars() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 90.0;
    rig.card.viewport.horizontal.visible = 40;
    rig.card.viewport.vertical.zoom = 2.0;
    let at = rig.on_time_axis();
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    for _ in 0..2 {
        rig.frame(vec![rig.button(at, true)]);
        rig.frame(vec![rig.button(at, false)]);
    }
    assert!(rig.card.viewport.horizontal == Default::default());
    assert_eq!(rig.card.viewport.vertical.zoom, 2.0, "prices stay");
}

#[test]
fn escape_puts_the_chart_back_where_the_press_found_it() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 12.0;
    let from = rig.in_plot();
    rig.press(from);
    rig.drag_to(from, from + Vec2::new(90.0, 70.0));
    assert!(rig.offset() > 12.0);
    rig.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(rig.offset(), 12.0);
    assert!(rig.card.viewport.vertical.is_auto());
    // The rest of the drag does nothing.
    rig.drag_to(from + Vec2::new(90.0, 70.0), from + Vec2::new(150.0, 70.0));
    assert_eq!(rig.offset(), 12.0);
    rig.release(from + Vec2::new(150.0, 70.0));
    assert!(rig.card.gesture.is_none());
}

#[test]
fn a_drag_released_outside_the_chart_ends() {
    let mut rig = Rig::new();
    let from = rig.in_plot();
    rig.press(from);
    rig.drag_to(from, Pos2::new(SIZE.x + 100.0, from.y));
    rig.release(Pos2::new(SIZE.x + 100.0, from.y));
    assert!(rig.card.gesture.is_none());
    let held = rig.offset();
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    assert_eq!(rig.offset(), held, "no stuck drag follows the pointer back");
}

#[test]
fn a_click_without_movement_changes_nothing() {
    let mut rig = Rig::new();
    let at = rig.in_plot();
    rig.press(at);
    rig.release(at);
    assert_eq!(rig.card.viewport, ChartViewport::default());
}

#[test]
fn the_wheel_zooms_and_a_sideways_swipe_scrolls() {
    let mut rig = Rig::new();
    let at = rig.in_plot();
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    rig.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, 40.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }]);
    rig.frames(&[]);
    assert_ne!(rig.card.viewport.horizontal.visible, DEFAULT_VISIBLE);
    let visible = rig.card.viewport.horizontal.visible;
    rig.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(60.0, 3.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }]);
    rig.frames(&[]);
    assert!(rig.offset() > 0.0, "swiping right shows older bars");
    assert_eq!(rig.card.viewport.horizontal.visible, visible);
}

// MARK: Drawings and data

fn trend_line() -> Drawing {
    let anchors = [
        Anchor {
            time_ms: 330 * HOUR,
            price: 250.0,
        },
        Anchor {
            time_ms: 380 * HOUR,
            price: 280.0,
        },
    ];
    Drawing::build(
        Tool::TrendLine,
        7,
        &anchors,
        Tool::TrendLine.default_style(),
    )
    .expect("builds")
}

#[test]
fn panning_never_rewrites_a_drawing_or_makes_an_undo_step() {
    let mut rig = Rig::new();
    rig.drawings = vec![trend_line()];
    let before = rig.drawings.clone();
    let candles = rig.candles.clone();
    rig.frames(&[]);
    let from = Pos2::new(rig.rect.left() + 120.0, rig.rect.top() + 60.0);
    rig.drag(from, Vec2::new(150.0, 90.0));
    rig.drag(from, Vec2::new(-210.0, -40.0));
    rig.drag(rig.on_price_axis(), Vec2::new(0.0, -50.0));
    assert_eq!(
        rig.drawings, before,
        "the anchors are financial coordinates"
    );
    assert_eq!(rig.candles, candles, "the data is the data");
    assert_eq!(rig.commands, 0, "no drawing command, so no undo entry");
}

#[test]
fn a_bar_moves_with_the_pointer_on_screen() {
    let mut rig = Rig::new();
    rig.drawings = vec![trend_line()];
    rig.frames(&[]);
    let first = rig.x_of(380);
    let from = Pos2::new(rig.rect.left() + 120.0, rig.rect.top() + 60.0);
    rig.drag(from, Vec2::new(-80.0, 0.0));
    rig.frames(&[]);
    let moved = rig.x_of(380);
    assert!(
        close(f64::from(moved - first), -80.0),
        "the bar the line ends on followed the pointer by 80 points: {first} to {moved}"
    );
}

/// The screen point of the trend line's first anchor.
fn line_start(rig: &Rig) -> Pos2 {
    Pos2::new(rig.x_of(330), rig.y_of(250.0))
}

#[test]
fn a_drag_that_takes_a_drawing_handle_edits_it_and_does_not_pan() {
    let mut rig = Rig::new();
    rig.drawings = vec![trend_line()];
    rig.selected = vec![7];
    rig.frames(&[]);
    let handle = line_start(&rig);
    rig.drag(handle, Vec2::new(-40.0, 25.0));
    assert_eq!(
        rig.card.viewport,
        ChartViewport::default(),
        "the view stayed"
    );
    assert!(rig.commands > 0, "the drawing was edited");
}

#[test]
fn a_drag_over_a_locked_drawing_pans_the_chart() {
    let mut rig = Rig::new();
    let mut line = trend_line();
    line.set_locked(true);
    rig.drawings = vec![line];
    rig.selected = vec![7];
    rig.frames(&[]);
    let handle = line_start(&rig);
    rig.drag(handle, Vec2::new(-90.0, 0.0));
    assert!(
        rig.offset() < 0.0,
        "a locked drawing does not hold the chart"
    );
    assert_eq!(rig.commands, 0, "and is not edited");
}

#[test]
fn a_drawing_tool_draws_and_never_pans() {
    let mut rig = Rig::new();
    rig.tool = Tool::TrendLine;
    rig.frames(&[]);
    let from = rig.in_plot();
    rig.drag(from, Vec2::new(-120.0, 40.0));
    assert_eq!(rig.card.viewport, ChartViewport::default());
    assert!(rig.commands > 0, "the line was committed");
}

#[test]
fn a_saved_chart_comes_back_scrolled_and_a_template_does_not_move_it() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 33.5;
    rig.card.viewport.vertical.manual = Some(PriceRange::new(90.0, 190.0));
    let saved = rig.card.saved_viewport();

    let mut reopened = ChartCard::default();
    reopened.restore_viewport(saved);
    assert_eq!(reopened.viewport.horizontal.right_offset, 33.5);
    assert_eq!(
        reopened.viewport.vertical.manual,
        Some(PriceRange::new(90.0, 190.0))
    );

    // Appearance is what a template carries: applying one leaves the position alone.
    let template = ChartCard::default().appearance();
    rig.card.set_appearance(template);
    assert_eq!(rig.card.saved_viewport(), saved);
}

// MARK: Live data and history

fn kline_at(hour: i64) -> Kline {
    series(1)
        .pop()
        .map(|bar| Kline {
            open_time: hour * HOUR,
            close_time: (hour + 1) * HOUR - 1,
            ..bar
        })
        .expect("a bar")
}

/// The x a bar is drawn at, found by its open time.
fn x_at_time(rig: &Rig, hour: i64) -> f32 {
    let index = rig
        .candles
        .iter()
        .position(|bar| bar.open_time == hour * HOUR)
        .expect("the bar is loaded");
    rig.x_of(index)
}

#[test]
fn a_new_bar_does_not_move_a_chart_scrolled_back() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 25.0;
    rig.frames(&[]);
    let before = x_at_time(&rig, 350);
    for hour in 400..403 {
        rig.candles.push(kline_at(hour));
        rig.frames(&[]);
    }
    assert_eq!(rig.offset(), 28.0, "it counts the three bars that came");
    assert!(close(f64::from(x_at_time(&rig, 350)), f64::from(before)));
}

#[test]
fn a_chart_held_on_the_newest_bar_follows_it() {
    let mut rig = Rig::new();
    rig.frames(&[]);
    rig.candles.push(kline_at(400));
    rig.frames(&[]);
    assert_eq!(rig.offset(), 0.0);
    assert!(rig.card.viewport.horizontal.follows_latest());
}

#[test]
fn a_chart_scrolled_into_empty_space_keeps_its_bars_too() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = -10.0;
    rig.frames(&[]);
    let before = x_at_time(&rig, 399);
    rig.candles.push(kline_at(400));
    rig.frames(&[]);
    assert_eq!(rig.offset(), -9.0, "one slot of the space became a bar");
    assert!(close(f64::from(x_at_time(&rig, 399)), f64::from(before)));
}

#[test]
fn older_bars_loading_do_not_move_the_view() {
    let mut rig = Rig::new();
    rig.candles = series(400).split_off(100);
    rig.card.viewport.horizontal.right_offset = 150.0;
    rig.frames(&[]);
    let before = x_at_time(&rig, 200);
    let mut older: Vec<Kline> = (0..100).map(kline_at).collect();
    older.append(&mut rig.candles);
    rig.candles = older;
    rig.frames(&[]);
    assert_eq!(rig.offset(), 150.0);
    assert!(
        close(f64::from(x_at_time(&rig, 200)), f64::from(before)),
        "the same bar stays where it was"
    );
}

#[test]
fn a_new_bar_during_a_drag_does_not_make_the_chart_jump() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 20.0;
    rig.frames(&[]);
    let from = rig.in_plot();
    rig.press(from);
    rig.drag_to(from, from + Vec2::new(60.0, 0.0));
    let before = rig.offset();
    rig.candles.push(kline_at(400));
    rig.frame(vec![egui::Event::PointerMoved(from + Vec2::new(60.0, 0.0))]);
    assert_eq!(rig.offset(), before + 1.0);
    rig.release(from + Vec2::new(60.0, 0.0));
}

// MARK: Scripts

const PLAIN_SCRIPT: &str = "//@version=6\nindicator(\"Plain\", overlay = true)\nplot(close)\n";
const RANGE_SCRIPT: &str =
    "//@version=6\nindicator(\"Range\", overlay = true)\nplot(chart.left_visible_bar_time)\n";

fn wait_for_script(rig: &mut Rig, id: u64) {
    for _ in 0..200 {
        rig.frame(Vec::new());
        if rig.card.pine_status(id).is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("Pine script {id} did not finish");
}

fn generation(rig: &Rig, id: u64) -> u64 {
    rig.card.runs.get(&id).expect("the script ran").generation
}

#[test]
fn panning_runs_a_script_again_only_if_it_reads_the_visible_range() {
    let mut rig = Rig::new();
    let plain = rig.card.add_pine_instance(None, "Plain", PLAIN_SCRIPT);
    let ranged = rig.card.add_pine_instance(None, "Range", RANGE_SCRIPT);
    wait_for_script(&mut rig, plain);
    wait_for_script(&mut rig, ranged);
    let (plain_before, ranged_before) = (generation(&rig, plain), generation(&rig, ranged));

    let from = rig.in_plot();
    rig.drag(from, Vec2::new(200.0, 40.0));
    rig.frames(&[]);

    assert_eq!(generation(&rig, plain), plain_before, "no rerun for a pan");
    assert!(
        generation(&rig, ranged) > ranged_before,
        "a script that reads chart.left_visible_bar_time follows the view"
    );
}

#[test]
fn the_chart_says_when_the_view_is_near_the_oldest_bar() {
    let mut rig = Rig::new();
    rig.frames(&[]);
    assert!(
        !rig.card.near_oldest_bar(),
        "400 bars held, the newest 120 shown"
    );
    rig.card.viewport.horizontal.right_offset = 400.0 - 120.0 - 10.0;
    rig.frames(&[]);
    assert!(rig.card.near_oldest_bar());
}

#[test]
fn the_cursor_is_a_crosshair_over_the_plot_and_a_hand_while_dragging() {
    let mut rig = Rig::new();
    let cursor = |rig: &Rig| rig.cursor;
    let from = rig.in_plot();
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.frame(Vec::new());
    assert_eq!(cursor(&rig), egui::CursorIcon::Crosshair);
    rig.press(from);
    rig.drag_to(from, from + Vec2::new(60.0, 0.0));
    assert_eq!(cursor(&rig), egui::CursorIcon::Grabbing);
    rig.release(from + Vec2::new(60.0, 0.0));
    assert_eq!(cursor(&rig), egui::CursorIcon::Crosshair);
}

// MARK: Cost

/// Prints what a frame of a drag costs over long histories. Run with
/// `cargo test --release --lib drag_frame_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn drag_frame_cost_report() {
    for count in [100, 10_000, 100_000] {
        let mut rig = Rig::new();
        rig.candles = series(count);
        rig.drawings = (0..200)
            .map(|index| {
                let anchors = [
                    Anchor {
                        time_ms: (count as i64 - 60 + index % 40) * HOUR,
                        price: 100.0 + index as f64,
                    },
                    Anchor {
                        time_ms: (count as i64 - 10) * HOUR,
                        price: 120.0 + index as f64,
                    },
                ];
                Drawing::build(
                    Tool::TrendLine,
                    index as u64 + 1,
                    &anchors,
                    Tool::TrendLine.default_style(),
                )
                .expect("builds")
            })
            .collect();
        rig.frames(&[]);
        let from = rig.in_plot();
        rig.press(from);
        let frames = 60;
        let started = std::time::Instant::now();
        for step in 1..=frames {
            let at = from + Vec2::new(step as f32 * 3.0, step as f32 * 0.5);
            rig.frame(vec![egui::Event::PointerMoved(at)]);
        }
        let per_frame = started.elapsed() / frames;
        println!("{count:>7} candles, 200 drawings: {per_frame:?} per drag frame");
        rig.release(from);
    }
}

// MARK: The price axis fits its labels

fn level(count: usize, base: f64) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let close = base * (1.0 + (index as f64 * 0.3).sin() * 0.05);
            Kline {
                open_time: index as i64 * HOUR,
                close_time: (index as i64 + 1) * HOUR - 1,
                open: close,
                high: close * 1.01,
                low: close * 0.99,
                close,
                volume: 1.0,
            }
        })
        .collect()
}

fn axis_width_of(candles: Vec<Kline>) -> f32 {
    let mut rig = Rig::new();
    rig.candles = candles;
    rig.frames(&[]);
    rig.rect.right() - rig.probe().plot.right()
}

#[test]
fn the_price_axis_is_no_wider_than_its_labels() {
    let short = axis_width_of(level(400, 1.5));
    let long = axis_width_of(level(400, 123_456.0));
    assert!(short < 70.0, "a short label leaves the axis at {short}");
    assert!(short >= 44.0, "the axis never drops under its floor");
    assert!(long > short, "{long} should be wider than {short}");
}

#[test]
fn a_price_out_of_view_does_not_widen_the_axis() {
    let plain = axis_width_of(level(400, 150.0));
    let mut spiked = level(400, 150.0);
    spiked[0].high = 98_765_432.0;
    // The spike is the oldest bar, far outside the 100 or so the plot shows.
    assert_eq!(axis_width_of(spiked), plain);
}
