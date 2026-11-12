//! The CoinMarketCap "Latest" charts: the Fear & Greed semicircle gauge and the Altcoin Season
//! scale, drawn from the cached reading instead of a price series.

use crate::service::market::cmc::{self, LatestReading};
use crate::ui::kit::{self, BOLD, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2, pos2,
    text::LayoutJob, vec2,
};
use std::f32::consts::PI;

const RED: Color32 = kit::RED;
const ORANGE: Color32 = kit::ORANGE;
const YELLOW: Color32 = Color32::from_rgb(255, 214, 10);
const GREEN: Color32 = kit::GREEN;
const MINT: Color32 = Color32::from_rgb(99, 230, 226);
const GRAY: Color32 = Color32::from_rgb(142, 142, 147);
const BLUE: Color32 = theme::SYSTEM_ACCENT;

/// Paints the gauge for `symbol` into `rect`. Returns its accessibility description.
pub fn paint(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    symbol: &str,
    reading: &LatestReading,
) -> String {
    if symbol == cmc::ALTCOIN_LATEST {
        altcoin(ui, painter, rect, reading);
        format!(
            "CMC Altcoin Season Index, {} out of 100, {}.",
            reading.value as i64, reading.classification
        )
    } else {
        fear_greed(ui, painter, rect, reading);
        format!(
            "CMC Crypto Fear and Greed Index, {} out of 100, {}.",
            reading.value as i64, reading.classification
        )
    }
}

/// "56" in a large bold face with a small secondary " / 100" sharing its baseline.
fn score(
    ui: &Ui,
    value: f64,
    size: f32,
    suffix_size: f32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    let format = |size: f32, weight: f32, color: Color32| egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color,
        coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
        valign: egui::Align::BOTTOM,
        ..Default::default()
    };
    job.append(
        &(value as i64).to_string(),
        0.0,
        format(size, BOLD, kit::primary(1.0)),
    );
    job.append(
        " / 100",
        0.0,
        format(suffix_size, REGULAR, theme::text_secondary()),
    );
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = 1;
    ui.painter().layout_job(job)
}

/// Fear & Greed card geometry. Below `STATS_MIN_HEIGHT` only the score and gauge are drawn;
/// the sparkline joins once it would get at least `SPARK_MIN_HEIGHT`.
const STATS_MIN_HEIGHT: f32 = 190.0;
const STATS_HEIGHT: f32 = 46.0;
const HERO_MIN_HEIGHT: f32 = 128.0;
const SPARK_MIN_HEIGHT: f32 = 44.0;
const SECTION_GAP: f32 = 10.0;

/// The five CMC sentiment zones as (upper bound, color, name).
const ZONES: [(f64, Color32, &str); 5] = [
    (25.0, RED, "Extreme Fear"),
    (45.0, ORANGE, "Fear"),
    (55.0, YELLOW, "Neutral"),
    (75.0, GREEN, "Greed"),
    (100.0, MINT, "Extreme Greed"),
];

fn zone_index(value: f64) -> usize {
    ZONES
        .iter()
        .position(|(bound, _, _)| value < *bound)
        .unwrap_or(ZONES.len() - 1)
}

fn zone_color(value: f64) -> Color32 {
    ZONES[zone_index(value)].1
}

fn fade(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn fear_greed(ui: &Ui, painter: &Painter, rect: Rect, reading: &LatestReading) {
    let inner = rect.shrink2(vec2(16.0, 8.0));
    let stats_height = STATS_HEIGHT;
    let remaining = inner.height() - HERO_MIN_HEIGHT;
    let show_stats = rect.height() >= STATS_MIN_HEIGHT
        && remaining >= stats_height
        && !reading.history.is_empty();
    if !show_stats {
        hero(ui, painter, rect, reading);
        return;
    }
    let spark_height = remaining - stats_height - SECTION_GAP;
    let hero_height = inner.height()
        - stats_height
        - SECTION_GAP
        - if spark_height >= SPARK_MIN_HEIGHT {
            spark_height + 8.0
        } else {
            0.0
        };
    hero(
        ui,
        painter,
        Rect::from_min_size(rect.min, vec2(rect.width(), hero_height + 8.0)),
        reading,
    );
    let mut y = inner.min.y + hero_height;
    painter.hline(inner.x_range(), y, Stroke::new(1.0, theme::separator()));
    y += 6.0;
    if spark_height >= SPARK_MIN_HEIGHT {
        sparkline(
            ui,
            painter,
            Rect::from_min_size(pos2(inner.min.x, y), vec2(inner.width(), spark_height)),
            &reading.history,
        );
        y += spark_height + 8.0;
    }
    stats(
        ui,
        painter,
        Rect::from_min_size(pos2(inner.min.x, y), vec2(inner.width(), stats_height)),
        reading,
    );
}

/// The big score and its label beside the zone gauge.
fn hero(ui: &Ui, painter: &Painter, rect: Rect, reading: &LatestReading) {
    let compact = rect.height() < 170.0;
    let inner = rect.shrink2(vec2(if compact { 12.0 } else { 16.0 }, 8.0));
    let value_width = if compact { 92.0 } else { 112.0 };
    let gap = if compact { 8.0 } else { 14.0 };
    let accent = zone_color(reading.value);

    let number = score(
        ui,
        reading.value,
        if compact { 35.0 } else { 44.0 },
        if compact { 11.0 } else { 13.0 },
        value_width + 40.0,
    );
    let label = kit::galley(
        ui,
        &reading.classification,
        if compact { 14.0 } else { 17.0 },
        SEMIBOLD,
        accent,
        value_width + 40.0,
    );
    let change = reading
        .days_ago(1)
        .map(|(_, previous)| reading.value - previous);
    let delta = change.map(|change| {
        let color = if change.abs() < 0.5 {
            theme::text_secondary()
        } else if change > 0.0 {
            GREEN
        } else {
            RED
        };
        let arrow = if change.abs() < 0.5 {
            "•"
        } else if change > 0.0 {
            "▲"
        } else {
            "▼"
        };
        kit::galley(
            ui,
            &format!("{arrow} {:.0} vs yesterday", change.abs()),
            if compact { 10.0 } else { 11.0 },
            MEDIUM,
            color,
            value_width + 60.0,
        )
    });
    let block = number.size().y
        + 2.0
        + label.size().y
        + delta.as_ref().map_or(0.0, |delta| 3.0 + delta.size().y);
    let mut y = inner.center().y - block / 2.0;
    let x = inner.min.x;
    let heights = (number.size().y, label.size().y);
    painter.galley(pos2(x, y), number, Color32::PLACEHOLDER);
    y += heights.0 + 2.0;
    painter.galley(pos2(x, y), label, Color32::PLACEHOLDER);
    if let Some(delta) = delta {
        painter.galley(pos2(x, y + heights.1 + 3.0), delta, Color32::PLACEHOLDER);
    }

    let gauge_width = ((inner.width() - value_width - gap) * 0.88).max(80.0);
    let gauge = Rect::from_center_size(
        pos2(
            inner.min.x + value_width + gap + (inner.width() - value_width - gap) / 2.0,
            inner.center().y,
        ),
        vec2(gauge_width, inner.height() * 0.9),
    );
    sentiment_gauge(ui, painter, gauge, reading.value);
}

/// Five zone arcs with the current one lit, tick labels at the zone edges, and a needle at
/// `value`.
fn sentiment_gauge(ui: &Ui, painter: &Painter, rect: Rect, value: f64) {
    let center = pos2(rect.center().x, rect.min.y + rect.height() * 0.88);
    let radius = (rect.width() / 2.0 - 18.0)
        .min(rect.height() * 0.78)
        .max(10.0);
    let thickness = (radius * 0.13).max(8.0);
    let active = zone_index(value);
    let mut lower = 0.0;
    for (index, (upper, color, _)) in ZONES.iter().enumerate() {
        let gap = 0.014;
        let start = PI + PI * (lower / 100.0) as f32 + gap;
        let end = PI + PI * (*upper / 100.0) as f32 - gap;
        lower = *upper;
        let color = if index == active {
            *color
        } else {
            fade(*color, 70)
        };
        let points: Vec<Pos2> = (0..=24)
            .map(|step| center + Vec2::angled(start + (end - start) * step as f32 / 24.0) * radius)
            .collect();
        painter.add(Shape::line(points, Stroke::new(thickness, color)));
    }
    // Zone-edge ticks just outside the arc.
    for edge in [25.0_f32, 50.0, 75.0] {
        let angle = PI * (1.0 + edge / 100.0);
        let from = center + Vec2::angled(angle) * (radius + thickness / 2.0 + 2.0);
        let to = center + Vec2::angled(angle) * (radius + thickness / 2.0 + 6.0);
        painter.line_segment([from, to], Stroke::new(1.0, theme::text_tertiary()));
        painter.text(
            center + Vec2::angled(angle) * (radius + thickness / 2.0 + 14.0),
            Align2::CENTER_CENTER,
            format!("{edge:.0}"),
            egui::FontId::proportional(9.0),
            theme::text_tertiary(),
        );
    }

    let accent = zone_color(value);
    let angle = PI * (1.0 + value.clamp(0.0, 100.0) as f32 / 100.0);
    let direction = Vec2::angled(angle);
    let normal = vec2(-direction.y, direction.x);
    let tip = center + direction * radius * 0.78;
    painter.add(Shape::convex_polygon(
        vec![center + normal * 3.5, tip, center - normal * 3.5],
        kit::primary(1.0),
        Stroke::NONE,
    ));
    painter.circle_filled(center, 7.0, kit::primary(1.0));
    painter.circle_filled(center, 3.5, accent);
    // A bead on the arc where the reading lands.
    let bead = center + direction * radius;
    painter.circle_filled(bead, thickness * 0.5 + 2.5, theme::surface());
    painter.circle_filled(bead, thickness * 0.5 - 0.5, kit::primary(1.0));

    for (text, x) in [
        ("Extreme Fear", center.x - radius),
        ("Extreme Greed", center.x + radius),
    ] {
        let galley = kit::galley(
            ui,
            text,
            9.0,
            REGULAR,
            theme::text_secondary(),
            radius * 1.2,
        );
        painter.galley(
            pos2(x - galley.size().x / 2.0, center.y + 12.0),
            galley,
            Color32::PLACEHOLDER,
        );
    }
}

/// Yesterday / last week / last month readings and the 30-day range.
fn stats(ui: &Ui, painter: &Painter, rect: Rect, reading: &LatestReading) {
    let mut cells: Vec<(&str, String, Color32, String)> = Vec::new();
    for (title, days) in [("Yesterday", 1), ("Last Week", 7), ("Last Month", 30)] {
        if let Some((_, value)) = reading.days_ago(days) {
            cells.push((
                title,
                format!("{value:.0}"),
                zone_color(value),
                ZONES[zone_index(value)].2.to_owned(),
            ));
        }
    }
    if let Some(((_, low), (_, high))) = reading.range(30) {
        cells.push((
            "30D Range",
            format!("{low:.0} – {high:.0}"),
            theme::text_primary(),
            format!(
                "{} → {}",
                ZONES[zone_index(low)].2,
                ZONES[zone_index(high)].2
            ),
        ));
    }
    if cells.is_empty() {
        return;
    }
    let column = rect.width() / cells.len() as f32;
    for (index, (title, value, color, detail)) in cells.iter().enumerate() {
        let x = rect.min.x + column * index as f32;
        if index > 0 {
            painter.vline(
                x,
                rect.min.y + 4.0..=rect.max.y - 4.0,
                Stroke::new(1.0, theme::separator()),
            );
        }
        let max = column - 10.0;
        let title = kit::galley(ui, title, 10.0, REGULAR, theme::text_secondary(), max);
        let value = kit::digits(ui, value, 15.0, SEMIBOLD, *color, max);
        let detail = kit::galley(ui, detail, 10.0, REGULAR, theme::text_secondary(), max);
        let total = title.size().y + value.size().y + detail.size().y + 2.0;
        let mut y = rect.min.y + (rect.height() - total) / 2.0;
        let center = x + column / 2.0;
        for galley in [title, value, detail] {
            let size = galley.size();
            painter.galley(pos2(center - size.x / 2.0, y), galley, Color32::PLACEHOLDER);
            y += size.y + 1.0;
        }
    }
}

/// The daily history as a zone-colored line over a soft fill, with the zone edges as guides and a
/// hover readout.
fn sparkline(ui: &Ui, painter: &Painter, rect: Rect, history: &[(i64, f64)]) {
    if history.len() < 2 {
        return;
    }
    let plot = Rect::from_min_max(rect.min + vec2(0.0, 12.0), rect.max - vec2(22.0, 0.0));
    let first = history[0].0;
    let span = (history[history.len() - 1].0 - first).max(1) as f32;
    let to_point = |(time, value): (i64, f64)| {
        pos2(
            plot.min.x + plot.width() * (time - first) as f32 / span,
            plot.max.y - plot.height() * (value.clamp(0.0, 100.0) / 100.0) as f32,
        )
    };

    for level in [25.0, 50.0, 75.0] {
        let y = plot.max.y - plot.height() * level / 100.0;
        painter.hline(plot.x_range(), y, Stroke::new(1.0, theme::separator()));
        painter.text(
            pos2(rect.max.x, y),
            Align2::RIGHT_CENTER,
            format!("{level:.0}"),
            egui::FontId::proportional(9.0),
            theme::text_tertiary(),
        );
    }
    painter.text(
        rect.min,
        Align2::LEFT_TOP,
        format!("{}D HISTORY", history.len()),
        egui::FontId::proportional(9.0),
        theme::text_secondary(),
    );

    let points: Vec<Pos2> = history.iter().map(|point| to_point(*point)).collect();
    let mut fill = Mesh::default();
    for (index, point) in history.iter().enumerate() {
        let color = zone_color(point.1);
        fill.colored_vertex(points[index], fade(color, 70));
        fill.colored_vertex(pos2(points[index].x, plot.max.y), fade(color, 0));
        if index > 0 {
            let base = (index * 2) as u32;
            fill.add_triangle(base - 2, base - 1, base);
            fill.add_triangle(base - 1, base, base + 1);
        }
    }
    painter.add(Shape::mesh(fill));
    for index in 1..points.len() {
        painter.line_segment(
            [points[index - 1], points[index]],
            Stroke::new(1.8, zone_color(history[index].1)),
        );
    }
    let last = points[points.len() - 1];
    let last_color = zone_color(history[history.len() - 1].1);
    painter.circle_filled(last, 5.0, fade(last_color, 60));
    painter.circle_filled(last, 2.5, last_color);

    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    if !rect.contains(pointer) {
        return;
    }
    let index = points
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1.x - pointer.x)
                .abs()
                .total_cmp(&(b.1.x - pointer.x).abs())
        })
        .map_or(0, |(index, _)| index);
    let (time, value) = history[index];
    let at = points[index];
    painter.vline(
        at.x,
        plot.y_range(),
        Stroke::new(1.0, theme::text_tertiary()),
    );
    painter.circle_filled(at, 4.0, theme::surface());
    painter.circle_filled(at, 3.0, zone_color(value));
    let text = format!(
        "{} · {:.0} {}",
        date_label(time),
        value,
        ZONES[zone_index(value)].2
    );
    let galley = kit::galley(ui, &text, 10.0, MEDIUM, kit::primary(1.0), 260.0);
    let size = galley.size() + vec2(12.0, 6.0);
    let left = (at.x - size.x / 2.0).clamp(rect.min.x, (rect.max.x - size.x).max(rect.min.x));
    let badge = Rect::from_min_size(pos2(left, rect.min.y - 2.0), size);
    painter.rect_filled(badge, 5.0, theme::chrome());
    painter.rect_stroke(
        badge,
        5.0,
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
    painter.galley(badge.min + vec2(6.0, 3.0), galley, Color32::PLACEHOLDER);
}

/// Epoch milliseconds → "Sep 27".
fn date_label(epoch_ms: i64) -> String {
    let (_, month, day) = crate::format::civil_from_days(epoch_ms.div_euclid(86_400_000));
    format!("{} {day}", MONTHS[(month as usize).clamp(1, 12) - 1])
}

fn altcoin(ui: &Ui, painter: &Painter, rect: Rect, reading: &LatestReading) {
    let compact = rect.height() < 190.0;
    let very_compact = rect.height() < 145.0;
    let spacing = if very_compact {
        2.0
    } else if compact {
        4.0
    } else {
        7.0
    };
    let inner = rect.shrink2(vec2(16.0, if compact { 6.0 } else { 12.0 }));
    let width = inner.width();
    let mut y = inner.min.y;

    let number = score(
        ui,
        reading.value,
        if very_compact {
            25.0
        } else if compact {
            29.0
        } else {
            34.0
        },
        if very_compact { 11.0 } else { 13.0 },
        width,
    );
    painter.galley(
        pos2(inner.center().x - number.size().x / 2.0, y),
        number.clone(),
        Color32::PLACEHOLDER,
    );
    y += number.size().y + spacing;

    let label = kit::galley(
        ui,
        &reading.classification,
        if very_compact {
            12.0
        } else if compact {
            14.0
        } else {
            16.0
        },
        SEMIBOLD,
        kit::primary(1.0),
        width,
    );
    painter.galley(
        pos2(inner.center().x - label.size().x / 2.0, y),
        label.clone(),
        Color32::PLACEHOLDER,
    );
    y += label.size().y + spacing;

    let scale_height = if compact { 26.0 } else { 34.0 };
    season_scale(
        ui,
        painter,
        Rect::from_min_size(pos2(inner.min.x, y), vec2(width, scale_height)),
        reading.value,
        compact,
    );
    y += scale_height + spacing;

    let statistics = [
        reading
            .yearly_high
            .as_ref()
            .map(|(value, date)| ("Yearly High", (*value as i64).to_string(), short_date(date))),
        reading
            .yearly_low
            .as_ref()
            .map(|(value, date)| ("Yearly Low", (*value as i64).to_string(), short_date(date))),
        Some((
            "Altcoin Market Cap",
            reading.altcoin_market_cap.map_or_else(
                || "—".to_owned(),
                |cap| format!("${}", crate::model::watchlist::quote::compact_number(cap)),
            ),
            String::new(),
        )),
    ];
    let statistics: Vec<_> = statistics.into_iter().flatten().collect();
    let height = if compact { 44.0 } else { 52.0 };
    if y + height > inner.max.y + 6.0 {
        return;
    }
    let column = width / statistics.len() as f32;
    for (index, (title, value, detail)) in statistics.iter().enumerate() {
        let x = inner.min.x + column * index as f32;
        if index > 0 {
            painter.vline(
                x,
                y + 4.0..=y + height - 4.0,
                Stroke::new(1.0, theme::separator()),
            );
        }
        let center = x + column / 2.0;
        let max = column - 8.0;
        let title = kit::galley(
            ui,
            title,
            if compact { 10.0 } else { 11.0 },
            REGULAR,
            theme::text_secondary(),
            max,
        );
        let value = kit::digits(ui, value, 13.0, SEMIBOLD, kit::primary(1.0), max);
        let detail = (!compact && !detail.is_empty())
            .then(|| kit::galley(ui, detail, 10.0, REGULAR, theme::text_secondary(), max));
        let gap = if compact { 0.0 } else { 2.0 };
        let total = title.size().y
            + gap
            + value.size().y
            + detail.as_ref().map_or(0.0, |detail| gap + detail.size().y);
        let mut row = y + (height - total) / 2.0;
        for galley in [Some(title), Some(value), detail].into_iter().flatten() {
            let size = galley.size();
            painter.galley(
                pos2(center - size.x / 2.0, row),
                galley,
                Color32::PLACEHOLDER,
            );
            row += size.y + gap;
        }
    }
}

/// An orange → gray → blue capsule with a triangle marker above `value` and the season labels
/// underneath.
fn season_scale(ui: &Ui, painter: &Painter, rect: Rect, value: f64, compact: bool) {
    let bar_height = if compact { 7.0 } else { 9.0 };
    let marker = if compact {
        vec2(9.0, 7.0)
    } else {
        vec2(11.0, 8.0)
    };
    let bar = Rect::from_min_size(
        pos2(rect.min.x, rect.min.y + marker.y + 2.0),
        vec2(rect.width(), bar_height),
    );
    painter.add(gradient_capsule(bar, &[ORANGE, GRAY, BLUE]));

    let fraction = value.clamp(0.0, 100.0) as f32 / 100.0;
    let left = bar.min.x + fraction * (bar.width() - marker.x);
    let top = bar.min.y - marker.y - 1.0;
    painter.add(Shape::convex_polygon(
        vec![
            pos2(left, top),
            pos2(left + marker.x, top),
            pos2(left + marker.x / 2.0, top + marker.y),
        ],
        kit::primary(1.0),
        Stroke::NONE,
    ));

    let size = if compact { 8.0 } else { 10.0 };
    let label_y = bar.max.y + 3.0;
    let third = rect.width() / 3.0;
    for (text, align, x) in [
        ("0  Bitcoin Season", Align2::LEFT_TOP, rect.min.x),
        ("Neutral", Align2::CENTER_TOP, rect.center().x),
        ("Altcoin Season  100", Align2::RIGHT_TOP, rect.max.x),
    ] {
        let galley = kit::galley(ui, text, size, REGULAR, theme::text_secondary(), third);
        let anchored = align.anchor_size(pos2(x, label_y), galley.size());
        painter.galley(anchored.min, galley, Color32::PLACEHOLDER);
    }
}

/// A capsule filled with a horizontal gradient through `stops`, as a mesh of one-point columns
/// whose height follows the rounded ends.
fn gradient_capsule(rect: Rect, stops: &[Color32]) -> Shape {
    let mut mesh = Mesh::default();
    let radius = rect.height() / 2.0;
    let columns = rect.width().ceil().max(2.0) as usize;
    for column in 0..=columns {
        let t = column as f32 / columns as f32;
        let x = rect.min.x + rect.width() * t;
        let from_edge = (x - rect.min.x).min(rect.max.x - x);
        let half = if from_edge >= radius {
            radius
        } else {
            let offset = radius - from_edge;
            (radius * radius - offset * offset).max(0.0).sqrt()
        };
        let color = sample(stops, t);
        let index = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x, rect.center().y - half), color);
        mesh.colored_vertex(pos2(x, rect.center().y + half), color);
        if column > 0 {
            mesh.add_triangle(index - 2, index - 1, index);
            mesh.add_triangle(index - 1, index, index + 1);
        }
    }
    Shape::mesh(mesh)
}

fn sample(stops: &[Color32], t: f32) -> Color32 {
    let scaled = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let index = (scaled.floor() as usize).min(stops.len() - 2);
    let local = scaled - index as f32;
    let (from, to) = (stops[index], stops[index + 1]);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * local).round() as u8;
    Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "2026-09-27" → "Sep 27".
fn short_date(text: &str) -> String {
    let mut parts = text.get(..10).unwrap_or(text).split('-');
    let (Some(_), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return text.to_owned();
    };
    match (month.parse::<usize>(), day.parse::<u32>()) {
        (Ok(month @ 1..=12), Ok(day)) => format!("{} {day}", MONTHS[month - 1]),
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_dates_read_like_swift() {
        assert_eq!(short_date("2026-09-27"), "Sep 27");
        assert_eq!(short_date("2025-12-19T00:00:00.000Z"), "Dec 19");
        assert_eq!(short_date("soon"), "soon");
    }

    /// Paints `symbol`'s gauge on a headless frame, returning the shapes drawn.
    fn painted(symbol: &str, reading: &LatestReading, size: Vec2) -> usize {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), size);
            paint(ui, ui.painter(), rect, symbol, reading);
        });
        output.textures_delta.clear();
        output.shapes.len()
    }

    fn reading(history: usize) -> LatestReading {
        LatestReading {
            value: 56.0,
            classification: "Greed".to_owned(),
            yearly_high: None,
            yearly_low: None,
            altcoin_market_cap: None,
            history: (0..history as i64)
                .map(|day| (day * 86_400_000, (day * 7 % 100) as f64))
                .collect(),
        }
    }

    #[test]
    fn the_gauge_draws_at_every_size_with_or_without_history() {
        for history in [0, 1, 2, 30, 90] {
            for (width, height) in [
                (120.0, 60.0),
                (260.0, 140.0),
                (420.0, 200.0),
                (700.0, 420.0),
            ] {
                let drawn = painted(
                    cmc::FEAR_GREED_LATEST,
                    &reading(history),
                    vec2(width, height),
                );
                assert!(drawn > 0, "{history} points at {width}x{height}");
            }
        }
    }

    #[test]
    fn a_tall_card_adds_the_history_panels() {
        let compact = painted(cmc::FEAR_GREED_LATEST, &reading(90), vec2(420.0, 160.0));
        let tall = painted(cmc::FEAR_GREED_LATEST, &reading(90), vec2(420.0, 420.0));
        let bare = painted(cmc::FEAR_GREED_LATEST, &reading(0), vec2(420.0, 420.0));
        assert!(tall > compact);
        assert!(tall > bare);
    }

    #[test]
    fn zones_follow_the_cmc_bounds() {
        assert_eq!(ZONES[zone_index(10.0)].2, "Extreme Fear");
        assert_eq!(ZONES[zone_index(25.0)].2, "Fear");
        assert_eq!(ZONES[zone_index(50.0)].2, "Neutral");
        assert_eq!(ZONES[zone_index(60.0)].2, "Greed");
        assert_eq!(ZONES[zone_index(100.0)].2, "Extreme Greed");
    }

    #[test]
    fn epoch_dates_label() {
        assert_eq!(date_label(1_791_504_000_000), "Oct 9");
    }

    #[test]
    fn the_gradient_passes_through_each_stop() {
        assert_eq!(sample(&[ORANGE, GRAY, BLUE], 0.0), ORANGE);
        assert_eq!(sample(&[ORANGE, GRAY, BLUE], 0.5), GRAY);
        assert_eq!(sample(&[ORANGE, GRAY, BLUE], 1.0), BLUE);
    }
}
