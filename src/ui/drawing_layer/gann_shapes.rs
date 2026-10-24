//! The Gann drawings projected onto the plot: the fan, the box and the two squares. Each function
//! turns one geometry and the projection into a [`Screen`]; the names and the arithmetic of the
//! angles are `model::gann`, the curves `ui::fib_geometry`.
//!
//! **A Gann angle is a ratio of price to time, taken from the second anchor.** The 1×1 line passes
//! through it and every other angle is that line's slope times the ratio, worked on the screen
//! positions of the anchors, so the angles follow the chart's scales and a 1×1 is no fixed 45
//! degrees. The preview and the finished drawing go through the same code.

use super::Projection;
use super::fib_shapes::{LABEL_GAP, OFF_PLOT, fan_labels, fib_style, look, styled};
use super::shape::{Band, LabelLook, LabelPlace, PivotLabel, Screen, Styled, vertex_handles};
use crate::model::chart_type::Rgba;
use crate::model::drawing_style::DrawingStyle;
use crate::model::fib_style::FibLevel;
use crate::model::fib_style::FibStyle;
use crate::model::gann::{self, GannKind};
use crate::model::pattern::format_ratio;
use crate::model::{Anchor, Geometry};
use crate::ui::fib_geometry;
use crate::ui::line_geometry as lines;
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::f64::consts::PI;

/// Builds the screen shape of a Gann drawing. `anchors` are the geometry's anchors on screen,
/// already checked to be finite.
pub(super) fn project(
    geometry: &Geometry,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: Vec<Pos2>,
) -> Option<Screen> {
    let Geometry::Gann { kind, points } = geometry else {
        return None;
    };
    if anchors.len() != kind.points() || points.len() != kind.points() {
        return None;
    }
    let fib = fib_style(kind.tool(), style);
    let mut screen = match kind {
        GannKind::Fan => fan(style, &fib, proj, &anchors),
        GannKind::Box => gann_box(style, &fib, proj, points, &anchors),
        GannKind::Square | GannKind::SquareFixed => {
            square(style, &fib, proj, *kind, points, &anchors)
        }
    };
    screen.backdrop = true;
    screen.finish(style.stroke.width / 2.0 + 8.0)
}

/// The active levels with a ratio that can be an angle, by growing ratio.
fn angles(levels: impl Iterator<Item = FibLevel>) -> Vec<FibLevel> {
    let mut rows: Vec<FibLevel> = levels.filter(|level| level.ratio > 0.0).collect();
    rows.sort_by(|a, b| a.ratio.total_cmp(&b.ratio));
    rows
}

/// The label of an angle at the end of its ray, where it leaves `area`: beside that end, inside
/// it, on whichever side the ray leaves by.
fn ray_label(text: String, color: Rgba, end: Pos2, area: Rect) -> Option<PivotLabel> {
    let edge = vec2(
        if end.x >= area.right() - 0.5 {
            1.0
        } else if end.x <= area.left() + 0.5 {
            -1.0
        } else {
            0.0
        },
        if end.y <= area.top() + 0.5 {
            -1.0
        } else if end.y >= area.bottom() - 0.5 {
            1.0
        } else {
            0.0
        },
    );
    let at = pos2(end.x - edge.x * LABEL_GAP, end.y - edge.y * LABEL_GAP);
    area.expand(OFF_PLOT).contains(at).then(|| PivotLabel {
        at,
        text,
        look: LabelLook::Ink(color),
        place: LabelPlace::On,
        push: vec2(0.0, -1.0),
        edge,
        on_select: false,
    })
}

// MARK: Fan

/// Rays from the first anchor: the 1×1 through the second, the others at ratios of its slope.
fn fan(style: &DrawingStyle, fib: &FibStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let plot = proj.plot;
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    let origin = anchors[0];
    let delta = anchors[1] - origin;
    if delta.length() <= 1e-3 {
        return screen;
    }
    let alpha = style.fill.color[3];
    let mut previous: Option<Vec2> = None;
    for level in angles(fib.levels.active().copied()) {
        let through = origin + vec2(delta.x, delta.y * level.ratio);
        let direction = through - origin;
        if direction.length() <= 1e-3 {
            continue;
        }
        let direction = direction.normalized();
        let color = look(&level, style);
        if let Some(before) = previous {
            let polygon = fib_geometry::fan_band(origin, before, direction, plot);
            if polygon.len() >= 3 {
                screen.bands.push(Band {
                    polygon,
                    color: [color[0], color[1], color[2], alpha],
                });
            }
        }
        previous = Some(direction);
        if let Some((from, to)) = lines::line_in_rect(origin, through, plot, false, true) {
            screen.styled.push(styled(vec![from, to], &level, color));
            if fib.show_labels
                && let Some(label) = ray_label(gann::angle_label(level.ratio), color, to, plot)
            {
                screen.labels.push(label);
            }
        }
    }
    screen
}

// MARK: Box

/// A box between the two anchors, cut at price levels (as ratios of its price range) and time
/// levels (as ratios of its time range, in bars), with optional rays from its corners.
fn gann_box(
    style: &DrawingStyle,
    fib: &FibStyle,
    proj: &Projection,
    points: &[Anchor],
    anchors: &[Pos2],
) -> Screen {
    let plot = proj.plot;
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);
    let boxed = Rect::from_two_pos(anchors[0], anchors[1]);
    // Reversed, every ratio is counted from the other corner.
    let at = |ratio: f32| if fib.reverse { 1.0 - ratio } else { ratio };
    let first_index = proj.times.index_of(points[0].time_ms);
    let span = proj.times.index_of(points[1].time_ms) - first_index;
    let price_y = |ratio: f32| {
        let price = points[0].price + (points[1].price - points[0].price) * f64::from(at(ratio));
        proj.scale.y(price)
    };
    let time_x = |ratio: f32| proj.x_at_index(first_index + span * f64::from(at(ratio)));
    let mut rows: Vec<(FibLevel, f32, f32)> = fib
        .levels
        .active()
        .map(|level| (*level, price_y(level.ratio), 0.0))
        .filter(|(_, y, _)| y.is_finite())
        .collect();
    rows.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut columns: Vec<(FibLevel, f32)> = fib
        .second
        .active()
        .map(|level| (*level, time_x(level.ratio)))
        .filter(|(_, x)| x.is_finite())
        .collect();
    columns.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (left, right) = (boxed.left(), boxed.right());
    let (top, bottom) = (boxed.top(), boxed.bottom());
    let alpha = style.fill.color[3];
    // The strips between neighbouring price levels, and between neighbouring time levels.
    let strip = |from: Pos2, to: Pos2, color: Rgba| {
        let rect = Rect::from_two_pos(from, to).intersect(plot);
        (rect.width() > 0.0 && rect.height() > 0.0).then(|| Band {
            polygon: vec![
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
            ],
            color: [color[0], color[1], color[2], alpha],
        })
    };
    for pair in rows.windows(2) {
        let color = look(&pair[1].0, style);
        screen
            .bands
            .extend(strip(pos2(left, pair[0].1), pos2(right, pair[1].1), color));
    }
    if fib.second_fill {
        for pair in columns.windows(2) {
            let color = look(&pair[1].0, style);
            screen
                .shades
                .extend(strip(pos2(pair[0].1, top), pos2(pair[1].1, bottom), color));
        }
    }
    for (level, y, _) in &rows {
        let color = look(level, style);
        if let Some((from, to)) =
            lines::line_in_rect(pos2(left, *y), pos2(right, *y), plot, false, false)
        {
            screen.styled.push(styled(vec![from, to], level, color));
        }
        if fib.show_labels {
            fan_labels(
                &mut screen,
                fib,
                plot,
                boxed,
                true,
                level,
                color,
                pos2(0.0, *y),
            );
        }
    }
    for (level, x) in &columns {
        let color = look(level, style);
        if let Some((from, to)) =
            lines::line_in_rect(pos2(*x, top), pos2(*x, bottom), plot, false, false)
        {
            screen.styled.push(styled(vec![from, to], level, color));
        }
        if fib.show_labels {
            fan_labels(
                &mut screen,
                fib,
                plot,
                boxed,
                false,
                level,
                color,
                pos2(*x, 0.0),
            );
        }
    }
    if fib.grid.enabled {
        for path in corner_rays(boxed) {
            if let Some((from, to)) = lines::line_in_rect(path[0], path[1], plot, false, false) {
                screen.styled.push(Styled::new(vec![from, to], &fib.grid));
            }
        }
    }
    screen
}

/// The rays of a box's corners: the two diagonals, and from every corner a ray to the middle of
/// each of the two sides it is not on (the 2×1 and 1×2).
pub(super) fn corner_rays(boxed: Rect) -> Vec<[Pos2; 2]> {
    let [a, b, c, d] = [
        boxed.left_top(),
        boxed.right_top(),
        boxed.right_bottom(),
        boxed.left_bottom(),
    ];
    let mut rays = vec![[a, c], [b, d]];
    let middle = |p: Pos2, q: Pos2| p.lerp(q, 0.5);
    // Each corner, with the two sides (as their end points) it is not on.
    for (corner, far) in [
        (a, [(b, c), (d, c)]),
        (b, [(a, d), (d, c)]),
        (c, [(a, d), (a, b)]),
        (d, [(a, b), (b, c)]),
    ] {
        for (p, q) in far {
            rays.push([corner, middle(p, q)]);
        }
    }
    rays
}

// MARK: Squares

/// A square, or a box of any shape: grid levels as ratios of its sides, fan lines inside it from
/// its first corner, arcs about its corners, and optionally its ranges and ratio.
fn square(
    style: &DrawingStyle,
    fib: &FibStyle,
    proj: &Projection,
    kind: GannKind,
    points: &[Anchor],
    anchors: &[Pos2],
) -> Screen {
    let plot = proj.plot;
    let standard = kind == GannKind::Square;
    let first_index = proj.times.index_of(points[0].time_ms);
    let bars = proj.times.index_of(points[1].time_ms) - first_index;
    let (near, mut far) = (anchors[0], anchors[1]);
    // A set price per bar decides how tall the box is: as many bars across as the anchors are
    // apart, and that many times the scale up.
    let mut price_range = (points[1].price - points[0].price).abs();
    let scaled = (standard && fib.scale > 0.0)
        .then(|| gann::scaled_price(points[0].price, points[1].price, bars, f64::from(fib.scale)))
        .flatten()
        .filter(|price| proj.scale.y(*price).is_finite());
    if let Some(price) = scaled {
        far.y = proj.scale.y(price);
        price_range = (price - points[0].price).abs();
    }
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(&[near, far]);
    let (origin, opposite) = if fib.reverse {
        (far, near)
    } else {
        (near, far)
    };
    let delta = opposite - origin;
    // A box with no width or no height has no inside.
    if delta.x.abs() <= 1e-3 || delta.y.abs() <= 1e-3 {
        return screen;
    }
    let boxed = Rect::from_two_pos(origin, opposite);
    // What is drawn inside the box is drawn inside the plot too.
    let clip = boxed.intersect(plot);
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return screen;
    }
    if style.fill.enabled {
        screen.bands.push(Band {
            polygon: vec![
                clip.left_top(),
                clip.right_top(),
                clip.right_bottom(),
                clip.left_bottom(),
            ],
            color: style.fill.color,
        });
    }
    // The grid: each level a line across the box and one down it.
    for level in fib.levels.active() {
        let color = look(level, style);
        let (x, y) = (
            origin.x + delta.x * level.ratio,
            origin.y + delta.y * level.ratio,
        );
        for (from, to) in [
            (pos2(boxed.left(), y), pos2(boxed.right(), y)),
            (pos2(x, boxed.top()), pos2(x, boxed.bottom())),
        ] {
            if let Some((from, to)) = lines::line_in_rect(from, to, plot, false, false) {
                screen.styled.push(styled(vec![from, to], level, color));
            }
        }
    }
    // The fan lines, from the first corner as far as the box goes.
    for level in angles(fib.second.active().copied()) {
        let through = origin + vec2(delta.x, delta.y * level.ratio);
        let color = look(&level, style);
        let Some((from, to)) = lines::line_in_rect(origin, through, boxed, false, true) else {
            continue;
        };
        let Some((from, to)) = lines::line_in_rect(from, to, plot, false, false) else {
            continue;
        };
        screen.styled.push(styled(vec![from, to], &level, color));
        if fib.show_labels
            && let Some(label) = ray_label(gann::angle_label(level.ratio), color, to, clip)
        {
            screen.labels.push(label);
        }
    }
    arcs(&mut screen, style, fib, (origin, opposite), clip);
    if standard && fib.readout {
        readout(&mut screen, proj, boxed, (bars, price_range), delta);
    }
    screen
}

/// The arcs of a square: quarter ellipses of radius a number of its sides, about its first
/// corner or the far one, cut to the box, and the fill between neighbouring arcs of one corner.
fn arcs(
    screen: &mut Screen,
    style: &DrawingStyle,
    fib: &FibStyle,
    (origin, opposite): (Pos2, Pos2),
    clip: Rect,
) {
    let delta = opposite - origin;
    let alpha = style.fill.color[3];
    for far_corner in [false, true] {
        let center = if far_corner { opposite } else { origin };
        // Into the box from this corner: the way to the other one.
        let inward = if far_corner { -delta } else { delta };
        let (about_x, about_y) = (
            if inward.x > 0.0 { 0.0 } else { PI },
            if inward.y > 0.0 { PI / 2.0 } else { -PI / 2.0 },
        );
        let range = (about_x, about_x + fib_geometry::sweep(about_x, about_y));
        let mut rows: Vec<FibLevel> = fib
            .third
            .active()
            .filter(|arc| arc.alt == far_corner && arc.ratio > 0.0)
            .copied()
            .collect();
        rows.sort_by(|a, b| a.ratio.total_cmp(&b.ratio));
        let mut inner: Vec<Pos2> = Vec::new();
        for (index, level) in rows.iter().enumerate() {
            let (rx, ry) = (delta.x.abs() * level.ratio, delta.y.abs() * level.ratio);
            let color = look(level, style);
            if let Some(runs) = fib_geometry::arc(center, rx, ry, range, clip) {
                for run in runs.into_iter().filter(|run| run.len() >= 2) {
                    screen.styled.push(styled(run, level, color));
                }
            }
            let sector = fib_geometry::disc(center, rx, ry, range, Some(center), clip);
            if fib.second_fill && index > 0 {
                let polygon = fib_geometry::annulus(&sector, &inner);
                if polygon.len() >= 3 {
                    screen.shades.push(Band {
                        polygon,
                        color: [color[0], color[1], color[2], alpha],
                    });
                }
            }
            inner = sector;
        }
    }
}

/// The ranges and ratio a standard square can print: the bars across it, the price range up it,
/// and the price per bar they make.
fn readout(
    screen: &mut Screen,
    proj: &Projection,
    boxed: Rect,
    (bars, price_range): (f64, f64),
    delta: Vec2,
) {
    let plain = |at: Pos2, text: String, edge: Vec2| PivotLabel {
        at,
        text,
        look: LabelLook::Plain,
        place: LabelPlace::On,
        push: vec2(0.0, 1.0),
        edge,
        on_select: false,
    };
    let center = boxed.center();
    let mut labels = vec![
        plain(
            pos2(center.x, boxed.bottom() + LABEL_GAP),
            format!("{} bars", format_ratio(bars.abs(), 1)),
            vec2(0.0, -1.0),
        ),
        plain(
            pos2(boxed.right() + LABEL_GAP, center.y),
            proj.format.format(price_range),
            vec2(-1.0, 0.0),
        ),
    ];
    if let Some(rate) = gann::price_per_bar(0.0, price_range, bars) {
        let corner = pos2(
            if delta.x > 0.0 {
                boxed.right()
            } else {
                boxed.left()
            },
            if delta.y > 0.0 {
                boxed.bottom()
            } else {
                boxed.top()
            },
        );
        let sx = if delta.x > 0.0 { 1.0 } else { -1.0 };
        let sy = if delta.y > 0.0 { 1.0 } else { -1.0 };
        labels.push(plain(
            pos2(corner.x + sx * LABEL_GAP, corner.y + sy * LABEL_GAP),
            format!("1 : {}", format_ratio(rate, 4)),
            vec2(-sx, -sy),
        ));
    }
    screen.labels.extend(labels);
}
