//! The chart patterns, Elliott waves and cycles projected onto the plot: their legs, fills,
//! pivot names, Fibonacci ratios and repeats. Each function turns one geometry and the
//! projection into a [`Screen`]; the ratios themselves are `model::pattern`, the construction of
//! the curves `ui::pattern_geometry`.
//!
//! The preview while a drawing is placed and the finished drawing go through the same code, so
//! one never differs from the other.

use super::Projection;
use super::shape::{Band, LabelLook, LabelPlace, PivotLabel, Screen, Styled, vertex_handles};
use crate::model::chart_type::{LineDash, Rgba};
use crate::model::cycle::{self, CycleKind};
use crate::model::drawing_style::DrawingStyle;
use crate::model::elliott::WaveKind;
use crate::model::pattern::{self, PatternKind};
use crate::model::{Anchor, Geometry};
use crate::ui::line_geometry as lines;
use crate::ui::pattern_geometry as curves;
use eframe::egui::{Pos2, pos2, vec2};

/// How far (px) beyond the pivots the labels and plates may reach, for the shape's bounds.
const LABEL_REACH: f32 = 48.0;
/// The width (px) of the dashed lines a ratio is measured along.
const MEASURE_WIDTH: f32 = 1.0;
/// The line joining a cyclic line's two anchors, as the Charting Library draws it.
const JOINING_LINE: Rgba = [128, 128, 128, 255];
/// Repeats closer than this (px) are not drawn: a hairline wall is not a cycle you can read.
const MIN_LINE_SPACING: f32 = 3.0;
/// Arcs narrower than this (px) are not drawn.
const MIN_ARC_WIDTH: f32 = 6.0;
/// How far (px) past the plot a triangle's lines are run when their apex is farther off.
const APEX_REACH: f32 = 200.0;

/// Builds the screen shape of a chart pattern, an Elliott wave or a cycle. `anchors` are the
/// geometry's anchors on screen, already checked to be finite.
pub(super) fn project(
    geometry: &Geometry,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: Vec<Pos2>,
) -> Option<Screen> {
    let screen = match geometry {
        Geometry::Pattern { kind, points } => chart_pattern(*kind, points, style, proj, &anchors),
        Geometry::Wave { kind, points } => elliott_wave(*kind, points, style, &anchors),
        Geometry::Cycle { kind, a, b } => cycle_shape(*kind, *a, *b, style, proj, &anchors),
        _ => return None,
    }?;
    screen.finish(style.stroke.width / 2.0 + LABEL_REACH)
}

/// A fill of the polygon `corners`, cut to the plot.
fn fill(proj: &Projection, style: &DrawingStyle, corners: &[Pos2]) -> Option<Band> {
    let polygon = lines::clip_polygon(corners, proj.plot);
    (polygon.len() >= 3).then_some(Band {
        polygon,
        color: style.fill.color,
    })
}

/// The midpoint of two points.
fn middle(a: Pos2, b: Pos2) -> Pos2 {
    a.lerp(b, 0.5)
}

/// A dashed line a ratio is measured along.
fn measuring(from: Pos2, to: Pos2, style: &DrawingStyle) -> Styled {
    Styled {
        path: vec![from, to],
        color: style.stroke.color,
        width: MEASURE_WIDTH,
        dash: LineDash::Dashed,
    }
}

// MARK: Chart patterns

fn chart_pattern(
    kind: PatternKind,
    points: &[Anchor],
    style: &DrawingStyle,
    proj: &Projection,
    anchors: &[Pos2],
) -> Option<Screen> {
    if anchors.len() != kind.points() || points.len() != kind.points() {
        return None;
    }
    let mut screen = Screen::empty();
    screen.lines.push(anchors.to_vec());

    // The legs' fills, and the lines that belong to the pattern but are not its legs.
    let mut regions: Vec<Vec<Pos2>> = Vec::new();
    match kind {
        PatternKind::Xabcd | PatternKind::Cypher => {
            regions.push(vec![anchors[0], anchors[1], anchors[2]]);
            regions.push(vec![anchors[2], anchors[3], anchors[4]]);
        }
        PatternKind::HeadAndShoulders => {
            // The head and the two shoulders, each the part of the zigzag above the neckline,
            // closed by it: a shoulder's outer side starts where its rising (or falling) leg
            // crosses the neckline. The neckline runs through the two troughs, from the start
            // to the end of the pattern.
            if let Some((left, right)) = kind.neckline() {
                let neck = (anchors[left], anchors[right]);
                let foot = |outer: Pos2, peak: Pos2| {
                    curves::segment_crossing(outer, peak, neck.0, neck.1).unwrap_or(outer)
                };
                regions.push(vec![foot(anchors[0], anchors[1]), anchors[1], anchors[2]]);
                regions.push(vec![anchors[2], anchors[3], anchors[4]]);
                regions.push(vec![anchors[4], anchors[5], foot(anchors[6], anchors[5])]);
            }
            if let Some((left, right)) = kind.neckline() {
                let (from, to) = (anchors[0].x, anchors[kind.points() - 1].x);
                if let (Some(start), Some(end)) = (
                    curves::line_at_x(anchors[left], anchors[right], from),
                    curves::line_at_x(anchors[left], anchors[right], to),
                ) {
                    screen.lines.push(vec![start, end]);
                }
            }
        }
        PatternKind::Abcd => {}
        PatternKind::Triangle => {
            let sides = curves::triangle_sides(
                anchors[0],
                anchors[1],
                anchors[2],
                anchors[3],
                proj.plot.right() + APEX_REACH,
            );
            screen.lines.push(vec![anchors[0], sides.ac_end]);
            screen.lines.push(vec![anchors[1], sides.bd_end]);
            regions.push(vec![anchors[0], sides.ac_end, sides.bd_end, anchors[1]]);
        }
        PatternKind::ThreeDrives => regions.push(anchors.to_vec()),
    }
    screen.bands = regions
        .iter()
        .filter_map(|region| fill(proj, style, region))
        .collect();

    let options = &style.pattern;
    if options.ratios {
        for ratio in pattern::ratios(kind, points) {
            let (from, to) = (anchors[ratio.from], anchors[ratio.to]);
            screen.styled.push(measuring(from, to, style));
            screen.labels.push(PivotLabel {
                at: middle(from, to),
                text: pattern::format_ratio(ratio.value, options.precision),
                look: LabelLook::Ratio,
                place: LabelPlace::On,
                push: curves::normal_up(from, to),
            });
        }
    }
    if options.labels {
        for (index, name) in kind.labels().iter().enumerate() {
            if !name.is_empty() {
                screen.labels.push(pivot_label(
                    anchors[index],
                    (*name).to_owned(),
                    LabelLook::Pill,
                    pattern::is_high(points, index),
                ));
            }
        }
    }
    screen.handles = vertex_handles(anchors);
    Some(screen)
}

/// A pivot's name, above a high and below a low.
fn pivot_label(at: Pos2, text: String, look: LabelLook, high: bool) -> PivotLabel {
    PivotLabel {
        at,
        text,
        look,
        place: LabelPlace::Beside,
        push: vec2(0.0, if high { -1.0 } else { 1.0 }),
    }
}

// MARK: Elliott waves

fn elliott_wave(
    kind: WaveKind,
    points: &[Anchor],
    style: &DrawingStyle,
    anchors: &[Pos2],
) -> Option<Screen> {
    if anchors.len() != kind.points() || points.len() != kind.points() {
        return None;
    }
    let mut screen = Screen::empty();
    screen.lines.push(anchors.to_vec());
    if style.pattern.labels {
        let names = style.pattern.degree.pivot_labels(kind);
        for (index, name) in names.into_iter().enumerate() {
            if !name.is_empty() {
                screen.labels.push(pivot_label(
                    anchors[index],
                    name,
                    LabelLook::Plain,
                    pattern::is_high(points, index),
                ));
            }
        }
    }
    screen.handles = vertex_handles(anchors);
    Some(screen)
}

// MARK: Cycles

fn cycle_shape(
    kind: CycleKind,
    a: Anchor,
    b: Anchor,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: &[Pos2],
) -> Option<Screen> {
    let (pa, pb) = (anchors[0], anchors[1]);
    let plot = proj.plot;
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(anchors);

    // The cycle is counted in bars: the distance between the anchors in the bar index, which the
    // screen's x is linear in, so a period in bars is a constant width in pixels.
    let (first, second) = (
        proj.times.index_of(a.time_ms),
        proj.times.index_of(b.time_ms),
    );
    let period = cycle::period(first, second);
    let (lo, hi) = (
        proj.index_at(plot.left() - 1.0),
        proj.index_at(plot.right() + 1.0),
    );
    let span = (pb.x - pa.x).abs();

    match kind {
        CycleKind::CyclicLines => {
            screen.styled.push(Styled {
                path: vec![pa, pb],
                color: JOINING_LINE,
                width: 1.0,
                dash: LineDash::Dashed,
            });
            if let Some(period) = period.filter(|_| span >= MIN_LINE_SPACING)
                && let Some(found) = cycle::positions(first, period, lo, hi)
            {
                for index in found {
                    let x = proj.x_at_index(index);
                    screen
                        .lines
                        .push(vec![pos2(x, plot.top()), pos2(x, plot.bottom())]);
                }
            }
        }
        CycleKind::TimeCycles => {
            if let Some(period) = period.filter(|_| span >= MIN_ARC_WIDTH) {
                // Over the baseline at the first anchor's price, or under it when the second
                // anchor is below the first.
                let up = pb.y <= pa.y;
                let reach = period.abs();
                if let Some(found) = cycle::positions(first, period, lo - reach, hi + reach) {
                    for start in found {
                        let (left, right) =
                            (proj.x_at_index(start), proj.x_at_index(start + period));
                        let arc = curves::arc_points(
                            (left + right) / 2.0,
                            pa.y,
                            (right - left).abs() / 2.0,
                            up,
                            plot.left() - 1.0,
                            plot.right() + 1.0,
                        );
                        let (Some(begin), Some(end)) = (arc.first().copied(), arc.last().copied())
                        else {
                            continue;
                        };
                        // The disc under the arc, closed along the baseline.
                        let mut region = arc.clone();
                        region.push(pos2(end.x, pa.y));
                        region.push(pos2(begin.x, pa.y));
                        screen.bands.extend(fill(proj, style, &region));
                        screen.lines.push(arc);
                    }
                }
            }
        }
        CycleKind::SineLine => {
            if let Some(wave) = curves::sine_points(pa, pb, plot.left() - 4.0, plot.right() + 4.0) {
                screen.lines.push(wave);
            }
        }
    }
    Some(screen)
}
