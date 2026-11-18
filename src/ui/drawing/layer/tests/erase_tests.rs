//! The Eraser cursor: a click or a sweep removes what the pointer passes over, in one undo step,
//! and only what selection could pick.

use super::*;
use crate::model::CursorMode;

/// A rig with the Eraser as the cursor and one rectangle drawn, and the rectangle's id.
fn erasing() -> (Rig, u64) {
    let mut rig = Rig::new();
    let id = rig.rect_with(0.0);
    rig.cursor = CursorMode::Eraser;
    rig.settle();
    (rig, id)
}

/// A point inside the rectangle `rect_with(offset)` makes.
fn inside(offset: f32) -> Pos2 {
    pos2(250.0 + offset, 230.0)
}

fn lock(rig: &mut Rig, index: usize) {
    let mut drawing = rig.drawings[index].clone();
    drawing.set_locked(true);
    rig.drawings[index] = drawing;
}

#[test]
fn a_click_erases_every_kind_of_drawing_in_one_undo_step() {
    for tool in Tool::GROUPED {
        let mut rig = Rig::new();
        rig.place(tool);
        let id = rig.last().id();
        rig.tool = Tool::Cursor;
        rig.selected.clear();
        let at = rig.body(id);
        let steps = rig.undo_steps;
        rig.cursor = CursorMode::Eraser;
        rig.settle();
        rig.click(at);
        assert!(rig.drawings.is_empty(), "{tool:?} was not erased at {at:?}");
        assert_eq!(rig.undo_steps, steps + 1, "{tool:?}: one step");
    }
}

#[test]
fn nothing_is_erased_until_the_button_is_let_go_and_what_is_passed_over_is_marked() {
    let (mut rig, id) = erasing();
    rig.move_to(inside(0.0));
    rig.press(inside(0.0));
    rig.move_to(inside(20.0));
    assert_eq!(
        rig.drawings.len(),
        1,
        "still there while the button is down"
    );
    assert!(rig.layer.is_marked_for_erasing(id), "drawn faint");
    assert!(
        rig.layer.claims_drag(),
        "the chart does not pan under a sweep"
    );
    rig.release(inside(20.0));
    assert!(rig.drawings.is_empty());
    assert!(!rig.layer.claims_drag());
}

#[test]
fn a_sweep_removes_everything_it_crosses_as_one_step_and_the_mode_stays_on() {
    let (mut rig, _) = erasing();
    rig.cursor = CursorMode::Cross;
    rig.rect_with(300.0);
    rig.rect_with(600.0);
    rig.cursor = CursorMode::Eraser;
    rig.settle();
    assert_eq!(rig.drawings.len(), 3);
    let steps = rig.undo_steps;
    rig.drag(pos2(150.0, 230.0), pos2(950.0, 230.0));
    assert!(rig.drawings.is_empty());
    assert_eq!(rig.undo_steps, steps + 1, "one step for the whole sweep");
    // Still the Eraser: the next drawing goes the same way.
    rig.cursor = CursorMode::Cross;
    rig.rect_with(0.0);
    rig.cursor = CursorMode::Eraser;
    rig.settle();
    rig.click(inside(0.0));
    assert!(rig.drawings.is_empty());
}

#[test]
fn a_quick_drag_cannot_step_over_a_thin_line() {
    let mut rig = Rig::new();
    rig.place(Tool::HorizontalLine);
    rig.tool = Tool::Cursor;
    let at = rig.body(rig.last().id());
    rig.cursor = CursorMode::Eraser;
    rig.settle();
    // One move of 400 points across the line, with no frame in between.
    rig.move_to(pos2(at.x, at.y - 200.0));
    rig.press(pos2(at.x, at.y - 200.0));
    rig.move_to(pos2(at.x, at.y + 150.0));
    rig.release(pos2(at.x, at.y + 150.0));
    assert!(rig.drawings.is_empty());
}

#[test]
fn a_locked_drawing_is_left_alone_and_says_so() {
    let (mut rig, _) = erasing();
    lock(&mut rig, 0);
    let steps = rig.undo_steps;
    rig.move_to(inside(0.0));
    rig.settle();
    assert!(
        rig.layer.erase_blocked,
        "the pointer is told it is protected"
    );
    rig.click(inside(0.0));
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, steps);
    // A sweep that crosses a locked drawing and a free one takes only the free one.
    rig.cursor = CursorMode::Cross;
    rig.rect_with(300.0);
    rig.cursor = CursorMode::Eraser;
    rig.settle();
    rig.drag(pos2(150.0, 230.0), pos2(650.0, 230.0));
    assert_eq!(rig.drawings.len(), 1);
    assert!(rig.drawings[0].locked());
}

#[test]
fn hidden_other_timeframe_and_hide_all_drawings_are_out_of_reach() {
    let (mut rig, _) = erasing();
    let mut hidden = rig.drawings[0].clone();
    hidden.set_hidden(true);
    rig.drawings[0] = hidden;
    rig.click(inside(0.0));
    assert_eq!(rig.drawings.len(), 1, "hidden");

    let mut weekly = rig.drawings[0].clone();
    weekly.set_hidden(false);
    weekly.set_visibility(Visibility::ALL.with_row(VisibilityRow::Days, false));
    rig.drawings[0] = weekly;
    rig.click(inside(0.0));
    assert_eq!(rig.drawings.len(), 1, "another timeframe's");

    rig.drawings[0].set_visibility(Visibility::ALL);
    rig.hide_all = true;
    rig.click(inside(0.0));
    assert_eq!(rig.drawings.len(), 1, "hide all");
}

#[test]
fn a_row_this_version_cannot_read_is_never_touched() {
    let (mut rig, _) = erasing();
    rig.drawings.push(Drawing::Unreadable(serde_json::json!({
        "Shape": {"id": 99, "geometry": {"hologram": {"at": 1}}}
    })));
    rig.drag(pos2(100.0, 230.0), pos2(900.0, 230.0));
    assert_eq!(rig.drawings.len(), 1);
    assert!(matches!(rig.drawings[0], Drawing::Unreadable(_)));
}

#[test]
fn a_click_on_overlapping_drawings_takes_the_top_one_only() {
    let (mut rig, bottom) = erasing();
    rig.cursor = CursorMode::Cross;
    let top = rig.rect_with(30.0);
    rig.cursor = CursorMode::Eraser;
    rig.settle();
    rig.click(pos2(280.0, 232.0));
    let left: Vec<u64> = rig.drawings.iter().map(Drawing::id).collect();
    assert_eq!(left, vec![bottom], "the one drawn last, {top}, is gone");
}

#[test]
fn escape_cancels_a_sweep_and_the_rest_of_the_press_does_not_pan() {
    let (mut rig, _) = erasing();
    rig.move_to(inside(0.0));
    rig.press(inside(0.0));
    rig.move_to(inside(20.0));
    rig.key(Key::Escape);
    assert!(rig.escape_consumed, "the key is spent");
    assert!(
        rig.layer.claims_drag(),
        "no pan before the button is let go"
    );
    rig.release(inside(20.0));
    assert_eq!(rig.drawings.len(), 1, "nothing was erased");
    assert!(!rig.layer.claims_drag());
}

#[test]
fn empty_space_erases_nothing_and_still_holds_the_drag() {
    let (mut rig, _) = erasing();
    let steps = rig.undo_steps;
    rig.move_to(pos2(900.0, 40.0));
    rig.press(pos2(900.0, 40.0));
    rig.move_to(pos2(940.0, 60.0));
    assert!(rig.layer.claims_drag(), "a drag in the Eraser never pans");
    rig.release(pos2(940.0, 60.0));
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, steps);
}

#[test]
fn the_eraser_neither_selects_nor_opens_the_drawing_menu() {
    let (mut rig, _) = erasing();
    // The workspace clears the selection when the Eraser is picked.
    rig.selected.clear();
    rig.drawings[0].set_locked(true);
    rig.right_click(inside(0.0));
    assert!(rig.selected.is_empty());
    assert!(rig.layer.menu.is_none());
    assert!(
        rig.background_click.is_some(),
        "the chart's own menu is still the right-click's"
    );
}

#[test]
fn putting_the_eraser_down_mid_sweep_drops_it() {
    let (mut rig, _) = erasing();
    rig.move_to(inside(0.0));
    rig.press(inside(0.0));
    assert!(rig.layer.claims_drag());
    rig.cursor = CursorMode::Cross;
    rig.move_to(inside(10.0));
    assert!(!rig.layer.is_marked_for_erasing(rig.drawings[0].id()));
    rig.release(inside(10.0));
    assert_eq!(rig.drawings.len(), 1);
}

#[test]
fn a_sweep_does_not_outlive_the_card_being_unseen() {
    let (mut rig, _) = erasing();
    let steps = rig.undo_steps;
    rig.move_to(inside(0.0));
    rig.press(inside(0.0));
    assert!(rig.layer.is_marked_for_erasing(rig.drawings[0].id()));
    // Another tab shows; the button is let go there.
    let up = rig.button(inside(0.0), false, PointerButton::Primary);
    rig.skip_frames(vec![up]);
    // Back on the chart: nothing is erased, and no press is left to hold the drag.
    rig.move_to(inside(5.0));
    rig.settle();
    assert_eq!(
        rig.drawings.len(),
        1,
        "the drawing was not erased from a gesture left behind"
    );
    assert_eq!(rig.undo_steps, steps);
    assert!(!rig.layer.claims_drag());
    assert!(!rig.layer.is_marked_for_erasing(rig.drawings[0].id()));
}

#[test]
fn a_press_that_goes_on_across_the_gap_erases_nothing_and_does_not_pan() {
    let (mut rig, _) = erasing();
    rig.move_to(inside(0.0));
    rig.press(inside(0.0));
    rig.skip_frames(Vec::new());
    rig.move_to(inside(20.0));
    assert!(
        rig.layer.claims_drag(),
        "the rest of that press is not a pan"
    );
    rig.release(inside(20.0));
    assert_eq!(rig.drawings.len(), 1);
    assert!(!rig.layer.claims_drag());
}
