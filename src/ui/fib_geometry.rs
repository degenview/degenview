//! Pure screen-space geometry for the Fibonacci and Gann curves: ellipses, arcs, the golden spiral
//! and the fans between rays. Like `drawing_geometry` and `line_geometry` it takes and returns
//! `Pos2`s, so it runs and is tested without a window.
//!
//! **Curves are sampled where they can be seen.** A circle a million pixels across is drawn as the
//! few hundred pixels of it that cross the plot, flattened to a quarter of a pixel, and the rest is
//! not sampled at all; a circle on screen is flattened the same way, so a ring never shows its
//! corners at any zoom. Every sampler is bounded in the points it makes, whatever the anchors are.

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::ui::drawing_geometry as geo;
use crate::ui::line_geometry as lines;

/// How far (px) a flattened curve may stray from the true curve.
pub const TOLERANCE: f32 = 0.25;
/// The most points one sampled curve makes.
pub const MAX_POINTS: usize = 6_000;
/// How deep a stretch of a curve is split.
const MAX_DEPTH: u32 = 24;
/// The most pieces a curve starts out as.
const MAX_INITIAL: usize = 512;
/// A curve whose size is beyond this (px) is not drawn: it is off the end of the world.
const MAX_SIZE: f32 = 5.0e6;

/// The golden ratio, the growth of the Fibonacci spiral every quarter turn.
pub const PHI: f64 = std::f64::consts::GOLDEN_RATIO;

struct Sampler<'a> {
    point_at: &'a dyn Fn(f64) -> Pos2,
    rect: Rect,
    keep_hidden: bool,
    runs: Vec<Vec<Pos2>>,
    budget: usize,
}

impl Sampler<'_> {
    fn segment(&mut self, from: Pos2, to: Pos2) {
        match self.runs.last_mut() {
            Some(run) if run.last() == Some(&from) => run.push(to),
            _ => self.runs.push(vec![from, to]),
        }
        self.budget = self.budget.saturating_sub(1);
    }

    fn refine(&mut self, (ta, tb): (f64, f64), (pa, pb): (Pos2, Pos2), depth: u32) {
        let middle = (ta + tb) / 2.0;
        let pm = (self.point_at)(middle);
        if !geo::is_finite(pm) {
            return;
        }
        let chord = pa.lerp(pb, 0.5);
        let sagitta = pm.distance(chord);
        // Where the stretch of curve can be: its chord and its middle, and a margin for how far it
        // bows, which a smooth curve between them does not exceed by much.
        let region = Rect::from_two_pos(pa, pb)
            .union(Rect::from_two_pos(pm, pm))
            .expand(2.0 * sagitta + TOLERANCE);
        let visible = region.intersects(self.rect);
        if !visible && !self.keep_hidden {
            return;
        }
        if visible && sagitta > TOLERANCE && depth < MAX_DEPTH && self.budget > 2 {
            self.refine((ta, middle), (pa, pm), depth + 1);
            self.refine((middle, tb), (pm, pb), depth + 1);
        } else {
            self.segment(pa, pb);
        }
    }
}

/// The curve `point_at` over the parameter range `from..=to`, as the polylines of it that can be
/// seen in `rect`, each flattened to within [`TOLERANCE`] of the curve. `initial` is how many
/// pieces the range is first cut into (enough that none turns more than a little); `keep_hidden`
/// keeps the stretches out of `rect` as coarse segments, for a polygon that is then clipped,
/// instead of leaving them out.
pub fn sample(
    point_at: &dyn Fn(f64) -> Pos2,
    (from, to): (f64, f64),
    initial: usize,
    rect: Rect,
    keep_hidden: bool,
) -> Vec<Vec<Pos2>> {
    let initial = initial.clamp(8, MAX_INITIAL);
    if !(from.is_finite() && to.is_finite()) || from == to {
        return Vec::new();
    }
    let mut sampler = Sampler {
        point_at,
        rect,
        keep_hidden,
        runs: Vec::new(),
        budget: MAX_POINTS,
    };
    let step = (to - from) / initial as f64;
    let mut before = (from, point_at(from));
    for index in 1..=initial {
        let t = if index == initial {
            to
        } else {
            from + step * index as f64
        };
        let here = (t, point_at(t));
        if geo::is_finite(before.1) && geo::is_finite(here.1) {
            sampler.refine((before.0, here.0), (before.1, here.1), 0);
        }
        before = here;
    }
    sampler.runs
}

// MARK: Ellipses

/// The point at angle `t` (radians, clockwise on screen from the right) of the ellipse.
fn on_ellipse(center: Pos2, rx: f32, ry: f32, t: f64) -> Pos2 {
    pos2(
        (f64::from(center.x) + f64::from(rx) * t.cos()) as f32,
        (f64::from(center.y) + f64::from(ry) * t.sin()) as f32,
    )
}

/// How many pieces an ellipse of these radii starts as: more for a larger one, so a piece never
/// turns far.
fn initial_pieces(rx: f32, ry: f32) -> usize {
    let size = f64::from(rx.max(ry));
    (64.0 + size.sqrt() * 2.0).min(MAX_INITIAL as f64) as usize
}

/// The ellipse about `center` with radii `rx`, `ry`, as the polylines of it inside `rect`. A
/// whole ellipse that is on screen is one polyline that closes on itself. `None` when it is off
/// the end of the world.
#[cfg(test)]
pub fn ellipse(center: Pos2, rx: f32, ry: f32, rect: Rect) -> Option<Vec<Vec<Pos2>>> {
    arc(center, rx, ry, (0.0, TAU), rect)
}

/// The part of the ellipse about `center` between the angles `range`, as the polylines of it
/// inside `rect`.
pub fn arc(
    center: Pos2,
    rx: f32,
    ry: f32,
    range: (f64, f64),
    rect: Rect,
) -> Option<Vec<Vec<Pos2>>> {
    if !valid_ellipse(center, rx, ry) {
        return None;
    }
    let point = |t: f64| on_ellipse(center, rx, ry, t);
    Some(sample(
        &point,
        range,
        initial_pieces(rx, ry),
        rect.expand(1.0),
        false,
    ))
}

fn valid_ellipse(center: Pos2, rx: f32, ry: f32) -> bool {
    geo::is_finite(center)
        && rx.is_finite()
        && ry.is_finite()
        && rx >= 0.0
        && ry >= 0.0
        && rx.max(ry) <= MAX_SIZE
}

/// The convex polygon of the part of the ellipse's disc between `range` (a whole disc when it
/// spans a turn), joined to `apex` when one is given (a pie slice, or a half disc through the
/// centre), cut to `rect`. Empty when it misses the rectangle.
pub fn disc(
    center: Pos2,
    rx: f32,
    ry: f32,
    range: (f64, f64),
    apex: Option<Pos2>,
    rect: Rect,
) -> Vec<Pos2> {
    if !valid_ellipse(center, rx, ry) {
        return Vec::new();
    }
    let point = |t: f64| on_ellipse(center, rx, ry, t);
    let runs = sample(
        &point,
        range,
        initial_pieces(rx, ry),
        rect.expand(1.0),
        true,
    );
    let mut polygon: Vec<Pos2> = Vec::new();
    // The runs join end to start (hidden stretches are kept), so there is one.
    for run in &runs {
        for point in run {
            if polygon.last() != Some(point) {
                polygon.push(*point);
            }
        }
    }
    if let Some(apex) = apex {
        polygon.push(apex);
    }
    lines::clip_polygon(&polygon, rect)
}

/// The region between two nested convex loops as one polygon for the even-odd fill: the outer
/// loop, a bridge to the inner one, the inner loop and the bridge back, which the fill cancels.
/// Just the outer loop when the inner one is empty.
pub fn annulus(outer: &[Pos2], inner: &[Pos2]) -> Vec<Pos2> {
    if outer.len() < 3 {
        return Vec::new();
    }
    let mut polygon = outer.to_vec();
    if inner.len() >= 3 {
        polygon.push(outer[0]);
        polygon.extend_from_slice(inner);
        polygon.push(inner[0]);
    }
    polygon
}

// MARK: Spiral

/// The golden spiral about `center` through the point at distance `radius` and angle `theta` (a
/// quarter turn grows it by [`PHI`]), as the polylines of it inside `rect`. `clockwise` is the
/// way it winds outward on screen. It is followed in to a quarter of a pixel and out to past the
/// plot, so it is drawn at every zoom; `None` when it is off the end of the world.
pub fn golden_spiral(
    center: Pos2,
    radius: f32,
    theta: f64,
    clockwise: bool,
    rect: Rect,
) -> Option<Vec<Vec<Pos2>>> {
    if !geo::is_finite(center) || !(radius.is_finite() && radius > 1e-3 && radius <= MAX_SIZE) {
        return None;
    }
    let r0 = f64::from(radius);
    let reach = f64::from(lines::reach(center, rect)).max(r0) * 2.0;
    // The angle at which the radius is `r`: a quarter turn per factor of PHI.
    let turns = |r: f64| FRAC_PI_2 * (r / r0).ln() / PHI.ln();
    let (inward, outward) = (turns(f64::from(TOLERANCE)), turns(reach));
    let direction = if clockwise { 1.0 } else { -1.0 };
    let point = |angle: f64| {
        let r = r0 * PHI.powf(angle / FRAC_PI_2);
        let t = theta + direction * angle;
        pos2(
            (f64::from(center.x) + r * t.cos()) as f32,
            (f64::from(center.y) + r * t.sin()) as f32,
        )
    };
    // Cut so that no piece turns more than a sixth of a turn.
    let pieces = (((outward - inward) / (PI / 6.0)).ceil() as usize).clamp(16, MAX_INITIAL);
    Some(sample(
        &point,
        (inward, outward),
        pieces,
        rect.expand(1.0),
        false,
    ))
}

// MARK: Fans

/// The polygon of the wedge from `origin` between the directions `a` and `b` (unit vectors, less
/// than a half turn apart), long enough to cover `rect`, cut to it.
pub fn fan_band(origin: Pos2, a: Vec2, b: Vec2, rect: Rect) -> Vec<Pos2> {
    let reach = lines::reach(origin, rect) * 2.0 + 1.0;
    let middle = (a + b).normalized();
    // The wedge's far edge is a chord; a bisector point past it keeps a wide wedge covering the
    // rectangle too.
    let half_cos = a.dot(middle).max(0.2);
    lines::clip_polygon(
        &[
            origin,
            origin + a * reach,
            origin + middle * (reach / half_cos),
            origin + b * reach,
        ],
        rect,
    )
}

/// A unit vector at `angle` radians (clockwise on screen from the right).
pub fn direction(angle: f32) -> Vec2 {
    vec2(angle.cos(), angle.sin())
}

/// The signed angle (radians, in `(-π, π]`) from direction `from` to direction `to`: positive is
/// clockwise on screen.
pub fn sweep(from: f64, to: f64) -> f64 {
    let mut delta = (to - from) % TAU;
    if delta > PI {
        delta -= TAU;
    } else if delta <= -PI {
        delta += TAU;
    }
    delta
}

#[cfg(test)]
#[path = "fib_geometry_tests.rs"]
mod tests;
