use super::framing::{
    Base, COARSE_STEP, Dial, FINE_STEP, Framing, MAX, MIN, POSITION_LIMIT, RUBBER_REACH, SNAP,
    Step, resist,
};
use crate::model::chart::canvas::visible_window;
use crate::model::chart::viewport::{ChartViewport, HorizontalViewport, PriceRange};
use eframe::egui::{Vec2, vec2};

const PLOT: Vec2 = vec2(800.0, 400.0);

/// 100 bars across (plus 10 free slots), scrolled 20 back from the newest of 1,000: slots 880..990
/// of a chart that ends at 1,010, centred on 935.
fn base_over(price: PriceRange, logarithmic: bool) -> Base {
    Base::from_view(
        &HorizontalViewport {
            visible: 100,
            right_offset: 20.0,
        },
        price,
        logarithmic,
        1_000,
        10,
        PLOT,
    )
}

fn base() -> Base {
    base_over(PriceRange::new(100.0, 200.0), false)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

/// The slots a viewport shows, read the way the chart reads them: the slot coordinate of the
/// plot's left edge, and how many slots there are.
fn window_of(view: &ChartViewport, base: &Base) -> (f64, usize) {
    let window = visible_window(
        base.total_bars,
        view.horizontal.visible,
        view.horizontal.right_offset,
        base.reserved,
    );
    (window.start as f64 - f64::from(window.lead), window.slots)
}

fn centre_of(view: &ChartViewport, base: &Base) -> f64 {
    let (left, slots) = window_of(view, base);
    left + slots as f64 / 2.0
}

fn range_of(view: &ChartViewport) -> PriceRange {
    view.vertical.manual.expect("a manual range")
}

fn framed(width: f64, height: f64, dx: f64, dy: f64) -> Framing {
    Framing {
        width,
        height,
        dx,
        dy,
    }
}

// MARK: The starting view

#[test]
fn the_default_framing_is_the_view_the_capture_started_with() {
    let base = base();
    let view = Framing::default().viewport(&base, PLOT);
    assert_eq!(view.horizontal.visible, 100);
    assert!(near(view.horizontal.right_offset, 20.0));
    assert!(near(centre_of(&view, &base), base.start_bar_center));
    let range = range_of(&view);
    assert!(
        near(range.minimum, 100.0) && near(range.maximum, 200.0),
        "{range:?}"
    );
    assert_eq!(view.vertical.zoom, 1.0);
}

#[test]
fn the_base_is_read_from_the_view_the_chart_draws() {
    let base = base();
    assert!(near(base.start_bar_center, 935.0));
    assert_eq!(base.bars_in_view, 110.0);
    assert_eq!((base.total_bars, base.reserved), (1_000, 10));
    assert_eq!(base.plot, PLOT);
}

// MARK: Width

#[test]
fn twice_the_width_shows_half_the_bars_and_half_the_width_twice() {
    let base = base();
    let double = framed(2.0, 1.0, 0.0, 0.0).viewport(&base, PLOT);
    assert_eq!(double.horizontal.visible, 45, "(110 / 2) - 10 free slots");
    let half = framed(0.5, 1.0, 0.0, 0.0).viewport(&base, PLOT);
    assert_eq!(half.horizontal.visible, 210, "(110 * 2) - 10 free slots");
}

#[test]
fn the_centre_stays_where_the_capture_had_it_when_the_width_changes() {
    let base = base();
    for width in [0.5, 0.75, 1.0, 1.3, 2.0] {
        let view = framed(width, 1.0, 0.0, 0.0).viewport(&base, PLOT);
        assert!(
            (centre_of(&view, &base) - 935.0).abs() < 1e-4,
            "{width}: {}",
            centre_of(&view, &base)
        );
    }
}

#[test]
fn a_wider_composition_shows_more_bars_of_the_same_width() {
    let base = base();
    let wide = Framing::default().viewport(&base, vec2(1_600.0, 400.0));
    assert_eq!(
        wide.horizontal.visible, 210,
        "twice the room, twice the bars"
    );
    let narrow = Framing::default().viewport(&base, vec2(400.0, 400.0));
    assert_eq!(narrow.horizontal.visible, 45);
}

#[test]
fn a_slot_keeps_the_captures_width_times_the_dial_to_within_a_half_slot() {
    let base = base();
    let capture_slot = f64::from(PLOT.x) / base.bars_in_view;
    for width in [0.5, 0.8, 1.0, 1.37, 2.0] {
        for across in [500.0_f32, 800.0, 1_333.0] {
            let plot = vec2(across, 400.0);
            let view = framed(width, 1.0, 0.0, 0.0).viewport(&base, plot);
            let slots = view.horizontal.visible as f64 + 10.0;
            let slot = f64::from(plot.x) / slots;
            let wanted = capture_slot * width;
            // The bar count is whole: the error is at most half a slot over the plot.
            assert!(
                (slot - wanted).abs() <= wanted * 0.5 / slots * 1.01,
                "{width} {across}: {slot} against {wanted}"
            );
        }
    }
}

#[test]
fn a_chart_may_show_fewer_than_twenty_bars() {
    let few = Base::from_view(
        &HorizontalViewport {
            visible: 30,
            right_offset: 0.0,
        },
        PriceRange::new(1.0, 2.0),
        false,
        500,
        0,
        PLOT,
    );
    let view = framed(2.0, 1.0, 0.0, 0.0).viewport(&few, PLOT);
    assert_eq!(view.horizontal.visible, 15);
}

#[test]
fn at_least_two_bars_are_always_shown() {
    let tiny = Base::from_view(
        &HorizontalViewport {
            visible: 3,
            right_offset: 0.0,
        },
        PriceRange::new(1.0, 2.0),
        false,
        500,
        0,
        PLOT,
    );
    let view = framed(2.0, 1.0, 0.0, 0.0).viewport(&tiny, PLOT);
    assert_eq!(view.horizontal.visible, 2);
    let with_free = base();
    let view = framed(2.0, 1.0, 0.0, 0.0).viewport(&with_free, vec2(1.0, 400.0));
    assert_eq!(view.horizontal.visible, 2, "a sliver of a plot");
}

// MARK: Height

#[test]
fn twice_the_height_shows_half_the_prices_about_the_same_centre() {
    let base = base();
    let double = range_of(&framed(1.0, 2.0, 0.0, 0.0).viewport(&base, PLOT));
    assert!(
        near(double.minimum, 125.0) && near(double.maximum, 175.0),
        "{double:?}"
    );
    let half = range_of(&framed(1.0, 0.5, 0.0, 0.0).viewport(&base, PLOT));
    assert!(
        near(half.minimum, 50.0) && near(half.maximum, 250.0),
        "{half:?}"
    );
}

#[test]
fn a_taller_composition_shows_more_prices_at_the_same_price_per_point() {
    let base = base();
    let tall = range_of(&Framing::default().viewport(&base, vec2(800.0, 600.0)));
    assert!(
        near(tall.minimum, 75.0) && near(tall.maximum, 225.0),
        "{tall:?}"
    );
    // 100 prices over 400 pt before; 150 over 600 pt now.
    assert!(near((tall.maximum - tall.minimum) / 600.0, 100.0 / 400.0));
}

#[test]
fn on_a_log_scale_the_height_works_in_log_space() {
    let base = base_over(PriceRange::new(100.0, 10_000.0), true);
    let range = range_of(&framed(1.0, 2.0, 0.0, 0.0).viewport(&base, PLOT));
    let centre = (range.minimum * range.maximum).sqrt();
    assert!(
        near(centre, 1_000.0),
        "the geometric centre is kept: {centre}"
    );
    assert!(
        near(range.maximum / range.minimum, 10.0),
        "half the log span: {range:?}"
    );
    let wide = range_of(&framed(1.0, 0.5, 0.0, 0.0).viewport(&base, PLOT));
    assert!(near(wide.maximum / wide.minimum, 1.0e4), "{wide:?}");
}

#[test]
fn a_vertical_move_is_a_share_of_the_starting_range_in_the_scales_space() {
    let base = base();
    let up = range_of(&framed(1.0, 1.0, 0.0, 0.2).viewport(&base, PLOT));
    assert!(near(up.minimum, 120.0) && near(up.maximum, 220.0), "{up:?}");
    // Still a share of the *starting* range when the dial has narrowed what is shown.
    let zoomed = range_of(&framed(1.0, 2.0, 0.0, 0.2).viewport(&base, PLOT));
    assert!(
        near(zoomed.minimum, 145.0) && near(zoomed.maximum, 195.0),
        "{zoomed:?}"
    );

    let log = base_over(PriceRange::new(100.0, 10_000.0), true);
    let moved = range_of(&framed(1.0, 1.0, 0.0, 0.25).viewport(&log, PLOT));
    assert!(
        near(moved.minimum, 100.0 * 100.0_f64.powf(0.25)),
        "{moved:?}"
    );
    assert!(near(moved.maximum / moved.minimum, 100.0));
}

// MARK: Position

#[test]
fn a_horizontal_move_shifts_the_centre_in_bars() {
    let base = base();
    for dx in [-20.0, -3.5, 0.0, 12.25, 20.0] {
        let view = framed(1.0, 1.0, dx, 0.0).viewport(&base, PLOT);
        assert!(
            (centre_of(&view, &base) - (935.0 + dx)).abs() < 1e-4,
            "{dx}: {}",
            centre_of(&view, &base)
        );
    }
}

#[test]
fn offsets_are_limited_to_a_quarter_of_the_starting_view() {
    let base = base();
    let bounds = Framing::default().bounds(&base, PLOT);
    assert_eq!(POSITION_LIMIT, 0.25);
    assert!(near(bounds.dx.0, -27.5), "{:?}", bounds.dx);
    assert_eq!(bounds.dy, (-0.25, 0.25));
}

#[test]
fn an_offset_beyond_the_limit_never_draws_past_the_rubber_band() {
    let base = base();
    let band = RUBBER_REACH * base.bars_in_view;
    let left = framed(1.0, 1.0, -500.0, 0.0).viewport(&base, PLOT);
    assert!((centre_of(&left, &base) - (935.0 - 27.5 - band)).abs() < 1e-4);
    let up = range_of(&framed(1.0, 1.0, 0.0, 9.0).viewport(&base, PLOT));
    assert!(
        near(up.minimum, 100.0 + 100.0 * (0.25 + RUBBER_REACH)),
        "{up:?}"
    );
    let bad = framed(1.0, 1.0, f64::NAN, f64::INFINITY).viewport(&base, PLOT);
    assert!(
        near(centre_of(&bad, &base), 935.0),
        "not a number is no move"
    );
}

#[test]
fn the_view_stops_at_the_newest_bar_and_the_free_slots_past_it() {
    let base = base();
    // The chart ends at slot 1,010 (1,000 bars and 10 free). The view's right edge is at 990.
    let bounds = Framing::default().bounds(&base, PLOT);
    assert!(near(bounds.dx.1, 20.0), "{:?}", bounds.dx);
    let edge = framed(1.0, 1.0, 20.0, 0.0).viewport(&base, PLOT);
    let (left, slots) = window_of(&edge, &base);
    assert!(
        (left + slots as f64 - 1_010.0).abs() < 1e-4,
        "right edge at the free slots' end"
    );
}

#[test]
fn zooming_in_leaves_more_room_to_move_toward_the_newest_and_zooming_out_less() {
    let base = base();
    let high = |width| framed(width, 1.0, 0.0, 0.0).bounds(&base, PLOT).dx.1;
    // 110 slots: the right edge, at 990, may go to 1,010.
    assert!(near(high(1.0), 20.0));
    // 55 slots: the same edge allows 47.5, but a quarter of the view is 27.5.
    assert!(near(high(2.0), 27.5));
    // 220 slots centred on 935 already pass the end: they stay where they are.
    assert_eq!(high(0.5), 0.0);
}

#[test]
fn the_view_stops_at_the_oldest_loaded_bar() {
    // Scrolled all the way back: the first slot shown is the first bar.
    let oldest = Base::from_view(
        &HorizontalViewport {
            visible: 100,
            right_offset: 900.0,
        },
        PriceRange::new(100.0, 200.0),
        false,
        1_000,
        0,
        PLOT,
    );
    assert!(near(oldest.start_bar_center, 50.0));
    let bounds = Framing::default().bounds(&oldest, PLOT);
    assert_eq!(bounds.dx.0, 0.0, "nothing older to show");
    assert!(near(bounds.dx.1, 25.0));
    // Zoomed in, there is room to go left until the edge is on bar 0 again.
    let bounds = framed(2.0, 1.0, 0.0, 0.0).bounds(&oldest, PLOT);
    assert!(near(bounds.dx.0, -25.0), "{:?}", bounds.dx);
    let view = framed(2.0, 1.0, -25.0, 0.0).viewport(&oldest, PLOT);
    let (left, _) = window_of(&view, &oldest);
    assert!(left.abs() < 1e-4, "{left}");
}

#[test]
fn a_short_history_cannot_be_dragged_into_more_emptiness() {
    // 60 bars in a plot that holds 100: the left of the plot is empty from the start.
    let young = Base::from_view(
        &HorizontalViewport {
            visible: 100,
            right_offset: 0.0,
        },
        PriceRange::new(100.0, 200.0),
        false,
        60,
        0,
        PLOT,
    );
    assert!(near(young.start_bar_center, 10.0));
    let bounds = Framing::default().bounds(&young, PLOT);
    assert_eq!(bounds.dx, (0.0, 0.0), "already at both ends");
}

#[test]
fn a_chart_scrolled_into_empty_space_keeps_what_it_shows_but_goes_no_further() {
    let past = Base::from_view(
        &HorizontalViewport {
            visible: 100,
            right_offset: -30.0,
        },
        PriceRange::new(100.0, 200.0),
        false,
        1_000,
        10,
        PLOT,
    );
    let bounds = Framing::default().bounds(&past, PLOT);
    assert_eq!(bounds.dx.1, 0.0);
    assert!(bounds.dx.0 < 0.0);
}

// MARK: Rubber band

#[test]
fn resistance_follows_a_little_of_the_pull_and_never_reaches_the_reach() {
    assert_eq!(resist(0.0, 5.0), 0.0);
    assert_eq!(resist(-1.0, 5.0), 0.0);
    assert_eq!(resist(3.0, 0.0), 0.0);
    assert_eq!(resist(f64::NAN, 5.0), 0.0);
    let small = resist(0.01, 5.0);
    assert!(
        small > 0.004 && small < 0.006,
        "about half the pull: {small}"
    );
    let mut last = 0.0;
    for over in [0.5, 1.0, 5.0, 50.0, 1e6] {
        let now = resist(over, 5.0);
        assert!(now > last && now < 5.0 && now < over, "{over}: {now}");
        last = now;
    }
}

#[test]
fn dragging_past_a_limit_gives_way_a_little_and_then_stops_following() {
    let base = base();
    let origin = Framing::default();
    let at_limit = origin.drag(&base, PLOT, 20.0, 0.0);
    assert!(near(at_limit.dx, 20.0));
    let pulled = origin.drag(&base, PLOT, 30.0, 0.0);
    assert!(pulled.dx > 20.0 && pulled.dx < 25.0, "{}", pulled.dx);
    let more = origin.drag(&base, PLOT, 60.0, 0.0);
    assert!(more.dx > pulled.dx);
    let far = origin.drag(&base, PLOT, 1e9, 0.0);
    assert!(far.dx < 20.0 + RUBBER_REACH * 110.0 + 1e-9, "{}", far.dx);
}

#[test]
fn dragging_inside_the_limits_follows_the_pointer_exactly() {
    let base = base();
    let got = Framing::default().drag(&base, PLOT, -12.5, 0.1);
    assert_eq!((got.dx, got.dy), (-12.5, 0.1));
}

#[test]
fn the_vertical_band_works_the_same() {
    let base = base();
    let got = Framing::default().drag(&base, PLOT, 0.0, 0.5);
    assert!(got.dy > 0.25 && got.dy < 0.25 + RUBBER_REACH, "{}", got.dy);
    let got = Framing::default().drag(&base, PLOT, 0.0, -0.5);
    assert!(
        got.dy < -0.25 && got.dy > -0.25 - RUBBER_REACH,
        "{}",
        got.dy
    );
}

#[test]
fn a_drag_keeps_the_dials_and_ignores_what_is_not_a_number() {
    let base = base();
    let from = framed(1.3, 0.8, 4.0, 0.05);
    let got = from.drag(&base, PLOT, f64::NAN, 0.1);
    assert_eq!((got.width, got.height), (1.3, 0.8));
    assert_eq!(got.dx, 4.0, "kept");
    assert_eq!(got.dy, 0.1);
}

#[test]
fn settling_puts_a_pulled_view_back_on_the_limit() {
    let base = base();
    let pulled = Framing::default().drag(&base, PLOT, 40.0, -9.0);
    assert!(pulled.dx > 20.0 && pulled.dy < -0.25);
    let settled = pulled.settle(&base, PLOT);
    assert!(near(settled.dx, 20.0) && settled.dy == -0.25, "{settled:?}");
    assert_eq!(
        settled.settle(&base, PLOT),
        settled,
        "settling again changes nothing"
    );
}

#[test]
fn settling_leaves_a_view_inside_the_limits_alone() {
    let base = base();
    let inside = framed(1.2, 0.9, 7.0, -0.1);
    assert_eq!(inside.settle(&base, PLOT), inside);
}

#[test]
fn settling_after_a_dial_moved_the_limits_brings_the_view_back() {
    let base = base();
    // 10 bars toward the newest is fine at 1×, but not once the view is wider.
    let moved = framed(0.5, 1.0, 10.0, 0.0);
    assert_eq!(moved.settle(&base, PLOT).dx, 0.0);
    // And a framing that was never settled still draws inside the limits plus the band.
    let view = moved.viewport(&base, PLOT);
    assert!((centre_of(&view, &base) - (935.0 + RUBBER_REACH * 110.0)).abs() < 1e-4);
}

// MARK: Dials

#[test]
fn the_dials_are_kept_between_half_and_double() {
    let mut framing = Framing::default();
    framing.set_width(0.1);
    framing.set_height(7.0);
    assert_eq!((framing.width, framing.height), (MIN, MAX));
    framing.set_width(1.28);
    assert_eq!(framing.width, 1.28);
    framing.set_width(f64::NAN);
    framing.set_height(f64::INFINITY);
    assert_eq!(
        (framing.width, framing.height),
        (1.28, MAX),
        "not a number changes nothing"
    );
    assert_eq!(framing.dial(Dial::Width), 1.28);
}

#[test]
fn letting_go_near_one_lands_on_one() {
    for (before, after) in [
        (1.0, 1.0),
        (1.039, 1.0),
        (0.961, 1.0),
        (1.04, 1.0),
        (0.96, 1.0),
        (1.05, 1.05),
        (0.95, 0.95),
        (1.5, 1.5),
        (0.5, 0.5),
    ] {
        let mut framing = framed(before, before, 3.0, 0.1);
        framing.release_snap();
        assert_eq!((framing.width, framing.height), (after, after), "{before}");
        assert_eq!(
            (framing.dx, framing.dy),
            (3.0, 0.1),
            "position is not snapped"
        );
    }
    let mut mixed = framed(1.02, 1.3, 0.0, 0.0);
    mixed.release_snap();
    assert_eq!((mixed.width, mixed.height), (1.0, 1.3));
    assert_eq!(SNAP, 0.04);
}

#[test]
fn keys_step_a_dial_by_the_documented_amounts() {
    let mut framing = Framing::default();
    framing.step(Dial::Width, Step::Fine, 1);
    assert_eq!(framing.width, 1.0 + FINE_STEP);
    framing.step(Dial::Width, Step::Coarse, 1);
    assert_eq!(framing.width, 1.12);
    framing.step(Dial::Height, Step::Coarse, -1);
    assert_eq!(framing.height, 1.0 - COARSE_STEP);
    framing.step(Dial::Height, Step::Fine, -5);
    assert_eq!(
        framing.height, 0.88,
        "only the direction of the sign counts"
    );
    framing.step(Dial::Height, Step::Fine, 0);
    assert_eq!(framing.height, 0.88);
    assert_eq!((FINE_STEP, COARSE_STEP), (0.02, 0.1));
}

#[test]
fn stepping_up_and_down_meets_again_without_drift() {
    let mut framing = Framing::default();
    for _ in 0..17 {
        framing.step(Dial::Width, Step::Fine, 1);
    }
    for _ in 0..17 {
        framing.step(Dial::Width, Step::Fine, -1);
    }
    assert_eq!(framing.width, 1.0);
    assert!(framing.is_default());
}

#[test]
fn keys_stop_at_the_ends() {
    let mut framing = Framing::default();
    for _ in 0..40 {
        framing.step(Dial::Width, Step::Coarse, 1);
        framing.step(Dial::Height, Step::Coarse, -1);
    }
    assert_eq!((framing.width, framing.height), (MAX, MIN));
}

// MARK: Reset

#[test]
fn each_reset_undoes_only_its_part() {
    let all = framed(1.4, 0.7, 6.0, -0.1);

    let mut framing = all;
    framing.reset_dial(Dial::Width);
    assert_eq!(framing, framed(1.0, 0.7, 6.0, -0.1));

    let mut framing = all;
    framing.reset_dial(Dial::Height);
    assert_eq!(framing, framed(1.4, 1.0, 6.0, -0.1));

    let mut framing = all;
    framing.reset_position();
    assert_eq!(
        framing,
        framed(1.4, 0.7, 0.0, 0.0),
        "keeps Width and Height"
    );

    let mut framing = all;
    framing.reset_view();
    assert_eq!(framing, Framing::default());
    assert!(framing.is_default());
}

#[test]
fn a_framing_is_default_only_when_nothing_changed() {
    assert!(Framing::default().is_default());
    for changed in [
        framed(1.02, 1.0, 0.0, 0.0),
        framed(1.0, 0.98, 0.0, 0.0),
        framed(1.0, 1.0, 0.5, 0.0),
        framed(1.0, 1.0, 0.0, -0.01),
    ] {
        assert!(!changed.is_default(), "{changed:?}");
    }
}

// MARK: Robustness

#[test]
fn a_base_with_no_plot_is_drawn_unscaled_instead_of_failing() {
    let mut base = base();
    base.plot = vec2(0.0, 0.0);
    let view = Framing::default().viewport(&base, PLOT);
    assert_eq!(view.horizontal.visible, 100);
    let range = range_of(&view);
    assert!(near(range.minimum, 100.0) && near(range.maximum, 200.0));
}

#[test]
fn a_price_range_a_scale_cannot_use_leaves_the_price_axis_alone() {
    let base = base_over(PriceRange::new(-5.0, 5.0), true);
    let view = Framing::default().viewport(&base, PLOT);
    assert_eq!(view.vertical.manual, None);
}
