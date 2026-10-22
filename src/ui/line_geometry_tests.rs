use super::*;

fn p(x: f32, y: f32) -> Pos2 {
    pos2(x, y)
}

fn plot() -> Rect {
    Rect::from_min_max(p(0.0, 0.0), p(100.0, 100.0))
}

fn near(a: Pos2, b: Pos2) -> bool {
    a.distance(b) < 1e-3
}

// MARK: Lines without an end

#[test]
fn a_line_extended_both_ways_runs_edge_to_edge() {
    let (from, to) = line_in_rect(p(10.0, 50.0), p(20.0, 50.0), plot(), true, true).unwrap();
    assert!(
        near(from, p(0.0, 50.0)) && near(to, p(100.0, 50.0)),
        "{from:?} {to:?}"
    );
    // A vertical one, and a diagonal one through the corners.
    let (top, bottom) = line_in_rect(p(30.0, 10.0), p(30.0, 20.0), plot(), true, true).unwrap();
    assert!(near(top, p(30.0, 0.0)) && near(bottom, p(30.0, 100.0)));
    let (a, b) = line_in_rect(p(-50.0, -50.0), p(-40.0, -40.0), plot(), true, true).unwrap();
    assert!(near(a, p(0.0, 0.0)) && near(b, p(100.0, 100.0)));
}

#[test]
fn a_ray_starts_at_its_origin_and_runs_one_way() {
    let (from, to) = line_in_rect(p(50.0, 50.0), p(60.0, 50.0), plot(), false, true).unwrap();
    assert!(near(from, p(50.0, 50.0)) && near(to, p(100.0, 50.0)));
    // Pointing the other way it runs the other way: the direction is origin to second point.
    let (from, to) = line_in_rect(p(50.0, 50.0), p(40.0, 50.0), plot(), false, true).unwrap();
    assert!(near(from, p(50.0, 50.0)) && near(to, p(0.0, 50.0)));
    // An origin outside the plot starts at the edge the ray enters by.
    let (from, to) = line_in_rect(p(-50.0, 20.0), p(-40.0, 20.0), plot(), false, true).unwrap();
    assert!(near(from, p(0.0, 20.0)) && near(to, p(100.0, 20.0)));
    // Pointing away from the plot it never arrives.
    assert!(line_in_rect(p(150.0, 50.0), p(160.0, 50.0), plot(), false, true).is_none());
}

#[test]
fn a_segment_is_clipped_and_one_that_misses_is_nothing() {
    let (from, to) = line_in_rect(p(10.0, 10.0), p(20.0, 30.0), plot(), false, false).unwrap();
    assert!(near(from, p(10.0, 10.0)) && near(to, p(20.0, 30.0)));
    let (from, to) = line_in_rect(p(-100.0, 50.0), p(50.0, 50.0), plot(), false, false).unwrap();
    assert!(near(from, p(0.0, 50.0)) && near(to, p(50.0, 50.0)));
    assert!(line_in_rect(p(-100.0, 50.0), p(-10.0, 50.0), plot(), false, false).is_none());
    // An infinite line beside the plot, vertical or not, misses it.
    assert!(line_in_rect(p(-10.0, 200.0), p(-10.0, 300.0), plot(), true, true).is_none());
    assert!(line_in_rect(p(-10.0, 0.0), p(-5.0, -50.0), plot(), true, true).is_none());
}

#[test]
fn degenerate_and_extreme_lines_stay_exact_and_finite() {
    assert!(line_in_rect(p(5.0, 5.0), p(5.0, 5.0), plot(), true, true).is_none());
    assert!(line_in_rect(p(f32::NAN, 5.0), p(5.0, 6.0), plot(), true, true).is_none());
    assert!(line_in_rect(p(5.0, f32::INFINITY), p(5.0, 6.0), plot(), true, true).is_none());
    // Anchors a million pixels away still land on the plot's edge.
    let (a, b) = line_in_rect(p(0.0, 0.0), p(1.0e6, 1.0e6), plot(), true, true).unwrap();
    assert!(
        near(a, p(0.0, 0.0)) && near(b, p(100.0, 100.0)),
        "{a:?} {b:?}"
    );
    // Nearly vertical across the whole height.
    let (top, bottom) =
        line_in_rect(p(50.0, -1.0e6), p(50.001, 1.0e6), plot(), true, true).unwrap();
    assert!(top.y.abs() < 1e-3 && (bottom.y - 100.0).abs() < 1e-3);
    assert!((top.x - 50.0).abs() < 1e-2);
    // Nearly horizontal.
    let (left, right) =
        line_in_rect(p(-1.0e6, 40.0), p(1.0e6, 40.001), plot(), true, true).unwrap();
    assert!(left.x.abs() < 1e-3 && (right.x - 100.0).abs() < 1e-3);
    // Tiny separation between the anchors still gives a direction.
    assert!(line_in_rect(p(10.0, 10.0), p(10.0, 10.01), plot(), true, true).is_some());
}

#[test]
fn the_reach_spans_the_plot_and_no_more() {
    assert!((reach(p(0.0, 0.0), plot()) - 141.421_36).abs() < 1e-3);
    assert!((reach(p(50.0, 50.0), plot()) - 70.710_68).abs() < 1e-3);
    assert!(
        (reach(p(300.0, 0.0), plot()) - (300.0f32.powi(2) + 100.0f32.powi(2)).sqrt()).abs() < 1e-3
    );
}

#[test]
fn a_polygon_is_cut_to_the_plot() {
    let inside = [p(10.0, 10.0), p(20.0, 10.0), p(20.0, 20.0), p(10.0, 20.0)];
    assert_eq!(clip_polygon(&inside, plot()), inside.to_vec());
    assert!(clip_polygon(&[p(200.0, 10.0), p(300.0, 10.0), p(250.0, 50.0)], plot()).is_empty());
    // Half in: the part inside is a 50 x 20 rectangle.
    let half = [p(50.0, 10.0), p(150.0, 10.0), p(150.0, 30.0), p(50.0, 30.0)];
    let clipped = clip_polygon(&half, plot());
    let area = (crate::ui::drawing_geometry::signed_area2(&clipped) / 2.0).abs();
    assert!((area - 1000.0).abs() < 1e-2, "area {area}");
    assert!(
        clipped
            .iter()
            .all(|point| plot().expand(1e-3).contains(*point))
    );
    // A band that covers the whole plot is the plot.
    let cover = [
        p(-50.0, -50.0),
        p(150.0, -50.0),
        p(150.0, 150.0),
        p(-50.0, 150.0),
    ];
    let area =
        (crate::ui::drawing_geometry::signed_area2(&clip_polygon(&cover, plot())) / 2.0).abs();
    assert!((area - 10_000.0).abs() < 1e-2);
    assert!(clip_polygon(&inside[..2], plot()).is_empty());
}

// MARK: Text

#[test]
fn text_follows_the_line_and_is_never_upside_down() {
    let angle = |from, to| text_angle(from, to).unwrap().to_degrees();
    assert!((angle(p(0.0, 0.0), p(10.0, 0.0))).abs() < 1e-4);
    assert!(
        (angle(p(0.0, 0.0), p(10.0, -10.0)) + 45.0).abs() < 1e-3,
        "rises: turns up"
    );
    assert!(
        (angle(p(0.0, 0.0), p(10.0, 10.0)) - 45.0).abs() < 1e-3,
        "falls: turns down"
    );
    // The same line drawn from the other end reads the same way.
    assert!((angle(p(10.0, -10.0), p(0.0, 0.0)) + 45.0).abs() < 1e-3);
    assert!((angle(p(10.0, 10.0), p(0.0, 0.0)) - 45.0).abs() < 1e-3);
    // Vertical either way reads bottom to top.
    assert!((angle(p(0.0, 0.0), p(0.0, 10.0)) - 90.0).abs() < 1e-3);
    assert!((angle(p(0.0, 0.0), p(0.0, -10.0)) - 90.0).abs() < 1e-3);
    for degrees in (0..360).step_by(7) {
        let radians = (degrees as f32).to_radians();
        let to = p(radians.cos() * 10.0, radians.sin() * 10.0);
        let turned = text_angle(p(0.0, 0.0), to).unwrap();
        assert!(
            turned > -FRAC_PI_2 - 1e-5 && turned <= FRAC_PI_2 + 1e-5,
            "{degrees}"
        );
    }
    assert!(text_angle(p(1.0, 1.0), p(1.0, 1.0)).is_none());
}

// MARK: Channels

#[test]
fn the_second_edge_keeps_the_first_edges_times_and_moves_by_a_price_offset() {
    // The edge a→b is y = x; c sits 5 above it at x = 5, so the offset is -5 (screen up).
    assert_eq!(
        channel_offset(p(0.0, 0.0), p(10.0, 10.0), p(5.0, 0.0)),
        vec2(0.0, -5.0)
    );
    // Wherever c is along the second edge, the offset is the same.
    assert_eq!(
        channel_offset(p(0.0, 0.0), p(10.0, 10.0), p(8.0, 3.0)),
        vec2(0.0, -5.0)
    );
    // A reversed edge gives the same offset.
    assert_eq!(
        channel_offset(p(10.0, 10.0), p(0.0, 0.0), p(5.0, 0.0)),
        vec2(0.0, -5.0)
    );
    // A vertical first edge is offset sideways by the perpendicular distance.
    assert_eq!(
        channel_offset(p(0.0, 0.0), p(0.0, 10.0), p(3.0, 4.0)),
        vec2(3.0, 0.0)
    );
    assert_eq!(
        channel_offset(p(1.0, 1.0), p(1.0, 1.0), p(3.0, 4.0)),
        Vec2::ZERO
    );
}

#[test]
fn a_disjoint_edge_runs_parallel_to_the_first_to_the_given_time() {
    let end = disjoint_end(p(0.0, 0.0), p(10.0, 5.0), p(20.0, 0.0), 40.0);
    assert!(near(end, p(40.0, 10.0)), "{end:?}");
    // Parallel: the same slope as a→b.
    let slope = |a: Pos2, b: Pos2| (b.y - a.y) / (b.x - a.x);
    assert!((slope(p(20.0, 0.0), end) - slope(p(0.0, 0.0), p(10.0, 5.0))).abs() < 1e-6);
    // Ending before it starts is the same line, run backwards.
    let back = disjoint_end(p(0.0, 0.0), p(10.0, 5.0), p(20.0, 0.0), 0.0);
    assert!(near(back, p(0.0, -10.0)));
    assert_eq!(y_at(p(0.0, 0.0), p(10.0, 5.0), 4.0), Some(2.0));
    assert_eq!(y_at(p(3.0, 0.0), p(3.0, 5.0), 4.0), None);
}

// MARK: Pitchforks

// The fixture: P1 (0, 0), P2 (10, 10), P3 (20, 0), worked by hand.
const P1: Pos2 = Pos2::new(0.0, 0.0);
const P2: Pos2 = Pos2::new(10.0, 10.0);
const P3: Pos2 = Pos2::new(20.0, 0.0);

fn build(variant: PitchforkVariant) -> Fork {
    fork(variant, P1, P2, P3).unwrap()
}

#[test]
fn the_standard_median_runs_from_p1_through_the_middle_of_p2_and_p3() {
    let standard = build(PitchforkVariant::Standard);
    assert_eq!(standard.origin, p(0.0, 0.0));
    assert_eq!(standard.middle, p(15.0, 5.0));
    assert_eq!(standard.handle, vec2(15.0, 5.0));
    assert_eq!(standard.half, vec2(5.0, -5.0));
    assert_eq!(standard.start, 1.0);
    // The handle ends on the base, and the median goes on to FORK_LENGTH handles.
    assert_eq!(standard.origin + standard.handle, standard.middle);
    assert_eq!(standard.median_end(), p(30.0, 10.0));
}

#[test]
fn level_one_passes_through_p2_and_p3_and_is_parallel_to_the_median() {
    let standard = build(PitchforkVariant::Standard);
    assert_eq!(standard.level_start(1.0, 1.0), P3);
    assert_eq!(standard.level_start(1.0, -1.0), P2);
    assert_eq!(standard.level_start(0.5, 1.0), p(17.5, 2.5));
    // Level 0 is the median's own height on the base.
    assert_eq!(standard.level_start(0.0, 1.0), standard.middle);
    // Every line runs along the handle: the same direction as the median.
    for (ratio, side) in [(0.5, 1.0), (0.5, -1.0), (1.0, 1.0), (1.0, -1.0), (2.0, 1.0)] {
        let line = standard.level_end(ratio, side) - standard.level_start(ratio, side);
        assert!(
            line.x * standard.handle.y - line.y * standard.handle.x == 0.0,
            "parallel"
        );
        assert!(line.dot(standard.handle) > 0.0, "same way");
        assert_eq!(line, standard.handle, "one handle long past the base");
    }
}

#[test]
fn schiff_starts_at_p1s_time_at_the_price_halfway_to_p2() {
    let schiff = build(PitchforkVariant::Schiff);
    assert_eq!(schiff.origin, p(0.0, 5.0));
    // From there to the middle of P2 and P3, which is level.
    assert_eq!(schiff.handle, vec2(15.0, 0.0));
    assert_eq!(schiff.middle, p(15.0, 5.0), "the base is the standard one");
    assert_eq!(schiff.half, build(PitchforkVariant::Standard).half);
}

#[test]
fn modified_schiff_starts_halfway_between_p1_and_p2_in_time_and_price() {
    let modified = build(PitchforkVariant::ModifiedSchiff);
    assert_eq!(modified.origin, p(5.0, 5.0));
    assert_eq!(modified.handle, vec2(10.0, 0.0));
    assert_eq!(modified.middle, p(15.0, 5.0));
}

#[test]
fn the_inside_fork_runs_from_the_middle_of_p1_p2_to_p3_with_its_levels_across_p1_p2() {
    let inside = build(PitchforkVariant::Inside);
    assert_eq!(inside.origin, p(5.0, 5.0));
    assert_eq!(inside.origin + inside.handle, P3, "the median reaches P3");
    assert_eq!(inside.start, 0.0);
    assert_eq!(inside.level_start(1.0, 1.0), P2);
    assert_eq!(inside.level_start(1.0, -1.0), P1);
    assert_eq!(inside.level_start(0.0, 1.0), inside.origin);
}

#[test]
fn the_four_variants_are_distinguishable() {
    let handles: Vec<(Pos2, Vec2)> = PitchforkVariant::ALL
        .iter()
        .map(|variant| {
            let made = build(*variant);
            (made.origin, made.handle)
        })
        .collect();
    for (index, one) in handles.iter().enumerate() {
        for other in &handles[index + 1..] {
            assert_ne!(one, other);
        }
    }
    // Only the standard one starts at a pivot, only the inside one ends at one.
    assert_eq!(handles[0].0, P1);
    assert_eq!(
        build(PitchforkVariant::Inside).origin + build(PitchforkVariant::Inside).handle,
        P3
    );
}

#[test]
fn an_upside_down_fork_mirrors_and_a_degenerate_one_has_no_handle() {
    // Mirror the fixture vertically: every construction mirrors with it.
    let flip = |q: Pos2| p(q.x, -q.y);
    for variant in PitchforkVariant::ALL {
        let up = fork(variant, P1, P2, P3).unwrap();
        let down = fork(variant, flip(P1), flip(P2), flip(P3)).unwrap();
        assert_eq!(down.origin, flip(up.origin), "{variant:?}");
        assert_eq!(down.handle, vec2(up.handle.x, -up.handle.y), "{variant:?}");
        assert_eq!(
            down.level_start(1.0, 1.0),
            flip(up.level_start(1.0, 1.0)),
            "{variant:?}"
        );
    }
    for variant in PitchforkVariant::ALL {
        assert!(fork(variant, P1, P1, P1).is_none(), "{variant:?}");
        assert!(fork(variant, P1, P2, p(f32::NAN, 0.0)).is_none());
    }
    // P2 and P3 together leave a base of no width but a handle that still points somewhere.
    let narrow = fork(PitchforkVariant::Standard, P1, P2, P2).unwrap();
    assert_eq!(narrow.half, Vec2::ZERO);
    assert_eq!(narrow.level_start(1.0, 1.0), narrow.middle);
}

#[test]
fn a_fork_is_unchanged_in_shape_by_zooming_the_axes() {
    // Scale x by 3 and y by 0.5 (an affine change of the chart's zoom): the fork's points are
    // the scaled points of the original, so ratios and parallels survive.
    let scale = |q: Pos2| p(q.x * 3.0, q.y * 0.5);
    for variant in PitchforkVariant::ALL {
        let before = fork(variant, P1, P2, P3).unwrap();
        let after = fork(variant, scale(P1), scale(P2), scale(P3)).unwrap();
        assert!(near(after.origin, scale(before.origin)), "{variant:?}");
        assert!(near(
            after.level_start(0.618, -1.0),
            scale(before.level_start(0.618, -1.0))
        ));
        assert!(near(
            after.level_end(1.5, 1.0),
            scale(before.level_end(1.5, 1.0))
        ));
        assert!(near(after.median_end(), scale(before.median_end())));
    }
}
