//! The Image tool: dropping a chosen picture, resizing it, replacing it and painting it.

use super::*;
use crate::model::image_content::{Content, ImageContent, ImageData};
use crate::service::asset_host::{FrozenAssets, freeze};
use menu::ImageOp;

fn picture_of(rig: &Rig) -> ImageData {
    match rig.last().geometry() {
        Some(Geometry::Image(image)) => image.clone(),
        other => panic!("not an image: {other:?}"),
    }
}

/// The picture's rectangle on screen.
fn rect_of(rig: &Rig) -> Rect {
    let image = picture_of(rig);
    Rect::from_min_size(rig.px(image.at), vec2(image.size[0], image.size[1]))
}

/// An image placed by a click, then left selected with the crosshair armed.
fn placed_image() -> (Rig, u64) {
    let mut rig = Rig::with_tool(Tool::Image);
    rig.pending = Some(sample_picture());
    // High enough on the plot that there is room to stretch it downwards.
    rig.click(pos2(100.0, 40.0));
    rig.tool = Tool::Cursor;
    let id = rig.last().id();
    rig.selected = vec![id];
    rig.settle();
    (rig, id)
}

fn another_picture(pixels: [u32; 2]) -> ImageContent {
    ImageContent::new("b".repeat(64), "other.png", pixels)
}

#[test]
fn a_chosen_picture_is_dropped_where_clicked_at_its_own_shape_and_the_choice_is_used_up() {
    let mut rig = Rig::with_tool(Tool::Image);
    rig.pending = Some(sample_picture());
    rig.click(POINTS[0]);
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, 1);
    let rect = rect_of(&rig);
    assert!(rect.left_top().distance(POINTS[0]) < 1.0, "{rect:?}");
    // 320 × 200 fits as it is (the plot is 1000 × 400, and the cap is 40 % of its width).
    assert!((rect.width() / rect.height() - 1.6).abs() < 0.01);
    assert!(rect.width() <= PLOT.width() * 0.4 + 0.5 && rect.height() <= PLOT.height() * 0.5 + 0.5);
    assert_eq!(picture_of(&rig).asset, "a".repeat(64));
    assert_eq!(picture_of(&rig).name, "levels.png");
    assert_eq!(rig.selected, vec![rig.last().id()]);
    // The choice is used up and the tool disarms: the next picture is chosen again.
    assert!(rig.pending.is_none());
    assert_eq!(rig.tool, Tool::Cursor);
}

#[test]
fn a_click_with_nothing_chosen_places_nothing() {
    let mut rig = Rig::with_tool(Tool::Image);
    rig.click(POINTS[0]);
    assert!(rig.drawings.is_empty());
    assert_eq!(rig.undo_steps, 0);
}

#[test]
fn a_large_picture_is_brought_to_the_plot_and_kept_on_it() {
    let mut rig = Rig::with_tool(Tool::Image);
    rig.pending = Some(Content::Image(ImageContent::new(
        "a".repeat(64),
        "big.png",
        [2_000, 1_500],
    )));
    // Near the bottom right corner: it would run off, so it is moved back onto the plot.
    rig.click(pos2(950.0, 380.0));
    let rect = rect_of(&rig);
    assert!(PLOT.contains_rect(rect.shrink(0.01)), "{rect:?}");
    assert!(rect.width() <= PLOT.width() * 0.4 + 0.5);
    assert!((rect.width() / rect.height() - 2_000.0 / 1_500.0).abs() < 0.01);
}

#[test]
fn a_picture_that_arrives_by_a_drop_or_a_paste_is_placed_in_the_middle_without_a_click() {
    let mut rig = Rig::new();
    rig.tool = Tool::Image;
    rig.pending = Some(sample_picture());
    rig.place_now = true;
    rig.settle();
    assert_eq!(rig.drawings.len(), 1);
    let rect = rect_of(&rig);
    assert!(rect.center().distance(PLOT.center()) < 2.0, "{rect:?}");
    assert!(rig.pending.is_none());
    assert_eq!(rig.undo_steps, 1);
}

#[test]
fn zooming_or_dragging_an_axis_moves_a_picture_but_never_resizes_it() {
    let (mut rig, id) = placed_image();
    let drawing = rig.drawings[0].clone();
    let before = rig.screen_of(id).picture.clone().expect("a picture").rect;
    // The time axis zoomed out and in, scrolled, and the price axis stretched or made
    // logarithmic: every one changes where the picture's corner is, none changes its size.
    let stretched = PriceScale {
        minimum: 90.0,
        maximum: 100.0,
        ..scale(false)
    };
    let log = scale(true);
    let mut corners = Vec::new();
    for (slot_width, start, price_scale) in [
        (25.0, 0, &log),
        (400.0, 2, &log),
        (100.0, 3, &stretched),
        (7.0, 0, &stretched),
    ] {
        let mut proj = projection(&rig.data, price_scale);
        proj.slot_width = slot_width;
        proj.start = start;
        let screen = rig.layer.screen(&drawing, &proj).expect("a screen");
        let rect = screen.picture.as_ref().expect("a picture").rect;
        assert_eq!(
            rect.size(),
            before.size(),
            "slot {slot_width}, start {start}: the size does not follow the axes"
        );
        corners.push(rect.min);
    }
    assert!(
        corners
            .iter()
            .any(|corner| corner.distance(before.min) > 5.0),
        "its corner does follow its chart point: {corners:?}"
    );
    // The hit area is the picture's rectangle, at the same size.
    let mut proj = projection(&rig.data, &log);
    proj.slot_width = 25.0;
    let screen = rig.layer.screen(&drawing, &proj).expect("a screen");
    assert_eq!(screen.hit[0].size(), before.size());
}

// MARK: Resizing

#[test]
fn a_corner_keeps_the_pictures_shape_and_the_opposite_corner() {
    let (mut rig, id) = placed_image();
    let before = rect_of(&rig);
    let screen = rig.screen_of(id);
    let corner = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::BottomRight))
        .unwrap()
        .pos;
    // A drag that is mostly sideways: the width leads and the height follows from the shape.
    rig.drag(corner, corner + vec2(80.0, 5.0));
    let after = rect_of(&rig);
    assert!(
        after.left_top().distance(before.left_top()) < 1.0,
        "the opposite corner stays"
    );
    assert!((after.width() - (before.width() + 80.0)).abs() < 1.5);
    assert!((after.width() / after.height() - 1.6).abs() < 0.02);
    // A drag that is mostly down: the larger of the two moves wins.
    let screen = rig.screen_of(id);
    let corner = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::BottomRight))
        .unwrap()
        .pos;
    rig.drag(corner, corner + vec2(2.0, 90.0));
    let again = rect_of(&rig);
    assert!((again.width() / again.height() - 1.6).abs() < 0.02);
    assert!(again.height() > after.height() + 60.0);
    assert_eq!(rig.undo_steps, 3);
}

#[test]
fn an_edge_keeps_the_shape_about_the_pictures_middle() {
    let (mut rig, id) = placed_image();
    let before = rect_of(&rig);
    let screen = rig.screen_of(id);
    let edge = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::Right))
        .unwrap()
        .pos;
    rig.drag(edge, edge + vec2(60.0, 0.0));
    let after = rect_of(&rig);
    assert!((after.width() - (before.width() + 60.0)).abs() < 1.5);
    assert!((after.width() / after.height() - 1.6).abs() < 0.02);
    assert!(
        (after.center().y - before.center().y).abs() < 1.0,
        "grows about its middle"
    );
    assert!((after.left() - before.left()).abs() < 1.0);
}

#[test]
fn with_the_shape_lock_off_each_side_moves_alone_and_dragging_past_the_other_edge_flips_it() {
    let (mut rig, id) = placed_image();
    rig.run(Action::Image(ImageOp::ToggleShapeLock), &[id]);
    assert!(!picture_of(&rig).keep_aspect);
    let before = rect_of(&rig);
    let screen = rig.screen_of(id);
    let edge = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::Bottom))
        .unwrap()
        .pos;
    rig.drag(edge, edge + vec2(0.0, 40.0));
    let after = rect_of(&rig);
    assert!((after.height() - (before.height() + 40.0)).abs() < 1.5);
    assert!((after.width() - before.width()).abs() < 1.0);
    // Past the top edge it turns over and is still a picture of some size.
    let screen = rig.screen_of(id);
    let edge = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::Bottom))
        .unwrap()
        .pos;
    rig.drag(edge, pos2(edge.x, after.top() - 50.0));
    let flipped = rect_of(&rig);
    assert!(
        flipped.height() > 40.0 && flipped.bottom() <= after.top() + 1.0,
        "{flipped:?}"
    );
}

#[test]
fn a_picture_is_never_dragged_smaller_than_a_few_pixels() {
    let (mut rig, id) = placed_image();
    let screen = rig.screen_of(id);
    let corner = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::BottomRight))
        .unwrap()
        .pos;
    let origin = rect_of(&rig).left_top();
    rig.drag(corner, origin + vec2(1.0, 1.0));
    let rect = rect_of(&rig);
    assert!(rect.width() >= 15.0 && rect.height() >= 9.0, "{rect:?}");
}

#[test]
fn moving_a_picture_moves_both_corners_and_keeps_its_size() {
    let (mut rig, id) = placed_image();
    let before = rect_of(&rig);
    let grab = rig.body(id);
    rig.drag(grab, grab + vec2(70.0, 30.0));
    let after = rect_of(&rig);
    assert!((after.size() - before.size()).length() < 1.0);
    assert!(((after.min - before.min) - vec2(70.0, 30.0)).length() < 1.5);
}

// MARK: Replacing, opacity, shape

#[test]
fn replacing_a_picture_keeps_its_place_width_and_opacity_in_one_step() {
    let (mut rig, id) = placed_image();
    rig.run(Action::Image(ImageOp::Opacity(0.5)), &[id]);
    let before = rect_of(&rig);
    let steps = rig.undo_steps;
    rig.replace = Some((id, another_picture([100, 400])));
    rig.settle();
    let image = picture_of(&rig);
    assert_eq!(image.asset, "b".repeat(64));
    assert_eq!(image.name, "other.png");
    assert_eq!(image.pixels, [100, 400]);
    assert_eq!(image.opacity, 0.5);
    assert_eq!(rig.last().id(), id, "the drawing is the same one");
    let after = rect_of(&rig);
    assert!(after.left_top().distance(before.left_top()) < 1.0);
    assert!((after.width() - before.width()).abs() < 1.0);
    assert!(
        (after.height() / after.width() - 4.0).abs() < 0.02,
        "the new picture's shape"
    );
    assert_eq!(rig.undo_steps, steps + 1);
}

#[test]
fn replacing_does_not_touch_a_locked_picture_or_one_that_is_gone() {
    let (mut rig, id) = placed_image();
    rig.drawings[0].set_locked(true);
    let before = rig.last().clone();
    rig.replace = Some((id, another_picture([10, 10])));
    rig.settle();
    assert_eq!(rig.last(), &before);
    rig.replace = Some((id + 99, another_picture([10, 10])));
    rig.settle();
    assert_eq!(rig.last(), &before);
}

#[test]
fn opacity_the_shape_lock_and_the_shape_reset_are_each_one_undoable_edit() {
    let (mut rig, id) = placed_image();
    let steps = rig.undo_steps;
    rig.run(Action::Image(ImageOp::Opacity(0.25)), &[id]);
    assert_eq!(picture_of(&rig).opacity, 0.25);
    assert_eq!(rig.undo_steps, steps + 1);
    // Setting what it already is changes nothing.
    rig.run(Action::Image(ImageOp::Opacity(0.25)), &[id]);
    assert_eq!(rig.undo_steps, steps + 1);
    rig.run(Action::Image(ImageOp::ToggleShapeLock), &[id]);
    assert!(!picture_of(&rig).keep_aspect);
    // Stretched out of shape, then brought back to it at the same width and corner.
    let screen = rig.screen_of(id);
    let edge = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::Bottom))
        .unwrap()
        .pos;
    rig.drag(edge, edge + vec2(0.0, 70.0));
    let stretched = rect_of(&rig);
    rig.run(Action::Image(ImageOp::ResetShape), &[id]);
    let reset = rect_of(&rig);
    assert!((reset.width() - stretched.width()).abs() < 1.0);
    assert!((reset.width() / reset.height() - 1.6).abs() < 0.02);
    assert!(reset.left_top().distance(stretched.left_top()) < 1.0);
}

#[test]
fn replace_and_copy_are_asked_of_the_workspace_not_done_here() {
    let (mut rig, id) = placed_image();
    let proj = projection(&rig.data, &rig.scale);
    let mut frame = DrawingFrame::new(&rig.drawings, Tool::Cursor, None, 50);
    rig.layer.run(
        Action::Image(ImageOp::Replace),
        &[id],
        None,
        &mut frame,
        &proj,
    );
    assert_eq!(frame.replace_request, Some(id));
    assert!(frame.commands.is_empty());
    rig.layer
        .run(Action::Image(ImageOp::Copy), &[id], None, &mut frame, &proj);
    assert_eq!(frame.copy_image, Some("a".repeat(64)));
    assert!(frame.commands.is_empty());
}

// MARK: Painting

fn frozen(rig: &Rig, asset: &str) {
    let picture = std::sync::Arc::new(eframe::egui::ColorImage::filled([8, 5], Color32::RED));
    freeze(
        &rig.ctx,
        FrozenAssets(std::sync::Arc::new(std::collections::HashMap::from([(
            asset.to_owned(),
            picture,
        )]))),
    );
}

#[test]
fn a_picture_that_has_arrived_is_uploaded_once_and_let_go_when_nothing_shows_it() {
    let (mut rig, id) = placed_image();
    assert!(
        !rig.layer.textures.holds(&"a".repeat(64)),
        "nothing to show yet"
    );
    frozen(&rig, &"a".repeat(64));
    rig.settle();
    assert!(rig.layer.textures.holds(&"a".repeat(64)));
    // Panning (a new projection) does not upload it again: the same handle is reused.
    let first = rig.layer.textures.id_of(&"a".repeat(64));
    rig.settle();
    assert_eq!(rig.layer.textures.id_of(&"a".repeat(64)), first);
    // Remove the drawing: the texture goes with it.
    rig.drawings.retain(|drawing| drawing.id() != id);
    rig.settle();
    assert!(!rig.layer.textures.holds(&"a".repeat(64)));
}

#[test]
fn a_picture_that_has_not_arrived_still_has_a_place_to_click_and_resize() {
    let (mut rig, id) = placed_image();
    let screen = rig.screen_of(id);
    assert!(screen.picture.is_some());
    assert_eq!(screen.handles.len(), 8);
    // No host and nothing frozen: the picture is missing, and says so where it will be.
    let rect = screen.hit[0];
    rig.selected.clear();
    rig.click(rect.center());
    assert_eq!(rig.selected, vec![id]);
}

#[test]
fn a_ghost_follows_the_pointer_while_a_picture_waits() {
    let mut rig = Rig::with_tool(Tool::Image);
    rig.pending = Some(sample_picture());
    frozen(&rig, &"a".repeat(64));
    rig.move_to(POINTS[1]);
    rig.settle();
    assert!(
        rig.layer.textures.holds(&"a".repeat(64)),
        "the waiting picture is shown at the pointer"
    );
    assert!(
        rig.drawings.is_empty(),
        "and nothing is placed until the click"
    );
    // Moving off the plot hides it.
    rig.move_to(pos2(-50.0, -50.0));
    rig.settle();
    assert!(!rig.layer.textures.holds(&"a".repeat(64)));
}
