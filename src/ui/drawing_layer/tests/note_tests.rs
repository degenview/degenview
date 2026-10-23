//! The text and note tools: Text, Comment, Price label, Flag mark and Pin.

use super::*;
use crate::model::{AnnotationKind, Caps, LeaderKind};

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

// MARK: Note, Price note and Callout

fn leader_at(rig: &Rig) -> (LeaderKind, Anchor, Anchor) {
    match rig.last().geometry() {
        Some(Geometry::Leader { kind, at, label }) => (*kind, *at, *label),
        other => panic!("not a leader: {other:?}"),
    }
}

#[test]
fn a_note_and_a_callout_wait_for_their_text_after_both_points() {
    for tool in [Tool::Note, Tool::Callout, Tool::Signpost] {
        let mut rig = Rig::with_tool(tool);
        rig.click(POINTS[0]);
        assert!(
            rig.layer.claims_drag(),
            "{tool:?} waits for its second point"
        );
        rig.click(POINTS[1]);
        rig.settle();
        assert!(rig.layer.is_editing_text(), "{tool:?} opens its editor");
        assert!(rig.drawings.is_empty() && rig.undo_steps == 0);
        rig.key(Key::Escape);
        rig.settle();
        assert!(rig.drawings.is_empty(), "{tool:?}: Escape leaves nothing");
        assert_eq!(rig.undo_steps, 0);
    }
}

#[test]
fn a_note_points_from_its_box_at_the_anchor_with_a_leader_and_a_dot() {
    let (mut rig, id) = placed(Tool::Note);
    let (kind, at, label) = leader_at(&rig);
    assert_eq!(kind, LeaderKind::Note);
    assert!(rig.px(at).distance(POINTS[0]) < 1.0 && rig.px(label).distance(POINTS[1]) < 1.0);
    let screen = rig.screen_of(id);
    let rect = screen.hit[0];
    // The box is centred on the label point.
    assert!(rect.center().distance(rig.px(label)) < 0.6);
    // The leader joins the anchor to the nearest point of the box, and a dot marks the anchor.
    let leader = &screen.lines[0];
    assert_eq!(leader[0], rig.px(at));
    assert!(rect.expand(0.01).contains(leader[1]));
    assert_eq!(screen.discs.len(), 1);
    assert_eq!(screen.discs[0].0, rig.px(at));
    // The anchor, the label and the width each have a handle of their own.
    let parts: Vec<Part> = screen.handles.iter().map(|h| h.part).collect();
    assert!(parts.contains(&Part::Anchor(0)) && parts.contains(&Part::Label));
    assert!(parts.contains(&Part::Width));
}

#[test]
fn a_notes_label_moves_by_the_pointers_travel_without_jumping_and_the_anchor_stays() {
    let (mut rig, id) = placed(Tool::Note);
    let (_, at, label) = leader_at(&rig);
    let screen = rig.screen_of(id);
    let handle = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Label)
        .unwrap()
        .pos;
    rig.drag(handle, handle + vec2(70.0, -25.0));
    let (_, at_after, label_after) = leader_at(&rig);
    assert_eq!(at_after, at, "the anchor does not move");
    let moved = rig.px(label_after) - rig.px(label);
    assert!((moved - vec2(70.0, -25.0)).length() < 1.5, "{moved:?}");
    // And dragging the anchor moves it alone.
    let screen = rig.screen_of(id);
    let handle = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Anchor(0))
        .unwrap()
        .pos;
    rig.drag(handle, handle + vec2(-40.0, 30.0));
    let (_, _, label_last) = leader_at(&rig);
    assert_eq!(label_last, label_after);
}

#[test]
fn a_callouts_tail_reaches_its_anchor_and_goes_when_the_anchor_is_inside() {
    let (mut rig, id) = placed(Tool::Callout);
    let (_, at, _) = leader_at(&rig);
    let screen = rig.screen_of(id);
    assert!(
        screen.frames[0].contains(&rig.px(at)),
        "the tail ends on the anchor"
    );
    assert!(screen.lines.is_empty(), "a callout has no separate leader");
    let rect = screen.hit[0];
    let plain = screen.frames[0].len();
    // Moved into its own bubble, the anchor has no tail to draw.
    let mut inside = rig.last().clone();
    if let Some(Geometry::Leader { at, .. }) = inside.geometry_mut() {
        *at = rig.proj().anchor(rect.center());
    }
    rig.drawings = vec![inside];
    let screen = rig.screen_of(id);
    assert!(screen.frames[0].len() < plain);
}

#[test]
fn a_price_note_is_added_at_once_and_writes_the_price_of_its_anchor() {
    let mut rig = Rig::with_tool(Tool::PriceNote);
    rig.click(POINTS[0]);
    rig.click(POINTS[1]);
    assert_eq!(rig.drawings.len(), 1, "its own text is optional");
    assert!(!rig.layer.is_editing_text());
    let id = rig.last().id();
    let (kind, at, _) = leader_at(&rig);
    assert_eq!(kind, LeaderKind::PriceNote);
    assert_eq!(
        rig.screen_of(id).heading,
        Some(PriceFormat::AUTO.format(at.price))
    );
    // It follows the anchor.
    let mut moved = rig.last().clone();
    if let Some(Geometry::Leader { at, .. }) = moved.geometry_mut() {
        at.price = 87.5;
    }
    rig.drawings = vec![moved];
    assert_eq!(
        rig.screen_of(id).heading,
        Some(PriceFormat::AUTO.format(87.5))
    );
    // Its text is its own, below the price, and the price is not stored in it.
    rig.drawings[0].set_text("support");
    assert_eq!(rig.last().text(), "support");
    assert!(
        !serde_json::to_string(rig.last())
            .unwrap()
            .contains(&PriceFormat::AUTO.format(87.5))
    );
    assert!(rig.screen_of(id).hit[0].height() > 20.0);
}

// MARK: Signpost

fn signpost_at(rig: &Rig) -> (Anchor, Anchor) {
    match rig.last().geometry() {
        Some(Geometry::Signpost { anchor, label }) => (*anchor, *label),
        other => panic!("not a signpost: {other:?}"),
    }
}

#[test]
fn a_signposts_plate_is_always_straight_above_or_below_its_anchor() {
    let (mut rig, id) = placed(Tool::Signpost);
    let (anchor, label) = signpost_at(&rig);
    // Placed with the second click off to the side: only its height was kept.
    assert_eq!(label.time_ms, anchor.time_ms);
    assert!((rig.px(label).y - POINTS[1].y).abs() < 1.5);
    let screen = rig.screen_of(id);
    let rect = screen.hit[0];
    assert!((rect.center().x - rig.px(anchor).x).abs() < 0.6);
    // The stem is one vertical line from the marker to the plate.
    let stem = &screen.styled[0].path;
    assert_eq!(stem[0].x, stem[1].x);
    assert_eq!(stem[0].x, rig.px(anchor).x);
    // The label handle cannot drag it sideways.
    let handle = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Label)
        .unwrap()
        .pos;
    rig.drag(handle, handle + vec2(120.0, 40.0));
    let (anchor, label) = signpost_at(&rig);
    assert_eq!(label.time_ms, anchor.time_ms, "the plate stays on its bar");
    // Nor can the anchor take the plate with it off its bar.
    let screen = rig.screen_of(id);
    let handle = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Anchor(0))
        .unwrap()
        .pos;
    rig.drag(handle, handle + vec2(-80.0, 20.0));
    let (anchor, label) = signpost_at(&rig);
    assert_eq!(label.time_ms, anchor.time_ms);
    // The coordinates tab does not offer the time of the label, which is not its own.
    assert_eq!(rig.last().geometry().unwrap().reads(1), (false, true));
}

#[test]
fn a_signposts_plate_keeps_clear_of_its_marker_on_the_side_it_was_put() {
    let (mut rig, id) = placed(Tool::Signpost);
    let (anchor, _) = signpost_at(&rig);
    let marker_y = rig.px(anchor).y;
    // A label barely below the anchor still puts the plate below it and clear of the marker.
    let mut near = rig.last().clone();
    if let Some(Geometry::Signpost { label, .. }) = near.geometry_mut() {
        label.price = anchor.price - 0.1;
    }
    rig.drawings = vec![near];
    let rect = rig.screen_of(id).hit[0];
    assert!(rect.top() > marker_y + 4.0, "{rect:?} vs {marker_y}");
    // And above it for a label above.
    let mut above = rig.last().clone();
    if let Some(Geometry::Signpost { label, .. }) = above.geometry_mut() {
        label.price = anchor.price + 0.1;
    }
    rig.drawings = vec![above];
    let rect = rig.screen_of(id).hit[0];
    assert!(rect.bottom() < marker_y - 4.0);
}

#[test]
fn a_signposts_emoji_pin_is_shown_on_a_disc_of_its_own_colour_only_when_asked() {
    let (mut rig, id) = placed(Tool::Signpost);
    let plain = rig.screen_of(id);
    assert!(plain.glyph.is_none() && plain.discs.len() == 1);
    let small = plain.discs[0].1;
    let mut with = rig.last().clone();
    let mut style = *with.style().unwrap();
    style.emoji_shown = true;
    style.emoji = crate::model::drawing_style::Emoji::Crown;
    style.emoji_fill = [200, 20, 20, 255];
    with.set_style(style);
    rig.drawings = vec![with];
    let screen = rig.screen_of(id);
    let (_, glyph, _) = screen.glyph.expect("the emoji");
    assert_eq!(glyph, crate::model::drawing_style::Emoji::Crown.glyph());
    assert_eq!(screen.discs[0].2, [200, 20, 20, 255]);
    assert!(screen.discs[0].1 > small);
}

#[test]
fn a_signpost_needs_a_height_to_exist() {
    use crate::model::Geometry;
    let a = anchor(300_000, 100.0);
    assert!(Geometry::from_anchors(Tool::Signpost, &[a, a]).is_none());
    assert!(Geometry::from_anchors(Tool::Signpost, &[a, anchor(900_000, 100.0)]).is_none());
    let made = Geometry::from_anchors(Tool::Signpost, &[a, anchor(900_000, 104.0)]).unwrap();
    assert_eq!(
        made,
        Geometry::Signpost {
            anchor: a,
            label: anchor(300_000, 104.0)
        }
    );
}

// MARK: Shared by the two-point notes

#[test]
fn the_two_point_notes_move_together_and_keep_their_look_through_a_round_trip() {
    for tool in [Tool::Note, Tool::PriceNote, Tool::Callout, Tool::Signpost] {
        let (mut rig, id) = placed(tool);
        let before = rig.last().anchors();
        let grab = rig.body(id);
        rig.drag(grab, grab + vec2(0.0, 40.0));
        let after = rig.last().anchors();
        let moved = rig.px(after[0]) - rig.px(before[0]);
        let moved_label = rig.px(after[1]) - rig.px(before[1]);
        assert!(
            (moved - vec2(0.0, 40.0)).length() < 1.5,
            "{tool:?}: {moved:?}"
        );
        assert!(
            (moved_label - moved).length() < 1.5,
            "{tool:?}: both ends move"
        );
        let json = serde_json::to_value(rig.last()).unwrap();
        let back: Drawing = serde_json::from_value(json).unwrap();
        assert_eq!(&back, rig.last(), "{tool:?}");
    }
}

// MARK: Emoji

#[test]
fn every_emoji_a_signpost_offers_is_one_the_apps_fonts_can_draw() {
    use crate::model::drawing_style::Emoji;
    // Only the fonts egui bundles: they are all a Linux build has, and the macOS system fonts the
    // app puts before them would hide a symbol that is missing everywhere else.
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let font = egui::FontId::proportional(14.0);
    let mut seen = std::collections::HashSet::new();
    for emoji in Emoji::ALL {
        assert!(seen.insert(emoji.glyph()), "{emoji:?} repeats a glyph");
        assert!(
            ctx.fonts_mut(|fonts| emoji.glyph().chars().all(|c| fonts.has_glyph(&font, c))),
            "{emoji:?} has no glyph in the bundled fonts"
        );
        assert!(!emoji.label().is_empty());
        // One character, so a stored name can never mean anything else.
        assert_eq!(emoji.glyph().chars().count(), 1, "{emoji:?}");
    }
    assert_eq!(Emoji::default(), Emoji::ALL[0]);
}
