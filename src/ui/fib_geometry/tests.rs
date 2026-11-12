//! The Fibonacci curves, checked against what they are in mathematics rather than against their own
//! output: a ring is everywhere one radius from its centre, a golden spiral is `PHI` times wider a
//! quarter turn on, and a curve far bigger than the plot is only ever sampled where it shows.

use super::*;
use crate::ui::drawing_geometry as geo;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

const PLOT: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 400.0));

fn dist(a: Pos2, b: Pos2) -> f32 {
    a.distance(b)
}

/// The most any chord of the polyline falls short of the circle about `center` of radius `r`:
/// a chord's middle is the furthest point of it from the curve.
fn flattening_error(runs: &[Vec<Pos2>], center: Pos2, r: f32) -> f32 {
    runs.iter()
        .flat_map(|run| run.windows(2))
        .map(|pair| (r - dist(pair[0].lerp(pair[1], 0.5), center)).abs())
        .fold(0.0, f32::max)
}

#[test]
fn a_ring_on_screen_is_one_closed_loop_within_a_quarter_of_a_pixel_of_the_circle() {
    let center = pos2(500.0, 200.0);
    let runs = ellipse(center, 150.0, 150.0, PLOT).unwrap();
    assert_eq!(runs.len(), 1, "one closed loop");
    let ring = &runs[0];
    assert!(
        dist(ring[0], *ring.last().unwrap()) < 1e-3,
        "it closes on itself"
    );
    for point in ring {
        assert!((dist(*point, center) - 150.0).abs() < 1e-3, "{point:?}");
    }
    let error = flattening_error(&runs, center, 150.0);
    assert!(error <= TOLERANCE + 1e-3, "{error}");
}

#[test]
fn an_ellipse_has_its_two_radii() {
    let center = pos2(500.0, 200.0);
    let runs = ellipse(center, 200.0, 80.0, PLOT).unwrap();
    for point in runs.iter().flatten() {
        let (dx, dy) = ((point.x - 500.0) / 200.0, (point.y - 200.0) / 80.0);
        assert!((dx * dx + dy * dy - 1.0).abs() < 1e-4, "{point:?}");
    }
    // Both extremes are reached.
    let xs: Vec<f32> = runs.iter().flatten().map(|p| p.x).collect();
    let ys: Vec<f32> = runs.iter().flatten().map(|p| p.y).collect();
    assert!((xs.iter().copied().fold(f32::MAX, f32::min) - 300.0).abs() < 0.5);
    assert!((ys.iter().copied().fold(f32::MIN, f32::max) - 280.0).abs() < 0.5);
}

#[test]
fn a_circle_far_bigger_than_the_plot_is_sampled_only_where_it_crosses() {
    // Its top is 100 px down the plot; the rest of it is a million pixels away.
    let radius = 1.0e6;
    let center = pos2(500.0, 100.0 + radius);
    let runs = ellipse(center, radius, radius, PLOT).unwrap();
    let count: usize = runs.iter().map(Vec::len).sum();
    assert!(count > 2 && count < 400, "{count} points");
    // Everything kept is near the plot, and flat to the tolerance.
    for point in runs.iter().flatten() {
        assert!(point.y < 1200.0, "{point:?}");
    }
    let error = flattening_error(&runs, center, radius);
    assert!(error <= TOLERANCE + 0.05, "{error}");
    // The topmost point of the stretch is the top of the circle.
    let top = runs.iter().flatten().map(|p| p.y).fold(f32::MAX, f32::min);
    assert!((top - 100.0).abs() < 0.5, "{top}");
}

#[test]
fn a_circle_that_misses_the_plot_has_nothing_to_draw() {
    let runs = ellipse(
        pos2(500.0, 200.0),
        40.0,
        40.0,
        PLOT.translate(vec2(5000.0, 0.0)),
    )
    .unwrap();
    assert!(runs.is_empty());
    // A circle that surrounds the plot without touching it too.
    let runs = ellipse(pos2(500.0, 200.0), 5000.0, 5000.0, PLOT).unwrap();
    assert!(runs.is_empty(), "{} runs", runs.len());
}

#[test]
fn a_bad_ellipse_is_refused_not_drawn() {
    assert!(ellipse(pos2(f32::NAN, 0.0), 10.0, 10.0, PLOT).is_none());
    assert!(ellipse(pos2(0.0, 0.0), f32::INFINITY, 10.0, PLOT).is_none());
    assert!(ellipse(pos2(0.0, 0.0), -3.0, 10.0, PLOT).is_none());
    assert!(ellipse(pos2(0.0, 0.0), 1.0e12, 10.0, PLOT).is_none());
    // Flat ellipses are still drawn, as the line they are.
    let flat = ellipse(pos2(500.0, 200.0), 0.0, 80.0, PLOT).unwrap();
    assert!(flat.iter().flatten().all(|p| (p.x - 500.0).abs() < 1e-3));
}

#[test]
fn an_arc_is_the_part_of_the_circle_between_its_angles() {
    // The half facing left of a circle of 100 about (500, 200): angles a quarter turn either
    // side of straight left.
    let center = pos2(500.0, 200.0);
    let runs = arc(center, 100.0, 100.0, (FRAC_PI_2, PI + FRAC_PI_2), PLOT).unwrap();
    for point in runs.iter().flatten() {
        assert!(point.x <= 500.0 + 1e-3, "{point:?}");
        assert!((dist(*point, center) - 100.0).abs() < 1e-3);
    }
    let first = runs[0][0];
    let last = *runs[0].last().unwrap();
    assert!(dist(first, pos2(500.0, 300.0)) < 1e-3, "{first:?}");
    assert!(dist(last, pos2(500.0, 100.0)) < 1e-3, "{last:?}");
}

#[test]
fn a_golden_spiral_is_phi_wider_every_quarter_turn() {
    let center = pos2(500.0, 200.0);
    let (r0, theta) = (20.0_f32, 0.3_f64);
    for clockwise in [true, false] {
        let runs = golden_spiral(center, r0, theta, clockwise, PLOT).unwrap();
        let sign = if clockwise { 1.0 } else { -1.0 };
        let on_curve = |quarters: f64| {
            let r = f64::from(r0) * PHI.powf(quarters);
            let t = theta + sign * quarters * FRAC_PI_2;
            pos2((500.0 + r * t.cos()) as f32, (200.0 + r * t.sin()) as f32)
        };
        for quarters in [-2.0, -1.0, 0.0, 1.0, 2.0, 3.0] {
            let want = on_curve(quarters);
            let near = runs
                .iter()
                .map(|run| geo::distance_to_polyline(want, run, false))
                .fold(f32::MAX, f32::min);
            assert!(
                near < 0.3,
                "quarter {quarters} (clockwise {clockwise}): {near}"
            );
        }
        // The first point on the spiral through the anchor is the anchor itself.
        let anchor = on_curve(0.0);
        assert!(
            runs.iter()
                .any(|run| geo::distance_to_polyline(anchor, run, false) < 0.3)
        );
    }
}

#[test]
fn a_golden_spiral_winds_outward_and_is_bounded() {
    let center = pos2(500.0, 200.0);
    let runs = golden_spiral(center, 30.0, 0.0, true, PLOT).unwrap();
    // Along the curve the radius only grows (a run is followed in order of winding).
    for run in &runs {
        for pair in run.windows(2) {
            assert!(dist(pair[1], center) >= dist(pair[0], center) - 0.05);
        }
    }
    // However big or small it starts, it is made of a bounded number of points.
    for radius in [0.01_f32, 0.5, 30.0, 4000.0, 2.0e6] {
        let runs = golden_spiral(center, radius, 1.0, true, PLOT).unwrap();
        let count: usize = runs.iter().map(Vec::len).sum();
        assert!(count <= 2 * MAX_POINTS, "{radius}: {count}");
    }
    assert!(golden_spiral(center, 1.0e9, 0.0, true, PLOT).is_none());
    assert!(golden_spiral(center, 0.0, 0.0, true, PLOT).is_none());
    assert!(golden_spiral(pos2(f32::NAN, 0.0), 10.0, 0.0, true, PLOT).is_none());
}

fn area(polygon: &[Pos2]) -> f32 {
    geo::signed_area2(polygon).abs() / 2.0
}

#[test]
fn a_disc_is_the_circles_area_and_is_cut_to_the_plot() {
    let center = pos2(500.0, 200.0);
    let whole = disc(center, 100.0, 100.0, (0.0, 2.0 * PI), None, PLOT);
    let want = std::f32::consts::PI * 100.0 * 100.0;
    assert!(
        (area(&whole) - want).abs() / want < 0.002,
        "{}",
        area(&whole)
    );
    // A disc spilling off the top keeps only the part inside: a circle of 100 centred 50 above
    // the plot's top edge covers the segment below y = 0.
    let cut = disc(
        pos2(500.0, -50.0),
        100.0,
        100.0,
        (0.0, 2.0 * PI),
        None,
        PLOT,
    );
    assert!(cut.iter().all(|p| p.y >= -1e-3));
    // That segment's area: r² acos(d/r) − d √(r² − d²) with d = 50.
    let segment =
        100.0f32.powi(2) * (0.5f32).acos() - 50.0 * (100.0f32.powi(2) - 50.0f32.powi(2)).sqrt();
    assert!(
        (area(&cut) - segment).abs() / segment < 0.005,
        "{} against {segment}",
        area(&cut)
    );
    // One that misses has no polygon.
    assert!(
        disc(
            pos2(5000.0, 5000.0),
            100.0,
            100.0,
            (0.0, 2.0 * PI),
            None,
            PLOT
        )
        .is_empty()
    );
}

#[test]
fn a_pie_slice_and_a_half_disc_are_the_right_part_of_the_disc() {
    let center = pos2(500.0, 200.0);
    // The half facing left, through its centre.
    let half = disc(
        center,
        100.0,
        100.0,
        (FRAC_PI_2, PI + FRAC_PI_2),
        Some(center),
        PLOT,
    );
    assert!(geo::polygon_contains(&half, pos2(450.0, 200.0)));
    assert!(!geo::polygon_contains(&half, pos2(550.0, 200.0)));
    let want = std::f32::consts::PI * 100.0 * 100.0 / 2.0;
    assert!((area(&half) - want).abs() / want < 0.005);
    // A quarter slice is a quarter of the area.
    let quarter = disc(center, 100.0, 100.0, (0.0, FRAC_PI_2), Some(center), PLOT);
    assert!((area(&quarter) - want / 2.0).abs() / (want / 2.0) < 0.005);
}

#[test]
fn an_annulus_fills_the_ring_between_two_circles_and_nothing_else() {
    let center = pos2(500.0, 200.0);
    let outer = disc(center, 150.0, 150.0, (0.0, 2.0 * PI), None, PLOT);
    let inner = disc(center, 80.0, 80.0, (0.0, 2.0 * PI), None, PLOT);
    let ring = annulus(&outer, &inner);
    assert!(geo::polygon_contains(&ring, pos2(500.0 + 115.0, 200.0)));
    assert!(geo::polygon_contains(&ring, pos2(500.0, 200.0 - 120.0)));
    assert!(!geo::polygon_contains(&ring, center), "the hole");
    assert!(
        !geo::polygon_contains(&ring, pos2(500.0 + 40.0, 200.0)),
        "inside the inner ring"
    );
    assert!(
        !geo::polygon_contains(&ring, pos2(500.0 + 170.0, 200.0)),
        "outside the outer"
    );
    // With no inner loop it is the outer disc.
    assert_eq!(annulus(&outer, &[]), outer);
    assert!(annulus(&[], &inner).is_empty());
}

#[test]
fn a_fan_band_covers_the_wedge_between_two_rays_and_no_more() {
    let origin = pos2(100.0, 300.0);
    let a = direction(0.0);
    let b = direction(-(PI as f32) / 3.0);
    let band = fan_band(origin, a, b, PLOT);
    // 30 degrees up from the right, well along: inside. 80 degrees: outside. Below the first ray:
    // outside.
    let inside = origin + direction(-(PI as f32) / 6.0) * 300.0;
    assert!(geo::polygon_contains(&band, inside));
    assert!(!geo::polygon_contains(
        &band,
        origin + direction(-1.4) * 100.0
    ));
    assert!(!geo::polygon_contains(&band, origin + vec2(300.0, 20.0)));
    assert!(band.iter().all(|p| PLOT.expand(0.01).contains(*p)));
    // Rays nearly opposite are still covered across the plot.
    let wide = fan_band(origin, direction(0.1), direction(-3.0), PLOT);
    assert!(geo::polygon_contains(&wide, pos2(300.0, 100.0)));
}

#[test]
fn a_sweep_is_the_short_way_round_and_signed() {
    assert!((sweep(0.0, FRAC_PI_2) - FRAC_PI_2).abs() < 1e-12);
    assert!((sweep(0.0, 3.0 * FRAC_PI_2) + FRAC_PI_2).abs() < 1e-12);
    assert!((sweep(FRAC_PI_4, -FRAC_PI_4) + FRAC_PI_2).abs() < 1e-12);
    assert!((sweep(6.0, 0.1) - (0.1 - 6.0 + 2.0 * PI)).abs() < 1e-12);
    // Half a turn is positive, never minus pi.
    assert!((sweep(0.0, PI) - PI).abs() < 1e-12);
    assert!((sweep(0.0, -PI) - PI).abs() < 1e-12);
}

#[test]
fn sampling_stops_at_the_budget_however_wild_the_curve() {
    // A curve that never settles: every point is a long way from the last.
    let wild = |t: f64| {
        pos2(
            ((t * 1000.0).sin() * 500.0) as f32 + 500.0,
            ((t * 777.0).cos() * 200.0) as f32 + 200.0,
        )
    };
    let runs = sample(&wild, (0.0, 10.0), 512, PLOT, false);
    let count: usize = runs.iter().map(Vec::len).sum();
    assert!(count <= 3 * MAX_POINTS, "{count}");
    // And a curve with nothing finite in it makes nothing.
    let nothing = |_: f64| pos2(f32::NAN, f32::NAN);
    assert!(sample(&nothing, (0.0, 1.0), 64, PLOT, false).is_empty());
    assert!(sample(&wild, (1.0, 1.0), 64, PLOT, false).is_empty());
}
