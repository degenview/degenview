//! Pure screen-space geometry for the drawing tools. Everything takes and returns `Pos2`s, so it
//! runs and is tested without a window, and nothing here knows about the drawing model or the
//! chart: the layer projects anchors to pixels and hands the points over.
//!
//! Every function rejects or safely handles bad input (NaN, infinity, coincident points, huge
//! values) by returning `None` or a degenerate-but-finite result, and bounds its allocations.

use eframe::egui::{Color32, Mesh, Pos2, Rect, Vec2, pos2, vec2};
use std::f32::consts::{PI, TAU};

/// How far (px) a flattened curve may stray from the true curve.
pub const FLATNESS: f32 = 0.25;
/// The most segments one flattened curve or arc uses.
pub const MAX_CURVE_SEGMENTS: usize = 256;
/// Coordinates beyond this are treated as off the end of the world.
pub const LIMIT: f32 = 1.0e7;

pub fn is_finite(point: Pos2) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.x.abs() < LIMIT && point.y.abs() < LIMIT
}

pub fn all_finite(points: &[Pos2]) -> bool {
    points.iter().all(|point| is_finite(*point))
}

// MARK: Distances and bounds

/// Distance from `point` to the segment `a`–`b`.
pub fn distance_to_segment(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let length_sq = ab.length_sq();
    if length_sq <= f32::EPSILON {
        return point.distance(a);
    }
    let t = ((point - a).dot(ab) / length_sq).clamp(0.0, 1.0);
    point.distance(a + ab * t)
}

/// Distance from `point` to the nearest segment of a polyline (`closed` adds the last→first
/// segment). A single point is a dot.
pub fn distance_to_polyline(point: Pos2, points: &[Pos2], closed: bool) -> f32 {
    match points {
        [] => f32::INFINITY,
        [only] => point.distance(*only),
        _ => {
            let open = points
                .windows(2)
                .map(|pair| distance_to_segment(point, pair[0], pair[1]));
            let closing = closed
                .then(|| distance_to_segment(point, points[points.len() - 1], points[0]))
                .into_iter();
            open.chain(closing).fold(f32::INFINITY, f32::min)
        }
    }
}

/// The smallest rectangle holding every point; `None` for no points.
pub fn bounds(points: &[Pos2]) -> Option<Rect> {
    let first = *points.first()?;
    Some(
        points
            .iter()
            .fold(Rect::from_min_max(first, first), |rect, point| {
                Rect::from_min_max(rect.min.min(*point), rect.max.max(*point))
            }),
    )
}

/// Whether `point` is inside the polygon by the even-odd rule (a ray cast to the right; a
/// point on the boundary may fall either way). The rule the fill uses too, so what is filled
/// is what can be picked.
pub fn polygon_contains(polygon: &[Pos2], point: Pos2) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for current in polygon {
        if (current.y > point.y) != (previous.y > point.y) {
            let x = previous.x
                + (point.y - previous.y) / (current.y - previous.y) * (current.x - previous.x);
            if point.x < x {
                inside = !inside;
            }
        }
        previous = *current;
    }
    inside
}

/// Twice the signed area of a polygon (positive when the points run clockwise on screen,
/// where y grows downwards).
pub fn signed_area2(polygon: &[Pos2]) -> f32 {
    let mut sum = 0.0;
    let mut previous = match polygon.last() {
        Some(last) => *last,
        None => return 0.0,
    };
    for current in polygon {
        sum += previous.x * current.y - current.x * previous.y;
        previous = *current;
    }
    sum
}

// MARK: Curves

fn lerp(a: Pos2, b: Pos2, t: f32) -> Pos2 {
    a + (b - a) * t
}

pub fn quad_point(p0: Pos2, p1: Pos2, p2: Pos2, t: f32) -> Pos2 {
    lerp(lerp(p0, p1, t), lerp(p1, p2, t), t)
}

pub fn cubic_point(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2, t: f32) -> Pos2 {
    let a = quad_point(p0, p1, p2, t);
    let b = quad_point(p1, p2, p3, t);
    lerp(a, b, t)
}

fn segments_for(deviation: f32) -> usize {
    if !deviation.is_finite() {
        return 1;
    }
    (deviation / FLATNESS)
        .sqrt()
        .ceil()
        .clamp(2.0, MAX_CURVE_SEGMENTS as f32) as usize
}

/// A quadratic Bézier as a polyline within [`FLATNESS`] of the curve. The endpoints are exact.
pub fn flatten_quad(p0: Pos2, p1: Pos2, p2: Pos2) -> Vec<Pos2> {
    if !all_finite(&[p0, p1, p2]) {
        return vec![p0, p2];
    }
    // A uniform split of n pieces strays at most |p0 - 2p1 + p2| / (4n²).
    let second = (p0.to_vec2() - p1.to_vec2() * 2.0 + p2.to_vec2()).length();
    let n = segments_for(second * 0.25);
    (0..=n)
        .map(|i| quad_point(p0, p1, p2, i as f32 / n as f32))
        .collect()
}

/// A cubic Bézier as a polyline within [`FLATNESS`] of the curve.
pub fn flatten_cubic(p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2) -> Vec<Pos2> {
    if !all_finite(&[p0, p1, p2, p3]) {
        return vec![p0, p3];
    }
    let d1 = (p0.to_vec2() - p1.to_vec2() * 2.0 + p2.to_vec2()).length();
    let d2 = (p1.to_vec2() - p2.to_vec2() * 2.0 + p3.to_vec2()).length();
    let n = segments_for(d1.max(d2) * 0.75);
    (0..=n)
        .map(|i| cubic_point(p0, p1, p2, p3, i as f32 / n as f32))
        .collect()
}

/// A circular arc: the circle through three points, from the first to the second, bulging
/// through the third.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    pub center: Pos2,
    pub radius: f32,
    pub start_angle: f32,
    /// Signed sweep in radians; negative runs the other way round the circle.
    pub sweep: f32,
}

/// The arc from `start` to `end` through `through`. `None` when the three points are (nearly)
/// collinear, coincide or would need a circle too large to draw: callers fall back to the
/// straight chord.
pub fn arc_through(start: Pos2, end: Pos2, through: Pos2) -> Option<Arc> {
    if !all_finite(&[start, end, through]) {
        return None;
    }
    let (ax, ay) = (start.x as f64, start.y as f64);
    let (bx, by) = (end.x as f64, end.y as f64);
    let (cx, cy) = (through.x as f64, through.y as f64);
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    // `d` is four times the triangle's area: tiny against the longest side means collinear.
    let longest = start
        .distance(end)
        .max(start.distance(through))
        .max(end.distance(through)) as f64;
    if longest < 1e-3 || d.abs() < 1e-6 * longest * longest {
        return None;
    }
    let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    let radius = ((ax - ux).powi(2) + (ay - uy).powi(2)).sqrt();
    if !radius.is_finite() || radius > 1.0e6 || radius < 1e-3 {
        return None;
    }
    let angle = |x: f64, y: f64| (y - uy).atan2(x - ux);
    let (a0, a1, am) = (angle(ax, ay), angle(bx, by), angle(cx, cy));
    let norm = |value: f64| value.rem_euclid(std::f64::consts::TAU);
    let counter = norm(a1 - a0);
    let mid = norm(am - a0);
    // Walk the way that meets `through` before `end`.
    let sweep = if mid <= counter {
        counter
    } else {
        counter - std::f64::consts::TAU
    };
    Some(Arc {
        center: pos2(ux as f32, uy as f32),
        radius: radius as f32,
        start_angle: a0 as f32,
        sweep: sweep as f32,
    })
}

impl Arc {
    pub fn point(&self, t: f32) -> Pos2 {
        let angle = self.start_angle + self.sweep * t;
        self.center + vec2(angle.cos(), angle.sin()) * self.radius
    }

    /// The arc as a polyline within [`FLATNESS`], endpoints included.
    pub fn flatten(&self) -> Vec<Pos2> {
        // A chord of angle θ strays r(1 - cos(θ/2)) from the circle.
        let step = if self.radius <= FLATNESS {
            PI / 2.0
        } else {
            2.0 * (1.0 - FLATNESS / self.radius).clamp(-1.0, 1.0).acos()
        };
        let n = (self.sweep.abs() / step.max(1e-3))
            .ceil()
            .clamp(4.0, MAX_CURVE_SEGMENTS as f32 * 2.0) as usize;
        (0..=n).map(|i| self.point(i as f32 / n as f32)).collect()
    }
}

/// An ellipse as a polygon within [`FLATNESS`] (no repeated closing point).
pub fn ellipse_polygon(center: Pos2, rx: f32, ry: f32) -> Vec<Pos2> {
    let (rx, ry) = (rx.abs(), ry.abs());
    if !is_finite(center) || !rx.is_finite() || !ry.is_finite() || rx > LIMIT || ry > LIMIT {
        return Vec::new();
    }
    let r = rx.max(ry);
    let step = if r <= FLATNESS {
        PI / 2.0
    } else {
        2.0 * (1.0 - FLATNESS / r).clamp(-1.0, 1.0).acos()
    };
    let n = (TAU / step.max(1e-3)).ceil().clamp(12.0, 720.0) as usize;
    (0..n)
        .map(|i| {
            let angle = TAU * i as f32 / n as f32;
            center + vec2(angle.cos() * rx, angle.sin() * ry)
        })
        .collect()
}

// MARK: Rectangles

/// The four corners of the rotated rectangle whose first edge is `a`→`b` and whose width is the
/// signed perpendicular distance of `width` from that edge's line. `None` for a zero-length
/// edge. A `width` on the line gives a flat rectangle (two coincident edges), not an error.
pub fn rotated_rect(a: Pos2, b: Pos2, width: Pos2) -> Option<[Pos2; 4]> {
    if !all_finite(&[a, b, width]) {
        return None;
    }
    let edge = b - a;
    let length = edge.length();
    if length < 1e-3 {
        return None;
    }
    let direction = edge / length;
    let normal = vec2(-direction.y, direction.x);
    let offset = (width - a).dot(normal);
    Some([a, b, b + normal * offset, a + normal * offset])
}

/// The rectangle with opposite corners `a` and `b`, constrained to a square when `square`
/// (the side is the longer of the two, the drag direction is kept).
pub fn constrained_corner(anchor: Pos2, pointer: Pos2, square: bool) -> Pos2 {
    if !square {
        return pointer;
    }
    let delta = pointer - anchor;
    let side = delta.x.abs().max(delta.y.abs());
    anchor + vec2(side.copysign(delta.x), side.copysign(delta.y))
}

/// `pointer` moved onto the nearest multiple of 45° around `from` (the Shift constraint of a
/// line).
pub fn snap_angle(from: Pos2, pointer: Pos2) -> Pos2 {
    let delta = pointer - from;
    let length = delta.length();
    if length < 1e-3 || !length.is_finite() {
        return pointer;
    }
    let step = PI / 4.0;
    let angle = (delta.y.atan2(delta.x) / step).round() * step;
    from + vec2(angle.cos(), angle.sin()) * length
}

// MARK: Arrows

/// The triangular head at `tip` of a line coming from `from`: `[tip, left, right]`. `None`
/// when the line has no length.
pub fn arrowhead(from: Pos2, tip: Pos2, length: f32, half_width: f32) -> Option<[Pos2; 3]> {
    let delta = tip - from;
    let line = delta.length();
    if !line.is_finite() || line < 1e-3 {
        return None;
    }
    let direction = delta / line;
    let base = tip - direction * length;
    let normal = vec2(-direction.y, direction.x);
    Some([tip, base + normal * half_width, base - normal * half_width])
}

/// A solid arrow from `tail` to `tip`: a shaft that widens from a thin tail to `thickness`,
/// then a head. The polygon, clockwise from the tail's left side. `None` for no length.
pub fn arrow_marker(tail: Pos2, tip: Pos2, thickness: f32) -> Option<Vec<Pos2>> {
    let delta = tip - tail;
    let length = delta.length();
    if !length.is_finite() || length < 1.0 {
        return None;
    }
    let direction = delta / length;
    let normal = vec2(-direction.y, direction.x);
    let thickness = thickness.clamp(1.0, 40.0);
    let head_half = thickness * 2.2;
    let head_length = (thickness * 3.6 + 4.0).min(length * 0.6);
    let shaft_half = thickness * 0.9;
    let base = tip - direction * head_length;
    let tail_half = shaft_half * 0.25;
    Some(vec![
        tail + normal * tail_half,
        base + normal * shaft_half,
        base + normal * head_half,
        tip,
        base - normal * head_half,
        base - normal * shaft_half,
        tail - normal * tail_half,
    ])
}

/// Pixel size of the fixed arrow-mark glyph: head width, head height, stem width, stem height.
pub const MARK_HEAD_WIDTH: f32 = 20.0;
pub const MARK_HEAD_HEIGHT: f32 = 12.0;
pub const MARK_STEM_WIDTH: f32 = 8.0;
pub const MARK_STEM_HEIGHT: f32 = 14.0;
/// How far the glyph's tip sits clear of the anchor.
pub const MARK_GAP: f32 = 4.0;

/// The outline of an up or down arrow mark whose tip points at `at` from below (up) or above
/// (down). The glyph has a fixed pixel size, whatever the chart's scales.
pub fn arrow_mark(at: Pos2, up: bool) -> Vec<Pos2> {
    let sign = if up { 1.0 } else { -1.0 };
    let tip = at + vec2(0.0, MARK_GAP * sign);
    let head_base = tip.y + MARK_HEAD_HEIGHT * sign;
    let end = head_base + MARK_STEM_HEIGHT * sign;
    let (head, stem) = (MARK_HEAD_WIDTH / 2.0, MARK_STEM_WIDTH / 2.0);
    vec![
        tip,
        pos2(tip.x + head, head_base),
        pos2(tip.x + stem, head_base),
        pos2(tip.x + stem, end),
        pos2(tip.x - stem, end),
        pos2(tip.x - stem, head_base),
        pos2(tip.x - head, head_base),
    ]
}

// MARK: Clipping and extension

/// The part of the segment `a`–`b` inside `rect` (Liang–Barsky), or `None` when it misses.
pub fn clip_segment(a: Pos2, b: Pos2, rect: Rect) -> Option<(Pos2, Pos2)> {
    if !all_finite(&[a, b]) {
        return None;
    }
    let delta = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    let checks = [
        (-delta.x, a.x - rect.left()),
        (delta.x, rect.right() - a.x),
        (-delta.y, a.y - rect.top()),
        (delta.y, rect.bottom() - a.y),
    ];
    for (p, q) in checks {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return None;
                }
                t1 = t1.min(r);
            }
        }
    }
    Some((a + delta * t0, a + delta * t1))
}

/// The line through `a` and `b`, prolonged past `a` (`before`) and/or past `b` (`after`) to the
/// edge of `rect`. Anchors never change: this is a drawing-time projection only. A zero-length
/// line is returned as it is.
pub fn extend_to_rect(a: Pos2, b: Pos2, rect: Rect, before: bool, after: bool) -> (Pos2, Pos2) {
    let delta = b - a;
    if !all_finite(&[a, b]) || delta.length() < 1e-3 {
        return (a, b);
    }
    // Far enough to leave any rectangle, then clipped back to it.
    let reach = (rect.width() + rect.height() + a.distance(b)) * 2.0 / delta.length();
    let start = if before { a - delta * reach } else { a };
    let end = if after { b + delta * reach } else { b };
    match clip_segment(start, end, rect) {
        Some(clipped) => clipped,
        None => (a, b),
    }
}

// MARK: Simplification and smoothing

/// Ramer–Douglas–Peucker: the points that keep the line within `epsilon` of the original, ends
/// included. Iterative, so a very long stroke cannot overflow the stack.
#[cfg(test)]
pub fn simplify(points: &[Pos2], epsilon: f32) -> Vec<Pos2> {
    simplify_indices(points, epsilon)
        .into_iter()
        .map(|index| points[index])
        .collect()
}

/// The indices [`simplify`] keeps, ascending, so a caller can keep its own data (the stored
/// anchors) for the surviving points rather than a projection round trip of them.
pub fn simplify_indices(points: &[Pos2], epsilon: f32) -> Vec<usize> {
    if points.len() < 3 || !epsilon.is_finite() || epsilon <= 0.0 {
        return (0..points.len()).collect();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0usize, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        if last <= first + 1 {
            continue;
        }
        let (mut far, mut far_distance) = (first, 0.0f32);
        for index in first + 1..last {
            let distance = distance_to_segment(points[index], points[first], points[last]);
            if distance > far_distance {
                (far, far_distance) = (index, distance);
            }
        }
        if far_distance > epsilon {
            keep[far] = true;
            stack.push((first, far));
            stack.push((far, last));
        }
    }
    keep.iter()
        .enumerate()
        .filter_map(|(index, kept)| kept.then_some(index))
        .collect()
}

/// Chaikin corner cutting, `passes` times, ends kept. Skipped (the points returned as they are)
/// once the result would pass `limit` points.
pub fn smooth(points: &[Pos2], passes: usize, limit: usize) -> Vec<Pos2> {
    let mut current = points.to_vec();
    for _ in 0..passes {
        if current.len() < 3 || current.len() * 2 > limit {
            break;
        }
        let mut next = Vec::with_capacity(current.len() * 2);
        next.push(current[0]);
        for pair in current.windows(2) {
            next.push(lerp(pair[0], pair[1], 0.25));
            next.push(lerp(pair[0], pair[1], 0.75));
        }
        next.push(current[current.len() - 1]);
        current = next;
    }
    current
}

// MARK: Polygon fill

/// A horizontal slice of a filled polygon: `[top-left, top-right, bottom-right, bottom-left]`.
pub type Trapezoid = [Pos2; 4];

struct Edge {
    top: Pos2,
    bottom: Pos2,
}

impl Edge {
    fn x_at(&self, y: f32) -> f32 {
        let height = self.bottom.y - self.top.y;
        if height <= f32::EPSILON {
            return self.top.x;
        }
        self.top.x + (self.bottom.x - self.top.x) * ((y - self.top.y) / height)
    }
}

/// The even-odd fill of a closed polygon as horizontal trapezoids. Exact for any polygon:
/// concave, self-intersecting or with a hole made by overlapping loops. A band is split where
/// two edges cross, so no trapezoid is bent. Empty for fewer than three finite points.
pub fn fill_even_odd(polygon: &[Pos2]) -> Vec<Trapezoid> {
    if polygon.len() < 3 || !all_finite(polygon) {
        return Vec::new();
    }
    let mut edges = Vec::with_capacity(polygon.len());
    let mut previous = polygon[polygon.len() - 1];
    for current in polygon {
        if (current.y - previous.y).abs() > f32::EPSILON {
            let (top, bottom) = if previous.y < current.y {
                (previous, *current)
            } else {
                (*current, previous)
            };
            edges.push(Edge { top, bottom });
        }
        previous = *current;
    }
    if edges.is_empty() {
        return Vec::new();
    }
    let mut ys: Vec<f32> = polygon.iter().map(|point| point.y).collect();
    ys.sort_by(f32::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-4);

    let mut out = Vec::new();
    let mut splits_left = 20_000usize;
    let mut bands: Vec<(f32, f32)> = ys.windows(2).map(|pair| (pair[0], pair[1])).collect();
    bands.reverse();
    while let Some((y0, y1)) = bands.pop() {
        if y1 - y0 < 1e-4 {
            continue;
        }
        let ym = (y0 + y1) / 2.0;
        let mut active: Vec<(f32, f32, f32)> = edges
            .iter()
            .filter(|edge| edge.top.y <= y0 + 1e-4 && edge.bottom.y >= y1 - 1e-4)
            .map(|edge| (edge.x_at(ym), edge.x_at(y0), edge.x_at(y1)))
            .collect();
        active.sort_by(|a, b| a.0.total_cmp(&b.0));
        // The first crossing is between neighbours: split the band there.
        let crossing = active.windows(2).find_map(|pair| {
            let (left, right) = (pair[0], pair[1]);
            let gap0 = right.1 - left.1;
            let gap1 = right.2 - left.2;
            if gap0 * gap1 < 0.0 {
                Some(y0 + (y1 - y0) * (gap0 / (gap0 - gap1)))
            } else {
                None
            }
        });
        if let Some(y) = crossing
            && splits_left > 0
            && y > y0 + 1e-3
            && y < y1 - 1e-3
        {
            splits_left -= 1;
            bands.push((y, y1));
            bands.push((y0, y));
            continue;
        }
        for pair in active.as_chunks::<2>().0 {
            out.push([
                pos2(pair[0].1, y0),
                pos2(pair[1].1, y0),
                pos2(pair[1].2, y1),
                pos2(pair[0].2, y1),
            ]);
        }
    }
    out
}

/// The area of a set of trapezoids.
#[cfg(test)]
pub fn trapezoid_area(trapezoids: &[Trapezoid]) -> f32 {
    trapezoids
        .iter()
        .map(|t| ((t[1].x - t[0].x) + (t[2].x - t[3].x)) / 2.0 * (t[3].y - t[0].y))
        .sum()
}

/// A filled polygon as one mesh in one colour (no antialiasing: the outline is stroked over
/// the fill's edge).
pub fn fill_mesh(polygon: &[Pos2], color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    for trapezoid in fill_even_odd(polygon) {
        let base = mesh.vertices.len() as u32;
        for corner in trapezoid {
            mesh.colored_vertex(corner, color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    mesh
}

// MARK: Stroke masks

/// The union of a thick round-capped stroke as a coverage map on the physical pixel grid. A
/// translucent stroke is painted from this as one image, so every pixel the stroke touches gets
/// the stroke's alpha exactly once however the path crosses itself; a mesh of triangles cannot
/// promise that where the path bends tighter than its own width.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeMask {
    /// Where the map's top-left pixel sits on screen, in points, on the pixel grid.
    pub origin: Pos2,
    /// Width and height in physical pixels.
    pub size: [usize; 2],
    pub pixels_per_point: f32,
    /// One coverage byte per pixel, row by row: 255 inside the stroke, a ramp across its edge.
    pub coverage: Vec<u8>,
}

impl StrokeMask {
    /// The map's extent on screen.
    pub fn rect(&self) -> Rect {
        Rect::from_min_size(
            self.origin,
            vec2(
                self.size[0] as f32 / self.pixels_per_point,
                self.size[1] as f32 / self.pixels_per_point,
            ),
        )
    }

    #[cfg(test)]
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.coverage[y * self.size[0] + x]
    }
}

/// The most pixels one mask may cover; a bigger stroke falls back to the mesh.
pub const MAX_MASK_PIXELS: usize = 12_000_000;
/// The most pixel visits a mask may cost to build.
const MAX_MASK_WORK: usize = 60_000_000;

/// Rasterizes a stroke of `width` points through `points` at `pixels_per_point`. `None` for an
/// unusable path or width, or a stroke too big to hold as a map.
pub fn rasterize_stroke(points: &[Pos2], width: f32, pixels_per_point: f32) -> Option<StrokeMask> {
    if points.is_empty()
        || !width.is_finite()
        || width <= 0.0
        || !pixels_per_point.is_finite()
        || pixels_per_point <= 0.0
        || !all_finite(points)
    {
        return None;
    }
    let ppp = pixels_per_point;
    let half = width / 2.0;
    let bounds = bounds(points)?.expand(half + 1.5);
    let origin = pos2(
        (bounds.left() * ppp).floor() / ppp,
        (bounds.top() * ppp).floor() / ppp,
    );
    let columns = ((bounds.right() - origin.x) * ppp).ceil() as usize + 1;
    let rows = ((bounds.bottom() - origin.y) * ppp).ceil() as usize + 1;
    if columns == 0 || rows == 0 || columns.checked_mul(rows)? > MAX_MASK_PIXELS {
        return None;
    }
    let mut coverage = vec![0u8; columns * rows];
    let half_px = half * ppp;
    let mut work = 0usize;
    let segments: Vec<(Pos2, Pos2)> = if points.len() == 1 {
        vec![(points[0], points[0])]
    } else {
        points.windows(2).map(|pair| (pair[0], pair[1])).collect()
    };
    for (a, b) in segments {
        let (min, max) = (a.min(b), a.max(b));
        let x0 = (((min.x - half - 1.0 - origin.x) * ppp).floor().max(0.0)) as usize;
        let y0 = (((min.y - half - 1.0 - origin.y) * ppp).floor().max(0.0)) as usize;
        let x1 = ((((max.x + half + 1.0 - origin.x) * ppp).ceil()) as usize).min(columns - 1);
        let y1 = ((((max.y + half + 1.0 - origin.y) * ppp).ceil()) as usize).min(rows - 1);
        work = work.saturating_add((x1 + 1).saturating_sub(x0) * (y1 + 1).saturating_sub(y0));
        if work > MAX_MASK_WORK {
            return None;
        }
        for y in y0..=y1 {
            let py = origin.y + (y as f32 + 0.5) / ppp;
            for x in x0..=x1 {
                let px = origin.x + (x as f32 + 0.5) / ppp;
                let distance = distance_to_segment(pos2(px, py), a, b) * ppp;
                let value = (half_px - distance + 0.5).clamp(0.0, 1.0);
                if value > 0.0 {
                    let byte = (value * 255.0).round() as u8;
                    let slot = &mut coverage[y * columns + x];
                    if byte > *slot {
                        *slot = byte;
                    }
                }
            }
        }
    }
    Some(StrokeMask {
        origin,
        size: [columns, rows],
        pixels_per_point: ppp,
        coverage,
    })
}

// MARK: Strokes

/// The triangles of a uniform-width stroke with round joins and caps, and the outline of the
/// whole shape. The triangles of a path that does not cross itself never overlap, so a
/// translucent stroke has one alpha everywhere (the Highlighter's requirement). The inner side
/// of a bend is mitred, the outer side is a fan around the vertex.
pub struct StrokeGeometry {
    pub triangles: Vec<[Pos2; 3]>,
    pub outline: Vec<Pos2>,
}

fn unit(vector: Vec2) -> Option<Vec2> {
    let length = vector.length();
    (length.is_finite() && length > 1e-4).then(|| vector / length)
}

fn left_normal(direction: Vec2) -> Vec2 {
    vec2(-direction.y, direction.x)
}

fn arc_points(center: Pos2, from: Vec2, to: Vec2, radius: f32, out: &mut Vec<Pos2>) {
    // The shorter way round from `from` to `to`, excluding both ends.
    let a0 = from.y.atan2(from.x);
    let a1 = to.y.atan2(to.x);
    let mut sweep = a1 - a0;
    while sweep > PI {
        sweep -= TAU;
    }
    while sweep < -PI {
        sweep += TAU;
    }
    let step = if radius <= FLATNESS {
        PI / 2.0
    } else {
        2.0 * (1.0 - FLATNESS / radius).clamp(-1.0, 1.0).acos()
    };
    let n = (sweep.abs() / step.max(0.05)).ceil() as usize;
    for i in 1..n {
        let angle = a0 + sweep * i as f32 / n as f32;
        out.push(center + vec2(angle.cos(), angle.sin()) * radius);
    }
}

/// Builds a stroke through `points` of the given `width`. A single point is a dot. Points closer
/// than a hundredth of a pixel to their predecessor are dropped.
pub fn stroke_geometry(points: &[Pos2], width: f32) -> Option<StrokeGeometry> {
    if !width.is_finite() || width <= 0.0 || !all_finite(points) {
        return None;
    }
    let half = width / 2.0;
    let mut path: Vec<Pos2> = Vec::with_capacity(points.len());
    for point in points {
        if path.last().is_none_or(|last| last.distance(*point) > 0.01) {
            path.push(*point);
        }
    }
    if path.is_empty() {
        return None;
    }
    if path.len() == 1 {
        // A dot: a regular polygon, fanned.
        let center = path[0];
        let polygon = ellipse_polygon(center, half, half);
        let triangles = (0..polygon.len())
            .map(|i| [center, polygon[i], polygon[(i + 1) % polygon.len()]])
            .collect();
        return Some(StrokeGeometry {
            triangles,
            outline: polygon,
        });
    }

    let n = path.len();
    let directions: Vec<Vec2> = path
        .windows(2)
        .map(|pair| unit(pair[1] - pair[0]).unwrap_or(vec2(1.0, 0.0)))
        .collect();
    // Per vertex: where the left and right edges of the incoming (`*_in`) and outgoing (`*_out`)
    // segments meet it, and the points of the round join on the outer side of a bend.
    let mut left_in = vec![Pos2::ZERO; n];
    let mut left_out = vec![Pos2::ZERO; n];
    let mut right_in = vec![Pos2::ZERO; n];
    let mut right_out = vec![Pos2::ZERO; n];
    let mut left_arc: Vec<Vec<Pos2>> = vec![Vec::new(); n];
    let mut right_arc: Vec<Vec<Pos2>> = vec![Vec::new(); n];
    let first_normal = left_normal(directions[0]);
    left_out[0] = path[0] + first_normal * half;
    right_out[0] = path[0] - first_normal * half;
    let last_normal = left_normal(directions[n - 2]);
    left_in[n - 1] = path[n - 1] + last_normal * half;
    right_in[n - 1] = path[n - 1] - last_normal * half;
    for i in 1..n - 1 {
        let (d0, d1) = (directions[i - 1], directions[i]);
        let (n0, n1) = (left_normal(d0), left_normal(d1));
        let cross = d0.x * d1.y - d0.y * d1.x;
        let p = path[i];
        if cross.abs() < 1e-5 {
            if d0.dot(d1) > 0.0 {
                // Straight on.
                left_in[i] = p + n0 * half;
                left_out[i] = left_in[i];
                right_in[i] = p - n0 * half;
                right_out[i] = right_in[i];
            } else {
                // Doubling back: the left side wraps the tip, the right collapses to the vertex.
                left_in[i] = p + n0 * half;
                left_out[i] = p - n0 * half;
                right_in[i] = p;
                right_out[i] = p;
                left_arc[i] = cap_points(p, n0, d0, half);
            }
            continue;
        }
        // The inner side meets at the miter point, held back from running past the shorter
        // neighbouring segment; the outer side is a round join.
        let miter = (n0 + n1) / (1.0 + n0.dot(n1)).max(1e-3);
        let reach = path[i - 1]
            .distance(p)
            .min(path[i + 1].distance(p))
            .max(half);
        let length = miter.length() * half;
        let miter = if length > reach {
            miter * (reach / length)
        } else {
            miter
        };
        if cross > 0.0 {
            // Turning towards the left normal: the left side is the inside.
            left_in[i] = p + miter * half;
            left_out[i] = left_in[i];
            right_in[i] = p - n0 * half;
            right_out[i] = p - n1 * half;
            arc_points(p, -n0, -n1, half, &mut right_arc[i]);
        } else {
            right_in[i] = p - miter * half;
            right_out[i] = right_in[i];
            left_in[i] = p + n0 * half;
            left_out[i] = p + n1 * half;
            arc_points(p, n0, n1, half, &mut left_arc[i]);
        }
    }

    let mut triangles = Vec::new();
    // The segment quads.
    for i in 0..n - 1 {
        let (l0, r0) = (left_out[i], right_out[i]);
        let (l1, r1) = (left_in[i + 1], right_in[i + 1]);
        triangles.push([l0, r0, l1]);
        triangles.push([r0, r1, l1]);
    }
    // Join fans on the outside of each bend.
    for i in 1..n - 1 {
        for (arc, from, to) in [
            (&left_arc[i], left_in[i], left_out[i]),
            (&right_arc[i], right_in[i], right_out[i]),
        ] {
            if from == to {
                continue;
            }
            let mut chain = vec![from];
            chain.extend(arc.iter().copied());
            chain.push(to);
            for pair in chain.windows(2) {
                triangles.push([path[i], pair[0], pair[1]]);
            }
        }
    }
    // Round caps: the end runs left to right round the front, the start right to left round
    // the back.
    let end_cap = cap_points(path[n - 1], last_normal, directions[n - 2], half);
    let start_cap = cap_points(path[0], -first_normal, -directions[0], half);
    for (center, cap, from, to) in [
        (path[n - 1], &end_cap, left_in[n - 1], right_in[n - 1]),
        (path[0], &start_cap, right_out[0], left_out[0]),
    ] {
        let mut chain = vec![from];
        chain.extend(cap.iter().copied());
        chain.push(to);
        for pair in chain.windows(2) {
            triangles.push([center, pair[0], pair[1]]);
        }
    }

    // The outline: down the left, round the end, back up the right, round the start.
    let mut outline = vec![left_out[0]];
    for i in 1..n - 1 {
        outline.push(left_in[i]);
        outline.extend(left_arc[i].iter().copied());
        if left_out[i] != left_in[i] {
            outline.push(left_out[i]);
        }
    }
    outline.push(left_in[n - 1]);
    outline.extend(end_cap.iter().copied());
    outline.push(right_in[n - 1]);
    for i in (1..n - 1).rev() {
        outline.push(right_out[i]);
        outline.extend(right_arc[i].iter().rev().copied());
        if right_out[i] != right_in[i] {
            outline.push(right_in[i]);
        }
    }
    outline.push(right_out[0]);
    outline.extend(start_cap.iter().copied());
    Some(StrokeGeometry { triangles, outline })
}

/// The points of a semicircle round `center`, from the edge point at `from_normal` through
/// `direction` to the opposite edge point, excluding the two end points.
fn cap_points(center: Pos2, from_normal: Vec2, direction: Vec2, radius: f32) -> Vec<Pos2> {
    let a0 = from_normal.y.atan2(from_normal.x);
    let cross = from_normal.x * direction.y - from_normal.y * direction.x;
    let sweep = if cross > 0.0 { PI } else { -PI };
    let step = if radius <= FLATNESS {
        PI / 2.0
    } else {
        2.0 * (1.0 - FLATNESS / radius).clamp(-1.0, 1.0).acos()
    };
    let n = (PI / step.max(0.05)).ceil().max(2.0) as usize;
    (1..n)
        .map(|i| {
            let angle = a0 + sweep * i as f32 / n as f32;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// The stroke as one antialiased mesh: opaque inside, fading to nothing over `feather` pixels
/// outside its outline. `None` when the path or width is unusable.
pub fn stroke_mesh(points: &[Pos2], width: f32, color: Color32, feather: f32) -> Option<Mesh> {
    let geometry = stroke_geometry(points, width)?;
    let mut mesh = Mesh::default();
    for triangle in &geometry.triangles {
        let base = mesh.vertices.len() as u32;
        for corner in triangle {
            mesh.colored_vertex(*corner, color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
    }
    if feather > 0.0 && geometry.outline.len() >= 3 {
        let outline = &geometry.outline;
        let clockwise = signed_area2(outline) > 0.0;
        let count = outline.len();
        let outward: Vec<Vec2> = (0..count)
            .map(|i| {
                let previous = outline[(i + count - 1) % count];
                let next = outline[(i + 1) % count];
                let tangent = unit(next - previous).unwrap_or(vec2(1.0, 0.0));
                // On screen (y down) a clockwise loop's outside is to the tangent's left.
                let normal = left_normal(tangent);
                if clockwise { normal } else { -normal }
            })
            .collect();
        let base = mesh.vertices.len() as u32;
        for (point, normal) in outline.iter().zip(&outward) {
            mesh.colored_vertex(*point, color);
            mesh.colored_vertex(*point + *normal * feather, Color32::TRANSPARENT);
        }
        for i in 0..count {
            let j = (i + 1) % count;
            let (a, b) = (base + 2 * i as u32, base + 2 * j as u32);
            mesh.add_triangle(a, a + 1, b + 1);
            mesh.add_triangle(a, b + 1, b);
        }
    }
    Some(mesh)
}

pub(in crate::ui) mod annotation;
pub(in crate::ui) mod fib;
pub mod line;
pub(in crate::ui) mod pattern;
#[cfg(test)]
mod tests;
