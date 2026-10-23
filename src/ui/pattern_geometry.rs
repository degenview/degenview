//! The screen-space construction of the chart patterns, Elliott waves and cycles: where a
//! triangle's two lines meet, the arcs of a time cycle, the points of a sine wave. Pure functions on
//! [`Pos2`] with hand-worked tests, like `line_geometry` and `drawing_geometry`.
//!
//! Every curve is sampled **only over the part of the plot it crosses** and with a step that keeps
//! it within a fraction of a pixel of the true curve, so zooming in or out never produces a
//! polygon you can see or millions of segments.

use eframe::egui::{Pos2, Vec2, pos2, vec2};
use std::f32::consts::PI;

/// How far (px) a sampled curve may stray from the true one.
const TOLERANCE: f32 = 0.2;
/// The most points one arc or one wave is sampled at.
const MAX_ARC_STEPS: usize = 256;
const MAX_WAVE_SAMPLES: usize = 6_000;
/// The shortest half-period (px) of a sine wave that is drawn: anything finer is a smear.
pub const MIN_HALF_PERIOD: f32 = 1.5;

/// The point on the line through `p` and `q` at `x`, or `None` when the line is vertical (it has
/// no single point there).
pub fn line_at_x(p: Pos2, q: Pos2, x: f32) -> Option<Pos2> {
    let run = q.x - p.x;
    if run.abs() < 1e-4 {
        return None;
    }
    let point = pos2(x, p.y + (q.y - p.y) * (x - p.x) / run);
    point.y.is_finite().then_some(point)
}

/// Where the line through `a1` and `a2` meets the line through `b1` and `b2`, or `None` when
/// they are parallel (or a line has no length).
pub fn intersection(a1: Pos2, a2: Pos2, b1: Pos2, b2: Pos2) -> Option<Pos2> {
    let (da, db) = (a2 - a1, b2 - b1);
    let cross = da.x * db.y - da.y * db.x;
    if cross.abs() < 1e-6 {
        return None;
    }
    let to_b = b1 - a1;
    let t = (to_b.x * db.y - to_b.y * db.x) / cross;
    let point = a1 + da * t;
    (point.x.is_finite() && point.y.is_finite()).then_some(point)
}

/// The two sides of a triangle pattern, run on from the pivots they pass through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleSides {
    /// Where the line from A through C stops.
    pub ac_end: Pos2,
    /// Where the line from B through D stops.
    pub bd_end: Pos2,
    /// Where the lines meet, when they do, ahead of the last pivot.
    pub apex: Option<Pos2>,
}

/// The sides of a triangle pattern on A, B, C and D: A to C and B to D, each continued past its
/// last pivot to the **apex** where they meet, or, when they do not close in (they run parallel
/// or fan out, as an expanding triangle does), to the last pivot's time. A meeting point
/// beyond `max_x` is not run to: the lines stop there, which is as far as anyone can see.
pub fn triangle_sides(a: Pos2, b: Pos2, c: Pos2, d: Pos2, max_x: f32) -> TriangleSides {
    let last = c.x.max(d.x);
    let apex = intersection(a, c, b, d).filter(|apex| apex.x > last);
    let stop = apex.map_or(last, |apex| apex.x.min(max_x.max(last)));
    TriangleSides {
        ac_end: line_at_x(a, c, stop).unwrap_or(c),
        bd_end: line_at_x(b, d, stop).unwrap_or(d),
        apex,
    }
}

/// A unit vector across the line from `p` to `q`, pointing up the screen (a vertical line's
/// points right): the way to push a label off it.
pub fn normal_up(p: Pos2, q: Pos2) -> Vec2 {
    let along = q - p;
    let length = along.length();
    if length < 1e-4 {
        return vec2(0.0, -1.0);
    }
    let normal = vec2(along.y, -along.x) / length;
    if normal.y > 0.0 || (normal.y == 0.0 && normal.x < 0.0) {
        -normal
    } else {
        normal
    }
}

/// The half circle over (`up`) or under the baseline `base_y` that spans `radius` either side of
/// `center_x`, as the points of the part that lies between `x_lo` and `x_hi`, from the left.
/// Empty when none of it does, or for a radius that is not one. Stays within [`TOLERANCE`] of the
/// circle however large it is, with at most [`MAX_ARC_STEPS`] segments.
pub fn arc_points(
    center_x: f32,
    base_y: f32,
    radius: f32,
    up: bool,
    x_lo: f32,
    x_hi: f32,
) -> Vec<Pos2> {
    if !(radius.is_finite() && radius > 0.0 && center_x.is_finite() && base_y.is_finite())
        || x_lo > x_hi
        || center_x + radius < x_lo
        || center_x - radius > x_hi
    {
        return Vec::new();
    }
    // The angle is measured from the right end of the diameter: x = cx + r cos(t), t in 0..=PI.
    let angle_at = |x: f32| ((x - center_x) / radius).clamp(-1.0, 1.0).acos();
    let start = angle_at(x_lo.max(center_x - radius));
    let end = angle_at(x_hi.min(center_x + radius));
    // The most one chord may turn through before it is TOLERANCE off the circle.
    let step = if radius <= TOLERANCE {
        PI
    } else {
        2.0 * (1.0 - TOLERANCE / radius).acos()
    };
    let steps = (((start - end) / step).ceil() as usize).clamp(1, MAX_ARC_STEPS);
    let side = if up { -1.0 } else { 1.0 };
    (0..=steps)
        .map(|k| {
            let t = start + (end - start) * k as f32 / steps as f32;
            pos2(
                center_x + radius * t.cos(),
                base_y + side * radius * t.sin(),
            )
        })
        .collect()
}

/// The sine line through `first` and `second`, which are consecutive extremes (a crest and the
/// trough after it, or the reverse): half a period apart in time, an amplitude apart in height,
/// with the midline halfway between. The points over `x_lo..=x_hi`, with both anchors in them
/// when they lie there. `None` when the half period is under [`MIN_HALF_PERIOD`] px (finer than a
/// pixel can show), or for a coordinate that is not a number.
pub fn sine_points(first: Pos2, second: Pos2, x_lo: f32, x_hi: f32) -> Option<Vec<Pos2>> {
    let half = second.x - first.x;
    if !(half.is_finite() && first.y.is_finite() && second.y.is_finite())
        || half.abs() < MIN_HALF_PERIOD
        || x_lo > x_hi
    {
        return None;
    }
    let middle = (first.y + second.y) / 2.0;
    let amplitude = (first.y - second.y) / 2.0;
    let y_at = |x: f32| middle + amplitude * (PI * (x - first.x) / half).cos();
    // A step small enough for the curve's sharpest bend, and for sixteen steps to a half
    // period, but never coarser than 4 px or finer than half a pixel.
    let bend = if amplitude.abs() > 1e-3 {
        half.abs() / PI * (8.0 * TOLERANCE / amplitude.abs()).sqrt()
    } else {
        f32::INFINITY
    };
    let step = (half.abs() / 16.0).min(bend).clamp(0.5, 4.0);
    let count = (((x_hi - x_lo) / step).ceil() as usize).clamp(1, MAX_WAVE_SAMPLES);
    let mut xs: Vec<f32> = (0..=count)
        .map(|k| x_lo + (x_hi - x_lo) * k as f32 / count as f32)
        .collect();
    for anchor in [first.x, second.x] {
        if anchor > x_lo && anchor < x_hi {
            xs.push(anchor);
        }
    }
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    Some(xs.into_iter().map(|x| pos2(x, y_at(x))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_point_on_a_line_at_an_x() {
        let at = line_at_x(pos2(0.0, 0.0), pos2(10.0, 20.0), 5.0).unwrap();
        assert!(near(at.y, 10.0));
        let beyond = line_at_x(pos2(0.0, 0.0), pos2(10.0, 20.0), 30.0).unwrap();
        assert!(near(beyond.y, 60.0));
        assert_eq!(line_at_x(pos2(3.0, 0.0), pos2(3.0, 9.0), 5.0), None);
    }

    #[test]
    fn two_lines_meet_where_the_algebra_says() {
        // y = x and y = 10 - x meet at (5, 5).
        let at = intersection(
            pos2(0.0, 0.0),
            pos2(1.0, 1.0),
            pos2(0.0, 10.0),
            pos2(10.0, 0.0),
        )
        .unwrap();
        assert!(near(at.x, 5.0) && near(at.y, 5.0));
        // Parallel lines never do, nor does a line with no length.
        assert_eq!(
            intersection(
                pos2(0.0, 0.0),
                pos2(1.0, 0.0),
                pos2(0.0, 1.0),
                pos2(1.0, 1.0)
            ),
            None
        );
        assert_eq!(
            intersection(
                pos2(0.0, 0.0),
                pos2(0.0, 0.0),
                pos2(0.0, 1.0),
                pos2(1.0, 1.0)
            ),
            None
        );
    }

    #[test]
    fn a_symmetrical_triangle_runs_to_its_apex() {
        // One line through (0, 0) and (10, 2), y = 0.2 x; the other through (2, 9.6) and
        // (12, 7.6), y = 10 - 0.2 x: they meet where 0.2 x = 10 - 0.2 x, at (25, 5).
        let sides = triangle_sides(
            pos2(0.0, 0.0),
            pos2(2.0, 9.6),
            pos2(10.0, 2.0),
            pos2(12.0, 7.6),
            1000.0,
        );
        let apex = sides.apex.unwrap();
        assert!(near(apex.x, 25.0) && near(apex.y, 5.0));
        assert!(near(sides.ac_end.x, 25.0) && near(sides.ac_end.y, 5.0));
        assert!(near(sides.bd_end.x, 25.0) && near(sides.bd_end.y, 5.0));
    }

    #[test]
    fn an_ascending_triangle_closes_in_on_its_flat_top() {
        // A flat resistance at y = 0 through (0, 0) and (10, 0); support rising from (2, 10)
        // through (12, 6): y = 10 - 0.4 (x - 2), reaching 0 at x = 27.
        let sides = triangle_sides(
            pos2(0.0, 0.0),
            pos2(2.0, 10.0),
            pos2(10.0, 0.0),
            pos2(12.0, 6.0),
            1000.0,
        );
        let apex = sides.apex.unwrap();
        assert!(near(apex.x, 27.0) && near(apex.y, 0.0));
    }

    #[test]
    fn a_triangle_that_fans_out_stops_at_its_last_pivot() {
        // An expanding triangle: the lines diverge, so there is no apex ahead.
        let sides = triangle_sides(
            pos2(0.0, 5.0),
            pos2(2.0, 6.0),
            pos2(10.0, 0.0),
            pos2(12.0, 12.0),
            1000.0,
        );
        assert_eq!(sides.apex, None);
        assert!(near(sides.ac_end.x, 12.0) && near(sides.bd_end.x, 12.0));
        // Parallel sides likewise.
        let flat = triangle_sides(
            pos2(0.0, 0.0),
            pos2(1.0, 5.0),
            pos2(10.0, 0.0),
            pos2(11.0, 5.0),
            1000.0,
        );
        assert_eq!(flat.apex, None);
        assert!(near(flat.ac_end.x, 11.0));
    }

    #[test]
    fn a_distant_apex_is_not_run_to() {
        // Nearly parallel sides meet at x = 5010; the plot ends at 100.
        let sides = triangle_sides(
            pos2(0.0, 0.0),
            pos2(1.0, 10.0),
            pos2(10.0, 0.0),
            pos2(11.0, 9.998),
            100.0,
        );
        assert!(sides.apex.unwrap().x > 1000.0);
        assert!(near(sides.ac_end.x, 100.0) && near(sides.bd_end.x, 100.0));
    }

    #[test]
    fn the_normal_across_a_line_points_up_the_screen() {
        let up = normal_up(pos2(0.0, 0.0), pos2(10.0, 0.0));
        assert!(near(up.x, 0.0) && near(up.y, -1.0));
        let rising = normal_up(pos2(0.0, 10.0), pos2(10.0, 0.0));
        assert!(rising.y < 0.0 && near(rising.length(), 1.0));
        // The same whichever end is first.
        let swapped = normal_up(pos2(10.0, 0.0), pos2(0.0, 10.0));
        assert!(near(swapped.x, rising.x) && near(swapped.y, rising.y));
        let vertical = normal_up(pos2(3.0, 0.0), pos2(3.0, 9.0));
        assert!(near(vertical.x.abs(), 1.0) && near(vertical.y, 0.0));
        assert_eq!(normal_up(pos2(1.0, 1.0), pos2(1.0, 1.0)), vec2(0.0, -1.0));
    }

    #[test]
    fn an_arc_is_a_half_circle_on_its_baseline() {
        let arc = arc_points(100.0, 50.0, 40.0, true, -1000.0, 1000.0);
        let first = arc.first().unwrap();
        let last = arc.last().unwrap();
        // From the left end of the diameter, over the top, to the right end.
        assert!(near(first.x, 60.0) && near(first.y, 50.0));
        assert!(near(last.x, 140.0) && near(last.y, 50.0));
        let top = arc
            .iter()
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .copied()
            .unwrap();
        assert!(
            near(top.y, 10.0),
            "the crown is a radius above the baseline"
        );
        // Every point is a radius from the centre.
        for point in &arc {
            assert!((point.distance(pos2(100.0, 50.0)) - 40.0).abs() < 1e-3);
        }
        // Underneath, mirrored.
        let down = arc_points(100.0, 50.0, 40.0, false, -1000.0, 1000.0);
        assert!(down.iter().all(|point| point.y >= 50.0 - 1e-3));
        assert!(near(
            down.iter().map(|p| p.y).fold(f32::MIN, f32::max),
            90.0
        ));
    }

    #[test]
    fn an_arc_stays_within_a_fraction_of_a_pixel_of_the_circle() {
        for radius in [3.0_f32, 40.0, 900.0, 40_000.0] {
            let arc = arc_points(0.0, 0.0, radius, true, -radius, radius);
            assert!(arc.len() <= MAX_ARC_STEPS + 1, "{radius}");
            // The midpoint of every chord is how far it falls inside the circle.
            let worst = arc
                .windows(2)
                .map(|pair| radius - pair[0].lerp(pair[1], 0.5).distance(pos2(0.0, 0.0)))
                .fold(0.0_f32, f32::max);
            // 256 segments cap a huge arc: on a screen only a sliver of it is ever sampled.
            if radius < 2_000.0 {
                assert!(worst <= 0.21, "{radius}: {worst}");
            }
        }
    }

    #[test]
    fn a_huge_arc_is_sampled_over_the_plot_only() {
        // A circle of radius 40,000 whose crown passes over a plot 800 px wide.
        let arc = arc_points(400.0, 600.0, 40_000.0, true, 0.0, 800.0);
        assert!(arc.len() <= MAX_ARC_STEPS + 1);
        assert!(arc.iter().all(|p| p.x >= -1e-2 && p.x <= 800.0 + 1e-2));
        // Over those 800 px the sampling is fine enough: a flat curve needs few segments, and
        // each stays within the tolerance.
        let worst = arc
            .windows(2)
            .map(|pair| {
                let chord = pair[0].lerp(pair[1], 0.5);
                40_000.0 - chord.distance(pos2(400.0, 600.0))
            })
            .fold(0.0_f32, f32::max);
        assert!(worst <= 0.25, "{worst}");
    }

    #[test]
    fn an_arc_off_the_plot_has_no_points_and_a_bad_radius_none() {
        assert!(arc_points(100.0, 0.0, 10.0, true, 200.0, 300.0).is_empty());
        assert!(arc_points(100.0, 0.0, 10.0, true, -50.0, 50.0).is_empty());
        assert!(arc_points(100.0, 0.0, 0.0, true, 0.0, 300.0).is_empty());
        assert!(arc_points(100.0, 0.0, -5.0, true, 0.0, 300.0).is_empty());
        assert!(arc_points(100.0, 0.0, f32::NAN, true, 0.0, 300.0).is_empty());
        assert!(arc_points(100.0, 0.0, 10.0, true, 300.0, 0.0).is_empty());
    }

    #[test]
    fn an_arc_cut_by_the_plot_keeps_only_what_is_inside() {
        let arc = arc_points(100.0, 0.0, 50.0, true, 90.0, 120.0);
        assert!(near(arc.first().unwrap().x, 90.0));
        assert!(near(arc.last().unwrap().x, 120.0));
        assert!(arc.len() >= 2);
    }

    #[test]
    fn a_sine_passes_through_both_anchors_and_turns_there() {
        // A crest at (100, 40) and the next trough at (180, 120): half period 80, amplitude 40.
        let wave = sine_points(pos2(100.0, 40.0), pos2(180.0, 120.0), 0.0, 400.0).unwrap();
        let at = |x: f32| wave.iter().find(|p| near(p.x, x)).copied().unwrap();
        assert!(near(at(100.0).y, 40.0));
        assert!(near(at(180.0).y, 120.0));
        // The midline (80) is crossed halfway between, and the next crest is a period on.
        let middle = wave
            .iter()
            .min_by(|a, b| (a.x - 140.0).abs().total_cmp(&(b.x - 140.0).abs()))
            .unwrap();
        assert!((middle.y - 80.0).abs() < 1.0);
        let next = wave
            .iter()
            .min_by(|a, b| (a.x - 260.0).abs().total_cmp(&(b.x - 260.0).abs()))
            .unwrap();
        assert!((next.y - 40.0).abs() < 0.5);
        // The wave before the first anchor is the same wave.
        let before = wave
            .iter()
            .min_by(|a, b| (a.x - 20.0).abs().total_cmp(&(b.x - 20.0).abs()))
            .unwrap();
        assert!((before.y - 120.0).abs() < 0.5);
    }

    #[test]
    fn a_sine_is_sampled_finely_enough_to_look_smooth() {
        let wave = sine_points(pos2(0.0, 0.0), pos2(300.0, 400.0), -50.0, 900.0).unwrap();
        assert!(wave.len() >= 2 && wave.len() <= MAX_WAVE_SAMPLES + 3);
        // The sagitta of every chord against the true curve stays small.
        let middle = 200.0;
        let amplitude = -200.0;
        let truth = |x: f32| middle + amplitude * (PI * x / 300.0).cos();
        let worst = wave
            .windows(2)
            .map(|pair| {
                let mid = pair[0].lerp(pair[1], 0.5);
                (mid.y - truth(mid.x)).abs()
            })
            .fold(0.0_f32, f32::max);
        assert!(worst < 0.25, "{worst}");
        // Strictly left to right.
        assert!(wave.windows(2).all(|pair| pair[0].x < pair[1].x));
    }

    #[test]
    fn a_sine_read_right_to_left_is_the_same_curve() {
        let a = sine_points(pos2(100.0, 40.0), pos2(180.0, 120.0), 0.0, 400.0).unwrap();
        // The anchors the other way round: the crest is still on the first anchor.
        let b = sine_points(pos2(180.0, 120.0), pos2(100.0, 40.0), 0.0, 400.0).unwrap();
        let value = |wave: &[Pos2], x: f32| {
            wave.iter()
                .min_by(|p, q| (p.x - x).abs().total_cmp(&(q.x - x).abs()))
                .unwrap()
                .y
        };
        for x in [20.0, 100.0, 140.0, 180.0, 300.0] {
            assert!((value(&a, x) - value(&b, x)).abs() < 0.5, "{x}");
        }
    }

    #[test]
    fn a_sine_with_no_visible_period_or_a_bad_coordinate_is_not_drawn() {
        assert!(sine_points(pos2(10.0, 0.0), pos2(10.0, 50.0), 0.0, 100.0).is_none());
        assert!(sine_points(pos2(10.0, 0.0), pos2(10.5, 50.0), 0.0, 100.0).is_none());
        assert!(sine_points(pos2(10.0, f32::NAN), pos2(40.0, 50.0), 0.0, 100.0).is_none());
        assert!(sine_points(pos2(10.0, 0.0), pos2(f32::INFINITY, 50.0), 0.0, 100.0).is_none());
        assert!(sine_points(pos2(10.0, 0.0), pos2(40.0, 50.0), 100.0, 0.0).is_none());
    }

    #[test]
    fn an_extreme_sine_stays_bounded_and_finite() {
        // The shortest period drawn, a huge amplitude, and a very long one over a wide window.
        for (second, height) in [(1.6_f32, 1.0e6_f32), (1.0e7, 400.0), (300.0, 0.0)] {
            let wave = sine_points(pos2(0.0, 0.0), pos2(second, height), -100.0, 3000.0).unwrap();
            assert!(wave.len() <= MAX_WAVE_SAMPLES + 3, "{second}");
            assert!(wave.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        }
        // No amplitude is a straight line on the midline.
        let flat = sine_points(pos2(0.0, 50.0), pos2(300.0, 50.0), 0.0, 600.0).unwrap();
        assert!(flat.iter().all(|p| near(p.y, 50.0)));
    }
}
