//! What the Edit menu over the editor may do. Worked out from the caret and selection the field
//! holds, not from whether the editor has the keyboard: opening a menu takes the keyboard away.

use super::assists::line_edit;
use super::diff::ChangeKind;
use super::edit_actions::{EditAction, caret_line, history_steps, selection, stored_cursor};
use super::edit_ops::Context;
use super::navigation::caret_index;
use super::{EditorState, text_hash};
use crate::pine_studio::debug::marks::BreakpointMark;
use eframe::egui::{self, text::CCursorRange};
use std::ops::Range;

/// The state of the Edit menu's items.
#[derive(Clone, Debug, Default)]
pub(in crate::pine_studio) struct EditorMenuState {
    pub can_undo: bool,
    pub can_redo: bool,
    /// Some text is selected.
    pub selection: bool,
    pub has_text: bool,
    /// The find bar is open with something to look for.
    pub can_step_matches: bool,
    pub can_toggle_comment: bool,
    pub can_move_up: bool,
    pub can_move_down: bool,
    /// The name at the caret has a declaration to go to.
    pub definition: bool,
    /// The name at the caret is a function with calls.
    pub usages: bool,
    /// The name at the caret can be renamed.
    pub rename: bool,
    /// The one-based line the caret is on.
    pub line: usize,
    pub breakpoint: Option<BreakpointMark>,
    /// How many breakpoints the script has.
    pub breakpoints: usize,
    /// What kind of change the caret's line is in, if it is in one.
    pub change: Option<ChangeKind>,
}

/// What costs a scan of the text to know, kept for the text and selection it was worked out for.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Findings {
    can_toggle_comment: bool,
    can_move_up: bool,
    can_move_down: bool,
    definition: bool,
    usages: bool,
    rename: bool,
}

/// The text hash, selection and caret the findings were made for.
type FindingsKey = (u64, Range<usize>, usize);

/// What the menu keeps between frames.
#[derive(Default)]
pub(super) struct Cache {
    /// The scans, for the text, selection and caret they were made for.
    findings: Option<(FindingsKey, Findings)>,
    /// What the undo history allows, for the text and cursor it was asked about, and the pass it
    /// was asked in: the history can change while the menu is shut, so it is asked again each time
    /// the menu opens, not each frame it stays up.
    history: Option<(u64, CCursorRange, u64, (bool, bool))>,
}

impl EditorState {
    /// The state of the Edit menu's items now.
    pub(in crate::pine_studio) fn menu_state(&mut self, ctx: &egui::Context) -> EditorMenuState {
        let selection = selection(ctx, self);
        let caret = caret_index(ctx, self.id).unwrap_or(0);
        let (can_undo, can_redo) = self.history(ctx);
        let found = self.findings(&selection, caret);
        let line = caret_line(&self.source, caret);
        EditorMenuState {
            can_undo,
            can_redo,
            // Every caret copies and cuts what it has selected, not only the one egui holds.
            selection: !selection.is_empty()
                || self.carets.extra.iter().any(|sel| sel.anchor != sel.head),
            has_text: !self.source.is_empty(),
            can_step_matches: self.find.open && !self.find.query.is_empty(),
            can_toggle_comment: found.can_toggle_comment,
            can_move_up: found.can_move_up,
            can_move_down: found.can_move_down,
            definition: found.definition,
            usages: found.usages,
            rename: found.rename,
            line,
            breakpoint: self.debug.breakpoint_at(line).copied(),
            breakpoints: self.debug.breakpoints.len(),
            change: self.changes().hunk_at(line - 1).map(|hunk| hunk.kind),
        }
    }

    /// Whether there is a step to undo and one to redo. Copying the undo history is the costly part
    /// of the menu, so it is done when the menu opens and when the text or cursor moves while it
    /// is up.
    fn history(&mut self, ctx: &egui::Context) -> (bool, bool) {
        let (text, cursor, pass) = (
            text_hash(&self.source),
            stored_cursor(ctx, self),
            ctx.cumulative_pass_nr(),
        );
        if let Some((cached_text, cached_cursor, cached_pass, steps)) = &mut self.nav.strip.history
            && *cached_text == text
            && *cached_cursor == cursor
            && pass <= *cached_pass + 1
        {
            *cached_pass = pass;
            return *steps;
        }
        let steps = history_steps(ctx, self);
        self.nav.strip.history = Some((text, cursor, pass, steps));
        steps
    }

    /// Worked out once per text and selection, not per frame.
    fn findings(&mut self, selection: &Range<usize>, caret: usize) -> Findings {
        let key = (text_hash(&self.source), selection.clone(), caret);
        if let Some((cached, findings)) = &self.nav.strip.findings
            && *cached == key
        {
            return *findings;
        }
        let table = self.symbols();
        let context = Context::new(&self.source, selection.clone());
        let has_edit = |action| line_edit(&context, action).is_some();
        let findings = Findings {
            can_toggle_comment: has_edit(EditAction::ToggleComment),
            can_move_up: has_edit(EditAction::MoveLineUp),
            can_move_down: has_edit(EditAction::MoveLineDown),
            definition: table.definition_of(caret).is_some(),
            usages: table
                .usages_of(caret)
                .is_some_and(|calls| !calls.is_empty()),
            rename: table.name_at(caret).is_some(),
        };
        self.nav.strip.findings = Some((key, findings));
        findings
    }
}
