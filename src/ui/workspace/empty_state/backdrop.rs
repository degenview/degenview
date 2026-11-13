//! The live trend line behind the top of a blank tab: a price that never stops zigzagging up and
//! down, scrolling left with its newest value at the right, a soft fill under it.
//!
//! The line is a pure function of the clock (straight segments between hashed vertices that
//! scroll past at a steady pace), so it keeps no state, never repeats and looks the same in any
//! two frames at the same time.

use super::sparkline::fade_fill;
use crate::ui::kit::loading::hash;
use crate::ui::theme;
use eframe::egui::epaint::PathStroke;
use eframe::egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};
use std::time::Duration;

/// The pause between frames while it plays (about 30 per second).
const FRAME: Duration = Duration::from_millis(33);
/// How far down the fill reaches before it is gone.
const FILL_DEPTH: f32 = 340.0;
/// Where the line wanders: this far below the top of the page, in a strip this tall. The page's
/// title starts below the strip, so no text is crossed by the line.
pub const LINE_TOP: f32 = 22.0;
pub const LINE_RANGE: f32 = 100.0;
/// Average distance between the line's turning points, in points.
const CELL: f32 = 60.0;
/// How far a turning point may sit from its even spot, in cells: it keeps the rhythm uneven
/// without ever swapping places with a neighbour.
const JITTER: f32 = 0.7;
/// Points per second the line scrolls left.
const VELOCITY: f64 = 64.0;
/// How far the slow trend of the vertices wanders over this many of them.
const TREND_SPAN: f64 = 0.16;
/// How far a vertex swings above or below the trend: at least this, and up to `TOOTH_EXTRA` more
/// (squared, so most are small and a few are sharp).
const TOOTH_MIN: f32 = 0.04;
const TOOTH_EXTRA: f32 = 0.15;
/// How much the slow trend is stretched, so the line climbs and falls over many teeth.
const TREND_GAIN: f32 = 1.5;
/// How far the line stops short of the right edge, as a share of the width: the last price.
const END_INSET: f32 = 0.07;
/// Distance between the grid's vertical lines; they scroll with the line.
const GRID_COLUMN: f32 = 96.0;
/// Horizontal grid lines across the strip the line wanders in (one more than this many gaps).
const GRID_ROWS: usize = 4;
/// How far the grid reaches above the strip, where it fades in.
const GRID_LEAD: f32 = 18.0;
/// A grid line is cut into pieces this long, each end carrying its own opacity.
const FADE_PIECE: f32 = 12.0;
/// Opacity of the grid at its strongest, as a share of the label colour.
const GRID_ALPHA: f32 = 0.09;
/// Share of the width over which the left end fades in.
const FADE_IN: f32 = 0.3;
/// Share of the width, before the last price, over which the fill fades out.
const FADE_OUT: f32 = 0.12;

/// A number in `0.0..1.0` that changes smoothly with `at` and is arbitrary but fixed for a given
/// position and `salt`.
fn noise(at: f64, salt: u64) -> f32 {
    let cell = at.floor();
    let fraction = (at - cell) as f32;
    let eased = fraction * fraction * fraction * (fraction * (fraction * 6.0 - 15.0) + 10.0);
    let before = hash(cell as i64, salt);
    let after = hash(cell as i64 + 1, salt);
    before + (after - before) * eased
}

/// The `k`th turning point of the line: where it is along the scroll, and how high (`0.0..=1.0`).
/// Every other one is a peak and the ones between are dips, around a trend that drifts slowly, so
/// the line zigzags while it climbs and falls.
fn vertex(k: i64) -> (f32, f32) {
    let x = (k as f32 + (hash(k, 7) - 0.5) * JITTER) * CELL;
    let trend = noise(k as f64 * TREND_SPAN, 11);
    let tooth = TOOTH_MIN + TOOTH_EXTRA * hash(k, 29).powi(2);
    let sign = if k.rem_euclid(2) == 0 { 1.0 } else { -1.0 };
    // The trend alone stays near the middle; spread it back out, softly, so a high stretch is
    // rounded off instead of running flat along the top.
    let level = 0.5 + 0.5 * (2.0 * (TREND_GAIN * (trend - 0.5) + sign * tooth)).tanh();
    (x, level)
}

/// The line `width` points wide at `time` seconds, as `(x, level)` from `x = 0` to `x = width`
/// with levels in `0.0..=1.0`: its turning points, and the two ends cut from the segments that
/// cross them.
pub fn zigzag(time: f64, width: f32) -> Vec<(f32, f32)> {
    let shift = time * VELOCITY;
    let first = (shift / f64::from(CELL)).floor() as i64 - 1;
    let last = ((shift + f64::from(width)) / f64::from(CELL)).ceil() as i64 + 1;
    let vertices: Vec<(f32, f32)> = (first..=last)
        .map(|k| {
            let (x, level) = vertex(k);
            // Relative to the scroll, in f64 first: `shift` grows without bound.
            ((f64::from(x) - shift) as f32, level)
        })
        .collect();
    let mut line = vec![(0.0, level_at(&vertices, 0.0))];
    line.extend(
        vertices
            .iter()
            .copied()
            .filter(|&(x, _)| x > 0.0 && x < width),
    );
    line.push((width, level_at(&vertices, width)));
    line
}

/// The height of the polyline `points` (ascending in `x`) at `x`.
fn level_at(points: &[(f32, f32)], x: f32) -> f32 {
    let after = points.partition_point(|&(point_x, _)| point_x < x);
    let (before, after) = (
        points[after.max(1) - 1],
        points[after.min(points.len() - 1)],
    );
    let span = after.0 - before.0;
    if span <= f32::EPSILON {
        before.1
    } else {
        before.1 + (after.1 - before.1) * ((x - before.0) / span)
    }
}

/// The x of each vertical grid line in a picture `width` wide at `time` seconds, ascending. They
/// scroll left at the line's pace, so the grid reads as the chart the line is drawn on.
pub fn grid_columns(time: f64, width: f32) -> Vec<f32> {
    let offset = (time * VELOCITY).rem_euclid(f64::from(GRID_COLUMN)) as f32;
    (0..)
        .map(|index| index as f32 * GRID_COLUMN - offset)
        .take_while(|&x| x <= width)
        .filter(|&x| x >= 0.0)
        .collect()
}

fn smoothstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

/// Paints the picture in the top of `area` and asks for the next frame. It keeps playing while the
/// window is behind another (the page is still on screen); a window that is hidden or minimised
/// is not drawn by the system, so it costs nothing then.
pub fn show(ui: &Ui, area: Rect) {
    let time = ui.input(|input| input.time);
    paint(&ui.painter().with_clip_rect(area), area, time);
    ui.ctx().request_repaint_after(FRAME);
}

/// The grid behind the line: faint, fading in from the left, out toward the right edge's last
/// price and at its top and bottom, so it dissolves into the page instead of ending in a frame.
fn paint_grid(painter: &Painter, band: Rect, time: f64) {
    let top = band.top() + LINE_TOP - GRID_LEAD;
    let bottom = band.top() + LINE_TOP + LINE_RANGE;
    let (left, right) = (band.left(), band.right());
    let color = theme::ink(255);
    let alpha = theme::pick_value(GRID_ALPHA, GRID_ALPHA * 0.8);
    let width = band.width();
    let weight = move |at: Pos2| {
        let across = smoothstep((at.x - left) / (width * FADE_IN))
            * smoothstep((right - at.x) / (width * FADE_IN));
        let down = smoothstep((at.y - top) / GRID_LEAD) * smoothstep((bottom - at.y) / GRID_LEAD);
        alpha * across * down
    };
    for row in 0..=GRID_ROWS {
        let y = band.top() + LINE_TOP + LINE_RANGE * row as f32 / GRID_ROWS as f32;
        faded_line(painter, pos2(left, y), pos2(right, y), color, &weight);
    }
    for x in grid_columns(time, width) {
        faded_line(
            painter,
            pos2(left + x, top),
            pos2(left + x, bottom),
            color,
            &weight,
        );
    }
}

/// A one-point line from `from` to `to` (horizontal or vertical) whose opacity at each spot is
/// `color` times `weight(spot)`: a strip of quads with a colour on every vertex.
fn faded_line(
    painter: &Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    weight: &impl Fn(Pos2) -> f32,
) {
    let along = to - from;
    let length = along.length();
    if length < 1.0 {
        return;
    }
    let side = vec2(-along.y, along.x) / length * 0.5;
    let pieces = (length / FADE_PIECE).ceil() as u32;
    let mut mesh = Mesh::default();
    for piece in 0..=pieces {
        let at = from + along * (piece as f32 / pieces as f32);
        let tint = color.gamma_multiply(weight(at).clamp(0.0, 1.0));
        mesh.colored_vertex(at - side, tint);
        mesh.colored_vertex(at + side, tint);
    }
    for piece in 0..pieces {
        let first = piece * 2;
        mesh.add_triangle(first, first + 1, first + 2);
        mesh.add_triangle(first + 1, first + 3, first + 2);
    }
    painter.add(Shape::mesh(mesh));
}

/// The picture at `time`.
pub fn paint(painter: &Painter, area: Rect, time: f64) {
    let band = Rect::from_min_size(area.min, vec2(area.width(), FILL_DEPTH.min(area.height())));
    if band.width() < 120.0 || band.height() < LINE_TOP + LINE_RANGE {
        return;
    }
    let left = band.left();
    let end = band.right() - band.width() * END_INSET;
    let line: Vec<Pos2> = zigzag(time, end - left)
        .into_iter()
        .map(|(x, level)| pos2(left + x, band.top() + LINE_TOP + LINE_RANGE * (1.0 - level)))
        .collect();
    let fade = band.width() * FADE_IN;
    let weight = move |x: f32| smoothstep((x - left) / fade);
    let fill_weight = move |x: f32| weight(x) * smoothstep((end - x) / (band.width() * FADE_OUT));
    let color = theme::UP;

    paint_grid(painter, band, time);
    fade_fill(
        painter,
        &line,
        band.bottom(),
        color,
        theme::pick_value(0.24, 0.18),
        fill_weight,
    );
    // A halo under the stroke, then the stroke itself, each fading in from the left.
    for (width, alpha) in [(9.0, 0.05), (5.0, 0.1), (1.8, 0.95)] {
        painter.add(Shape::line(
            line.clone(),
            PathStroke::new_uv(width, move |_, point| {
                color.gamma_multiply(alpha * weight(point.x))
            }),
        ));
    }

    // The latest price: a dashed line out to the edge and a dot that pulses.
    let Some(&dot) = line.last() else {
        return;
    };
    painter.add(Shape::dashed_line(
        &[dot, pos2(band.right(), dot.y)],
        Stroke::new(1.0, color.gamma_multiply(0.35)),
        4.0,
        4.0,
    ));
    let pulse = ((time * 2.2).sin() * 0.5 + 0.5) as f32;
    painter.circle_filled(
        dot,
        5.0 + 6.0 * pulse,
        color.gamma_multiply(0.22 * (1.0 - pulse)),
    );
    painter.circle_filled(dot, 3.4, color);
    painter.circle_stroke(dot, 3.4, Stroke::new(1.0, Color32::from_black_alpha(40)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_spans_its_width_with_levels_in_range() {
        let line = zigzag(12.5, 1800.0);
        assert_eq!(line.first().map(|point| point.0), Some(0.0));
        assert_eq!(line.last().map(|point| point.0), Some(1800.0));
        assert!(line.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(line.iter().all(|&(_, level)| (0.0..=1.0).contains(&level)));
        assert_eq!(line, zigzag(12.5, 1800.0));
    }

    #[test]
    fn it_zigzags_with_a_turn_every_few_dozen_points() {
        let line = zigzag(3.0, 1800.0);
        let turns = line
            .windows(3)
            .filter(|three| (three[1].1 - three[0].1) * (three[2].1 - three[1].1) < 0.0)
            .count();
        let expected = 1800.0 / CELL;
        assert!(
            (turns as f32) > expected * 0.6,
            "{turns} turns in {expected} cells"
        );
        let low = line.iter().map(|point| point.1).fold(f32::MAX, f32::min);
        let high = line.iter().map(|point| point.1).fold(f32::MIN, f32::max);
        assert!(high - low > 0.3, "a flat line is not a trend");
    }

    #[test]
    fn it_scrolls_left_without_jumping_from_one_frame_to_the_next() {
        let step = 1.0 / 30.0;
        let before = zigzag(7.0, 1800.0);
        let after = zigzag(7.0 + step, 1800.0);
        // A frame later the same shape has moved left by the scroll speed.
        let moved = (VELOCITY * step) as f32;
        let furthest = (50..1700)
            .map(|x| x as f32)
            .map(|x| (level_at(&before, x + moved) - level_at(&after, x)).abs())
            .fold(0.0, f32::max);
        assert!(
            furthest < 0.01,
            "the shape is unchanged by the scroll: {furthest}"
        );
    }

    #[test]
    fn the_grid_scrolls_with_the_line_and_stays_evenly_spaced() {
        let columns = grid_columns(5.0, 1000.0);
        assert!(columns.len() >= 10);
        assert!(columns.iter().all(|x| (0.0..=1000.0).contains(x)));
        assert!(
            columns
                .windows(2)
                .all(|pair| (pair[1] - pair[0] - GRID_COLUMN).abs() < 0.01)
        );
        // A frame later every line has moved left by the line's own scroll.
        let step = 1.0 / 30.0;
        let later = grid_columns(5.0 + step, 1000.0);
        let moved = (VELOCITY * step) as f32;
        assert!(columns.iter().all(|&x| {
            x < moved
                || later
                    .iter()
                    .any(|&other| (other - (x - moved)).abs() < 0.01)
        }));
    }

    #[test]
    fn it_never_comes_back_to_the_same_picture() {
        let first = zigzag(0.0, 600.0);
        assert!(
            (1..200).all(|second| zigzag(f64::from(second) * 3.7, 600.0) != first),
            "the vertices are hashed, not periodic"
        );
    }
}
