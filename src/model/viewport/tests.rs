use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9 * a.abs().max(b.abs()).max(1.0)
}

// MARK: Horizontal

#[test]
fn dragging_right_reveals_older_bars_and_left_newer_ones() {
    let mut view = HorizontalViewport::default();
    view.pan(0.0, 50.0, 5.0, 1000);
    assert!(close(view.right_offset, 10.0), "ten bars back in time");
    view.pan(10.0, -30.0, 5.0, 1000);
    assert!(close(view.right_offset, 4.0), "six bars toward the newest");
}

#[test]
fn a_short_series_zooms_from_the_default_width_without_jumping() {
    // A young pool holding 60 bars: the first zoom step must not snap to 60 bars.
    let mut view = HorizontalViewport::default();
    view.set_visible(110, 60);
    assert_eq!(view.visible, 110);
    view.set_visible(400, 60);
    assert_eq!(
        view.visible, DEFAULT_VISIBLE,
        "never wider than a deep chart's default"
    );
    view.set_visible(1, 60);
    assert_eq!(view.visible, MIN_VISIBLE);
    // A deep series still zooms out to everything it holds.
    view.set_visible(5_000, 800);
    assert_eq!(view.visible, 800);
}

#[test]
fn a_drag_keeps_the_fraction_of_a_bar() {
    let mut view = HorizontalViewport::default();
    view.pan(0.0, 7.0, 20.0, 1000);
    assert!(close(view.right_offset, 0.35));
}

#[test]
fn the_same_drag_moves_further_when_bars_are_narrower() {
    let (mut wide, mut narrow) = (HorizontalViewport::default(), HorizontalViewport::default());
    wide.pan(0.0, 100.0, 20.0, 1000);
    narrow.pan(0.0, 100.0, 4.0, 1000);
    assert!(close(wide.right_offset, 5.0) && close(narrow.right_offset, 25.0));
}

#[test]
fn a_drag_is_measured_from_where_it_began_not_accumulated() {
    let mut view = HorizontalViewport::default();
    for dx in [3.0, 9.0, 14.0, 20.0] {
        view.pan(2.0, dx, 10.0, 1000);
    }
    assert!(close(view.right_offset, 4.0));
}

#[test]
fn history_stops_where_a_few_bars_remain() {
    let mut view = HorizontalViewport::default();
    view.pan(0.0, 1.0e6, 5.0, 300);
    assert!(close(view.right_offset, (300 - MIN_VISIBLE) as f64));
}

#[test]
fn the_chart_can_be_dragged_into_empty_space_but_only_so_far() {
    let mut view = HorizontalViewport::default();
    view.pan(0.0, -100.0, 10.0, 1000);
    assert!(close(view.right_offset, -10.0), "ten bars of empty space");
    view.pan(0.0, -1.0e6, 10.0, 1000);
    assert!(
        close(view.right_offset, -((DEFAULT_VISIBLE - MIN_VISIBLE) as f64)),
        "never unbounded"
    );
    assert!(!view.follows_latest());
}

#[test]
fn a_short_series_can_still_be_dragged_into_the_future() {
    let view = HorizontalViewport::default();
    let (low, high) = view.offset_limits(5);
    assert!(low < 0.0 && high >= low, "{low} {high}");
}

#[test]
fn follow_latest_is_the_unscrolled_state() {
    let mut view = HorizontalViewport::default();
    assert!(view.follows_latest());
    view.scroll_by(0.4, 500);
    assert!(!view.follows_latest());
    view.scroll_by(-0.4, 500);
    assert!(view.follows_latest());
}

#[test]
fn the_newest_bar_is_shown_until_it_is_scrolled_out_on_the_right() {
    let mut view = HorizontalViewport::default();
    for offset in [
        0.0,
        -10.0,
        -((DEFAULT_VISIBLE - MIN_VISIBLE) as f64),
        3.0,
        10.0,
    ] {
        view.right_offset = offset;
        assert!(view.shows_newest(10), "{offset}");
    }
    view.right_offset = 10.4;
    assert!(view.shows_newest(10), "its centre is still inside");
    view.right_offset = 10.5;
    assert!(!view.shows_newest(10), "its centre is on the edge");
    view.right_offset = 11.0;
    assert!(!view.shows_newest(10));
    view.right_offset = 0.5;
    assert!(
        !view.shows_newest(0),
        "with no margin it leaves at half a bar"
    );
}

#[test]
fn a_bad_offset_or_slot_width_cannot_poison_the_view() {
    let mut view = HorizontalViewport {
        right_offset: f64::NAN,
        ..Default::default()
    };
    view.clamp(500);
    assert_eq!(view.right_offset, 0.0);
    view.right_offset = 3.0;
    view.pan(3.0, 40.0, 0.0, 500);
    assert_eq!(view.right_offset, 3.0, "no slot width, no movement");
    view.pan(f64::INFINITY, 1.0, 5.0, 500);
    assert_eq!(view.right_offset, 0.0);
}

#[test]
fn zoom_clamps_to_the_series_and_keeps_the_offset_valid() {
    let mut view = HorizontalViewport {
        visible: 120,
        right_offset: -90.0,
    };
    view.set_visible(30, 500);
    assert_eq!(view.visible, 30);
    assert!(close(view.right_offset, -10.0), "{}", view.right_offset);
    view.set_visible(5, 500);
    assert_eq!(view.visible, MIN_VISIBLE);
    view.set_visible(10_000, 400);
    assert_eq!(view.visible, 400);
}

#[test]
fn stretching_the_time_axis_scales_from_where_the_drag_began() {
    let mut view = HorizontalViewport::default();
    view.stretch(120, 2.0, 1000);
    assert_eq!(view.visible, 240);
    view.stretch(120, 0.5, 1000);
    assert_eq!(view.visible, 60);
    view.stretch(120, f64::NAN, 1000);
    assert_eq!(view.visible, 60);
}

// MARK: Vertical

fn range(minimum: f64, maximum: f64) -> PriceRange {
    PriceRange::new(minimum, maximum)
}

#[test]
fn dragging_down_shows_higher_prices() {
    let mut vertical = VerticalViewport::default();
    vertical.pan(range(100.0, 200.0), 50.0, 500.0, false);
    let shown = vertical.manual.expect("a drag switches autoscale off");
    assert!(close(shown.minimum, 110.0) && close(shown.maximum, 210.0));
    assert!(!vertical.is_auto());
}

#[test]
fn dragging_up_shows_lower_prices() {
    let mut vertical = VerticalViewport::default();
    vertical.pan(range(100.0, 200.0), -250.0, 500.0, false);
    let shown = vertical.manual.unwrap();
    assert!(close(shown.minimum, 50.0) && close(shown.maximum, 150.0));
}

#[test]
fn a_pan_keeps_the_height_of_the_range() {
    let mut vertical = VerticalViewport::default();
    vertical.pan(range(100.0, 200.0), 123.0, 400.0, false);
    let shown = vertical.manual.unwrap();
    assert!(close(shown.maximum - shown.minimum, 100.0));
}

#[test]
fn a_logarithmic_pan_moves_by_ratio_and_never_leaves_the_positive_prices() {
    let mut vertical = VerticalViewport::default();
    let origin = range(10.0, 1000.0);
    vertical.pan(origin, -500.0, 500.0, true);
    let low = vertical.manual.unwrap();
    assert!(low.minimum > 0.0 && low.is_valid(true), "{low:?}");
    assert!(close(low.maximum / low.minimum, 100.0), "the ratio is kept");
    vertical.pan(origin, -1.0e9, 500.0, true);
    assert_eq!(
        vertical.manual,
        Some(low),
        "a drag too far to draw is refused"
    );
    vertical.pan(origin, 100.0, 500.0, true);
    let shown = vertical.manual.unwrap();
    assert!(close(shown.minimum.ln() - 10f64.ln(), 0.2 * 100f64.ln()));
}

#[test]
fn an_undrawable_pan_leaves_the_range_alone() {
    let mut vertical = VerticalViewport::default();
    vertical.pan(range(1.0, 2.0), f32::NAN, 500.0, false);
    assert!(vertical.is_auto());
    vertical.pan(range(1.0, 2.0), 10.0, 0.0, false);
    assert!(vertical.is_auto());
    vertical.pan(range(f64::NAN, 2.0), 10.0, 100.0, false);
    assert!(vertical.is_auto());
    vertical.pan(range(-5.0, 2.0), 10.0, 100.0, true);
    assert!(vertical.is_auto(), "a log scale has no negative prices");
}

#[test]
fn autoscale_returns_the_range_the_data_asks_for() {
    let mut vertical = VerticalViewport::default();
    let auto = range(100.0, 200.0);
    assert_eq!(vertical.resolve(auto, false), auto);
    vertical.pan(auto, 100.0, 500.0, false);
    assert_ne!(vertical.resolve(auto, false), auto);
    vertical.reset();
    assert_eq!(vertical.resolve(auto, false), auto);
    assert!(vertical.is_default());
}

#[test]
fn a_manual_range_does_not_follow_the_data() {
    let mut vertical = VerticalViewport::default();
    vertical.pan(range(100.0, 200.0), 50.0, 500.0, false);
    let first = vertical.resolve(range(100.0, 200.0), false);
    let later = vertical.resolve(range(5000.0, 6000.0), false);
    assert_eq!(
        first, later,
        "it would snap back on the next frame otherwise"
    );
}

#[test]
fn a_saved_range_a_log_scale_cannot_draw_falls_back_to_autoscale() {
    let vertical = VerticalViewport {
        zoom: 1.0,
        manual: Some(range(-5.0, 10.0)),
    };
    let auto = range(1.0, 2.0);
    assert_eq!(vertical.resolve(auto, true), auto);
}

#[test]
fn the_axis_drag_stretches_the_automatic_range_about_its_centre() {
    let origin = VerticalViewport::default();
    let mut vertical = origin;
    vertical.stretch(&origin, 0.5, false);
    assert!(close(vertical.zoom, 2.0));
    let shown = vertical.resolve(range(100.0, 200.0), false);
    assert!(close(shown.minimum, 125.0) && close(shown.maximum, 175.0));
    assert!(vertical.is_auto(), "autoscale keeps following the bars");
}

#[test]
fn the_axis_drag_is_limited() {
    let origin = VerticalViewport::default();
    let mut vertical = origin;
    vertical.stretch(&origin, 1.0e-9, false);
    assert_eq!(vertical.zoom, ZOOM_LIMITS.1);
    vertical.stretch(&origin, 1.0e9, false);
    assert_eq!(vertical.zoom, ZOOM_LIMITS.0);
}

#[test]
fn the_axis_drag_scales_a_manual_range_where_it_stands() {
    let origin = VerticalViewport {
        zoom: 1.0,
        manual: Some(range(100.0, 200.0)),
    };
    let mut vertical = origin;
    vertical.stretch(&origin, 2.0, false);
    let shown = vertical.manual.unwrap();
    assert!(close(shown.minimum, 50.0) && close(shown.maximum, 250.0));
}

#[test]
fn the_logarithmic_axis_drag_scales_in_log_space() {
    let range = range(10.0, 1000.0);
    let scaled = range.scaled(2.0, true).unwrap();
    assert!(close(scaled.minimum, 1.0) && close(scaled.maximum, 10000.0));
}

#[test]
fn a_range_never_collapses_to_nothing() {
    assert!(range(100.0, 200.0).scaled(1.0e-30, false).is_none());
    assert!(range(100.0, 200.0).scaled(0.0, false).is_none());
    assert!(range(100.0, 200.0).scaled(f64::NAN, false).is_none());
    assert!(range(200.0, 100.0).scaled(2.0, false).is_none());
}

// MARK: Whole view

#[test]
fn resetting_returns_to_the_newest_bars_at_the_natural_scale() {
    let mut view = ChartViewport::default();
    view.horizontal.visible = 40;
    view.horizontal.right_offset = 12.5;
    view.vertical.manual = Some(range(1.0, 2.0));
    view.vertical.zoom = 3.0;
    view.reset();
    assert_eq!(view, ChartViewport::default());
}

#[test]
fn a_saved_view_comes_back_as_it_was() {
    let mut view = ChartViewport::default();
    view.horizontal.right_offset = -7.25;
    view.vertical.manual = Some(range(60_000.0, 70_000.0));
    let json = serde_json::to_string(&view.saved()).unwrap();
    let mut restored = ChartViewport::default();
    restored.restore(serde_json::from_str(&json).unwrap());
    assert_eq!(restored.horizontal.right_offset, -7.25);
    assert_eq!(restored.vertical.manual, view.vertical.manual);
}

#[test]
fn a_layout_saved_before_the_viewport_decodes_as_the_newest_bars() {
    let saved: SavedViewport = serde_json::from_str("{}").unwrap();
    assert_eq!(saved, SavedViewport::default());
    let mut view = ChartViewport::default();
    view.restore(saved);
    assert!(view.horizontal.follows_latest() && view.vertical.is_auto());
}

#[test]
fn a_corrupt_saved_view_is_not_restored() {
    let mut view = ChartViewport::default();
    view.restore(SavedViewport {
        right_offset: f64::NAN,
        price_range: Some(range(5.0, 1.0)),
    });
    assert!(view.horizontal.follows_latest() && view.vertical.is_auto());
}

// MARK: Pan gesture

fn gesture(view: &ChartViewport) -> PanGesture {
    PanGesture::begin(view, range(100.0, 200.0))
}

#[test]
fn a_horizontal_drag_leaves_autoscale_on_despite_a_shaky_hand() {
    let mut view = ChartViewport::default();
    let mut drag = gesture(&view);
    drag.update(&mut view, (80.0, 2.5), 10.0, 400.0, 1000, false);
    assert!(close(view.horizontal.right_offset, 8.0));
    assert!(view.vertical.is_auto());
}

#[test]
fn a_vertical_drag_takes_the_price_range_with_the_whole_distance() {
    let mut view = ChartViewport::default();
    let mut drag = gesture(&view);
    drag.update(&mut view, (0.0, 3.0), 10.0, 400.0, 1000, false);
    assert!(view.vertical.is_auto(), "not yet");
    drag.update(&mut view, (0.0, 40.0), 10.0, 400.0, 1000, false);
    let shown = view.vertical.manual.expect("autoscale is off");
    assert!(
        close(shown.minimum, 110.0) && close(shown.maximum, 210.0),
        "the distance before the lock-in counts: {shown:?}"
    );
}

#[test]
fn a_diagonal_drag_moves_both_ways_at_once() {
    let mut view = ChartViewport::default();
    let mut drag = gesture(&view);
    drag.update(&mut view, (-50.0, -100.0), 10.0, 400.0, 1000, false);
    assert!(close(view.horizontal.right_offset, -5.0));
    let shown = view.vertical.manual.unwrap();
    assert!(close(shown.minimum, 75.0) && close(shown.maximum, 175.0));
}

#[test]
fn dragging_back_to_the_start_returns_the_chart_where_it_was() {
    let mut view = ChartViewport::default();
    let mut drag = gesture(&view);
    drag.update(&mut view, (90.0, 60.0), 10.0, 400.0, 1000, false);
    drag.update(&mut view, (0.0, 0.0), 10.0, 400.0, 1000, false);
    assert!(close(view.horizontal.right_offset, 0.0));
    let shown = view.vertical.manual.unwrap();
    assert!(close(shown.minimum, 100.0) && close(shown.maximum, 200.0));
}

#[test]
fn a_new_drag_starts_from_the_manual_range_the_last_one_left() {
    let mut view = ChartViewport::default();
    let mut first = gesture(&view);
    first.update(&mut view, (0.0, 40.0), 10.0, 400.0, 1000, false);
    let shown = view.vertical.manual.unwrap();
    let mut second = PanGesture::begin(&view, shown);
    second.update(&mut view, (0.0, 40.0), 10.0, 400.0, 1000, false);
    let shown = view.vertical.manual.unwrap();
    assert!(close(shown.minimum, 120.0) && close(shown.maximum, 220.0));
}

// MARK: Zoom to a rectangle

mod zoom_to_a_rectangle {
    use super::super::zoom::{self, MIN_SELECTION, Selection, Slots};
    use super::*;
    use crate::model::canvas::visible_window;

    const PLOT: (f32, f32, f32, f32) = (100.0, 20.0, 700.0, 420.0);

    /// The slots a plot of `PLOT`'s width lays over `view`, as the card computes them.
    fn slots_of(view: &HorizontalViewport, total: usize, reserved: usize) -> Slots {
        let window = visible_window(total, view.visible, view.right_offset, reserved);
        Slots {
            first: window.start as f64 - f64::from(window.lead),
            left: f64::from(PLOT.0),
            width: f64::from(PLOT.2 - PLOT.0) / (view.visible + reserved) as f64,
        }
    }

    /// The slot coordinates the plot's left and right edges show for `view`.
    fn edges(view: &HorizontalViewport, total: usize, reserved: usize) -> (f64, f64) {
        let right = (total + reserved) as f64 - view.right_offset;
        (right - (view.visible + reserved) as f64, right)
    }

    #[test]
    fn corners_dragged_in_any_direction_give_the_same_rectangle() {
        let (a, b) = ((200.0, 80.0), (320.0, 260.0));
        let expected = Selection::between(a, b, PLOT);
        assert_eq!(Selection::between(b, a, PLOT), expected);
        assert_eq!(Selection::between((a.0, b.1), (b.0, a.1), PLOT), expected);
        assert_eq!(Selection::between((b.0, a.1), (a.0, b.1), PLOT), expected);
        assert_eq!((expected.left, expected.right), (200.0, 320.0));
        assert_eq!((expected.top, expected.bottom), (80.0, 260.0));
    }

    #[test]
    fn a_rectangle_is_cut_to_the_plot_and_never_not_a_number() {
        let s = Selection::between((-50.0, -50.0), (5000.0, 5000.0), PLOT);
        assert_eq!((s.left, s.top, s.right, s.bottom), PLOT);
        let s = Selection::between((f32::NAN, 100.0), (f32::INFINITY, f32::NAN), PLOT);
        assert!(
            [s.left, s.right, s.top, s.bottom]
                .iter()
                .all(|v| v.is_finite())
        );
        assert!(s.left <= s.right && s.top <= s.bottom);
    }

    #[test]
    fn a_click_sized_rectangle_zooms_nothing() {
        let view = HorizontalViewport::default();
        let slots = slots_of(&view, 1000, 10);
        let thin = Selection::between((300.0, 100.0), (300.0 + MIN_SELECTION - 0.5, 300.0), PLOT);
        assert!(!thin.spans_time());
        assert_eq!(zoom::horizontal(&thin, slots, 1000, 10), None);
        let flat = Selection::between((300.0, 100.0), (400.0, 100.0 + MIN_SELECTION - 0.5), PLOT);
        assert!(flat.spans_time() && !flat.spans_price());
        assert!(zoom::horizontal(&flat, slots, 1000, 10).is_some());
        let zero = Selection::between((300.0, 100.0), (300.0, 100.0), PLOT);
        assert_eq!(zoom::horizontal(&zero, slots, 1000, 10), None);
    }

    #[test]
    fn the_selected_bars_fill_the_plot_within_half_a_bar() {
        for (offset, reserved) in [(0.0, 10), (0.0, 0), (250.0, 10), (-6.5, 4), (333.3, 0)] {
            let view = HorizontalViewport {
                visible: 120,
                right_offset: offset,
            };
            let total = 1000;
            let slots = slots_of(&view, total, reserved);
            let s = Selection::between((220.0, 60.0), (520.0, 200.0), PLOT);
            let zoomed = zoom::horizontal(&s, slots, total, reserved).expect("zooms");
            let (first, last) = edges(&zoomed, total, reserved);
            let (want_first, want_last) = (slots.at(s.left), slots.at(s.right));
            assert!(
                (first - want_first).abs() <= 0.5 + 1e-9,
                "offset {offset}: left edge {first} vs {want_first}"
            );
            assert!(
                (last - want_last).abs() <= 0.5 + 1e-9,
                "offset {offset}: right edge {last} vs {want_last}"
            );
            assert!(zoomed.visible < view.visible, "it zooms in");
        }
    }

    #[test]
    fn a_zoom_into_old_history_stays_there() {
        let view = HorizontalViewport {
            visible: 120,
            right_offset: 600.0,
        };
        let slots = slots_of(&view, 1000, 10);
        let s = Selection::between((150.0, 60.0), (400.0, 200.0), PLOT);
        let zoomed = zoom::horizontal(&s, slots, 1000, 10).expect("zooms");
        assert!(
            zoomed.right_offset > 500.0,
            "still hundreds of bars back, not at the newest: {}",
            zoomed.right_offset
        );
        assert!(!zoomed.follows_latest());
    }

    #[test]
    fn a_narrow_selection_grows_to_the_fewest_bars_about_its_centre() {
        let view = HorizontalViewport::default();
        let total = 1000;
        let slots = slots_of(&view, total, 0);
        // 20 points wide is under 4 bars of 5 points each.
        let s = Selection::between((300.0, 60.0), (320.0, 200.0), PLOT);
        let zoomed = zoom::horizontal(&s, slots, total, 0).expect("zooms");
        assert_eq!(zoomed.visible, MIN_VISIBLE);
        let (first, last) = edges(&zoomed, total, 0);
        let centre = (slots.at(s.left) + slots.at(s.right)) / 2.0;
        assert!(((first + last) / 2.0 - centre).abs() < 1e-6);
    }

    #[test]
    fn the_result_always_respects_the_pan_limits() {
        // A selection in the empty space past the newest bar cannot push the view out of bounds.
        let view = HorizontalViewport {
            visible: 120,
            right_offset: -80.0,
        };
        let total = 300;
        let slots = slots_of(&view, total, 5);
        let s = Selection::between((500.0, 60.0), (700.0, 200.0), PLOT);
        let zoomed = zoom::horizontal(&s, slots, total, 5).expect("zooms");
        let (low, high) = zoomed.offset_limits(total);
        assert!(zoomed.right_offset >= low && zoomed.right_offset <= high);
        assert!(zoomed.right_offset.is_finite());
        assert!(zoomed.visible >= MIN_VISIBLE);
    }

    #[test]
    fn unusable_geometry_is_refused_not_propagated() {
        let s = Selection::between((200.0, 60.0), (400.0, 200.0), PLOT);
        let slots = |first, width| Slots {
            first,
            left: 100.0,
            width,
        };
        assert_eq!(zoom::horizontal(&s, slots(0.0, 0.0), 10, 0), None);
        assert_eq!(zoom::horizontal(&s, slots(0.0, f64::NAN), 10, 0), None);
        assert_eq!(zoom::horizontal(&s, slots(f64::NAN, 5.0), 10, 0), None);
        assert_eq!(zoom::horizontal(&s, slots(0.0, f64::INFINITY), 10, 0), None);
        // An empty chart still gives a clamped, finite answer or nothing.
        if let Some(view) = zoom::horizontal(&s, slots(0.0, 5.0), 0, 0) {
            assert!(view.right_offset.is_finite());
        }
    }

    #[test]
    fn a_huge_history_keeps_its_precision() {
        let total = 2_000_000;
        let view = HorizontalViewport {
            visible: 120,
            right_offset: 1_500_000.5,
        };
        let slots = slots_of(&view, total, 10);
        let s = Selection::between((200.0, 60.0), (420.0, 200.0), PLOT);
        let zoomed = zoom::horizontal(&s, slots, total, 10).expect("zooms");
        let (first, last) = edges(&zoomed, total, 10);
        assert!((first - slots.at(s.left)).abs() <= 0.5 + 1e-6);
        assert!((last - slots.at(s.right)).abs() <= 0.5 + 1e-6);
    }

    #[test]
    fn a_price_range_is_sorted_and_valid_for_the_scale() {
        assert_eq!(
            zoom::price_range(110.0, 90.0, false),
            Some(PriceRange::new(90.0, 110.0))
        );
        // Negative prices are fine on a linear scale and refused on a log one.
        assert!(zoom::price_range(-5.0, 5.0, false).is_some());
        assert_eq!(zoom::price_range(-5.0, 5.0, true), None);
        assert_eq!(zoom::price_range(0.0, 5.0, true), None);
        assert!(zoom::price_range(0.5, 5.0, true).is_some());
    }

    #[test]
    fn a_degenerate_price_range_is_refused() {
        assert_eq!(zoom::price_range(10.0, 10.0, false), None);
        assert_eq!(zoom::price_range(f64::NAN, 10.0, false), None);
        assert_eq!(zoom::price_range(1.0, f64::INFINITY, false), None);
        // Thinner than a billionth of the price cannot be drawn.
        assert_eq!(zoom::price_range(1e6, 1e6 + 1e-4, false), None);
        // A tiny instrument is fine when the range is relatively wide.
        assert!(zoom::price_range(1.0e-7, 1.5e-7, false).is_some());
        assert!(zoom::price_range(1.0e-7, 1.5e-7, true).is_some());
    }

    #[test]
    fn a_log_range_keeps_its_ratio() {
        // The card reads these prices from the log scale's inverse; the range stores them as is.
        let (low, high) = (0.002_f64, 0.5_f64);
        let range = zoom::price_range(high, low, true).expect("valid");
        assert!(close(range.maximum / range.minimum, high / low));
        assert!(range.minimum > 0.0);
    }
}
