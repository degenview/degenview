//! The drawing layer driven the way the app drives it: pointer and key events through a real egui
//! context into `DrawingLayer::update` and `paint`, with the workspace's side (applying commands,
//! counting undo steps) played by a small rig.

use super::*;
use crate::model::{DrawingStyle, Kline, Visibility};
use eframe::egui::{Event, Key, Modifiers, RawInput, Sense, pos2, vec2};
use menu::Action;

const PLOT: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 400.0));

fn candles() -> Vec<Kline> {
    (0..10)
        .map(|index| Kline {
            open_time: index * 60_000,
            close_time: index * 60_000 + 59_999,
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 1.0,
        })
        .collect()
}

fn scale(logarithmic: bool) -> PriceScale {
    PriceScale {
        minimum: 80.0,
        maximum: 120.0,
        top: 0.0,
        height: 400.0,
        logarithmic,
    }
}

fn projection<'a>(data: &'a [Kline], scale: &'a PriceScale) -> Projection<'a> {
    Projection {
        plot: PLOT,
        pixels_per_point: 2.0,
        format: PriceFormat::AUTO,
        scale,
        times: TimeIndex::new(data),
        start: 0,
        slot_width: 100.0,
        lead: 0.0,
    }
}

fn anchor(time_ms: i64, price: f64) -> Anchor {
    Anchor { time_ms, price }
}

/// Four well-separated points inside the plot.
const POINTS: [Pos2; 4] = [
    pos2(200.0, 200.0),
    pos2(420.0, 110.0),
    pos2(640.0, 270.0),
    pos2(520.0, 340.0),
];

// MARK: Projection and editing

#[test]
fn anchors_round_trip_through_the_screen() {
    let data = candles();
    for logarithmic in [false, true] {
        let scale = scale(logarithmic);
        let proj = projection(&data, &scale);
        let a = anchor(150_000, 97.5);
        let back = proj.anchor(proj.point(a));
        assert!((back.time_ms - a.time_ms).abs() <= 1, "{back:?}");
        // Screen coordinates are f32, so the price comes back to pixel precision.
        assert!((back.price - a.price).abs() < 1e-3, "{back:?}");
    }
}

#[test]
fn candle_centres_land_mid_slot_and_a_lead_shifts_right() {
    let data = candles();
    let scale = scale(false);
    let mut proj = projection(&data, &scale);
    assert_eq!(proj.x(0), 50.0);
    assert_eq!(proj.x(60_000), 150.0);
    proj.lead = 2.0;
    assert_eq!(proj.x(0), 250.0);
    proj.lead = 0.0;
    proj.start = 3;
    assert_eq!(proj.x(3 * 60_000), 50.0);
}

fn trend_line(id: u64, locked: bool) -> Drawing {
    Drawing::TrendLine {
        id,
        a: anchor(60_000, 90.0),
        b: anchor(180_000, 110.0),
        locked,
        hidden: false,
        visibility: Visibility::ALL,
        style: crate::model::drawing_style::DrawingStyle::trend_line(),
    }
}

#[test]
fn dragging_the_body_moves_every_anchor_and_keeps_the_id() {
    let data = candles();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let moved = edit::edited(
        &trend_line(7, false),
        Part::Body,
        pos2(300.0, 200.0),
        pos2(400.0, 200.0),
        anchor(0, 0.0),
        &proj,
    );
    let Drawing::TrendLine { id, a, b, .. } = moved else {
        panic!("variant changed");
    };
    assert_eq!(id, 7);
    assert_eq!(a.time_ms, 120_000);
    assert_eq!(b.time_ms, 240_000);
    assert!((a.price - 90.0).abs() < 1e-3 && (b.price - 110.0).abs() < 1e-3);
}

#[test]
fn dragging_a_handle_moves_only_that_anchor() {
    let data = candles();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let original = trend_line(1, false);
    let target = proj.anchor(pos2(550.0, 200.0));
    let moved = edit::edited(
        &original,
        Part::Anchor(1),
        Pos2::ZERO,
        pos2(550.0, 200.0),
        target,
        &proj,
    );
    let (before, after) = (original.anchors(), moved.anchors());
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1].time_ms, 300_000);
    assert!((after[1].price - 100.0).abs() < 1e-3);
}

#[test]
fn box_handles_resize_one_edge_and_flip_past_the_opposite_one() {
    let (a, b) = (anchor(100, 20.0), anchor(200, 10.0));
    // The top edge only changes the higher price.
    let (a2, b2) = edit::resize_box(a, b, BoxHandle::Top, anchor(0, 30.0));
    assert_eq!((a2.price, b2.price), (30.0, 10.0));
    assert_eq!((a2.time_ms, b2.time_ms), (100, 200));
    // The left edge only changes the earlier time.
    let (a3, b3) = edit::resize_box(a, b, BoxHandle::Left, anchor(50, 0.0));
    assert_eq!((a3.time_ms, b3.time_ms), (50, 200));
    assert_eq!((a3.price, b3.price), (20.0, 10.0));
    // A corner moves both axes; dragging past the opposite corner flips the box.
    let (a4, b4) = edit::resize_box(a, b, BoxHandle::BottomRight, anchor(60, 40.0));
    assert_eq!((a4.time_ms, b4.time_ms), (100, 60));
    assert_eq!((a4.price, b4.price), (20.0, 40.0));
}

#[test]
fn vertices_can_be_inserted_and_removed_but_not_below_the_minimum() {
    let path = Drawing::build(
        Tool::Path,
        1,
        &[anchor(0, 1.0), anchor(10, 2.0), anchor(20, 3.0)],
        Tool::Path.default_style(),
    )
    .unwrap();
    let longer = edit::with_inserted_vertex(&path, 0, anchor(5, 9.0)).unwrap();
    assert_eq!(longer.anchors()[1], anchor(5, 9.0));
    assert_eq!(longer.anchors().len(), 4);
    let shorter = edit::without_vertex(&longer, 1).unwrap();
    assert_eq!(shorter.anchors().len(), 3);
    let two = edit::without_vertex(&shorter, 0).unwrap();
    assert!(
        edit::without_vertex(&two, 0).is_none(),
        "a path keeps two vertices"
    );
    assert!(edit::with_inserted_vertex(&path, 9, anchor(0, 0.0)).is_none());
    let rect = Drawing::build(
        Tool::Rectangle,
        2,
        &[anchor(0, 1.0), anchor(10, 2.0)],
        Tool::Rectangle.default_style(),
    )
    .unwrap();
    assert!(edit::with_inserted_vertex(&rect, 0, anchor(1, 1.0)).is_none());
    assert!(edit::without_vertex(&rect, 0).is_none());
    // A closed polyline keeps three.
    let mut closed = Drawing::build(
        Tool::Polyline,
        3,
        &[anchor(0, 1.0), anchor(10, 2.0), anchor(20, 1.5)],
        Tool::Polyline.default_style(),
    )
    .unwrap();
    if let Some(Geometry::Polyline { closed, .. }) = closed.geometry_mut() {
        *closed = true;
    }
    assert!(edit::without_vertex(&closed, 0).is_none());
}

#[test]
fn a_duplicate_is_offset_unlocked_and_has_its_own_id() {
    let data = candles();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let original = trend_line(4, true);
    let copy = edit::duplicate(&original, 9, &proj);
    assert_eq!(copy.id(), 9);
    assert!(!copy.locked());
    let (before, after) = (original.anchors(), copy.anchors());
    let moved = proj.point(after[0]) - proj.point(before[0]);
    assert!((moved - edit::DUPLICATE_OFFSET).length() < 0.5, "{moved:?}");
}

// MARK: The magnet

#[test]
fn the_strong_magnet_takes_the_nearest_price_of_the_nearest_bar() {
    let data = candles();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    // Near the high of the third bar.
    let near_high = pos2(250.0, scale.y(109.0));
    let snapped = snap::snap(near_high, &proj, MagnetMode::Strong).unwrap();
    assert_eq!((snapped.time_ms, snapped.price), (120_000, 110.0));
    // Far from every price of the bar, strong still snaps, weak does not.
    let between = pos2(250.0, scale.y(97.0));
    assert!(snap::snap(between, &proj, MagnetMode::Strong).is_some());
    assert!(snap::snap(between, &proj, MagnetMode::Weak).is_none());
    // Weak takes it once it is close enough.
    let close = pos2(255.0, scale.y(105.0) + 6.0);
    assert_eq!(
        snap::snap(close, &proj, MagnetMode::Weak).map(|snapped| snapped.price),
        Some(105.0)
    );
    assert!(snap::snap(near_high, &proj, MagnetMode::Off).is_none());
}

#[test]
fn the_magnet_ignores_space_beyond_the_data_and_empty_charts() {
    let data = candles();
    let scale = scale(false);
    let mut proj = projection(&data, &scale);
    proj.lead = 0.0;
    // The data ends at x = 1000 (ten 100 px slots); further right is open space.
    assert!(snap::snap(pos2(1200.0, 200.0), &proj, MagnetMode::Strong).is_none());
    assert!(snap::snap(pos2(-150.0, 200.0), &proj, MagnetMode::Strong).is_none());
    let none: Vec<Kline> = Vec::new();
    let empty = projection(&none, &scale);
    assert!(snap::snap(pos2(100.0, 100.0), &empty, MagnetMode::Strong).is_none());
}

#[test]
fn the_magnet_works_on_a_log_scale() {
    let data = candles();
    let scale = scale(true);
    let proj = projection(&data, &scale);
    let snapped = snap::snap(pos2(150.0, scale.y(91.0)), &proj, MagnetMode::Strong).unwrap();
    assert_eq!((snapped.time_ms, snapped.price), (60_000, 90.0));
}

// MARK: A rig that plays the workspace

struct Rig {
    ctx: egui::Context,
    time: f64,
    layer: DrawingLayer,
    data: Vec<Kline>,
    scale: PriceScale,
    drawings: Vec<Drawing>,
    selected: Vec<u64>,
    tool: Tool,
    next_id: u64,
    magnet: MagnetMode,
    keep_drawing: bool,
    hide_all: bool,
    timeframe: Timeframe,
    modifiers: Modifiers,
    sent_modifiers: Modifiers,
    focused: bool,
    /// How many undo entries the workspace would have pushed.
    undo_steps: usize,
    escape_consumed: bool,
    opened: Option<u64>,
    style_clipboard: Option<(Tool, DrawingStyle)>,
    style_copied: Option<(Tool, DrawingStyle)>,
    background_click: Option<Pos2>,
    keys_blocked: bool,
    reveal_all: bool,
}

impl Rig {
    fn new() -> Self {
        let mut rig = Self {
            ctx: egui::Context::default(),
            time: 0.0,
            layer: DrawingLayer::default(),
            data: candles(),
            scale: scale(false),
            drawings: Vec::new(),
            selected: Vec::new(),
            tool: Tool::Cursor,
            next_id: 1,
            magnet: MagnetMode::Off,
            keep_drawing: true,
            hide_all: false,
            timeframe: Timeframe::Day,
            modifiers: Modifiers::NONE,
            sent_modifiers: Modifiers::NONE,
            focused: true,
            undo_steps: 0,
            escape_consumed: false,
            opened: None,
            style_clipboard: None,
            style_copied: None,
            background_click: None,
            keys_blocked: false,
            reveal_all: false,
        };
        // egui hit-tests against the previous frame's widgets.
        rig.settle();
        rig
    }

    fn with_tool(tool: Tool) -> Self {
        let mut rig = Self::new();
        rig.tool = tool;
        rig
    }

    fn proj(&self) -> Projection<'_> {
        projection(&self.data, &self.scale)
    }

    fn px(&self, anchor: Anchor) -> Pos2 {
        self.proj().point(anchor)
    }

    fn frame(&mut self, mut events: Vec<Event>) {
        self.time += 0.1;
        if self.modifiers != self.sent_modifiers {
            events.insert(0, Event::ModifiersChanged(self.modifiers));
            self.sent_modifiers = self.modifiers;
        }
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 400.0))),
            time: Some(self.time),
            focused: self.focused,
            events,
            ..Default::default()
        };
        let mut frame = DrawingFrame::new(&self.drawings, self.tool, None, self.next_id)
            .with_selection(self.selected.clone())
            .with_view(self.timeframe, self.hide_all)
            .with_behavior(self.magnet, self.keep_drawing)
            .with_style_clipboard(self.style_clipboard)
            .with_keys_blocked(self.keys_blocked);
        let (layer, data, scale) = (&mut self.layer, &self.data, &self.scale);
        let mut output = self.ctx.run_ui(input, |ui| {
            let response = ui.allocate_rect(PLOT, Sense::click_and_drag());
            let proj = projection(data, scale);
            layer.update(ui, &response, &proj, &mut frame);
            layer.paint(&ui.painter().clone(), &proj, &frame);
            layer.overlay(ui, &proj, &mut frame);
        });
        output.textures_delta.clear();
        self.next_id = frame.next_id;
        if let Some(select) = frame.select.take() {
            self.selected = select;
        }
        self.escape_consumed |= frame.escape_consumed;
        if frame.disarm {
            self.tool = Tool::Cursor;
        }
        if frame.open_properties.is_some() {
            self.opened = frame.open_properties;
        }
        if frame.style_copied.is_some() {
            self.style_copied = frame.style_copied;
        }
        self.background_click = self.background_click.or(frame.background_click);
        self.reveal_all |= frame.reveal_all;
        let commands = std::mem::take(&mut frame.commands);
        drop(frame);
        self.apply(commands);
    }

    /// What `ChartWorkspace::apply_drawing_commands` does, minus the undo stack's contents.
    fn apply(&mut self, commands: Vec<DrawingCommand>) {
        if crate::model::drawings::apply_commands(&mut self.drawings, commands) {
            self.undo_steps += 1;
        }
    }

    /// A point on the drawing's body that the floating toolbar does not cover.
    fn body(&mut self, id: u64) -> Pos2 {
        let screen = self.screen_of(id);
        let toolbar = self
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::new("drawing-toolbar")))
            .map(|rect| rect.expand(10.0));
        body_point(&screen, toolbar)
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }

    fn button(&self, at: Pos2, pressed: bool, button: PointerButton) -> Event {
        Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: self.modifiers,
        }
    }

    fn move_to(&mut self, at: Pos2) {
        self.frame(vec![Event::PointerMoved(at)]);
    }

    fn press(&mut self, at: Pos2) {
        let event = self.button(at, true, PointerButton::Primary);
        self.frame(vec![event]);
    }

    fn release(&mut self, at: Pos2) {
        let event = self.button(at, false, PointerButton::Primary);
        self.frame(vec![event]);
    }

    fn click(&mut self, at: Pos2) {
        self.move_to(at);
        self.press(at);
        self.release(at);
    }

    fn double_click(&mut self, at: Pos2) {
        self.click(at);
        self.press(at);
        self.release(at);
    }

    fn right_click(&mut self, at: Pos2) {
        self.move_to(at);
        let down = self.button(at, true, PointerButton::Secondary);
        self.frame(vec![down]);
        let up = self.button(at, false, PointerButton::Secondary);
        self.frame(vec![up]);
    }

    /// A press, a path of moves, a release.
    fn drag_path(&mut self, path: &[Pos2]) {
        self.move_to(path[0]);
        self.press(path[0]);
        for point in &path[1..] {
            self.move_to(*point);
        }
        self.release(*path.last().unwrap());
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        let delta = to - from;
        let steps = ((delta.length() / 10.0).ceil() as usize).max(2);
        let path: Vec<Pos2> = (0..=steps)
            .map(|step| from + delta * (step as f32 / steps as f32))
            .collect();
        self.drag_path(&path);
    }

    fn key(&mut self, key: Key) {
        let modifiers = self.modifiers;
        let event = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.frame(vec![event(true)]);
        self.frame(vec![event(false)]);
    }

    fn screen_of(&mut self, id: u64) -> Arc<Screen> {
        let drawing = self.drawings.iter().find(|d| d.id() == id).unwrap().clone();
        let proj = projection(&self.data, &self.scale);
        self.layer.screen(&drawing, &proj).expect("screen shape")
    }

    fn last(&self) -> &Drawing {
        self.drawings.last().expect("a drawing")
    }

    /// The gesture that places `tool` with `POINTS`, by clicks.
    fn place(&mut self, tool: Tool) {
        self.tool = tool;
        match tool.spec().placement {
            Placement::Freehand => {
                let path = [POINTS[0], pos2(260.0, 150.0), pos2(330.0, 190.0), POINTS[1]];
                self.drag_path(&path);
            }
            Placement::Single => self.click(POINTS[0]),
            Placement::Points(count) => {
                for point in &POINTS[..usize::from(count)] {
                    self.click(*point);
                }
            }
            Placement::Open { .. } => {
                for point in &POINTS[..3] {
                    self.click(*point);
                }
                self.double_click(POINTS[3]);
            }
        }
    }
}

/// A point on the shape's body that is inside the plot and clear of every handle: along a
/// stroke, or inside a solid.
fn body_point(screen: &Screen, avoid: Option<Rect>) -> Pos2 {
    let usable = |point: Pos2| {
        PLOT.shrink(6.0).contains(point)
            && avoid.is_none_or(|rect| !rect.contains(point))
            && screen
                .handles
                .iter()
                .all(|handle| handle.pos.distance(point) > 14.0)
    };
    let along = |path: &Vec<Pos2>| {
        let count = path.len() - 1;
        (0..count)
            .flat_map(|step| {
                let index = (count / 3 + step) % count;
                [0.3, 0.5, 0.7].map(|t| path[index].lerp(path[index + 1], t))
            })
            .find(|point| usable(*point))
    };
    if let Some(point) = screen
        .lines
        .iter()
        .chain(screen.styled.iter().map(|styled| &styled.path))
        .filter(|path| path.len() >= 2)
        .find_map(along)
    {
        return point;
    }
    for path in &screen.outlines {
        let mut closed = path.clone();
        closed.push(path[0]);
        if let Some(point) = along(&closed) {
            return point;
        }
    }
    let solid = &screen.solids[0];
    let centroid = solid
        .iter()
        .fold(Pos2::ZERO, |sum, point| sum + point.to_vec2())
        / solid.len() as f32;
    let centre_line = solid[0].lerp(solid[3], 0.5);
    [centroid, centre_line]
        .into_iter()
        .find(|point| geo::polygon_contains(solid, *point))
        .unwrap_or(centroid)
}

// MARK: Every tool, whole lifecycle

#[test]
fn every_tool_is_placed_with_one_gesture_and_one_undo_step() {
    for tool in Tool::GROUPED {
        let mut rig = Rig::new();
        rig.place(tool);
        assert_eq!(rig.drawings.len(), 1, "{tool:?} did not commit");
        assert_eq!(rig.last().tool(), Some(tool));
        assert_eq!(rig.undo_steps, 1, "{tool:?}: one gesture, one step");
        assert_eq!(
            rig.selected,
            vec![rig.last().id()],
            "{tool:?} selects what it made"
        );
        assert!(
            rig.layer.gesture.is_none(),
            "{tool:?} left a gesture behind"
        );
        assert!(!rig.layer.claims_drag());
        assert_eq!(rig.tool, tool, "keep drawing stays on the tool");
    }
}

#[test]
fn the_legacy_tools_still_place_as_before() {
    for tool in [Tool::TrendLine, Tool::Fib, Tool::Ruler] {
        let mut rig = Rig::with_tool(tool);
        rig.click(POINTS[0]);
        assert!(
            rig.layer.claims_drag(),
            "{tool:?} waits for its second click"
        );
        rig.click(POINTS[1]);
        assert_eq!(rig.drawings.len(), 1, "{tool:?}");
        assert_eq!(rig.last().tool(), Some(tool));
        // A drag places both ends at once.
        let mut dragged = Rig::with_tool(tool);
        dragged.drag(POINTS[0], POINTS[2]);
        assert_eq!(dragged.drawings.len(), 1, "{tool:?} by drag");
        assert!(dragged.layer.gesture.is_none());
    }
}

#[test]
fn the_ruler_stays_out_of_the_undo_history() {
    let mut rig = Rig::with_tool(Tool::Ruler);
    rig.click(POINTS[0]);
    rig.click(POINTS[1]);
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, 0);
}

#[test]
fn a_two_point_shape_can_also_be_dragged_out() {
    for tool in [
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Circle,
        Tool::Arrow,
        Tool::ArrowMarker,
    ] {
        let mut rig = Rig::with_tool(tool);
        rig.drag(POINTS[0], POINTS[2]);
        assert_eq!(rig.drawings.len(), 1, "{tool:?}");
        assert_eq!(rig.undo_steps, 1);
    }
    // Three-point shapes take the first two by drag and the third by click.
    for tool in [
        Tool::Triangle,
        Tool::Arc,
        Tool::Curve,
        Tool::RotatedRectangle,
    ] {
        let mut rig = Rig::with_tool(tool);
        rig.drag(POINTS[0], POINTS[2]);
        assert_eq!(
            rig.drawings.len(),
            0,
            "{tool:?} is not done after two anchors"
        );
        assert!(rig.layer.claims_drag());
        rig.click(POINTS[1]);
        assert_eq!(rig.drawings.len(), 1, "{tool:?}");
        assert_eq!(rig.undo_steps, 1);
    }
    let mut double = Rig::with_tool(Tool::DoubleCurve);
    double.drag(POINTS[0], POINTS[2]);
    double.click(POINTS[1]);
    assert!(double.drawings.is_empty());
    double.click(POINTS[3]);
    assert_eq!(double.drawings.len(), 1);
}

#[test]
fn escape_cancels_an_unfinished_drawing_without_an_undo_entry() {
    for tool in Tool::GROUPED {
        let placement = tool.spec().placement;
        if matches!(placement, Placement::Freehand | Placement::Single) {
            continue;
        }
        let mut rig = Rig::with_tool(tool);
        rig.click(POINTS[0]);
        assert!(rig.layer.claims_drag(), "{tool:?} is waiting");
        rig.move_to(POINTS[1]);
        rig.key(Key::Escape);
        assert!(rig.layer.gesture.is_none(), "{tool:?}");
        assert!(rig.escape_consumed, "{tool:?} spends the key");
        assert!(rig.drawings.is_empty());
        assert_eq!(rig.undo_steps, 0, "{tool:?}: a cancelled draft is no edit");
        assert_eq!(rig.tool, tool, "the tool stays armed");
    }
}

#[test]
fn switching_tools_abandons_the_draft() {
    let mut rig = Rig::with_tool(Tool::Rectangle);
    rig.click(POINTS[0]);
    assert!(rig.layer.claims_drag());
    rig.tool = Tool::Triangle;
    rig.move_to(POINTS[1]);
    assert!(rig.layer.gesture.is_none());
    rig.tool = Tool::Cursor;
    rig.settle();
    assert!(rig.drawings.is_empty() && rig.undo_steps == 0);
}

#[test]
fn losing_focus_or_the_card_being_unseen_abandons_the_gesture() {
    let mut rig = Rig::with_tool(Tool::Arc);
    rig.click(POINTS[0]);
    assert!(rig.layer.claims_drag());
    rig.focused = false;
    rig.frame(Vec::new());
    assert!(rig.layer.gesture.is_none(), "focus loss");
    rig.focused = true;
    rig.settle();

    rig.click(POINTS[0]);
    assert!(rig.layer.claims_drag());
    // Frames pass without this card running (another tab is showing).
    for _ in 0..3 {
        let _ = rig.ctx.run_ui(RawInput::default(), |_| {});
    }
    rig.frame(Vec::new());
    assert!(rig.layer.gesture.is_none(), "unseen card");
    assert!(rig.drawings.is_empty() && rig.undo_steps == 0);
}

#[test]
fn a_right_button_drag_never_starts_a_drawing() {
    let mut rig = Rig::with_tool(Tool::Rectangle);
    rig.move_to(POINTS[0]);
    let down = rig.button(POINTS[0], true, PointerButton::Secondary);
    rig.frame(vec![down]);
    rig.move_to(POINTS[1]);
    rig.move_to(POINTS[2]);
    let up = rig.button(POINTS[2], false, PointerButton::Secondary);
    rig.frame(vec![up]);
    assert!(rig.layer.gesture.is_none());
    assert!(rig.drawings.is_empty());
}

#[test]
fn keep_drawing_off_returns_to_the_crosshair_after_each_drawing() {
    for tool in [Tool::Rectangle, Tool::Brush, Tool::ArrowMarkUp, Tool::Path] {
        let mut rig = Rig::new();
        rig.keep_drawing = false;
        rig.place(tool);
        assert_eq!(rig.drawings.len(), 1, "{tool:?}");
        assert_eq!(rig.tool, Tool::Cursor, "{tool:?} disarms");
    }
    // The ruler never counts: it is a measurement.
    let mut rig = Rig::with_tool(Tool::Ruler);
    rig.keep_drawing = false;
    rig.click(POINTS[0]);
    rig.click(POINTS[1]);
    assert_eq!(rig.tool, Tool::Ruler);
}

#[test]
fn every_placed_tool_can_be_selected_moved_and_deleted() {
    for tool in Tool::GROUPED {
        let mut rig = Rig::new();
        rig.place(tool);
        let id = rig.last().id();
        rig.tool = Tool::Cursor;
        rig.selected.clear();
        let before = rig.last().anchors();
        let steps = rig.undo_steps;
        let at = rig.body(id);

        rig.click(at);
        assert_eq!(
            rig.selected,
            vec![id],
            "{tool:?} is not selectable at {at:?}"
        );
        // Clicking empty space clears the selection (the top right corner: a pitchfork's bands
        // run on to the bottom right).
        rig.click(pos2(940.0, 30.0));
        assert!(rig.selected.is_empty(), "{tool:?}");
        rig.click(at);
        // Selecting floats the toolbar over the shape: grab it where the toolbar is not.
        let at = rig.body(id);

        // Drag the body: every anchor moves, in one step.
        rig.drag(at, at + vec2(40.0, 25.0));
        assert_eq!(rig.undo_steps, steps + 1, "{tool:?} move is one step");
        let after = rig.last().anchors();
        assert_eq!(after.len(), before.len());
        assert!(after != before, "{tool:?} did not move");
        for (old, new) in before.iter().zip(&after) {
            assert!(
                new.time_ms != old.time_ms || new.price != old.price,
                "{tool:?}"
            );
        }

        // Delete removes it, in one step.
        rig.key(Key::Delete);
        assert!(rig.drawings.is_empty(), "{tool:?}");
        assert_eq!(rig.undo_steps, steps + 2);
    }
}

#[test]
fn every_handle_of_every_tool_reshapes_it() {
    for tool in Tool::GROUPED {
        let mut rig = Rig::new();
        rig.place(tool);
        let id = rig.last().id();
        rig.tool = Tool::Cursor;
        rig.selected = vec![id];
        rig.settle();
        let handle_count = |rig: &mut Rig| {
            rig.screen_of(id)
                .handles
                .iter()
                .filter(|spot| !matches!(spot.part, Part::Body))
                .count()
        };
        let count = handle_count(&mut rig);
        if matches!(tool, Tool::Brush | Tool::Highlighter) {
            assert_eq!(count, 0, "a stroke has no handles");
            continue;
        }
        assert!(count > 0, "{tool:?} offers no handle");
        for index in 0..count {
            // The shape changes under each drag, so its handles are read afresh.
            let (part, pos) = rig
                .screen_of(id)
                .handles
                .iter()
                .filter(|spot| !matches!(spot.part, Part::Body))
                .map(|spot| (spot.part, spot.pos))
                .nth(index)
                .unwrap();
            // A handle that sits off the plot (a channel's second edge, with this test's wide
            // third click) cannot be pressed.
            if !PLOT.contains(pos) {
                continue;
            }
            let before = rig.last().clone();
            let steps = rig.undo_steps;
            // Away from the other handles, inside the plot.
            let to = if pos.x < 700.0 {
                pos + vec2(60.0, 12.0)
            } else {
                pos - vec2(60.0, 12.0)
            };
            rig.drag(pos, to);
            assert_eq!(rig.undo_steps, steps + 1, "{tool:?} {part:?}");
            assert!(*rig.last() != before, "{tool:?} {part:?} changed nothing");
            assert_eq!(rig.last().tool(), before.tool());
            assert_eq!(rig.last().id(), id);
            rig.selected = vec![id];
            rig.settle();
        }
    }
}

#[test]
fn a_locked_drawing_selects_but_does_not_move() {
    let mut rig = Rig::new();
    rig.place(Tool::Rectangle);
    let id = rig.last().id();
    let mut locked = rig.last().clone();
    locked.set_locked(true);
    rig.drawings[0] = locked;
    rig.tool = Tool::Cursor;
    let at = rig.body(id);
    let steps = rig.undo_steps;
    rig.drag(at, at + vec2(50.0, 30.0));
    assert_eq!(rig.selected, vec![id]);
    assert_eq!(rig.undo_steps, steps, "no edit");
    rig.key(Key::Delete);
    assert_eq!(rig.drawings.len(), 1, "a locked drawing is not deleted");
}

#[test]
fn hidden_and_out_of_timeframe_drawings_are_neither_drawn_nor_picked() {
    let mut rig = Rig::new();
    rig.place(Tool::Rectangle);
    let id = rig.last().id();
    rig.tool = Tool::Cursor;
    let at = rig.body(id);
    rig.selected.clear();

    rig.hide_all = true;
    rig.click(at);
    assert!(rig.selected.is_empty(), "hide all");
    rig.hide_all = false;

    let mut hidden = rig.last().clone();
    hidden.set_hidden(true);
    rig.drawings[0] = hidden;
    rig.click(at);
    assert!(rig.selected.is_empty(), "hidden");

    let mut weekly = rig.last().clone();
    weekly.set_hidden(false);
    weekly.set_visibility(Visibility::ALL.with(Timeframe::Day, false));
    rig.drawings[0] = weekly;
    rig.click(at);
    assert!(rig.selected.is_empty(), "not shown on this timeframe");
    rig.timeframe = Timeframe::Week;
    rig.click(at);
    assert_eq!(rig.selected, vec![id]);
    // Nothing was rewritten along the way.
    assert_eq!(rig.undo_steps, 1);
}

#[test]
fn shift_click_builds_a_selection_that_moves_together_in_one_step() {
    let mut rig = Rig::new();
    rig.place(Tool::Rectangle);
    rig.place(Tool::Triangle);
    // Place them apart: the second used the same points, so move it first.
    let (first, second) = (rig.drawings[0].id(), rig.drawings[1].id());
    rig.tool = Tool::Cursor;
    rig.selected = vec![second];
    let at = rig.body(second);
    rig.drag(at, at + vec2(0.0, 40.0));
    rig.selected.clear();
    let steps = rig.undo_steps;

    let a = rig.body(first);
    let b = rig.body(second);
    rig.click(a);
    rig.modifiers = Modifiers::SHIFT;
    rig.click(b);
    rig.modifiers = Modifiers::NONE;
    assert_eq!(rig.selected.len(), 2, "{:?}", rig.selected);
    let before: Vec<_> = rig.drawings.iter().map(Drawing::anchors).collect();
    rig.drag(b, b + vec2(60.0, 0.0));
    assert_eq!(rig.undo_steps, steps + 1, "moving a selection is one step");
    for (index, old) in before.iter().enumerate() {
        assert!(
            &rig.drawings[index].anchors() != old,
            "drawing {index} did not move"
        );
    }
    // Shift-click again removes one from the selection.
    rig.modifiers = Modifiers::SHIFT;
    let b = rig.body(second);
    rig.click(b);
    rig.modifiers = Modifiers::NONE;
    assert_eq!(rig.selected, vec![first]);
}

#[test]
fn double_clicking_a_drawing_asks_for_its_properties() {
    let mut rig = Rig::new();
    rig.place(Tool::Ellipse);
    let id = rig.last().id();
    rig.tool = Tool::Cursor;
    let at = rig.body(id);
    rig.double_click(at);
    assert_eq!(rig.opened, Some(id));
    // Enter does the same for the selection.
    rig.opened = None;
    rig.key(Key::Enter);
    assert_eq!(rig.opened, Some(id));
}

// MARK: Open shapes

#[test]
fn a_path_finishes_on_double_click_enter_escape_or_not_at_all() {
    // Double-click: four vertices, not five.
    let mut rig = Rig::new();
    rig.place(Tool::Path);
    assert_eq!(rig.last().anchors().len(), 4);

    // Enter.
    let mut enter = Rig::with_tool(Tool::Path);
    for point in &POINTS[..3] {
        enter.click(*point);
    }
    assert!(enter.drawings.is_empty());
    enter.move_to(pos2(300.0, 300.0));
    enter.key(Key::Enter);
    assert_eq!(enter.drawings.len(), 1);
    assert_eq!(enter.last().anchors().len(), 3);

    // Escape with something to show finishes it; with a single vertex it abandons it.
    let mut escape = Rig::with_tool(Tool::Path);
    escape.click(POINTS[0]);
    escape.click(POINTS[1]);
    escape.key(Key::Escape);
    assert_eq!(escape.drawings.len(), 1);
    assert!(escape.escape_consumed);
    let mut lone = Rig::with_tool(Tool::Path);
    lone.click(POINTS[0]);
    lone.key(Key::Escape);
    assert!(lone.drawings.is_empty());
    assert_eq!(lone.undo_steps, 0);
}

#[test]
fn backspace_takes_back_the_last_vertex() {
    let mut rig = Rig::with_tool(Tool::Path);
    for point in &POINTS[..3] {
        rig.click(*point);
    }
    rig.key(Key::Backspace);
    rig.move_to(pos2(300.0, 300.0));
    rig.key(Key::Enter);
    assert_eq!(rig.last().anchors().len(), 2);
    let mut empty = Rig::with_tool(Tool::Polyline);
    empty.click(POINTS[0]);
    empty.key(Key::Backspace);
    assert!(empty.layer.gesture.is_none());
}

#[test]
fn a_polyline_closes_on_its_first_vertex_and_a_right_click_leaves_it_open() {
    let mut closed = Rig::with_tool(Tool::Polyline);
    for point in &POINTS[..3] {
        closed.click(*point);
    }
    closed.click(POINTS[0] + vec2(3.0, 2.0));
    let Some(Geometry::Polyline {
        closed: is_closed,
        points,
    }) = closed.last().geometry().cloned()
    else {
        panic!("polyline");
    };
    assert!(is_closed);
    assert_eq!(points.len(), 3, "the closing click adds no vertex");

    let mut open = Rig::with_tool(Tool::Polyline);
    for point in &POINTS[..3] {
        open.click(*point);
    }
    open.right_click(pos2(700.0, 300.0));
    let Some(Geometry::Polyline {
        closed: is_closed, ..
    }) = open.last().geometry().cloned()
    else {
        panic!("polyline");
    };
    assert!(!is_closed);
    // A path never closes on its first vertex.
    let mut path = Rig::with_tool(Tool::Path);
    for point in &POINTS[..3] {
        path.click(*point);
    }
    path.click(POINTS[0]);
    assert!(path.drawings.is_empty(), "the click added a vertex instead");
}

#[test]
fn a_path_stops_collecting_at_the_vertex_limit() {
    let mut rig = Rig::with_tool(Tool::Path);
    let spot = |index: usize| {
        pos2(
            100.0 + (index % 800) as f32,
            100.0 + (index / 800) as f32 * 7.0,
        )
    };
    rig.click(spot(0));
    // Fill the draft directly: a click per vertex would take thousands of frames.
    if let Some(Gesture::Draft(draft)) = &mut rig.layer.gesture {
        let proj = projection(&rig.data, &rig.scale);
        draft.placed = (0..MAX_POINTS - 1).map(|i| proj.anchor(spot(i))).collect();
    }
    rig.click(pos2(900.0, 390.0));
    assert_eq!(rig.drawings.len(), 1);
    assert!(rig.last().anchors().len() <= MAX_POINTS);
}

#[test]
fn vertices_are_added_by_double_click_on_a_selected_path_is_not_a_properties_request() {
    // Inserting a vertex arrives with the context menu; double-clicking opens properties.
    let mut rig = Rig::new();
    rig.place(Tool::Path);
    let id = rig.last().id();
    rig.tool = Tool::Cursor;
    let at = rig.body(id);
    rig.double_click(at);
    assert_eq!(rig.opened, Some(id));
}

// MARK: Shift, magnet and freehand

#[test]
fn shift_squares_a_rectangle_and_circles_an_ellipse_and_levels_a_line() {
    for tool in [Tool::Rectangle, Tool::Ellipse] {
        let mut rig = Rig::with_tool(tool);
        rig.modifiers = Modifiers::SHIFT;
        rig.click(POINTS[0]);
        rig.click(pos2(500.0, 260.0));
        let anchors = rig.last().anchors();
        let (a, b) = (rig.px(anchors[0]), rig.px(anchors[1]));
        let size = b - a;
        assert!(
            (size.x.abs() - size.y.abs()).abs() < 1.0,
            "{tool:?} {size:?}"
        );
    }
    let mut line = Rig::with_tool(Tool::Arrow);
    line.modifiers = Modifiers::SHIFT;
    line.click(POINTS[0]);
    line.click(pos2(500.0, 215.0));
    let anchors = line.last().anchors();
    let delta = line.px(anchors[1]) - line.px(anchors[0]);
    assert!(delta.y.abs() < 1.0, "the line snapped level: {delta:?}");
}

#[test]
fn the_magnet_snaps_placed_anchors_but_not_brush_samples() {
    let mut rig = Rig::with_tool(Tool::Arrow);
    rig.magnet = MagnetMode::Strong;
    rig.click(pos2(255.0, rig.scale.y(108.0)));
    rig.click(pos2(555.0, rig.scale.y(92.0)));
    let anchors = rig.last().anchors();
    assert_eq!((anchors[0].time_ms, anchors[0].price), (120_000, 110.0));
    assert_eq!((anchors[1].time_ms, anchors[1].price), (300_000, 90.0));

    // The invert key turns it off.
    let mut inverted = Rig::with_tool(Tool::Arrow);
    inverted.magnet = MagnetMode::Strong;
    inverted.modifiers = Modifiers::COMMAND;
    inverted.click(pos2(255.0, inverted.scale.y(108.0)));
    inverted.click(pos2(555.0, inverted.scale.y(92.0)));
    let free = inverted.last().anchors();
    assert!(free[0].price != 110.0 && free[0].price != 105.0);

    // A freehand stroke keeps the pointer's own samples.
    let mut brush = Rig::with_tool(Tool::Brush);
    brush.magnet = MagnetMode::Strong;
    brush.drag_path(&[pos2(220.0, 133.0), pos2(260.0, 191.0), pos2(310.0, 152.0)]);
    let prices: Vec<f64> = brush.last().anchors().iter().map(|a| a.price).collect();
    assert!(
        prices
            .iter()
            .all(|price| ![90.0, 100.0, 105.0, 110.0].contains(price))
    );
}

#[test]
fn a_brush_stroke_is_one_drawing_simplified_but_faithful() {
    let mut rig = Rig::with_tool(Tool::Brush);
    // A long, nearly straight wobbling stroke has many samples.
    let path: Vec<Pos2> = (0..120)
        .map(|i| pos2(100.0 + i as f32 * 5.0, 200.0 + (i as f32 * 0.7).sin() * 0.2))
        .collect();
    rig.drag_path(&path);
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, 1);
    let points = rig.last().anchors();
    assert!(points.len() >= 2 && points.len() < 20, "{}", points.len());
    let first = rig.px(points[0]);
    let last = rig.px(*points.last().unwrap());
    assert!(first.distance(path[0]) < 8.0 && last.distance(*path.last().unwrap()) < 8.0);
    // A click without a drag leaves a dot.
    let mut dot = Rig::with_tool(Tool::Highlighter);
    dot.click(POINTS[0]);
    assert_eq!(dot.drawings.len(), 1);
    assert_eq!(dot.last().anchors().len(), 1);
}

#[test]
fn a_barely_moved_two_point_press_is_a_click_not_a_collapsed_shape() {
    let mut rig = Rig::with_tool(Tool::Rectangle);
    rig.drag(POINTS[0], POINTS[0] + vec2(1.0, 0.0));
    assert!(rig.drawings.is_empty());
    assert!(rig.layer.claims_drag(), "it waits for the second click");
    // A second click on the same spot makes no rectangle and abandons the draft.
    rig.click(POINTS[0]);
    assert!(rig.drawings.is_empty());
    assert!(rig.layer.gesture.is_none());
    assert_eq!(rig.undo_steps, 0);
}

// MARK: Screen shapes

#[test]
fn a_shape_is_rebuilt_only_when_it_or_the_view_changes() {
    let mut rig = Rig::new();
    rig.place(Tool::Curve);
    let id = rig.last().id();
    let first = rig.screen_of(id);
    let again = rig.screen_of(id);
    assert!(Arc::ptr_eq(&first, &again), "unchanged: the cached shape");
    // A different view invalidates it.
    rig.data.push(rig.data[9].clone());
    let panned = rig.screen_of(id);
    assert!(!Arc::ptr_eq(&first, &panned));
    // So does editing the drawing.
    let mut edited = rig.last().clone();
    edited.anchors_mut()[2].price += 3.0;
    rig.drawings[0] = edited;
    let changed = rig.screen_of(id);
    assert!(!Arc::ptr_eq(&panned, &changed));
}

#[test]
fn shapes_survive_extreme_and_broken_anchors() {
    let mut rig = Rig::new();
    let wild = [
        anchor(i64::MAX / 4, 1e300),
        anchor(-i64::MAX / 4, -1e300),
        anchor(0, 0.0),
        anchor(0, 0.0),
    ];
    for tool in Tool::GROUPED {
        let count = match tool.spec().placement {
            Placement::Points(count) => usize::from(count),
            Placement::Single => 1,
            _ => 3,
        };
        let drawing = Drawing::Shape {
            id: 1,
            geometry: Geometry::from_anchors(tool, &wild[..count]).unwrap_or(Geometry::Path {
                points: wild[..2].to_vec(),
            }),
            style: tool.default_style(),
            text: "label".into(),
            locked: false,
            hidden: false,
            visibility: Visibility::ALL,
        };
        rig.drawings = vec![drawing];
        // Painting and hit-testing must neither panic nor hang.
        rig.selected = vec![1];
        rig.click(POINTS[0]);
        rig.move_to(POINTS[1]);
    }
    let nan = Drawing::Shape {
        id: 2,
        geometry: Geometry::Rectangle {
            a: anchor(0, f64::NAN),
            b: anchor(1, 1.0),
        },
        style: DrawingStyle::default(),
        text: String::new(),
        locked: false,
        hidden: false,
        visibility: Visibility::ALL,
    };
    rig.drawings = vec![nan, Drawing::Unreadable(serde_json::Value::Null)];
    rig.click(POINTS[2]);
    rig.settle();
}

#[test]
fn locked_drawings_report_locked_hits() {
    let data = candles();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let line = Drawing::TrendLine {
        id: 3,
        a: anchor(0, 100.0),
        b: anchor(540_000, 100.0),
        locked: true,
        hidden: false,
        visibility: Visibility::ALL,
        style: crate::model::drawing_style::DrawingStyle::trend_line(),
    };
    let y = scale.y(100.0);
    let frame = DrawingFrame::new(std::slice::from_ref(&line), Tool::Cursor, None, 1);
    let mut layer = DrawingLayer::default();
    let hit = layer
        .hit_test(&frame, pos2(500.0, y + 1.0), &proj, true)
        .expect("hit");
    assert_eq!((hit.id, hit.part, hit.locked), (3, Part::Body, true));
    let empty = DrawingFrame::new(&[], Tool::Cursor, None, 1);
    assert!(
        layer
            .hit_test(&empty, pos2(500.0, y), &proj, true)
            .is_none()
    );
}

#[test]
fn an_extended_trend_line_is_hit_beyond_its_anchors_and_a_plain_one_is_not() {
    let data = candles();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    let y = scale.y(100.0);
    let at = |extend_right: bool| {
        let mut style = crate::model::drawing_style::DrawingStyle::trend_line();
        style.extend_right = extend_right;
        Drawing::TrendLine {
            id: 3,
            a: anchor(0, 100.0),
            b: anchor(180_000, 100.0),
            locked: false,
            hidden: false,
            visibility: Visibility::ALL,
            style,
        }
    };
    let beyond = pos2(proj.plot.right() - 40.0, y + 1.0);
    let mut layer = DrawingLayer::default();
    for (extend, hit) in [(false, false), (true, true)] {
        let line = at(extend);
        let frame = DrawingFrame::new(std::slice::from_ref(&line), Tool::Cursor, None, 1);
        assert_eq!(
            layer.hit_test(&frame, beyond, &proj, true).is_some(),
            hit,
            "extend_right {extend}"
        );
    }
}

#[test]
fn styled_trend_lines_paint_in_every_dash_and_extension() {
    use crate::model::chart_type::LineDash;
    let ctx = egui::Context::default();
    let data = candles();
    let scale = scale(false);
    let proj = projection(&data, &scale);
    for dash in LineDash::ALL {
        for (left, right) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut style = crate::model::drawing_style::DrawingStyle::trend_line();
            style.stroke.dash = dash;
            style.stroke.color = [255, 193, 7, 200];
            style.extend_left = left;
            style.extend_right = right;
            let line = Drawing::TrendLine {
                id: 1,
                a: anchor(60_000, 99.0),
                b: anchor(240_000, 101.0),
                locked: false,
                hidden: false,
                visibility: Visibility::ALL,
                style,
            };
            let mut layer = DrawingLayer::default();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let painter = ui.painter().clone();
                for emphasized in [false, true] {
                    layer.paint_drawing(&painter, &proj, &line, emphasized, emphasized, false);
                }
            });
            output.textures_delta.clear();
        }
    }
}

#[test]
fn committing_assigns_sequential_ids_and_selects_the_new_drawing() {
    let mut rig = Rig::with_tool(Tool::Rectangle);
    rig.next_id = 5;
    rig.click(POINTS[0]);
    rig.click(POINTS[1]);
    rig.tool = Tool::Brush;
    rig.click(POINTS[2]);
    assert_eq!(
        rig.drawings.iter().map(Drawing::id).collect::<Vec<_>>(),
        vec![5, 6]
    );
    assert_eq!(rig.next_id, 7);
    assert_eq!(rig.selected, vec![6]);
    // The crosshair draws nothing.
    rig.tool = Tool::Cursor;
    rig.click(pos2(900.0, 380.0));
    assert_eq!(rig.next_id, 7);
}

#[test]
fn saved_drawings_reopen_as_the_same_shapes() {
    let mut rig = Rig::new();
    for tool in Tool::GROUPED {
        rig.place(tool);
    }
    let json = serde_json::to_string(&rig.drawings).unwrap();
    let back: Vec<Drawing> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, rig.drawings);
    assert_eq!(rig.drawings.len(), Tool::GROUPED.len());
}

// MARK: Menus and the toolbar

impl Rig {
    /// Runs a menu action on `ids` the way a picked row does, then settles one frame.
    fn run(&mut self, action: Action, ids: &[u64]) {
        let mut frame = DrawingFrame::new(&self.drawings, self.tool, None, self.next_id)
            .with_selection(self.selected.clone())
            .with_style_clipboard(self.style_clipboard);
        let proj = projection(&self.data, &self.scale);
        self.layer.run(action, ids, None, &mut frame, &proj);
        self.next_id = frame.next_id;
        if let Some(select) = frame.select.take() {
            self.selected = select;
        }
        if frame.style_copied.is_some() {
            self.style_copied = frame.style_copied;
        }
        self.background_click = self.background_click.or(frame.background_click);
        if frame.open_properties.is_some() {
            self.opened = frame.open_properties;
        }
        let commands = std::mem::take(&mut frame.commands);
        drop(frame);
        self.apply(commands);
    }

    fn ids(&self) -> Vec<u64> {
        self.drawings.iter().map(Drawing::id).collect()
    }

    fn rect_with(&mut self, offset: f32) -> u64 {
        self.tool = Tool::Rectangle;
        self.click(pos2(200.0 + offset, 200.0));
        self.click(pos2(300.0 + offset, 260.0));
        self.tool = Tool::Cursor;
        self.last().id()
    }
}

#[test]
fn duplicating_copies_each_selected_drawing_offset_and_selects_the_copies_in_one_step() {
    let mut rig = Rig::new();
    let a = rig.rect_with(0.0);
    let b = rig.rect_with(300.0);
    rig.selected = vec![a, b];
    let steps = rig.undo_steps;
    let before: Vec<_> = rig.drawings.iter().map(Drawing::anchors).collect();
    rig.run(Action::Duplicate, &[a, b]);
    assert_eq!(rig.drawings.len(), 4);
    assert_eq!(rig.undo_steps, steps + 1);
    assert_eq!(rig.selected, vec![3, 4], "the copies are selected");
    assert_eq!(rig.drawings[2].tool(), Some(Tool::Rectangle));
    assert!(rig.drawings[2].anchors() != before[0]);
    assert_eq!(
        rig.drawings[0].anchors(),
        before[0],
        "the originals stay put"
    );
    // A ruler is a measurement: it is not copied.
    let mut ruler = Rig::with_tool(Tool::Ruler);
    ruler.click(POINTS[0]);
    ruler.click(POINTS[1]);
    let id = ruler.last().id();
    ruler.run(Action::Duplicate, &[id]);
    assert_eq!(ruler.drawings.len(), 1);
}

/// The platform's primary key as the OS reports it: ⌘ on a Mac (`Modifiers::COMMAND` alone leaves
/// `mac_cmd` off), Ctrl elsewhere.
fn primary_key() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::default()
        }
    } else {
        Modifiers::CTRL
    }
}

#[test]
fn the_duplicate_shortcut_copies_the_selection() {
    let mut rig = Rig::new();
    let a = rig.rect_with(0.0);
    rig.selected = vec![a];
    rig.modifiers = primary_key();
    rig.key(Key::D);
    rig.modifiers = Modifiers::NONE;
    assert_eq!(rig.drawings.len(), 2);
    assert_eq!(rig.selected, vec![2]);
}

#[test]
fn lock_hide_and_unhide_are_edits_and_leave_the_drawing_in_place() {
    let mut rig = Rig::new();
    let a = rig.rect_with(0.0);
    let b = rig.rect_with(300.0);
    let steps = rig.undo_steps;
    rig.run(Action::ToggleLock, &[a, b]);
    assert!(rig.drawings.iter().all(Drawing::locked));
    rig.run(Action::ToggleLock, &[a, b]);
    assert!(rig.drawings.iter().all(|d| !d.locked()));
    rig.run(Action::Hide, &[a]);
    assert!(rig.drawings[0].hidden());
    assert_eq!(rig.drawings.len(), 2, "hidden is not deleted");
    assert!(rig.selected.is_empty());
    // Each was one step.
    assert_eq!(rig.undo_steps, steps + 3);
}

#[test]
fn deleting_skips_locked_drawings() {
    let mut rig = Rig::new();
    let a = rig.rect_with(0.0);
    let b = rig.rect_with(300.0);
    rig.run(Action::ToggleLock, &[b]);
    rig.run(Action::Delete, &[a, b]);
    assert_eq!(rig.ids(), vec![b]);
    assert_eq!(rig.drawings.len(), 1);
}

#[test]
fn bring_forward_and_send_backward_reorder_the_stack() {
    let mut rig = Rig::new();
    let a = rig.rect_with(0.0);
    let b = rig.rect_with(300.0);
    let c = rig.rect_with(600.0);
    rig.run(Action::Forward, &[a]);
    assert_eq!(rig.ids(), vec![b, a, c]);
    rig.run(Action::ToFront, &[a]);
    assert_eq!(rig.ids(), vec![b, c, a]);
    rig.run(Action::Backward, &[a]);
    assert_eq!(rig.ids(), vec![b, a, c]);
    rig.run(Action::ToBack, &[c]);
    assert_eq!(rig.ids(), vec![c, b, a]);
    // A group keeps its own order.
    rig.run(Action::ToFront, &[c, b]);
    assert_eq!(rig.ids(), vec![a, c, b]);
    // The topmost drawing is the one picked where two overlap.
    let mut overlap = Rig::new();
    let first = overlap.rect_with(0.0);
    let second = overlap.rect_with(0.0);
    let at = pos2(250.0, 200.0);
    overlap.click(at);
    assert_eq!(overlap.selected, vec![second]);
    overlap.run(Action::ToFront, &[first]);
    overlap.selected.clear();
    overlap.click(at);
    assert_eq!(overlap.selected, vec![first]);
}

#[test]
fn style_can_be_copied_and_pasted_onto_the_same_tool_only() {
    let mut rig = Rig::new();
    let a = rig.rect_with(0.0);
    let b = rig.rect_with(300.0);
    rig.tool = Tool::Ellipse;
    rig.click(pos2(200.0, 100.0));
    rig.click(pos2(300.0, 160.0));
    rig.tool = Tool::Cursor;
    let ellipse = rig.last().id();
    // Style a.
    let mut styled = rig.drawings[0].clone();
    let mut style = *styled.style().unwrap();
    style.stroke.width = 9.0;
    style.fill.color = [1, 2, 3, 40];
    styled.set_style(style);
    rig.drawings[0] = styled;

    rig.run(Action::CopyStyle, &[a]);
    let (tool, copied) = rig.style_copied.expect("copied");
    assert_eq!(tool, Tool::Rectangle);
    assert_eq!(copied.stroke.width, 9.0);
    rig.style_clipboard = rig.style_copied;

    let steps = rig.undo_steps;
    rig.run(Action::PasteStyle, &[b, ellipse]);
    assert_eq!(rig.undo_steps, steps + 1);
    assert_eq!(rig.drawings[1].style().unwrap().stroke.width, 9.0);
    assert_eq!(rig.drawings[1].style().unwrap().fill.color, [1, 2, 3, 40]);
    assert!(
        rig.drawings[2].style().unwrap().stroke.width != 9.0,
        "an ellipse is not a rectangle"
    );
    // A locked drawing keeps its style.
    rig.run(Action::ToggleLock, &[b]);
    let mut other = rig.drawings[1].clone();
    let mut restored = *other.style().unwrap();
    restored.stroke.width = 3.0;
    other.set_style(restored);
    rig.drawings[1] = other;
    rig.run(Action::PasteStyle, &[b]);
    assert_eq!(rig.drawings[1].style().unwrap().stroke.width, 3.0);
}

#[test]
fn the_menu_adds_and_removes_vertices_of_a_path() {
    let mut rig = Rig::new();
    rig.place(Tool::Path);
    rig.tool = Tool::Cursor;
    let id = rig.last().id();
    let count = rig.last().anchors().len();
    let midpoint = rig
        .px(rig.last().anchors()[0])
        .lerp(rig.px(rig.last().anchors()[1]), 0.5);
    let mut frame = DrawingFrame::new(&rig.drawings, Tool::Cursor, None, rig.next_id);
    let proj = projection(&rig.data, &rig.scale);
    rig.layer.run(
        Action::AddVertex,
        &[id],
        Some((Part::Body, midpoint)),
        &mut frame,
        &proj,
    );
    let commands = std::mem::take(&mut frame.commands);
    drop(frame);
    rig.apply(commands);
    assert_eq!(rig.last().anchors().len(), count + 1);
    let inserted = rig.px(rig.last().anchors()[1]);
    assert!(
        inserted.distance(midpoint) < 2.0,
        "{inserted:?} vs {midpoint:?}"
    );
    rig.run(Action::RemoveVertex(1), &[id]);
    assert_eq!(rig.last().anchors().len(), count);
    // Not below the minimum.
    for _ in 0..count {
        rig.run(Action::RemoveVertex(0), &[id]);
    }
    assert_eq!(rig.last().anchors().len(), 2);
}

#[test]
fn settings_ask_for_the_dialog_and_a_locked_drawing_has_none() {
    let mut rig = Rig::new();
    let a = rig.rect_with(0.0);
    let b = rig.rect_with(300.0);
    rig.run(Action::Settings, &[a]);
    assert_eq!(rig.opened, Some(a));
    rig.opened = None;
    rig.run(Action::Settings, &[a, b]);
    assert_eq!(rig.opened, None, "one drawing at a time");
}

#[test]
fn right_click_opens_a_menu_on_a_drawing_and_reports_the_background_for_the_chart_menu() {
    let mut rig = Rig::new();
    let id = rig.rect_with(0.0);
    let at = rig.body(id);
    rig.right_click(at);
    assert!(rig.layer.menu.is_some());
    assert!(rig.background_click.is_none());
    assert_eq!(
        rig.selected,
        vec![id],
        "the drawing under the pointer is selected"
    );
    rig.escape_consumed = false;
    rig.key(Key::Escape);
    assert!(rig.layer.menu.is_none());
    assert!(
        rig.escape_consumed,
        "Escape closed the menu and nothing else"
    );

    // Empty chart is the chart menu's: no drawing menu, and the click is handed over.
    rig.right_click(pos2(900.0, 380.0));
    assert!(rig.layer.menu.is_none());
    assert_eq!(rig.background_click, Some(pos2(900.0, 380.0)));
    // A press outside an open menu closes it.
    rig.right_click(at);
    assert!(rig.layer.menu.is_some());
    rig.click(pos2(100.0, 30.0));
    assert!(rig.layer.menu.is_none());
    assert_eq!(rig.undo_steps, 1, "opening and closing menus edits nothing");
}

#[test]
fn right_click_does_nothing_while_drawing_with_a_tool() {
    let mut rig = Rig::with_tool(Tool::Rectangle);
    rig.right_click(POINTS[0]);
    assert!(rig.layer.menu.is_none());
}

#[test]
fn the_toolbar_floats_over_a_selection_only_while_nothing_is_in_progress() {
    let mut rig = Rig::new();
    let id = rig.rect_with(0.0);
    rig.selected = vec![id];
    rig.settle();
    let toolbar = egui::Id::new("drawing-toolbar");
    let shown = |rig: &Rig| rig.ctx.memory(|memory| memory.area_rect(toolbar)).is_some();
    assert!(shown(&rig), "a selected drawing has its toolbar");
    let bounds = rig.screen_of(id).bounds;
    let rect = rig.ctx.memory(|memory| memory.area_rect(toolbar)).unwrap();
    assert!(
        rect.bottom() <= bounds.top() + 1.0 || rect.top() >= bounds.bottom() - 1.0,
        "clear of the shape"
    );
    assert!(PLOT.contains_rect(rect), "{rect:?}");
    // Every tool's selection gets one without trouble.
    for tool in Tool::GROUPED {
        let mut rig = Rig::new();
        rig.place(tool);
        rig.tool = Tool::Cursor;
        rig.settle();
        assert!(
            rig.ctx.memory(|memory| memory.area_rect(toolbar)).is_some(),
            "{tool:?}"
        );
    }
}

#[test]
fn a_dialog_that_has_the_keyboard_keeps_return_delete_and_duplicate() {
    let mut rig = Rig::new();
    let id = rig.rect_with(0.0);
    rig.selected = vec![id];
    rig.keys_blocked = true;
    rig.key(Key::Enter);
    assert_eq!(rig.opened, None, "Return belongs to the dialog");
    rig.key(Key::Delete);
    rig.key(Key::Backspace);
    assert_eq!(rig.drawings.len(), 1, "Delete belongs to the dialog");
    rig.modifiers = primary_key();
    rig.key(Key::D);
    rig.modifiers = Modifiers::NONE;
    assert_eq!(rig.drawings.len(), 1, "so does the duplicate shortcut");
    // With the dialog gone they work again.
    rig.keys_blocked = false;
    rig.key(Key::Enter);
    assert_eq!(rig.opened, Some(id));
}

#[test]
fn placing_a_drawing_while_everything_is_hidden_shows_everything_again() {
    let mut rig = Rig::with_tool(Tool::Rectangle);
    rig.hide_all = true;
    rig.click(POINTS[0]);
    rig.click(POINTS[1]);
    assert_eq!(rig.drawings.len(), 1);
    assert!(
        rig.reveal_all,
        "or the new drawing would be invisible and unreachable"
    );
    let mut shown = Rig::with_tool(Tool::Rectangle);
    shown.click(POINTS[0]);
    shown.click(POINTS[1]);
    assert!(!shown.reveal_all);
}

#[test]
fn a_line_at_one_price_offers_an_alert_there_and_a_sloped_line_none() {
    for tool in [Tool::HorizontalLine, Tool::HorizontalRay, Tool::Crossline] {
        let mut rig = Rig::with_tool(tool);
        rig.click(POINTS[0]);
        let drawing = rig.last().clone();
        assert!(menu::is_price_line(&drawing), "{tool:?}");
        let mut frame = DrawingFrame::new(&rig.drawings, Tool::Cursor, None, 10);
        let proj = projection(&rig.data, &rig.scale);
        rig.layer
            .run(Action::Alert, &[drawing.id()], None, &mut frame, &proj);
        assert_eq!(
            frame.alert_price,
            Some(drawing.anchors()[0].price),
            "{tool:?}"
        );
        assert!(
            frame.commands.is_empty(),
            "asking for an alert edits nothing"
        );
    }
    for tool in [
        Tool::TrendLine,
        Tool::Ray,
        Tool::VerticalLine,
        Tool::Pitchfork,
    ] {
        let mut rig = Rig::new();
        rig.place(tool);
        assert!(!menu::is_price_line(rig.last()), "{tool:?}");
    }
}

// MARK: Text on a line

/// A trend line, placed and selected, with the crosshair back and its text place painted.
fn line_awaiting_text() -> (Rig, u64) {
    let mut rig = Rig::with_tool(Tool::TrendLine);
    rig.click(POINTS[0]);
    rig.click(POINTS[1]);
    rig.tool = Tool::Cursor;
    let id = rig.last().id();
    rig.selected = vec![id];
    rig.settle();
    (rig, id)
}

/// Where the text place of drawing `id` is, as the painter last laid it out.
fn text_place(rig: &Rig, id: u64) -> Pos2 {
    rig.layer.text_boxes.get(&id).expect("a text place").center
}

/// Types `text` into the editor that is open and presses Return.
fn type_and_keep(rig: &mut Rig, text: &str) {
    rig.settle();
    rig.frame(vec![Event::Text(text.to_owned())]);
    rig.settle();
    rig.key(Key::Enter);
    rig.settle();
}

#[test]
fn a_selected_line_invites_text_and_a_click_on_the_invitation_opens_the_editor() {
    let (mut rig, id) = line_awaiting_text();
    assert!(!rig.layer.is_editing_text());
    let place = text_place(&rig, id);
    rig.click(place);
    assert!(rig.layer.is_editing_text());
    assert_eq!(
        rig.selected,
        vec![id],
        "the line stays selected while it is typed on"
    );
    assert_eq!(rig.drawings.len(), 1);
}

#[test]
fn typed_text_is_kept_by_return_as_one_undo_step_and_the_line_keeps_its_form_in_the_row() {
    let (mut rig, id) = line_awaiting_text();
    let steps = rig.undo_steps;
    let place = text_place(&rig, id);
    rig.click(place);
    rig.settle();
    rig.frame(vec![Event::Text("sup".to_owned())]);
    rig.frame(vec![Event::Text("port".to_owned())]);
    assert_eq!(rig.undo_steps, steps, "nothing is written while typing");
    assert_eq!(rig.last().text(), "");
    rig.key(Key::Enter);
    rig.settle();
    assert!(!rig.layer.is_editing_text());
    assert_eq!(rig.last().text(), "support");
    assert_eq!(rig.undo_steps, steps + 1, "one edit, one step");
    assert_eq!(rig.last().id(), id);
    assert_eq!(rig.last().tool(), Some(Tool::TrendLine));
    // The text now shows where it was invited, and a click on it edits it again.
    let place = text_place(&rig, id);
    rig.click(place);
    assert!(rig.layer.is_editing_text());
}

#[test]
fn escape_drops_the_edit_and_is_spent_on_it() {
    let (mut rig, id) = line_awaiting_text();
    let steps = rig.undo_steps;
    let place = text_place(&rig, id);
    rig.click(place);
    rig.settle();
    rig.frame(vec![Event::Text("never".to_owned())]);
    rig.escape_consumed = false;
    rig.key(Key::Escape);
    rig.settle();
    assert!(!rig.layer.is_editing_text());
    assert_eq!(rig.last().text(), "");
    assert_eq!(rig.undo_steps, steps);
    assert!(rig.escape_consumed, "so it does not also deselect the line");
    assert_eq!(rig.selected, vec![id]);
}

#[test]
fn keys_typed_into_the_editor_do_not_reach_the_chart() {
    let (mut rig, id) = line_awaiting_text();
    let place = text_place(&rig, id);
    rig.click(place);
    rig.settle();
    rig.frame(vec![Event::Text("x".to_owned())]);
    rig.key(Key::Backspace);
    rig.key(Key::Delete);
    assert_eq!(
        rig.drawings.len(),
        1,
        "Delete and Backspace edit the text, not the chart"
    );
    assert!(rig.layer.is_editing_text());
    // Return with the line selected and no editor open opens the properties; with it open it
    // keeps the text instead.
    rig.opened = None;
    rig.key(Key::Enter);
    rig.settle();
    assert_eq!(rig.opened, None);
}

#[test]
fn shift_return_adds_a_line_and_return_keeps_both() {
    let (mut rig, id) = line_awaiting_text();
    let place = text_place(&rig, id);
    rig.click(place);
    rig.settle();
    rig.frame(vec![Event::Text("one".to_owned())]);
    rig.modifiers = Modifiers::SHIFT;
    rig.key(Key::Enter);
    rig.modifiers = Modifiers::NONE;
    rig.settle();
    assert!(
        rig.layer.is_editing_text(),
        "Shift+Return does not end the edit"
    );
    rig.frame(vec![Event::Text("two".to_owned())]);
    rig.key(Key::Enter);
    rig.settle();
    assert_eq!(rig.last().text(), "one\ntwo");
}

#[test]
fn clearing_the_text_returns_the_line_to_its_original_stored_form() {
    let (mut rig, id) = line_awaiting_text();
    let plain = serde_json::to_value(rig.last()).unwrap();
    let place = text_place(&rig, id);
    rig.click(place);
    type_and_keep(&mut rig, "hi");
    assert!(
        serde_json::to_value(rig.last())
            .unwrap()
            .get("Shape")
            .is_some()
    );
    let place = text_place(&rig, id);
    rig.click(place);
    rig.settle();
    for _ in 0..2 {
        rig.key(Key::Backspace);
    }
    rig.key(Key::Enter);
    rig.settle();
    assert_eq!(rig.last().text(), "");
    assert_eq!(serde_json::to_value(rig.last()).unwrap(), plain);
}

#[test]
fn a_locked_line_offers_no_text_and_a_drawing_that_cannot_hold_text_none() {
    let (mut rig, id) = line_awaiting_text();
    let place = text_place(&rig, id);
    let mut locked = rig.last().clone();
    locked.set_locked(true);
    rig.drawings[0] = locked;
    rig.settle();
    rig.click(place);
    assert!(!rig.layer.is_editing_text());
    // A triangle has no text.
    let mut rig = Rig::with_tool(Tool::Triangle);
    for point in &POINTS[..3] {
        rig.click(*point);
    }
    rig.tool = Tool::Cursor;
    rig.selected = vec![rig.last().id()];
    rig.settle();
    assert!(rig.layer.text_boxes.is_empty());
    assert!(!rig.last().takes_text());
}

#[test]
fn the_edit_ends_when_the_line_is_deleted_under_it() {
    let (mut rig, id) = line_awaiting_text();
    let place = text_place(&rig, id);
    rig.click(place);
    rig.settle();
    rig.drawings.clear();
    rig.settle();
    assert!(!rig.layer.is_editing_text());
}

/// The selected line with its toolbar and invitation, the same with its text being edited, and
/// a selected pitchfork, as pictures. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_selection_and_the_editor() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    if gallery_dir().is_none() {
        return;
    }
    let size = vec2(1000.0, 400.0);
    let shoot = |rig: &mut Rig, name: &str, events: Vec<Vec<Event>>| {
        let mut screen = Screen::new(size);
        let mut last = None;
        let mut script = events.into_iter();
        for _ in 0..6 {
            let events = script.next().unwrap_or_default();
            last = Some(screen.run(events, |ui| {
                let response = ui.allocate_rect(PLOT, Sense::click_and_drag());
                let proj = projection(&rig.data, &rig.scale);
                let mut frame = DrawingFrame::new(&rig.drawings, Tool::Cursor, None, 99)
                    .with_selection(rig.selected.clone())
                    .with_view(Timeframe::Day, false);
                rig.layer.update(ui, &response, &proj, &mut frame);
                rig.layer.paint(&ui.painter().clone(), &proj, &frame);
                rig.layer.overlay(ui, &proj, &mut frame);
                if let Some(select) = frame.select.take() {
                    rig.selected = select;
                }
            }));
        }
        screen.save(name, last.expect("a frame"));
    };
    let (mut rig, id) = line_awaiting_text();
    shoot(&mut rig, "selected-line-invites-text", Vec::new());
    let place = text_place(&rig, id);
    let button = |pressed| Event::PointerButton {
        pos: place,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    shoot(
        &mut rig,
        "text-editor-open",
        vec![
            vec![Event::PointerMoved(place)],
            vec![button(true)],
            vec![button(false)],
            Vec::new(),
            vec![Event::Text("support".to_owned())],
        ],
    );
    let mut fork = Rig::with_tool(Tool::Pitchfork);
    for point in &POINTS[..3] {
        fork.click(*point);
    }
    fork.tool = Tool::Cursor;
    fork.selected = vec![fork.last().id()];
    shoot(&mut fork, "selected-pitchfork", Vec::new());
}
