//! A cursor that marks the chart (Demonstration) leaves the drawings out of reach: nothing is
//! hovered, picked, moved or deleted, and the chart's own menu is still the right-click's.

use super::*;
use crate::model::CursorMode;

#[test]
fn demonstration_picks_moves_and_deletes_nothing() {
    let mut rig = Rig::new();
    let id = rig.rect_with(0.0);
    rig.selected.clear();
    rig.cursor = CursorMode::Demonstration;
    rig.settle();
    let steps = rig.undo_steps;
    let before = rig.drawings.clone();
    let at = pos2(250.0, 230.0);
    rig.click(at);
    assert!(rig.selected.is_empty(), "no selection of {id}");
    rig.drag(at, at + vec2(80.0, 30.0));
    assert_eq!(rig.drawings, before, "nothing moved");
    rig.selected = vec![id];
    rig.key(Key::Delete);
    assert_eq!(rig.drawings, before, "Delete does not reach it either");
    assert_eq!(rig.undo_steps, steps);
    assert!(!rig.layer.claims_drag(), "the pan is the card's to refuse");
}

#[test]
fn demonstration_keeps_the_charts_menu_on_a_right_click() {
    let mut rig = Rig::new();
    rig.rect_with(0.0);
    rig.cursor = CursorMode::Demonstration;
    rig.settle();
    rig.right_click(pos2(250.0, 230.0));
    assert!(rig.layer.menu.is_none(), "no drawing menu");
    assert!(rig.background_click.is_some());
}
