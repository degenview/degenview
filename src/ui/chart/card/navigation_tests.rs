//! Navigation through real pointer events: that a drag follows the pointer, that each part of the
//! chart answers its own gesture, and that moving the view never edits a drawing or the data.

use super::*;
use crate::model::chart::viewport::{DEFAULT_VISIBLE, MIN_BARS_AHEAD, MIN_VISIBLE};
use crate::model::{Anchor, Drawing, IndicatorConfig, Tool};
use crate::ui::replay::overlay::{PickKind, ReplayChartFrame};
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
    /// The workspace's cursor, as the drawing layer is told it.
    cursor_mode: crate::model::CursorMode,
    /// Whether the drawing layer was told a long touch is not a right-click, last frame.
    long_press_values: bool,
    selected: Vec<u64>,
    time: f64,
    rect: Rect,
    commands: usize,
    /// The cursor the last frame asked for.
    cursor: egui::CursorIcon,
    /// The chart is in "drag over bars" mode, and what it answered with the last time.
    selecting: bool,
    picked_range: Option<(i64, i64)>,
    /// An Escape was spent by the card, so the workspace's own Escape would not also act.
    escape_consumed: bool,
    /// The window has the keyboard focus.
    focused: bool,
    /// The keys held down, as the window reports them.
    modifiers: Modifiers,
    sent_modifiers: Modifiers,
    /// Whether a drawing tool stays armed after a drawing is placed.
    keep_drawing: bool,
    /// How many times the card asked for the held tool to be put down.
    put_downs: usize,
    /// The chart menu the last frame asked for.
    menu: Option<ContextRequest>,
}

impl Rig {
    fn new() -> Self {
        let mut card = ChartCard::default();
        card.set_timeframe(Timeframe::HOUR_1);
        let mut rig = Self {
            ctx: egui::Context::default(),
            card,
            candles: series(400),
            drawings: Vec::new(),
            scale_mode: PriceScaleMode::Regular,
            tool: Tool::Cursor,
            cursor_mode: crate::model::CursorMode::Cross,
            long_press_values: false,
            selected: Vec::new(),
            time: 0.0,
            rect: Rect::NOTHING,
            commands: 0,
            cursor: egui::CursorIcon::Default,
            selecting: false,
            picked_range: None,
            escape_consumed: false,
            focused: true,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
            keep_drawing: true,
            put_downs: 0,
            menu: None,
        };
        rig.frames(&[]);
        rig
    }

    fn frame(&mut self, mut events: Vec<egui::Event>) {
        self.time += 0.016;
        if self.modifiers != self.sent_modifiers {
            events.insert(0, egui::Event::ModifiersChanged(self.modifiers));
            self.sent_modifiers = self.modifiers;
        }
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
            time: Some(self.time),
            events,
            focused: self.focused,
            ..Default::default()
        };
        let (card, candles, drawings, scale_mode) = (
            &mut self.card,
            &self.candles,
            &self.drawings,
            self.scale_mode,
        );
        let (tool, selected) = (self.tool, self.selected.clone());
        let (selecting, cursor_mode) = (self.selecting, self.cursor_mode);
        let keep_drawing = self.keep_drawing;
        let (mut rect, mut commands) = (Rect::NOTHING, 0);
        let (mut put_down, mut menu) = (false, None);
        let mut picked = None;
        let mut consumed = false;
        let mut long_press = false;
        let mut output = self.ctx.run_ui(input, |ui| {
            let mut frame = DrawingFrame::new(drawings, tool, None, 100)
                .with_selection(selected.clone())
                .with_cursor(cursor_mode)
                .with_behavior(crate::model::MagnetMode::default(), keep_drawing);
            let mut replay = ReplayChartFrame {
                selecting,
                active: false,
                picked_ms: None,
                picked_range: None,
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
            menu = pointer.context;
            put_down = frame.put_down;
            commands = frame.commands.len();
            picked = replay.picked_range;
            consumed = frame.escape_consumed;
            long_press = frame.long_press_values;
        });
        output.textures_delta.clear();
        self.cursor = output.platform_output.cursor_icon;
        self.rect = rect;
        self.commands += commands;
        self.long_press_values = long_press;
        self.menu = menu;
        if put_down {
            // What the workspace does with the card's request.
            self.put_downs += 1;
            self.tool = Tool::Cursor;
            self.card.set_zoom_armed(false);
        }
        self.escape_consumed |= consumed;
        if picked.is_some() {
            self.picked_range = picked;
        }
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
        -((DEFAULT_VISIBLE - MIN_BARS_AHEAD) as f64)
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

impl Rig {
    /// The (fractional) bar index drawn at `x`.
    fn bar_at(&self, x: f32) -> f64 {
        let probe = self.probe();
        f64::from((x - probe.plot.left()) / probe.slot_width - 0.5 - probe.lead)
            + probe.start as f64
    }

    fn wheel(&mut self, delta: Vec2, modifiers: Modifiers) {
        self.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase: egui::TouchPhase::Move,
            modifiers,
        }]);
        self.frames(&[]);
    }
}

/// One zoom input a person makes.
type Gesture = fn(&mut Rig);

fn wheel_in(rig: &mut Rig) {
    rig.wheel(Vec2::new(0.0, 60.0), Modifiers::NONE);
}

fn wheel_out(rig: &mut Rig) {
    rig.wheel(Vec2::new(0.0, -60.0), Modifiers::NONE);
}

fn command_wheel(rig: &mut Rig) {
    rig.wheel(Vec2::new(0.0, 60.0), Modifiers::COMMAND);
}

fn pinch(rig: &mut Rig) {
    rig.frames(&[egui::Event::Zoom(1.5)]);
}

/// A rig scrolled back into history, with the pointer `fraction` of the way across the plot.
fn rig_pointing_at(fraction: f32) -> (Rig, Pos2) {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 120.0;
    rig.frames(&[]);
    let probe = rig.probe();
    let at = Pos2::new(
        probe.plot.left() + probe.plot.width() * fraction,
        rig.in_plot().y,
    );
    rig.frames(&[egui::Event::PointerMoved(at)]);
    (rig, at)
}

#[test]
fn the_plain_wheel_zooms_about_the_right_edge() {
    for (name, zoom) in [("in", wheel_in as Gesture), ("out", wheel_out)] {
        for fraction in [0.1, 0.5, 0.9] {
            let (mut rig, _) = rig_pointing_at(fraction);
            let (visible, edge) = (
                rig.card.viewport.horizontal.visible,
                rig.probe().plot.right(),
            );
            let before = rig.bar_at(edge);
            zoom(&mut rig);
            assert_ne!(
                rig.card.viewport.horizontal.visible, visible,
                "wheel {name} zooms"
            );
            assert!(
                (rig.bar_at(edge) - before).abs() < 0.05,
                "wheel {name} at {fraction}: the right edge moved"
            );
            assert!(close(rig.offset(), 120.0), "the offset is left alone");
        }
    }
}

#[test]
fn ctrl_wheel_and_a_pinch_zoom_about_the_pointer() {
    for (name, zoom) in [("ctrl wheel", command_wheel as Gesture), ("pinch", pinch)] {
        for fraction in [0.1, 0.5, 0.9] {
            let (mut rig, at) = rig_pointing_at(fraction);
            let (visible, before) = (rig.card.viewport.horizontal.visible, rig.bar_at(at.x));
            zoom(&mut rig);
            assert_ne!(
                rig.card.viewport.horizontal.visible, visible,
                "{name} zooms"
            );
            let after = rig.bar_at(at.x);
            assert!(
                (after - before).abs() < 0.05,
                "{name} at {fraction}: the bar under the pointer moved from {before} to {after}"
            );
        }
    }
}

#[test]
fn the_wheel_over_the_price_axis_stretches_the_prices_as_dragging_it_does() {
    for (name, zoom) in [
        ("wheel", wheel_in as Gesture),
        ("ctrl wheel", command_wheel),
        ("pinch", pinch),
    ] {
        let mut rig = Rig::new();
        let at = rig.on_price_axis();
        rig.frames(&[egui::Event::PointerMoved(at)]);
        let before = rig.card.viewport;
        zoom(&mut rig);
        let after = rig.card.viewport;
        assert!(after.vertical.zoom > 1.0, "{name} makes the candles taller");
        assert_eq!(
            after.horizontal, before.horizontal,
            "{name} leaves the time alone"
        );
    }

    let mut rig = Rig::new();
    let at = rig.on_price_axis();
    rig.frames(&[egui::Event::PointerMoved(at)]);
    wheel_out(&mut rig);
    assert!(
        rig.card.viewport.vertical.zoom < 1.0,
        "wheeling out shrinks them"
    );

    // A drag of the same axis in the same direction does the same.
    let mut dragged = Rig::new();
    let from = dragged.on_price_axis();
    dragged.drag(from, Vec2::new(0.0, -50.0));
    assert!(dragged.card.viewport.vertical.zoom > 1.0);
}

#[test]
fn the_wheel_over_the_price_axis_stretches_a_manual_range_about_its_centre() {
    let mut rig = Rig::new();
    rig.card.viewport.vertical.manual = Some(PriceRange::new(100.0, 200.0));
    let at = rig.on_price_axis();
    rig.frames(&[egui::Event::PointerMoved(at)]);
    wheel_in(&mut rig);
    let range = rig.card.viewport.vertical.manual.expect("still manual");
    assert!(range.maximum - range.minimum < 100.0, "a narrower range");
    assert!(
        close((range.minimum + range.maximum) / 2.0, 150.0),
        "same centre"
    );
}

#[test]
fn a_pinch_spreading_the_fingers_shows_fewer_bars() {
    let mut rig = Rig::new();
    let at = rig.in_plot();
    rig.frames(&[egui::Event::PointerMoved(at)]);
    rig.frames(&[egui::Event::Zoom(1.25)]);
    let narrowed = rig.card.viewport.horizontal.visible;
    assert!(narrowed < DEFAULT_VISIBLE);
    rig.frames(&[egui::Event::Zoom(0.8)]);
    assert!(rig.card.viewport.horizontal.visible > narrowed);
}

#[test]
fn a_slow_scroll_adds_up_instead_of_rounding_away() {
    let mut rig = Rig::new();
    let at = rig.in_plot();
    rig.frames(&[egui::Event::PointerMoved(at)]);
    // Two points a frame is under one bar of 120: only the sum of many frames shows.
    for _ in 0..40 {
        rig.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 2.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        }]);
    }
    assert!(rig.card.viewport.horizontal.visible < DEFAULT_VISIBLE - 10);
}

#[test]
fn a_pinch_is_ignored_off_the_chart_and_in_select_portion() {
    let mut rig = Rig::new();
    rig.frames(&[egui::Event::PointerMoved(Pos2::new(-50.0, -50.0))]);
    rig.frames(&[egui::Event::Zoom(1.5)]);
    assert_eq!(rig.card.viewport, ChartViewport::default());

    let at = rig.in_plot();
    rig.card.snapshot_select = true;
    rig.frames(&[egui::Event::PointerMoved(at)]);
    rig.frames(&[egui::Event::Zoom(1.5)]);
    assert_eq!(rig.card.viewport, ChartViewport::default());
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
fn a_chart_scrolled_into_empty_space_slides_left_with_a_new_bar() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = -10.0;
    rig.frames(&[]);
    let newest = x_at_time(&rig, 399);
    let older = x_at_time(&rig, 390);
    rig.candles.push(kline_at(400));
    rig.frames(&[]);
    assert_eq!(rig.offset(), -10.0, "the space it left is kept");
    assert!(
        close(f64::from(x_at_time(&rig, 400)), f64::from(newest)),
        "the new bar takes the newest bar's place"
    );
    assert!(x_at_time(&rig, 390) < older, "the rest moved left");
}

#[test]
fn a_chart_scrolled_back_with_its_newest_bar_on_screen_slides_left_with_a_new_bar() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 4.0;
    rig.frames(&[]);
    let newest = x_at_time(&rig, 399);
    for hour in 400..403 {
        rig.candles.push(kline_at(hour));
        rig.frames(&[]);
    }
    assert_eq!(rig.offset(), 4.0, "the new bars never leave the plot");
    assert!(close(f64::from(x_at_time(&rig, 402)), f64::from(newest)));
}

#[test]
fn a_chart_whose_newest_bar_is_off_screen_stays_put_until_it_is_scrolled_back_to_it() {
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 11.0;
    rig.frames(&[]);
    rig.candles.push(kline_at(400));
    rig.frames(&[]);
    assert_eq!(
        rig.offset(),
        12.0,
        "the default margin of 10 slots hides it past 10.5"
    );
    rig.card.viewport.horizontal.right_offset = 9.0;
    rig.frames(&[]);
    rig.candles.push(kline_at(401));
    rig.frames(&[]);
    assert_eq!(rig.offset(), 9.0, "in view again, it follows the bars");
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
    rig.card
        .pine
        .generation_for_tests(id)
        .expect("the script ran")
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

#[test]
fn each_cursor_shows_its_own_pointer_over_the_plot_and_the_axes_keep_theirs() {
    use crate::model::CursorMode;
    for (mode, over_plot) in [
        (CursorMode::Cross, egui::CursorIcon::Crosshair),
        (CursorMode::Dot, egui::CursorIcon::None),
        (CursorMode::Arrow, egui::CursorIcon::Default),
    ] {
        let mut rig = Rig::new();
        rig.card.set_cursor_mode(mode);
        let plot = rig.in_plot();
        rig.frame(vec![egui::Event::PointerMoved(plot)]);
        rig.frame(Vec::new());
        assert_eq!(rig.cursor, over_plot, "{mode:?} over the plot");
        assert_eq!(rig.card.dot_shown, mode == CursorMode::Dot);
        let price_axis = rig.on_price_axis();
        rig.frame(vec![egui::Event::PointerMoved(price_axis)]);
        rig.frame(Vec::new());
        assert_eq!(rig.cursor, egui::CursorIcon::ResizeVertical, "{mode:?}");
        assert!(!rig.card.dot_shown, "the dot stays on the plot");
        let time_axis = rig.on_time_axis();
        rig.frame(vec![egui::Event::PointerMoved(time_axis)]);
        rig.frame(Vec::new());
        assert_eq!(rig.cursor, egui::CursorIcon::ResizeHorizontal, "{mode:?}");
    }
}

#[test]
fn every_cursor_that_leaves_the_drag_alone_pans_with_it() {
    use crate::model::CursorMode;
    for mode in CursorMode::ALL.into_iter().filter(|mode| mode.caps().pans) {
        let mut rig = Rig::new();
        rig.card.set_cursor_mode(mode);
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(100.0, 0.0));
        assert!(rig.offset() > 0.0, "{mode:?} pans: {}", rig.offset());
        assert!(
            rig.card.viewport.vertical.is_auto(),
            "{mode:?} left the price scale alone"
        );
    }
}

#[test]
fn the_eraser_holds_the_drag_so_the_chart_never_pans_but_the_axes_and_wheel_still_work() {
    use crate::model::CursorMode;
    let mut rig = Rig::new();
    rig.cursor_mode = CursorMode::Eraser;
    rig.card.set_cursor_mode(CursorMode::Eraser);
    rig.frames(&[]);
    let from = rig.in_plot();
    rig.drag(from, Vec2::new(120.0, 0.0));
    assert_eq!(rig.offset(), 0.0, "no pan under the Eraser");
    assert_eq!(rig.cursor, egui::CursorIcon::Crosshair);
    // The price axis still stretches the scale and the wheel still zooms.
    let axis = rig.on_price_axis();
    rig.drag(axis, Vec2::new(-40.0, -60.0));
    assert!(
        rig.card.viewport.vertical.zoom > 1.0,
        "the price axis answers"
    );
    let before = rig.card.viewport.horizontal.visible;
    rig.frame(vec![
        egui::Event::PointerMoved(from),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 60.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        },
    ]);
    rig.frames(&[]);
    assert!(
        rig.card.viewport.horizontal.visible != before,
        "the wheel zooms"
    );
}

/// A rig on the Demonstration cursor.
fn demonstrating() -> Rig {
    use crate::model::CursorMode;
    let mut rig = Rig::new();
    rig.cursor_mode = CursorMode::Demonstration;
    rig.card.set_cursor_mode(CursorMode::Demonstration);
    rig.frames(&[]);
    rig
}

fn strokes(rig: &Rig) -> Vec<usize> {
    use crate::model::cursor::demo::MarkShape;
    rig.card
        .demo
        .marks()
        .iter()
        .map(|mark| match &mark.shape {
            MarkShape::Stroke(points) => points.len(),
            MarkShape::Dot(_) => 0,
        })
        .collect()
}

/// How far (ms of chart time) the single stroke on the card reaches from where it began.
fn stroke_reach(rig: &Rig) -> i64 {
    use crate::model::cursor::demo::MarkShape;
    match &rig.card.demo.marks()[0].shape {
        MarkShape::Stroke(points) => {
            points.last().expect("a point").time_ms - points.first().expect("a point").time_ms
        }
        MarkShape::Dot(_) => 0,
    }
}

#[test]
fn a_quick_stroke_reaches_where_the_button_was_let_go() {
    let mut rig = demonstrating();
    let from = rig.in_plot();
    let far = from + Vec2::new(150.0, 0.0);
    // The press, one move past the drag threshold, then the last of the move arrives with the
    // release in one frame: the stroke must take that position, not stop at the move before it.
    rig.press(from);
    rig.frame(vec![egui::Event::PointerMoved(from + Vec2::new(12.0, 0.0))]);
    rig.frame(vec![egui::Event::PointerMoved(far), rig.button(far, false)]);
    let wanted = (130.0 / rig.slot_width()) as i64 * HOUR;
    assert!(
        stroke_reach(&rig) >= wanted,
        "the stroke reaches {} ms, the hand went {wanted}+ ms",
        stroke_reach(&rig)
    );
    assert!(!rig.card.demonstration_claims_drag(), "and it is finished");
}

#[test]
fn a_drag_under_demonstration_draws_a_stroke_and_never_pans() {
    let mut rig = demonstrating();
    let from = rig.in_plot();
    rig.press(from);
    rig.drag_to(from, from + Vec2::new(90.0, 40.0));
    assert!(
        rig.card.demonstration_claims_drag(),
        "held while it is drawn"
    );
    rig.release(from + Vec2::new(90.0, 40.0));
    assert_eq!(rig.offset(), 0.0, "the chart stayed");
    assert!(rig.card.viewport.vertical.is_auto());
    let lengths = strokes(&rig);
    assert_eq!(lengths.len(), 1);
    assert!(lengths[0] >= 4, "a point for each move: {lengths:?}");
    assert!(
        !rig.card.demonstration_claims_drag(),
        "let go with the button"
    );
    assert_eq!(rig.cursor, egui::CursorIcon::Default, "the plain arrow");
}

#[test]
fn alt_click_puts_a_dot_and_a_plain_click_leaves_nothing() {
    let mut rig = demonstrating();
    let at = rig.in_plot();
    rig.press(at);
    rig.release(at);
    assert!(
        rig.card.demo.marks().is_empty(),
        "a plain click marks nothing"
    );
    rig.modifiers = Modifiers::ALT;
    rig.frames(&[]);
    rig.press(at);
    rig.release(at);
    assert_eq!(strokes(&rig), vec![0], "one dot");
}

#[test]
fn the_marks_fade_away_by_the_clock_not_by_the_frames() {
    use crate::model::cursor::demo::{FADE_SECS, HOLD_SECS};
    let mut rig = demonstrating();
    let from = rig.in_plot();
    rig.drag(from, Vec2::new(60.0, 0.0));
    assert_eq!(rig.card.demo.marks().len(), 1);
    // Frames close together change nothing; one a while later finds it gone.
    for _ in 0..30 {
        rig.frame(Vec::new());
    }
    assert_eq!(rig.card.demo.marks().len(), 1, "still held");
    rig.time += HOLD_SECS + FADE_SECS + 1.0;
    rig.frame(Vec::new());
    assert!(rig.card.demo.marks().is_empty());
}

#[test]
fn another_cursor_or_another_market_drops_the_marks_and_nothing_is_a_drawing() {
    use crate::model::CursorMode;
    let mut rig = demonstrating();
    let from = rig.in_plot();
    rig.drag(from, Vec2::new(60.0, 0.0));
    assert_eq!(rig.commands, 0, "a mark is never a drawing command");
    assert_eq!(rig.card.demo.marks().len(), 1);
    rig.card.abort_drawing();
    assert!(rig.card.demo.marks().is_empty(), "another market");
    rig.drag(from, Vec2::new(60.0, 0.0));
    assert_eq!(rig.card.demo.marks().len(), 1);
    rig.card.set_cursor_mode(CursorMode::Cross);
    assert!(rig.card.demo.marks().is_empty(), "another cursor");
}

#[test]
fn demonstration_leaves_the_drawings_alone_and_the_axes_working() {
    let mut rig = demonstrating();
    rig.drawings.push(Drawing::Ruler {
        id: 1,
        a: Anchor {
            time_ms: 380 * HOUR,
            price: 120.0,
        },
        b: Anchor {
            time_ms: 390 * HOUR,
            price: 140.0,
        },
    });
    let axis = rig.on_price_axis();
    rig.drag(axis, Vec2::new(-40.0, -60.0));
    assert!(
        rig.card.viewport.vertical.zoom > 1.0,
        "the price axis answers"
    );
    assert_eq!(rig.commands, 0);
    assert_eq!(rig.drawings.len(), 1);
}

#[test]
fn once_the_values_tooltip_shows_the_press_is_its_and_the_chart_does_not_pan() {
    let mut rig = Rig::new();
    let from = rig.in_plot();
    rig.press(from);
    rig.time += 0.6;
    rig.frame(Vec::new());
    assert!(rig.card.shows_values_tooltip());
    rig.drag_to(from, from + Vec2::new(120.0, 0.0));
    assert_eq!(rig.offset(), 0.0, "held for the tooltip, not panned");
    rig.release(from + Vec2::new(120.0, 0.0));
    // The next press, moved at once, is an ordinary pan.
    rig.drag(from, Vec2::new(100.0, 0.0));
    assert!(rig.offset() > 0.0);
}

#[test]
fn a_long_touch_is_only_the_tooltips_while_the_tooltip_has_the_press() {
    let mut rig = Rig::new();
    let at = rig.in_plot();
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    assert!(
        !rig.long_press_values,
        "before any press the menu is the long touch's"
    );
    rig.press(at);
    assert!(!rig.long_press_values, "still waiting for the hold");
    rig.time += 0.6;
    rig.frame(Vec::new());
    rig.frame(Vec::new());
    assert!(rig.card.shows_values_tooltip());
    assert!(rig.long_press_values, "the tooltip has the press");
    // The long touch arrives with the release: the frame that sees it still has the tooltip's press.
    rig.frame(vec![rig.button(at, false)]);
    assert!(rig.long_press_values);
    rig.frame(Vec::new());
    assert!(
        !rig.long_press_values,
        "and the next press starts from nothing"
    );
}

#[test]
fn a_press_the_tooltip_never_took_keeps_its_long_touch_menu() {
    use crate::model::CursorMode;
    // On a drawing the press is the drawing's, so there is no tooltip and the menu is not taken.
    let mut rig = Rig::new();
    let covering = Drawing::build(
        Tool::Rectangle,
        1,
        &[
            Anchor {
                time_ms: 0,
                price: 0.0,
            },
            Anchor {
                time_ms: 400 * HOUR,
                price: 10_000.0,
            },
        ],
        Tool::Rectangle.default_style(),
    )
    .expect("a rectangle");
    rig.drawings.push(covering);
    rig.frames(&[]);
    let at = rig.in_plot();
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    rig.press(at);
    rig.time += 0.6;
    rig.frame(Vec::new());
    rig.frame(Vec::new());
    assert!(!rig.card.shows_values_tooltip());
    assert!(
        !rig.long_press_values,
        "a drawing, locked or not, keeps its long-touch menu"
    );
    rig.release(at);

    // The option off, and the cursors that use the press themselves.
    for (on, mode) in [
        (false, CursorMode::Cross),
        (true, CursorMode::Demonstration),
        (true, CursorMode::Eraser),
    ] {
        let mut rig = Rig::new();
        rig.card.set_values_tooltip(on);
        rig.cursor_mode = mode;
        rig.card.set_cursor_mode(mode);
        let at = rig.in_plot();
        rig.frame(vec![egui::Event::PointerMoved(at)]);
        rig.press(at);
        rig.time += 0.6;
        rig.frame(Vec::new());
        rig.frame(Vec::new());
        assert!(!rig.long_press_values, "{mode:?} with the option {on}");
    }
}

#[test]
fn an_armed_drawing_tool_never_has_its_press_taken_by_the_tooltip() {
    let mut rig = Rig::new();
    rig.tool = Tool::TrendLine;
    rig.frames(&[]);
    let from = rig.in_plot();
    let to = from + Vec2::new(120.0, 40.0);
    rig.press(from);
    // Held still well past the long press, then dragged: the drag draws the line.
    rig.time += 0.6;
    rig.frame(Vec::new());
    rig.frame(Vec::new());
    assert!(
        !rig.card.shows_values_tooltip(),
        "a tool is armed: no tooltip"
    );
    assert!(!rig.long_press_values);
    rig.drag_to(from, to);
    rig.release(to);
    assert!(rig.commands >= 1, "the drag placed its line");
}

#[test]
fn a_cursor_change_moves_nothing_on_the_chart() {
    use crate::model::CursorMode;
    let mut rig = Rig::new();
    rig.card.viewport.horizontal.right_offset = 37.5;
    rig.frames(&[]);
    let (offset, range) = (rig.offset(), rig.range());
    for mode in CursorMode::ALL {
        rig.card.set_cursor_mode(mode);
        rig.frames(&[]);
        assert_eq!(rig.offset(), offset, "{mode:?} kept the view");
        assert_eq!(rig.range(), range, "{mode:?} kept the price range");
    }
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

/// The axis of a chart of `candles`, with no countdown to reserve room for: what is measured is
/// the price labels alone (the countdown's room is in `countdown_tests`).
fn axis_width_of(candles: Vec<Kline>) -> f32 {
    let mut rig = Rig::new();
    rig.card.appearance_mut().canvas.bar_countdown = false;
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

#[test]
fn a_drag_over_bars_picks_them_in_order_and_does_not_pan() {
    for backwards in [false, true] {
        let mut rig = Rig::new();
        rig.card.set_pick_kind(PickKind::LogsRange);
        rig.selecting = true;
        rig.frames(&[]);
        let start = rig.probe().start;
        let (a, b) = (start + 10, start + 20);
        let (from, to) = (rig.x_of(a), rig.x_of(b));
        let y = rig.in_plot().y;
        let before = rig.offset();
        if backwards {
            rig.press(Pos2::new(to, y));
            rig.drag_to(Pos2::new(to, y), Pos2::new(from, y));
            rig.release(Pos2::new(from, y));
        } else {
            rig.press(Pos2::new(from, y));
            rig.drag_to(Pos2::new(from, y), Pos2::new(to, y));
            rig.release(Pos2::new(to, y));
        }
        let candles = &rig.candles;
        assert_eq!(
            rig.picked_range,
            Some((candles[a].open_time, candles[b].open_time)),
            "backwards: {backwards}"
        );
        assert_eq!(rig.offset(), before, "the drag is not a pan");
    }
}

#[test]
fn a_click_on_a_bar_picks_that_bar_alone() {
    let mut rig = Rig::new();
    rig.card.set_pick_kind(PickKind::LogsRange);
    rig.selecting = true;
    rig.frames(&[]);
    let index = rig.probe().start + 15;
    let at = Pos2::new(rig.x_of(index), rig.in_plot().y);
    rig.press(at);
    rig.release(at);
    let time = rig.candles[index].open_time;
    assert_eq!(rig.picked_range, Some((time, time)));
}

#[test]
fn a_range_picker_has_the_drag_to_itself_under_demonstration() {
    use crate::model::CursorMode;
    let mut rig = Rig::new();
    rig.cursor_mode = CursorMode::Demonstration;
    rig.card.set_cursor_mode(CursorMode::Demonstration);
    rig.card.set_pick_kind(PickKind::LogsRange);
    rig.selecting = true;
    rig.frames(&[]);
    let probe = rig.probe();
    let first = probe.start + 30;
    let y = rig.in_plot().y;
    let from = Pos2::new(rig.x_of(first), y);
    let to = Pos2::new(rig.x_of(first + 20), y);
    rig.press(from);
    rig.drag_to(from, to);
    assert!(
        rig.card.demo.marks().is_empty() && !rig.card.demonstration_claims_drag(),
        "no stroke is drawn over the bars being picked"
    );
    rig.release(to);
    assert!(
        rig.picked_range.is_some(),
        "the picker still gets its range"
    );
    assert!(rig.card.demo.marks().is_empty());
}

#[test]
fn a_drag_past_the_plot_ends_on_the_last_bar_shown() {
    let mut rig = Rig::new();
    rig.card.set_pick_kind(PickKind::LogsRange);
    rig.selecting = true;
    rig.frames(&[]);
    let probe = rig.probe();
    let first = probe.start + 30;
    let y = rig.in_plot().y;
    rig.press(Pos2::new(rig.x_of(first), y));
    rig.drag_to(
        Pos2::new(rig.x_of(first), y),
        Pos2::new(rig.rect.right() + 200.0, y),
    );
    rig.release(Pos2::new(rig.rect.right() + 200.0, y));
    let (from, to) = rig.picked_range.expect("picked");
    assert_eq!(from, rig.candles[first].open_time);
    assert!(to >= from && to <= rig.candles.last().unwrap().open_time);
    assert!(to > from, "the end is a later bar");
}

#[test]
fn nothing_is_picked_and_the_chart_still_pans_when_the_mode_is_off() {
    let mut rig = Rig::new();
    rig.card.set_pick_kind(PickKind::LogsRange);
    let before = rig.offset();
    rig.drag(rig.in_plot(), Vec2::new(-120.0, 0.0));
    assert_eq!(rig.picked_range, None);
    assert_ne!(rig.offset(), before, "the drag panned");
}

#[test]
fn the_band_over_the_listed_bars_draws_without_moving_the_view() {
    let mut rig = Rig::new();
    let start = rig.probe().start;
    let (a, b) = (
        rig.candles[start + 5].open_time,
        rig.candles[start + 25].open_time,
    );
    let before = rig.offset();
    rig.card.set_logs_range(Some((a, b)));
    rig.frames(&[]);
    assert_eq!(rig.card.logs_range(), Some((a, b)));
    assert_eq!(rig.offset(), before);
    // Out of view on the left: nothing to draw, and no panic.
    rig.card.set_logs_range(Some((-10 * HOUR, -5 * HOUR)));
    rig.frames(&[]);
}

/// The drag over bars, and the band it leaves, as pictures. Run by hand with `DEGENVIEW_GALLERY_DIR`.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_bar_range() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::{self, AppTheme};
    if gallery_dir().is_none() {
        return;
    }
    let candles = series(400);
    for (name, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let mut screen = Screen::new(SIZE);
        theme::apply(&screen.ctx, appearance);
        let mut card = ChartCard::default();
        card.set_timeframe(Timeframe::HOUR_1);
        let mut selecting = true;
        card.set_pick_kind(PickKind::LogsRange);
        let picked = std::cell::Cell::new(None);
        let frame = |screen: &mut Screen,
                     card: &mut ChartCard,
                     selecting: bool,
                     events: Vec<egui::Event>| {
            theme::sync(&screen.ctx);
            screen.run(events, |ui| {
                let drawings: Vec<Drawing> = Vec::new();
                let mut frame = DrawingFrame::new(&drawings, Tool::Cursor, None, 100);
                let mut replay = ReplayChartFrame {
                    selecting,
                    active: false,
                    picked_ms: None,
                    picked_range: None,
                };
                card.body(
                    ui,
                    "BTCUSDT",
                    &candles,
                    false,
                    None,
                    PriceScaleMode::Regular,
                    DataSource::Binance,
                    &IndicatorConfig::default(),
                    None,
                    1,
                    &mut frame,
                    &mut replay,
                );
                picked.set(picked.get().or(replay.picked_range));
            })
        };
        for _ in 0..3 {
            frame(&mut screen, &mut card, selecting, Vec::new());
        }
        let probe = card.probe.expect("drawn");
        let x = |index: usize| {
            probe.plot.left()
                + probe.slot_width * (index as f32 - probe.start as f32 + 0.5 + probe.lead)
        };
        let y = probe.plot.top() + 200.0;
        let (a, b) = (probe.start + 40, probe.start + 70);
        let button = |at: Pos2, pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        // Armed, the pointer over a bar.
        let output = frame(
            &mut screen,
            &mut card,
            selecting,
            vec![egui::Event::PointerMoved(Pos2::new(x(a), y))],
        );
        screen.save(&format!("bar-range-hover-{name}"), output);
        frame(
            &mut screen,
            &mut card,
            selecting,
            vec![button(Pos2::new(x(a), y), true)],
        );
        let mut last = None;
        for step in 1..=6 {
            let at = Pos2::new(x(a) + (x(b) - x(a)) * step as f32 / 6.0, y);
            last = Some(frame(
                &mut screen,
                &mut card,
                selecting,
                vec![egui::Event::PointerMoved(at)],
            ));
        }
        screen.save(&format!("bar-range-drag-{name}"), last.expect("a frame"));
        frame(
            &mut screen,
            &mut card,
            selecting,
            vec![button(Pos2::new(x(b), y), false)],
        );
        let (from, to) = picked.get().expect("picked");
        selecting = false;
        card.set_pick_kind(PickKind::default());
        card.set_logs_range(Some((from, to)));
        let mut last = None;
        for _ in 0..3 {
            last = Some(frame(&mut screen, &mut card, selecting, Vec::new()));
        }
        screen.save(&format!("bar-range-set-{name}"), last.expect("a frame"));
    }
}

// MARK: Select portion

mod select_portion {
    use super::*;

    fn selecting() -> Rig {
        let mut rig = Rig::new();
        rig.card.set_snapshot_select(true);
        rig.frames(&[]);
        rig
    }

    #[test]
    fn a_drag_does_not_pan_and_leaves_drawings_alone() {
        let mut rig = selecting();
        rig.drawings = vec![trend_line()];
        let before = (rig.drawings.clone(), rig.card.viewport);
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(120.0, 60.0));
        // Start it on the line itself, too.
        let on_line = Pos2::new(rig.x_of(340), rig.y_of(255.0));
        rig.drag(on_line, Vec2::new(-90.0, 40.0));
        assert_eq!((rig.drawings.clone(), rig.card.viewport), before);
        assert_eq!(rig.commands, 0, "no drawing, no undo step");
    }

    #[test]
    fn the_axes_do_not_scale_either() {
        let mut rig = selecting();
        let viewport = rig.card.viewport;
        let axis = rig.on_price_axis();
        rig.drag(axis, Vec2::new(0.0, -80.0));
        let axis = rig.on_time_axis();
        rig.drag(axis, Vec2::new(80.0, 0.0));
        assert_eq!(rig.card.viewport, viewport);
    }

    #[test]
    fn the_wheel_and_a_swipe_do_nothing() {
        let mut rig = selecting();
        let viewport = rig.card.viewport;
        let at = rig.in_plot();
        rig.frame(vec![egui::Event::PointerMoved(at)]);
        for delta in [Vec2::new(0.0, 40.0), Vec2::new(60.0, 3.0)] {
            rig.frame(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            }]);
            rig.frames(&[]);
        }
        assert_eq!(rig.card.viewport, viewport);
    }

    #[test]
    fn the_card_does_not_ask_for_a_cursor_of_its_own() {
        let mut rig = selecting();
        for at in [rig.in_plot(), rig.on_price_axis(), rig.on_time_axis()] {
            rig.frame(vec![egui::Event::PointerMoved(at)]);
            assert_eq!(rig.cursor, egui::CursorIcon::Default, "{at:?}");
        }
    }

    #[test]
    fn it_ends_when_the_workspace_says_so() {
        let mut rig = selecting();
        rig.card.set_snapshot_select(false);
        rig.frames(&[]);
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(100.0, 0.0));
        assert!(rig.offset() > 1.0, "the chart pans again");
    }

    #[test]
    fn the_snapshot_frame_is_the_body_below_the_header() {
        let mut rig = Rig::new();
        rig.card.set_top_inset(36.0);
        rig.frames(&[]);
        let (body, frame) = (rig.card.body_rect(), rig.card.snapshot_frame());
        assert_eq!(body, rig.rect);
        assert_eq!(frame.min, Pos2::new(body.left(), body.top() + 36.0));
        assert_eq!(frame.max, body.max);
        assert_eq!(frame.size(), rig.card.snapshot_size());
    }
}

// MARK: Zoom In

mod zoom_in {
    use super::*;

    fn armed() -> Rig {
        let mut rig = Rig::new();
        rig.card.set_zoom_armed(true);
        rig.frames(&[]);
        rig
    }

    fn escape() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    /// The price drawn at `y`, on the scale the chart has now.
    fn price_at(rig: &Rig, y: f32) -> f64 {
        let probe = rig.probe();
        probe.maximum
            - f64::from((y - probe.plot.top()) / probe.plot.height())
                * (probe.maximum - probe.minimum)
    }

    /// The (fractional) bar index under `x`.
    fn bar_at(rig: &Rig, x: f32) -> f64 {
        let probe = rig.probe();
        f64::from((x - probe.plot.left()) / probe.slot_width - probe.lead - 0.5)
            + probe.start as f64
    }

    #[test]
    fn a_drag_zooms_both_axes_to_the_rectangle() {
        let mut rig = armed();
        let from = rig.in_plot();
        let to = from + Vec2::new(200.0, 120.0);
        let (first, last) = (bar_at(&rig, from.x), bar_at(&rig, to.x));
        let (top, bottom) = (price_at(&rig, from.y), price_at(&rig, to.y));
        let before = rig.card.viewport.horizontal.visible;
        let original = rig.card.viewport;
        rig.press(from);
        rig.drag_to(from, to);
        rig.release(to);
        let was = rig
            .card
            .take_zoomed()
            .expect("the workspace is told to put the tool down");
        assert_eq!(
            was.viewport, original,
            "and which view the zoom started from"
        );
        assert!(rig.card.viewport.horizontal.visible < before);
        let slot = f64::from(rig.slot_width());
        let plot = rig.probe().plot;
        // The bars under the rectangle's sides are now at the plot's sides, within half a bar.
        let left = f64::from(plot.left())
            + slot * (first - rig.probe().start as f64 + 0.5 + f64::from(rig.probe().lead));
        assert!(
            (left - f64::from(plot.left())).abs() <= slot * 1.01,
            "left {left}"
        );
        let right_bar = bar_at(&rig, plot.right());
        assert!(
            (right_bar - last).abs() <= 1.01,
            "right {right_bar} vs {last}"
        );
        // The price range is the rectangle's, and autoscale is off.
        let (low, high) = rig.range();
        assert!(close(high, top), "top {high} vs {top}");
        assert!(close(low, bottom), "bottom {low} vs {bottom}");
        assert!(!rig.card.viewport.vertical.is_auto());
        assert_eq!(rig.commands, 0, "no drawing, no undo step");
    }

    #[test]
    fn every_drag_direction_gives_the_same_view() {
        let start = Pos2::new(0.0, 0.0);
        let _ = start;
        let mut views = Vec::new();
        for flip in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            let mut rig = armed();
            let centre = rig.in_plot();
            let (a, b) = (
                centre - Vec2::new(90.0 * flip.0, 60.0 * flip.1),
                centre + Vec2::new(90.0 * flip.0, 60.0 * flip.1),
            );
            rig.press(a);
            rig.drag_to(a, b);
            rig.release(b);
            assert!(rig.card.take_zoomed().is_some());
            views.push((
                rig.card.viewport.horizontal.visible,
                rig.card.viewport.horizontal.right_offset,
                rig.range(),
            ));
        }
        for view in &views[1..] {
            assert_eq!(view.0, views[0].0);
            assert!(close(view.1, views[0].1), "{} vs {}", view.1, views[0].1);
            assert!(close(view.2.0, views[0].2.0) && close(view.2.1, views[0].2.1));
        }
    }

    #[test]
    fn the_pointer_shows_the_zoom_cursor_over_the_plot_only() {
        let mut rig = armed();
        let at = rig.in_plot();
        rig.frame(vec![egui::Event::PointerMoved(at)]);
        assert_eq!(rig.cursor, egui::CursorIcon::ZoomIn);
        let axis = rig.on_price_axis();
        rig.frame(vec![egui::Event::PointerMoved(axis)]);
        assert_eq!(rig.cursor, egui::CursorIcon::ResizeVertical);
        // Not armed: the usual crosshair.
        let mut plain = Rig::new();
        let at = plain.in_plot();
        plain.frame(vec![egui::Event::PointerMoved(at)]);
        assert_ne!(plain.cursor, egui::CursorIcon::ZoomIn);
    }

    #[test]
    fn a_drag_never_pans_and_leaves_drawings_alone() {
        let mut rig = armed();
        rig.drawings = vec![trend_line()];
        let before = rig.drawings.clone();
        // Start the rectangle on the line itself.
        let from = Pos2::new(rig.x_of(340), rig.y_of(255.0));
        rig.drag(from, Vec2::new(-120.0, 80.0));
        assert_eq!(rig.drawings, before);
        assert_eq!(rig.commands, 0);
        assert!(
            rig.card.take_zoomed().is_some(),
            "it zoomed instead of moving the line or panning"
        );
    }

    #[test]
    fn an_unarmed_drag_still_pans() {
        let mut rig = Rig::new();
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(100.0, 0.0));
        assert!(rig.offset() > 1.0);
        assert!(rig.card.take_zoomed().is_none());
        assert!(!rig.card.can_zoom_out());
    }

    #[test]
    fn escape_cancels_the_rectangle_and_the_release_does_nothing() {
        let mut rig = armed();
        let (from, to) = (rig.in_plot(), rig.in_plot() + Vec2::new(180.0, 100.0));
        let viewport = rig.card.viewport;
        rig.press(from);
        rig.drag_to(from, to);
        rig.escape_consumed = false;
        rig.frame(vec![escape()]);
        assert!(
            rig.escape_consumed,
            "the first Escape is spent on the rectangle"
        );
        rig.drag_to(to, to + Vec2::new(20.0, 20.0));
        rig.release(to);
        assert_eq!(rig.card.viewport, viewport);
        assert!(rig.card.take_zoomed().is_none());
        assert!(!rig.card.zoom_claims_drag());
        // Idle, an Escape is not the card's: the workspace disarms the tool with it.
        rig.escape_consumed = false;
        rig.frame(vec![escape()]);
        assert!(!rig.escape_consumed);
    }

    #[test]
    fn a_second_button_cancels_the_rectangle() {
        let mut rig = armed();
        let (from, to) = (rig.in_plot(), rig.in_plot() + Vec2::new(180.0, 100.0));
        let viewport = rig.card.viewport;
        rig.press(from);
        rig.drag_to(from, to);
        rig.frame(vec![egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        rig.release(to);
        assert_eq!(rig.card.viewport, viewport);
        assert!(rig.card.take_zoomed().is_none());
    }

    #[test]
    fn losing_the_window_or_the_tool_drops_the_rectangle() {
        for how in 0..3 {
            let mut rig = armed();
            let (from, to) = (rig.in_plot(), rig.in_plot() + Vec2::new(180.0, 100.0));
            let viewport = rig.card.viewport;
            rig.press(from);
            rig.drag_to(from, to);
            assert!(rig.card.zoom_claims_drag());
            match how {
                0 => {
                    rig.focused = false;
                    rig.frame(Vec::new());
                    rig.focused = true;
                }
                1 => {
                    rig.card.set_zoom_armed(false);
                    rig.frame(Vec::new());
                }
                _ => {
                    // The card goes unseen for a frame (another tab), then comes back.
                    for _ in 0..2 {
                        rig.time += 0.016;
                        let input = egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SIZE)),
                            time: Some(rig.time),
                            ..Default::default()
                        };
                        let _ = rig.ctx.run_ui(input, |_| {});
                    }
                    rig.frame(Vec::new());
                }
            }
            assert!(!rig.card.zoom_claims_drag(), "case {how}");
            rig.release(to);
            assert_eq!(rig.card.viewport, viewport, "case {how}");
            assert!(rig.card.take_zoomed().is_none(), "case {how}");
        }
    }

    #[test]
    fn a_tiny_drag_zooms_nothing_and_leaves_the_tool_armed() {
        let mut rig = armed();
        let from = rig.in_plot();
        let viewport = rig.card.viewport;
        // Past egui's drag threshold, under the minimum selection.
        rig.drag(from, Vec2::new(7.0, 7.0));
        assert_eq!(rig.card.viewport, viewport);
        assert!(rig.card.take_zoomed().is_none());
        assert!(rig.card.zoom_is_live(true, false));
        // A plain click does nothing either.
        rig.press(from);
        rig.release(from);
        assert_eq!(rig.card.viewport, viewport);
    }

    #[test]
    fn a_flat_rectangle_zooms_time_only() {
        let mut rig = armed();
        let from = rig.in_plot();
        let before = rig.card.viewport.horizontal.visible;
        rig.drag(from, Vec2::new(200.0, 3.0));
        assert!(rig.card.take_zoomed().is_some());
        assert!(rig.card.viewport.horizontal.visible < before);
        assert!(
            rig.card.viewport.vertical.is_auto(),
            "the price axis is left alone"
        );
    }

    #[test]
    fn a_zoom_into_old_history_stays_in_old_history() {
        let mut rig = armed();
        rig.card.viewport.horizontal.right_offset = 200.0;
        rig.frames(&[]);
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(150.0, 100.0));
        assert!(rig.card.take_zoomed().is_some());
        assert!(
            rig.offset() > 150.0,
            "not thrown to the newest bar: {}",
            rig.offset()
        );
        assert!(!rig.card.viewport.horizontal.follows_latest());
    }

    #[test]
    fn the_result_is_always_a_view_the_chart_can_draw() {
        // Whatever is dragged, from anywhere, to anywhere, even off the plot.
        let corners = [(-400.0, -400.0), (900.0, 900.0), (3.0, 500.0), (700.0, 5.0)];
        for &(dx, dy) in &corners {
            let mut rig = armed();
            let from = rig.in_plot();
            rig.drag(from, Vec2::new(dx, dy));
            let h = rig.card.viewport.horizontal;
            let (low, high) = rig
                .card
                .viewport
                .horizontal
                .offset_limits(rig.candles.len());
            assert!(h.right_offset.is_finite() && h.right_offset >= low && h.right_offset <= high);
            assert!(h.visible >= MIN_VISIBLE);
            if let Some(range) = rig.card.viewport.vertical.manual {
                assert!(range.is_valid(false));
            }
        }
    }

    #[test]
    fn a_logarithmic_range_is_ratio_correct() {
        let mut rig = armed();
        rig.scale_mode = PriceScaleMode::Logarithmic;
        rig.frames(&[]);
        let probe = rig.probe();
        let (min, max, top, height) = (
            probe.minimum,
            probe.maximum,
            probe.plot.top(),
            probe.plot.height(),
        );
        let from = rig.in_plot();
        let to = from + Vec2::new(160.0, 100.0);
        rig.press(from);
        rig.drag_to(from, to);
        rig.release(to);
        assert!(rig.card.take_zoomed().is_some());
        let range = rig.card.viewport.vertical.manual.expect("manual range");
        // Prices read through the log map, not linearly.
        let at =
            |y: f32| (min.ln() + (max.ln() - min.ln()) * f64::from(1.0 - (y - top) / height)).exp();
        assert!(
            close(range.maximum, at(from.y)),
            "{} vs {}",
            range.maximum,
            at(from.y)
        );
        assert!(
            close(range.minimum, at(to.y)),
            "{} vs {}",
            range.minimum,
            at(to.y)
        );
        assert!(range.minimum > 0.0);
    }

    #[test]
    fn the_axes_and_panes_keep_their_own_drags_while_armed() {
        let mut rig = armed();
        let axis = rig.on_price_axis();
        rig.drag(axis, Vec2::new(0.0, -60.0));
        assert!(
            rig.card.viewport.vertical.zoom != 1.0,
            "the price axis still stretches"
        );
        assert!(rig.card.take_zoomed().is_none());
        let visible = rig.card.viewport.horizontal.visible;
        let axis = rig.on_time_axis();
        rig.drag(axis, Vec2::new(-80.0, 0.0));
        assert!(
            rig.card.viewport.horizontal.visible != visible,
            "the time axis still stretches"
        );
        assert!(rig.card.take_zoomed().is_none());
    }

    #[test]
    fn the_replay_start_picker_wins_the_pointer() {
        let mut rig = armed();
        rig.selecting = true;
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(160.0, 90.0));
        assert!(rig.card.take_zoomed().is_none(), "the picker is not a zoom");
        assert!(!rig.card.can_zoom_out());
    }

    #[test]
    fn zoom_out_goes_back_to_the_exact_views_in_turn() {
        let mut rig = armed();
        assert!(!rig.card.can_zoom_out());
        let first = rig.card.viewport;
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(250.0, 150.0));
        assert!(rig.card.take_zoomed().is_some());
        let second = rig.card.viewport;
        assert!(rig.card.can_zoom_out());
        rig.card.set_zoom_armed(true);
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(120.0, 80.0));
        assert!(rig.card.take_zoomed().is_some());
        rig.card.zoom_out();
        assert_eq!(rig.card.viewport, second);
        rig.card.zoom_out();
        assert_eq!(rig.card.viewport, first);
        assert!(!rig.card.can_zoom_out());
        rig.card.zoom_out();
        assert_eq!(rig.card.viewport, first, "nothing left is not an error");
    }

    #[test]
    fn undoing_and_redoing_a_zoom_keeps_zoom_outs_history_in_step() {
        let mut rig = armed();
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(250.0, 150.0));
        let before = rig.card.take_zoomed().expect("zoomed");
        let zoomed = rig.card.viewport;
        // Undo: the view the zoom left goes back, and there is nothing to zoom out of.
        let undone = rig
            .card
            .apply_view(before, ViewMove::Back)
            .expect("applies");
        assert_eq!(undone.viewport, zoomed);
        assert_eq!(rig.card.viewport, before.viewport);
        assert!(!rig.card.can_zoom_out());
        // Redo: the zoomed view is back, with the view it came from to return to.
        let redone = rig
            .card
            .apply_view(undone, ViewMove::Forward)
            .expect("applies");
        assert_eq!(redone.viewport, before.viewport);
        assert_eq!(rig.card.viewport, zoomed);
        assert!(rig.card.can_zoom_out());
        assert_eq!(rig.card.zoom_out().map(|left| left.viewport), Some(zoomed));
        assert_eq!(rig.card.viewport, before.viewport);
    }

    /// The bars the chart shows first, as (index, fractional lead): which bars are on screen.
    fn first_bar(rig: &Rig) -> (usize, f32) {
        let probe = rig.probe();
        (probe.start, probe.lead)
    }

    #[test]
    fn a_stored_view_comes_back_on_the_same_bars_after_new_bars_arrive() {
        let mut rig = armed();
        rig.card.viewport.horizontal.right_offset = 200.0;
        rig.frames(&[]);
        let shown = first_bar(&rig);
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(200.0, 100.0));
        assert!(rig.card.can_zoom_out());
        // Bars arrive in two bursts while the view is on old history, then Zoom out.
        for total in [410, 425] {
            rig.candles = series(total);
            rig.frames(&[]);
        }
        rig.card.zoom_out().expect("a view to go back to");
        rig.frames(&[]);
        let now = first_bar(&rig);
        assert_eq!(now.0, shown.0, "the same bars, not 25 newer");
        assert!((now.1 - shown.1).abs() < 1e-3);
    }

    #[test]
    fn a_view_that_showed_the_newest_bar_still_follows_it() {
        let mut rig = armed();
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(200.0, 100.0));
        rig.candles = series(430);
        rig.frames(&[]);
        rig.card.zoom_out().expect("a view to go back to");
        rig.frames(&[]);
        assert_eq!(
            rig.offset(),
            0.0,
            "newest bar kept where the margin puts it"
        );
        assert_eq!(rig.card.viewport.horizontal.visible, DEFAULT_VISIBLE);
    }

    #[test]
    fn redo_of_a_zoom_is_put_on_the_same_bars_after_new_bars_arrive() {
        let mut rig = armed();
        rig.card.viewport.horizontal.right_offset = 200.0;
        rig.frames(&[]);
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(200.0, 100.0));
        rig.frames(&[]);
        let zoomed_at = first_bar(&rig);
        let before = rig.card.take_zoomed().expect("zoomed");
        // Undo, then bars arrive, then redo.
        let undone = rig
            .card
            .apply_view(before, ViewMove::Back)
            .expect("applies");
        rig.candles = series(430);
        rig.frames(&[]);
        rig.card
            .apply_view(undone, ViewMove::Forward)
            .expect("applies");
        rig.frames(&[]);
        let now = first_bar(&rig);
        assert_eq!(now.0, zoomed_at.0);
        assert!((now.1 - zoomed_at.1).abs() < 1e-3);
    }

    #[test]
    fn a_view_of_another_series_is_never_put_back() {
        // Candles to a price-based type, and one price-based type to another.
        for (from_type, to_type) in [
            (ChartType::Candles, ChartType::Renko),
            (ChartType::Renko, ChartType::Kagi),
        ] {
            let mut rig = armed();
            rig.card.set_chart_type(from_type);
            rig.frames(&[]);
            rig.card.zoom_to_for_test(40, 30.0);
            let step = rig.card.take_zoomed().expect("zoomed");
            rig.card.set_chart_type(to_type);
            let viewport = rig.card.viewport;
            assert!(!rig.card.can_zoom_out(), "{from_type:?} to {to_type:?}");
            assert!(rig.card.zoom_out().is_none());
            assert!(rig.card.apply_view(step, ViewMove::Back).is_none());
            assert_eq!(rig.card.viewport, viewport, "nothing was restored");
        }
    }

    #[test]
    fn editing_a_construction_makes_earlier_views_stale_too() {
        use crate::model::chart::kind::{SizeMethod, SizeRule};
        let rule = |amount| SizeRule {
            method: SizeMethod::Traditional,
            amount,
            ..Default::default()
        };
        let mut rig = armed();
        rig.card.set_chart_type(ChartType::Renko);
        rig.card.chart_style_mut().renko.size = rule(1.0);
        rig.frames(&[]);
        rig.card.zoom_to_for_test(40, 30.0);
        let step = rig.card.take_zoomed().expect("zoomed");
        assert!(rig.card.can_zoom_out());
        // Same type, other brick size: other bricks.
        rig.card.chart_style_mut().renko.size = rule(5.0);
        rig.frames(&[]);
        assert!(!rig.card.can_zoom_out());
        assert!(rig.card.apply_view(step, ViewMove::Back).is_none());
        // Restyling that does not change the bricks leaves a new zoom alone.
        rig.card.zoom_to_for_test(30, 20.0);
        rig.frames(&[]);
        assert!(rig.card.can_zoom_out());
    }

    #[test]
    fn resetting_the_view_forgets_the_zooms() {
        let mut rig = armed();
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(250.0, 150.0));
        assert!(rig.card.can_zoom_out());
        rig.card.reset_view();
        assert!(!rig.card.can_zoom_out());
    }

    #[test]
    fn the_history_is_bounded() {
        let mut rig = armed();
        for _ in 0..12 {
            rig.card.set_zoom_armed(true);
            rig.card.viewport.horizontal.visible = 300;
            rig.card.viewport.vertical.manual = None;
            let from = rig.in_plot();
            rig.drag(from, Vec2::new(200.0, 120.0));
            rig.card.take_zoomed();
        }
        let mut steps = 0;
        while rig.card.can_zoom_out() {
            rig.card.zoom_out();
            steps += 1;
        }
        assert_eq!(steps, crate::model::chart::viewport::zoom::MAX_BACK);
    }

    #[test]
    fn a_zoom_on_a_replay_cut_series_never_goes_past_its_bars() {
        let mut rig = armed();
        rig.candles.truncate(150);
        rig.frames(&[]);
        let from = rig.in_plot();
        rig.drag(from, Vec2::new(300.0, 100.0));
        let h = rig.card.viewport.horizontal;
        let (low, high) = h.offset_limits(150);
        assert!(h.right_offset >= low && h.right_offset <= high);
        assert_eq!(rig.candles.len(), 150);
    }
}

/// The Zoom In rectangle while it is dragged, and the chart after the zoom, as pictures. Run by
/// hand with `DEGENVIEW_GALLERY_DIR`.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_zoom_in() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::{self, AppTheme};
    if gallery_dir().is_none() {
        return;
    }
    let candles = series(400);
    for (name, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let mut screen = Screen::new(SIZE);
        theme::apply(&screen.ctx, appearance);
        let mut card = ChartCard::default();
        card.set_timeframe(Timeframe::HOUR_1);
        card.set_zoom_armed(true);
        let frame = |screen: &mut Screen, card: &mut ChartCard, events: Vec<egui::Event>| {
            theme::sync(&screen.ctx);
            screen.run(events, |ui| {
                let drawings: Vec<Drawing> = Vec::new();
                let mut frame = DrawingFrame::new(&drawings, Tool::Cursor, None, 100);
                let mut replay = ReplayChartFrame {
                    selecting: false,
                    active: false,
                    picked_ms: None,
                    picked_range: None,
                };
                card.body(
                    ui,
                    "BTCUSDT",
                    &candles,
                    false,
                    None,
                    PriceScaleMode::Regular,
                    DataSource::Binance,
                    &IndicatorConfig::default(),
                    None,
                    1,
                    &mut frame,
                    &mut replay,
                );
            })
        };
        for _ in 0..3 {
            frame(&mut screen, &mut card, Vec::new());
        }
        let plot = card.probe.expect("drawn").plot;
        let (from, to) = (
            Pos2::new(plot.left() + 300.0, plot.top() + 60.0),
            Pos2::new(plot.left() + 470.0, plot.top() + 200.0),
        );
        let button = |at: Pos2, pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let output = frame(
            &mut screen,
            &mut card,
            vec![egui::Event::PointerMoved(from)],
        );
        screen.save(&format!("zoom-in-hover-{name}"), output);
        frame(&mut screen, &mut card, vec![button(from, true)]);
        let mut last = None;
        for step in 1..=6 {
            let at = from + (to - from) * (step as f32 / 6.0);
            last = Some(frame(
                &mut screen,
                &mut card,
                vec![egui::Event::PointerMoved(at)],
            ));
        }
        screen.save(&format!("zoom-in-drag-{name}"), last.expect("a frame"));
        frame(&mut screen, &mut card, vec![button(to, false)]);
        let mut last = None;
        for _ in 0..3 {
            last = Some(frame(&mut screen, &mut card, Vec::new()));
        }
        screen.save(&format!("zoom-in-done-{name}"), last.expect("a frame"));
    }
}

/// A right-click with a tool in hand puts it down; the menu is the next click's.
mod right_click {
    use super::*;
    use crate::model::chart::context::Target;

    /// The target of the menu a right-click at `at` asked for, over the frames it took, if any.
    fn answer(rig: &mut Rig, at: Pos2) -> Option<Target> {
        rig.menu = None;
        rig.frame(vec![egui::Event::PointerMoved(at)]);
        let mut menu = None;
        for pressed in [true, false] {
            rig.frame(vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
            menu = menu.or(rig.menu.take());
        }
        rig.frame(Vec::new());
        menu = menu.or(rig.menu.take());
        menu.map(|request| request.target)
    }

    #[test]
    fn the_cursor_opens_the_menu_on_the_first_right_click() {
        let mut rig = Rig::new();
        let at = rig.in_plot();
        assert_eq!(answer(&mut rig, at), Some(Target::Background));
        assert_eq!(rig.put_downs, 0);
    }

    #[test]
    fn a_drawing_tool_is_put_down_by_the_first_right_click_and_the_second_opens_the_menu() {
        for keep_drawing in [true, false] {
            let mut rig = Rig::new();
            rig.keep_drawing = keep_drawing;
            rig.tool = Tool::Rectangle;
            let at = rig.in_plot();
            assert_eq!(answer(&mut rig, at), None, "keep drawing {keep_drawing}");
            assert_eq!(rig.put_downs, 1);
            assert_eq!(rig.tool, Tool::Cursor);
            assert_eq!(answer(&mut rig, at), Some(Target::Background));
            assert_eq!(rig.put_downs, 1, "the second click puts nothing down");
        }
    }

    #[test]
    fn zoom_in_is_put_down_the_same_way() {
        let mut rig = Rig::new();
        rig.card.set_zoom_armed(true);
        let at = rig.in_plot();
        assert_eq!(answer(&mut rig, at), None);
        assert_eq!(rig.put_downs, 1);
        assert!(!rig.card.is_zoom_armed());
        assert_eq!(answer(&mut rig, at), Some(Target::Background));
    }

    #[test]
    fn the_axes_open_their_menu_at_once_and_keep_the_tool() {
        let mut rig = Rig::new();
        rig.tool = Tool::TrendLine;
        let axis = rig.on_price_axis();
        assert_eq!(answer(&mut rig, axis), Some(Target::PriceAxis));
        let time = rig.on_time_axis();
        assert_eq!(answer(&mut rig, time), Some(Target::TimeAxis));
        assert_eq!(rig.put_downs, 0);
        assert_eq!(rig.tool, Tool::TrendLine);
    }

    #[test]
    fn a_half_placed_drawing_is_abandoned_and_the_tool_put_down_by_one_click() {
        let mut rig = Rig::new();
        rig.tool = Tool::ThreeDrivesPattern;
        let plot = rig.in_plot();
        for step in 0..3 {
            let at = plot + Vec2::new(30.0 * step as f32, 20.0 * step as f32);
            rig.frame(vec![egui::Event::PointerMoved(at)]);
            rig.frame(vec![rig.button(at, true)]);
            rig.frame(vec![rig.button(at, false)]);
        }
        assert_eq!(answer(&mut rig, plot), None);
        assert_eq!(rig.tool, Tool::Cursor);
        assert_eq!(rig.commands, 0, "nothing was drawn");
        assert_eq!(answer(&mut rig, plot), Some(Target::Background));
    }

    const PANE_SCRIPT: &str = "//@version=6\nindicator(\"Pane\")\nplot(close)\n";

    /// A chart with one script pane, and a point inside that pane.
    fn with_pane() -> (Rig, Pos2) {
        let mut rig = Rig::new();
        let id = rig.card.add_pine_instance(None, "Pane", PANE_SCRIPT);
        wait_for_script(&mut rig, id);
        rig.frames(&[]);
        assert_eq!(rig.card.pane_count(), 1, "the script has a pane of its own");
        // The panes sit below the time axis.
        let pane = Pos2::new(rig.rect.center().x, rig.rect.bottom() - 20.0);
        (rig, pane)
    }

    fn click_plot(rig: &mut Rig, at: Pos2) {
        rig.frame(vec![egui::Event::PointerMoved(at)]);
        rig.frame(vec![rig.button(at, true)]);
        rig.frame(vec![rig.button(at, false)]);
    }

    #[test]
    fn an_open_shape_is_finished_by_the_right_click_that_puts_its_tool_down_on_the_plot_or_a_pane()
    {
        for tool in [Tool::Polyline, Tool::Path] {
            for keep_drawing in [true, false] {
                for in_pane in [false, true] {
                    let (mut rig, pane) = with_pane();
                    rig.keep_drawing = keep_drawing;
                    rig.tool = tool;
                    let plot = rig.in_plot();
                    click_plot(&mut rig, plot);
                    click_plot(&mut rig, plot + Vec2::new(80.0, 30.0));
                    assert_eq!(rig.commands, 0, "still being drawn");
                    let at = if in_pane { pane } else { plot };
                    let case = format!("{tool:?}, keep drawing {keep_drawing}, pane {in_pane}");
                    assert_eq!(answer(&mut rig, at), None, "{case}");
                    assert_eq!(rig.put_downs, 1, "{case}");
                    assert_eq!(rig.tool, Tool::Cursor, "{case}");
                    assert_eq!(rig.commands, 1, "the shape was committed: {case}");
                    let next = answer(&mut rig, at);
                    if in_pane {
                        assert!(matches!(next, Some(Target::Pane { .. })), "{case}");
                    } else {
                        assert_eq!(next, Some(Target::Background), "{case}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_pane_right_click_abandons_a_half_placed_drawing_as_the_plot_does() {
        let (mut rig, pane) = with_pane();
        rig.tool = Tool::ThreeDrivesPattern;
        let plot = rig.in_plot();
        for step in 0..3 {
            click_plot(
                &mut rig,
                plot + Vec2::new(30.0 * step as f32, 20.0 * step as f32),
            );
        }
        assert_eq!(answer(&mut rig, pane), None);
        assert_eq!(rig.tool, Tool::Cursor);
        assert_eq!(rig.commands, 0, "nothing was drawn");
    }
}
