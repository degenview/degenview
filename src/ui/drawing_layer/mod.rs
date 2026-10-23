//! The drawing layer of a chart card: projects anchors onto the plot, hit-tests, runs the draw and
//! edit gestures and paints. It never mutates the drawing model; it reports changes as
//! `DrawingCommand`s in a `DrawingFrame` that the workspace applies through its undo stack.
//!
//! One interaction controller serves every tool. A tool's [`Placement`] says how its anchors
//! arrive (a freehand drag, one click, a fixed number of points, or any number finished by a
//! gesture); its [`Caps`](crate::model::Caps) say which style controls it has. Nothing below
//! branches on the tool for pointer handling.

mod draw;
mod edit;
mod forecast_shapes;
mod line_shapes;
#[cfg(test)]
mod line_tests;
mod menu;
mod note_shapes;
mod pattern_shapes;
#[cfg(test)]
mod pattern_tests;
mod shape;
mod snap;
mod table_shapes;
#[cfg(test)]
mod tests;
mod text;
mod text_edit;
mod volume_shapes;

use super::chart_card::PriceScale;
use super::drawing_geometry as geo;
use super::drawing_render::{
    self, FIB_COLORS, FIB_RATIOS, FibLevelLine, HANDLE_RADIUS, HIT_TOLERANCE, RulerCorner,
    RulerPart, RulerReadout, TimeIndex,
};
use crate::model::drawing_style::DrawingStyle;
use crate::model::{
    Anchor, Caps, Drawing, DrawingCommand, Geometry, MAX_POINTS, MagnetMode, Placement,
    PriceFormat, Timeframe, Tool,
};
use edit::{BoxHandle, Part};
use eframe::egui::{self, Color32, CursorIcon, Painter, PointerButton, Pos2, Rect, Response};
use shape::Screen;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
pub(super) use text::TextMeasure;

/// Brush samples closer than this (px) to the previous one are dropped.
const BRUSH_MIN_STEP: f32 = 1.5;
/// A drag shorter than this (px) starts a click-move-click draft instead.
const MIN_DRAG: f32 = 4.0;
/// A drawing whose anchors all lie within this many px of its first is not made.
const MIN_EXTENT: f32 = 2.0;
/// A click this near (px) the first vertex of an open shape closes it.
const CLOSE_RADIUS: f32 = 9.0;
/// How far (px) a finished stroke may stray from the sampled one when it is simplified.
const SIMPLIFY_EPSILON: f32 = 0.4;
/// The reach (px) of a handle beyond its drawn radius.
const HANDLE_REACH: f32 = HANDLE_RADIUS + 3.0;
/// The note left in the context for the workspace: a table's cell is selected on some chart, so
/// Copy, Cut and Paste are the cell's this frame and not the drawing's.
pub(in crate::ui) const TABLE_CELL_FLAG: &str = "drawing-table-cell-selected";
/// The reach (px) of a table's row or column border.
const DIVIDER_REACH: f32 = 4.0;

/// Per-frame exchange between the workspace and one chart card's drawing layer.
/// The workspace fills the inputs and applies the outputs after the card draws.
pub struct DrawingFrame<'a> {
    // Inputs.
    pub drawings: &'a [Drawing],
    pub tool: Tool,
    /// The selected drawings, only those that belong to this chart.
    pub selected: Vec<u64>,
    /// First unused id; the layer increments it for every drawing it creates.
    pub next_id: u64,
    /// The chart's timeframe: a drawing shows only on the ones it is visible on.
    pub timeframe: Timeframe,
    pub magnet: MagnetMode,
    /// Every drawing is out of sight (and out of reach) while this is on.
    pub hide_all: bool,
    /// Stay on the tool after a drawing is committed; off returns to the crosshair.
    pub keep_drawing: bool,
    /// Per-tool styles that replace the built-in defaults.
    pub defaults: Option<&'a HashMap<Tool, DrawingStyle>>,
    /// The style the user copied, for the menu's Paste style.
    pub style_clipboard: Option<(Tool, DrawingStyle)>,
    /// A dialog or sheet has the keyboard: Return, Delete and the duplicate shortcut are its.
    pub keys_blocked: bool,
    /// A drawing shown in place of the stored one with its id: the properties dialog's live
    /// preview of an edit not yet committed.
    pub preview: Option<&'a Drawing>,
    // Outputs.
    pub commands: Vec<DrawingCommand>,
    /// `Some(x)` means set the selection to `x` (`Some(vec![])` clears it).
    pub select: Option<Vec<u64>>,
    /// An Escape press was spent cancelling a draft or edit on this card.
    pub escape_consumed: bool,
    /// A drawing was committed and `keep_drawing` is off: return to the crosshair.
    pub disarm: bool,
    /// The user asked to edit this drawing's properties (double-click or Enter).
    pub open_properties: Option<u64>,
    /// A style was copied from a drawing (menu).
    pub style_copied: Option<(Tool, DrawingStyle)>,
    /// The menu's Copy: the workspace copies the selected drawings.
    pub copy_requested: bool,
    /// A right-click on empty chart (no drawing under it, nothing else claiming it): the chart's
    /// own menu opens there.
    pub background_click: Option<Pos2>,
    /// A drawing was placed while every drawing was hidden: show them again, or it would vanish.
    pub reveal_all: bool,
    /// What was chosen and waits for a click to place it (a picture), while its tool is armed.
    pub pending_content: Option<crate::model::image_content::Content>,
    /// The content was placed: the workspace forgets it, and the tool disarms.
    pub content_used: bool,
    /// Place the waiting content now, in the middle of the plot, without a click: set on the
    /// focused chart when a picture arrives by a drop or a paste.
    pub place_pending: bool,
    /// A picture to put in place of the one drawing `id` shows, from the workspace's pick.
    pub replace_with: Option<(u64, crate::model::image_content::ImageContent)>,
    /// The user asked to replace the picture of this drawing: the workspace asks for a new one.
    pub replace_request: Option<u64>,
    /// The user asked to copy the picture with this name.
    pub copy_image: Option<String>,
    /// The user asked for something of a link card: the workspace opens the link, copies it,
    /// asks for another, or fetches the preview.
    pub link_request: Option<(u64, LinkRequest)>,
    /// The user asked for a price alert at this price (a horizontal line's); the workspace opens
    /// the alert editor for them to review.
    pub alert_price: Option<f64>,
}

/// What can be asked of a Post card that only the workspace can do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkRequest {
    /// Open the link in the browser: always a deliberate click.
    Open,
    Copy,
    /// Choose another link for the card.
    Replace,
    /// Fetch the preview the service offers.
    Fetch,
}

impl<'a> DrawingFrame<'a> {
    pub fn new(drawings: &'a [Drawing], tool: Tool, selected: Option<u64>, next_id: u64) -> Self {
        Self {
            drawings,
            tool,
            selected: selected.into_iter().collect(),
            next_id,
            timeframe: Timeframe::Day,
            magnet: MagnetMode::Off,
            hide_all: false,
            keep_drawing: true,
            defaults: None,
            style_clipboard: None,
            keys_blocked: false,
            preview: None,
            commands: Vec::new(),
            select: None,
            escape_consumed: false,
            disarm: false,
            open_properties: None,
            style_copied: None,
            copy_requested: false,
            background_click: None,
            reveal_all: false,
            pending_content: None,
            content_used: false,
            place_pending: false,
            replace_with: None,
            replace_request: None,
            copy_image: None,
            link_request: None,
            alert_price: None,
        }
    }

    pub fn with_selection(mut self, selected: Vec<u64>) -> Self {
        self.selected = selected;
        self
    }

    pub fn with_view(mut self, timeframe: Timeframe, hide_all: bool) -> Self {
        self.timeframe = timeframe;
        self.hide_all = hide_all;
        self
    }

    pub fn with_behavior(mut self, magnet: MagnetMode, keep_drawing: bool) -> Self {
        self.magnet = magnet;
        self.keep_drawing = keep_drawing;
        self
    }

    pub fn with_defaults(mut self, defaults: &'a HashMap<Tool, DrawingStyle>) -> Self {
        self.defaults = Some(defaults);
        self
    }

    pub fn with_style_clipboard(mut self, clipboard: Option<(Tool, DrawingStyle)>) -> Self {
        self.style_clipboard = clipboard;
        self
    }

    pub fn with_keys_blocked(mut self, blocked: bool) -> Self {
        self.keys_blocked = blocked;
        self
    }

    pub fn with_pending_content(
        mut self,
        content: Option<crate::model::image_content::Content>,
    ) -> Self {
        self.pending_content = content;
        self
    }

    pub fn with_picture_work(
        mut self,
        place_pending: bool,
        replace_with: Option<(u64, crate::model::image_content::ImageContent)>,
    ) -> Self {
        self.place_pending = place_pending;
        self.replace_with = replace_with;
        self
    }

    pub fn with_preview(mut self, preview: Option<&'a Drawing>) -> Self {
        self.preview = preview;
        self
    }

    /// The style a new drawing of `tool` starts with.
    fn style_for(&self, tool: Tool) -> DrawingStyle {
        self.defaults
            .and_then(|defaults| defaults.get(&tool))
            .copied()
            .unwrap_or_else(|| tool.default_style())
    }

    /// Whether the drawing is shown and can be picked.
    fn shows(&self, drawing: &Drawing) -> bool {
        !self.hide_all && drawing.is_visible(self.timeframe)
    }

    fn is_selected(&self, id: u64) -> bool {
        self.selected.contains(&id)
    }
}

/// Anchor <-> screen mapping for the current frame.
pub(super) struct Projection<'a> {
    pub plot: Rect,
    /// Physical pixels per point, for the shapes that are painted as images.
    pub pixels_per_point: f32,
    /// How the chart writes a price: the fib labels, the ruler's readout and the arrow's
    /// price labels follow it.
    pub format: PriceFormat,
    pub scale: &'a PriceScale,
    pub times: TimeIndex<'a>,
    /// Global index of the first shown candle.
    pub start: usize,
    pub slot_width: f32,
    pub lead: f32,
    /// Sizes text with the painting context, for the shapes sized to their text.
    pub measure: TextMeasure,
}

/// Everything a projected shape depends on besides the drawing: when it changes, cached
/// screen shapes are rebuilt.
#[derive(Clone, Copy, PartialEq)]
struct ProjectionKey {
    plot: Rect,
    slot_width: f32,
    start: usize,
    lead: f32,
    price: (f64, f64, f32, f32, bool),
    data: (usize, i64, i64),
    /// The newest bar's open, high, low, close and volume: a regression is fitted to prices, a
    /// VWAP and a profile to prices and volume, and the live bar changes them without changing the
    /// bars' count or times.
    newest: [u64; 5],
}

impl Projection<'_> {
    pub(super) fn x(&self, time_ms: i64) -> f32 {
        self.x_at_index(self.times.index_of(time_ms))
    }

    /// Where the bar at (fractional) `index` of the bars the chart draws sits on screen.
    pub(super) fn x_at_index(&self, index: f64) -> f32 {
        let index = index - self.start as f64;
        self.plot.left() + self.slot_width * (index as f32 + 0.5) + self.slot_width * self.lead
    }

    /// The (fractional) bar index at screen position `x`.
    pub(super) fn index_at(&self, x: f32) -> f64 {
        let slots = (x - self.plot.left()) / self.slot_width - 0.5 - self.lead;
        f64::from(slots) + self.start as f64
    }

    pub(super) fn time(&self, x: f32) -> i64 {
        let slots = (x - self.plot.left()) / self.slot_width - 0.5 - self.lead;
        self.times.time_at(f64::from(slots) + self.start as f64)
    }

    pub(super) fn point(&self, anchor: Anchor) -> Pos2 {
        Pos2::new(self.x(anchor.time_ms), self.scale.y(anchor.price))
    }

    pub(super) fn anchor(&self, point: Pos2) -> Anchor {
        Anchor {
            time_ms: self.time(point.x),
            price: self.scale.price(point.y),
        }
    }

    fn key(&self) -> ProjectionKey {
        let time_at = |index: Option<usize>| {
            index
                .and_then(|index| self.times.bar(index))
                .map_or(0, |bar| bar.open_time)
        };
        let len = self.times.len();
        ProjectionKey {
            plot: self.plot,
            slot_width: self.slot_width,
            start: self.start,
            lead: self.lead,
            price: (
                self.scale.minimum,
                self.scale.maximum,
                self.scale.top,
                self.scale.height,
                self.scale.logarithmic,
            ),
            data: (
                len,
                time_at((len > 0).then_some(0)),
                time_at(len.checked_sub(1)),
            ),
            newest: len
                .checked_sub(1)
                .and_then(|index| self.times.bar(index))
                .map_or([0; 5], |bar| {
                    [bar.open, bar.high, bar.low, bar.close, bar.volume].map(f64::to_bits)
                }),
        }
    }

    /// The pointer clamped into the plot.
    fn clamp(&self, point: Pos2) -> Pos2 {
        Pos2::new(
            point.x.clamp(self.plot.left(), self.plot.right()),
            point.y.clamp(self.plot.top(), self.plot.bottom()),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hit {
    id: u64,
    part: Part,
    locked: bool,
}

/// A drawing being placed.
struct Draft {
    tool: Tool,
    /// The anchors placed so far (for a freehand stroke, every sample).
    placed: Vec<Anchor>,
    /// Where the next anchor would land: follows the pointer.
    rubber: Anchor,
    /// The button is held from the press that began this draft.
    dragging: bool,
    /// How many anchors there were when the current press began.
    placed_at_press: usize,
}

/// A placed drawing being moved or reshaped. Every drawing it touches commits together, so a
/// move of several is one undo step.
struct Edit {
    originals: Vec<Drawing>,
    currents: Vec<Drawing>,
    part: Part,
    press: Pos2,
}

enum Gesture {
    Draft(Draft),
    Edit(Edit),
}

struct CacheEntry {
    drawing: Drawing,
    key: ProjectionKey,
    screen: Option<Arc<Screen>>,
}

#[derive(Default)]
pub(super) struct DrawingLayer {
    gesture: Option<Gesture>,
    hover: Option<Hit>,
    /// Escape cancelled a gesture mid-drag; keep the plot from panning until release.
    cancelled_drag: bool,
    cache: HashMap<u64, CacheEntry>,
    /// Where each drawing's text was laid out last frame, for finding it with a click.
    text_boxes: HashMap<u64, draw::TextBox>,
    /// The axis tags of the drawings painted this frame, for the frame's label pass.
    tags: Vec<shape::Tag>,
    /// The text being edited in place.
    text_edit: Option<text_edit::TextEditing>,
    /// The drawings whose note is open (a pin's): view state, never saved or synced.
    open_notes: HashSet<u64>,
    /// The selected cell of the selected table, as `(drawing, row, column)`: view state too.
    table_cell: Option<(u64, usize, usize)>,
    /// A table just placed, to start editing its first cell once it is on the chart.
    pending_cell: Option<(u64, u8)>,
    /// The pictures painted, as textures.
    textures: draw::Textures,
    /// Where the pointer was on the plot last frame, for the picture waiting to be placed.
    pointer: Option<Pos2>,
    /// The right-click menu, while it is open.
    menu: Option<menu::MenuState>,
    /// The frame the layer last ran on: a gap means the card was not shown, and a gesture
    /// begun before it must not wait for a pointer that is long gone.
    last_frame: u64,
}

// MARK: Original line drawings

fn fib_levels(a: Anchor, b: Anchor, proj: &Projection) -> Vec<FibLevelLine> {
    FIB_RATIOS
        .iter()
        .zip(FIB_COLORS)
        .filter_map(|(ratio, color)| {
            drawing_render::fib_price(a.price, b.price, *ratio, false, proj.scale.logarithmic).map(
                |price| FibLevelLine {
                    ratio: *ratio,
                    y: proj.scale.y(price),
                    label: proj.format.format(price),
                    color,
                },
            )
        })
        .collect()
}

fn ruler_rect(a: Anchor, b: Anchor, proj: &Projection) -> Rect {
    Rect::from_two_pos(proj.point(a), proj.point(b))
}

fn end_hit(pointer: Pos2, a: Pos2, b: Pos2) -> Option<Part> {
    if a.distance(pointer) <= HANDLE_REACH {
        Some(Part::Anchor(0))
    } else if b.distance(pointer) <= HANDLE_REACH {
        Some(Part::Anchor(1))
    } else {
        None
    }
}

/// The hit on the Fibonacci and the ruler, which paint themselves.
fn hit_legacy(drawing: &Drawing, pointer: Pos2, proj: &Projection) -> Option<Part> {
    match drawing {
        Drawing::Fib { a, b, .. } => {
            let (pa, pb) = (proj.point(*a), proj.point(*b));
            end_hit(pointer, pa, pb).or_else(|| {
                drawing_render::hit_fibonacci(pa, pb, &fib_levels(*a, *b, proj), pointer)
                    .then_some(Part::Body)
            })
        }
        Drawing::Ruler { a, b, .. } => drawing_render::hit_ruler(ruler_rect(*a, *b, proj), pointer)
            .map(|part| match part {
                RulerPart::Corner(corner) => Part::Corner(corner),
                RulerPart::Edge => Part::Body,
            }),
        _ => None,
    }
}

// MARK: Gestures

/// `anchor`, kept from constructing a degenerate shape: the pointer for the second anchor of a
/// box under Shift is held square, and a line under Shift to a multiple of 45 degrees.
fn constrained(tool: Tool, first: Option<Pos2>, pointer: Pos2, shift: bool) -> Pos2 {
    // A signpost's plate is straight above or below its anchor, whatever the modifiers.
    if tool == Tool::Signpost
        && let Some(first) = first
    {
        return Pos2::new(first.x, pointer.y);
    }
    let Some(first) = first.filter(|_| shift) else {
        return pointer;
    };
    match tool {
        Tool::Rectangle | Tool::Ellipse => geo::constrained_corner(first, pointer, true),
        Tool::TrendLine
        | Tool::Ray
        | Tool::InfoLine
        | Tool::ExtendedLine
        | Tool::TrendAngle
        | Tool::Arrow
        | Tool::ArrowMarker => geo::snap_angle(first, pointer),
        _ => pointer,
    }
}

impl DrawingLayer {
    /// Whether a gesture owns the plot drag, so the card must not pan.
    pub(super) fn claims_drag(&self) -> bool {
        self.gesture.is_some() || self.cancelled_drag
    }

    /// The drawings whose note is open, in no particular order.
    pub(super) fn open_notes(&self) -> Vec<u64> {
        self.open_notes.iter().copied().collect()
    }

    /// Opens the notes of these drawings: a snapshot shows a pin as it stands on the chart.
    pub(super) fn set_open_notes(&mut self, ids: &[u64]) {
        self.open_notes = ids.iter().copied().collect();
    }

    /// Forgets the hovered drawing (input is bypassed this frame).
    pub(super) fn clear_hover(&mut self) {
        self.hover = None;
    }

    /// Drops whatever gesture is in progress without committing anything.
    fn abort(&mut self) {
        self.gesture = None;
    }

    /// The cached screen shape of `drawing` under `proj`, built if it is new or changed.
    fn screen(&mut self, drawing: &Drawing, proj: &Projection) -> Option<Arc<Screen>> {
        let key = proj.key();
        let id = drawing.id();
        if let Some(entry) = self.cache.get(&id)
            && entry.key == key
            && entry.drawing == *drawing
        {
            return entry.screen.clone();
        }
        let screen = shape::build(drawing, proj).map(Arc::new);
        self.cache.insert(
            id,
            CacheEntry {
                drawing: drawing.clone(),
                key,
                screen: screen.clone(),
            },
        );
        screen
    }

    /// The topmost drawing under `pointer`; a selected drawing's handles win over anything.
    fn hit_test(
        &mut self,
        frame: &DrawingFrame,
        pointer: Pos2,
        proj: &Projection,
        handles: bool,
    ) -> Option<Hit> {
        if handles
            && let [selected] = frame.selected[..]
            && let Some(drawing) = frame.drawings.iter().find(|d| d.id() == selected)
            && frame.shows(drawing)
            && !drawing.locked()
            && let Some(screen) = self.screen(drawing, proj)
            && let Some(part) = screen
                .handle_at(pointer, HANDLE_REACH)
                .or_else(|| screen.divider_at(pointer, DIVIDER_REACH))
        {
            return Some(Hit {
                id: selected,
                part,
                locked: false,
            });
        }
        for drawing in frame.drawings.iter().rev() {
            if !frame.shows(drawing) {
                continue;
            }
            let part = match drawing {
                Drawing::Brush { .. } | Drawing::Shape { .. } => {
                    let Some(screen) = self.screen(drawing, proj) else {
                        continue;
                    };
                    let style = drawing.style().copied().unwrap_or_default();
                    let open = self.open_notes.contains(&drawing.id());
                    let tolerance = HIT_TOLERANCE + style.stroke.width / 2.0;
                    // A trend line's ends can be grabbed without selecting it first, as they
                    // always could, whether or not it has text.
                    let end = (drawing.tool() == Some(Tool::TrendLine))
                        .then(|| screen.handle_at(pointer, HANDLE_REACH))
                        .flatten();
                    end.or_else(|| {
                        screen
                            .hits_body(pointer, tolerance, style.fill.enabled, open)
                            .then_some(Part::Body)
                    })
                }
                _ => hit_legacy(drawing, pointer, proj),
            };
            if let Some(part) = part {
                return Some(Hit {
                    id: drawing.id(),
                    part,
                    locked: drawing.locked(),
                });
            }
        }
        None
    }

    /// Runs one frame of drawing input. Returns whether the plot drag is claimed.
    pub(super) fn update(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        proj: &Projection,
        frame: &mut DrawingFrame,
    ) -> bool {
        let plot = proj.plot;
        let (pointer, press, modifiers, focused) = ui.input(|input| {
            (
                input.pointer.latest_pos(),
                input.pointer.press_origin(),
                input.modifiers,
                input.focused,
            )
        });

        self.pointer = pointer.filter(|point| plot.contains(*point));
        // A gesture must not outlive the conditions it began under: the card going unseen
        // (another tab), the window losing focus, or the tool being switched.
        let frame_nr = ui.ctx().cumulative_frame_nr();
        if self.gesture.is_some() && (!focused || frame_nr > self.last_frame + 1) {
            self.abort();
        }
        self.last_frame = frame_nr;
        if matches!(&self.gesture, Some(Gesture::Draft(draft)) if draft.tool != frame.tool) {
            self.abort();
        }
        // A cell is selected only while its table is the one thing selected.
        if let Some((id, ..)) = self.table_cell
            && frame.selected != [id]
        {
            self.table_cell = None;
        }
        if self.table_cell.is_some() {
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new(TABLE_CELL_FLAG), true));
        }
        self.place_waiting_content(frame, proj);
        self.replace_picture(frame);
        self.handle_table_keys(ui, frame, proj);
        self.handle_keys(ui, response, frame, proj);

        self.hover = None;
        if self.gesture.is_none()
            && response.hovered()
            && let Some(point) = pointer.filter(|point| plot.contains(*point))
        {
            self.hover = self.hit_test(frame, point, proj, true);
        }

        let primary = PointerButton::Primary;
        if response.drag_started_by(primary) {
            let began = press.filter(|point| plot.contains(*point));
            if let Some(point) = began {
                self.begin_drag(point, modifiers, frame, proj);
            } else if matches!(&self.gesture, Some(Gesture::Draft(_))) {
                self.begin_press_in_draft();
            }
        }
        if response.dragged_by(primary)
            && let Some(point) = pointer
        {
            self.continue_drag(proj.clamp(point), modifiers, frame, proj);
        }
        if response.drag_stopped_by(primary) {
            self.cancelled_drag = false;
            let at = pointer.map(|point| proj.clamp(point));
            self.finish_drag(at, modifiers, frame, proj);
        }
        if response.clicked()
            && let Some(point) = response
                .interact_pointer_pos()
                .or(pointer)
                .filter(|point| plot.contains(*point))
        {
            // A fast third click (egui counts it as a triple) is still the second click of a double.
            let double = response.double_clicked() || response.triple_clicked();
            self.click(point, modifiers, double, frame, proj);
        }
        if response.secondary_clicked()
            && let Some(point) = response
                .interact_pointer_pos()
                .or(pointer)
                .filter(|point| plot.contains(*point))
        {
            self.secondary_click(point, frame, proj);
        }

        // A draft follows the pointer between clicks.
        if let Some(Gesture::Draft(draft)) = &mut self.gesture
            && !draft.dragging
            && let Some(point) = pointer.filter(|point| plot.contains(*point))
        {
            let first = draft.placed.first().map(|anchor| proj.point(*anchor));
            let point = constrained(draft.tool, first, point, modifiers.shift);
            draft.rubber = Self::anchor_for(draft.tool, point, modifiers, frame, proj);
        }

        self.set_cursor(ui, pointer, frame, plot);
        self.claims_drag()
    }

    /// The anchor a placed or dragged point lands on: snapped by the magnet (inverted while the
    /// invert key is held) for every tool except the freehand ones, whose samples are the
    /// pointer's own.
    fn anchor_for(
        tool: Tool,
        point: Pos2,
        modifiers: egui::Modifiers,
        frame: &DrawingFrame,
        proj: &Projection,
    ) -> Anchor {
        let freehand = tool.spec().placement == Placement::Freehand;
        if !freehand
            && !modifiers.shift
            && let Some(snapped) = snap::snap(point, proj, frame.magnet.inverted(modifiers.command))
        {
            return snapped;
        }
        proj.anchor(point)
    }

    fn handle_keys(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) {
        let (escape, enter, delete, backspace) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::Escape),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Delete),
                input.key_pressed(egui::Key::Backspace),
            )
        });
        let typing = ui.ctx().egui_wants_keyboard_input() || frame.keys_blocked;
        if escape
            && self.gesture.is_none()
            && !self.is_editing_text()
            && (!self.open_notes.is_empty() || self.table_cell.is_some())
        {
            // Escape lets go of what was opened or selected inside a drawing first.
            self.open_notes.clear();
            self.table_cell = None;
            frame.escape_consumed = true;
            return;
        }
        if escape {
            match self.gesture.take() {
                Some(Gesture::Draft(draft)) => {
                    // A path or polyline with something to show is finished; anything else is
                    // abandoned. Either way the key is spent.
                    if is_open(draft.tool) && draft.placed.len() >= 2 {
                        self.commit_draft(draft, false, frame, proj);
                    }
                    frame.escape_consumed = true;
                    self.cancelled_drag = response.dragged();
                }
                Some(Gesture::Edit(_)) => {
                    frame.escape_consumed = true;
                    self.cancelled_drag = response.dragged();
                }
                None => {}
            }
        }
        if enter
            && !typing
            && let Some(Gesture::Draft(draft)) = &self.gesture
            && is_open(draft.tool)
            && let Some(Gesture::Draft(draft)) = self.gesture.take()
        {
            self.finish_open(draft, false, frame, proj);
            return;
        }
        if backspace
            && !typing
            && let Some(Gesture::Draft(draft)) = &mut self.gesture
            && (is_open(draft.tool)
                || (matches!(draft.tool.spec().placement, Placement::Points(_)) && !draft.dragging))
        {
            draft.placed.pop();
            if draft.placed.is_empty() {
                self.gesture = None;
            }
            return;
        }
        if self.gesture.is_none()
            && !typing
            && !frame.selected.is_empty()
            && ui.input_mut(|input| super::shortcut::DUPLICATE_DRAWING.consume(input))
        {
            self.duplicate_selection(frame, proj);
            return;
        }
        if self.gesture.is_none() && !typing {
            if (delete || backspace) && !frame.selected.is_empty() {
                for id in &frame.selected {
                    if frame.drawings.iter().any(|d| d.id() == *id && !d.locked()) {
                        frame.commands.push(DrawingCommand::Remove(*id));
                    }
                }
                frame.select = Some(Vec::new());
            } else if enter
                && let [id] = frame.selected[..]
                && frame.drawings.iter().any(|d| d.id() == id && !d.locked())
            {
                frame.open_properties = Some(id);
            }
        }
    }

    fn begin_drag(
        &mut self,
        press: Pos2,
        modifiers: egui::Modifiers,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) {
        if let Some(Gesture::Draft(_)) = &self.gesture {
            // A press while a draft waits for its next anchor: the anchor lands at release.
            self.begin_press_in_draft();
            return;
        }
        if frame.tool == Tool::Cursor {
            if modifiers.shift {
                return;
            }
            let Some(hit) = self.hit_test(frame, press, proj, true) else {
                return;
            };
            // Pressing a drawing outside the selection selects it alone; inside, the selection
            // moves together.
            let selection = if frame.is_selected(hit.id) {
                frame.selected.clone()
            } else {
                frame.select = Some(vec![hit.id]);
                vec![hit.id]
            };
            if hit.locked {
                return;
            }
            let moving_group = hit.part == Part::Body && selection.len() > 1;
            let originals: Vec<Drawing> = frame
                .drawings
                .iter()
                .filter(|drawing| {
                    if moving_group {
                        selection.contains(&drawing.id()) && !drawing.locked()
                    } else {
                        drawing.id() == hit.id
                    }
                })
                .cloned()
                .collect();
            if !originals.is_empty() {
                self.gesture = Some(Gesture::Edit(Edit {
                    currents: originals.clone(),
                    originals,
                    part: hit.part,
                    press,
                }));
            }
            return;
        }
        let tool = frame.tool;
        let anchor = Self::anchor_for(tool, press, egui::Modifiers::NONE, frame, proj);
        self.gesture = Some(Gesture::Draft(Draft {
            tool,
            placed: vec![anchor],
            rubber: anchor,
            dragging: true,
            placed_at_press: 0,
        }));
    }

    fn begin_press_in_draft(&mut self) {
        if let Some(Gesture::Draft(draft)) = &mut self.gesture {
            draft.dragging = true;
            draft.placed_at_press = draft.placed.len();
        }
    }

    fn continue_drag(
        &mut self,
        pointer: Pos2,
        modifiers: egui::Modifiers,
        frame: &DrawingFrame,
        proj: &Projection,
    ) {
        match &mut self.gesture {
            Some(Gesture::Edit(edit)) => {
                let target = match edit.part {
                    Part::Body => proj.anchor(pointer),
                    _ => Self::anchor_for(Tool::Cursor, pointer, modifiers, frame, proj),
                };
                edit.currents = edit
                    .originals
                    .iter()
                    .map(|original| {
                        edit::edited(original, edit.part, edit.press, pointer, target, proj)
                    })
                    .collect();
            }
            Some(Gesture::Draft(draft)) if draft.dragging => {
                let tool = draft.tool;
                if tool.spec().placement == Placement::Freehand {
                    let anchor = proj.anchor(pointer);
                    draft.rubber = anchor;
                    let far_enough = draft
                        .placed
                        .last()
                        .is_none_or(|last| proj.point(*last).distance(pointer) >= BRUSH_MIN_STEP);
                    if far_enough && draft.placed.len() < MAX_POINTS {
                        draft.placed.push(anchor);
                    }
                } else {
                    let first = draft.placed.first().map(|anchor| proj.point(*anchor));
                    let point = constrained(tool, first, pointer, modifiers.shift);
                    draft.rubber = Self::anchor_for(tool, point, modifiers, frame, proj);
                }
            }
            _ => {}
        }
    }

    fn finish_drag(
        &mut self,
        at: Option<Pos2>,
        modifiers: egui::Modifiers,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) {
        match self.gesture.take() {
            Some(Gesture::Edit(edit)) => {
                for (original, current) in edit.originals.iter().zip(&edit.currents) {
                    if current != original {
                        frame.commands.push(DrawingCommand::Replace {
                            id: original.id(),
                            drawing: current.clone(),
                        });
                    }
                }
            }
            Some(Gesture::Draft(mut draft)) if draft.dragging => {
                draft.dragging = false;
                let tool = draft.tool;
                let placement = tool.spec().placement;
                if placement == Placement::Freehand {
                    self.commit_draft(draft, false, frame, proj);
                    return;
                }
                if placement == Placement::Single {
                    // Pressed and let go (with a wobble): the anchor is where it was pressed.
                    self.commit_draft(draft, false, frame, proj);
                    return;
                }
                let first = draft.placed.first().map(|anchor| proj.point(*anchor));
                let release = at.map(|point| {
                    let point = constrained(tool, first, point, modifiers.shift);
                    Self::anchor_for(tool, point, modifiers, frame, proj)
                });
                let fresh = draft.placed_at_press == 0;
                let dragged_far = fresh
                    && release.is_some_and(|anchor| {
                        let origin = proj.point(draft.placed[0]);
                        origin.distance(proj.point(anchor)) >= MIN_DRAG
                    });
                if let Some(anchor) = release
                    && (dragged_far || !fresh)
                {
                    draft.placed.push(anchor);
                }
                draft.rubber = release.unwrap_or(draft.rubber);
                self.after_placement(draft, frame, proj);
            }
            other => self.gesture = other,
        }
    }

    /// A draft has gained anchors: commit it if it is complete, else keep waiting.
    fn after_placement(&mut self, draft: Draft, frame: &mut DrawingFrame, proj: &Projection) {
        match draft.tool.spec().placement {
            Placement::Points(count) if draft.placed.len() >= usize::from(count) => {
                self.commit_draft(draft, false, frame, proj);
            }
            Placement::Open { .. } if draft.placed.len() >= MAX_POINTS => {
                self.commit_draft(draft, false, frame, proj);
            }
            _ => self.gesture = Some(Gesture::Draft(draft)),
        }
    }

    fn click(
        &mut self,
        point: Pos2,
        modifiers: egui::Modifiers,
        double: bool,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) {
        // A click while a draft waits for its next anchor.
        if let Some(Gesture::Draft(mut draft)) = self.gesture.take() {
            let tool = draft.tool;
            if is_open(tool) {
                if double {
                    // The first click of the double click already placed the last vertex.
                    self.finish_open(draft, false, frame, proj);
                    return;
                }
                let closes = matches!(tool.spec().placement, Placement::Open { closable: true })
                    && draft.placed.len() >= 3
                    && draft
                        .placed
                        .first()
                        .is_some_and(|first| proj.point(*first).distance(point) <= CLOSE_RADIUS);
                if closes {
                    self.finish_open(draft, true, frame, proj);
                    return;
                }
            }
            let first = draft.placed.first().map(|anchor| proj.point(*anchor));
            let point = constrained(tool, first, point, modifiers.shift);
            draft
                .placed
                .push(Self::anchor_for(tool, point, modifiers, frame, proj));
            draft.dragging = false;
            self.after_placement(draft, frame, proj);
            return;
        }
        match frame.tool {
            Tool::Cursor => {
                // A click on a cell of the one selected table selects it; a second edits it.
                if !modifiers.shift
                    && let Some((id, row, column)) = self.table_cell_at(frame, point, proj)
                {
                    self.table_cell = Some((id, row, column));
                    if double {
                        self.begin_cell_edit(id, row, column, None, frame, proj);
                    }
                    return;
                }
                // A click on the text place of the one selected drawing edits its text, ahead of
                // whatever line the text sits on.
                if !modifiers.shift
                    && let Some(id) = self.text_at(frame, point)
                    && self.begin_text_edit(id, frame, proj)
                {
                    return;
                }
                // Shift is for building a selection: a handle under the pointer is no target.
                let hit = self.hit_test(frame, point, proj, !modifiers.shift);
                if !modifiers.shift {
                    self.toggle_notes(hit, point, frame, proj);
                }
                frame.select = Some(match (hit, modifiers.shift) {
                    (Some(hit), true) => {
                        let mut selection = frame.selected.clone();
                        if let Some(index) = selection.iter().position(|id| *id == hit.id) {
                            selection.remove(index);
                        } else {
                            selection.push(hit.id);
                        }
                        selection
                    }
                    (Some(hit), false) => vec![hit.id],
                    (None, true) => frame.selected.clone(),
                    (None, false) => Vec::new(),
                });
                if double
                    && let Some(hit) = hit
                    && !hit.locked
                {
                    // A pin's note is written where it opens; the rest open their properties.
                    let in_place = frame.drawings.iter().any(|drawing| {
                        drawing.id() == hit.id
                            && drawing.edits_text_in_place()
                            && drawing
                                .tool()
                                .is_some_and(|tool| tool.caps().has(Caps::COLLAPSIBLE))
                    });
                    if !(in_place && self.begin_text_edit(hit.id, frame, proj)) {
                        frame.open_properties = Some(hit.id);
                    }
                }
            }
            tool => {
                let anchor = Self::anchor_for(tool, point, modifiers, frame, proj);
                let draft = Draft {
                    tool,
                    placed: vec![anchor],
                    rubber: anchor,
                    dragging: false,
                    placed_at_press: 0,
                };
                match tool.spec().placement {
                    // One click is the whole gesture for a dot and for a marker.
                    Placement::Freehand | Placement::Single => {
                        self.commit_draft(draft, false, frame, proj);
                    }
                    _ => self.gesture = Some(Gesture::Draft(draft)),
                }
            }
        }
    }

    /// A click on a pin opens its note, or shuts it when it is open; a click anywhere else shuts
    /// every note, except on the open note itself.
    fn toggle_notes(
        &mut self,
        hit: Option<Hit>,
        point: Pos2,
        frame: &DrawingFrame,
        proj: &Projection,
    ) {
        let pin = hit.and_then(|hit| {
            frame
                .drawings
                .iter()
                .find(|drawing| drawing.id() == hit.id)
                .filter(|drawing| {
                    drawing
                        .tool()
                        .is_some_and(|tool| tool.caps().has(Caps::COLLAPSIBLE))
                })
        });
        let Some(pin) = pin else {
            self.open_notes.clear();
            return;
        };
        let id = pin.id();
        let was_open = self.open_notes.contains(&id);
        let on_note = was_open
            && self
                .screen(pin, proj)
                .and_then(|screen| screen.popup.as_ref().map(|popup| popup.rect))
                .is_some_and(|rect| rect.contains(point));
        self.open_notes.clear();
        if was_open == on_note {
            // Opened, or the note itself was clicked and stays as it is.
            self.open_notes.insert(id);
        }
    }

    fn secondary_click(&mut self, point: Pos2, frame: &mut DrawingFrame, proj: &Projection) {
        // A right-click finishes an open shape (a polyline's documented alternative to a
        // double-click) and abandons a drawing of a fixed number of points that is half placed;
        // otherwise it opens the menu of what is under it.
        if let Some(Gesture::Draft(draft)) = self.gesture.take() {
            if is_open(draft.tool) && draft.placed.len() >= 2 {
                self.finish_open(draft, false, frame, proj);
            } else if !matches!(draft.tool.spec().placement, Placement::Points(_)) {
                self.gesture = Some(Gesture::Draft(draft));
            }
            return;
        }
        self.open_menu(point, frame, proj);
    }

    /// Finishes a path or polyline: commits it when it has enough vertices, else drops it.
    fn finish_open(
        &mut self,
        mut draft: Draft,
        closed: bool,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) {
        // A double-click lands two clicks on one spot; keep one vertex.
        if let [.., previous, last] = draft.placed[..]
            && proj.point(previous).distance(proj.point(last)) < 2.0
        {
            draft.placed.pop();
        }
        self.commit_draft(draft, closed, frame, proj);
    }

    /// Turns a draft into a committed drawing (one command, one undo step), or drops it when
    /// its anchors make no shape.
    fn commit_draft(
        &mut self,
        draft: Draft,
        closed: bool,
        frame: &mut DrawingFrame,
        proj: &Projection,
    ) {
        self.gesture = None;
        let tool = draft.tool;
        let mut anchors = draft.placed;
        if tool.spec().placement == Placement::Freehand {
            let screen: Vec<Pos2> = anchors.iter().map(|anchor| proj.point(*anchor)).collect();
            anchors = geo::simplify_indices(&screen, SIMPLIFY_EPSILON)
                .into_iter()
                .map(|index| anchors[index])
                .collect();
        }
        // Anchors that all land within a couple of pixels of the first make a shape nobody
        // can see or grab: treat the gesture as a mis-click.
        let spec = tool.spec();
        if matches!(
            spec.placement,
            Placement::Points(_) | Placement::Open { .. }
        ) && let Some(first) = anchors.first().map(|anchor| proj.point(*anchor))
            && anchors
                .iter()
                .all(|anchor| proj.point(*anchor).distance(first) < MIN_EXTENT)
        {
            return;
        }
        let id = frame.next_id;
        let built = if tool.caps().has(Caps::CONTENT) {
            // What was chosen is dropped where the click was: a picture at its own size, brought
            // to fit the plot.
            let Some(content) = frame.pending_content.clone() else {
                return;
            };
            let Some(first) = anchors.first().map(|anchor| proj.point(*anchor)) else {
                return;
            };
            // Where its top-left corner goes, and (for a picture) the size the plot allows it.
            let (corner, content) = match content {
                crate::model::image_content::Content::Image(mut picture) => {
                    let rect = placed_rect(&picture, first, proj.plot);
                    picture.size = [rect.width(), rect.height()];
                    (
                        proj.anchor(rect.left_top()),
                        crate::model::image_content::Content::Image(picture),
                    )
                }
                crate::model::image_content::Content::Link(card) => {
                    let rect = placed_card_rect(&card, first, proj.plot);
                    (
                        proj.anchor(rect.left_top()),
                        crate::model::image_content::Content::Link(card),
                    )
                }
            };
            Drawing::build_with(tool, id, &[corner], frame.style_for(tool), &content)
        } else if tool.caps().has(Caps::POSITION) && anchors.len() == 1 {
            // A position is a click: its target, stop and width come from what is on screen.
            forecast_shapes::sized_position(tool, anchors[0], proj)
                .and_then(|sized| Drawing::build(tool, id, &sized, frame.style_for(tool)))
        } else if tool.caps().has(Caps::BARS) {
            // A bars pattern copies the bars the chart draws between its first two clicks and sets
            // them down at the third.
            forecast_shapes::captured_pattern(&anchors, proj)
                .and_then(|geometry| Drawing::from_geometry(id, geometry, frame.style_for(tool)))
        } else {
            Drawing::build(tool, id, &anchors, frame.style_for(tool))
        };
        let Some(mut drawing) = built else {
            return;
        };
        if closed && let Some(Geometry::Polyline { closed, .. }) = drawing.geometry_mut() {
            *closed = true;
        }
        frame.next_id += 1;
        if tool.caps().has(Caps::TEXT_FIRST) {
            // Nothing without its text: the editor opens, and the drawing is added with what is
            // typed. Escape leaves nothing behind.
            self.begin_draft_text_edit(drawing, proj);
        } else {
            frame.commands.push(DrawingCommand::Add(drawing));
            frame.select = Some(vec![id]);
            if tool.caps().has(Caps::TABLE) {
                // Its first cell takes the keyboard as soon as the table is on the chart.
                self.pending_cell = Some((id, 0));
            }
        }
        if frame.hide_all {
            frame.reveal_all = true;
        }
        if tool.caps().has(Caps::CONTENT) {
            // What was chosen is used up: the next one is chosen again.
            frame.content_used = true;
            frame.disarm = true;
        } else if !frame.keep_drawing && tool != Tool::Ruler {
            frame.disarm = true;
        }
    }

    fn set_cursor(&self, ui: &egui::Ui, pointer: Option<Pos2>, frame: &DrawingFrame, plot: Rect) {
        let over_plot = pointer.is_some_and(|point| plot.contains(point));
        let icon = match &self.gesture {
            Some(Gesture::Edit(_)) => Some(CursorIcon::Grabbing),
            Some(Gesture::Draft(_)) => Some(CursorIcon::Crosshair),
            None if !over_plot => None,
            None if frame.tool != Tool::Cursor => Some(CursorIcon::Crosshair),
            None if pointer.is_some_and(|point| self.text_at(frame, point).is_some()) => {
                Some(CursorIcon::Text)
            }
            None => self.hover.map(|hit| match hit.part {
                Part::Body => CursorIcon::Move,
                Part::Anchor(_) | Part::Label => CursorIcon::Grab,
                Part::Width | Part::Column(_) => CursorIcon::ResizeHorizontal,
                Part::Row(_) => CursorIcon::ResizeVertical,
                Part::Box(handle) => match handle {
                    BoxHandle::TopLeft | BoxHandle::BottomRight => CursorIcon::ResizeNwSe,
                    BoxHandle::TopRight | BoxHandle::BottomLeft => CursorIcon::ResizeNeSw,
                    BoxHandle::Top | BoxHandle::Bottom => CursorIcon::ResizeVertical,
                    BoxHandle::Left | BoxHandle::Right => CursorIcon::ResizeHorizontal,
                },
                Part::Corner(RulerCorner::TopLeft | RulerCorner::BottomRight) => {
                    CursorIcon::ResizeNwSe
                }
                Part::Corner(_) => CursorIcon::ResizeNeSw,
            }),
        };
        if let Some(icon) = icon {
            ui.ctx().set_cursor_icon(icon);
        }
    }

    // MARK: Painting

    /// Paints every drawing (the ones being edited at their live position) and any draft.
    pub(super) fn paint(&mut self, painter: &Painter, proj: &Projection, frame: &DrawingFrame) {
        let clipped = painter.with_clip_rect(proj.plot);
        // The gesture is borrowed while painting, so it is lifted out and put back.
        let gesture = self.gesture.take();
        let editing: HashMap<u64, &Drawing> = match &gesture {
            Some(Gesture::Edit(edit)) => edit
                .currents
                .iter()
                .map(|drawing| (drawing.id(), drawing))
                .collect(),
            _ => HashMap::new(),
        };
        let hovered = self.hover.map(|hit| hit.id);
        let single = frame.selected.len() == 1;
        let mut live = HashSet::new();
        self.tags.clear();
        self.textures.begin();
        for stored in frame.drawings {
            live.insert(stored.id());
            if !frame.shows(stored) {
                self.text_boxes.remove(&stored.id());
                continue;
            }
            let drawing = match (editing.get(&stored.id()), frame.preview) {
                (Some(current), _) => *current,
                (None, Some(preview)) if preview.id() == stored.id() => preview,
                _ => stored,
            };
            let id = drawing.id();
            let selected = frame.is_selected(id);
            let emphasized = selected || hovered == Some(id);
            self.paint_drawing(
                &clipped,
                proj,
                drawing,
                emphasized,
                selected && single,
                false,
            );
        }
        self.cache.retain(|id, _| live.contains(id));
        self.text_boxes.retain(|id, _| live.contains(id));

        if let Some(Gesture::Draft(draft)) = &gesture {
            self.paint_draft(&clipped, proj, frame, draft);
        }
        // The picture waiting to be placed follows the pointer, faint, where a click would put it.
        if frame.tool.caps().has(Caps::CONTENT)
            && let Some(content) = &frame.pending_content
            && let Some(point) = self.pointer
        {
            match content {
                crate::model::image_content::Content::Image(picture) => {
                    let rect = placed_rect(picture, point, proj.plot);
                    draw::draw_picture(
                        &clipped,
                        &mut self.textures,
                        rect,
                        &picture.asset,
                        &picture.name,
                        0.55,
                    );
                }
                crate::model::image_content::Content::Link(card) => {
                    let rect = placed_card_rect(card, point, proj.plot);
                    crate::ui::content_card::paint(&clipped, rect, card, 0.55, None);
                }
            }
        }
        // A drawing waiting for its text is shown as it will be, growing as it is typed into.
        if let Some(preview) = self
            .text_edit
            .as_ref()
            .and_then(text_edit::TextEditing::preview)
        {
            self.paint_drawing(&clipped, proj, &preview, true, true, true);
        }
        drop(editing);
        self.gesture = gesture;
        self.textures.end();
    }

    /// Paints the tags the drawings painted this frame asked for on the price and time axes.
    pub(super) fn paint_tags(
        &self,
        painter: &Painter,
        rect: Rect,
        plot: Rect,
        proj: &Projection,
        style: &super::canvas_render::CanvasStyle,
    ) {
        for tag in &self.tags {
            draw::paint_tag(painter, *tag, rect, plot, proj, style);
        }
    }

    fn paint_draft(
        &mut self,
        painter: &Painter,
        proj: &Projection,
        frame: &DrawingFrame,
        draft: &Draft,
    ) {
        let tool = draft.tool;
        let mut anchors = draft.placed.clone();
        if tool.spec().placement != Placement::Freehand {
            anchors.push(draft.rubber);
        }
        let preview = match tool {
            Tool::Fib | Tool::Ruler => {
                let (a, b) = (draft.placed[0], draft.rubber);
                Some(match tool {
                    Tool::Fib => Drawing::Fib {
                        id: 0,
                        a,
                        b,
                        locked: false,
                        hidden: false,
                        visibility: Default::default(),
                    },
                    _ => Drawing::Ruler { id: 0, a, b },
                })
            }
            Tool::Brush => Drawing::build(Tool::Brush, 0, &anchors, frame.style_for(tool)),
            _ => {
                let geometry = if tool.caps().has(Caps::POSITION) {
                    // Sized from the view as it will be when placed.
                    anchors
                        .last()
                        .and_then(|last| forecast_shapes::sized_position(tool, *last, proj))
                        .and_then(|sized| Geometry::from_anchors(tool, &sized))
                } else if tool.caps().has(Caps::BARS) {
                    forecast_shapes::bars_preview(&anchors, proj)
                } else {
                    Geometry::preview(tool, &anchors)
                };
                geometry.map(|geometry| Drawing::Shape {
                    id: 0,
                    geometry,
                    style: frame.style_for(tool),
                    text: String::new(),
                    locked: false,
                    hidden: false,
                    visibility: Default::default(),
                })
            }
        };
        let Some(preview) = preview else {
            return;
        };
        // Handles on the placed anchors, except for a freehand stroke, which has none.
        let handles = tool.spec().placement != Placement::Freehand;
        self.paint_drawing(painter, proj, &preview, true, handles, true);
        // A closable shape shows where a click would close it.
        if matches!(tool.spec().placement, Placement::Open { closable: true })
            && draft.placed.len() >= 3
        {
            let first = proj.point(draft.placed[0]);
            let near = proj.point(draft.rubber).distance(first) <= CLOSE_RADIUS;
            if near {
                painter.circle_stroke(first, CLOSE_RADIUS, egui::Stroke::new(1.5, Color32::WHITE));
            }
        }
    }

    fn paint_drawing(
        &mut self,
        painter: &Painter,
        proj: &Projection,
        drawing: &Drawing,
        emphasized: bool,
        with_handles: bool,
        draft: bool,
    ) {
        match drawing {
            Drawing::Fib { a, b, .. } => {
                drawing_render::draw_fibonacci(
                    painter,
                    proj.plot,
                    proj.point(*a),
                    proj.point(*b),
                    &fib_levels(*a, *b, proj),
                    emphasized,
                );
            }
            Drawing::Ruler { a, b, .. } => {
                let readout = RulerReadout::new(
                    a.price,
                    b.price,
                    proj.times.bars_between(a.time_ms, b.time_ms),
                    (b.time_ms - a.time_ms) / 1000,
                );
                drawing_render::draw_ruler(
                    painter,
                    proj.plot,
                    ruler_rect(*a, *b, proj),
                    &readout,
                    proj.format,
                    emphasized && !draft,
                    draft,
                );
            }
            Drawing::Brush { .. } | Drawing::Shape { .. } => {
                // A draft is rebuilt every frame as it grows; a placed drawing is cached.
                let screen = if draft {
                    shape::build(drawing, proj).map(Arc::new)
                } else {
                    self.screen(drawing, proj)
                };
                // Nothing showing means nothing to click: forget where the text was.
                let Some(screen) = screen.filter(|screen| screen.bounds.intersects(proj.plot))
                else {
                    if !draft {
                        self.text_boxes.remove(&drawing.id());
                    }
                    return;
                };
                if !draft {
                    self.tags.extend(screen.tags.iter().copied());
                }
                let text = if self
                    .text_edit
                    .as_ref()
                    .is_some_and(|editing| editing.id == drawing.id())
                {
                    draw::TextMode::Editing
                } else if with_handles
                    && !draft
                    && !drawing.locked()
                    && drawing.edits_text_in_place()
                {
                    draw::TextMode::Invite
                } else {
                    draw::TextMode::Text
                };
                let text_box = draw::paint(
                    painter,
                    &screen,
                    drawing,
                    proj,
                    draw::Emphasis {
                        halo: emphasized && !draft,
                        selected: with_handles,
                        text,
                        open: !draft && self.open_notes.contains(&drawing.id()),
                        cell: self
                            .table_cell
                            .filter(|(id, ..)| !draft && *id == drawing.id())
                            .map(|(_, row, column)| (row, column)),
                        divider: self
                            .hover
                            .filter(|hit| !draft && hit.id == drawing.id())
                            .map(|hit| hit.part)
                            .filter(|part| matches!(part, Part::Column(_) | Part::Row(_))),
                    },
                    &mut self.textures,
                );
                if !draft {
                    match text_box {
                        Some(found) => {
                            self.text_boxes.insert(drawing.id(), found);
                        }
                        None => {
                            self.text_boxes.remove(&drawing.id());
                        }
                    }
                }
                // A trend line shows its ends while hovered as well, as it always did.
                let hover_handles = emphasized
                    && !draft
                    && !drawing.locked()
                    && drawing.tool() == Some(Tool::TrendLine);
                if (with_handles || hover_handles) && !drawing.locked() {
                    let color = drawing
                        .style()
                        .map_or(Color32::WHITE, |style| {
                            super::series_render::color(style.stroke.color)
                        })
                        .to_opaque();
                    draw::paint_handles(painter, &screen, color);
                } else if with_handles {
                    draw::paint_locked_frame(painter, screen.bounds, Color32::WHITE);
                }
            }
            Drawing::Unreadable(_) => {}
        }
    }
}

impl DrawingLayer {
    /// Puts the waiting picture in the middle of the plot when the workspace says it arrived by
    /// a drop or a paste, instead of waiting for a click.
    fn place_waiting_content(&mut self, frame: &mut DrawingFrame, proj: &Projection) {
        if !frame.place_pending || frame.pending_content.is_none() {
            return;
        }
        frame.place_pending = false;
        let (tool, size) = match frame.pending_content.clone().expect("checked above") {
            crate::model::image_content::Content::Image(picture) => (
                Tool::Image,
                placed_rect(&picture, proj.plot.center(), proj.plot).size(),
            ),
            crate::model::image_content::Content::Link(card) => (
                Tool::Post,
                placed_card_rect(&card, proj.plot.center(), proj.plot).size(),
            ),
        };
        let at = proj.anchor(proj.plot.center() - size / 2.0);
        let draft = Draft {
            tool,
            placed: vec![at],
            rubber: at,
            dragging: false,
            placed_at_press: 0,
        };
        self.commit_draft(draft, false, frame, proj);
    }

    /// Swaps the picture of a drawing for the one the user chose: the same place, the same
    /// width and the same opacity, one undo step. A picture of another shape takes the shape
    /// that suits it, from the same top-left corner, unless the drawing was told not to keep it.
    fn replace_picture(&mut self, frame: &mut DrawingFrame) {
        let Some((id, content)) = frame.replace_with.take() else {
            return;
        };
        let Some(drawing) = frame
            .drawings
            .iter()
            .find(|drawing| drawing.id() == id && !drawing.locked())
        else {
            return;
        };
        let mut changed = drawing.clone();
        let Some(Geometry::Image(image)) = changed.geometry_mut() else {
            return;
        };
        image.name.clone_from(&content.name);
        image.swap_picture(&content);
        frame.commands.push(DrawingCommand::Replace {
            id,
            drawing: changed,
        });
    }
}

/// The rectangle a picture takes when it is dropped with its top-left corner at `at`: the size it
/// starts at, scaled down to at most 40 % of the plot's width and half its height, and moved back
/// onto the plot if that would put it over an edge.
fn placed_rect(picture: &crate::model::image_content::ImageContent, at: Pos2, plot: Rect) -> Rect {
    let (width, height) = (picture.size[0].max(1.0), picture.size[1].max(1.0));
    let scale = (plot.width() * 0.4 / width)
        .min(plot.height() * 0.5 / height)
        .min(1.0);
    let size = egui::vec2((width * scale).max(24.0), (height * scale).max(24.0));
    let left = at.x.min(plot.right() - size.x).max(plot.left());
    let top = at.y.min(plot.bottom() - size.y).max(plot.top());
    Rect::from_min_size(Pos2::new(left, top), size)
}

/// The rectangle a card takes when it is dropped with its top-left corner at `at`: its own size,
/// moved back onto the plot if that would put it over an edge.
fn placed_card_rect(card: &crate::model::link_card::LinkCard, at: Pos2, plot: Rect) -> Rect {
    let size = egui::vec2(
        card.size[0].min(plot.width()),
        card.size[1].min(plot.height()),
    );
    let left = at.x.min(plot.right() - size.x).max(plot.left());
    let top = at.y.min(plot.bottom() - size.y).max(plot.top());
    Rect::from_min_size(Pos2::new(left, top), size)
}

/// Whether a tool collects any number of vertices (a path or polyline).
fn is_open(tool: Tool) -> bool {
    matches!(tool.spec().placement, Placement::Open { .. })
}
