//! What the editor keeps between frames: the text against the saved text, the compile and draft
//! timers, notices, the marks a debugger or profiler sets, and where the caret and view are.

use super::diff::DiffSnapshot;
use super::find::FindState;
use super::syntax::Span;
use super::{
    COMPILE_DELAY, DRAFT_DELAY, EditorState, Flash, carets, diff, navigation, swatches, syntax,
    text_hash,
};
use crate::model::pine::script::CompileStatus;
use crate::pine::syntax::token::Diagnostic;
use crate::pine_studio::debug::marks::DebugMarks;
use crate::pine_studio::profiler::marks::ProfileColumn;
use eframe::egui::{
    self, Id,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};
use std::sync::Arc;
use std::time::Instant;

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
            runtime_problems: Vec::new(),
            sample: false,
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
            caret_hidden: false,
            text_visible: None,
            caret_seen: None,
            select_to: None,
            debug: DebugMarks::default(),
            menu_line: None,
            profile: None,
            profile_hover: None,
            carets: carets::CaretState::default(),
            nav: navigation::NavState::default(),
            notice: None,
            swatches: swatches::SwatchState::default(),
            #[cfg(test)]
            scroll_y: 0.0,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.source != self.saved_source
    }

    /// Says something in the status strip for a few seconds.
    pub fn set_notice(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
    }

    /// The status strip's note while it is still fresh.
    #[cfg(test)]
    pub fn notice(&self) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|(_, at)| at.elapsed() < crate::pine_studio::library::external::NOTICE_FOR)
            .map(|(text, _)| text.as_str())
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
        self.swatches.open = None;
    }

    /// The file now holds `text` (someone else wrote it) and the person chose to keep what they
    /// typed over it: the typing stays, and `text` becomes what the editor measures changes against
    /// and what a save is checked against, so a save overwrites it on purpose.
    pub fn rebase_saved(&mut self, text: &str) {
        self.saved_source = text.to_owned();
        self.saved_key = diff::text_key(text);
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
    /// Lets go of the keyboard, so a sheet drawn over the editor does not have its keys typed here.
    pub fn surrender_focus(&self, ctx: &egui::Context) {
        ctx.memory_mut(|memory| memory.surrender_focus(self.id));
    }

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

    /// The runtime failures being shown with the compile diagnostics.
    #[cfg(test)]
    pub fn runtime_problems(&self) -> &[Diagnostic] {
        &self.runtime_problems
    }

    /// Hands the editor the runtime failures of the script to draw and count this frame.
    pub fn set_runtime_problems(&mut self, problems: Vec<Diagnostic>) {
        self.runtime_problems = problems;
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

    /// Scrolls to a one-based line and flashes all of it, next frame.
    pub fn show_line(&mut self, line: usize) {
        let line = line.max(1);
        let start: usize = self
            .source
            .split_inclusive('\n')
            .take(line - 1)
            .map(|text| text.chars().count())
            .sum();
        self.scroll_to_line = Some(line - 1);
        self.flash = Some(Flash::line(start));
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

    /// Marks the editor as a sample: everything of the editor, minus the Save button, since there is
    /// no script behind it.
    pub fn sampled(mut self) -> Self {
        self.sample = true;
        self
    }

    /// Puts the caret before character `index` without taking focus, for an editor shown as a
    /// sample.
    pub fn place_caret(&self, ctx: &egui::Context, index: usize) {
        let mut edit = TextEditState::load(ctx, self.id).unwrap_or_default();
        edit.cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
        edit.store(ctx, self.id);
    }

    /// Where the caret is, as a character index, if the text has been laid out.
    #[cfg(test)]
    pub fn caret_index(&self, ctx: &egui::Context) -> Option<usize> {
        TextEditState::load(ctx, self.id)
            .and_then(|edit| edit.cursor.char_range())
            .map(|range| range.primary.index.0)
    }

    /// Moves the caret to a diagnostic's start and flashes it, next frame.
    pub fn reveal(&mut self, diagnostic: &Diagnostic) {
        self.jump_to = Some(diagnostic.range.start.offset);
    }

    pub(super) fn spans(&mut self) -> Arc<Vec<Span>> {
        let key = text_hash(&self.source);
        if let Some((cached, spans)) = &self.highlighted
            && *cached == key
        {
            return Arc::clone(spans);
        }
        let table = self.symbols();
        let spans = Arc::new(syntax::classify(&self.source, Some(&table)));
        self.highlighted = Some((key, Arc::clone(&spans)));
        spans
    }
}
