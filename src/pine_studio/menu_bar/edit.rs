//! The Edit menu: undo, the clipboard, find, the line commands, going to a name's declaration and
//! the breakpoint and change of the caret's line. Everything the editor's right-click menu and
//! its gutters offer is here too.

use super::entry::{Entry, Item, Submenu, grouped};
use crate::pine_studio::commands::Cmd;
use crate::pine_studio::debug::PanelAction;
use crate::pine_studio::debug::marks::GutterEvent;
use crate::pine_studio::editor::{EditAction, EditorMenuState, breakpoint_rows, change_rows};
use crate::ui::icons::Icon;
use crate::ui::shortcut;

fn edit(label: &str, icon: Icon, action: EditAction, shortcut: &shortcut::Shortcut) -> Item {
    Item::new(label, Cmd::Edit(action))
        .icon(icon)
        .shortcut(shortcut)
}

pub(super) fn entries(state: &EditorMenuState) -> Vec<Entry> {
    grouped(vec![
        history(state),
        clipboard(state),
        find(state),
        lines(state),
        names(state),
        vec![breakpoint(state).into(), change(state).into()],
    ])
}

fn history(state: &EditorMenuState) -> Vec<Entry> {
    vec![
        edit("Undo", Icon::Undo, EditAction::Undo, &shortcut::UNDO)
            .enabled(state.can_undo)
            .into(),
        edit("Redo", Icon::Redo, EditAction::Redo, &shortcut::REDO)
            .enabled(state.can_redo)
            .into(),
    ]
}

fn clipboard(state: &EditorMenuState) -> Vec<Entry> {
    vec![
        edit("Cut", Icon::Scissors, EditAction::Cut, &shortcut::CUT)
            .enabled(state.selection)
            .into(),
        edit("Copy", Icon::CopyDoc, EditAction::Copy, &shortcut::COPY)
            .enabled(state.selection)
            .into(),
        edit(
            "Paste",
            Icon::Clipboard,
            EditAction::Paste,
            &shortcut::PASTE,
        )
        .into(),
        edit(
            "Select All",
            Icon::SelectArea,
            EditAction::SelectAll,
            &shortcut::SELECT_ALL,
        )
        .enabled(state.has_text)
        .into(),
    ]
}

fn find(state: &EditorMenuState) -> Vec<Entry> {
    vec![
        edit("Find…", Icon::Magnifier, EditAction::Find, &shortcut::FIND).into(),
        edit(
            "Find Next",
            Icon::ArrowDown,
            EditAction::FindNext,
            &shortcut::FIND_NEXT,
        )
        .enabled(state.can_step_matches)
        .into(),
        edit(
            "Find Previous",
            Icon::ArrowUp,
            EditAction::FindPrevious,
            &shortcut::FIND_PREVIOUS,
        )
        .enabled(state.can_step_matches)
        .into(),
    ]
}

fn lines(state: &EditorMenuState) -> Vec<Entry> {
    vec![
        edit(
            "Toggle Comment",
            Icon::Code,
            EditAction::ToggleComment,
            &shortcut::TOGGLE_COMMENT,
        )
        .enabled(state.can_toggle_comment)
        .into(),
        edit(
            "Duplicate Line Up",
            Icon::CopyDoc,
            EditAction::DuplicateLineUp,
            &shortcut::DUPLICATE_LINE_UP,
        )
        .enabled(state.has_text)
        .into(),
        edit(
            "Duplicate Line Down",
            Icon::CopyDoc,
            EditAction::DuplicateLineDown,
            &shortcut::DUPLICATE_LINE_DOWN,
        )
        .enabled(state.has_text)
        .into(),
        edit(
            "Move Line Up",
            Icon::ChevronUp,
            EditAction::MoveLineUp,
            &shortcut::MOVE_LINE_UP,
        )
        .enabled(state.can_move_up)
        .into(),
        edit(
            "Move Line Down",
            Icon::ChevronDown,
            EditAction::MoveLineDown,
            &shortcut::MOVE_LINE_DOWN,
        )
        .enabled(state.can_move_down)
        .into(),
    ]
}

fn names(state: &EditorMenuState) -> Vec<Entry> {
    vec![
        edit(
            "Go to Definition",
            Icon::ArrowUpRight,
            EditAction::GoToDefinition,
            &shortcut::GO_TO_DEFINITION,
        )
        .enabled(state.definition)
        .into(),
        Item::new("Find Usages", Cmd::Edit(EditAction::FindUsages))
            .icon(Icon::Magnifier)
            .enabled(state.usages)
            .into(),
        edit(
            "Rename Symbol…",
            Icon::Pencil,
            EditAction::Rename,
            &shortcut::RENAME_SYMBOL,
        )
        .enabled(state.rename)
        .into(),
    ]
}

/// The breakpoint of the caret's line: the rows of the gutter's menu, and one to clear them all.
fn breakpoint(state: &EditorMenuState) -> Submenu {
    let mut entries: Vec<Entry> = breakpoint_rows(state.line, state.breakpoint.as_ref())
        .into_iter()
        .map(|row| {
            let item = Item::new(row.label, Cmd::Gutter(row.event))
                .icon(row.icon)
                .destructive(row.destructive);
            match row.event {
                GutterEvent::Toggle { .. } => item.shortcut(&shortcut::DEBUG_TOGGLE_BREAKPOINT),
                _ => item,
            }
            .into()
        })
        .collect();
    if state.breakpoints > 0 {
        entries.push(Entry::Separator);
        entries.push(
            Item::new(
                "Remove All Breakpoints",
                Cmd::DebugPanel(PanelAction::RemoveAllBreakpoints),
            )
            .icon(Icon::Trash)
            .destructive(true)
            .into(),
        );
    }
    Submenu::new("Breakpoint", entries).icon(Icon::Circle)
}

/// The change the caret's line is in: the rows of the change column's menu. Greyed when the line
/// is not in a change.
fn change(state: &EditorMenuState) -> Submenu {
    let entries = state
        .change
        .map(|kind| {
            change_rows(kind)
                .into_iter()
                .map(|row| {
                    Item::new(row.label, Cmd::Edit(EditAction::Change(row.command)))
                        .icon(row.icon)
                        .into()
                })
                .collect()
        })
        .unwrap_or_default();
    Submenu::new("Change", entries)
        .icon(Icon::Compare)
        .enabled(state.change.is_some())
}
