//! The lines, channels and pitchforks projected onto the plot: the visible pieces of every line
//! (those with no end are cut to the plot here, never drawn from a far-away coordinate), their
//! fills, handles, tags and text. Each function turns one geometry and the projection into a
//! [`Screen`]; the construction itself lives in `ui::line_geometry` and `model::regression`.

use super::Projection;
use super::edit::Part;
use super::shape::{
    AngleMark, Band, EndLabel, HandleSpot, Readout, Screen, Styled, Tag, TextPlace, with_heads,
};
use crate::model::drawing_style::{DrawingStyle, LineEnd};
use crate::model::regression;
use crate::model::{Anchor, Geometry};
use crate::ui::line_geometry::{self as lines, FORK_LENGTH};
use eframe::egui::{Pos2, Vec2, pos2, vec2};

/// How far (px) beyond the plot a line at a price or a time may be and still be built: its
/// stroke may reach in.
const OFF_PLOT: f32 = 12.0;
/// The most points one regression line is sampled at on a logarithmic scale.
const MAX_SAMPLES: usize = 200;
/// The spacing (px) of those samples.
const SAMPLE_STEP: f32 = 8.0;

fn spot(index: usize, pos: Pos2) -> HandleSpot {
    HandleSpot {
        part: Part::Anchor(index),
        pos,
        control: false,
    }
}

/// The direction away from `other` at `point`, level to the right when they coincide.
fn outward(point: Pos2, other: Pos2) -> Vec2 {
    let away = point - other;
    if away.length() > 1e-3 {
        away.normalized()
    } else {
        vec2(1.0, 0.0)
    }
}

/// `screen`, with a piece of a line added when it is visible.
fn push(screen: &mut Screen, piece: Option<(Pos2, Pos2)>) {
    if let Some((from, to)) = piece {
        screen.lines.push(vec![from, to]);
    }
}

/// Builds the screen shape of a line, channel or pitchfork. `anchors` are the geometry's
/// anchors on screen, already checked to be finite.
pub(super) fn project(
    geometry: &Geometry,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: Vec<Pos2>,
) -> Option<Screen> {
    let stored = geometry.anchors();
    let screen = match geometry {
        Geometry::TrendLine { .. }
        | Geometry::Ray { .. }
        | Geometry::InfoLine { .. }
        | Geometry::ExtendedLine { .. }
        | Geometry::TrendAngle { .. } => line_family(geometry, style, proj, &anchors),
        Geometry::HorizontalLine { .. } => level(style, proj, anchors[0], stored[0], false),
        Geometry::HorizontalRay { .. } => level(style, proj, anchors[0], stored[0], true),
        Geometry::VerticalLine { .. } => vertical(style, proj, anchors[0], stored[0]),
        Geometry::Crossline { .. } => cross(style, proj, anchors[0], stored[0]),
        Geometry::ParallelChannel { .. } => parallel(style, proj, &anchors),
        Geometry::FlatChannel { .. } => flat(style, proj, &anchors, &stored),
        Geometry::DisjointChannel { .. } => disjoint(style, proj, &anchors, &stored),
        Geometry::Regression { .. } => regression_trend(style, proj, &anchors, &stored),
        Geometry::Pitchfork { variant, .. } => pitchfork(*variant, style, proj, &anchors),
        _ => return None,
    };
    screen.finish(style.stroke.width / 2.0 + 8.0)
}

// MARK: Lines

/// The trend line, ray, info line, extended line and trend angle: one line through `a` and `b`,
/// continued past either by the style's extension flags.
fn line_family(
    geometry: &Geometry,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: &[Pos2],
) -> Screen {
    let (pa, pb) = (anchors[0], anchors[1]);
    let plot = proj.plot;
    let (before, after) = (style.extend_left, style.extend_right);
    let mut screen = Screen::empty();
    if let Some((from, to)) = lines::line_in_rect(pa, pb, plot, before, after) {
        // An end that is clipped or continued has no anchor to carry a head.
        let head = |wanted: LineEnd, anchored: bool, end: Pos2, anchor: Pos2| {
            if anchored && end.distance(anchor) < 0.5 {
                wanted
            } else {
                LineEnd::Plain
            }
        };
        let (path, heads) = with_heads(
            vec![from, to],
            head(style.head_start, !before, from, pa),
            head(style.head_end, !after, to, pb),
            style.stroke.width,
        );
        screen.lines.push(path);
        screen.solids = heads;
    }
    screen.arrow = Some((pa, pb));
    screen.middle_dot = style.middle_point.then(|| pa.lerp(pb, 0.5));
    // Its end prices are printed with the statistics (see `draw_arrow_extras`).
    if matches!(geometry, Geometry::TrendAngle { .. }) {
        // Read from the left end, so the angle is the line's rise to the right.
        let (origin, toward) = if pa.x <= pb.x { (pa, pb) } else { (pb, pa) };
        screen.angle = Some(AngleMark { origin, toward });
    }
    screen.handles = vec![spot(0, pa), spot(1, pb)];
    // The text follows the stretch between the anchors, or what shows of the line if that is
    // out of view.
    screen.text = lines::line_in_rect(pa, pb, plot, false, false)
        .or_else(|| lines::line_in_rect(pa, pb, plot, before, after))
        .map(|(from, to)| TextPlace::Line { from, to });
    screen
}

/// A horizontal line across the plot, or (`ray`) a horizontal ray from its origin to the right.
fn level(style: &DrawingStyle, proj: &Projection, at: Pos2, stored: Anchor, ray: bool) -> Screen {
    let plot = proj.plot;
    let mut screen = Screen::empty();
    let y = at.y;
    let in_view = y >= plot.top() - OFF_PLOT && y <= plot.bottom() + OFF_PLOT;
    let left = if ray {
        at.x.max(plot.left())
    } else {
        plot.left()
    };
    if in_view && left <= plot.right() {
        let (from, to) = (pos2(left, y), pos2(plot.right(), y));
        screen.lines.push(vec![from, to]);
        screen.text = Some(TextPlace::Line { from, to });
        let handle = if ray {
            (at.x >= plot.left() && at.x <= plot.right()).then_some(at)
        } else {
            Some(pos2(plot.center().x, y))
        };
        screen.handles = handle.map(|pos| spot(0, pos)).into_iter().collect();
    }
    if style.price_labels && y >= plot.top() && y <= plot.bottom() && left <= plot.right() {
        screen.tags.push(Tag::Price {
            y,
            price: stored.price,
            color: style.stroke.color,
        });
    }
    screen
}

/// A vertical line from the top of the plot to its bottom.
fn vertical(style: &DrawingStyle, proj: &Projection, at: Pos2, stored: Anchor) -> Screen {
    let plot = proj.plot;
    let mut screen = Screen::empty();
    let x = at.x;
    if x >= plot.left() - OFF_PLOT && x <= plot.right() + OFF_PLOT {
        screen
            .lines
            .push(vec![pos2(x, plot.top()), pos2(x, plot.bottom())]);
        screen.text = Some(TextPlace::Column {
            x,
            top: plot.top(),
            bottom: plot.bottom(),
        });
        screen.handles = vec![spot(0, pos2(x, plot.center().y))];
        if style.time_label && x >= plot.left() && x <= plot.right() {
            screen.tags.push(Tag::Time {
                x,
                time_ms: stored.time_ms,
                color: style.stroke.color,
            });
        }
    }
    screen
}

/// A horizontal and a vertical line through one point.
fn cross(style: &DrawingStyle, proj: &Projection, at: Pos2, stored: Anchor) -> Screen {
    let plot = proj.plot;
    let mut screen = Screen::empty();
    let (x, y) = (at.x, at.y);
    let across = y >= plot.top() - OFF_PLOT && y <= plot.bottom() + OFF_PLOT;
    let down = x >= plot.left() - OFF_PLOT && x <= plot.right() + OFF_PLOT;
    if across {
        screen
            .lines
            .push(vec![pos2(plot.left(), y), pos2(plot.right(), y)]);
    }
    if down {
        screen
            .lines
            .push(vec![pos2(x, plot.top()), pos2(x, plot.bottom())]);
    }
    screen.handles = vec![spot(0, at)];
    if style.price_labels && y >= plot.top() && y <= plot.bottom() {
        screen.tags.push(Tag::Price {
            y,
            price: stored.price,
            color: style.stroke.color,
        });
    }
    if style.time_label && x >= plot.left() && x <= plot.right() {
        screen.tags.push(Tag::Time {
            x,
            time_ms: stored.time_ms,
            color: style.stroke.color,
        });
    }
    screen
}

// MARK: Channels

/// A parallelogram from the line `a`→`b` and its copy moved by `offset`, with either side run on
/// for as far as the plot reaches.
pub(super) fn band(
    a: Pos2,
    b: Pos2,
    offset: Vec2,
    before: bool,
    after: bool,
    proj: &Projection,
) -> Vec<Pos2> {
    let edge = b - a;
    let length = edge.length();
    if length < 1e-3 {
        return Vec::new();
    }
    let direction = edge / length;
    let run = lines::reach(a, proj.plot) + lines::reach(b, proj.plot) + length + offset.length();
    let start = if before { a - direction * run } else { a };
    let end = if after { b + direction * run } else { b };
    lines::clip_polygon(&[start, end, end + offset, start + offset], proj.plot)
}

/// Two parallel edges: `a`→`b`, and the one through `c`. The middle line and the extra levels sit
/// at ratios of the offset between them.
fn parallel(style: &DrawingStyle, proj: &Projection, anchors: &[Pos2]) -> Screen {
    let (pa, pb, pc) = (anchors[0], anchors[1], anchors[2]);
    let plot = proj.plot;
    let (before, after) = (style.extend_left, style.extend_right);
    let offset = lines::channel_offset(pa, pb, pc);
    let at = |ratio: f32| (pa + offset * ratio, pb + offset * ratio);
    let mut screen = Screen::empty();
    for ratio in [0.0, 1.0] {
        let (from, to) = at(ratio);
        push(
            &mut screen,
            lines::line_in_rect(from, to, plot, before, after),
        );
    }
    let mut level = |ratio: f32, line: &crate::model::drawing_style::MiddleLine| {
        let (from, to) = at(ratio);
        if let Some((from, to)) = lines::line_in_rect(from, to, plot, before, after) {
            screen.styled.push(Styled::new(vec![from, to], line));
        }
    };
    if style.middle_line.enabled {
        level(0.5, &style.middle_line);
    }
    for extra in style
        .channel_levels
        .iter()
        .filter(|extra| extra.line.enabled)
    {
        level(extra.ratio, &extra.line);
    }
    screen.fill = band(pa, pb, offset, before, after, proj);
    let (second_a, second_b) = at(1.0);
    screen.handles = vec![
        spot(0, pa),
        spot(1, pb),
        spot(2, second_a.lerp(second_b, 0.5)),
    ];
    screen.text = lines::line_in_rect(pa, pb, plot, false, false)
        .or_else(|| lines::line_in_rect(pa, pb, plot, before, after))
        .map(|(from, to)| TextPlace::Line { from, to });
    screen
}

/// A sloped edge `a`→`b` and a level one at `c`'s price over the same times.
fn flat(style: &DrawingStyle, proj: &Projection, anchors: &[Pos2], stored: &[Anchor]) -> Screen {
    let (pa, pb, pc) = (anchors[0], anchors[1], anchors[2]);
    let plot = proj.plot;
    let (before, after) = (style.extend_left, style.extend_right);
    let (fa, fb) = (pos2(pa.x, pc.y), pos2(pb.x, pc.y));
    let mut screen = Screen::empty();
    push(
        &mut screen,
        lines::line_in_rect(pa, pb, plot, before, after),
    );
    push(
        &mut screen,
        lines::line_in_rect(fa, fb, plot, before, after),
    );
    // The region between them, over the times they cover and as far as they are run on.
    let (left_runs, right_runs) = if pa.x <= pb.x {
        (before, after)
    } else {
        (after, before)
    };
    let low = if left_runs {
        plot.left() - 1.0
    } else {
        pa.x.min(pb.x)
    };
    let high = if right_runs {
        plot.right() + 1.0
    } else {
        pa.x.max(pb.x)
    };
    if let (Some(y_low), Some(y_high)) = (lines::y_at(pa, pb, low), lines::y_at(pa, pb, high)) {
        screen.fill = lines::clip_polygon(
            &[
                pos2(low, y_low),
                pos2(high, y_high),
                pos2(high, pc.y),
                pos2(low, pc.y),
            ],
            plot,
        );
    }
    screen.handles = vec![spot(0, pa), spot(1, pb), spot(2, fa.lerp(fb, 0.5))];
    if style.price_labels {
        screen.end_labels = vec![
            EndLabel {
                at: pa,
                price: stored[0].price,
                outward: outward(pa, pb),
            },
            EndLabel {
                at: pb,
                price: stored[1].price,
                outward: outward(pb, pa),
            },
            EndLabel {
                at: fa.lerp(fb, 0.5),
                price: stored[2].price,
                outward: vec2(0.0, if pc.y <= pa.y.min(pb.y) { -1.0 } else { 1.0 }),
            },
        ];
    }
    screen.text = lines::line_in_rect(pa, pb, plot, false, false)
        .or_else(|| lines::line_in_rect(pa, pb, plot, before, after))
        .map(|(from, to)| TextPlace::Line { from, to });
    screen
}

/// Two parallel edges that start and end at their own times: `a`→`b`, and the one from `c` to the
/// time of `d`.
fn disjoint(
    style: &DrawingStyle,
    proj: &Projection,
    anchors: &[Pos2],
    stored: &[Anchor],
) -> Screen {
    let (pa, pb, pc, pd) = (anchors[0], anchors[1], anchors[2], anchors[3]);
    let plot = proj.plot;
    let (before, after) = (style.extend_left, style.extend_right);
    let pe = lines::disjoint_end(pa, pb, pc, pd.x);
    let mut screen = Screen::empty();
    push(
        &mut screen,
        lines::line_in_rect(pa, pb, plot, before, after),
    );
    push(
        &mut screen,
        lines::line_in_rect(pc, pe, plot, before, after),
    );
    // Run the second edge the same way as the first, so the region has no twist.
    let (start, end) = if (pe - pc).dot(pb - pa) >= 0.0 {
        (pc, pe)
    } else {
        (pe, pc)
    };
    screen.fill = lines::clip_polygon(&[pa, pb, end, start], plot);
    screen.handles = vec![spot(0, pa), spot(1, pb), spot(2, pc), spot(3, pe)];
    screen.text = lines::line_in_rect(pa, pb, plot, false, false)
        .or_else(|| lines::line_in_rect(pa, pb, plot, before, after))
        .map(|(from, to)| TextPlace::Line { from, to });
    if style.price_labels {
        // The second edge ends where it meets the time of `d`, so its price is read off the scale.
        let ends = [
            (pa, pb, stored[0].price),
            (pb, pa, stored[1].price),
            (pc, pe, stored[2].price),
            (pe, pc, proj.scale.price(pe.y)),
        ];
        screen.end_labels = ends
            .into_iter()
            .map(|(at, other, price)| EndLabel {
                at,
                price,
                outward: outward(at, other),
            })
            .collect();
    }
    screen
}

// MARK: Regression

/// The regression trend: the least-squares line of the chosen price over the bars between its
/// two anchors, and the bands the chosen number of standard deviations either side.
fn regression_trend(
    style: &DrawingStyle,
    proj: &Projection,
    anchors: &[Pos2],
    stored: &[Anchor],
) -> Screen {
    let plot = proj.plot;
    let options = &style.regression;
    let bars = proj.times.bars();
    let mut screen = Screen::empty();
    let (first, second) = (&stored[0], &stored[1]);
    let span = regression::bar_span(bars, first.time_ms, second.time_ms);
    // A logarithmic scale is fitted in logs; a price that cannot be (zero, negative) is fitted
    // as it is, which is the honest answer.
    let fitted = span.and_then(|_| {
        let fit =
            |log| regression::fit_bars(bars, first.time_ms, second.time_ms, options.source, log);
        if proj.scale.logarithmic {
            fit(true).or_else(|| fit(false))
        } else {
            fit(false)
        }
    });
    let (Some((lo, hi)), Some(fit)) = (span, fitted) else {
        // Nothing to fit (one bar, or none): show where it was put.
        screen.lines.push(vec![anchors[0], anchors[1]]);
        screen.handles = vec![spot(0, anchors[0]), spot(1, anchors[1])];
        return screen;
    };

    // Positions along the run, from the first bar to the last (or on to the plot's right edge).
    let last = if style.extend_right {
        (proj.index_at(plot.right()).ceil()).max(hi as f64)
    } else {
        hi as f64
    };
    let first_x = proj.x_at_index(lo as f64);
    let last_x = proj.x_at_index(last);
    let samples = if fit.log {
        (((last_x - first_x) / SAMPLE_STEP).ceil() as usize).clamp(1, MAX_SAMPLES)
    } else {
        1
    };
    let path = |deviations: f64| -> Vec<Pos2> {
        (0..=samples)
            .map(|step| {
                let index = lo as f64 + (last - lo as f64) * step as f64 / samples as f64;
                let price = fit.band_at(index - lo as f64, deviations);
                pos2(proj.x_at_index(index), proj.scale.y(price))
            })
            .collect()
    };
    let base = path(0.0);
    let upper = path(f64::from(options.upper_dev));
    let lower = path(f64::from(options.lower_dev));
    if options.base.enabled {
        screen.styled.push(Styled::new(base.clone(), &options.base));
    }
    if options.upper.enabled {
        screen
            .styled
            .push(Styled::new(upper.clone(), &options.upper));
        screen.bands.push(Band {
            polygon: lines::clip_polygon(&between(&upper, &base), plot),
            color: style.fill.color,
        });
    }
    if options.lower.enabled {
        screen
            .styled
            .push(Styled::new(lower.clone(), &options.lower));
        screen.bands.push(Band {
            polygon: lines::clip_polygon(&between(&lower, &base), plot),
            color: style.fill.color,
        });
    }
    // Hit-testing needs a line even when all three are hidden.
    if screen.styled.is_empty() {
        screen.lines.push(base.clone());
    }
    if options.pearson
        && let Some(end) = base.last()
    {
        screen.readout = Some(Readout {
            at: *end,
            text: format!("Pearson's R: {:.2}", fit.r),
        });
    }
    let handle_at = |time_ms: i64| {
        let (bar, _) = regression::bar_span(bars, time_ms, time_ms).unwrap_or((lo, hi));
        let index = bar as f64;
        pos2(
            proj.x_at_index(index),
            proj.scale.y(fit.value_at(index - lo as f64)),
        )
    };
    screen.handles = vec![
        spot(0, handle_at(first.time_ms)),
        spot(1, handle_at(second.time_ms)),
    ];
    screen
}

/// The polygon between two lines drawn left to right: along `top`, then back along `bottom`.
fn between(top: &[Pos2], bottom: &[Pos2]) -> Vec<Pos2> {
    top.iter().chain(bottom.iter().rev()).copied().collect()
}

// MARK: Pitchforks

/// One line of a fork before it is cut to the plot.
struct Prong {
    /// Where it sits across the fork: the median is 0, a level is its ratio on either side.
    key: f32,
    start: Pos2,
    end: Pos2,
    color: crate::model::chart_type::Rgba,
    width: f32,
    dash: crate::model::chart_type::LineDash,
    median: bool,
}

/// An Andrews pitchfork of the given variant through three pivots.
fn pitchfork(
    variant: crate::model::PitchforkVariant,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: &[Pos2],
) -> Screen {
    let (p1, p2, p3) = (anchors[0], anchors[1], anchors[2]);
    let plot = proj.plot;
    let (before, after) = (style.extend_left, style.extend_right);
    let mut screen = Screen::empty();
    screen.handles = vec![spot(0, p1), spot(1, p2), spot(2, p3)];
    screen.guides = vec![[p1, p2], [p2, p3]];
    let Some(fork) = lines::fork(variant, p1, p2, p3) else {
        screen.lines.push(vec![p1, p2]);
        return screen;
    };

    let one = |color| {
        if style.use_one_color {
            style.stroke.color
        } else {
            color
        }
    };
    let mut prongs = vec![Prong {
        key: 0.0,
        start: fork.origin,
        end: fork.median_end(),
        color: style.stroke.color,
        width: style.stroke.width,
        dash: style.stroke.dash,
        median: true,
    }];
    for level in style
        .pitchfork_levels
        .iter()
        .filter(|level| level.line.enabled && level.ratio != 0.0)
    {
        for side in [-1.0, 1.0] {
            prongs.push(Prong {
                key: level.ratio * side,
                start: fork.level_start(level.ratio, side),
                end: fork.level_end(level.ratio, side),
                color: one(level.line.color),
                width: level.line.width,
                dash: level.line.dash,
                median: false,
            });
        }
    }
    prongs.sort_by(|a, b| a.key.total_cmp(&b.key));

    for prong in &prongs {
        let piece = lines::line_in_rect(prong.start, prong.end, plot, before, after);
        if prong.median {
            push(&mut screen, piece);
        } else if let Some((from, to)) = piece {
            screen.styled.push(Styled {
                path: vec![from, to],
                color: prong.color,
                width: prong.width,
                dash: prong.dash,
            });
        }
    }

    // The regions between neighbouring lines, from the base on, in the colour of the outer line.
    let direction = fork.handle.normalized();
    let run = lines::reach(fork.origin, plot) + fork.handle.length() * FORK_LENGTH;
    let along = |prong: &Prong| {
        let from = if prong.median {
            fork.origin + fork.handle * fork.start
        } else {
            prong.start
        };
        let start = if before { from - direction * run } else { from };
        let end = if after {
            from + direction * run
        } else {
            prong.end
        };
        (start, end)
    };
    for pair in prongs.windows(2) {
        let outer = if pair[0].key.abs() >= pair[1].key.abs() {
            &pair[0]
        } else {
            &pair[1]
        };
        let (s0, e0) = along(&pair[0]);
        let (s1, e1) = along(&pair[1]);
        let color = if outer.median {
            style.stroke.color
        } else {
            outer.color
        };
        screen.bands.push(Band {
            polygon: lines::clip_polygon(&[s0, e0, e1, s1], plot),
            color: [color[0], color[1], color[2], style.fill.color[3]],
        });
    }
    screen
}
