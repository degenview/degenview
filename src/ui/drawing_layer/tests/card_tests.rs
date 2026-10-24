//! The Post tool: placing a link card, moving and resizing it, and what it asks of the
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
    let mut rig = Rig::with_tool(Tool::Post);
    rig.pending = Some(sample_content(Tool::Post));
    rig.click(POINTS[0]);
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, 1);
    assert_eq!(rig.last().tool(), Some(Tool::Post));
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

#[test]
fn a_click_with_no_link_chosen_places_nothing() {
    let mut rig = Rig::with_tool(Tool::Post);
    rig.click(POINTS[0]);
    assert!(rig.drawings.is_empty());
    assert_eq!(rig.undo_steps, 0);
}

#[test]
fn a_card_dropped_near_an_edge_is_moved_back_onto_the_plot() {
    let mut rig = Rig::with_tool(Tool::Post);
    rig.pending = Some(sample_content(Tool::Post));
    rig.click(pos2(980.0, 390.0));
    let rect = rect_of(&rig);
    assert!(PLOT.expand(0.5).contains_rect(rect), "{rect:?}");
}

#[test]
fn only_the_post_tool_makes_a_card() {
    let post = sample_content(Tool::Post);
    let at = Anchor {
        time_ms: 1,
        price: 1.0,
    };
    assert!(Drawing::build_with(Tool::Post, 1, &[at], Tool::Post.default_style(), &post).is_some());
    assert!(
        Drawing::build_with(Tool::Image, 1, &[at], Tool::Image.default_style(), &post).is_none()
    );
}

/// The post of the sample link and `count` candles `step_ms` apart whose third one holds its time.
fn candles_around_the_post(step_ms: i64, count: usize) -> (i64, Vec<Kline>) {
    let time =
        LinkCard::new(Link::parse("https://x.com/degenview/status/1750000000000000000").unwrap())
            .link
            .time_ms()
            .expect("a snowflake id has a time");
    let first = time - time.rem_euclid(step_ms) - 2 * step_ms;
    let bars = (0..count as i64)
        .map(|index| Kline {
            open_time: first + index * step_ms,
            close_time: first + (index + 1) * step_ms - 1,
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 1.0,
        })
        .collect();
    (time, bars)
}

fn post_line(data: &[Kline], scale: &PriceScale, rect: Rect) -> Option<Pos2> {
    let card =
        LinkCard::new(Link::parse("https://x.com/degenview/status/1750000000000000000").unwrap());
    note_shapes::post_candle(&card, rect, &projection(data, scale))
}

#[test]
fn a_post_points_at_the_candle_that_holds_its_time_on_any_timeframe() {
    let scale = scale(false);
    let card_rect = Rect::from_min_size(pos2(100.0, 20.0), vec2(220.0, 100.0));
    // 1H candles, then 1D candles: the line lands on whichever candle holds the time.
    for step in [3_600_000, 86_400_000] {
        let (time, data) = candles_around_the_post(step, 6);
        let point = post_line(&data, &scale, card_rect).expect("a candle holds it");
        let proj = projection(&data, &scale);
        let index = data.partition_point(|bar| bar.open_time <= time) - 1;
        assert_eq!(index, 2);
        assert_eq!(point.x, proj.x_at_index(index as f64), "step {step}");
    }
}

#[test]
fn the_line_meets_the_high_of_a_candle_from_above_and_its_low_from_below() {
    let scale = scale(false);
    let (_, data) = candles_around_the_post(3_600_000, 6);
    let proj = projection(&data, &scale);
    let (high, low) = (proj.scale.y(110.0), proj.scale.y(90.0));
    let above = Rect::from_min_size(pos2(100.0, high - 150.0), vec2(220.0, 100.0));
    let below = Rect::from_min_size(pos2(100.0, low + 20.0), vec2(220.0, 100.0));
    assert_eq!(post_line(&data, &scale, above).unwrap().y, high);
    assert_eq!(post_line(&data, &scale, below).unwrap().y, low);
}

#[test]
fn a_post_older_than_the_loaded_candles_or_newer_than_the_last_has_no_line() {
    let scale = scale(false);
    let rect = Rect::from_min_size(pos2(100.0, 20.0), vec2(220.0, 100.0));
    let (time, mut data) = candles_around_the_post(3_600_000, 6);
    data.drain(..3);
    assert!(data[0].open_time > time);
    assert!(
        post_line(&data, &scale, rect).is_none(),
        "older than the data"
    );
    let (_, mut data) = candles_around_the_post(3_600_000, 6);
    data.truncate(2);
    assert!(
        post_line(&data, &scale, rect).is_none(),
        "after the newest candle"
    );
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
    let (mut rig, id) = placed_card(Tool::Post);
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
fn the_preview_chip_is_offered_only_while_the_card_has_no_preview() {
    use crate::model::link_card::LinkMeta;
    let (mut rig, _) = placed_card(Tool::Post);
    let labels = |rig: &Rig| -> Vec<&'static str> {
        menu::link_chips(&rig.drawings[0])
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    };
    assert_eq!(labels(&rig), ["Open", "Copy link", "Replace", "Preview"]);
    if let Some(Geometry::Card { card, .. }) = rig.drawings[0].geometry_mut() {
        card.meta = Some(LinkMeta {
            title: "Developers".into(),
            author: "@degenview".into(),
            excerpt: "Hello".into(),
            fetched_at_ms: 1,
            avatar: None,
        });
    }
    assert_eq!(labels(&rig), ["Open", "Copy link", "Replace"]);
}

#[test]
fn a_card_has_no_style_controls_for_a_divider_to_follow() {
    let (rig, _) = placed_card(Tool::Post);
    assert!(!menu::has_style_controls(&rig.drawings[0]));
    let (rig, _) = placed_card(Tool::Image);
    assert!(!menu::has_style_controls(&rig.drawings[0]));
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
    assert_eq!(rig.last().tool(), Some(Tool::Post));
    assert!(rig.pending.is_none(), "the choice is used up");
}

#[test]
fn a_card_survives_being_saved_and_a_hand_edited_bad_link_does_not() {
    let (rig, _) = placed_card(Tool::Post);
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
    card.link = Link {
        id: 7,
        handle: Some("not a handle".into()),
    };
    assert!(!Geometry::Card { anchor, card }.is_valid());
}
