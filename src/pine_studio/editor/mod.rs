//! The code editor pane: a line-numbered monospaced text field with token colouring, diagnostic
//! underlines, the current-line band, indent guides and the Swift editing assists, above the
//! status/Save bar. Ported from `ScriptEditorView` and `LineNumberedTextEditorView`.
//!
//! A pure view: it never compiles. The host feeds `diagnostics` and `status` back after a
//! change (`EditorOutput::changed`).

pub(in crate::pine_studio) mod call_sites;
pub mod color_sites;
pub(in crate::pine_studio) mod completion;
pub mod diff;
pub(in crate::pine_studio) mod diff_view;
pub mod edit_ops;
pub(in crate::pine_studio) mod find;
pub mod multi_cursor;
pub(in crate::pine_studio) mod signature;
pub mod symbols;
pub mod syntax;

use super::debug::marks::{DebugMarks, GutterEvent};
use super::profiler::marks::ProfileColumn;
use crate::model::pine::script::CompileStatus;
use completion::Completion;
use diff::{DiffHunk, DiffSnapshot};
use edit_ops::Edit;
use find::{FindEvent, FindState};

use crate::model::editor_theme::SurfaceColor;
use crate::pine::syntax::token::Diagnostic;
use crate::ui::editor_appearance::{self, Appearance, EditorFont};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Event, FontId, Id, Key, Margin, Modifiers, Rect, RichText, Sense, Shape, Stroke,
    Ui, Vec2,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use syntax::Span;

mod assists;
mod carets;
mod completion_view;
mod gutter;
mod indent_guides;
mod navigation;
mod signature_view;
mod state;
pub(in crate::pine_studio) mod status;
mod swatches;
mod text_view;

use assists::apply_assists;
use completion_view::{completion_keys, update_completion};
use gutter::breakpoints::breakpoint_gutter;
use gutter::changes::change_gutter;
use gutter::profile::{PROFILE_WIDTH, profile_gutter};
use gutter::{gutter_width, paint_gutter};
pub use navigation::EditAction;
use signature_view::draw_signature;
use status::status_bar;
use text_view::text_view;

#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod navigation_tests;
#[cfg(test)]
mod swatch_tests;

const TEXT_INSET: f32 = 8.0;

const FONT_SIZE: f32 = 13.0;

/// A draft is written this long after the last keystroke.
pub const DRAFT_DELAY: Duration = Duration::from_secs(1);
/// The text is compiled this long after the last keystroke, so a name still being typed is not
/// reported as undefined. Shorter than the preview's own apply delay, which waits for the compile.
pub const COMPILE_DELAY: Duration = Duration::from_millis(500);
const FLASH: Duration = Duration::from_millis(700);

/// A brief highlight of where a jump landed: the character it landed on, or the whole line.
#[derive(Clone, Copy)]
struct Flash {
    /// The character offset the jump landed on.
    index: usize,
    at: Instant,
    whole_line: bool,
}

impl Flash {
    fn at(index: usize) -> Self {
        Self {
            index,
            at: Instant::now(),
            whole_line: false,
        }
    }

    fn line(index: usize) -> Self {
        Self {
            whole_line: true,
            ..Self::at(index)
        }
    }

    /// 1 when it starts, 0 when it is over; the line holds its strength before it fades.
    fn strength(&self) -> f32 {
        let age = (self.at.elapsed().as_secs_f32() / FLASH.as_secs_f32()).clamp(0.0, 1.0);
        if self.whole_line {
            1.0 - age * age
        } else {
            1.0 - age
        }
    }
}

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
    /// Failures of text that compiled but failed when it ran, drawn and counted like `diagnostics`
    /// (the workspace sets them each frame; they are never part of the compile result).
    runtime_problems: Vec<Diagnostic>,
    /// The editor is a sample (Settings' theme preview): it has no script to save.
    sample: bool,
    pub status: CompileStatus,
    highlighted: Option<(u64, Arc<Vec<Span>>)>,
    /// A UTF-16 offset to move the caret to on the next frame (a Problems row was clicked).
    jump_to: Option<usize>,
    flash: Option<Flash>,
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
    /// The caret is scrolled out of the text area. The suggestion list and the signature panel are
    /// areas of their own, which the editor's clip does not reach, so they hide instead of drawing
    /// over whatever sits next to the editor.
    caret_hidden: bool,
    /// The part of the text area on screen, which the suggestion list and the signature panel stay
    /// inside. Set each frame by `show`.
    text_visible: Option<Rect>,
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
    /// A short note for the status strip ("Reloaded: …"), and when it was made.
    notice: Option<(String, Instant)>,
    /// Colour swatches in front of constant colours, and the picker they open.
    swatches: swatches::SwatchState,
}

/// The open suggestion list and its highlighted row.
struct CompletionView {
    completion: Completion,
    selected: usize,
    /// An arrow key moved the highlight: scroll it into view when the list is next drawn.
    scroll_to_selected: bool,
}

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

pub fn show(ui: &mut Ui, state: &mut EditorState) -> EditorOutput {
    let mut output = EditorOutput::default();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let bar = super::drawer::bottom_bar::rect(full);
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
    let font = editor_appearance::editor_font();
    let gutter_width = gutter_width(ui, &font, state.source.split('\n').count());
    let gutter = Rect::from_min_max(
        egui::pos2(profile_column.max.x, code.min.y),
        egui::pos2(profile_column.max.x + gutter_width, code.max.y),
    );
    let text_area = Rect::from_min_max(egui::pos2(gutter.max.x, code.min.y), code.max);

    let look = editor_appearance::current();
    ui.painter()
        .rect_filled(area, 0.0, look.surface(SurfaceColor::Background));
    ui.painter()
        .rect_filled(gutter, 0.0, look.surface(SurfaceColor::GutterBackground));
    let separator = look.surface(SurfaceColor::GutterSeparator);
    if separator.a() > 0 {
        let x = gutter.max.x - 0.5;
        ui.painter().line_segment(
            [egui::pos2(x, gutter.min.y), egui::pos2(x, gutter.max.y)],
            Stroke::new(1.0, separator),
        );
    }
    if state.find.open {
        let bar_rect =
            Rect::from_min_max(area.min, egui::pos2(area.max.x, area.min.y + find_height));
        run_find_bar(ui, bar_rect, state);
    }

    swatches::keys(ui.ctx(), state);
    let mut edited = completion_keys(ui.ctx(), state);
    navigation::keys(ui.ctx(), state);
    navigation::call_list_keys(ui.ctx(), state);
    navigation::edit_menu(ui.ctx(), state);
    edited |= carets::apply(ui.ctx(), state, text_area);
    edited |= apply_assists(ui.ctx(), state);
    let rows = text_view(ui, state, text_area, &look, &font, &mut edited);
    let visible = text_area.intersect(ui.clip_rect());
    state.text_visible = Some(visible);
    state.caret_hidden = state
        .caret_rect
        .is_some_and(|rect| !visible.intersects(rect));
    // A rename changes the text but is not typing: no suggestions follow it.
    let renamed = navigation::rename_box(ui.ctx(), state);
    navigation::call_list(ui, state, &look, &font);
    edited |= renamed;
    edited |= update_completion(ui, state, &look, &font, edited && !renamed);
    edited |= swatches::picker(ui, state, &look, &font);
    if state.swatches.open.is_none() {
        draw_signature(ui, state, &look, &font);
    }
    paint_gutter(ui, gutter, &rows, &state.debug, &look, &font);
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
    /// The zero-based line the caret is on, while the caret is a point.
    caret_line: Option<usize>,
}

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// Replaces the whole text with a script the assistant wrote, as one undo step: Cmd-Z brings back
/// exactly what was here. The caret goes to the first line that differs, so the change is where the
/// editor looks.
pub(super) fn apply_script(ctx: &egui::Context, state: &mut EditorState, script: &str) {
    state.preview = None;
    let first = state
        .source
        .chars()
        .zip(script.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let edit = Edit {
        range: 0..state.source.chars().count(),
        replacement: script.to_owned(),
        selection: first..first,
    };
    apply_undoable(ctx, state, &edit);
}

/// Applies `edit` to the text as one undo step and selects what it asks for.
fn apply_undoable(ctx: &egui::Context, state: &mut EditorState, edit: &Edit) {
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
}

type Undoer = egui::util::undoer::Undoer<(CCursorRange, String)>;

/// One edit that is made over many frames and stays one undo step: a colour picker drag. egui's
/// undoer would cut a new step whenever the text rests for a second; holding the undoer as it was
/// at the start keeps the whole gesture in the step that `begin` opened.
pub(super) struct UndoGesture {
    /// The history with the text as it was when the gesture began added to it.
    pristine: Undoer,
    /// The history before that, for cancelling.
    rollback: Undoer,
}

impl UndoGesture {
    /// Marks the text as it stands now as an undo point and returns the cursor it had.
    pub(super) fn begin(ctx: &egui::Context, id: Id, source: &str) -> (Self, CCursorRange) {
        let mut edit_state = TextEditState::load(ctx, id).unwrap_or_default();
        let cursor = edit_state
            .cursor
            .char_range()
            .unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
        let rollback = edit_state.undoer();
        let mut pristine = rollback.clone();
        pristine.add_undo(&(cursor, source.to_owned()));
        edit_state.set_undoer(pristine.clone());
        edit_state.store(ctx, id);
        (Self { pristine, rollback }, cursor)
    }

    /// Puts the history back to the gesture's start. Called after every frame's change.
    pub(super) fn hold(&self, ctx: &egui::Context, id: Id) {
        let mut edit_state = TextEditState::load(ctx, id).unwrap_or_default();
        edit_state.set_undoer(self.pristine.clone());
        edit_state.store(ctx, id);
    }

    /// Takes the gesture back: the history is what it was, and the cursor too.
    pub(super) fn cancel(self, ctx: &egui::Context, id: Id, cursor: CCursorRange) {
        let mut edit_state = TextEditState::load(ctx, id).unwrap_or_default();
        edit_state.set_undoer(self.rollback);
        edit_state.cursor.set_char_range(Some(cursor));
        edit_state.store(ctx, id);
    }
}

/// `cursor` after the characters in `range` were replaced by `length` characters: whatever is
/// after the range moves with the text, and whatever was inside goes to the end of the new text.
pub(super) fn with_range_shifted(
    cursor: CCursorRange,
    range: &std::ops::Range<usize>,
    length: usize,
) -> CCursorRange {
    let shift = |index: usize| {
        if index <= range.start {
            index
        } else if index >= range.end {
            index - range.len() + length
        } else {
            range.start + length
        }
    };
    CCursorRange::two(
        CCursor::new(shift(cursor.secondary.index.0)),
        CCursor::new(shift(cursor.primary.index.0)),
    )
}

/// Replaces the text by `source` (an edit made outside the field) and puts the cursor at `cursor`.
/// The undo history is the caller's business.
pub(super) fn apply_site_text(
    ctx: &egui::Context,
    state: &mut EditorState,
    source: String,
    cursor: CCursorRange,
) {
    state.source = source;
    let mut edit_state = TextEditState::load(ctx, state.id).unwrap_or_default();
    edit_state.cursor.set_char_range(Some(cursor));
    edit_state.store(ctx, state.id);
    state.select_to = None;
    ctx.request_repaint();
}

#[cfg(test)]
mod tests;
