//! Inline colour swatches and the picker they open.
//!
//! Every constant colour in the script ([`color_sites`]) gets a small square in room opened in
//! front of it (a section's leading space in the layout, so the text itself never changes and
//! every offset stays where it was). Clicking a swatch opens [`inline`]'s picker beside it; while a
//! gesture is going on the site is rewritten from the text as it stood when the gesture began, so
//! a change in length never leaves a stale offset, and the whole gesture is one undo step.
//!
//! Detection, values and rewriting are pure and live in `pine_studio::color_sites`; this file is
//! the egui side: where the swatches go, the picker's life (what opens, closes and cancels it),
//! and how an edit reaches the text field's text and undo history.

use super::{EditorState, UndoGesture, apply_site_text, text_hash, with_range_shifted};
use crate::model::editor_theme::{SurfaceColor, SyntaxRole};
use crate::pine_studio::editor::color_sites::{ColorMap, ColorSite, Picked, rewrite};
use crate::ui::editor_appearance::{Appearance, EditorFont};
use crate::ui::kit::color_picker::inline::{self, Header, Style};
use crate::ui::kit::color_picker::{Hsv, paint_swatch_on};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Event, Frame, Galley, Id, Key, LayerId, Margin, Modifiers, Order,
    Pos2, Rect, Sense, Shadow, Stroke, Ui, Vec2, pos2,
};
use std::sync::Arc;

/// Room between the picker and the swatch it hangs from.
const PICKER_GAP: f32 = 6.0;
/// The picker's distance from the window's edge.
const WINDOW_EDGE: f32 = 8.0;
const PICKER_MARGIN: i8 = 12;
/// What the frame puts between its outer edge and the content: the margin and the border.
const FRAME_INSET: f32 = PICKER_MARGIN as f32 + 1.0;

/// How big a swatch is and how much room it takes, from the editor's font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Metrics {
    pub side: f32,
    /// Air between what comes before and the swatch.
    pub lead: f32,
    /// Room opened before the colour: air, the swatch, and a little air after it.
    pub gap: f32,
}

impl Metrics {
    pub fn new(font_size: f32, char_width: f32) -> Self {
        let side = (font_size * 0.85).round().max(8.0);
        let lead = (char_width * 0.25).max(1.5);
        Self {
            side,
            lead,
            gap: lead + side + (char_width * 0.45).max(2.0),
        }
    }
}

/// An open picker.
pub(super) struct Open {
    /// Character index where the colour's expression starts: what the picker is attached to. Edits
    /// happen inside the expression, so it does not move.
    start: usize,
    hsv: Hsv,
    /// The text's hash as the picker last left it: any other text means someone else edited.
    expected: u64,
    gesture: Option<Gesture>,
    /// A key has been used in the picker: draw the focus ring.
    keyboard: bool,
}

/// One drag, click or key press in the picker.
struct Gesture {
    /// The whole text as the gesture began.
    original: Vec<char>,
    original_source: String,
    site: ColorSite,
    /// The cursor as the gesture began, which the edit moves along with the text after it.
    cursor: egui::text::CCursorRange,
    undo: UndoGesture,
    /// What the user has moved since the gesture began.
    rgb: bool,
    transparency: bool,
}

#[derive(Default)]
pub(super) struct SwatchState {
    map: Option<(u64, Arc<ColorMap>)>,
    /// The swatches drawn this frame: where each colour starts and where its square is.
    rects: Vec<(usize, Rect)>,
    /// A swatch was clicked: open (or close) its picker.
    clicked: Option<usize>,
    pub open: Option<Open>,
    /// The room opened per swatch, as last laid out (column selection reads it).
    pub gap: f32,
}

impl SwatchState {
    /// Keeps the colours of text that was scanned while it was laid out.
    pub(super) fn cache(&mut self, key: u64, map: Arc<ColorMap>) {
        self.map = Some((key, map));
    }
}

impl EditorState {
    /// The colours in the text, scanned once per change like the syntax spans.
    pub(super) fn colors(&mut self) -> Arc<ColorMap> {
        let key = text_hash(&self.source);
        if let Some((cached, map)) = &self.swatches.map
            && *cached == key
        {
            return Arc::clone(map);
        }
        let map = Arc::new(crate::pine_studio::editor::color_sites::map(&self.source));
        self.swatches.map = Some((key, Arc::clone(&map)));
        map
    }
}

// MARK: Painting

/// Draws the swatches of the rows on screen and records where they are. Returns nothing: a click
/// is left in `state.swatches.clicked` for [`picker`].
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &mut Ui,
    state: &mut EditorState,
    map: &ColorMap,
    galley: &Galley,
    origin: Pos2,
    look: &Appearance,
    metrics: Metrics,
) {
    state.swatches.rects.clear();
    if map.sites.is_empty() {
        return;
    }
    let clip = ui.clip_rect();
    let rows = &galley.rows;
    let first = rows.partition_point(|row| origin.y + row.pos.y + row.row.size.y < clip.top());
    let end = rows.partition_point(|row| origin.y + row.pos.y <= clip.bottom());
    let edge = look.role(SyntaxRole::Default);
    let interactive = !state.sample;
    for index in map.on_lines(first, end) {
        let site = &map.sites[index];
        let Some(placed) = rows.get(map.lines[index]) else {
            continue;
        };
        // The colour's first glyph sits after the room opened for the swatch.
        let x = match placed.row.glyphs.get(map.columns[index]) {
            Some(glyph) => origin.x + placed.pos.x + glyph.pos.x,
            None => continue,
        };
        let centre_y = origin.y + placed.pos.y + placed.row.size.y / 2.0;
        let rect = Rect::from_min_size(
            pos2(
                x - metrics.gap + metrics.lead,
                centre_y - metrics.side / 2.0,
            ),
            Vec2::splat(metrics.side),
        );
        if !rect.intersects(clip) {
            continue;
        }
        state.swatches.rects.push((site.range.start, rect));
        let id = state.id.with(("swatch", site.range.start));
        let response = if interactive {
            // `click_and_drag` takes the press from the text field under it, so a click on the
            // square never moves the caret.
            ui.interact(
                rect.expand2(Vec2::new(1.0, 0.0)),
                id,
                Sense::click_and_drag(),
            )
        } else {
            ui.interact(rect, id, Sense::hover())
        };
        let open = state
            .swatches
            .open
            .as_ref()
            .is_some_and(|open| open.start == site.range.start);
        if response.hovered() && interactive {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if response.clicked() && interactive {
            state.swatches.clicked = Some(site.range.start);
        }
        let color = Color32::from_rgba_unmultiplied(
            (site.rgba >> 24) as u8,
            (site.rgba >> 16) as u8,
            (site.rgba >> 8) as u8,
            site.rgba as u8,
        );
        let painter = ui.painter();
        paint_swatch_on(
            painter,
            rect,
            color,
            0.0,
            look.surface(SurfaceColor::Background),
        );
        let line = if open || response.hovered() {
            theme::with_alpha(edge, 0.9)
        } else {
            theme::with_alpha(edge, 0.35)
        };
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, line), egui::StrokeKind::Outside);
        // The edge `paint_swatch_on` draws is faint; this one follows the editor's own text colour.
    }
}

/// How far a pointer at `x` (relative to the text's origin) is past the swatches' room on `line`,
/// so column selection counts character cells and not room.
pub(super) fn room_before(map: &ColorMap, galley: &Galley, line: usize, x: f32, gap: f32) -> f32 {
    let Some(placed) = galley.rows.get(line) else {
        return 0.0;
    };
    map.on_lines(line, line + 1)
        .map(|index| {
            let Some(glyph) = placed.row.glyphs.get(map.columns[index]) else {
                return 0.0;
            };
            let begins = placed.pos.x + glyph.pos.x - gap;
            (x - begins).clamp(0.0, gap)
        })
        .sum()
}

// MARK: The picker

fn picker_id(state: &EditorState) -> Id {
    state.id.with("color-picker")
}

fn layer(state: &EditorState) -> LayerId {
    LayerId::new(Order::Foreground, picker_id(state).with("area"))
}

/// Esc closes the picker (and abandons a gesture still going). Taken before the editor sees it.
pub(super) fn keys(ctx: &egui::Context, state: &mut EditorState) {
    if state.swatches.open.is_none() {
        return;
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        close(ctx, state, true);
        ctx.memory_mut(|memory| memory.request_focus(state.id));
        return;
    }
    // Undo and redo belong to the text: close the picker and give the field the keys.
    let undo = ctx.input(|input| {
        input.events.iter().any(|event| {
            matches!(event, Event::Key { key: Key::Z, pressed: true, modifiers, .. } if modifiers.command)
        })
    });
    let idle = state
        .swatches
        .open
        .as_ref()
        .is_some_and(|open| open.gesture.is_none());
    if undo && idle {
        close(ctx, state, false);
        ctx.memory_mut(|memory| memory.request_focus(state.id));
    }
}

/// Closes the picker. With `cancel` a gesture still going is undone.
fn close(ctx: &egui::Context, state: &mut EditorState, cancel: bool) {
    let Some(open) = state.swatches.open.take() else {
        return;
    };
    if cancel && let Some(gesture) = open.gesture {
        state.source = gesture.original_source;
        gesture.undo.cancel(ctx, state.id, gesture.cursor);
    }
    ctx.request_repaint();
}

fn open_at(ctx: &egui::Context, state: &mut EditorState, start: usize) {
    let map = state.colors();
    let Some(index) = map.at(start) else {
        return;
    };
    let site = &map.sites[index];
    let [r, g, b] = site.rgb();
    let mut hsv = Hsv::from_srgba_unmultiplied([r, g, b, 255]);
    hsv.a = inline::opacity_of(site.transparency_percent());
    // Whatever pointed into the text before is not what the picker is about.
    state.completion = None;
    state.completion_wanted = false;
    state.completion_explicit = false;
    let first = state.swatches.open.is_none();
    state.swatches.open = Some(Open {
        start,
        hsv,
        expected: text_hash(&state.source),
        gesture: None,
        keyboard: false,
    });
    if first {
        // The arrow keys then adjust the square and not the caret.
        ctx.memory_mut(|memory| memory.request_focus(picker_id(state).with("square")));
    }
    ctx.request_repaint();
}

/// The room a picker takes, margins included.
fn picker_size() -> Vec2 {
    inline::Layout::new(Pos2::ZERO).total + Vec2::splat(2.0 * FRAME_INSET)
}

/// Where the picker goes: under the swatch, or over it when there is no room, and inside the
/// window either way.
fn picker_position(anchor: Rect, screen: Rect, size: Vec2) -> Pos2 {
    let below = anchor.bottom() + PICKER_GAP;
    let above = anchor.top() - PICKER_GAP - size.y;
    let y = if below + size.y + WINDOW_EDGE <= screen.bottom() || above < screen.top() + WINDOW_EDGE
    {
        below
    } else {
        above
    };
    let x = anchor.left() - FRAME_INSET;
    let max_x = (screen.right() - WINDOW_EDGE - size.x).max(screen.left() + WINDOW_EDGE);
    let max_y = (screen.bottom() - WINDOW_EDGE - size.y).max(screen.top() + WINDOW_EDGE);
    pos2(
        x.clamp(screen.left() + WINDOW_EDGE, max_x),
        y.clamp(screen.top() + WINDOW_EDGE, max_y),
    )
}

/// Opens, draws, and applies the picker. Returns whether it changed the text.
pub(super) fn picker(
    ui: &mut Ui,
    state: &mut EditorState,
    look: &Appearance,
    code: &EditorFont,
) -> bool {
    let ctx = ui.ctx().clone();
    if let Some(start) = state.swatches.clicked.take() {
        let same = state
            .swatches
            .open
            .as_ref()
            .is_some_and(|open| open.start == start);
        if same {
            close(&ctx, state, false);
        } else {
            open_at(&ctx, state, start);
        }
    }
    let Some(mut open) = state.swatches.open.take() else {
        return false;
    };
    let map = state.colors();
    let anchor = state
        .swatches
        .rects
        .iter()
        .find(|(start, _)| *start == open.start)
        .map(|(_, rect)| *rect);
    let visible = state.text_visible;
    let site_index = map.at(open.start);
    // Text that is not what the picker left, a colour that went away, or a swatch scrolled out of
    // view: the picker has nothing to hang from.
    let stale = text_hash(&state.source) != open.expected
        || state.sample
        || site_index.is_none()
        || anchor.is_none()
        || !visible
            .zip(anchor)
            .is_some_and(|(visible, anchor)| visible.intersects(anchor));
    if stale {
        ctx.request_repaint();
        return false;
    }
    let (anchor, site_index) = (anchor.unwrap_or(Rect::NOTHING), site_index.unwrap_or(0));

    // A press anywhere else closes it; a press on a swatch is that swatch's click.
    let pressed = ctx.input(|input| {
        input
            .pointer
            .any_pressed()
            .then(|| input.pointer.interact_pos())
            .flatten()
    });
    if let Some(position) = pressed {
        let inside = ctx.layer_id_at(position) == Some(layer(state));
        let on_swatch = state
            .swatches
            .rects
            .iter()
            .any(|(_, rect)| rect.expand2(Vec2::new(1.0, 0.0)).contains(position));
        if !inside && !on_swatch {
            state.swatches.open = Some(open);
            close(&ctx, state, false);
            return false;
        }
        open.keyboard = false;
    }
    if ctx.input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, Event::Key { pressed: true, .. }))
    }) {
        open.keyboard = true;
    }

    let screen = ctx.content_rect();
    let position = picker_position(anchor, screen, picker_size());
    let id = picker_id(state);
    let style = Style {
        background: look.surface(SurfaceColor::PopupBackground),
        focus: if open.keyboard {
            look.role(SyntaxRole::Default)
        } else {
            Color32::TRANSPARENT
        },
        text: look.role(SyntaxRole::Default),
    };
    let header = Header {
        font: code.at(&ctx, (code.size - 2.0).max(9.0)),
    };
    let site = map.sites[site_index].clone();
    let mut changed = false;
    let source_id = state.id;
    egui::Area::new(id.with("area"))
        .order(Order::Foreground)
        .fixed_pos(position)
        .show(&ctx, |ui| {
            Frame::new()
                .fill(style.background)
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(10.0)
                .inner_margin(Margin::same(PICKER_MARGIN))
                .shadow(Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(110),
                })
                .show(ui, |ui| {
                    let output =
                        inline::show(ui, id, &mut open.hsv, &header, style, |hsv, output| {
                            // The expression as it will read once this frame's input is applied.
                            let touched = output.color_changed || output.transparency_changed;
                            if touched && open.gesture.is_none() {
                                open.gesture = Some(Gesture::begin(&ctx, state, source_id, &site));
                            }
                            if let Some(gesture) = &mut open.gesture {
                                gesture.rgb |= output.color_changed;
                                gesture.transparency |= output.transparency_changed;
                                let picked = gesture.picked(hsv);
                                let text = gesture.text(picked);
                                if touched {
                                    changed |= gesture.apply(&ctx, state, &text);
                                    open.expected = text_hash(&state.source);
                                }
                                return text;
                            }
                            original_text(&state.source, &site)
                        });
                    if output.finished {
                        open.gesture = None;
                    }
                });
        });
    state.swatches.open = Some(open);
    changed
}

fn original_text(source: &str, site: &ColorSite) -> String {
    source
        .chars()
        .skip(site.range.start)
        .take(site.range.len())
        .collect()
}

impl Gesture {
    fn begin(ctx: &egui::Context, state: &EditorState, id: Id, site: &ColorSite) -> Self {
        let (undo, cursor) = UndoGesture::begin(ctx, id, &state.source);
        Self {
            original: state.source.chars().collect(),
            original_source: state.source.clone(),
            site: site.clone(),
            cursor,
            undo,
            rgb: false,
            transparency: false,
        }
    }

    /// What the user has moved so far, as the values to write.
    fn picked(&self, hsv: &Hsv) -> Picked {
        let [r, g, b, _] = hsv.to_srgba_unmultiplied();
        Picked {
            rgb: self.rgb.then_some([r, g, b]),
            transparency: self
                .transparency
                .then(|| inline::transparency_percent(hsv.a)),
        }
    }

    /// The site's new text.
    fn text(&self, picked: Picked) -> String {
        rewrite(&self.site, &self.original[self.site.range.clone()], picked)
    }

    /// Writes `text` over the site in the text as the gesture began, keeping the undo step open.
    fn apply(&self, ctx: &egui::Context, state: &mut EditorState, text: &str) -> bool {
        let range = self.site.range.clone();
        let mut source = String::with_capacity(self.original_source.len() + 16);
        source.extend(&self.original[..range.start]);
        source.push_str(text);
        source.extend(&self.original[range.end..]);
        if source == state.source {
            return false;
        }
        let cursor = with_range_shifted(self.cursor, &range, text.chars().count());
        apply_site_text(ctx, state, source, cursor);
        self.undo.hold(ctx, state.id);
        true
    }
}

#[cfg(test)]
pub(super) fn picker_position_for_tests(anchor: Rect, screen: Rect) -> Pos2 {
    picker_position(anchor, screen, picker_size())
}

#[cfg(test)]
pub(super) fn picker_size_for_tests() -> Vec2 {
    picker_size()
}

#[cfg(test)]
pub(super) fn picker_margin_for_tests() -> f32 {
    FRAME_INSET
}

#[cfg(test)]
impl Open {
    pub(super) fn start(&self) -> usize {
        self.start
    }
}

#[cfg(test)]
impl SwatchState {
    pub(super) fn rects_for_tests(&self) -> &[(usize, Rect)] {
        &self.rects
    }
}

/// Where the swatch of the colour at `start` is, if it is on screen (tests read it).
#[cfg(test)]
pub(super) fn swatch_rect(state: &EditorState, start: usize) -> Option<Rect> {
    state
        .swatches
        .rects
        .iter()
        .find(|(at, _)| *at == start)
        .map(|(_, rect)| *rect)
}
