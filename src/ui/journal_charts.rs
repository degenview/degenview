//! Small painter-drawn charts for the Trade Journal: a line/area chart with a hover read-out, a
//! signed bar chart, a ring, a split bar and a radar. Plain `egui::Painter`, like the rest of the
//! app's charts. The geometry (ticks, nearest point, radar vertices) is pure and tested.

use eframe::egui::{
    self, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2,
};

use super::kit::size;
use super::theme;
use degenview_journal::overview::nice_step;

const AXIS_LEFT: f32 = 52.0;
const AXIS_BOTTOM: f32 = 22.0;
const PAD: f32 = 6.0;

// MARK: Geometry

/// Up to `count` round tick values covering `[low, high]`, always including the range's inside.
pub fn nice_ticks(low: f64, high: f64, count: usize) -> Vec<f64> {
    if !(low.is_finite() && high.is_finite()) || count == 0 {
        return Vec::new();
    }
    if high <= low {
        return vec![low];
    }
    let step = nice_step((high - low) / count as f64);
    let first = (low / step).ceil();
    let last = (high / step).floor();
    (0..=((last - first) as i64).max(0))
        .map(|index| (first + index as f64) * step)
        .filter(|tick| *tick >= low - step * 1e-9 && *tick <= high + step * 1e-9)
        .collect()
}

/// Index of the value in `xs` (ascending) nearest `x`.
pub fn nearest(xs: &[f64], x: f64) -> Option<usize> {
    if xs.is_empty() {
        return None;
    }
    let after = xs.partition_point(|value| *value < x);
    let candidates = [after.checked_sub(1), (after < xs.len()).then_some(after)];
    candidates.into_iter().flatten().min_by(|&a, &b| {
        (xs[a] - x)
            .abs()
            .partial_cmp(&(xs[b] - x).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Vertices of a radar polygon: axis `i` of `n` points up for `i = 0` and runs clockwise; a value
/// of 100 reaches `radius`.
pub fn radar_points(center: Pos2, radius: f32, values: &[f64]) -> Vec<Pos2> {
    let n = values.len().max(1) as f32;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let angle = -std::f32::consts::FRAC_PI_2 + index as f32 * std::f32::consts::TAU / n;
            let length = radius * (*value as f32 / 100.0).clamp(0.0, 1.0);
            center + vec2(angle.cos(), angle.sin()) * length
        })
        .collect()
}

/// A cell of exactly `size`, its content on the left and centred vertically. `allocate_ui_with_layout`
/// alone advances a row by what the content used, not by the size asked for.
pub fn fixed_cell(ui: &mut Ui, size: egui::Vec2, content: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(size);
            content(ui);
        },
    );
}

// MARK: Line chart

pub struct Line {
    pub name: String,
    pub points: Vec<(f64, f64)>,
    pub color: Color32,
    pub fill: bool,
    pub dashed: bool,
}

pub struct LineChart<'a> {
    pub lines: &'a [Line],
    /// A horizontal reference (starting balance, zero).
    pub baseline: Option<f64>,
    pub x_label: &'a dyn Fn(f64) -> String,
    pub y_label: &'a dyn Fn(f64) -> String,
    pub empty: &'a str,
}

fn bounds(lines: &[Line], baseline: Option<f64>) -> Option<((f64, f64), (f64, f64))> {
    let mut x = (f64::INFINITY, f64::NEG_INFINITY);
    let mut y = (f64::INFINITY, f64::NEG_INFINITY);
    for point in lines.iter().flat_map(|line| line.points.iter()) {
        x = (x.0.min(point.0), x.1.max(point.0));
        y = (y.0.min(point.1), y.1.max(point.1));
    }
    if let Some(base) = baseline {
        y = (y.0.min(base), y.1.max(base));
    }
    (x.0.is_finite() && y.0.is_finite()).then_some((x, y))
}

pub fn line_chart(ui: &mut Ui, id: egui::Id, chart: &LineChart<'_>, height: f32) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let Some(((x0, x1), (y0, y1))) = bounds(chart.lines, chart.baseline) else {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            chart.empty,
            FontId::proportional(size::BODY),
            theme::text_tertiary(),
        );
        return;
    };
    let pad_y = ((y1 - y0) * 0.08).max(1e-9);
    let (y0, y1) = (y0 - pad_y, y1 + pad_y);
    let (x0, x1) = if x1 > x0 {
        (x0, x1)
    } else {
        (x0 - 1.0, x0 + 1.0)
    };
    let plot = Rect::from_min_max(
        pos2(rect.left() + AXIS_LEFT, rect.top() + PAD),
        pos2(rect.right() - PAD, rect.bottom() - AXIS_BOTTOM),
    );
    let to_x = |x: f64| plot.left() + ((x - x0) / (x1 - x0)) as f32 * plot.width();
    let to_y = |y: f64| plot.bottom() - ((y - y0) / (y1 - y0)) as f32 * plot.height();

    let grid = Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.7));
    for tick in nice_ticks(y0, y1, 4) {
        let y = to_y(tick);
        painter.hline(plot.x_range(), y, grid);
        painter.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            (chart.y_label)(tick),
            FontId::proportional(size::CAPTION),
            theme::text_tertiary(),
        );
    }
    for (index, tick) in nice_ticks(x0, x1, 4).into_iter().enumerate() {
        let x = to_x(tick);
        let align = if index == 0 {
            Align2::LEFT_TOP
        } else {
            Align2::CENTER_TOP
        };
        painter.text(
            pos2(x, plot.bottom() + 6.0),
            align,
            (chart.x_label)(tick),
            FontId::proportional(size::CAPTION),
            theme::text_tertiary(),
        );
    }
    if let Some(base) = chart.baseline {
        let y = to_y(base);
        painter.extend(Shape::dashed_line(
            &[pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, theme::with_alpha(theme::text_secondary(), 0.5)),
            3.0,
            3.0,
        ));
    }

    for line in chart.lines {
        let points: Vec<Pos2> = line
            .points
            .iter()
            .map(|(x, y)| pos2(to_x(*x), to_y(*y)))
            .collect();
        if points.is_empty() {
            continue;
        }
        if line.fill && points.len() > 1 {
            let floor = chart
                .baseline
                .map_or(plot.bottom(), to_y)
                .clamp(plot.top(), plot.bottom());
            let mut mesh = Mesh::default();
            let top = theme::with_alpha(line.color, 0.22);
            let bottom = theme::with_alpha(line.color, 0.0);
            for point in &points {
                mesh.colored_vertex(*point, top);
                mesh.colored_vertex(pos2(point.x, floor), bottom);
            }
            for index in 0..points.len() as u32 - 1 {
                let base = index * 2;
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base + 1, base + 3, base + 2);
            }
            painter.add(Shape::mesh(mesh));
        }
        let stroke = Stroke::new(1.6, line.color);
        if line.dashed {
            painter.extend(Shape::dashed_line(&points, stroke, 4.0, 3.0));
        } else if points.len() == 1 {
            painter.circle_filled(points[0], 3.0, line.color);
        } else {
            painter.add(Shape::line(points, stroke));
        }
    }

    // Hover read-out on the first line's nearest point.
    let Some(hover) = response.hover_pos().filter(|pos| plot.contains(*pos)) else {
        return;
    };
    let main = &chart.lines[0];
    let xs: Vec<f64> = main.points.iter().map(|p| p.0).collect();
    let at = x0 + f64::from((hover.x - plot.left()) / plot.width()) * (x1 - x0);
    let Some(index) = nearest(&xs, at) else {
        return;
    };
    let (px, py) = main.points[index];
    let marker = pos2(to_x(px), to_y(py));
    painter.vline(
        marker.x,
        plot.y_range(),
        Stroke::new(1.0, theme::with_alpha(theme::text_secondary(), 0.5)),
    );
    painter.circle_filled(marker, 3.5, main.color);
    painter.circle_stroke(marker, 3.5, Stroke::new(1.5, theme::background()));

    let mut rows = vec![(chart.x_label)(px).to_owned()];
    rows.push(format!("{}  {}", main.name, (chart.y_label)(py)));
    for other in &chart.lines[1..] {
        let oxs: Vec<f64> = other.points.iter().map(|p| p.0).collect();
        if let Some(i) = nearest(&oxs, px) {
            rows.push(format!(
                "{}  {}",
                other.name,
                (chart.y_label)(other.points[i].1)
            ));
        }
    }
    tooltip(ui, id, &painter, rect, marker, &rows);
}

fn tooltip(
    ui: &Ui,
    _id: egui::Id,
    painter: &egui::Painter,
    bounds: Rect,
    anchor: Pos2,
    rows: &[String],
) {
    let font = FontId::proportional(size::CAPTION + 1.0);
    let galleys: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            ui.painter().layout_no_wrap(
                row.clone(),
                font.clone(),
                if index == 0 {
                    theme::text_secondary()
                } else {
                    theme::text_primary()
                },
            )
        })
        .collect();
    let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 16.0;
    let height =
        galleys.iter().map(|g| g.size().y).sum::<f32>() + 12.0 + 2.0 * galleys.len() as f32;
    let mut left = anchor.x + 12.0;
    if left + width > bounds.right() {
        left = anchor.x - 12.0 - width;
    }
    let top = (anchor.y - height - 10.0).max(bounds.top());
    let card = Rect::from_min_size(pos2(left, top), vec2(width, height));
    painter.rect_filled(card, 8.0, theme::chrome());
    painter.rect_stroke(
        card,
        8.0,
        Stroke::new(1.0, theme::separator()),
        egui::StrokeKind::Inside,
    );
    let mut y = card.top() + 6.0;
    for galley in galleys {
        let h = galley.size().y + 2.0;
        painter.galley(pos2(card.left() + 8.0, y), galley, Color32::PLACEHOLDER);
        y += h;
    }
}

// MARK: Bar chart

pub struct Bar {
    pub label: String,
    pub value: f64,
    pub color: Color32,
    pub detail: String,
}

/// Signed bars about zero. `labels` are the x labels shown under some of the bars.
pub fn bar_chart(
    ui: &mut Ui,
    bars: &[Bar],
    y_label: &dyn Fn(f64) -> String,
    height: f32,
    empty: &str,
) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    if bars.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            empty,
            FontId::proportional(size::BODY),
            theme::text_tertiary(),
        );
        return;
    }
    let low = bars.iter().map(|b| b.value).fold(0.0, f64::min);
    let high = bars.iter().map(|b| b.value).fold(0.0, f64::max);
    let pad = ((high - low) * 0.08).max(1e-9);
    let (y0, y1) = (
        if low < 0.0 { low - pad } else { 0.0 },
        if high > 0.0 { high + pad } else { 0.0 },
    );
    let (y0, y1) = if y1 > y0 { (y0, y1) } else { (-1.0, 1.0) };
    let plot = Rect::from_min_max(
        pos2(rect.left() + AXIS_LEFT, rect.top() + PAD),
        pos2(rect.right() - PAD, rect.bottom() - AXIS_BOTTOM),
    );
    let to_y = |y: f64| plot.bottom() - ((y - y0) / (y1 - y0)) as f32 * plot.height();
    let grid = Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.7));
    for tick in nice_ticks(y0, y1, 4) {
        let y = to_y(tick);
        painter.hline(plot.x_range(), y, grid);
        painter.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            y_label(tick),
            FontId::proportional(size::CAPTION),
            theme::text_tertiary(),
        );
    }
    let zero = to_y(0.0);
    painter.hline(
        plot.x_range(),
        zero,
        Stroke::new(1.0, theme::with_alpha(theme::text_secondary(), 0.6)),
    );
    let slot = plot.width() / bars.len() as f32;
    let bar_width = (slot * 0.7).clamp(2.0, 28.0);
    let hovered = response
        .hover_pos()
        .filter(|pos| plot.contains(*pos))
        .map(|pos| (((pos.x - plot.left()) / slot) as usize).min(bars.len() - 1));
    let label_every = ((bars.len() as f32 * 56.0 / plot.width()).ceil() as usize).max(1);
    for (index, bar) in bars.iter().enumerate() {
        let center = plot.left() + slot * (index as f32 + 0.5);
        let top = to_y(bar.value.max(0.0));
        let bottom = to_y(bar.value.min(0.0));
        let body = Rect::from_min_max(
            pos2(center - bar_width / 2.0, top),
            pos2(center + bar_width / 2.0, bottom.max(top + 1.0)),
        );
        let color = if hovered == Some(index) {
            bar.color
        } else {
            theme::with_alpha(bar.color, 0.85)
        };
        painter.rect_filled(body, 2.0, color);
        if index % label_every == 0 {
            painter.text(
                pos2(center, plot.bottom() + 6.0),
                Align2::CENTER_TOP,
                &bar.label,
                FontId::proportional(size::CAPTION),
                theme::text_tertiary(),
            );
        }
    }
    if let Some(index) = hovered {
        let bar = &bars[index];
        let anchor = pos2(plot.left() + slot * (index as f32 + 0.5), to_y(bar.value));
        tooltip(
            ui,
            egui::Id::new("bar-tip"),
            &painter,
            rect,
            anchor,
            &[bar.label.clone(), bar.detail.clone()],
        );
    }
}

// MARK: Ring, split bar, radar

/// A donut showing `fraction` (0 to 1) in `color`; nothing when there is no figure.
pub fn ring(ui: &mut Ui, fraction: Option<f64>, color: Color32, diameter: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(diameter, diameter), Sense::hover());
    let painter = ui.painter_at(rect);
    let center = rect.center();
    let radius = diameter / 2.0 - 3.0;
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(4.0, theme::with_alpha(theme::text_tertiary(), 0.25)),
    );
    if let Some(fraction) = fraction.filter(|f| f.is_finite()) {
        let sweep = (fraction.clamp(0.0, 1.0) as f32) * std::f32::consts::TAU;
        if sweep > 0.001 {
            let steps = ((sweep / std::f32::consts::TAU) * 64.0).ceil().max(2.0) as usize;
            let points: Vec<Pos2> = (0..=steps)
                .map(|step| {
                    let angle = -std::f32::consts::FRAC_PI_2 + sweep * step as f32 / steps as f32;
                    center + vec2(angle.cos(), angle.sin()) * radius
                })
                .collect();
            painter.add(Shape::line(points, Stroke::new(4.0, color)));
        }
    }
}

/// Two shares of a whole in the win and loss colours; an empty track when both are zero.
pub fn split_bar(ui: &mut Ui, win: f64, loss: f64, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, theme::with_alpha(theme::text_tertiary(), 0.25));
    let total = win + loss;
    if total > 0.0 && total.is_finite() {
        let split = rect.left() + rect.width() * (win / total) as f32;
        painter.rect_filled(
            Rect::from_min_max(rect.min, pos2((split - 1.0).max(rect.left()), rect.max.y)),
            3.0,
            theme::WIN,
        );
        painter.rect_filled(
            Rect::from_min_max(pos2((split + 1.0).min(rect.right()), rect.min.y), rect.max),
            3.0,
            theme::LOSS,
        );
    }
}

/// A radar over `axes` (label, 0 to 100). The labels sit outside the polygon.
pub fn radar(ui: &mut Ui, axes: &[(&str, f64)], height: f32, accent: Color32) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let center = rect.center();
    let radius = (rect.height().min(rect.width() * 0.6) / 2.0 - 22.0).max(20.0);
    let values: Vec<f64> = axes.iter().map(|(_, v)| *v).collect();
    for ring in [25.0, 50.0, 75.0, 100.0] {
        let points = radar_points(center, radius, &vec![ring; axes.len()]);
        painter.add(Shape::closed_line(
            points,
            Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.8)),
        ));
    }
    for (index, vertex) in radar_points(center, radius, &vec![100.0; axes.len()])
        .into_iter()
        .enumerate()
    {
        painter.line_segment(
            [center, vertex],
            Stroke::new(1.0, theme::with_alpha(theme::separator(), 0.6)),
        );
        let label = radar_points(center, radius + 14.0, &vec![100.0; axes.len()])[index];
        let align = if label.x < center.x - 4.0 {
            Align2::RIGHT_CENTER
        } else if label.x > center.x + 4.0 {
            Align2::LEFT_CENTER
        } else {
            Align2::CENTER_CENTER
        };
        painter.text(
            label,
            align,
            axes[index].0,
            FontId::proportional(size::CAPTION),
            theme::text_tertiary(),
        );
    }
    let polygon = radar_points(center, radius, &values);
    if polygon.len() >= 3 {
        painter.add(Shape::convex_polygon(
            polygon.clone(),
            theme::with_alpha(accent, 0.18),
            Stroke::new(1.5, accent),
        ));
    }
    if let Some(pos) = response.hover_pos() {
        for (index, vertex) in polygon.iter().enumerate() {
            if vertex.distance(pos) < 10.0 {
                tooltip(
                    ui,
                    egui::Id::new("radar-tip"),
                    &painter,
                    rect,
                    *vertex,
                    &[
                        axes[index].0.to_owned(),
                        format!("{:.0} / 100", axes[index].1),
                    ],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_round_and_inside_the_range() {
        let ticks = nice_ticks(-130.0, 470.0, 4);
        assert_eq!(ticks, [0.0, 200.0, 400.0]);
        assert_eq!(
            nice_ticks(0.0, 1.0, 4),
            [0.0, 0.25, 0.5, 0.75, 1.0].map(|t| (t * 1e9f64).round() / 1e9)
        );
        assert_eq!(nice_ticks(5.0, 5.0, 4), [5.0]);
        assert!(nice_ticks(f64::NAN, 1.0, 4).is_empty());
        assert!(nice_ticks(0.0, 10.0, 0).is_empty());
    }

    #[test]
    fn the_nearest_point_is_found_from_either_side() {
        let xs = [1.0, 2.0, 10.0];
        assert_eq!(nearest(&xs, 0.0), Some(0));
        assert_eq!(nearest(&xs, 1.4), Some(0));
        assert_eq!(nearest(&xs, 1.6), Some(1));
        assert_eq!(nearest(&xs, 7.0), Some(2));
        assert_eq!(nearest(&xs, 99.0), Some(2));
        assert_eq!(nearest(&[], 1.0), None);
    }

    #[test]
    fn a_radar_axis_runs_clockwise_from_the_top() {
        let points = radar_points(pos2(100.0, 100.0), 50.0, &[100.0, 100.0, 100.0, 100.0]);
        assert!((points[0] - pos2(100.0, 50.0)).length() < 1e-3);
        assert!((points[1] - pos2(150.0, 100.0)).length() < 1e-3);
        assert!((points[2] - pos2(100.0, 150.0)).length() < 1e-3);
        let half = radar_points(pos2(0.0, 0.0), 40.0, &[50.0]);
        assert!((half[0] - pos2(0.0, -20.0)).length() < 1e-3);
        // Out-of-range values are clamped.
        let clamped = radar_points(pos2(0.0, 0.0), 10.0, &[500.0, -5.0]);
        assert!((clamped[0].to_vec2().length() - 10.0).abs() < 1e-3);
        assert!(clamped[1].to_vec2().length() < 1e-3);
    }

    #[test]
    fn every_chart_draws_with_data_and_without() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                let fmt = |v: f64| format!("{v:.0}");
                let line = Line {
                    name: "Equity".into(),
                    points: vec![(0.0, 1.0), (1.0, 3.0), (2.0, 2.0)],
                    color: theme::WIN,
                    fill: true,
                    dashed: false,
                };
                line_chart(
                    ui,
                    egui::Id::new("l"),
                    &LineChart {
                        lines: &[line],
                        baseline: Some(0.0),
                        x_label: &fmt,
                        y_label: &fmt,
                        empty: "none",
                    },
                    140.0,
                );
                line_chart(
                    ui,
                    egui::Id::new("e"),
                    &LineChart {
                        lines: &[],
                        baseline: None,
                        x_label: &fmt,
                        y_label: &fmt,
                        empty: "none",
                    },
                    140.0,
                );
                let one = Line {
                    name: "x".into(),
                    points: vec![(5.0, 5.0)],
                    color: theme::LOSS,
                    fill: true,
                    dashed: true,
                };
                line_chart(
                    ui,
                    egui::Id::new("o"),
                    &LineChart {
                        lines: &[one],
                        baseline: None,
                        x_label: &fmt,
                        y_label: &fmt,
                        empty: "none",
                    },
                    140.0,
                );
                let bars = vec![
                    Bar {
                        label: "a".into(),
                        value: 5.0,
                        color: theme::WIN,
                        detail: "x".into(),
                    },
                    Bar {
                        label: "b".into(),
                        value: -3.0,
                        color: theme::LOSS,
                        detail: "y".into(),
                    },
                ];
                bar_chart(ui, &bars, &fmt, 120.0, "none");
                bar_chart(ui, &[], &fmt, 120.0, "none");
                ring(ui, Some(0.4), theme::WIN, 40.0);
                ring(ui, None, theme::WIN, 40.0);
                split_bar(ui, 3.0, 1.0, 100.0);
                split_bar(ui, 0.0, 0.0, 100.0);
                radar(
                    ui,
                    &[("a", 10.0), ("b", 50.0), ("c", 90.0)],
                    200.0,
                    theme::WIN,
                );
            },
        );
        out.textures_delta.clear();
    }
}
