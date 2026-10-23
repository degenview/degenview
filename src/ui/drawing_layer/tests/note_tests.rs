//! The text and note tools: Text, Comment, Price label, Flag mark and Pin.

use super::*;
use crate::model::{AnnotationKind, Caps};

fn annotation_at(rig: &Rig) -> (AnnotationKind, Anchor) {
    match rig.last().geometry() {
        Some(Geometry::Annotation { kind, at }) => (*kind, *at),
        other => panic!("not an annotation: {other:?}"),
    }
}

/// A rig with one finished drawing of `tool` placed at the first point, the crosshair armed and
/// the drawing selected.
fn placed(tool: Tool) -> (Rig, u64) {
    let mut rig = Rig::with_tool(tool);
    rig.place(tool);
    rig.tool = Tool::Cursor;
    let id = rig.last().id();
    rig.selected = vec![id];
    rig.settle();
    (rig, id)
}

// MARK: Text and Comment: nothing without their text

#[test]
fn a_text_waits_for_its_text_and_escape_leaves_nothing_behind() {
    for tool in [Tool::Text, Tool::Comment] {
        let mut rig = Rig::with_tool(tool);
        rig.click(POINTS[0]);
        rig.settle();
        assert!(rig.layer.is_editing_text(), "{tool:?} opens its editor");
        assert!(rig.drawings.is_empty(), "{tool:?} is not added yet");
        assert_eq!(rig.undo_steps, 0);
        // The draft is on the chart meanwhile, growing as it is typed into.
        rig.frame(vec![Event::Text("typed".to_owned())]);
        assert!(rig.layer.text_edit.as_ref().unwrap().preview().is_some());
        rig.escape_consumed = false;
        rig.key(Key::Escape);
        rig.settle();
        assert!(!rig.layer.is_editing_text());
        assert!(rig.drawings.is_empty(), "{tool:?}: Escape adds nothing");
        assert_eq!(rig.undo_steps, 0, "{tool:?}: and is no undo step");
        assert!(rig.escape_consumed);
        assert_eq!(rig.tool, tool, "the tool stays armed");
    }
}

#[test]
fn keeping_no_text_adds_nothing() {
    let mut rig = Rig::with_tool(Tool::Text);
    rig.click(POINTS[0]);
    rig.settle();
    rig.key(Key::Enter);
    rig.settle();
    assert!(rig.drawings.is_empty() && rig.undo_steps == 0);
    // Spaces and line breaks are no text.
    rig.click(POINTS[1]);
    rig.settle();
    rig.frame(vec![Event::Text("   ".to_owned())]);
    rig.key(Key::Enter);
    rig.settle();
    assert!(rig.drawings.is_empty() && rig.undo_steps == 0);
}

#[test]
fn a_text_is_added_with_its_text_in_one_undo_step_at_the_clicked_point() {
    let mut rig = Rig::with_tool(Tool::Text);
    rig.click(POINTS[0]);
    type_and_keep(&mut rig, "Breakout");
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, 1);
    assert_eq!(rig.last().text(), "Breakout");
    assert_eq!(rig.selected, vec![rig.last().id()]);
    let (kind, at) = annotation_at(&rig);
    assert_eq!(kind, AnnotationKind::Text);
    assert!(
        rig.px(at).distance(POINTS[0]) < 1.0,
        "the anchor is where it was clicked"
    );
    // Its top-left corner is on the anchor: the box reaches right and down of it.
    let id = rig.last().id();
    let screen = rig.screen_of(id);
    let rect = screen.hit[0];
    assert!(rect.left_top().distance(rig.px(at)) < 0.5);
    assert!(rect.width() > 20.0 && rect.height() > 10.0);
    // And the tool is still armed for the next one.
    assert_eq!(rig.tool, Tool::Text);
}

#[test]
fn a_selected_texts_text_is_edited_by_clicking_it_and_a_blank_box_is_still_clickable() {
    let (mut rig, id) = placed(Tool::Text);
    let place = text_place(&rig, id);
    rig.click(place);
    assert!(rig.layer.is_editing_text());
    rig.key(Key::Escape);
    rig.settle();
    // The box has no background by default, yet a click inside it, off the letters, picks it.
    rig.selected.clear();
    rig.settle();
    let rect = rig.screen_of(id).hit[0];
    assert!(!rig.drawings.is_empty());
    rig.click(rect.right_bottom() - vec2(1.5, 1.5));
    assert_eq!(rig.selected, vec![id]);
    // A click well outside deselects it.
    rig.click(pos2(900.0, 20.0));
    assert!(rig.selected.is_empty());
}

#[test]
fn a_boxs_right_edge_sets_the_width_the_text_wraps_at() {
    let (mut rig, id) = placed(Tool::Text);
    let screen = rig.screen_of(id);
    let handle = screen
        .handles
        .iter()
        .find(|handle| handle.part == Part::Width)
        .expect("a width handle");
    let before = screen.hit[0].width();
    let from = handle.pos;
    rig.drag(from, from + vec2(90.0, 0.0));
    let style = *rig.last().style().unwrap();
    assert!(style.wrap, "dragging the width turns wrapping on");
    assert!(
        (style.box_width - (before + 90.0)).abs() < 1.5,
        "{}",
        style.box_width
    );
    assert_eq!(rig.undo_steps, 2, "the drag is one more step");
    // Narrower than the narrowest is held there.
    let screen = rig.screen_of(id);
    let handle = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Width)
        .unwrap();
    let from = handle.pos;
    rig.drag(from, from - vec2(600.0, 0.0));
    let style = *rig.last().style().unwrap();
    assert_eq!(style.box_width, crate::model::drawing_style::MIN_BOX_WIDTH);
}

#[test]
fn a_comments_tail_points_at_its_anchor() {
    let (mut rig, id) = placed(Tool::Comment);
    let (_, at) = annotation_at(&rig);
    let screen = rig.screen_of(id);
    let tip = rig.px(at);
    assert!(
        screen.frames[0].contains(&tip),
        "the outline reaches the anchor"
    );
    let rect = screen.hit[0];
    assert!(rect.bottom() < tip.y, "the bubble is above the point");
    assert_eq!(
        screen
            .handles
            .iter()
            .filter(|h| h.part == Part::Anchor(0))
            .count(),
        1
    );
}

#[test]
fn a_text_and_a_comment_keep_their_look_in_the_style_and_their_words_in_the_drawing() {
    for tool in [Tool::Text, Tool::Comment] {
        let (rig, _) = placed(tool);
        assert_eq!(rig.last().text(), "a note");
        let style = *rig.last().style().unwrap();
        assert_eq!(style, tool.default_style().sanitized());
        let json = serde_json::to_value(rig.last()).unwrap();
        let back: Drawing = serde_json::from_value(json).unwrap();
        assert_eq!(&back, rig.last());
    }
}

// MARK: Price label

#[test]
fn a_price_label_shows_the_price_of_its_anchor_and_follows_it() {
    let mut rig = Rig::with_tool(Tool::PriceLabel);
    rig.place(Tool::PriceLabel);
    let id = rig.last().id();
    let (_, at) = annotation_at(&rig);
    let shown = |rig: &mut Rig| rig.screen_of(id).heading.clone().expect("a price");
    assert_eq!(shown(&mut rig), PriceFormat::AUTO.format(at.price));
    // It is worked out when drawn, never stored: the drawing holds no price text.
    assert_eq!(rig.last().text(), "");
    assert!(
        !serde_json::to_string(rig.last())
            .unwrap()
            .contains(&PriceFormat::AUTO.format(at.price))
    );
    // Moving the anchor changes what it says, on a linear scale and on a logarithmic one.
    for price in [91.234, 1.5, -3.25, 1234.5] {
        let mut moved = rig.last().clone();
        if let Some(Geometry::Annotation { at, .. }) = moved.geometry_mut() {
            at.price = price;
        }
        rig.drawings = vec![moved];
        assert_eq!(shown(&mut rig), PriceFormat::AUTO.format(price), "{price}");
    }
}

#[test]
fn a_price_label_points_left_at_the_price_it_names() {
    let mut rig = Rig::with_tool(Tool::PriceLabel);
    rig.place(Tool::PriceLabel);
    let id = rig.last().id();
    let (_, at) = annotation_at(&rig);
    let screen = rig.screen_of(id);
    let tip = rig.px(at);
    assert!(screen.frames[0].contains(&tip));
    let rect = screen.hit[0];
    assert!(rect.left() > tip.x && (rect.center().y - tip.y).abs() < 0.5);
    assert!(
        !Tool::PriceLabel.caps().has(Caps::TEXT),
        "its text is its price"
    );
    assert!(!rig.last().edits_text_in_place());
}

// MARK: Flag mark

#[test]
fn a_flag_is_planted_on_its_anchor_and_has_no_text() {
    let mut rig = Rig::with_tool(Tool::FlagMark);
    rig.click(POINTS[0]);
    assert_eq!(rig.drawings.len(), 1);
    assert!(!rig.layer.is_editing_text());
    let id = rig.last().id();
    let (_, at) = annotation_at(&rig);
    let screen = rig.screen_of(id);
    assert_eq!(
        screen.lines[0][0],
        rig.px(at),
        "the pole stands on the anchor"
    );
    assert!(screen.lines[0][1].y < screen.lines[0][0].y);
    assert!(screen.text.is_none());
    assert!(!rig.last().edits_text_in_place());
    assert_eq!(Tool::FlagMark.caps(), Caps::COLOR);
}

// MARK: Pin

#[test]
fn a_pin_is_placed_at_once_with_its_note_shut() {
    let mut rig = Rig::with_tool(Tool::Pin);
    rig.click(POINTS[0]);
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, 1);
    assert!(!rig.layer.is_editing_text(), "a pin's text is optional");
    assert!(rig.layer.open_notes.is_empty());
    assert_eq!(annotation_at(&rig).0, AnnotationKind::Pin);
}

#[test]
fn clicking_a_pin_opens_its_note_and_clicking_again_or_elsewhere_shuts_it() {
    let (mut rig, id) = placed(Tool::Pin);
    rig.drawings[0].set_text("Earnings gap");
    rig.settle();
    let pin = rig.body(id);
    rig.click(pin);
    assert!(rig.layer.open_notes.contains(&id));
    // Opening a note is view state: no undo step, nothing written to the drawing.
    let steps = rig.undo_steps;
    let stored = rig.last().clone();
    rig.click(pin);
    assert!(rig.layer.open_notes.is_empty());
    assert_eq!(rig.undo_steps, steps);
    assert_eq!(rig.last(), &stored);
    rig.click(pin);
    assert!(rig.layer.open_notes.contains(&id));
    // A click on the note itself leaves it open.
    let note = rig.screen_of(id).popup.as_ref().unwrap().rect;
    assert!(PLOT.contains_rect(note), "the note stays on the plot");
    rig.click(note.center());
    assert!(
        rig.layer.open_notes.contains(&id),
        "{:?}",
        rig.layer.open_notes
    );
    // A click elsewhere shuts it.
    rig.click(pos2(900.0, 360.0));
    assert!(rig.layer.open_notes.is_empty());
}

#[test]
fn a_pins_note_can_be_picked_only_while_it_is_open_and_escape_shuts_it() {
    let (mut rig, id) = placed(Tool::Pin);
    rig.drawings[0].set_text("Earnings gap");
    rig.settle();
    let note = rig.screen_of(id).popup.as_ref().unwrap().rect;
    let inside = note.center();
    rig.selected.clear();
    rig.click(inside);
    assert!(rig.selected.is_empty(), "a shut note is not there to click");
    let pin = rig.body(id);
    rig.click(pin);
    rig.selected.clear();
    rig.click(inside);
    assert_eq!(rig.selected, vec![id], "an open note is part of the pin");
    rig.escape_consumed = false;
    rig.key(Key::Escape);
    assert!(rig.layer.open_notes.is_empty());
    assert!(rig.escape_consumed, "the key is spent shutting it");
}

#[test]
fn double_clicking_a_pin_edits_its_note_where_it_opens() {
    let (mut rig, id) = placed(Tool::Pin);
    let pin = rig.body(id);
    rig.double_click(pin);
    assert!(rig.layer.is_editing_text());
    assert!(
        rig.layer.open_notes.contains(&id),
        "the note opens to be written in"
    );
    assert_eq!(rig.opened, None, "not the properties");
    type_and_keep(&mut rig, "The gap fills");
    assert_eq!(rig.last().text(), "The gap fills");
    assert_eq!(rig.last().id(), id);
}

#[test]
fn a_pins_note_opens_clear_of_the_pin_and_inside_the_plot_near_every_edge() {
    for point in [
        pos2(20.0, 30.0),
        pos2(980.0, 30.0),
        pos2(500.0, 390.0),
        pos2(10.0, 200.0),
        pos2(990.0, 200.0),
    ] {
        let mut rig = Rig::with_tool(Tool::Pin);
        rig.click(point);
        rig.drawings[0]
            .set_text("A note that is long enough to need two lines when it wraps at its width");
        let id = rig.last().id();
        let screen = rig.screen_of(id);
        let note = screen.popup.as_ref().unwrap().rect;
        assert!(PLOT.contains_rect(note), "{point:?}: {note:?}");
        let glyph = screen.solids[0].iter().fold(Rect::NOTHING, |all, p| {
            all.union(Rect::from_min_max(*p, *p))
        });
        assert!(!note.intersects(glyph), "{point:?} covers its own pin");
    }
}

#[test]
fn a_pin_has_a_colour_a_note_and_a_collapsible_note() {
    let caps = Tool::Pin.caps();
    assert!(caps.has(Caps::COLOR) && caps.has(Caps::COLLAPSIBLE) && caps.has(Caps::BOX));
    assert!(caps.has(Caps::INLINE_TEXT) && !caps.has(Caps::TEXT_FIRST));
    assert_eq!(
        Tool::Pin.max_text(),
        crate::model::drawings::LONG_TEXT_CHARS
    );
    let (mut rig, _) = placed(Tool::Pin);
    rig.drawings[0].set_text(&"x".repeat(5_000));
    assert_eq!(
        rig.last().text().chars().count(),
        crate::model::drawings::LONG_TEXT_CHARS
    );
}

// MARK: Shared

#[test]
fn the_new_tools_move_with_their_anchor_when_dragged() {
    for tool in [
        Tool::Text,
        Tool::Comment,
        Tool::PriceLabel,
        Tool::FlagMark,
        Tool::Pin,
    ] {
        let (mut rig, id) = placed(tool);
        let before = annotation_at(&rig).1;
        let grab = rig.body(id);
        rig.drag(grab, grab + vec2(60.0, 30.0));
        let after = annotation_at(&rig).1;
        let moved = rig.px(after) - rig.px(before);
        assert!(
            (moved - vec2(60.0, 30.0)).length() < 1.5,
            "{tool:?}: {moved:?}"
        );
        assert_eq!(rig.undo_steps, 2, "{tool:?}: placing, then the drag");
    }
}

#[test]
fn a_locked_text_cannot_be_edited_in_place() {
    let (mut rig, id) = placed(Tool::Text);
    rig.drawings[0].set_locked(true);
    rig.settle();
    let place = text_place(&rig, id);
    rig.click(place);
    assert!(!rig.layer.is_editing_text());
}
