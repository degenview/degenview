use super::selection::{MIN_SIDE, dragged, released};
use eframe::egui::{Pos2, Rect, pos2, vec2};

fn area() -> Rect {
    Rect::from_min_size(pos2(10.0, 20.0), vec2(800.0, 500.0))
}

#[test]
fn a_drag_in_any_direction_is_the_same_rectangle() {
    let (a, b) = (pos2(100.0, 120.0), pos2(300.0, 320.0));
    let want = Rect::from_min_max(a, b);
    // Down-right, up-left, down-left, up-right.
    assert_eq!(released(a, b, area()), Some(want));
    assert_eq!(released(b, a, area()), Some(want));
    assert_eq!(
        released(pos2(100.0, 320.0), pos2(300.0, 120.0), area()),
        Some(want)
    );
    assert_eq!(
        released(pos2(300.0, 120.0), pos2(100.0, 320.0), area()),
        Some(want)
    );
}

#[test]
fn a_selection_is_kept_inside_the_area() {
    let got = released(pos2(-50.0, 40.0), pos2(2_000.0, 900.0), area()).expect("large");
    assert_eq!(
        got,
        Rect::from_min_max(pos2(10.0, 40.0), pos2(810.0, 520.0))
    );

    let got = released(pos2(5_000.0, -5_000.0), pos2(400.0, 200.0), area()).expect("large");
    assert_eq!(
        got,
        Rect::from_min_max(pos2(400.0, 20.0), pos2(810.0, 200.0))
    );
}

#[test]
fn a_selection_smaller_than_the_minimum_on_a_side_is_cancelled() {
    let from = pos2(100.0, 100.0);
    assert_eq!(released(from, from, area()), None, "a click");
    assert_eq!(
        released(from, pos2(100.0 + MIN_SIDE - 0.5, 300.0), area()),
        None,
        "too narrow"
    );
    assert_eq!(
        released(from, pos2(300.0, 100.0 + MIN_SIDE - 0.5), area()),
        None,
        "too short"
    );
    assert_eq!(
        released(from, pos2(101.0, 400.0), area()),
        None,
        "a sliver is no selection"
    );
}

#[test]
fn the_minimum_itself_counts() {
    let from = pos2(100.0, 100.0);
    let got = released(from, pos2(100.0 + MIN_SIDE, 100.0 + MIN_SIDE), area()).expect("24 pt");
    assert_eq!(got.size(), vec2(MIN_SIDE, MIN_SIDE));
}

#[test]
fn the_minimum_applies_to_what_is_left_after_clamping() {
    // Dragged from just inside the edge to far outside: only 10 pt of it is in the area.
    assert_eq!(
        released(pos2(800.0, 100.0), pos2(2_000.0, 400.0), area()),
        None
    );
}

#[test]
fn the_rectangle_shown_while_dragging_is_not_cancelled() {
    let from = pos2(100.0, 100.0);
    let shown = dragged(from, pos2(105.0, 103.0), area());
    assert_eq!(shown, Rect::from_min_max(from, pos2(105.0, 103.0)));
    assert_eq!(released(from, pos2(105.0, 103.0), area()), None);
}

#[test]
fn a_corner_that_is_not_a_number_counts_as_the_areas_top_left() {
    let got = dragged(pos2(f32::NAN, 100.0), pos2(300.0, 300.0), area());
    assert_eq!(got, Rect::from_min_max(area().min, pos2(300.0, 300.0)));
    assert!(dragged(Pos2::new(f32::INFINITY, f32::NAN), pos2(0.0, 0.0), area()).is_finite());
}

#[test]
fn an_empty_area_selects_nothing() {
    let empty = Rect::from_min_size(pos2(10.0, 10.0), vec2(0.0, 0.0));
    assert_eq!(released(pos2(0.0, 0.0), pos2(100.0, 100.0), empty), None);
}
