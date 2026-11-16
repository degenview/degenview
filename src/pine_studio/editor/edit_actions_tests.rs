//! The Edit menu's items, run through the editor's frames: they do what the keyboard does, on the
//! editor's own undo history, and the menu's state follows the caret the field holds.

use super::diff::ChangeKind;
use super::gutter::changes::ChangeCommand;
use super::multi_cursor::Sel;
use super::navigation_tests::{Harness, SCRIPT, cmd, key};
use super::*;
use crate::pine_studio::debug::marks::{BreakpointMark, MarkKind};

/// The script with `from` replaced, which `Harness::new` has as its saved text.
fn edited(from: &str, to: &str) -> Harness {
    let mut h = Harness::new(SCRIPT);
    h.state.source = SCRIPT.replace(from, to);
    h.frame(vec![]);
    h
}

/// Types `text` and lets the history keep it as a step of its own.
fn type_and_settle(h: &mut Harness, text: &str) {
    h.type_text(text);
    h.time += 2.0;
    h.frame(vec![]);
}

fn run(h: &mut Harness, action: EditAction) {
    h.state.request_edit(action);
    h.frame(vec![]);
}

#[test]
fn undo_and_redo_from_the_menu_step_the_fields_own_history() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    let before = h.state.menu_state(&h.ctx);
    assert!(!before.can_undo && !before.can_redo);
    type_and_settle(&mut h, "x");
    assert!(h.state.source.starts_with('x'));
    let typed = h.state.menu_state(&h.ctx);
    assert!(typed.can_undo && !typed.can_redo);

    run(&mut h, EditAction::Undo);
    assert_eq!(h.state.source, SCRIPT);
    let undone = h.state.menu_state(&h.ctx);
    assert!(!undone.can_undo && undone.can_redo);

    run(&mut h, EditAction::Redo);
    assert!(h.state.source.starts_with('x'));
}

#[test]
fn the_menus_undo_is_the_keyboards() {
    let mut menu = Harness::new(SCRIPT);
    let mut keyboard = Harness::new(SCRIPT);
    for h in [&mut menu, &mut keyboard] {
        h.place(0, 0);
        type_and_settle(h, "x");
        h.state.source.push('y');
        h.time += 2.0;
        h.frame(vec![]);
    }
    run(&mut menu, EditAction::Undo);
    keyboard.frame(vec![key(Key::Z, cmd())]);
    assert_eq!(menu.state.source, keyboard.state.source);
    assert_eq!(menu.primary(), keyboard.primary());
    run(&mut menu, EditAction::Undo);
    keyboard.frame(vec![key(Key::Z, cmd())]);
    assert_eq!(menu.state.source, keyboard.state.source);
}

#[test]
fn undo_with_nothing_to_undo_leaves_the_text() {
    let mut h = Harness::new(SCRIPT);
    run(&mut h, EditAction::Undo);
    assert_eq!(h.state.source, SCRIPT);
}

#[test]
fn a_menu_edit_is_one_undo_step_that_undo_takes_back() {
    let mut h = Harness::new(SCRIPT);
    h.place(h.index(3, 0), h.index(3, 0));
    run(&mut h, EditAction::ToggleComment);
    assert!(h.state.source.contains("// plot("));
    run(&mut h, EditAction::Undo);
    assert_eq!(h.state.source, SCRIPT);
}

#[test]
fn the_line_commands_do_what_their_shortcuts_do() {
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let alt_shift = Modifiers { shift: true, ..alt };
    for (action, event) in [
        (EditAction::ToggleComment, key(Key::Slash, cmd())),
        (
            EditAction::DuplicateLineDown,
            key(Key::ArrowDown, alt_shift),
        ),
        (EditAction::DuplicateLineUp, key(Key::ArrowUp, alt_shift)),
        (EditAction::MoveLineUp, key(Key::ArrowUp, alt)),
        (EditAction::MoveLineDown, key(Key::ArrowDown, alt)),
    ] {
        let mut menu = Harness::new(SCRIPT);
        let mut keyboard = Harness::new(SCRIPT);
        for h in [&mut menu, &mut keyboard] {
            let at = h.index(2, 2);
            h.place(at, at);
        }
        run(&mut menu, action);
        keyboard.frame(vec![event]);
        assert_ne!(menu.state.source, SCRIPT, "{action:?} did something");
        assert_eq!(menu.state.source, keyboard.state.source, "{action:?}");
        assert_eq!(menu.primary(), keyboard.primary(), "{action:?} caret");
    }
}

#[test]
fn move_line_is_off_where_there_is_no_line_to_move_into() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    let first = h.state.menu_state(&h.ctx);
    assert!(!first.can_move_up && first.can_move_down);
    let last = h.state.source.chars().count() - 1;
    h.place(last, last);
    let bottom = h.state.menu_state(&h.ctx);
    assert!(bottom.can_move_up);
}

#[test]
fn select_all_selects_every_character() {
    let mut h = Harness::new(SCRIPT);
    run(&mut h, EditAction::SelectAll);
    assert_eq!(h.primary(), (0, SCRIPT.chars().count()));
    assert!(h.state.menu_state(&h.ctx).selection);
}

#[test]
fn find_opens_the_bar_with_the_selected_word_and_next_steps_through_matches() {
    let mut h = Harness::new(SCRIPT);
    let (from, to) = (h.index(2, 0), h.index(2, 3));
    h.place(from, to);
    assert!(!h.state.menu_state(&h.ctx).can_step_matches);
    run(&mut h, EditAction::Find);
    assert!(h.state.find.open);
    assert_eq!(h.state.find.query, "len");
    assert!(h.state.menu_state(&h.ctx).can_step_matches);

    run(&mut h, EditAction::FindNext);
    h.frame(vec![]);
    let second = (h.index(3, 19), h.index(3, 22));
    assert_eq!(h.primary(), second);
    run(&mut h, EditAction::FindPrevious);
    h.frame(vec![]);
    assert_eq!(h.primary(), (from, to));
}

#[test]
fn find_next_does_nothing_while_the_bar_is_closed() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    run(&mut h, EditAction::FindNext);
    assert_eq!(h.primary(), (0, 0));
    assert!(!h.state.find.open);
}

#[test]
fn find_usages_lists_the_calls_of_the_function_at_the_caret() {
    let source = "//@version=6\nindicator(\"T\")\nhalf(x) => x / 2\na = half(1)\nb = half(2)\n";
    let mut h = Harness::new(source);
    let at = h.index(2, 1);
    h.place(at, at);
    assert!(h.state.menu_state(&h.ctx).usages);
    run(&mut h, EditAction::FindUsages);
    assert_eq!(h.state.call_list_rows().len(), 2);
}

#[test]
fn the_menu_state_follows_the_stored_caret_not_the_keyboard() {
    let mut h = Harness::new(SCRIPT);
    let on_use = h.index(3, 20);
    h.place(on_use, on_use);
    let in_view = |h: &mut Harness| {
        h.ctx.memory_mut(|m| m.surrender_focus(h.state.id));
        h.frame(vec![]);
        h.frame(vec![]);
        h.state.menu_state(&h.ctx)
    };
    let menu = in_view(&mut h);
    assert!(menu.definition && menu.rename, "a name with a declaration");
    assert!(!menu.selection);
    assert_eq!(menu.line, 4);
    assert_eq!(
        h.state.edit_menu(),
        crate::native_bridge::EditMenu::default(),
        "the macOS menu still wants the keyboard"
    );
}

#[test]
fn cut_and_copy_are_on_when_an_extra_caret_has_a_selection_though_the_primary_has_none() {
    let mut h = Harness::new("a b\nc d\n");
    h.place(1, 1);
    h.state.set_extra_carets(&[5]);
    assert!(
        !h.state.menu_state(&h.ctx).selection,
        "bare carets have nothing to cut"
    );
    h.state.carets.extra = vec![Sel { anchor: 4, head: 7 }];
    assert!(h.state.menu_state(&h.ctx).selection);
}

#[test]
fn the_undo_history_is_read_when_the_menu_opens_and_again_after_a_gap() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    type_and_settle(&mut h, "x");
    let open = h.state.menu_state(&h.ctx);
    assert!(open.can_undo);
    // Still up a frame later: the same answer without asking the history again.
    h.frame(vec![]);
    assert_eq!(h.state.menu_state(&h.ctx).can_undo, open.can_undo);
    // Undone with the menu shut, then opened again with the same text and cursor as before
    // would have been (undo and redo): the history is asked, not remembered.
    for _ in 0..4 {
        h.frame(vec![]);
    }
    h.frame(vec![key(Key::Z, cmd())]);
    for _ in 0..4 {
        h.frame(vec![]);
    }
    let after = h.state.menu_state(&h.ctx);
    assert!(!after.can_undo && after.can_redo);
}

#[test]
fn the_menu_state_has_the_breakpoint_and_the_change_of_the_carets_line() {
    let mut h = edited("len = 14", "len = 20");
    h.state.set_debug_marks(DebugMarks {
        breakpoints: vec![BreakpointMark {
            line: 3,
            kind: MarkKind::Line,
            enabled: true,
            unverified: false,
        }],
        ..DebugMarks::default()
    });
    let on_change = h.index(2, 0);
    h.place(on_change, on_change);
    let menu = h.state.menu_state(&h.ctx);
    assert_eq!(menu.line, 3);
    assert!(menu.breakpoint.is_some());
    assert_eq!(menu.breakpoints, 1);
    assert_eq!(menu.change, Some(ChangeKind::Modified));

    h.place(0, 0);
    let elsewhere = h.state.menu_state(&h.ctx);
    assert!(elsewhere.breakpoint.is_none() && elsewhere.change.is_none());
}

#[test]
fn reverting_the_change_at_the_caret_from_the_menu_is_one_undoable_step() {
    let mut h = edited("len = 14", "len = 20");
    let at = h.index(2, 0);
    h.place(at, at);
    run(&mut h, EditAction::Change(ChangeCommand::Revert));
    assert_eq!(h.state.source, SCRIPT);
    run(&mut h, EditAction::Undo);
    assert!(h.state.source.contains("len = 20"));
}

#[test]
fn showing_the_change_at_the_caret_opens_its_comparison() {
    let mut h = edited("len = 14", "len = 20");
    let at = h.index(2, 0);
    h.place(at, at);
    run(&mut h, EditAction::Change(ChangeCommand::Show));
    assert!(h.state.preview.is_some());
}

#[test]
fn a_menu_action_hands_the_keyboard_back_to_the_editor() {
    let mut h = Harness::new(SCRIPT);
    h.ctx.memory_mut(|m| m.surrender_focus(h.state.id));
    h.frame(vec![]);
    run(&mut h, EditAction::SelectAll);
    h.frame(vec![]);
    assert!(h.ctx.memory(|m| m.has_focus(h.state.id)));
    // A press on the strip would have taken it; typing then replaces the selection.
    h.type_text("z");
    assert_eq!(h.state.source, "z");
}
