//! The anchored VWAP and the volume profiles projected onto the plot.
//!
//! Both are worked from the bars the chart draws (constructed bars on a Renko chart, and only what
//! a replay has shown), by `model::volume_profile`. A shape is built once per drawing and
//! projection and reused, so the arithmetic is not redone while the pointer moves; a bar whose
//! volume changes rebuilds it (the projection key reads the newest bar's volume). A market with no
//! volume has none to show, and the shape says so instead of drawing a line it made up.

use super::Projection;
use super::shape::{
    Band, ColorBox, LabelLook, LabelPlace, PivotLabel, Screen, Styled, vertex_handles,
};
use crate::model::chart_type::{LineDash, Rgba};
use crate::model::drawing_style::DrawingStyle;
use crate::model::forecast_style::{ProfileOptions, ProfileVolume, RowsLayout};
use crate::model::volume_profile::{self, RowSize};
use crate::model::{Anchor, Geometry};
use crate::ui::line_geometry as lines;
use eframe::egui::{Pos2, Rect, pos2, vec2};

/// How far (px) beyond the shape its labels may reach, for its bounds.
const REACH: f32 = 60.0;
/// A row shorter than this (px) is still drawn this tall, so a thin row is not invisible.
const MIN_ROW_HEIGHT: f32 = 0.5;
/// The share of its colour's opacity a row outside the value area keeps.
const OUTSIDE_VALUE_AREA: f32 = 0.55;
/// The opacity of the shade between a VWAP's bands.
const BAND_SHADE: f32 = 0.12;

/// Builds the screen shape of an anchored VWAP or a volume profile. `anchors` are the geometry's
/// anchors on screen, already checked to be finite.
pub(super) fn project(
    geometry: &Geometry,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: Vec<Pos2>,
) -> Option<Screen> {
    let screen = match geometry {
        Geometry::Vwap { at } => vwap(*at, style, proj, &anchors),
        Geometry::FixedProfile { a, b } => profile(Range::Fixed(*a, *b), style, proj, &anchors),
        Geometry::AnchoredProfile { at } => profile(Range::Anchored(*at), style, proj, &anchors),
        _ => return None,
    }?;
    screen.finish(style.stroke.width / 2.0 + REACH)
}

/// The bar a time falls on, as an index into the bars the chart draws (never before the first).
fn bar_at(proj: &Projection, time_ms: i64) -> usize {
    proj.times.index_of(time_ms).round().max(0.0) as usize
}

fn faded(color: Rgba, factor: f32) -> Rgba {
    [
        color[0],
        color[1],
        color[2],
        (f32::from(color[3]) * factor).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Whether a time is before the first bar the chart has loaded: older bars page in as the chart is
/// scrolled back, so what is drawn then covers only the bars there are.
fn before_the_loaded_bars(proj: &Projection, time_ms: i64) -> bool {
    proj.times.index_of(time_ms) < -0.5
}

/// A grey note on a plate, for what cannot be shown.
fn note(at: Pos2, text: &str) -> PivotLabel {
    PivotLabel {
        at,
        text: text.to_owned(),
        look: LabelLook::Plate {
            fill: [88, 88, 88, 255],
            ink: [255, 255, 255, 255],
        },
        place: LabelPlace::Beside,
        push: vec2(0.0, -1.0),
        edge: vec2(0.0, 0.0),
        on_select: false,
    }
}

// MARK: Anchored VWAP

fn vwap(
    anchor: Anchor,
    style: &DrawingStyle,
    proj: &Projection,
    points: &[Pos2],
) -> Option<Screen> {
    let options = &style.vwap;
    let bars = proj.times.bars();
    let first = bar_at(proj, anchor.time_ms);
    let series = volume_profile::anchored_vwap(bars, first, options.source);
    let mut screen = Screen::empty();

    // Only the stretch the plot can see (with a bar of margin) is made into a path.
    let (left, right) = (
        proj.plot.left() - proj.slot_width,
        proj.plot.right() + proj.slot_width,
    );
    let visible: Vec<&volume_profile::VwapPoint> = series
        .iter()
        .filter(|point| {
            let x = proj.x_at_index(point.index as f64);
            x >= left && x <= right
        })
        .collect();
    let path = |offset: f64| -> Vec<Pos2> {
        visible
            .iter()
            .filter_map(|point| {
                let price = point.vwap + offset * point.deviation;
                // A band under zero has no place on a logarithmic scale.
                (!proj.scale.logarithmic || price > 0.0)
                    .then(|| pos2(proj.x_at_index(point.index as f64), proj.scale.y(price)))
            })
            .collect()
    };
    let main = path(0.0);
    if main.len() >= 2 {
        screen.lines.push(main);
    }
    for band in options.bands.iter().filter(|band| band.enabled) {
        let multiplier = f64::from(band.multiplier);
        let (upper, lower) = (path(multiplier), path(-multiplier));
        for edge in [&upper, &lower] {
            if edge.len() >= 2 {
                screen.styled.push(Styled {
                    path: edge.clone(),
                    color: band.color,
                    width: 1.0,
                    dash: LineDash::Solid,
                });
            }
        }
        if options.fill && upper.len() >= 2 && upper.len() == lower.len() {
            let mut polygon = upper;
            polygon.extend(lower.into_iter().rev());
            let polygon = lines::clip_polygon(&polygon, proj.plot);
            if polygon.len() >= 3 {
                screen.shades.push(Band {
                    polygon,
                    color: faded(band.color, BAND_SHADE),
                });
            }
        }
    }

    // The handle sits on the line at the anchor's bar, or where the anchor was put when there is no
    // line to sit on.
    let at = series.first().map_or(points[0], |start| {
        pos2(
            proj.x_at_index(start.index as f64),
            proj.scale.y(start.vwap),
        )
    });
    screen.handles = vertex_handles(&[at]);
    if series.is_empty() && first < bars.len() {
        screen.labels.push(note(at, "No volume data"));
    } else if before_the_loaded_bars(proj, anchor.time_ms) {
        screen.labels.push(note(at, "From the loaded bars only"));
    }
    Some(screen)
}

// MARK: Volume profile

/// Which bars a profile covers.
enum Range {
    /// Between two times.
    Fixed(Anchor, Anchor),
    /// From a time to the latest bar.
    Anchored(Anchor),
}

fn profile(
    range: Range,
    style: &DrawingStyle,
    proj: &Projection,
    points: &[Pos2],
) -> Option<Screen> {
    let options = &style.profile;
    let bars = proj.times.bars();
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(points);
    if bars.is_empty() {
        return Some(screen);
    }
    let (first, last) = match range {
        Range::Fixed(a, b) => {
            let (a, b) = (bar_at(proj, a.time_ms), bar_at(proj, b.time_ms));
            (a.min(b), a.max(b))
        }
        Range::Anchored(at) => (bar_at(proj, at.time_ms), bars.len() - 1),
    };
    if first >= bars.len() {
        return Some(screen);
    }
    let last = last.min(bars.len() - 1);

    // The rows: a number of them, or rows of so many of the chart's price steps.
    let size = match options.layout {
        RowsLayout::Rows => RowSize::Count(options.rows),
        RowsLayout::Ticks => {
            RowSize::Height(f64::from(options.ticks) * proj.format.tick_size(bars[first].close))
        }
    };
    let left = proj.x_at_index(first as f64) - proj.slot_width / 2.0;
    let right = proj.x_at_index(last as f64) + proj.slot_width / 2.0;
    let Some(built) = volume_profile::profile(
        bars,
        first,
        last,
        size,
        f64::from(options.value_area) / 100.0,
    ) else {
        screen.labels.push(note(
            pos2(left.max(proj.plot.left()), proj.plot.top() + 20.0),
            "No volume data",
        ));
        return Some(screen);
    };

    histogram(&mut screen, &built, options, proj, left, right);
    let starts = match range {
        Range::Fixed(a, b) => a.time_ms.min(b.time_ms),
        Range::Anchored(at) => at.time_ms,
    };
    if before_the_loaded_bars(proj, starts) {
        screen.labels.push(note(
            pos2(left.max(proj.plot.left()), proj.plot.top() + 20.0),
            "From the loaded bars only",
        ));
    }
    // The point of control and the edges of the value area, across the range.
    let line = |price: f64, to: f32, color: Rgba, width: f32, dash: LineDash| Styled {
        path: vec![
            pos2(left, proj.scale.y(price)),
            pos2(to, proj.scale.y(price)),
        ],
        color,
        width,
        dash,
    };
    if options.poc {
        let to = if options.extend_poc {
            proj.plot.right().max(right)
        } else {
            right
        };
        screen.styled.push(line(
            built.poc_price(),
            to,
            options.poc_color,
            1.5,
            LineDash::Solid,
        ));
    }
    if options.value_area_lines {
        for price in [built.value_area_high(), built.value_area_low()] {
            screen.styled.push(line(
                price,
                right,
                options.value_area_color,
                1.0,
                LineDash::Dashed,
            ));
        }
    }
    // The range, drawn while the profile is selected.
    let (top, bottom) = (
        proj.scale.y(built.rows[built.rows.len() - 1].high),
        proj.scale.y(built.rows[0].low),
    );
    let (top, bottom) = (top.min(bottom), top.max(bottom));
    screen.guides.extend([
        [pos2(left, top), pos2(right, top)],
        [pos2(left, bottom), pos2(right, bottom)],
        [pos2(left, top), pos2(left, bottom)],
        [pos2(right, top), pos2(right, bottom)],
    ]);
    Some(screen)
}

/// The bars of the histogram, from the range's left edge (or its right, leftwards) as wide as the
/// longest bar is allowed, each in its mode's colours, those outside the value area faded.
fn histogram(
    screen: &mut Screen,
    built: &volume_profile::Profile,
    options: &ProfileOptions,
    proj: &Projection,
    left: f32,
    right: f32,
) {
    let longest = (right - left).abs() * options.width / 100.0;
    let (origin, direction) = if options.right {
        (right, -1.0)
    } else {
        (left, 1.0)
    };
    let scale = match options.volume {
        ProfileVolume::Delta => built
            .rows
            .iter()
            .map(|row| (row.up - row.down).abs())
            .fold(0.0, f64::max),
        _ => built.peak(),
    };
    if !(scale.is_finite() && scale > 0.0) {
        return;
    }
    let length = |volume: f64| (volume / scale) as f32 * longest;
    for (index, row) in built.rows.iter().enumerate() {
        let (y0, y1) = (proj.scale.y(row.high), proj.scale.y(row.low));
        let (top, bottom) = (y0.min(y1), y0.max(y1));
        if bottom < proj.plot.top() - 1.0 || top > proj.plot.bottom() + 1.0 {
            continue;
        }
        let bottom = bottom.max(top + MIN_ROW_HEIGHT);
        let inside = index >= built.value_area.0 && index <= built.value_area.1;
        let tint = |color: Rgba| faded(color, if inside { 1.0 } else { OUTSIDE_VALUE_AREA });
        let mut bar = |from: f32, to: f32, color: Rgba| {
            if (to - from).abs() > 0.0 {
                screen.boxes.push(ColorBox {
                    rect: Rect::from_two_pos(
                        pos2(origin + direction * from, top),
                        pos2(origin + direction * to, bottom),
                    ),
                    color: tint(color),
                });
            }
        };
        match options.volume {
            ProfileVolume::Total => bar(0.0, length(row.total()), options.up),
            ProfileVolume::UpDown => {
                let up = length(row.up);
                bar(0.0, up, options.up);
                bar(up, up + length(row.down), options.down);
            }
            ProfileVolume::Delta => {
                let delta = row.up - row.down;
                bar(
                    0.0,
                    length(delta.abs()),
                    if delta >= 0.0 {
                        options.up
                    } else {
                        options.down
                    },
                );
            }
        }
    }
}
