//! Pure screen-space geometry for the lines, channels and pitchforks: clipping a line that has
//! no end to the plot, the pitchfork constructions, the channel edges and the text angle. Like
//! `drawing_geometry` it takes and returns `Pos2`s, so it runs and is tested without a window.
//!
//! Everything that extends past a drawing's anchors is computed here against the plot rectangle
//! and returned as a finite, visible piece. Nothing stands in for infinity with an enormous
//! coordinate: the clipping works on the line's parameter and may leave the range open.
//!
//! The constructions work on projected points. Lines are straight on screen on every price
//! scale, and midpoints, parallels and ratios are unchanged by the scale's zoom and offset (an
//! affine map), so a pitchfork or a channel built here keeps its shape when the chart moves.

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::f32::consts::{FRAC_PI_2, PI};

use crate::model::PitchforkVariant;

/// Points further than this are off the end of the world (the same bound `drawing_geometry`
/// uses), and a drawing there is not drawn.
const LIMIT: f64 = 1.0e9;

fn finite(point: Pos2) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && f64::from(point.x).abs() < LIMIT
        && f64::from(point.y).abs() < LIMIT
}

// MARK: Lines without an end

/// The part of the line through `a` and `b` that lies inside `rect`. `before` continues the line
/// past `a` for ever and `after` past `b`; with neither it is the segment `a`–`b`. `None` when
/// that misses the rectangle, and for coincident points (there is no direction to follow).
///
/// Liang–Barsky on the line's parameter, in `f64`: the parameter range is left open for an
/// unbounded side, so a line anchored a million pixels away still lands exactly on the edge.
pub fn line_in_rect(
    a: Pos2,
    b: Pos2,
    rect: Rect,
    before: bool,
    after: bool,
) -> Option<(Pos2, Pos2)> {
    if !finite(a) || !finite(b) {
        return None;
    }
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
    if dx == 0.0 && dy == 0.0 {
        return None;
    }
    let mut t0 = if before { f64::NEG_INFINITY } else { 0.0 };
    let mut t1 = if after { f64::INFINITY } else { 1.0 };
    let checks = [
        (-dx, ax - f64::from(rect.left())),
        (dx, f64::from(rect.right()) - ax),
        (-dy, ay - f64::from(rect.top())),
        (dy, f64::from(rect.bottom()) - ay),
    ];
    for (p, q) in checks {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    if t0 > t1 || !t0.is_finite() || !t1.is_finite() {
        return None;
    }
    Some((
        pos2((ax + dx * t0) as f32, (ay + dy * t0) as f32),
        pos2((ax + dx * t1) as f32, (ay + dy * t1) as f32),
    ))
}

/// The distance from `from` to the corner of `rect` furthest from it: a length that reaches
/// across the whole rectangle and no further.
pub fn reach(from: Pos2, rect: Rect) -> f32 {
    [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ]
    .into_iter()
    .map(|corner| corner.distance(from))
    .fold(0.0, f32::max)
}

/// `polygon` cut to `rect` (Sutherland–Hodgman). Exact for a convex polygon, which every band of
/// a channel or a pitchfork is; a concave one may keep zero-width bridges, which fill nothing.
pub fn clip_polygon(polygon: &[Pos2], rect: Rect) -> Vec<Pos2> {
    if polygon.len() < 3 || !polygon.iter().all(|point| finite(*point)) {
        return Vec::new();
    }
    type Edge = (fn(Pos2, Rect) -> bool, fn(Pos2, Pos2, Rect) -> Pos2);
    let edges: [Edge; 4] = [
        (|p, r| p.x >= r.left(), |a, b, r| cross_x(a, b, r.left())),
        (|p, r| p.x <= r.right(), |a, b, r| cross_x(a, b, r.right())),
        (|p, r| p.y >= r.top(), |a, b, r| cross_y(a, b, r.top())),
        (
            |p, r| p.y <= r.bottom(),
            |a, b, r| cross_y(a, b, r.bottom()),
        ),
    ];
    let mut output = polygon.to_vec();
    for (inside, cross) in edges {
        if output.is_empty() {
            break;
        }
        let input = std::mem::take(&mut output);
        let mut previous = input[input.len() - 1];
        for current in input {
            match (inside(previous, rect), inside(current, rect)) {
                (true, true) => output.push(current),
                (true, false) => output.push(cross(previous, current, rect)),
                (false, true) => {
                    output.push(cross(previous, current, rect));
                    output.push(current);
                }
                (false, false) => {}
            }
            previous = current;
        }
    }
    output
}

fn cross_x(a: Pos2, b: Pos2, x: f32) -> Pos2 {
    let t = (x - a.x) / (b.x - a.x);
    pos2(x, a.y + (b.y - a.y) * t)
}

fn cross_y(a: Pos2, b: Pos2, y: f32) -> Pos2 {
    let t = (y - a.y) / (b.y - a.y);
    pos2(a.x + (b.x - a.x) * t, y)
}

// MARK: Text

/// The angle (radians, clockwise on screen) to turn text so it follows the line from `from` to
/// `to`: always within a quarter turn either side of level, so it is never upside down. `None`
/// for a line with no direction.
pub fn text_angle(from: Pos2, to: Pos2) -> Option<f32> {
    let delta = to - from;
    if !(delta.length() > 1e-3) {
        return None;
    }
    let mut angle = delta.y.atan2(delta.x);
    if angle > FRAC_PI_2 {
        angle -= PI;
    } else if angle <= -FRAC_PI_2 {
        angle += PI;
    }
    Some(angle)
}

// MARK: Channels

/// The second edge of a parallel channel, as the vector from the first edge's points `a` and `b`
/// to the matching points of the edge through `c`. The edge keeps `a`'s and `b`'s times and moves
/// by a price offset, which is how a channel is read (a fixed price distance); a vertical first
/// edge has no such offset and takes the perpendicular one instead.
pub fn channel_offset(a: Pos2, b: Pos2, c: Pos2) -> Vec2 {
    let edge = b - a;
    if edge.x.abs() > 1e-3 {
        let on_edge = a.y + edge.y * (c.x - a.x) / edge.x;
        vec2(0.0, c.y - on_edge)
    } else {
        let length = edge.length();
        if length < 1e-3 {
            return Vec2::ZERO;
        }
        let normal = vec2(-edge.y, edge.x) / length;
        normal * (c - a).dot(normal)
    }
}

/// Where the line through `a` and `b` is at horizontal position `x`; `None` for a vertical line.
pub fn y_at(a: Pos2, b: Pos2, x: f32) -> Option<f32> {
    let dx = b.x - a.x;
    (dx.abs() > 1e-3).then(|| a.y + (b.y - a.y) * (x - a.x) / dx)
}

/// The end of a disjoint channel's second edge: the point at `end_x` on the line through `c`
/// that runs parallel to `a`→`b`. A vertical first edge is followed by `c` + (`b` − `a`).
pub fn disjoint_end(a: Pos2, b: Pos2, c: Pos2, end_x: f32) -> Pos2 {
    let dx = b.x - a.x;
    if dx.abs() > 1e-3 {
        let slope = (b.y - a.y) / dx;
        pos2(end_x, c.y + slope * (end_x - c.x))
    } else {
        c + (b - a)
    }
}

// MARK: Pitchforks

/// How far along its handle a pitchfork's lines run when they are not extended, in handle
/// lengths from the median's start. The median goes through the base (at 1) and on to this; the
/// other lines run from their start to it. Inferred: TradingView does not state the length.
pub const FORK_LENGTH: f32 = 2.0;

/// A pitchfork's construction: where the median starts, the vector of its handle, and the
/// base its other lines are measured across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fork {
    pub origin: Pos2,
    /// From the origin to the end of the handle; every line of the fork runs along it.
    pub handle: Vec2,
    /// The middle of the base, where level 0 sits.
    pub middle: Pos2,
    /// From the middle of the base to its far end (level 1 on one side).
    pub half: Vec2,
    /// How far along the handle the other lines start: 1 at the base of a regular fork, 0 for
    /// an inside one, whose base is where its median starts.
    pub start: f32,
}

/// The fork `variant` makes of the pivots `p1`, `p2`, `p3`. `None` when its handle has no
/// length (the pivots lie on one point).
///
/// - Standard: the median runs from P1 through the middle of P2–P3.
/// - Schiff: the same, but from P1's time at the price halfway between P1 and P2.
/// - Modified Schiff: the same, but from the middle of P1–P2 in both time and price.
/// - Inside: from the middle of P1–P2 to P3, with the levels measured across P1–P2.
pub fn fork(variant: PitchforkVariant, p1: Pos2, p2: Pos2, p3: Pos2) -> Option<Fork> {
    if ![p1, p2, p3].iter().all(|point| finite(*point)) {
        return None;
    }
    let mid = |a: Pos2, b: Pos2| a.lerp(b, 0.5);
    let fork = match variant {
        PitchforkVariant::Standard
        | PitchforkVariant::Schiff
        | PitchforkVariant::ModifiedSchiff => {
            let origin = match variant {
                PitchforkVariant::Standard => p1,
                PitchforkVariant::Schiff => pos2(p1.x, (p1.y + p2.y) / 2.0),
                _ => mid(p1, p2),
            };
            let middle = mid(p2, p3);
            Fork {
                origin,
                handle: middle - origin,
                middle,
                half: (p3 - p2) / 2.0,
                start: 1.0,
            }
        }
        PitchforkVariant::Inside => {
            let origin = mid(p1, p2);
            Fork {
                origin,
                handle: p3 - origin,
                middle: origin,
                half: (p2 - p1) / 2.0,
                start: 0.0,
            }
        }
    };
    (fork.handle.length() > 1e-3).then_some(fork)
}

impl Fork {
    /// Where the median ends when it is not extended.
    pub fn median_end(&self) -> Pos2 {
        self.origin + self.handle * FORK_LENGTH
    }

    /// Where the line at `ratio` half-bases from the middle of the base, on `side` (-1 or 1),
    /// starts: on the base, parallel to the median.
    pub fn level_start(&self, ratio: f32, side: f32) -> Pos2 {
        self.middle + self.half * (ratio * side)
    }

    /// Where that line ends when it is not extended.
    pub fn level_end(&self, ratio: f32, side: f32) -> Pos2 {
        self.level_start(ratio, side) + self.handle * (FORK_LENGTH - self.start)
    }
}

#[cfg(test)]
#[path = "line_geometry_tests.rs"]
mod tests;
