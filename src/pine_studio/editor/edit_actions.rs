//! What the Edit menus ask of the editor: the macOS menu bar's and the strip over the editor's.
//! An action is requested at any time ([`EditorState::request_edit`]) and run at the start of the
//! editor's next frame, when the text and the caret are the editor's to change.

use super::assists::line_edit;
use super::edit_ops::Context;
use super::gutter::changes::{ChangeCommand, run_change};
use super::navigation::{caret_index, clipboard};
use super::{EditorState, Undoer, apply_undoable, open_find, step_match};
use eframe::egui::{
    self,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};
use std::ops::Range;

/// An item of an Edit menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    FindNext,
    FindPrevious,
    ToggleComment,
    DuplicateLineUp,
    DuplicateLineDown,
    MoveLineUp,
    MoveLineDown,
    GoToDefinition,
    FindUsages,
    Rename,
    /// An item of the menu of the change the caret's line is in.
    Change(ChangeCommand),
}

/// The selection as an ordered range of characters; a bare caret is an empty range.
pub(super) fn selection(ctx: &egui::Context, state: &EditorState) -> Range<usize> {
    TextEditState::load(ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .map(|range| {
            let (a, b) = (range.primary.index.0, range.secondary.index.0);
            a.min(b)..a.max(b)
        })
        .unwrap_or(0..0)
}

/// The cursor the field holds, or the start of the text when it holds none yet.
pub(super) fn stored_cursor(ctx: &egui::Context, state: &EditorState) -> CCursorRange {
    TextEditState::load(ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .unwrap_or_else(|| CCursorRange::one(CCursor::new(0)))
}

/// Runs `action`. Returns whether the text changed. Except for the actions that hand the keyboard
/// to a field of their own (Find, Rename), the editor takes it back: the click that chose the item
/// took it away, and typing should go on where it was.
pub(super) fn run(ctx: &egui::Context, state: &mut EditorState, action: EditAction) -> bool {
    let caret = caret_index(ctx, state.id);
    let mut changed = false;
    let mut refocus = true;
    match action {
        EditAction::Undo => changed = step_history(ctx, state, false),
        EditAction::Redo => changed = step_history(ctx, state, true),
        EditAction::Cut | EditAction::Copy | EditAction::Paste => clipboard(ctx, state, action),
        EditAction::SelectAll => select_all(ctx, state),
        EditAction::Find => {
            open_find(ctx, state);
            refocus = false;
        }
        EditAction::FindNext | EditAction::FindPrevious => {
            if state.find.open {
                step_match(state, action == EditAction::FindNext);
            }
        }
        EditAction::ToggleComment
        | EditAction::DuplicateLineUp
        | EditAction::DuplicateLineDown
        | EditAction::MoveLineUp
        | EditAction::MoveLineDown => {
            let context = Context::new(&state.source, selection(ctx, state));
            if let Some(edit) = line_edit(&context, action) {
                apply_undoable(ctx, state, &edit);
                changed = true;
            }
        }
        EditAction::GoToDefinition => {
            if let Some(caret) = caret {
                state.go_to_definition(ctx, caret);
            }
        }
        EditAction::FindUsages => {
            if let Some(caret) = caret {
                let at = state
                    .caret_rect
                    .map_or_else(|| ctx.content_rect().center(), |rect| rect.left_bottom());
                state.find_usages(ctx, caret, at);
            }
        }
        EditAction::Rename => {
            if let Some(caret) = caret {
                state.begin_rename(caret);
            }
            refocus = false;
        }
        EditAction::Change(command) => {
            if let Some(hunk) = hunk_at_caret(ctx, state) {
                changed = run_change(ctx, state, hunk, command);
            }
        }
    }
    if refocus {
        ctx.memory_mut(|memory| memory.request_focus(state.id));
    }
    ctx.request_repaint();
    changed
}

/// The change the caret's line is in.
pub(super) fn hunk_at_caret(
    ctx: &egui::Context,
    state: &mut EditorState,
) -> Option<super::diff::DiffHunk> {
    let line = caret_line(&state.source, caret_index(ctx, state.id)?);
    state.changes().hunk_at(line - 1).cloned()
}

/// The one-based line the character at `index` is on.
pub(super) fn caret_line(source: &str, index: usize) -> usize {
    1 + source.chars().take(index).filter(|c| *c == '\n').count()
}

fn select_all(ctx: &egui::Context, state: &EditorState) {
    let mut edit_state = TextEditState::load(ctx, state.id).unwrap_or_default();
    edit_state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(0),
        CCursor::new(state.source.chars().count()),
    )));
    edit_state.store(ctx, state.id);
}

/// The field's own undo history, stepped as its ⌘Z / ⇧⌘Z would: the text field feeds the history
/// the text it sees first, then asks it for the step. There is never a second history.
fn step_history(ctx: &egui::Context, state: &mut EditorState, redo: bool) -> bool {
    let mut edit_state = TextEditState::load(ctx, state.id).unwrap_or_default();
    let current = (stored_cursor(ctx, state), state.source.clone());
    let mut undoer = edit_state.undoer();
    undoer.feed_state(ctx.input(|input| input.time), &current);
    let stepped = if redo {
        undoer.redo(&current)
    } else {
        undoer.undo(&current)
    }
    .cloned();
    edit_state.set_undoer(undoer);
    let changed = stepped.is_some();
    if let Some((cursor, text)) = stepped {
        state.source = text;
        edit_state.cursor.set_char_range(Some(cursor));
        state.select_to = None;
    }
    edit_state.store(ctx, state.id);
    changed
}

/// Whether the field's history has a step that changes the text to undo, and one to redo, from the
/// text as it stands. A step that only moves the caret does not count.
pub(super) fn history_steps(ctx: &egui::Context, state: &EditorState) -> (bool, bool) {
    let Some(edit_state) = TextEditState::load(ctx, state.id) else {
        return (false, false);
    };
    let current = (stored_cursor(ctx, state), state.source.clone());
    let mut undoer: Undoer = edit_state.undoer();
    let can_redo = undoer.has_redo(&current);
    undoer.feed_state(ctx.input(|input| input.time), &current);
    let can_undo = undoer
        .undo(&current)
        .is_some_and(|(_, text)| *text != current.1);
    (can_undo, can_redo)
}
