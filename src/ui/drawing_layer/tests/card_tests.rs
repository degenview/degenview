//! The Post and Idea tools: placing a link card, moving and resizing it, and what it asks of the
//! workspace.

use super::*;
use crate::model::link_card::{CARD_SIZE_RANGE, DEFAULT_CARD_SIZE, Link, LinkCard};
use crate::ui::LinkRequest;
use menu::Action;

fn card_of(rig: &Rig) -> (Anchor, LinkCard) {
    match rig.last().geometry() {
        Some(Geometry::Card { anchor, card }) => (*anchor, card.clone()),
        other => panic!("not a card: {other:?}"),
    }
}

fn rect_of(rig: &Rig) -> Rect {
    let (anchor, card) = card_of(rig);
    Rect::from_min_size(rig.px(anchor), vec2(card.size[0], card.size[1]))
}

/// A post card placed by a click and left selected.
fn placed_card(tool: Tool) -> (Rig, u64) {
    let mut rig = Rig::with_tool(tool);
    rig.pending = Some(sample_content(tool));
    rig.click(pos2(100.0, 40.0));
    rig.tool = Tool::Cursor;
    let id = rig.last().id();
    rig.selected = vec![id];
    rig.settle();
    (rig, id)
}

#[test]
fn a_chosen_link_becomes_a_card_where_clicked_and_the_choice_is_used_up() {
    for tool in [Tool::Post, Tool::Idea] {
        let mut rig = Rig::with_tool(tool);
        rig.pending = Some(sample_content(tool));
        rig.click(POINTS[0]);
        assert_eq!(rig.drawings.len(), 1, "{tool:?}");
        assert_eq!(rig.undo_steps, 1);
        assert_eq!(rig.last().tool(), Some(tool));
        let rect = rect_of(&rig);
        assert!(rect.left_top().distance(POINTS[0]) < 1.0, "{rect:?}");
        assert_eq!(
            rect.size(),
            vec2(DEFAULT_CARD_SIZE[0], DEFAULT_CARD_SIZE[1])
        );
        assert_eq!(rig.selected, vec![rig.last().id()]);
        assert!(rig.pending.is_none());
        assert_eq!(rig.tool, Tool::Cursor);
    }
}

#[test]
fn a_click_with_no_link_chosen_places_nothing() {
    let mut rig = Rig::with_tool(Tool::Post);
    rig.click(POINTS[0]);
    assert!(rig.drawings.is_empty());
    assert_eq!(rig.undo_steps, 0);
}

#[test]
fn a_card_dropped_near_an_edge_is_moved_back_onto_the_plot() {
    let mut rig = Rig::with_tool(Tool::Idea);
    rig.pending = Some(sample_content(Tool::Idea));
    rig.click(pos2(980.0, 390.0));
    let rect = rect_of(&rig);
    assert!(PLOT.expand(0.5).contains_rect(rect), "{rect:?}");
}

#[test]
fn a_link_of_the_other_kind_does_not_make_a_card_for_the_tool() {
    let post = sample_content(Tool::Post);
    let at = Anchor {
        time_ms: 1,
        price: 1.0,
    };
    assert!(Drawing::build_with(Tool::Idea, 1, &[at], Tool::Idea.default_style(), &post).is_none());
    assert!(Drawing::build_with(Tool::Post, 1, &[at], Tool::Post.default_style(), &post).is_some());
}

#[test]
fn a_post_goes_to_its_own_time_when_that_bar_is_on_screen_and_anywhere_else_to_the_click() {
    let placed = Anchor {
        time_ms: 5_000,
        price: 42.0,
    };
    // On screen: its own time, the price it was placed at.
    assert_eq!(
        anchor_for_card(Some(8_000), placed, (0, 10_000)),
        Anchor {
            time_ms: 8_000,
            price: 42.0
        }
    );
    // The plot's edges may come in either order.
    assert_eq!(
        anchor_for_card(Some(8_000), placed, (10_000, 0)).time_ms,
        8_000
    );
    // Off screen, or an idea (no time): where it was placed.
    assert_eq!(anchor_for_card(Some(20_000), placed, (0, 10_000)), placed);
    assert_eq!(anchor_for_card(None, placed, (0, 10_000)), placed);
}

#[test]
fn dragging_the_corner_resizes_the_card_within_its_limits_in_one_step() {
    let (mut rig, id) = placed_card(Tool::Post);
    let before = rect_of(&rig);
    let corner = rig
        .screen_of(id)
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::BottomRight))
        .expect("a corner handle")
        .pos;
    let steps = rig.undo_steps;
    rig.drag(corner, corner + vec2(40.0, 20.0));
    let after = rect_of(&rig);
    assert!(
        (after.width() - (before.width() + 40.0)).abs() < 1.5,
        "{after:?}"
    );
    assert!((after.height() - (before.height() + 20.0)).abs() < 1.5);
    assert!(
        after.left_top().distance(before.left_top()) < 1.0,
        "the corner stays put"
    );
    assert_eq!(rig.undo_steps, steps + 1);
    // Dragged far past the limits it stops at them, both ways.
    let corner = after.right_bottom();
    rig.drag(corner, corner - vec2(300.0, 200.0));
    assert_eq!(card_of(&rig).1.size, CARD_SIZE_RANGE.0);
    let corner = rect_of(&rig).right_bottom();
    rig.drag(corner, corner + vec2(700.0, 150.0));
    assert_eq!(card_of(&rig).1.size[0], CARD_SIZE_RANGE.1[0]);
}

#[test]
fn dragging_the_body_moves_the_card_and_keeps_its_size() {
    let (mut rig, id) = placed_card(Tool::Idea);
    let (_, before) = card_of(&rig);
    let from = rig.body(id);
    rig.drag(from, from + vec2(60.0, 30.0));
    let (_, after) = card_of(&rig);
    assert_eq!(after.size, before.size);
    let moved = rect_of(&rig);
    assert!(
        (moved.left() - 160.0).abs() < 2.0 && (moved.top() - 70.0).abs() < 2.0,
        "{moved:?}"
    );
}

#[test]
fn open_copy_replace_and_fetch_are_asked_of_the_workspace_and_change_nothing_here() {
    let (mut rig, id) = placed_card(Tool::Post);
    let proj = projection(&rig.data, &rig.scale);
    for request in [
        LinkRequest::Open,
        LinkRequest::Copy,
        LinkRequest::Replace,
        LinkRequest::Fetch,
    ] {
        let mut frame = DrawingFrame::new(&rig.drawings, Tool::Cursor, None, 50);
        rig.layer
            .run(Action::Link(request), &[id], None, &mut frame, &proj);
        assert_eq!(frame.link_request, Some((id, request)));
        assert!(frame.commands.is_empty());
    }
}

#[test]
fn a_locked_card_can_be_opened_and_copied_but_not_replaced_or_fetched_into() {
    let (mut rig, id) = placed_card(Tool::Post);
    rig.drawings[0].set_locked(true);
    let proj = projection(&rig.data, &rig.scale);
    for (request, allowed) in [
        (LinkRequest::Open, true),
        (LinkRequest::Copy, true),
        (LinkRequest::Replace, false),
        (LinkRequest::Fetch, false),
    ] {
        let mut frame = DrawingFrame::new(&rig.drawings, Tool::Cursor, None, 50);
        rig.layer
            .run(Action::Link(request), &[id], None, &mut frame, &proj);
        assert_eq!(frame.link_request.is_some(), allowed, "{request:?}");
    }
}

#[test]
fn a_waiting_link_is_placed_in_the_middle_of_the_plot_without_a_click() {
    let mut rig = Rig::with_tool(Tool::Post);
    rig.pending = Some(sample_content(Tool::Post));
    rig.place_now = true;
    rig.settle();
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, 1);
    let rect = rect_of(&rig);
    assert!(PLOT.contains_rect(rect));
    assert!((rect.center().x - PLOT.center().x).abs() < 2.0);
    assert!(matches!(
        rig.last().geometry(),
        Some(Geometry::Card { card, .. }) if matches!(card.link, Link::Post { .. })
    ));
    assert!(rig.pending.is_none(), "the choice is used up");
}

#[test]
fn a_card_survives_being_saved_and_a_hand_edited_bad_link_does_not() {
    let (rig, _) = placed_card(Tool::Idea);
    let json = serde_json::to_string(&rig.drawings).unwrap();
    let back: Vec<Drawing> = serde_json::from_str(&json).unwrap();
    // A price read off the screen may differ from its text form in the last digit.
    let (Some(Geometry::Card { anchor: a, card: c }), Some(Geometry::Card { anchor: b, card: d })) =
        (back[0].geometry(), rig.drawings[0].geometry())
    else {
        panic!("cards");
    };
    assert_eq!((a.time_ms, c), (b.time_ms, d));
    assert!((a.price - b.price).abs() < 1e-9);
    assert!(back[0].geometry().unwrap().is_valid());
    // The link is a structured value, not text: a card of a link that does not parse is refused.
    let (anchor, mut card) = card_of(&rig);
    card.link = Link::Idea {
        id: "no spaces allowed".into(),
        symbol: None,
        slug: None,
    };
    assert!(!Geometry::Card { anchor, card }.is_valid());
}
