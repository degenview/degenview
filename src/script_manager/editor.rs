//! The code editor pane: a line-numbered monospaced text field with token colouring, diagnostic
//! underlines, the current-line band, indent guides and the Swift editing assists, above the
//! status/Save bar. Ported from `ScriptEditorView` and `LineNumberedTextEditorView`.
//!
//! A pure view: it never compiles. The host feeds `diagnostics` and `status` back after a
//! change (`EditorOutput::changed`).

use super::completion::{self, Completion};
use super::debug_marks::{BreakpointMark, DebugMarks, GutterEvent, MarkKind, StopMark};
use super::diff::{self, ChangeKind, DiffHunk, DiffSnapshot};
use super::diff_view::{self, PreviewEvent};
use super::edit_ops::{self, Context, Edit};
use super::find::{self, FindEvent, FindState};
use super::model::CompileStatus;
use super::profile_marks::{LineMark, ProfileColumn};
use super::signature;
use super::syntax::{self, Span};
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{
    self, Align, Align2, Color32, Event, FontId, Id, Key, Margin, Modifiers, Rect, RichText, Sense,
    Shape, Stroke, TextBuffer, Ui, UiBuilder, Vec2,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod carets;
mod navigation;
pub use navigation::EditAction;
#[cfg(test)]
mod navigation_tests;

const GUTTER_WIDTH: f32 = 64.0;
/// The profiler's column, left of the gutter while the profiler is on: a flame, a bar and a share.
const PROFILE_WIDTH: f32 = 104.0;
const PROFILE_FLAME: f32 = 11.0;
const PROFILE_BAR_X: f32 = 21.0;
const PROFILE_BAR_WIDTH: f32 = 40.0;
const PROFILE_BAR_HEIGHT: f32 = 6.0;
/// Where a breakpoint marker sits in the gutter, from its left edge: right of the change strip,
/// left of the line numbers.
const MARKER_X: f32 = 21.0;
const MARKER_RADIUS: f32 = 5.0;
/// The change column at the gutter's left edge: the bar's inset and width, and the (much wider,
/// so no one has to aim at three points) strip that takes the click.
const CHANGE_INSET: f32 = 2.0;
const CHANGE_BAR: f32 = 3.0;
const CHANGE_HIT: f32 = 12.0;
/// How far a deleted-lines wedge reaches above and below its boundary.
const WEDGE_HALF: f32 = 4.0;
const TEXT_INSET: f32 = 8.0;
/// Room kept under the last line, so it can be scrolled clear of the status bar and the
/// horizontal scrollbar.
const END_CLEARANCE: f32 = 28.0;
/// How far below the caret's line a scroll to the caret keeps visible.
const CARET_CLEARANCE: f32 = 14.0;
const FONT_SIZE: f32 = 13.0;
const LINE_NUMBER_SIZE: f32 = 11.0;
/// A draft is written this long after the last keystroke.
pub const DRAFT_DELAY: Duration = Duration::from_secs(1);
/// The text is compiled this long after the last keystroke, so a name still being typed is not
/// reported as undefined. Shorter than the preview's own apply delay, which waits for the compile.
pub const COMPILE_DELAY: Duration = Duration::from_millis(500);
const FLASH: Duration = Duration::from_millis(700);

pub struct EditorState {
    id: Id,
    pub source: String,
    saved_source: String,
    saved_key: u64,
    /// The comparison of `saved_source` and `source`, kept while neither has changed.
    diff: Option<((u64, u64), Arc<DiffSnapshot>)>,
    /// The change whose comparison popover is open.
    preview: Option<DiffHunk>,
    /// A line to bring into view next frame (stepping between changes).
    scroll_to_line: Option<usize>,
    /// The line a stopped script is at, to bring into view next frame unless it already is.
    reveal_stop: Option<usize>,
    pub diagnostics: Vec<Diagnostic>,
    pub status: CompileStatus,
    highlighted: Option<(u64, Arc<Vec<Span>>)>,
    /// A UTF-16 offset to move the caret to on the next frame (a Problems row was clicked).
    jump_to: Option<usize>,
    flash: Option<(usize, Instant)>,
    last_edit: Option<Instant>,
    /// When the text last changed without being compiled; `diagnostics` is that much out of date.
    compile_requested: Option<Instant>,
    pub(super) find: FindState,
    completion: Option<CompletionView>,
    /// Typing (or an explicit request) asked for suggestions; moving the caret withdraws it.
    completion_wanted: bool,
    completion_explicit: bool,
    last_caret: Option<usize>,
    caret_rect: Option<Rect>,
    /// The caret position last scrolled to, so only a caret move scrolls (not a mouse wheel).
    caret_seen: Option<usize>,
    /// How far down the text the editor was scrolled when last drawn.
    #[cfg(test)]
    scroll_y: f32,
    /// A match to select (char range) on the next frame.
    select_to: Option<std::ops::Range<usize>>,
    /// What a debugging session shows in the text and the gutter, set by the host each frame.
    debug: DebugMarks,
    /// The line whose breakpoint menu is open.
    menu_line: Option<usize>,
    /// The profiler's column, while the profiler is on.
    profile: Option<ProfileColumn>,
    /// The lines the pointer is over in the profile column: the code block to wash.
    profile_hover: Option<(usize, usize)>,
    /// The extra carets (Sublime-style multiple cursors) and the click that may add one.
    carets: carets::CaretState,
    /// Go to definition, the right-click menu and the rename box.
    nav: navigation::NavState,
}

/// The open suggestion list and its highlighted row.
struct CompletionView {
    completion: Completion,
    selected: usize,
    /// An arrow key moved the highlight: scroll it into view when the list is next drawn.
    scroll_to_selected: bool,
}

/// Suggestion rows visible at once; longer lists scroll.
const COMPLETION_ROWS: usize = 10;
const COMPLETION_ROW_HEIGHT: f32 = 22.0;

#[derive(Default)]
pub struct EditorOutput {
    /// The text changed this frame: compile it.
    pub changed: bool,
    pub save: bool,
    /// Clicks in the breakpoint column.
    pub gutter: Vec<GutterEvent>,
    /// Where the text is drawn: right of the profile column and the gutter.
    pub text_area: Option<Rect>,
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl EditorState {
    pub fn new(script_id: &str, source: &str, saved_source: &str) -> Self {
        Self {
            id: Id::new(("script-editor", script_id)),
            source: source.to_owned(),
            saved_source: saved_source.to_owned(),
            saved_key: diff::text_key(saved_source),
            diff: None,
            preview: None,
            scroll_to_line: None,
            reveal_stop: None,
            diagnostics: Vec::new(),
            status: CompileStatus::NotCompiled,
            highlighted: None,
            jump_to: None,
            flash: None,
            last_edit: None,
            compile_requested: None,
            find: FindState::default(),
            completion: None,
            completion_wanted: false,
            completion_explicit: false,
            last_caret: None,
            caret_rect: None,
            caret_seen: None,
            select_to: None,
            debug: DebugMarks::default(),
            menu_line: None,
            profile: None,
            profile_hover: None,
            carets: carets::CaretState::default(),
            nav: navigation::NavState::default(),
            #[cfg(test)]
            scroll_y: 0.0,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.source != self.saved_source
    }

    /// Shows `text` as if it had been typed, for an edit made from outside (the MCP server). The
    /// caller has already written it where it belongs, so no draft write is pending; with `saved`
    /// the text is also the new saved source. Anything tied to the old text's positions (a pending
    /// jump, a suggestion list, a find selection) is dropped.
    pub fn replace_source(&mut self, text: &str, saved: bool) {
        self.source = text.to_owned();
        if saved {
            self.saved_source = text.to_owned();
            self.saved_key = diff::text_key(text);
        }
        self.last_edit = None;
        self.highlighted = None;
        self.jump_to = None;
        self.flash = None;
        self.completion = None;
        self.completion_wanted = false;
        self.completion_explicit = false;
        self.select_to = None;
        self.carets.clear();
        self.nav.rename = None;
    }

    pub fn mark_saved(&mut self) {
        self.saved_source = self.source.clone();
        self.saved_key = diff::text_key(&self.saved_source);
        self.last_edit = None;
    }

    /// What differs from the saved script, computed once per change of either text (never while
    /// painting) and shared by the gutter, the status chip, the preview and the revert.
    pub fn changes(&mut self) -> Arc<DiffSnapshot> {
        let key = (text_hash(&self.source), self.saved_key);
        if let Some((cached, snapshot)) = &self.diff
            && *cached == key
        {
            return Arc::clone(snapshot);
        }
        let snapshot = Arc::new(DiffSnapshot::compute(&self.saved_source, &self.source));
        self.diff = Some((key, Arc::clone(&snapshot)));
        snapshot
    }

    pub fn saved_source(&self) -> &str {
        &self.saved_source
    }

    /// The pending draft, once typing has paused for `DRAFT_DELAY`.
    pub fn take_draft_due(&mut self, now: Instant) -> bool {
        match self.last_edit {
            Some(at) if now.duration_since(at) >= DRAFT_DELAY => {
                self.last_edit = None;
                self.is_dirty()
            }
            _ => false,
        }
    }

    pub fn has_pending_draft(&self) -> bool {
        self.last_edit.is_some()
    }

    /// Notes that the text changed and `diagnostics` no longer match it.
    pub fn request_compile(&mut self) {
        self.compile_requested = Some(Instant::now());
    }

    /// Whether the text has changed since it was last compiled.
    pub fn compile_pending(&self) -> bool {
        self.compile_requested.is_some()
    }

    /// Whether typing has paused for `COMPILE_DELAY` since the text last changed.
    pub fn compile_due(&self, now: Instant) -> bool {
        self.compile_requested
            .is_some_and(|at| now.duration_since(at) >= COMPILE_DELAY)
    }

    /// Records that `diagnostics` and `status` now describe `source`.
    pub fn compiled(&mut self) {
        self.compile_requested = None;
    }

    /// Moves the caret to `offset` (UTF-16 units) and flashes the spot, next frame. An offset past
    /// the end of the text lands at the end.
    pub fn reveal_offset(&mut self, offset: usize) {
        self.jump_to = Some(offset);
    }

    /// Hands the editor the debugger's marks to draw this frame.
    pub fn set_debug_marks(&mut self, marks: DebugMarks) {
        self.debug = marks;
    }

    /// Shows the profiler's column (or removes it) this frame.
    pub fn set_profile_column(&mut self, column: Option<ProfileColumn>) {
        if column.is_none() {
            self.profile_hover = None;
        }
        self.profile = column;
    }

    /// The one-based line the caret is on, once the editor has shown the text.
    pub fn caret_line(&self) -> Option<usize> {
        let index = self.caret_seen?;
        Some(
            1 + self
                .source
                .chars()
                .take(index)
                .filter(|c| *c == '\n')
                .count(),
        )
    }

    /// Scrolls to a one-based line and flashes its start, next frame.
    pub fn show_line(&mut self, line: usize) {
        let line = line.max(1);
        let start: usize = self
            .source
            .split_inclusive('\n')
            .take(line - 1)
            .map(|text| text.chars().count())
            .sum();
        self.scroll_to_line = Some(line - 1);
        self.flash = Some((start, Instant::now()));
    }

    /// Brings a one-based line into view next frame, centred, unless it is already well inside the
    /// editor. Used when the debugger stops.
    pub fn reveal_stop_line(&mut self, line: usize) {
        self.reveal_stop = Some(line.max(1) - 1);
    }

    /// How far down the text the editor was scrolled when last drawn.
    #[cfg(test)]
    pub fn scroll_y(&self) -> f32 {
        self.scroll_y
    }

    /// The offset the caret is waiting to be moved to.
    #[cfg(test)]
    pub fn pending_jump(&self) -> Option<usize> {
        self.jump_to
    }

    /// Moves the caret to a diagnostic's start and flashes it, next frame.
    pub fn reveal(&mut self, diagnostic: &Diagnostic) {
        self.jump_to = Some(diagnostic.range.start.offset);
    }

    fn spans(&mut self) -> Arc<Vec<Span>> {
        let key = text_hash(&self.source);
        if let Some((cached, spans)) = &self.highlighted
            && *cached == key
        {
            return Arc::clone(spans);
        }
        let spans = Arc::new(syntax::classify(&self.source));
        self.highlighted = Some((key, Arc::clone(&spans)));
        spans
    }
}

/// "No problems", "1 warning", "2 errors" — the chip's text and look.
pub fn status_chip(status: CompileStatus, diagnostics: &[Diagnostic]) -> (Icon, Color32, String) {
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .count();
    let warnings = diagnostics.len() - errors;
    let plural = |n: usize, noun: &str| format!("{n} {noun}{}", if n == 1 { "" } else { "s" });
    match status {
        CompileStatus::NotCompiled => (
            Icon::CircleDashed,
            theme::text_secondary(),
            "Not compiled".to_owned(),
        ),
        CompileStatus::Valid => (
            Icon::CheckCircleFill,
            Color32::from_rgb(0x30, 0xD1, 0x58),
            "No problems".to_owned(),
        ),
        CompileStatus::Warning => (
            Icon::TriangleFill,
            theme::YELLOW,
            plural(warnings.max(1), "warning"),
        ),
        CompileStatus::Error => (
            Icon::XmarkOctagonFill,
            theme::syntax::string(),
            plural(errors.max(1), "error"),
        ),
    }
}

pub fn show(ui: &mut Ui, state: &mut EditorState) -> EditorOutput {
    let mut output = EditorOutput::default();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let bar = super::bottom_bar::rect(full);
    let area = Rect::from_min_max(full.min, egui::pos2(full.max.x, bar.min.y));
    handle_find_shortcuts(ui.ctx(), state);
    let find_height = if state.find.open {
        find::BAR_HEIGHT
    } else {
        0.0
    };
    let code = Rect::from_min_max(egui::pos2(area.min.x, area.min.y + find_height), area.max);
    let profile_width = if state.profile.is_some() {
        PROFILE_WIDTH
    } else {
        0.0
    };
    let profile_column =
        Rect::from_min_max(code.min, egui::pos2(code.min.x + profile_width, code.max.y));
    let gutter = Rect::from_min_max(
        egui::pos2(profile_column.max.x, code.min.y),
        egui::pos2(profile_column.max.x + GUTTER_WIDTH, code.max.y),
    );
    let text_area = Rect::from_min_max(egui::pos2(gutter.max.x, code.min.y), code.max);

    ui.painter()
        .rect_filled(area, 0.0, theme::editor_background());
    ui.painter()
        .rect_filled(gutter, 0.0, theme::gutter_background());
    if state.find.open {
        let bar_rect =
            Rect::from_min_max(area.min, egui::pos2(area.max.x, area.min.y + find_height));
        run_find_bar(ui, bar_rect, state);
    }

    let mut edited = completion_keys(ui.ctx(), state);
    navigation::keys(ui.ctx(), state);
    navigation::edit_menu(ui.ctx(), state);
    edited |= carets::apply(ui.ctx(), state, text_area);
    edited |= apply_assists(ui.ctx(), state);
    let rows = text_view(ui, state, text_area, &mut edited);
    // A rename changes the text but is not typing: no suggestions follow it.
    let renamed = navigation::rename_box(ui.ctx(), state);
    edited |= renamed;
    edited |= update_completion(ui, state, edited && !renamed);
    draw_signature(ui, state);
    paint_gutter(ui, gutter, &rows, &state.debug);
    if state.profile.is_some() {
        profile_gutter(ui, profile_column, &rows, state);
    }
    let changes = state.changes();
    edited |= change_gutter(ui, gutter, &rows, state, &changes);
    output.gutter = breakpoint_gutter(ui, gutter, &rows, state);
    output.text_area = Some(text_area);
    output.changed = edited;
    if edited {
        state.last_edit = Some(Instant::now());
    }
    if state.last_edit.is_some() {
        ui.ctx().request_repaint_after(DRAFT_DELAY);
    }

    output.save = status_bar(ui, bar, state, &changes);
    output
}

// MARK: Completion

/// Handles the keys the suggestion list owns while it is open (↑ ↓ Return Tab Esc), and the
/// explicit invocations (`shortcut::COMPLETE`) always. Returns whether the text changed.
fn completion_keys(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let focused = ctx.memory(|memory| memory.has_focus(state.id));
    if !focused {
        return false;
    }
    let mut accept = false;
    ctx.input_mut(|input| {
        let events = std::mem::take(&mut input.events);
        for event in events {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = &event
            else {
                input.events.push(event);
                continue;
            };
            let plain = !modifiers.command && !modifiers.alt && !modifiers.ctrl && !modifiers.shift;
            if shortcut::COMPLETE.matches_for(shortcut::is_mac(), *key, *modifiers) {
                state.completion_wanted = true;
                state.completion_explicit = true;
                continue;
            }
            let Some(view) = state.completion.as_mut().filter(|_| plain) else {
                input.events.push(event);
                continue;
            };
            let count = view.completion.items.len();
            match key {
                Key::ArrowDown => {
                    view.selected = (view.selected + 1) % count;
                    view.scroll_to_selected = true;
                }
                Key::ArrowUp => {
                    view.selected = (view.selected + count - 1) % count;
                    view.scroll_to_selected = true;
                }
                Key::Enter | Key::Tab => accept = true,
                Key::Escape => {
                    state.completion = None;
                    state.completion_wanted = false;
                    state.completion_explicit = false;
                }
                _ => input.events.push(event),
            }
        }
    });
    accept && accept_completion(ctx, state)
}

/// Replaces the typed prefix with the highlighted suggestion. Returns whether the text changed.
fn accept_completion(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let Some(view) = state.completion.take() else {
        return false;
    };
    state.completion_wanted = false;
    state.completion_explicit = false;
    let Some(item) = view.completion.items.get(view.selected) else {
        return false;
    };
    let range = view.completion.replace;
    let name_end = range.start + item.label.chars().count();
    // A function gets its call parentheses and the caret between them, unless the name is
    // already followed by a call.
    let call = item.callable && state.source.chars().nth(range.end) != Some('(');
    let edit = Edit {
        range: range.clone(),
        replacement: if call {
            format!("{}()", item.label)
        } else {
            item.label.clone()
        },
        selection: {
            let caret = name_end + usize::from(call);
            caret..caret
        },
    };
    state.source = edit.apply(&state.source);
    if let Some(mut edit_state) = TextEditState::load(ctx, state.id) {
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(edit.selection.start))));
        edit_state.store(ctx, state.id);
    }
    true
}

/// Recomputes the suggestions after the frame's edits and draws the list under the caret.
/// Returns whether a row click changed the text.
fn update_completion(ui: &mut Ui, state: &mut EditorState, edited: bool) -> bool {
    let ctx = ui.ctx().clone();
    if !state.carets.extra.is_empty() {
        // Suggestions follow one caret.
        state.completion = None;
        state.completion_wanted = false;
        state.completion_explicit = false;
        return false;
    }
    let focused = ctx.memory(|memory| memory.has_focus(state.id));
    let caret = TextEditState::load(&ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .filter(|range| range.primary == range.secondary)
        .map(|range| range.primary.index.0);
    if edited {
        state.completion_wanted = true;
    } else if caret != state.last_caret && !state.completion_explicit {
        // The caret moved on its own (click, arrows): the suggestions no longer apply.
        state.completion_wanted = false;
    }
    state.last_caret = caret;
    let Some(caret) = caret.filter(|_| focused && state.completion_wanted) else {
        state.completion = None;
        return false;
    };
    let fresh = completion::complete(&state.source, caret, state.completion_explicit);
    state.completion = match (fresh, state.completion.take()) {
        (Some(next), Some(old)) => {
            let same = old.completion.items == next.items;
            Some(CompletionView {
                selected: if same {
                    old.selected.min(next.items.len() - 1)
                } else {
                    0
                },
                scroll_to_selected: old.scroll_to_selected || !same,
                completion: next,
            })
        }
        (Some(next), None) => Some(CompletionView {
            completion: next,
            selected: 0,
            scroll_to_selected: false,
        }),
        (None, _) => None,
    };
    let (Some(view), Some(caret_rect)) = (state.completion.as_ref(), state.caret_rect) else {
        return false;
    };
    let mut clicked = None;
    let selected = view.selected;
    let scroll_to_selected = view.scroll_to_selected;
    let visible_height =
        view.completion.items.len().min(COMPLETION_ROWS) as f32 * COMPLETION_ROW_HEIGHT;
    egui::Area::new(Id::new("script-completion"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(caret_rect.left(), caret_rect.bottom() + 4.0))
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(theme::chrome())
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(6.0)
                .inner_margin(Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(412.0);
                    egui::ScrollArea::vertical()
                        .min_scrolled_height(visible_height)
                        .max_height(visible_height)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (index, item) in view.completion.items.iter().enumerate() {
                                let (rect, response) = ui.allocate_exact_size(
                                    Vec2::new(412.0, COMPLETION_ROW_HEIGHT),
                                    Sense::click(),
                                );
                                let on = index == selected;
                                if on {
                                    ui.painter().rect_filled(rect, 4.0, theme::SYSTEM_ACCENT);
                                    if scroll_to_selected {
                                        ui.scroll_to_rect(rect, None);
                                    }
                                } else if response.hovered() {
                                    ui.painter().rect_filled(rect, 4.0, theme::ink(14));
                                }
                                let ink = if on {
                                    Color32::WHITE
                                } else {
                                    theme::text_primary()
                                };
                                let dim = if on {
                                    Color32::WHITE.gamma_multiply(0.8)
                                } else {
                                    theme::text_secondary()
                                };
                                let badge = Rect::from_center_size(
                                    egui::pos2(rect.min.x + 14.0, rect.center().y),
                                    Vec2::splat(16.0),
                                );
                                ui.painter().rect_stroke(
                                    badge,
                                    3.0,
                                    Stroke::new(1.0, dim),
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    badge.center(),
                                    Align2::CENTER_CENTER,
                                    item.kind.badge().to_ascii_uppercase(),
                                    FontId::monospace(10.0),
                                    dim,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.min.x + 30.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    &item.label,
                                    FontId::monospace(12.0),
                                    ink,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.max.x - 8.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    item.kind.detail(),
                                    FontId::proportional(11.0),
                                    dim,
                                );
                                if response.clicked() {
                                    clicked = Some(index);
                                }
                            }
                        });
                });
        });
    if let Some(view) = state.completion.as_mut() {
        view.scroll_to_selected = false;
    }
    match clicked {
        Some(index) => {
            if let Some(view) = state.completion.as_mut() {
                view.selected = index;
            }
            accept_completion(&ctx, state)
        }
        None => false,
    }
}

/// The signature panel over the caret line: `ta.sma(source, length) → series float`, the
/// argument under the caret underlined. Quiet while the suggestion list is open.
fn draw_signature(ui: &mut Ui, state: &EditorState) {
    let ctx = ui.ctx().clone();
    if state.completion.is_some() || !ctx.memory(|memory| memory.has_focus(state.id)) {
        return;
    }
    let Some(caret_rect) = state.caret_rect else {
        return;
    };
    let Some(range) = TextEditState::load(&ctx, state.id).and_then(|edit| edit.cursor.char_range())
    else {
        return;
    };
    if range.primary != range.secondary {
        return;
    }
    let Some(help) = signature::help(&state.source, range.primary.index.0) else {
        return;
    };
    let mono = FontId::monospace(12.0);
    let plain = |color: Color32| egui::TextFormat::simple(mono.clone(), color);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &format!("{}(", help.name),
        0.0,
        plain(theme::text_primary()),
    );
    for (index, parameter) in help.parameters.iter().enumerate() {
        if index > 0 {
            job.append(", ", 0.0, plain(theme::text_primary()));
        }
        let mut format = plain(if Some(index) == help.active {
            Color32::WHITE
        } else {
            theme::text_primary()
        });
        if Some(index) == help.active {
            format.underline = Stroke::new(1.5, theme::SYSTEM_ACCENT);
        }
        job.append(parameter, 0.0, format);
    }
    job.append(")", 0.0, plain(theme::text_primary()));
    if !help.returns.is_empty() {
        job.append(
            &format!(" → {}", help.returns),
            0.0,
            plain(theme::text_secondary()),
        );
    }
    // The panel is as wide as the signature needs, within the window: it slides left to fit
    // rather than wrapping while there is room, and only wraps once the whole window is too
    // narrow. It sits above the caret line, or below it when there is no room above.
    const MARGIN: Margin = Margin::symmetric(10, 6);
    const EDGE: f32 = 8.0;
    const GAP: f32 = 4.0;
    let bounds = ctx.content_rect().shrink(EDGE);
    // The frame's margin plus its one-point border on each side.
    let chrome = MARGIN.sum() + Vec2::splat(2.0);
    job.wrap.max_width = f32::INFINITY;
    let natural = ctx
        .fonts_mut(|fonts| fonts.layout_job(job.clone()))
        .size()
        .x;
    job.wrap.max_width = natural.min((bounds.width() - chrome.x).max(1.0)) + 0.5;
    let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
    let size = galley.size() + chrome;
    let x = caret_rect
        .left()
        .min(bounds.right() - size.x)
        .max(bounds.left());
    let above = caret_rect.top() - GAP - size.y;
    let y = if above >= bounds.top() {
        above
    } else {
        caret_rect.bottom() + GAP
    };
    egui::Area::new(Id::new("script-signature"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .fixed_pos(egui::pos2(x, y))
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(theme::chrome())
                .stroke(Stroke::new(1.0, theme::separator()))
                .corner_radius(6.0)
                .inner_margin(MARGIN)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(galley).selectable(false));
                });
        });
}

/// `shortcut::FIND` opens the find bar (seeded with a single-line selection); `FIND_NEXT` /
/// `FIND_PREVIOUS` step through matches.
fn handle_find_shortcuts(ctx: &egui::Context, state: &mut EditorState) {
    let (open, next, previous) = ctx.input(|i| {
        (
            shortcut::FIND.peek(i),
            shortcut::FIND_NEXT.peek(i),
            shortcut::FIND_PREVIOUS.peek(i),
        )
    });
    if open {
        // Like Xcode: Find with a single-line selection searches for that text.
        let selected = TextEditState::load(ctx, state.id)
            .and_then(|edit| edit.cursor.char_range())
            .map(|range| {
                let (a, b) = (range.primary.index.0, range.secondary.index.0);
                state
                    .source
                    .chars()
                    .skip(a.min(b))
                    .take(a.abs_diff(b))
                    .collect::<String>()
            })
            .filter(|text| !text.is_empty() && !text.contains('\n'));
        if let Some(text) = selected {
            state.find.query = text;
            state.find.current = 0;
        }
        state.find.open = true;
        state.find.needs_focus = true;
    } else if (next || previous) && state.find.open {
        step_match(state, next);
    }
}

fn step_match(state: &mut EditorState, forward: bool) {
    let found = find::matches(&state.source, &state.find.query);
    if let Some(index) = find::step(
        state.find.current.min(found.len().saturating_sub(1)),
        found.len(),
        forward,
    ) {
        state.find.current = index;
        state.select_to = Some(found[index].clone());
    }
}

fn run_find_bar(ui: &mut Ui, rect: Rect, state: &mut EditorState) {
    let count = find::matches(&state.source, &state.find.query).len();
    match find::show(ui, rect, &mut state.find, count) {
        FindEvent::None => {}
        FindEvent::QueryChanged => {
            let found = find::matches(&state.source, &state.find.query);
            let caret = TextEditState::load(ui.ctx(), state.id)
                .and_then(|edit| edit.cursor.char_range())
                .map_or(0, |range| {
                    range.primary.index.0.min(range.secondary.index.0)
                });
            state.find.current = find::nearest(&found, caret);
            state.select_to = found.get(state.find.current).cloned();
        }
        FindEvent::Next => step_match(state, true),
        FindEvent::Previous => step_match(state, false),
        FindEvent::Close => {
            state.find.open = false;
            ui.memory_mut(|memory| memory.request_focus(state.id));
        }
    }
}

/// What the gutter needs from the text view: each line's top and height on screen.
struct GutterRows {
    rows: Vec<(f32, f32)>,
}

fn text_view(ui: &mut Ui, state: &mut EditorState, area: Rect, edited: &mut bool) -> GutterRows {
    let id = state.id;
    let font = FontId::monospace(FONT_SIZE);
    let spans = state.spans();
    let current = text_hash(&state.source);

    if let Some(offset) = state.jump_to.take() {
        let index = syntax::compiled_utf16_to_char_index(&state.source, offset);
        let mut edit_state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
        edit_state.store(ui.ctx(), id);
        ui.memory_mut(|memory| memory.request_focus(id));
        state.flash = Some((index, Instant::now()));
        ui.ctx().request_repaint();
    }
    if let Some(range) = state.select_to.take() {
        let mut edit_state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(range.start),
            CCursor::new(range.end),
        )));
        edit_state.store(ui.ctx(), id);
        // The find field keeps focus so typing refines the search; the match is selected and
        // scrolled into view.
        state.flash = Some((range.start, Instant::now()));
        ui.ctx().request_repaint();
    }
    let flash = state.flash;
    let mut scroll_to = state.scroll_to_line.take();
    let mut reveal_stop = state.reveal_stop.take();
    let find_matches = if state.find.open {
        find::matches(&state.source, &state.find.query)
    } else {
        Vec::new()
    };
    let find_current = state.find.current;

    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| {
        // While the buffer is what was classified, reuse it; mid-edit text is classified now.
        let fresh;
        let spans: &[Span] = if text_hash(text.as_str()) == current {
            &spans
        } else {
            fresh = syntax::classify(text.as_str());
            &fresh
        };
        let mut job = syntax::layout_job(text.as_str(), spans, font.clone(), theme::text_primary());
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };

    let mut pane = ui.new_child(UiBuilder::new().max_rect(area));
    pane.set_clip_rect(area.intersect(ui.clip_rect()));
    let char_width = pane.fonts_mut(|fonts| fonts.glyph_width(&FontId::monospace(FONT_SIZE), ' '));
    let mut rows = GutterRows { rows: Vec::new() };
    let scroll = egui::ScrollArea::both()
        .id_salt(("script-editor-scroll", id))
        .auto_shrink([false, false])
        .show_viewport(&mut pane, |ui, viewport| {
            // Decorations that sit behind the text are painted into a slot reserved first.
            let behind = ui.painter().add(Shape::Noop);
            let visible_width = viewport.width().max(ui.available_width());
            let text_edit = egui::TextEdit::multiline(&mut state.source)
                .id(id)
                .font(font.clone())
                .frame(egui::Frame::NONE)
                .margin(Margin {
                    left: TEXT_INSET as i8,
                    right: TEXT_INSET as i8,
                    top: TEXT_INSET as i8,
                    bottom: (TEXT_INSET + END_CLEARANCE) as i8,
                })
                .desired_width(f32::INFINITY)
                .min_size(Vec2::new(visible_width, viewport.height()))
                .lock_focus(true)
                .layouter(&mut layouter)
                .show(ui);
            if text_edit.response.response.changed() {
                *edited = true;
            }
            let galley = &text_edit.galley;
            let origin = text_edit.galley_pos;
            carets::resolve(ui.ctx(), state, galley, origin, char_width);
            navigation::hover(ui, state, &text_edit.response.response, galley, origin);
            navigation::context_menu(
                ui.ctx(),
                state,
                &text_edit.response.response,
                galley,
                origin,
            );
            for placed in &galley.rows {
                rows.rows.push((origin.y + placed.pos.y, placed.row.size.y));
            }

            if let Some(line) = scroll_to.take()
                && let Some(row) = galley
                    .rows
                    .get(line.min(galley.rows.len().saturating_sub(1)))
            {
                let top = origin.y + row.pos.y;
                ui.scroll_to_rect(
                    Rect::from_min_size(egui::pos2(origin.x, top), Vec2::new(1.0, row.row.size.y))
                        .expand2(Vec2::new(0.0, 40.0)),
                    Some(Align::Center),
                );
            }
            if let Some(line) = reveal_stop.take()
                && let Some(row) = galley.rows.get(line)
            {
                let top = origin.y + row.pos.y;
                let target =
                    Rect::from_min_size(egui::pos2(origin.x, top), Vec2::new(1.0, row.row.size.y))
                        .expand2(Vec2::new(0.0, 40.0));
                if !ui.clip_rect().contains_rect(target) {
                    ui.scroll_to_rect(target, Some(Align::Center));
                }
            }
            let caret = text_edit.cursor_range;
            state.caret_rect = caret.map(|range| {
                galley
                    .pos_from_cursor(range.primary)
                    .translate(origin.to_vec2())
            });
            // A caret move keeps its line, and some room under it, in view: egui alone stops at
            // the viewport's edge, which leaves the last line cut off.
            let caret_index = caret.map(|range| range.primary.index.0);
            if caret_index != state.caret_seen {
                state.caret_seen = caret_index;
                if let Some(rect) = state.caret_rect
                    && text_edit.response.response.has_focus()
                {
                    let target =
                        Rect::from_min_max(rect.min, rect.max + Vec2::new(0.0, CARET_CLEARANCE));
                    ui.scroll_to_rect(target, None);
                }
            }
            let mut under = Vec::new();
            let clip = ui.clip_rect();
            let band_x = clip.x_range();
            // Current-line band (caret only) and indent guides, behind the text.
            let mut behind_shapes = Vec::new();
            if let Some(stop) = state.debug.stop
                && let Some(row) = galley.rows.get(stop.line.saturating_sub(1))
            {
                let top = origin.y + row.pos.y;
                let color = if stop.error {
                    theme::editor_debug_error_background()
                } else if stop.innermost {
                    theme::editor_debug_line_background()
                } else {
                    theme::editor_debug_caller_background()
                };
                behind_shapes.push(Shape::rect_filled(
                    Rect::from_x_y_ranges(band_x, top..=top + row.row.size.y),
                    0.0,
                    color,
                ));
            }
            // The code the pointer is on in the profile column, first line to last.
            if let Some((first, last)) = state.profile_hover
                && let Some(first_row) = galley.rows.get(first.saturating_sub(1))
                && let Some(last_row) = galley.rows.get(
                    last.saturating_sub(1)
                        .min(galley.rows.len().saturating_sub(1)),
                )
            {
                let top = origin.y + first_row.pos.y;
                let bottom = origin.y + last_row.pos.y + last_row.row.size.y;
                behind_shapes.push(Shape::rect_filled(
                    Rect::from_x_y_ranges(band_x, top..=bottom.max(top)),
                    0.0,
                    theme::profiler_block(),
                ));
            }
            if let Some(range) = caret
                && range.primary == range.secondary
                && let Some(row) = galley
                    .rows
                    .get(galley.layout_from_cursor(range.primary).row)
            {
                let top = origin.y + row.pos.y;
                behind_shapes.push(Shape::rect_filled(
                    Rect::from_x_y_ranges(band_x, top..=top + row.row.size.y),
                    0.0,
                    theme::ink(15),
                ));
            }
            for (index, span) in find_matches.iter().enumerate() {
                let a = galley.pos_from_cursor(CCursor::new(span.start));
                let b = galley.pos_from_cursor(CCursor::new(span.end));
                if (a.min.y - b.min.y).abs() < 1.0 {
                    let rect = Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y))
                        .translate(origin.to_vec2());
                    let alpha = if index == find_current { 0.6 } else { 0.28 };
                    behind_shapes.push(Shape::rect_filled(
                        rect,
                        2.0,
                        theme::with_alpha(theme::YELLOW, alpha),
                    ));
                }
            }
            // Same-identifier highlights and the bracket pair, only while the caret is a point.
            if let Some(range) = caret
                && range.primary == range.secondary
            {
                let at = range.primary.index.0;
                let span_rect = |span: std::ops::Range<usize>| {
                    let a = galley.pos_from_cursor(CCursor::new(span.start));
                    let b = galley.pos_from_cursor(CCursor::new(span.end));
                    ((a.min.y - b.min.y).abs() < 1.0).then(|| {
                        Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y))
                            .translate(origin.to_vec2())
                    })
                };
                for (index, span) in edit_ops::occurrences(&state.source, at)
                    .into_iter()
                    .enumerate()
                {
                    if let Some(rect) = span_rect(span) {
                        // The token under the caret is stronger (0.7) than the others (0.45).
                        let alpha = if index == 0 { 178 } else { 115 };
                        behind_shapes.push(Shape::rect_filled(
                            rect,
                            2.0,
                            Color32::from_rgba_unmultiplied(63, 99, 148, alpha),
                        ));
                    }
                }
                if let Some((position, partner)) = edit_ops::bracket_match(&state.source, at) {
                    let (color, spans) = match partner {
                        Some(other) => (theme::ink(56), vec![position, other]),
                        None => (
                            theme::with_alpha(theme::syntax::string(), 0.3),
                            vec![position],
                        ),
                    };
                    for position in spans {
                        if let Some(rect) = span_rect(position..position + 1) {
                            behind_shapes.push(Shape::rect_filled(rect, 2.0, color));
                        }
                    }
                }
            }
            behind_shapes.extend(indent_guides(
                &state.source,
                galley,
                origin,
                char_width,
                caret.map(|c| c.primary.index.0),
            ));
            let (extra_behind, extra_over) =
                carets::shapes(state, galley, origin, ui.visuals(), char_width);
            behind_shapes.extend(extra_behind);
            ui.painter().set(behind, Shape::Vec(behind_shapes));
            ui.painter().extend(extra_over);

            // Diagnostic underlines and their merged hover messages (over the text).
            for diagnostic in &state.diagnostics {
                if let Some(rect) = underline_rect(&state.source, galley, origin, diagnostic) {
                    let color = match diagnostic.severity {
                        DiagnosticSeverity::Error => theme::syntax::string(),
                        DiagnosticSeverity::Warning => theme::YELLOW,
                    };
                    ui.painter().line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        Stroke::new(1.0, color),
                    );
                    under.push((rect, diagnostic.message.clone()));
                }
            }
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
                && text_edit.response.response.hovered()
            {
                let hits: Vec<&(Rect, String)> = under
                    .iter()
                    .filter(|(rect, _)| rect.expand2(Vec2::new(0.0, 2.0)).contains(pointer))
                    .collect();
                // The tooltip hangs off the underlined text, not the whole editor (whose bottom
                // edge is where a plain `on_hover_text` would put it).
                if let Some((anchor, _)) = hits.first() {
                    let messages: Vec<&str> =
                        hits.iter().map(|(_, message)| message.as_str()).collect();
                    let mut at_text = text_edit.response.response.clone();
                    at_text.interact_rect = *anchor;
                    at_text.on_hover_text(messages.join("\n"));
                }
            }

            // A name under the pointer shows its value while the script is stopped.
            if !state.debug.values.is_empty()
                && under.is_empty()
                && let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
                && text_edit.response.response.hovered()
            {
                let index = galley
                    .cursor_from_pos(pointer.to_vec2() - origin.to_vec2())
                    .index
                    .0;
                if let Some(word) = word_at(&state.source, index)
                    && let Some((value, type_name)) = state.debug.values.get(&word.text)
                {
                    let a = galley.pos_from_cursor(CCursor::new(word.range.start));
                    let b = galley.pos_from_cursor(CCursor::new(word.range.end));
                    if (a.min.y - b.min.y).abs() < 1.0 {
                        let anchor = Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y))
                            .translate(origin.to_vec2());
                        if anchor.expand2(Vec2::new(0.0, 1.0)).contains(pointer) {
                            let mut at_text = text_edit.response.response.clone();
                            at_text.interact_rect = anchor;
                            at_text
                                .on_hover_text(format!("{} = {value}  ({type_name})", word.text));
                        }
                    }
                }
            }
            // The values of the stopped line's names, after its text.
            if let Some(stop) = state.debug.stop
                && !state.debug.values.is_empty()
                && let Some(row) = galley.rows.get(stop.line.saturating_sub(1))
                && let Some(text) = state.source.split('\n').nth(stop.line.saturating_sub(1))
            {
                let inline = inline_values(text, &state.debug.values);
                if !inline.is_empty() {
                    ui.painter().text(
                        egui::pos2(
                            origin.x + row.pos.x + row.row.size.x + 3.0 * char_width,
                            origin.y + row.pos.y + row.row.size.y / 2.0,
                        ),
                        Align2::LEFT_CENTER,
                        inline,
                        FontId::monospace(FONT_SIZE - 1.5),
                        theme::text_tertiary(),
                    );
                }
            }

            // Flash the revealed position, and bring it into view.
            if let Some((index, started)) = flash {
                let age = started.elapsed();
                let cursor_rect = galley.pos_from_cursor(CCursor::new(index));
                let placed = cursor_rect.translate(origin.to_vec2());
                if age < FLASH {
                    if age.is_zero() || age < Duration::from_millis(40) {
                        ui.scroll_to_rect(
                            placed.expand2(Vec2::new(0.0, 40.0)),
                            Some(Align::Center),
                        );
                    }
                    let fade = 1.0 - age.as_secs_f32() / FLASH.as_secs_f32();
                    let size = Vec2::new(char_width * 1.2, placed.height());
                    ui.painter().rect_filled(
                        Rect::from_min_size(placed.min - Vec2::new(2.0, 0.0), size).expand(3.0),
                        4.0,
                        theme::with_alpha(theme::YELLOW, 0.35 * fade),
                    );
                    ui.ctx().request_repaint();
                }
            }
        });
    if state.flash.is_some_and(|(_, at)| at.elapsed() >= FLASH) {
        state.flash = None;
    }
    #[cfg(test)]
    {
        state.scroll_y = scroll.state.offset.y;
    }
    #[cfg(not(test))]
    let _ = scroll;
    rows
}

/// An identifier in the text, with its place as character offsets.
struct Word {
    text: String,
    range: std::ops::Range<usize>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The identifier the character at `index` is part of (or ends), if any.
fn word_at(source: &str, index: usize) -> Option<Word> {
    let chars: Vec<char> = source.chars().collect();
    let at = if chars.get(index).is_some_and(|c| is_word(*c)) {
        index
    } else if index > 0 && chars.get(index - 1).is_some_and(|c| is_word(*c)) {
        index - 1
    } else {
        return None;
    };
    let mut start = at;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    let text: String = chars[start..end].iter().collect();
    // A number, or the field of a dotted name, is not a variable.
    if text.chars().next().is_some_and(|c| c.is_ascii_digit())
        || (start > 0 && chars[start - 1] == '.')
    {
        return None;
    }
    Some(Word {
        text,
        range: start..end,
    })
}

type Values = std::collections::HashMap<String, (String, String)>;

/// `name = value` for each distinct variable named on the line, in order of appearance.
fn inline_values(line: &str, values: &Values) -> String {
    let code = line.split("//").next().unwrap_or(line);
    let mut seen: Vec<&str> = Vec::new();
    let mut parts = Vec::new();
    let mut start: Option<usize> = None;
    for (offset, c) in code
        .char_indices()
        .chain(std::iter::once((code.len(), ' ')))
    {
        match (start, is_word(c)) {
            (None, true) => start = Some(offset),
            (Some(begin), false) => {
                let word = &code[begin..offset];
                let field = code[..begin].ends_with('.');
                if !field
                    && !seen.contains(&word)
                    && let Some((value, _)) = values.get(word)
                {
                    seen.push(word);
                    parts.push(format!("{word} = {value}"));
                }
                start = None;
            }
            _ => {}
        }
    }
    parts.join("   ")
}

fn paint_gutter(ui: &Ui, gutter: Rect, rows: &GutterRows, marks: &DebugMarks) {
    let painter = ui.painter().with_clip_rect(gutter);
    let font = FontId::monospace(LINE_NUMBER_SIZE);
    for (index, (top, height)) in rows.rows.iter().enumerate() {
        if top + height < gutter.min.y || *top > gutter.max.y {
            continue;
        }
        let line = index + 1;
        let stopped = marks.stop.is_some_and(|stop| stop.line == line);
        painter.text(
            egui::pos2(gutter.max.x - 8.0, top + height / 2.0),
            Align2::RIGHT_CENTER,
            line.to_string(),
            font.clone(),
            if stopped {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
        let center = egui::pos2(gutter.min.x + MARKER_X, top + height / 2.0);
        if let Some(mark) = marks.breakpoint_at(line) {
            paint_breakpoint(&painter, center, mark);
        }
        if let Some(stop) = marks.stop.filter(|stop| stop.line == line) {
            paint_stop_arrow(&painter, center, stop);
        }
    }
}

// MARK: Profile column

/// The profiler's column: a bar and the share of the script's time beside each measured line, a
/// flame on the hottest, and a tooltip with the figures. Hovering a line washes the code block it
/// covers (the next frame; the wash is painted with the text). The column holds its width while the
/// profiler is on, so nothing moves when a profile arrives.
fn profile_gutter(ui: &mut Ui, column: Rect, rows: &GutterRows, state: &mut EditorState) {
    let Some(content) = state.profile.clone() else {
        return;
    };
    let painter = ui.painter().with_clip_rect(column);
    painter.line_segment(
        [column.right_top(), column.right_bottom()],
        Stroke::new(1.0, theme::separator()),
    );
    let response = ui.interact(column, state.id.with("profile-column"), Sense::hover());
    let row_at = |y: f32| {
        let index = rows.rows.partition_point(|(top, height)| top + height <= y);
        rows.rows
            .get(index)
            .filter(|(top, _)| *top <= y)
            .map(|_| index + 1)
    };
    let first = rows
        .rows
        .partition_point(|(top, height)| top + height < column.min.y)
        + 1;
    let last = rows.rows.partition_point(|(top, _)| *top <= column.max.y);
    let pointer_line = response.hover_pos().and_then(|pointer| row_at(pointer.y));
    // Marks of a profile whose inputs or data have changed are faded.
    let (marks, fade) = match &content {
        ProfileColumn::Marks(marks) => (marks, 1.0),
        ProfileColumn::Dated(marks) => (marks, 0.5),
        ProfileColumn::Empty(why) => {
            if response.hovered() {
                response.on_hover_text(*why);
            }
            if state.profile_hover.take().is_some() {
                ui.ctx().request_repaint();
            }
            return;
        }
    };
    let mut hovered: Option<&LineMark> = None;
    for mark in marks.in_view(first, last) {
        let (top, height) = rows.rows[mark.line - 1];
        let center_y = top + height / 2.0;
        if pointer_line == Some(mark.line) {
            hovered = Some(mark);
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(column.min.x, top),
                    egui::pos2(column.max.x, top + height),
                ),
                0.0,
                theme::ink(10),
            );
        }
        if mark.quiet {
            continue;
        }
        if mark.flame {
            icons::paint(
                &painter,
                Rect::from_center_size(
                    egui::pos2(column.min.x + 6.0 + PROFILE_FLAME / 2.0, center_y),
                    Vec2::splat(PROFILE_FLAME),
                ),
                Icon::Flame,
                theme::profiler_flame().gamma_multiply(fade),
            );
        }
        let bar_left = column.min.x + PROFILE_BAR_X;
        let track = Rect::from_min_size(
            egui::pos2(bar_left, center_y - PROFILE_BAR_HEIGHT / 2.0),
            Vec2::new(PROFILE_BAR_WIDTH, PROFILE_BAR_HEIGHT),
        );
        painter.rect_filled(track, 2.0, theme::profiler_track().gamma_multiply(fade));
        // Never thinner than a sliver, so a small share is still a mark.
        let width = (mark.bar * PROFILE_BAR_WIDTH).clamp(1.5, PROFILE_BAR_WIDTH);
        painter.rect_filled(
            Rect::from_min_size(track.min, Vec2::new(width, PROFILE_BAR_HEIGHT)),
            2.0,
            theme::profiler_heat(mark.heat).gamma_multiply(fade),
        );
        painter.text(
            egui::pos2(column.max.x - 6.0, center_y),
            Align2::RIGHT_CENTER,
            &mark.label,
            FontId::monospace(10.0),
            if mark.flame {
                theme::text_primary()
            } else {
                theme::text_secondary()
            }
            .gamma_multiply(fade),
        );
    }
    // A line that ran but took too little to draw can still be asked about.
    if hovered.is_none()
        && let Some(line) = pointer_line
    {
        hovered = marks.mark(line);
    }
    let next = hovered.and_then(|mark| mark.block.or(Some((mark.line, mark.line))));
    if state.profile_hover != next {
        state.profile_hover = next;
        ui.ctx().request_repaint();
    }
    if let Some(mark) = hovered
        && let Some(pointer) = response.hover_pos()
        && let Some((top, height)) = rows.rows.get(mark.line - 1).copied()
    {
        let mut at_line = response.clone();
        at_line.interact_rect = Rect::from_min_size(
            egui::pos2(column.min.x, top),
            Vec2::new(column.width(), height),
        );
        let _ = pointer;
        at_line.on_hover_ui(|ui| profile_tooltip(ui, mark));
    }
}

fn profile_tooltip(ui: &mut Ui, mark: &LineMark) {
    ui.set_max_width(320.0);
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.label(kit::text(
        mark.tooltip.title.clone(),
        12.0,
        kit::SEMIBOLD,
        theme::text_primary(),
    ));
    egui::Grid::new("profile-tooltip")
        .num_columns(2)
        .spacing(Vec2::new(12.0, 2.0))
        .show(ui, |ui| {
            for (label, value) in &mark.tooltip.rows {
                ui.label(kit::text(
                    *label,
                    11.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.label(kit::text(
                    value.clone(),
                    11.5,
                    kit::MEDIUM,
                    theme::text_primary(),
                ));
                ui.end_row();
            }
        });
    for note in &mark.tooltip.notes {
        ui.label(kit::text(
            note.clone(),
            11.0,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    }
}

fn paint_breakpoint(painter: &egui::Painter, center: egui::Pos2, mark: &BreakpointMark) {
    let live = mark.enabled && !mark.unverified;
    let color = if live {
        theme::editor_breakpoint()
    } else {
        theme::editor_breakpoint_muted()
    };
    let r = MARKER_RADIUS;
    match mark.kind {
        MarkKind::Logpoint => {
            let points = vec![
                center + Vec2::new(0.0, -r - 0.8),
                center + Vec2::new(r + 0.8, 0.0),
                center + Vec2::new(0.0, r + 0.8),
                center + Vec2::new(-r - 0.8, 0.0),
            ];
            if live {
                painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
            } else {
                painter.add(Shape::closed_line(points, Stroke::new(1.4, color)));
            }
        }
        MarkKind::Line | MarkKind::Conditional => {
            if live {
                painter.circle_filled(center, r, color);
            } else {
                painter.circle_stroke(center, r - 0.6, Stroke::new(1.4, color));
            }
            if mark.kind == MarkKind::Conditional {
                // An equals sign cut into the dot: it stops only when something holds.
                let ink = if live { Color32::WHITE } else { color };
                for dy in [-1.4, 1.4] {
                    painter.line_segment(
                        [center + Vec2::new(-2.4, dy), center + Vec2::new(2.4, dy)],
                        Stroke::new(1.3, ink),
                    );
                }
            }
        }
    }
}

/// The marker for the line a stopped script is at; it covers a breakpoint on the same line.
fn paint_stop_arrow(painter: &egui::Painter, center: egui::Pos2, stop: StopMark) {
    let color = if stop.error {
        theme::editor_breakpoint()
    } else {
        theme::editor_debug_line()
    };
    let points = vec![
        center + Vec2::new(-5.6, -3.8),
        center + Vec2::new(0.6, -3.8),
        center + Vec2::new(5.8, 0.0),
        center + Vec2::new(0.6, 3.8),
        center + Vec2::new(-5.6, 3.8),
    ];
    if stop.innermost || stop.error {
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    } else {
        // A caller's line: the same arrow, outlined, over the gutter so a breakpoint beneath does
        // not show through it.
        painter.add(Shape::convex_polygon(
            points,
            theme::gutter_background(),
            Stroke::new(1.5, color),
        ));
    }
}

/// The breakpoint column: a click on a line (its number or the marker beside it) sets or clears its
/// breakpoint, and a right-click opens that breakpoint's menu. Painted over by `paint_gutter`.
fn breakpoint_gutter(
    ui: &mut Ui,
    gutter: Rect,
    rows: &GutterRows,
    state: &mut EditorState,
) -> Vec<GutterEvent> {
    let hit = Rect::from_min_max(
        egui::pos2(gutter.min.x + CHANGE_HIT, gutter.min.y),
        gutter.max,
    );
    let response = ui.interact(hit, state.id.with("breakpoints"), Sense::click());
    let response = accessibility::label(
        response,
        "Breakpoint gutter: click a line to set a breakpoint",
    );
    let row_at = |y: f32| {
        let index = rows.rows.partition_point(|(top, height)| top + height <= y);
        rows.rows
            .get(index)
            .filter(|(top, _)| *top <= y)
            .map(|_| index + 1)
    };
    let mut events = Vec::new();
    if let Some(pointer) = response.hover_pos()
        && let Some(line) = row_at(pointer.y)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if state.debug.breakpoint_at(line).is_none() {
            let (top, height) = rows.rows[line - 1];
            // A faint dot where a click would put one.
            ui.painter().with_clip_rect(gutter).circle_filled(
                egui::pos2(gutter.min.x + MARKER_X, top + height / 2.0),
                MARKER_RADIUS,
                theme::with_alpha(theme::editor_breakpoint(), 0.32),
            );
        }
    }
    let clicked_line = |position: Option<egui::Pos2>| position.and_then(|p| row_at(p.y));
    if response.clicked()
        && let Some(line) = clicked_line(response.interact_pointer_pos())
    {
        events.push(GutterEvent::Toggle { line });
    }
    if response.secondary_clicked()
        && let Some(line) = clicked_line(response.interact_pointer_pos())
    {
        state.menu_line = Some(line);
    }
    if let Some(line) = state.menu_line {
        let mark = state.debug.breakpoint_at(line).copied();
        egui::Popup::context_menu(&response)
            .frame(kit::menu_frame())
            .show(|ui| {
                kit::menu_fit(
                    ui,
                    if mark.is_some() {
                        &[
                            "Edit Breakpoint…",
                            "Disable Breakpoint",
                            "Remove Breakpoint",
                        ]
                    } else {
                        &[
                            "Add Breakpoint",
                            "Add Conditional Breakpoint…",
                            "Add Logpoint…",
                        ]
                    },
                );
                ui.spacing_mut().item_spacing.y = 0.0;
                kit::menu_heading(ui, &format!("Line {line}"));
                match mark {
                    Some(mark) => {
                        if kit::menu_item(ui, Icon::Pencil, "Edit Breakpoint…", true, false) {
                            events.push(GutterEvent::Edit { line });
                            ui.close();
                        }
                        let (icon, label) = if mark.enabled {
                            (Icon::EyeSlash, "Disable Breakpoint")
                        } else {
                            (Icon::Eye, "Enable Breakpoint")
                        };
                        if kit::menu_item(ui, icon, label, true, false) {
                            events.push(GutterEvent::SetEnabled {
                                line,
                                enabled: !mark.enabled,
                            });
                            ui.close();
                        }
                        if kit::menu_item(ui, Icon::Trash, "Remove Breakpoint", true, true) {
                            events.push(GutterEvent::Toggle { line });
                            ui.close();
                        }
                    }
                    None => {
                        if kit::menu_item(ui, Icon::Plus, "Add Breakpoint", true, false) {
                            events.push(GutterEvent::Toggle { line });
                            ui.close();
                        }
                        if kit::menu_item(
                            ui,
                            Icon::Pencil,
                            "Add Conditional Breakpoint…",
                            true,
                            false,
                        ) {
                            events.push(GutterEvent::Edit { line });
                            ui.close();
                        }
                        if kit::menu_item(ui, Icon::Code, "Add Logpoint…", true, false) {
                            events.push(GutterEvent::AddLogpoint { line });
                            ui.close();
                        }
                    }
                }
            });
    }
    events
}

// MARK: Change gutter

/// What a context-menu row on a change asks for.
#[derive(Clone, Copy)]
enum ChangeCommand {
    Show,
    Revert,
    CopyOriginal,
    CopyCurrent,
}

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// "Line 12", "Lines 18–20": a lines label starting a sentence.
fn sentence_lines(range: &std::ops::Range<usize>) -> String {
    let label = diff_view::lines_label(range);
    let mut chars = label.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The hover text: the count and what a click does, in words (the colour is not the message).
fn change_tooltip(hunk: &DiffHunk) -> String {
    match hunk.kind {
        ChangeKind::Added => format!(
            "{} added — Click to inspect",
            plural(hunk.added_lines(), "line")
        ),
        ChangeKind::Deleted => format!(
            "{} deleted — Click to restore",
            plural(hunk.removed_lines(), "line")
        ),
        ChangeKind::Modified => format!(
            "{} modified — Click to compare",
            sentence_lines(&hunk.new_lines)
        ),
    }
}

/// The spoken description of a marker.
fn change_description(hunk: &DiffHunk) -> String {
    match hunk.kind {
        ChangeKind::Added => format!("{}: Added since last save", sentence_lines(&hunk.new_lines)),
        ChangeKind::Modified => format!(
            "{}: Modified since last save",
            sentence_lines(&hunk.new_lines)
        ),
        ChangeKind::Deleted if hunk.new_lines.start == 0 => format!(
            "At the start: {} deleted since last save",
            plural(hunk.removed_lines(), "line")
        ),
        ChangeKind::Deleted => format!(
            "After line {}: {} deleted since last save",
            hunk.new_lines.start,
            plural(hunk.removed_lines(), "line")
        ),
    }
}

fn change_color(kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added => theme::editor_diff_added(),
        ChangeKind::Deleted => theme::editor_diff_deleted(),
        ChangeKind::Modified => theme::editor_diff_modified(),
    }
}

/// A change's vertical extent on screen, from the rows the text view laid out. A deleted change
/// has no rows of its own: it is the boundary above the line it used to precede (the foot of the
/// last row at the end of the text), so its extent is one y.
fn change_span(rows: &GutterRows, hunk: &DiffHunk, empty_top: f32) -> (f32, f32) {
    let top_of = |line: usize| rows.rows.get(line).map(|(top, _)| *top);
    match hunk.kind {
        ChangeKind::Deleted => {
            let y = top_of(hunk.new_lines.start)
                .or_else(|| rows.rows.last().map(|(top, height)| top + height))
                .unwrap_or(empty_top);
            (y, y)
        }
        ChangeKind::Added | ChangeKind::Modified => {
            let top = top_of(hunk.new_lines.start).unwrap_or(empty_top);
            let bottom = rows
                .rows
                .get(hunk.new_lines.end.saturating_sub(1))
                .map_or(top, |(row_top, height)| row_top + height);
            (top, bottom)
        }
    }
}

/// The clickable strip of a change at the gutter's edge.
fn change_hit(gutter: Rect, span: (f32, f32), kind: ChangeKind) -> Rect {
    let (top, bottom) = match kind {
        ChangeKind::Deleted => (span.0 - 6.0, span.1 + 6.0),
        _ => span,
    };
    Rect::from_min_max(
        egui::pos2(gutter.min.x, top),
        egui::pos2(gutter.min.x + CHANGE_HIT, bottom),
    )
    .intersect(gutter)
}

/// Paints and handles the change markers of the visible lines, and hosts the open comparison.
/// Returns whether a revert changed the text.
fn change_gutter(
    ui: &mut Ui,
    gutter: Rect,
    rows: &GutterRows,
    state: &mut EditorState,
    changes: &DiffSnapshot,
) -> bool {
    let ctx = ui.ctx().clone();
    let empty_top = gutter.min.y + TEXT_INSET;
    let first = rows
        .rows
        .partition_point(|(top, height)| top + height < gutter.min.y);
    let last = rows
        .rows
        .partition_point(|(top, _)| *top <= gutter.max.y)
        .saturating_sub(1);
    let painter = ui.painter().with_clip_rect(gutter);
    let dragging = ui.input(|input| input.pointer.any_down());
    let bar_x = gutter.min.x + CHANGE_INSET;
    let mut command: Option<(DiffHunk, ChangeCommand)> = None;
    for index in changes.in_view(first, last) {
        let hunk = &changes.hunks()[index];
        let span = change_span(rows, hunk, empty_top);
        let hit = change_hit(gutter, span, hunk.kind);
        if hit.height() <= 0.0 {
            continue;
        }
        let response = ui.interact(hit, state.id.with(("change", index)), Sense::click());
        let response = accessibility::label(response, change_description(hunk));
        let hovered = response.hovered();
        if hovered {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let color = change_color(hunk.kind);
        match hunk.kind {
            ChangeKind::Deleted => {
                // A wedge pointing into the text at the boundary where the lines were.
                let (x, y) = (bar_x, span.0);
                let reach = if hovered { 7.0 } else { 5.0 };
                painter.add(Shape::convex_polygon(
                    vec![
                        egui::pos2(x, y - WEDGE_HALF),
                        egui::pos2(x + reach, y),
                        egui::pos2(x, y + WEDGE_HALF),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
            ChangeKind::Added | ChangeKind::Modified => {
                let width = CHANGE_BAR + if hovered { 1.5 } else { 0.0 };
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(bar_x, span.0 + 1.0),
                        egui::pos2(bar_x + width, (span.1 - 1.0).max(span.0 + 1.0)),
                    ),
                    1.5,
                    color,
                );
            }
        }
        if !dragging {
            response.clone().on_hover_text(change_tooltip(hunk));
        }
        if response.clicked() {
            let open = state.preview.as_ref().is_some_and(|p| p.same_change(hunk));
            state.preview = (!open).then(|| hunk.clone());
        }
        egui::Popup::context_menu(&response)
            .frame(kit::menu_frame())
            .show(|ui| {
                ui.set_width(200.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                let (show, revert, original, current) = match hunk.kind {
                    ChangeKind::Added => (
                        "Show Changes",
                        "Revert Added Lines",
                        None,
                        Some("Copy Current Lines"),
                    ),
                    ChangeKind::Deleted => (
                        "Show Deleted Lines",
                        "Restore Deleted Lines",
                        Some("Copy Original Lines"),
                        None,
                    ),
                    ChangeKind::Modified => (
                        "Compare Changes",
                        "Revert Change",
                        Some("Copy Original"),
                        Some("Copy Current"),
                    ),
                };
                let rows = [
                    (Icon::Eye, show, Some(ChangeCommand::Show)),
                    (Icon::Undo, revert, Some(ChangeCommand::Revert)),
                    (
                        Icon::CopyDoc,
                        original.unwrap_or(""),
                        original.map(|_| ChangeCommand::CopyOriginal),
                    ),
                    (
                        Icon::CopyDoc,
                        current.unwrap_or(""),
                        current.map(|_| ChangeCommand::CopyCurrent),
                    ),
                ];
                for (icon, label, chosen) in rows {
                    if let Some(chosen) = chosen
                        && kit::menu_item(ui, icon, label, true, false)
                    {
                        command = Some((hunk.clone(), chosen));
                        ui.close();
                    }
                }
            });
    }

    let mut edited = false;
    if let Some((hunk, chosen)) = command {
        match chosen {
            ChangeCommand::Show => state.preview = Some(hunk),
            ChangeCommand::Revert => edited |= apply_hunk_edit(&ctx, state, &hunk),
            ChangeCommand::CopyOriginal => ctx.copy_text(hunk.old_text),
            ChangeCommand::CopyCurrent => ctx.copy_text(hunk.new_text),
        }
    }

    // The open comparison follows its change through edits elsewhere and closes when the change
    // is gone (reverted by hand, saved, or edited into something else).
    let Some(wanted) = state.preview.clone() else {
        return edited;
    };
    let Some(index) = changes.find(&wanted) else {
        state.preview = None;
        return edited;
    };
    let hunk = changes.hunks()[index].clone();
    let span = change_span(rows, &hunk, empty_top);
    let anchor = Rect::from_min_max(
        egui::pos2(gutter.min.x, span.0.clamp(gutter.min.y, gutter.max.y)),
        egui::pos2(
            gutter.min.x + CHANGE_HIT,
            span.1.clamp(gutter.min.y, gutter.max.y),
        ),
    )
    .expand2(Vec2::new(0.0, 2.0));
    let event = diff_view::show(
        &ctx,
        state.id.with("change-preview"),
        anchor,
        &hunk,
        (index, changes.hunks().len()),
    );
    match event {
        PreviewEvent::None => {}
        PreviewEvent::Dismiss => state.preview = None,
        PreviewEvent::Apply => edited |= apply_hunk_edit(&ctx, state, &hunk),
        PreviewEvent::Step(forward) => {
            if let Some(next) = changes.neighbour(index, forward) {
                let next = &changes.hunks()[next];
                state.scroll_to_line = Some(next.new_lines.start);
                state.preview = Some(next.clone());
                ctx.request_repaint();
            }
        }
    }
    edited
}

/// Puts the saved lines back for `hunk`, through the same edit the typing assists use, as one
/// undo step. Refuses (and closes the comparison) if the text is no longer what the hunk was
/// computed from. Returns whether the text changed.
fn apply_hunk_edit(ctx: &egui::Context, state: &mut EditorState, hunk: &DiffHunk) -> bool {
    state.preview = None;
    let Ok(edit) = diff::revert_edit(hunk, &state.source) else {
        return false;
    };
    let mut edit_state = TextEditState::load(ctx, state.id).unwrap_or_default();
    // egui folds edits made within a second of each other into one undo step; mark the text as it
    // stands now so undoing the revert returns to exactly this, not to before the last keystrokes.
    let before = edit_state
        .cursor
        .char_range()
        .unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
    let mut undoer = edit_state.undoer();
    undoer.add_undo(&(before, state.source.clone()));
    state.source = edit.apply(&state.source);
    edit_state.set_undoer(undoer);
    edit_state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(edit.selection.start),
        CCursor::new(edit.selection.end),
    )));
    edit_state.store(ctx, state.id);
    // Whatever pointed into the old text is stale now.
    state.completion = None;
    state.completion_wanted = false;
    state.completion_explicit = false;
    state.select_to = None;
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    ctx.request_repaint();
    true
}

/// Leading-space level of each line, with blank lines keeping only the guides both neighbours
/// share.
fn indent_levels(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.split('\n').collect();
    let raw: Vec<Option<usize>> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                None
            } else {
                let mut width = 0;
                for c in line.chars() {
                    match c {
                        ' ' => width += 1,
                        '\t' => width += 4,
                        _ => break,
                    }
                }
                Some(width / 4)
            }
        })
        .collect();
    (0..raw.len())
        .map(|i| match raw[i] {
            Some(level) => level,
            None => {
                let before = raw[..i].iter().rev().find_map(|l| *l);
                let after = raw[i + 1..].iter().find_map(|l| *l);
                match (before, after) {
                    (Some(a), Some(b)) => a.min(b),
                    _ => 0,
                }
            }
        })
        .collect()
}

fn indent_guides(
    source: &str,
    galley: &egui::Galley,
    origin: egui::Pos2,
    char_width: f32,
    caret_index: Option<usize>,
) -> Vec<Shape> {
    let levels = indent_levels(source);
    let caret_line = caret_index.map(|c| source.chars().take(c).filter(|&ch| ch == '\n').count());
    let active_level = caret_line.and_then(|line| levels.get(line).copied());
    // The active guide runs over the contiguous lines around the caret that still have it.
    let active_span = caret_line
        .zip(active_level)
        .filter(|(_, l)| *l > 0)
        .map(|(line, level)| {
            let mut start = line;
            while start > 0 && levels[start - 1] >= level {
                start -= 1;
            }
            let mut end = line;
            while end + 1 < levels.len() && levels[end + 1] >= level {
                end += 1;
            }
            (start, end, level)
        });
    let mut shapes = Vec::new();
    for (line, level) in levels.iter().enumerate() {
        let Some(row) = galley.rows.get(line) else {
            break;
        };
        let top = origin.y + row.pos.y;
        let bottom = top + row.row.size.y;
        for guide in 1..=*level {
            let x = origin.x + char_width * (4 * (guide - 1)) as f32 + 0.5;
            let active = active_span.is_some_and(|(s, e, l)| guide == l && line >= s && line <= e);
            let color = if active {
                theme::text_secondary().gamma_multiply(0.6)
            } else {
                theme::ink(22)
            };
            shapes.push(Shape::line_segment(
                [egui::pos2(x, top), egui::pos2(x, bottom)],
                Stroke::new(1.0, color),
            ));
        }
    }
    shapes
}

/// The screen rect a diagnostic underlines: its start to its end on the first line it touches
/// (one character wide when empty).
fn underline_rect(
    source: &str,
    galley: &egui::Galley,
    origin: egui::Pos2,
    diagnostic: &Diagnostic,
) -> Option<Rect> {
    let start = syntax::compiled_utf16_to_char_index(source, diagnostic.range.start.offset);
    let mut end =
        syntax::compiled_utf16_to_char_index(source, diagnostic.range.end.offset).max(start);
    // Underline only the first line of a multi-line range.
    let line_end = source
        .chars()
        .skip(start)
        .position(|c| c == '\n')
        .map_or(source.chars().count(), |n| start + n);
    end = end.min(line_end);
    if end == start {
        end = (start + 1).min(line_end.max(start + 1));
    }
    let a = galley.pos_from_cursor(CCursor::new(start));
    let b = galley.pos_from_cursor(CCursor::new(end));
    if (a.min.y - b.min.y).abs() > 1.0 {
        return None;
    }
    Some(
        Rect::from_min_max(
            egui::pos2(a.min.x, a.min.y),
            egui::pos2(b.min.x.max(a.min.x + 4.0), a.max.y),
        )
        .translate(origin.to_vec2()),
    )
}

// MARK: Editing assists

/// Handles Return, Tab, Backspace, paired typing and the line shortcuts before the text field
/// sees them. Returns whether any edit changed the text.
fn apply_assists(ctx: &egui::Context, state: &mut EditorState) -> bool {
    let id = state.id;
    if !ctx.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    let Some(mut edit_state) = TextEditState::load(ctx, id) else {
        return false;
    };
    let mut selection = edit_state
        .cursor
        .char_range()
        .map(|range| {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            a.min(b)..a.max(b)
        })
        .unwrap_or(0..0);
    let mut changed = false;
    let mut handled_any = false;

    let events = ctx.input(|input| input.events.clone());
    let mut remaining = Vec::with_capacity(events.len());
    for event in events {
        let edit = match &event {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => key_edit(&state.source, selection.clone(), *key, *modifiers),
            Event::Text(text) => {
                edit_ops::typed(&Context::new(&state.source, selection.clone()), text)
            }
            _ => None,
        };
        match edit {
            Some(edit) => {
                state.source = edit.apply(&state.source);
                selection = edit.selection.clone();
                changed = true;
                handled_any = true;
            }
            None => remaining.push(event),
        }
    }
    if handled_any {
        ctx.input_mut(|input| input.events = remaining);
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(selection.start),
            CCursor::new(selection.end),
        )));
        edit_state.store(ctx, id);
    }
    changed
}

fn key_edit(
    source: &str,
    selection: std::ops::Range<usize>,
    key: Key,
    modifiers: Modifiers,
) -> Option<Edit> {
    let context = Context::new(source, selection);
    let plain = !modifiers.command && !modifiers.alt && !modifiers.ctrl;
    let is_mac = shortcut::is_mac();
    let chord = |shortcut: shortcut::Shortcut| shortcut.matches_for(is_mac, key, modifiers);
    match key {
        Key::Enter if plain && !modifiers.shift => Some(edit_ops::newline(&context)),
        Key::Tab if plain && !modifiers.shift => Some(edit_ops::indent(&context)),
        Key::Tab if plain && modifiers.shift => edit_ops::outdent(&context),
        Key::Backspace if plain && !modifiers.shift => edit_ops::backspace(&context),
        Key::Slash if chord(shortcut::TOGGLE_COMMENT) => edit_ops::toggle_comment(&context),
        Key::ArrowUp | Key::ArrowDown => {
            let down = key == Key::ArrowDown;
            if chord(shortcut::DUPLICATE_LINE_UP) || chord(shortcut::DUPLICATE_LINE_DOWN) {
                edit_ops::duplicate(&context, down)
            } else if chord(shortcut::MOVE_LINE_UP) || chord(shortcut::MOVE_LINE_DOWN) {
                edit_ops::move_lines(&context, down)
            } else {
                None
            }
        }
        _ => None,
    }
}

// MARK: Status bar

/// The chip and the Save button. Returns whether Save was clicked.
fn status_bar(ui: &mut Ui, bar: Rect, state: &EditorState, changes: &DiffSnapshot) -> bool {
    super::bottom_bar::paint(ui, bar);
    let mut save = false;
    let mut inner = ui.new_child(
        UiBuilder::new()
            .max_rect(bar.shrink2(Vec2::new(12.0, 0.0)))
            .layout(egui::Layout::left_to_right(Align::Center)),
    );
    let (icon, color, text) = status_chip(state.status, &state.diagnostics);
    inner.spacing_mut().item_spacing.x = 5.0;
    icons::inline(&mut inner, icon, 12.0, color);
    inner
        .label(egui::RichText::new(text).size(10.0).color(color))
        .on_hover_text("Compile status of the code in the editor");
    if !changes.is_clean() {
        let (added, removed) = (changes.lines_added(), changes.lines_removed());
        inner.add_space(6.0);
        inner
            .label(
                RichText::new(format!("+{added}"))
                    .size(10.0)
                    .color(theme::editor_diff_added()),
            )
            .on_hover_text(format!("{added} lines added since the last save"));
        inner
            .label(
                RichText::new(format!("−{removed}"))
                    .size(10.0)
                    .color(theme::editor_diff_deleted()),
            )
            .on_hover_text(format!("{removed} lines removed since the last save"));
    }
    inner.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        // egui never draws a button under `interact_size.y`, so the bar lowers it.
        ui.spacing_mut().interact_size.y = 20.0;
        let button = egui::Button::new(egui::RichText::new("Save").size(12.0));
        if ui
            .add(button)
            .on_hover_text(shortcut::SAVE.hint("Save the script"))
            .clicked()
        {
            save = true;
        }
    });
    save
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::{DiagnosticCategory, SourcePosition, SourceRange};

    fn diagnostic(
        severity: DiagnosticSeverity,
        line: usize,
        column: usize,
        offset: usize,
    ) -> Diagnostic {
        Diagnostic {
            code: "PINE3001".into(),
            severity,
            category: DiagnosticCategory::Semantic,
            message: "message".into(),
            range: SourceRange::new(
                SourcePosition::new(line, column, offset),
                SourcePosition::new(line, column + 1, offset + 1),
            ),
        }
    }

    #[test]
    fn the_chip_reads_like_swift() {
        let error = diagnostic(DiagnosticSeverity::Error, 1, 1, 0);
        let warning = diagnostic(DiagnosticSeverity::Warning, 1, 1, 0);
        assert_eq!(
            status_chip(CompileStatus::NotCompiled, &[]).2,
            "Not compiled"
        );
        assert_eq!(status_chip(CompileStatus::Valid, &[]).2, "No problems");
        assert_eq!(
            status_chip(CompileStatus::Warning, std::slice::from_ref(&warning)).2,
            "1 warning"
        );
        assert_eq!(
            status_chip(CompileStatus::Warning, &[warning.clone(), warning]).2,
            "2 warnings"
        );
        assert_eq!(
            status_chip(CompileStatus::Error, &[error.clone(), error]).2,
            "2 errors"
        );
    }

    #[test]
    fn blank_lines_keep_only_shared_guides() {
        let levels = indent_levels("a\n    b\n        c\n\n    d\n\n");
        assert_eq!(levels, [0, 1, 2, 1, 1, 0, 0]);
    }

    #[test]
    fn a_compile_waits_for_typing_to_pause() {
        let mut state = EditorState::new("s", "plot(close)", "plot(close)");
        assert!(!state.compile_pending());
        state.request_compile();
        let start = state.compile_requested.unwrap();
        assert!(state.compile_pending());
        assert!(!state.compile_due(start + Duration::from_millis(100)));
        assert!(state.compile_due(start + COMPILE_DELAY));
        // Another keystroke starts the wait again.
        state.request_compile();
        let again = state.compile_requested.unwrap();
        assert!(!state.compile_due(again + COMPILE_DELAY - Duration::from_millis(1)));
        state.compiled();
        assert!(!state.compile_pending());
        assert!(!state.compile_due(again + COMPILE_DELAY));
    }

    #[test]
    fn dirty_tracking_and_draft_debounce() {
        let mut state = EditorState::new("s", "plot(close)", "plot(close)");
        assert!(!state.is_dirty());
        state.source.push('\n');
        assert!(state.is_dirty());
        let start = Instant::now();
        state.last_edit = Some(start);
        assert!(!state.take_draft_due(start + Duration::from_millis(400)));
        assert!(state.take_draft_due(start + DRAFT_DELAY + Duration::from_millis(10)));
        assert!(
            !state.take_draft_due(start + Duration::from_secs(30)),
            "once per edit"
        );
        state.mark_saved();
        assert!(!state.is_dirty());
    }

    #[test]
    fn the_editor_draws_headlessly_with_problems_and_a_jump() {
        let mut state = EditorState::new("s", "//@version=6\nindicator(\"x\")\nplot(close)\n", "");
        state.diagnostics = vec![diagnostic(DiagnosticSeverity::Error, 3, 1, 26)];
        state.status = CompileStatus::Error;
        let ctx = egui::Context::default();
        for frame in 0..3 {
            if frame == 1 {
                let d = state.diagnostics[0].clone();
                state.reveal(&d);
            }
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let result = show(ui, &mut state);
                assert!(!result.save);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn a_diagnostic_tooltip_hangs_off_the_underlined_text() {
        let mut state = EditorState::new("s", "//@version=6\nindicator(\"x\")\nplot(close)\n", "");
        // `plot` on the third line.
        state.diagnostics = vec![diagnostic(DiagnosticSeverity::Error, 3, 1, 26)];
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
        let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(900.0, 700.0));
        let mut time = 0.0;
        let mut frame = |state: &mut EditorState, pointer: Option<egui::Pos2>| {
            time += 0.1;
            let mut input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                ..Default::default()
            };
            input.events.push(match pointer {
                Some(pos) => egui::Event::PointerMoved(pos),
                None => egui::Event::PointerGone,
            });
            let mut output = ctx.run_ui(input, |ui| {
                show(ui, state);
            });
            output.textures_delta.clear();
        };
        let tooltip_top = |ctx: &egui::Context| {
            ctx.memory(|memory| {
                memory
                    .layer_ids()
                    .filter(|layer| layer.order == egui::Order::Tooltip)
                    .find_map(|layer| memory.area_rect(layer.id))
            })
        };
        // Sweep the pointer over the top rows until the underline shows its message.
        let mut found = None;
        'sweep: for y in (10..40).step_by(3) {
            for x in (100..200).step_by(4) {
                let pos = egui::pos2(x as f32, y as f32);
                for _ in 0..3 {
                    frame(&mut state, Some(pos));
                }
                if let Some(rect) = tooltip_top(&ctx) {
                    found = Some((pos, rect));
                    break 'sweep;
                }
            }
        }
        let (pointer, tooltip) = found.expect("hovering the underline shows its message");
        assert!(
            tooltip.min.y < pointer.y + 40.0,
            "tooltip at y={} for a pointer at y={}",
            tooltip.min.y,
            pointer.y
        );
    }

    #[test]
    fn the_find_bar_steps_through_matches_and_draws() {
        let mut state = EditorState::new("s", "plot(close)\nplot(open)\nplot(high)\n", "");
        state.find.open = true;
        state.find.query = "PLOT".into();
        step_match(&mut state, true);
        assert_eq!(state.find.current, 1);
        assert_eq!(state.select_to, Some(12..16));
        step_match(&mut state, true);
        step_match(&mut state, true);
        assert_eq!(state.find.current, 0, "wraps past the last match");
        step_match(&mut state, false);
        assert_eq!(state.find.current, 2, "and back");
        state.find.query = "nothing".into();
        step_match(&mut state, true);
        assert_eq!(state.find.current, 2, "no matches: nowhere to go");

        state.find.query = "plot".into();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, &mut state);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn accepting_a_suggestion_replaces_the_typed_prefix() {
        let mut state = EditorState::new("s", "x = ta.pi", "");
        let caret = state.source.chars().count();
        let completion = completion::complete(&state.source, caret, false).expect("suggestions");
        assert_eq!(completion.items[0].label, "ta.pivothigh");
        state.completion = Some(CompletionView {
            completion,
            selected: 1,
            scroll_to_selected: false,
        });
        let ctx = egui::Context::default();
        assert!(accept_completion(&ctx, &mut state));
        assert_eq!(state.source, "x = ta.pivotlow()");
        assert!(state.completion.is_none());
        assert!(
            !accept_completion(&ctx, &mut state),
            "nothing left to accept"
        );
    }

    #[test]
    fn the_editor_draws_with_suggestions_open() {
        let mut state = EditorState::new("s", "plot(cl", "");
        state.completion_wanted = true;
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, &mut state);
            });
            output.textures_delta.clear();
        }
    }

    /// Accepts the suggestion `label` for `source` (caret at its end); returns the new text and caret.
    fn accept(source: &str, label: &str) -> (String, usize) {
        let mut state = EditorState::new("s", source, "");
        let caret = source.chars().count();
        let completion = completion::complete(source, caret, true).expect("suggestions");
        let selected = completion
            .items
            .iter()
            .position(|item| item.label == label)
            .expect("the suggestion is offered");
        state.completion = Some(CompletionView {
            completion,
            selected,
            scroll_to_selected: false,
        });
        let ctx = egui::Context::default();
        TextEditState::default().store(&ctx, state.id);
        assert!(accept_completion(&ctx, &mut state));
        let caret = TextEditState::load(&ctx, state.id)
            .and_then(|edit| edit.cursor.char_range())
            .map(|range| range.primary.index.0);
        (state.source, caret.unwrap_or(usize::MAX))
    }

    #[test]
    fn accepting_a_function_adds_parentheses_with_the_caret_between() {
        assert_eq!(accept("x = ta.sm", "ta.sma"), ("x = ta.sma()".into(), 11));
        assert_eq!(
            accept("c = color.ne", "color.new"),
            ("c = color.new()".into(), 14)
        );
        let own = "half(x) => x / 2\nha";
        assert_eq!(accept(own, "half"), ("half(x) => x / 2\nhalf()".into(), 22));
    }

    #[test]
    fn accepting_anything_else_adds_no_parentheses() {
        assert_eq!(
            accept("c = color.re", "color.red"),
            ("c = color.red".into(), 13)
        );
        assert_eq!(accept("x = ba", "barstate"), ("x = barstate".into(), 12));
        assert_eq!(accept("x = clo", "close"), ("x = close".into(), 9));
        // The call is already there: only the name is replaced.
        let (text, caret) = accept("ta.sm", "ta.sma");
        assert_eq!((text.as_str(), caret), ("ta.sma()", 7));
        let ctx = egui::Context::default();
        let mut state = EditorState::new("s", "ta.sm(close, 3)", "");
        let completion = completion::complete(&state.source, 5, true).expect("suggestions");
        let selected = completion
            .items
            .iter()
            .position(|i| i.label == "ta.sma")
            .unwrap();
        state.completion = Some(CompletionView {
            completion,
            selected,
            scroll_to_selected: false,
        });
        assert!(accept_completion(&ctx, &mut state));
        assert_eq!(state.source, "ta.sma(close, 3)");
    }

    /// Draws the editor with `source` focused and the caret at its end, `frames` times, sending
    /// `events` on the first of them; returns the last frame's shapes.
    fn draw_suggestions(
        ctx: &egui::Context,
        state: &mut EditorState,
        clock: &mut f64,
        events: Vec<Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut shapes = Vec::new();
        for frame in 0..3 {
            let raw = egui::RawInput {
                time: Some(*clock),
                events: if frame == 0 {
                    events.clone()
                } else {
                    Vec::new()
                },
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                show(ui, state);
            });
            output.textures_delta.clear();
            shapes = output.shapes;
            // Long scrolls animate: leave time for them to land.
            *clock += 1.0;
        }
        shapes
    }

    /// The rect of the highlighted suggestion and the clip it is drawn through.
    fn highlighted_row(shapes: &[egui::epaint::ClippedShape]) -> Option<(Rect, Rect)> {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect)
                if rect.fill == theme::SYSTEM_ACCENT
                    && rect.rect.height() == COMPLETION_ROW_HEIGHT =>
            {
                Some((rect.rect, clipped.clip_rect))
            }
            _ => None,
        })
    }

    #[test]
    fn the_suggestion_list_shows_ten_rows_and_follows_the_arrow_keys() {
        let ctx = egui::Context::default();
        let mut state = EditorState::new("s", "color.", "");
        let mut clock = 0.0;
        let mut edit = TextEditState::default();
        edit.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(6usize))));
        edit.store(&ctx, state.id);
        ctx.memory_mut(|memory| memory.request_focus(state.id));
        state.completion_wanted = true;
        state.last_caret = Some(6);
        draw_suggestions(&ctx, &mut state, &mut clock, Vec::new());
        let count = state
            .completion
            .as_ref()
            .expect("suggestions")
            .completion
            .items
            .len();
        assert!(count > COMPLETION_ROWS, "color. offers {count}");
        let area = ctx
            .memory(|memory| memory.area_rect(Id::new("script-completion")))
            .expect("the list was laid out");
        let rows = COMPLETION_ROWS as f32 * COMPLETION_ROW_HEIGHT;
        assert!(
            area.height() >= rows && area.height() < rows + 40.0,
            "the list is {} tall for {rows} of rows",
            area.height()
        );

        // Down through more rows than fit: the highlight stays inside what the list shows.
        for step in 1..=count + 2 {
            let shapes = draw_suggestions(
                &ctx,
                &mut state,
                &mut clock,
                vec![key_press(Key::ArrowDown)],
            );
            let (row, clip) = highlighted_row(&shapes).expect("a highlighted row");
            assert!(
                clip.contains_rect(row),
                "step {step}: the highlight {row:?} is outside the list {clip:?}"
            );
        }
        // Up wraps to the last row and scrolls back through the list.
        for step in 1..=count + 2 {
            let shapes =
                draw_suggestions(&ctx, &mut state, &mut clock, vec![key_press(Key::ArrowUp)]);
            let (row, clip) = highlighted_row(&shapes).expect("a highlighted row");
            assert!(
                clip.contains_rect(row),
                "up step {step}: the highlight {row:?} is outside the list {clip:?}"
            );
        }
    }

    fn key_press(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn the_editor_draws_a_signature_panel_inside_a_call() {
        let mut state = EditorState::new("s", "plot(close, ", "");
        let ctx = egui::Context::default();
        // Put the caret at the end of the text and focus the editor, then draw a few frames.
        let mut edit = TextEditState::default();
        edit.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(12usize))));
        edit.store(&ctx, state.id);
        ctx.memory_mut(|memory| memory.request_focus(state.id));
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, &mut state);
            });
            output.textures_delta.clear();
        }
        assert!(signature::help(&state.source, 12).is_some());
        assert!(
            ctx.memory(|memory| memory.area_rect(Id::new("script-signature")))
                .is_some(),
            "the panel was laid out"
        );
    }

    /// The signature panel's rect for a caret inside `source`, in a window `width` wide.
    fn signature_panel(source: &str, width: f32) -> Rect {
        let mut state = EditorState::new("s", source, "");
        let ctx = egui::Context::default();
        let caret = source.chars().count();
        let mut edit = TextEditState::default();
        edit.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
        edit.store(&ctx, state.id);
        ctx.memory_mut(|memory| memory.request_focus(state.id));
        for _ in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    Vec2::new(width, 700.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                show(ui, &mut state);
            });
            output.textures_delta.clear();
        }
        ctx.memory(|memory| memory.area_rect(Id::new("script-signature")))
            .expect("the panel was laid out")
    }

    #[test]
    fn a_long_signature_stays_on_one_line_while_the_window_has_room() {
        // `strategy(...)` is ~135 characters: far past the old 540-point wrap.
        let wide = signature_panel("strategy(", 1600.0);
        assert!(wide.width() > 540.0, "{}", wide.width());
        assert!(wide.height() < 40.0, "one line, got {}", wide.height());
        assert!(wide.right() <= 1600.0 - 8.0 + 0.5, "inside the window");
    }

    #[test]
    fn a_signature_wraps_only_when_the_window_is_too_narrow_and_stays_inside_it() {
        let narrow = signature_panel("strategy(", 480.0);
        assert!(narrow.height() > 40.0, "wrapped, got {}", narrow.height());
        assert!(
            narrow.left() >= 7.5 && narrow.right() <= 480.0 - 7.5,
            "{narrow:?}"
        );
    }

    // MARK: Changes against the saved text

    fn run_frame(ctx: &egui::Context, state: &mut EditorState, time: f64, events: Vec<Event>) {
        let raw = egui::RawInput {
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            show(ui, state);
        });
        output.textures_delta.clear();
    }

    fn undo_key(redo: bool) -> Event {
        Event::Key {
            key: Key::Z,
            physical_key: Some(Key::Z),
            pressed: true,
            repeat: false,
            modifiers: Modifiers {
                shift: redo,
                ..Modifiers::COMMAND
            },
        }
    }

    #[test]
    fn typing_marks_changes_and_save_clears_them() {
        let mut state = EditorState::new("s", "a\nb\n", "a\nb\n");
        assert!(state.changes().is_clean());
        state.source.push_str("c\nd\n");
        let changes = state.changes();
        assert_eq!(changes.hunks().len(), 1);
        assert_eq!(changes.hunks()[0].kind, ChangeKind::Added);
        assert_eq!((changes.lines_added(), changes.lines_removed()), (2, 0));
        assert!(
            Arc::ptr_eq(&changes, &state.changes()),
            "computed once per change"
        );

        // A draft write never moves the baseline.
        state.last_edit = Some(Instant::now() - DRAFT_DELAY * 2);
        assert!(state.take_draft_due(Instant::now()));
        assert_eq!(state.changes().lines_added(), 2);

        state.mark_saved();
        assert!(state.changes().is_clean());
        assert!(!state.is_dirty());
        state.source.push_str("e\n");
        assert_eq!(
            state.changes().lines_added(),
            1,
            "counts restart from the save"
        );
    }

    #[test]
    fn a_save_that_did_not_happen_keeps_every_marker() {
        // `save_open` only calls `mark_saved` once the store accepted the text.
        let mut state = EditorState::new("s", "a\nX\nc\n", "a\nb\nc\n");
        let before = state.changes();
        assert_eq!(before.hunks().len(), 1);
        assert_eq!(state.changes().hunks(), before.hunks());
        assert_eq!(state.saved_source(), "a\nb\nc\n");
    }

    #[test]
    fn markers_are_against_the_saved_text_even_for_invalid_pine() {
        // A reopened draft: `source` is the draft, the baseline is the saved script.
        let mut state =
            EditorState::new("s", "plot(close\nplot(open)\n", "plot(close)\nplot(open)\n");
        crate::script_manager::compile_into(&mut state);
        assert_eq!(state.status, CompileStatus::Error);
        let changes = state.changes();
        assert_eq!(changes.hunks().len(), 1);
        assert_eq!(changes.hunks()[0].kind, ChangeKind::Modified);
        assert_eq!(changes.hunks()[0].new_lines, 0..1);
    }

    #[test]
    fn an_external_replace_updates_the_baseline_only_when_saved() {
        let mut state = EditorState::new("s", "a\n", "a\n");
        state.replace_source("a\nb\n", false);
        assert_eq!(state.changes().lines_added(), 1);
        state.replace_source("a\nb\n", true);
        assert!(state.changes().is_clean());
    }

    #[test]
    fn hunk_edits_reject_stale_hunks_without_touching_the_text() {
        let mut state = EditorState::new("s", "a\nX\nc\n", "a\nb\nc\n");
        let hunk = state.changes().hunks()[0].clone();
        state.source = "a\nXY\nc\n".into();
        let ctx = egui::Context::default();
        state.preview = Some(hunk.clone());
        assert!(!apply_hunk_edit(&ctx, &mut state, &hunk));
        assert_eq!(state.source, "a\nXY\nc\n");
        assert!(state.preview.is_none(), "the stale comparison closes");
    }

    #[test]
    fn an_open_comparison_closes_when_its_change_is_edited_away() {
        let mut state = EditorState::new("s", "a\nX\nc\n", "a\nb\nc\n");
        state.preview = Some(state.changes().hunks()[0].clone());
        let ctx = egui::Context::default();
        run_frame(&ctx, &mut state, 0.0, vec![]);
        run_frame(&ctx, &mut state, 0.1, vec![]);
        assert!(state.preview.is_some(), "still the same change");
        state.source = "a\nb\nc\n".into();
        run_frame(&ctx, &mut state, 0.2, vec![]);
        assert!(state.preview.is_none());
    }

    #[test]
    fn a_revert_is_one_undo_step_and_redo_replays_it() {
        let saved = "plot(close)\n";
        let typed = "fast = ta.ema(close, 9)\nslow = ta.ema(close, 21)\nfast2 = 1\nplot(close)\n";
        let mut state = EditorState::new("s", saved, saved);
        let ctx = egui::Context::default();
        ctx.memory_mut(|memory| memory.request_focus(state.id));
        run_frame(&ctx, &mut state, 0.0, vec![]);
        // Typed just now: egui has not yet closed that run of edits.
        state.source = typed.to_owned();
        run_frame(&ctx, &mut state, 0.1, vec![]);
        run_frame(&ctx, &mut state, 0.2, vec![]);
        let hunk = state.changes().hunks()[0].clone();
        assert_eq!(hunk.new_lines, 0..3);
        assert!(apply_hunk_edit(&ctx, &mut state, &hunk));
        assert_eq!(state.source, saved);
        assert!(state.changes().is_clean());
        run_frame(&ctx, &mut state, 0.3, vec![]);
        run_frame(&ctx, &mut state, 0.4, vec![]);
        assert_eq!(state.source, saved);

        run_frame(&ctx, &mut state, 0.5, vec![undo_key(false)]);
        assert_eq!(state.source, typed, "one undo brings every line back");
        assert_eq!(state.changes().lines_added(), 3, "and the gutter with it");
        run_frame(&ctx, &mut state, 0.6, vec![undo_key(true)]);
        assert_eq!(state.source, saved, "redo reverts again");
        assert!(state.changes().is_clean());
    }

    #[test]
    fn restoring_deleted_lines_puts_the_caret_after_them() {
        let mut state = EditorState::new("s", "a = 1\nc = 3\n", "a = 1\nb = 2\nc = 3\n");
        let ctx = egui::Context::default();
        let hunk = state.changes().hunks()[0].clone();
        assert!(apply_hunk_edit(&ctx, &mut state, &hunk));
        assert_eq!(state.source, "a = 1\nb = 2\nc = 3\n");
        let caret = TextEditState::load(&ctx, state.id)
            .and_then(|edit| edit.cursor.char_range())
            .map(|range| range.primary.index.0);
        assert_eq!(caret, Some("a = 1\nb = 2\n".chars().count()));
    }

    #[test]
    fn the_gutter_draws_every_kind_of_change() {
        let saved: String = (0..60).map(|i| format!("v{i} = {i}\n")).collect();
        let current = saved
            .replace("v5 = 5\n", "")
            .replace("v20 = 20\n", "v20 = 21\n")
            .replace("v40 = 40\n", "v40 = 40\nnew = 1\nnew2 = 2\n");
        {
            let mut state = EditorState::new("s", &current, &saved);
            let ctx = egui::Context::default();
            assert_eq!(state.changes().hunks().len(), 3);
            state.preview = Some(state.changes().hunks()[1].clone());
            for step in 0..4 {
                run_frame(&ctx, &mut state, f64::from(step) * 0.1, vec![]);
            }
            assert!(state.preview.is_some());
        }
    }

    // MARK: Profile column

    const PROFILED: &str =
        "//@version=6\nindicator(\"T\")\nt = 0.0\nfor i = 0 to 9\n    t += i\nplot(t)\n";

    fn profile_marks() -> std::sync::Arc<super::super::profile_marks::ProfileMarks> {
        let report = crate::pine::profile::support::report(
            "t = 0.0\nfor i = 0 to 9\n    t += i\nplot(t)\n",
            20,
            1_000,
        );
        std::sync::Arc::new(super::super::profile_marks::ProfileMarks::build(
            &report, PROFILED, 0,
        ))
    }

    fn move_to(ctx: &egui::Context, state: &mut EditorState, x: f32, y: f32, time: f64) {
        run_frame(
            ctx,
            state,
            time,
            vec![Event::PointerMoved(egui::pos2(x, y))],
        );
        run_frame(ctx, state, time + 0.01, vec![]);
    }

    #[test]
    fn the_profile_column_takes_room_only_while_the_profiler_is_on() {
        let mut state = EditorState::new("s", PROFILED, PROFILED);
        let ctx = egui::Context::default();
        let text_left = |state: &mut EditorState, ctx: &egui::Context, time: f64| {
            let mut left = 0.0;
            let raw = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                left = show(ui, state).text_area.expect("drawn").min.x;
            });
            output.textures_delta.clear();
            left
        };
        let plain = text_left(&mut state, &ctx, 0.0);
        state.set_profile_column(Some(ProfileColumn::Empty("Run the profiler.")));
        assert_eq!(text_left(&mut state, &ctx, 0.1), plain + PROFILE_WIDTH);
        state.set_profile_column(None);
        assert_eq!(
            text_left(&mut state, &ctx, 0.2),
            plain,
            "and back when it is off"
        );
    }

    #[test]
    fn hovering_a_line_in_the_profile_column_washes_the_block_it_covers() {
        let mut state = EditorState::new("s", PROFILED, PROFILED);
        state.set_profile_column(Some(ProfileColumn::Marks(profile_marks())));
        let ctx = egui::Context::default();
        for step in 0..3 {
            run_frame(&ctx, &mut state, f64::from(step) * 0.1, vec![]);
        }
        assert_eq!(state.profile_hover, None);
        let mut found = None;
        let mut y = 4.0;
        while y < 140.0 && found.is_none() {
            move_to(&ctx, &mut state, 30.0, y, 1.0 + f64::from(y));
            found = state.profile_hover;
            y += 3.0;
        }
        assert!(found.is_some(), "the pointer found a measured line");
        // Leaving the column ends it.
        move_to(&ctx, &mut state, 400.0, 60.0, 500.0);
        assert_eq!(state.profile_hover, None);
    }

    #[test]
    fn an_empty_column_says_why_and_hovers_nothing() {
        let mut state = EditorState::new("s", PROFILED, PROFILED);
        state.set_profile_column(Some(ProfileColumn::Empty(
            "This profile is of earlier text.",
        )));
        let ctx = egui::Context::default();
        for step in 0..3 {
            run_frame(&ctx, &mut state, f64::from(step) * 0.1, vec![]);
        }
        move_to(&ctx, &mut state, 30.0, 40.0, 5.0);
        assert_eq!(state.profile_hover, None);
    }
}
