use super::*;

/// The signed perpendicular offset a rotated rectangle's third point implies.
fn rotated_rect_offset(a: Pos2, b: Pos2, width: Pos2) -> f32 {
    let edge = b - a;
    let length = edge.length();
    if length < 1e-3 {
        return 0.0;
    }
    let direction = edge / length;
    (width - a).dot(vec2(-direction.y, direction.x))
}

fn p(x: f32, y: f32) -> Pos2 {
    pos2(x, y)
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

/// How many of the triangles cover each sample point, over a grid.
fn coverage(triangles: &[[Pos2; 3]], area: Rect, step: f32) -> Vec<u32> {
    let mut counts = Vec::new();
    let mut y = area.top() + 0.123;
    while y < area.bottom() {
        let mut x = area.left() + 0.071;
        while x < area.right() {
            let point = p(x, y);
            counts.push(
                triangles
                    .iter()
                    .filter(|triangle| polygon_contains(&triangle[..], point))
                    .count() as u32,
            );
            x += step;
        }
        y += step;
    }
    counts
}

fn triangle_area(triangles: &[[Pos2; 3]]) -> f32 {
    triangles
        .iter()
        .map(|t| signed_area2(&t[..]).abs() / 2.0)
        .sum()
}

#[test]
fn distances_to_segments_and_polylines() {
    let (a, b) = (p(0.0, 0.0), p(10.0, 0.0));
    assert_eq!(distance_to_segment(p(5.0, 3.0), a, b), 3.0);
    assert_eq!(distance_to_segment(p(-4.0, 3.0), a, b), 5.0);
    // A zero-length segment is a point.
    assert_eq!(distance_to_segment(p(3.0, 4.0), a, a), 5.0);
    let square = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
    assert_eq!(distance_to_polyline(p(0.0, 5.0), &square, true), 0.0);
    assert_eq!(distance_to_polyline(p(0.0, 5.0), &square, false), 5.0);
    assert_eq!(distance_to_polyline(p(1.0, 1.0), &[], false), f32::INFINITY);
    assert_eq!(distance_to_polyline(p(3.0, 4.0), &[a], false), 5.0);
}

#[test]
fn bounds_cover_every_point() {
    assert!(bounds(&[]).is_none());
    let rect = bounds(&[p(3.0, -1.0), p(-2.0, 8.0), p(5.0, 4.0)]).unwrap();
    assert_eq!((rect.min, rect.max), (p(-2.0, -1.0), p(5.0, 8.0)));
}

#[test]
fn bezier_endpoints_are_exact_and_midpoints_match_the_formula() {
    let (p0, p1, p2) = (p(0.0, 0.0), p(50.0, 100.0), p(100.0, 0.0));
    assert_eq!(quad_point(p0, p1, p2, 0.0), p0);
    assert_eq!(quad_point(p0, p1, p2, 1.0), p2);
    assert_eq!(quad_point(p0, p1, p2, 0.5), p(50.0, 50.0));
    let (c1, c2) = (p(0.0, 100.0), p(100.0, 100.0));
    let cubic = cubic_point(p0, c1, c2, p2, 0.5);
    assert!(close(cubic.x, 50.0, 1e-4) && close(cubic.y, 75.0, 1e-4));
}

#[test]
fn flattened_curves_stay_within_the_flatness_of_the_true_curve() {
    let (p0, p1, p2) = (p(0.0, 0.0), p(300.0, 600.0), p(600.0, 0.0));
    let polyline = flatten_quad(p0, p1, p2);
    assert_eq!(polyline.first(), Some(&p0));
    assert_eq!(polyline.last(), Some(&p2));
    for i in 0..=200 {
        let on_curve = quad_point(p0, p1, p2, i as f32 / 200.0);
        let distance = distance_to_polyline(on_curve, &polyline, false);
        assert!(distance <= FLATNESS + 0.01, "{distance}");
    }
    let (c1, c2) = (p(0.0, 500.0), p(600.0, -500.0));
    let polyline = flatten_cubic(p0, c1, c2, p2);
    for i in 0..=200 {
        let on_curve = cubic_point(p0, c1, c2, p2, i as f32 / 200.0);
        assert!(distance_to_polyline(on_curve, &polyline, false) <= FLATNESS + 0.01);
    }
}

#[test]
fn a_flat_curve_needs_few_segments_and_a_huge_one_is_capped() {
    let flat = flatten_quad(p(0.0, 0.0), p(5.0, 0.0), p(10.0, 0.0));
    assert!(flat.len() <= 4, "{}", flat.len());
    let huge = flatten_cubic(p(0.0, 0.0), p(1e6, 1e6), p(-1e6, 1e6), p(10.0, 0.0));
    assert!(huge.len() <= MAX_CURVE_SEGMENTS + 1);
}

#[test]
fn curves_with_bad_numbers_degrade_to_their_chord() {
    let bad = p(f32::NAN, 0.0);
    assert_eq!(flatten_quad(p(0.0, 0.0), bad, p(1.0, 1.0)).len(), 2);
    assert_eq!(flatten_cubic(p(0.0, 0.0), bad, bad, p(1.0, 1.0)).len(), 2);
    assert!(arc_through(bad, p(1.0, 1.0), p(2.0, 0.0)).is_none());
}

#[test]
fn an_arc_passes_through_its_three_points() {
    let (start, end, through) = (p(0.0, 0.0), p(100.0, 0.0), p(50.0, -30.0));
    let arc = arc_through(start, end, through).unwrap();
    for point in [start, end, through] {
        assert!(close(arc.center.distance(point), arc.radius, 0.01));
    }
    let polyline = arc.flatten();
    assert!(polyline.first().unwrap().distance(start) < 0.01);
    assert!(polyline.last().unwrap().distance(end) < 0.01);
    // The bulge passes the third point, on its own side of the chord.
    assert!(distance_to_polyline(through, &polyline, false) < FLATNESS + 0.01);
    assert!(polyline.iter().all(|point| point.y <= 0.01));
}

#[test]
fn an_arc_flips_with_the_side_of_the_third_point() {
    let (start, end) = (p(0.0, 0.0), p(100.0, 0.0));
    let up = arc_through(start, end, p(50.0, -30.0)).unwrap().flatten();
    let down = arc_through(start, end, p(50.0, 30.0)).unwrap().flatten();
    assert!(up.iter().all(|point| point.y <= 0.01));
    assert!(down.iter().all(|point| point.y >= -0.01));
}

#[test]
fn an_arc_can_be_more_than_a_half_circle() {
    // The third point far beyond the chord: the long way round.
    let arc = arc_through(p(0.0, 0.0), p(100.0, 0.0), p(50.0, -200.0)).unwrap();
    assert!(arc.sweep.abs() > PI);
    let polyline = arc.flatten();
    assert!(distance_to_polyline(p(50.0, -200.0), &polyline, false) < 0.5);
}

#[test]
fn collinear_coincident_and_enormous_arcs_are_refused() {
    assert!(arc_through(p(0.0, 0.0), p(100.0, 0.0), p(50.0, 0.0)).is_none());
    assert!(arc_through(p(0.0, 0.0), p(100.0, 0.0), p(50.0, 1e-5)).is_none());
    assert!(arc_through(p(5.0, 5.0), p(5.0, 5.0), p(5.0, 5.0)).is_none());
    assert!(arc_through(p(0.0, 0.0), p(1.0, 0.0), p(0.5, 1e-9)).is_none());
    // Nearly straight but real: a very large, still finite circle.
    let nearly = arc_through(p(0.0, 0.0), p(1000.0, 0.0), p(500.0, -2.0));
    if let Some(arc) = nearly {
        assert!(arc.radius.is_finite());
        assert!(arc.flatten().len() <= MAX_CURVE_SEGMENTS * 2 + 1);
    }
}

#[test]
fn an_ellipse_polygon_is_round_enough_and_a_circle_when_the_radii_match() {
    let polygon = ellipse_polygon(p(100.0, 50.0), 80.0, 30.0);
    for point in &polygon {
        let normalized = ((point.x - 100.0) / 80.0).powi(2) + ((point.y - 50.0) / 30.0).powi(2);
        assert!(close(normalized, 1.0, 1e-4));
    }
    let area = signed_area2(&polygon).abs() / 2.0;
    assert!(
        close(area, PI * 80.0 * 30.0, PI * 80.0 * 30.0 * 0.01),
        "{area}"
    );
    let circle = ellipse_polygon(p(0.0, 0.0), 40.0, 40.0);
    assert!(
        circle
            .iter()
            .all(|point| close(point.to_vec2().length(), 40.0, 1e-3))
    );
    assert!(ellipse_polygon(p(f32::NAN, 0.0), 1.0, 1.0).is_empty());
    assert!(ellipse_polygon(p(0.0, 0.0), f32::INFINITY, 1.0).is_empty());
    // A tiny one is still a valid polygon.
    assert!(ellipse_polygon(p(0.0, 0.0), 0.0, 0.0).len() >= 12);
}

#[test]
fn a_rotated_rectangle_has_right_angles_and_the_requested_width() {
    let (a, b) = (p(10.0, 10.0), p(110.0, 70.0));
    let corners = rotated_rect(a, b, p(40.0, 120.0)).unwrap();
    assert_eq!((corners[0], corners[1]), (a, b));
    let edge = |i: usize| corners[(i + 1) % 4] - corners[i];
    for i in 0..4 {
        assert!(edge(i).dot(edge((i + 1) % 4)).abs() < 1e-2, "corner {i}");
    }
    // Opposite edges are equal, so the fourth corner is derived, not free.
    assert!(close(edge(0).length(), edge(2).length(), 1e-3));
    assert!(close(edge(1).length(), edge(3).length(), 1e-3));
    let offset = rotated_rect_offset(a, b, p(40.0, 120.0));
    assert!(close(edge(1).length(), offset.abs(), 1e-2));
    // The third point on the other side flips the rectangle.
    let flipped = rotated_rect(a, b, p(80.0, -50.0)).unwrap();
    assert!(rotated_rect_offset(a, b, p(80.0, -50.0)) * offset < 0.0);
    assert!(flipped[2] != corners[2]);
}

#[test]
fn rotated_rectangle_degenerates_safely() {
    assert!(rotated_rect(p(5.0, 5.0), p(5.0, 5.0), p(9.0, 9.0)).is_none());
    assert!(rotated_rect(p(0.0, 0.0), p(1e-5, 0.0), p(9.0, 9.0)).is_none());
    assert!(rotated_rect(p(0.0, 0.0), p(10.0, f32::NAN), p(9.0, 9.0)).is_none());
    // The width point on the edge's own line: a flat rectangle with no area.
    let flat = rotated_rect(p(0.0, 0.0), p(10.0, 0.0), p(5.0, 0.0)).unwrap();
    assert!(signed_area2(&flat).abs() < 1e-4);
    assert_eq!(
        rotated_rect_offset(p(1.0, 1.0), p(1.0, 1.0), p(9.0, 9.0)),
        0.0
    );
}

#[test]
fn shift_constraints_make_squares_and_forty_five_degree_lines() {
    let anchor = p(10.0, 10.0);
    let square = constrained_corner(anchor, p(60.0, -20.0), true);
    assert_eq!(square, p(60.0, -40.0));
    assert_eq!(
        constrained_corner(anchor, p(60.0, -20.0), false),
        p(60.0, -20.0)
    );
    let snapped = snap_angle(anchor, p(100.0, 18.0));
    assert!(close(snapped.y, 10.0, 1e-3), "{snapped:?}");
    assert!(close(
        anchor.distance(snapped),
        anchor.distance(p(100.0, 18.0)),
        1e-3
    ));
    let diagonal = snap_angle(anchor, p(60.0, 70.0));
    assert!(close(diagonal.x - 10.0, diagonal.y - 10.0, 1e-2));
    assert_eq!(snap_angle(anchor, anchor), anchor);
}

#[test]
fn an_arrowhead_points_the_way_the_line_runs() {
    let head = arrowhead(p(0.0, 0.0), p(100.0, 0.0), 12.0, 5.0).unwrap();
    assert_eq!(head[0], p(100.0, 0.0));
    assert!(close(head[1].x, 88.0, 1e-4) && close(head[2].x, 88.0, 1e-4));
    assert!(close(head[1].y, -head[2].y, 1e-4));
    let diagonal = arrowhead(p(0.0, 0.0), p(30.0, 40.0), 10.0, 4.0).unwrap();
    // The base midpoint lies back along the line.
    let base = (diagonal[1].to_vec2() + diagonal[2].to_vec2()) / 2.0;
    assert!(close(base.length(), 50.0 - 10.0, 1e-3));
    assert!(arrowhead(p(5.0, 5.0), p(5.0, 5.0), 10.0, 4.0).is_none());
    assert!(arrowhead(p(0.0, 0.0), p(f32::NAN, 0.0), 10.0, 4.0).is_none());
}

#[test]
fn an_arrow_marker_is_a_tapered_shaft_and_a_head() {
    let polygon = arrow_marker(p(0.0, 0.0), p(200.0, 0.0), 4.0).unwrap();
    assert_eq!(polygon.len(), 7);
    assert_eq!(polygon[3], p(200.0, 0.0));
    // Symmetric about the axis, wider at the head than at the tail.
    assert!(close(polygon[0].y, -polygon[6].y, 1e-4));
    assert!(polygon[2].y.abs() > polygon[1].y.abs());
    assert!(polygon[1].y.abs() > polygon[0].y.abs());
    // A short marker keeps its head inside its length.
    let short = arrow_marker(p(0.0, 0.0), p(10.0, 0.0), 4.0).unwrap();
    assert!(
        short
            .iter()
            .all(|point| point.x >= -0.01 && point.x <= 10.01)
    );
    assert!(arrow_marker(p(0.0, 0.0), p(0.2, 0.0), 4.0).is_none());
}

#[test]
fn arrow_marks_keep_a_fixed_size_and_point_at_the_anchor() {
    let up = arrow_mark(p(100.0, 100.0), true);
    let down = arrow_mark(p(100.0, 100.0), false);
    assert_eq!(up[0], p(100.0, 100.0 + MARK_GAP));
    assert_eq!(down[0], p(100.0, 100.0 - MARK_GAP));
    // The up arrow lies below the anchor, the down arrow above.
    assert!(up.iter().all(|point| point.y >= 100.0));
    assert!(down.iter().all(|point| point.y <= 100.0));
    let height = up.iter().map(|point| point.y).fold(f32::MIN, f32::max) - up[0].y;
    assert!(close(height, MARK_HEAD_HEIGHT + MARK_STEM_HEIGHT, 1e-4));
    let width = up.iter().map(|point| point.x).fold(f32::MIN, f32::max)
        - up.iter().map(|point| point.x).fold(f32::MAX, f32::min);
    assert!(close(width, MARK_HEAD_WIDTH, 1e-4));
    // Moving the anchor only translates it: the size never depends on the chart.
    let moved = arrow_mark(p(400.0, -50.0), true);
    assert_eq!(moved[1] - moved[0], up[1] - up[0]);
}

#[test]
fn polygon_containment_is_even_odd() {
    let square = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
    assert!(polygon_contains(&square, p(5.0, 5.0)));
    assert!(!polygon_contains(&square, p(15.0, 5.0)));
    assert!(!polygon_contains(&square, p(5.0, -1.0)));
    // A concave "C": the notch is outside.
    let c = [
        p(0.0, 0.0),
        p(30.0, 0.0),
        p(30.0, 10.0),
        p(10.0, 10.0),
        p(10.0, 20.0),
        p(30.0, 20.0),
        p(30.0, 30.0),
        p(0.0, 30.0),
    ];
    assert!(polygon_contains(&c, p(5.0, 15.0)));
    assert!(!polygon_contains(&c, p(20.0, 15.0)));
    assert!(polygon_contains(&c, p(20.0, 5.0)));
    // Degenerate input.
    assert!(!polygon_contains(&[p(0.0, 0.0), p(1.0, 1.0)], p(0.5, 0.5)));
}

#[test]
fn a_five_pointed_star_has_a_hole_under_even_odd() {
    let star: Vec<Pos2> = (0..5)
        .map(|i| {
            let angle = TAU * (i * 2) as f32 / 5.0 - PI / 2.0;
            p(100.0 + 100.0 * angle.cos(), 100.0 + 100.0 * angle.sin())
        })
        .collect();
    assert!(
        !polygon_contains(&star, p(100.0, 100.0)),
        "the middle is a hole"
    );
    assert!(polygon_contains(&star, p(100.0, 20.0)), "a tip is inside");
}

#[test]
fn the_fill_of_simple_polygons_has_the_right_area() {
    let square = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
    assert!(close(trapezoid_area(&fill_even_odd(&square)), 100.0, 1e-3));
    let triangle = [p(0.0, 0.0), p(20.0, 0.0), p(0.0, 10.0)];
    assert!(close(
        trapezoid_area(&fill_even_odd(&triangle)),
        100.0,
        1e-3
    ));
    // Concave: the C is the 30x30 square minus a 20x10 notch.
    let c = [
        p(0.0, 0.0),
        p(30.0, 0.0),
        p(30.0, 10.0),
        p(10.0, 10.0),
        p(10.0, 20.0),
        p(30.0, 20.0),
        p(30.0, 30.0),
        p(0.0, 30.0),
    ];
    assert!(close(
        trapezoid_area(&fill_even_odd(&c)),
        900.0 - 200.0,
        1e-2
    ));
    // Winding does not matter.
    let mut reversed = c;
    reversed.reverse();
    assert!(close(
        trapezoid_area(&fill_even_odd(&reversed)),
        700.0,
        1e-2
    ));
}

#[test]
fn a_bow_tie_fills_both_wings_and_nothing_between() {
    // Two triangles meeting at the centre: (0,0)-(20,20) cross (0,20)-(20,0).
    let bow = [p(0.0, 0.0), p(20.0, 20.0), p(20.0, 0.0), p(0.0, 20.0)];
    let area = trapezoid_area(&fill_even_odd(&bow));
    // Left and right wings: two triangles of base 20 and height 10.
    assert!(close(area, 200.0, 0.5), "{area}");
    assert!(polygon_contains(&bow, p(2.0, 10.0)));
    assert!(!polygon_contains(&bow, p(10.0, 2.0)));
}

#[test]
fn a_star_fill_leaves_its_centre_empty() {
    let star: Vec<Pos2> = (0..5)
        .map(|i| {
            let angle = TAU * (i * 2) as f32 / 5.0 - PI / 2.0;
            p(100.0 * angle.cos(), 100.0 * angle.sin())
        })
        .collect();
    let trapezoids = fill_even_odd(&star);
    let covered = |point: Pos2| {
        trapezoids.iter().any(|t| {
            point.y >= t[0].y && point.y <= t[3].y && {
                let f = (point.y - t[0].y) / (t[3].y - t[0].y).max(1e-6);
                let left = t[0].x + (t[3].x - t[0].x) * f;
                let right = t[1].x + (t[2].x - t[1].x) * f;
                point.x >= left && point.x <= right
            }
        })
    };
    assert!(!covered(p(0.0, 0.0)));
    assert!(covered(p(0.0, -90.0)));
    assert!(covered(p(80.0, -25.0)));
    assert!(!covered(p(60.0, 45.0)), "between two arms");
}

#[test]
fn polygon_fill_survives_degenerate_input() {
    assert!(fill_even_odd(&[]).is_empty());
    assert!(fill_even_odd(&[p(0.0, 0.0), p(1.0, 1.0)]).is_empty());
    // All on one horizontal line.
    assert!(fill_even_odd(&[p(0.0, 5.0), p(5.0, 5.0), p(9.0, 5.0)]).is_empty());
    assert!(fill_even_odd(&[p(0.0, 0.0), p(f32::NAN, 1.0), p(1.0, 0.0)]).is_empty());
    // Coincident points and a spike do not panic or hang.
    let spiky = [
        p(0.0, 0.0),
        p(0.0, 0.0),
        p(10.0, 0.0),
        p(10.0, 0.0),
        p(5.0, 50.0),
    ];
    let area = trapezoid_area(&fill_even_odd(&spiky));
    assert!(close(area, 250.0, 0.5), "{area}");
}

#[test]
fn a_dense_self_intersecting_polygon_terminates() {
    let zigzag: Vec<Pos2> = (0..400)
        .map(|i| {
            let x = (i * 37 % 211) as f32;
            let y = (i * 91 % 197) as f32;
            p(x, y)
        })
        .collect();
    let trapezoids = fill_even_odd(&zigzag);
    assert!(trapezoids.len() < 2_000_000);
    assert!(trapezoid_area(&trapezoids).is_finite());
}

#[test]
fn clipping_cuts_a_segment_to_the_rectangle() {
    let rect = Rect::from_min_max(p(0.0, 0.0), p(100.0, 100.0));
    let (a, b) = clip_segment(p(-50.0, 50.0), p(150.0, 50.0), rect).unwrap();
    assert_eq!((a, b), (p(0.0, 50.0), p(100.0, 50.0)));
    assert_eq!(
        clip_segment(p(10.0, 10.0), p(20.0, 20.0), rect),
        Some((p(10.0, 10.0), p(20.0, 20.0)))
    );
    assert!(clip_segment(p(-10.0, -10.0), p(-1.0, 200.0), rect).is_none());
    assert!(clip_segment(p(0.0, f32::NAN), p(1.0, 1.0), rect).is_none());
    // A point segment outside misses; one inside stays.
    assert!(clip_segment(p(200.0, 200.0), p(200.0, 200.0), rect).is_none());
    assert!(clip_segment(p(50.0, 50.0), p(50.0, 50.0), rect).is_some());
}

#[test]
fn a_line_extends_to_the_plot_edge_without_moving_its_anchors() {
    let rect = Rect::from_min_max(p(0.0, 0.0), p(200.0, 100.0));
    let (a, b) = (p(50.0, 50.0), p(100.0, 50.0));
    assert_eq!(
        extend_to_rect(a, b, rect, true, true),
        (p(0.0, 50.0), p(200.0, 50.0))
    );
    assert_eq!(extend_to_rect(a, b, rect, false, true), (a, p(200.0, 50.0)));
    assert_eq!(extend_to_rect(a, b, rect, true, false), (p(0.0, 50.0), b));
    assert_eq!(extend_to_rect(a, b, rect, false, false), (a, b));
    // A sloped line stops at the top and bottom edges as well.
    let (c, d) = extend_to_rect(p(100.0, 40.0), p(110.0, 50.0), rect, true, true);
    assert!(close(c.x - c.y, 60.0, 0.1) && close(d.x - d.y, 60.0, 0.1));
    assert!(c.x >= 0.0 && d.x <= 200.0 && c.y >= 0.0 && d.y <= 100.0);
    // Zero length and non-finite input come back unchanged.
    assert_eq!(extend_to_rect(a, a, rect, true, true), (a, a));
    let bad = p(f32::INFINITY, 0.0);
    assert_eq!(extend_to_rect(a, bad, rect, true, true), (a, bad));
}

#[test]
fn simplification_keeps_shape_and_ends() {
    let straight: Vec<Pos2> = (0..100)
        .map(|i| p(i as f32, 0.2 * (i % 2) as f32))
        .collect();
    let simplified = simplify(&straight, 0.5);
    assert_eq!(simplified, vec![p(0.0, 0.0), p(99.0, 0.2)]);
    // A corner survives.
    let corner: Vec<Pos2> = (0..=10)
        .map(|i| p(i as f32 * 10.0, 0.0))
        .chain((1..=10).map(|i| p(100.0, i as f32 * 10.0)))
        .collect();
    let simplified = simplify(&corner, 0.5);
    assert_eq!(
        simplified,
        vec![p(0.0, 0.0), p(100.0, 0.0), p(100.0, 100.0)]
    );
    // Nothing to do for short or unthresholded input.
    assert_eq!(simplify(&corner[..2], 1.0).len(), 2);
    assert_eq!(simplify(&corner, 0.0).len(), corner.len());
    assert_eq!(simplify(&corner, f32::NAN).len(), corner.len());
}

#[test]
fn simplifying_a_hundred_thousand_points_is_fast_and_safe() {
    let points: Vec<Pos2> = (0..100_000)
        .map(|i| p(i as f32 * 0.01, (i as f32 * 0.003).sin() * 50.0))
        .collect();
    let simplified = simplify(&points, 0.25);
    assert!(simplified.len() < points.len() / 10);
    assert_eq!(simplified.first(), points.first());
    assert_eq!(simplified.last(), points.last());
}

#[test]
fn smoothing_keeps_the_ends_and_cuts_corners() {
    let corner = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)];
    let smoothed = smooth(&corner, 2, 1000);
    assert_eq!(smoothed.first(), corner.first());
    assert_eq!(smoothed.last(), corner.last());
    assert!(smoothed.len() > corner.len());
    assert!(distance_to_polyline(p(10.0, 0.0), &smoothed, false) > 0.5);
    // The limit stops growth; short input is untouched.
    assert_eq!(smooth(&corner, 5, 4).len(), 3);
    assert_eq!(smooth(&corner[..2], 3, 100).len(), 2);
}

#[test]
fn a_straight_stroke_covers_its_length_times_its_width_plus_two_caps() {
    let geometry = stroke_geometry(&[p(0.0, 0.0), p(100.0, 0.0)], 10.0).unwrap();
    let area = triangle_area(&geometry.triangles);
    let expected = 100.0 * 10.0 + PI * 25.0;
    assert!(
        close(area, expected, expected * 0.01),
        "{area} vs {expected}"
    );
}

#[test]
fn a_stroke_never_covers_a_point_twice() {
    // The whole point of the single-mesh stroke: translucent paint must not double up where
    // two parts of one stroke meet.
    let paths: Vec<Vec<Pos2>> = vec![
        vec![p(20.0, 20.0), p(120.0, 20.0), p(120.0, 120.0)],
        vec![
            p(20.0, 20.0),
            p(120.0, 20.0),
            p(120.0, 120.0),
            p(20.0, 120.0),
        ],
        vec![p(20.0, 60.0), p(70.0, 20.0), p(120.0, 60.0), p(170.0, 20.0)],
        (0..40)
            .map(|i| {
                let angle = i as f32 * 0.12;
                p(
                    100.0 + angle.cos() * (20.0 + i as f32),
                    100.0 + angle.sin() * (20.0 + i as f32),
                )
            })
            .collect(),
    ];
    for path in paths {
        let geometry = stroke_geometry(&path, 12.0).unwrap();
        let area = bounds(&path).unwrap().expand(20.0);
        let counts = coverage(&geometry.triangles, area, 1.7);
        let doubled = counts.iter().filter(|count| **count > 1).count();
        let covered = counts.iter().filter(|count| **count >= 1).count();
        assert!(covered > 100);
        // A few grid points sit on a shared edge; nothing more than that may double.
        assert!(
            doubled * 200 <= covered,
            "{doubled} doubled of {covered} covered for {path:?}"
        );
    }
}

#[test]
fn a_stroke_covers_its_centre_line_and_stays_within_its_half_width() {
    let path = [p(0.0, 0.0), p(80.0, 0.0), p(80.0, 80.0)];
    let geometry = stroke_geometry(&path, 10.0).unwrap();
    for i in 0..=20 {
        let on_line = p(i as f32 * 4.0, 0.0);
        assert!(
            geometry
                .triangles
                .iter()
                .any(|t| polygon_contains(&t[..], on_line)
                    || t.iter().any(|v| v.distance(on_line) < 0.5)),
            "{on_line:?}"
        );
    }
    for triangle in &geometry.triangles {
        for vertex in triangle {
            assert!(
                distance_to_polyline(*vertex, &path, false) <= 5.0 + 0.01,
                "{vertex:?}"
            );
        }
    }
    assert!(geometry.outline.len() > 6);
}

#[test]
fn a_dot_and_a_doubled_back_stroke_still_draw() {
    let dot = stroke_geometry(&[p(10.0, 10.0)], 8.0).unwrap();
    assert!(dot.triangles.len() >= 12);
    let area = triangle_area(&dot.triangles);
    assert!(close(area, PI * 16.0, PI * 16.0 * 0.05), "{area}");
    // Repeated points collapse to a dot.
    let repeated = stroke_geometry(&[p(1.0, 1.0), p(1.0, 1.0), p(1.0, 1.0)], 4.0).unwrap();
    assert!(!repeated.triangles.is_empty());
    // A path that turns back on itself must not panic or yield non-finite vertices.
    let back = stroke_geometry(&[p(0.0, 0.0), p(50.0, 0.0), p(0.0, 0.0)], 6.0).unwrap();
    assert!(
        back.triangles
            .iter()
            .flatten()
            .all(|vertex| is_finite(*vertex))
    );
}

#[test]
fn bad_strokes_are_refused() {
    assert!(stroke_geometry(&[], 4.0).is_none());
    assert!(stroke_geometry(&[p(0.0, 0.0), p(1.0, 1.0)], 0.0).is_none());
    assert!(stroke_geometry(&[p(0.0, 0.0), p(1.0, 1.0)], f32::NAN).is_none());
    assert!(stroke_geometry(&[p(0.0, 0.0), p(f32::NAN, 1.0)], 4.0).is_none());
    assert!(stroke_mesh(&[p(0.0, 0.0), p(1e9, 1.0)], 4.0, Color32::RED, 1.0).is_none());
}

#[test]
fn the_stroke_mesh_is_one_colour_inside_and_fades_to_nothing_outside() {
    let color = Color32::from_rgba_unmultiplied(242, 54, 69, 51);
    let mesh = stroke_mesh(
        &[p(0.0, 0.0), p(60.0, 0.0), p(60.0, 60.0)],
        10.0,
        color,
        1.0,
    )
    .unwrap();
    assert!(mesh.indices.len().is_multiple_of(3));
    assert!(
        mesh.indices
            .iter()
            .all(|index| (*index as usize) < mesh.vertices.len())
    );
    let opaque = mesh
        .vertices
        .iter()
        .filter(|vertex| vertex.color == color)
        .count();
    let faded = mesh
        .vertices
        .iter()
        .filter(|vertex| vertex.color == Color32::TRANSPARENT)
        .count();
    assert!(opaque > 0 && faded > 0);
    assert_eq!(
        opaque + faded,
        mesh.vertices.len(),
        "only two colours: one alpha throughout"
    );
    // Without a feather there is no fade at all.
    let hard = stroke_mesh(&[p(0.0, 0.0), p(60.0, 0.0)], 10.0, color, 0.0).unwrap();
    assert!(hard.vertices.iter().all(|vertex| vertex.color == color));
}

#[test]
fn the_fill_mesh_is_a_single_colour_and_empty_for_nothing() {
    let color = Color32::from_rgba_unmultiplied(0, 0, 255, 51);
    let mesh = fill_mesh(
        &[p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)],
        color,
    );
    assert!(!mesh.is_empty());
    assert!(mesh.vertices.iter().all(|vertex| vertex.color == color));
    assert!(fill_mesh(&[], color).is_empty());
}

#[test]
fn a_stroke_mask_is_full_inside_ramps_at_the_edge_and_empty_outside() {
    let mask = rasterize_stroke(&[p(10.0, 20.0), p(60.0, 20.0)], 9.0, 1.0).unwrap();
    let rect = mask.rect();
    assert!(rect.contains(p(10.0, 20.0)) && rect.contains(p(60.0, 20.0)));
    let at = |x: f32, y: f32| {
        let (column, row) = (
            ((x - mask.origin.x) * mask.pixels_per_point) as usize,
            ((y - mask.origin.y) * mask.pixels_per_point) as usize,
        );
        // Beyond the map nothing is covered.
        if column >= mask.size[0] || row >= mask.size[1] {
            0
        } else {
            mask.at(column, row)
        }
    };
    assert_eq!(at(30.0, 20.0), 255, "the middle");
    assert_eq!(at(30.0, 23.5), 255, "within the half width");
    assert!(at(30.0, 24.7) > 0 && at(30.0, 24.7) < 255, "the edge ramps");
    assert_eq!(at(30.0, 30.0), 0, "outside");
    // The round caps reach half a width past the ends, and no further.
    assert!(at(6.0, 20.0) > 200);
    assert_eq!(at(0.0, 20.0), 0);
}

#[test]
fn overlapping_parts_of_one_stroke_never_cover_a_pixel_more_than_once() {
    // A hairpin: the path doubles back over itself, closer than its own width.
    let path = [p(10.0, 10.0), p(80.0, 10.0), p(80.0, 14.0), p(10.0, 14.0)];
    let mask = rasterize_stroke(&path, 12.0, 1.0).unwrap();
    // Where the two runs overlap the coverage is still one full unit, not two.
    let column = ((40.0 - mask.origin.x) as usize).min(mask.size[0] - 1);
    let row = ((12.0 - mask.origin.y) as usize).min(mask.size[1] - 1);
    assert_eq!(mask.at(column, row), 255);
    // And a self-crossing stroke has the same silhouette as the union of its parts.
    let cross = rasterize_stroke(
        &[p(10.0, 10.0), p(70.0, 70.0), p(10.0, 70.0), p(70.0, 10.0)],
        8.0,
        2.0,
    )
    .unwrap();
    let full = cross.coverage.iter().filter(|byte| **byte == 255).count();
    assert!(full > 1000);
}

#[test]
fn a_stroke_mask_follows_the_pixel_grid_at_any_scale() {
    for ppp in [1.0f32, 1.5, 2.0, 3.0] {
        let mask = rasterize_stroke(&[p(3.3, 4.7), p(20.1, 9.9)], 6.0, ppp).unwrap();
        // The origin is a whole number of physical pixels, so the map lands on the grid.
        assert!((mask.origin.x * ppp - (mask.origin.x * ppp).round()).abs() < 1e-3);
        assert!((mask.origin.y * ppp - (mask.origin.y * ppp).round()).abs() < 1e-3);
        let rect = mask.rect();
        assert!(
            rect.contains(p(3.3, 4.7)) && rect.contains(p(20.1, 9.9)),
            "{ppp}"
        );
    }
    // A dot is a disc.
    let dot = rasterize_stroke(&[p(10.0, 10.0)], 8.0, 1.0).unwrap();
    let covered = dot.coverage.iter().filter(|byte| **byte > 0).count();
    assert!((40..80).contains(&covered), "{covered}");
}

#[test]
fn a_stroke_mask_refuses_bad_or_enormous_input() {
    assert!(rasterize_stroke(&[], 4.0, 1.0).is_none());
    assert!(rasterize_stroke(&[p(0.0, 0.0), p(1.0, 1.0)], 0.0, 1.0).is_none());
    assert!(rasterize_stroke(&[p(0.0, 0.0), p(1.0, 1.0)], f32::NAN, 1.0).is_none());
    assert!(rasterize_stroke(&[p(0.0, 0.0), p(1.0, 1.0)], 4.0, 0.0).is_none());
    assert!(rasterize_stroke(&[p(0.0, 0.0), p(f32::NAN, 1.0)], 4.0, 1.0).is_none());
    // Too big to hold: the caller draws the mesh instead.
    assert!(rasterize_stroke(&[p(0.0, 0.0), p(9000.0, 9000.0)], 20.0, 2.0).is_none());
}

#[test]
fn a_dense_long_stroke_masks_in_reasonable_time() {
    let points: Vec<Pos2> = (0..4000)
        .map(|i| {
            p(
                20.0 + i as f32 * 0.35,
                200.0 + (i as f32 * 0.05).sin() * 120.0,
            )
        })
        .collect();
    let mask = rasterize_stroke(&points, 20.0, 2.0).unwrap();
    assert!(mask.coverage.contains(&255));
}
