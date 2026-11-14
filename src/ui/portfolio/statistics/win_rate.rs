//! The win rate panel and its ring.

use super::hero::win_shares;
use crate::model::portfolio::statistics::PortfolioStatistics;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, REGULAR, SEMIBOLD};
use crate::ui::portfolio::blocks::{caption_color, draw, line, panel, text_at};
use crate::ui::portfolio::empty;
use crate::ui::theme;
use eframe::egui::{Align2, Color32, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};

pub(super) fn win_panel(ui: &mut Ui, stats: &PortfolioStatistics, content_height: f32) {
    panel(ui, content_height, Icon::Gauge, "Win Rate", |header, ui| {
        let priced = stats.asset_results.len();
        let area = ui.max_rect();
        if priced == 0 {
            empty::compact(
                ui,
                area,
                empty::Scene::Allocation,
                "No Win Rate Yet",
                "Priced holdings appear here.",
            );
            return;
        }
        header.label(kit::text(
            format!("{priced} priced"),
            kit::size::SUBHEADLINE,
            REGULAR,
            caption_color(),
        ));
        paint_win_rate(ui, area, stats);
    });
}

/// A ring of in profit, at a loss and level, with its legend beside it, the pair centred in
/// `area`.
fn paint_win_rate(ui: &Ui, area: Rect, stats: &PortfolioStatistics) {
    let secondary = caption_color();
    let priced = stats.asset_results.len();
    let shares = win_shares(stats.winners, stats.losers, priced);
    let level = priced.saturating_sub(stats.winners + stats.losers);
    let unpriced = stats.asset_count.saturating_sub(priced);

    let diameter = area.height().min(132.0).min(area.width() * 0.45);
    let legend_width = (area.width() - diameter - 28.0).clamp(120.0, 210.0);
    let block = diameter + 28.0 + legend_width;
    let left = (area.center().x - block / 2.0).max(area.left());
    let centre = pos2(left + diameter / 2.0, area.center().y);
    let width = 14.0;
    let radius = diameter / 2.0 - width / 2.0;

    ring(
        ui,
        centre,
        radius,
        width,
        &[
            (shares[0], theme::UP),
            (shares[1], theme::DOWN),
            (shares[2], theme::text_tertiary()),
        ],
    );
    let rate = line(
        ui,
        &format!("{:.0}%", shares[0] * 100.0),
        24.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    let caption = line(ui, "win rate", kit::size::CAPTION, REGULAR, secondary);
    let stack = rate.size().y + caption.size().y;
    let top = centre.y - stack / 2.0;
    draw(ui, pos2(centre.x - rate.size().x / 2.0, top), &rate);
    draw(
        ui,
        pos2(centre.x - caption.size().x / 2.0, top + rate.size().y),
        &caption,
    );

    let mut rows = vec![
        (theme::UP, "In profit", stats.winners),
        (theme::DOWN, "At a loss", stats.losers),
        (theme::text_tertiary(), "Break-even", level),
    ];
    if unpriced > 0 {
        rows.push((theme::ink(80), "No price", unpriced));
    }
    let row_height = 34.0;
    let x0 = left + diameter + 28.0;
    let first = area.center().y - rows.len() as f32 * row_height / 2.0;
    for (index, (colour, label, count)) in rows.iter().enumerate() {
        let middle = first + (index as f32 + 0.5) * row_height;
        ui.painter()
            .circle_filled(pos2(x0 + 5.0, middle), 4.5, *colour);
        text_at(
            ui,
            pos2(x0 + 18.0, middle),
            Align2::LEFT_CENTER,
            label,
            kit::size::SUBHEADLINE,
            REGULAR,
            secondary,
        );
        text_at(
            ui,
            pos2(x0 + legend_width, middle),
            Align2::RIGHT_CENTER,
            &count.to_string(),
            15.0,
            SEMIBOLD,
            theme::text_primary(),
        );
        if index + 1 < rows.len() {
            ui.painter().hline(
                x0..=x0 + legend_width,
                middle + row_height / 2.0,
                Stroke::new(1.0, theme::separator().gamma_multiply(0.5)),
            );
        }
    }
}

/// Segments of a ring, each a share of the whole in its colour, on a faint track. Several
/// segments are kept apart by a small gap.
fn ring(ui: &Ui, centre: Pos2, radius: f32, width: f32, segments: &[(f32, Color32)]) {
    use std::f32::consts::{FRAC_PI_2, TAU};
    ui.painter()
        .circle_stroke(centre, radius, Stroke::new(width, theme::ink(10)));
    let shown: Vec<&(f32, Color32)> = segments.iter().filter(|(share, _)| *share > 0.0).collect();
    if let [(_, colour)] = shown.as_slice() {
        ui.painter()
            .circle_stroke(centre, radius, Stroke::new(width, *colour));
        return;
    }
    let gap = 0.06;
    let mut start = -FRAC_PI_2;
    for (share, colour) in shown {
        let sweep = share * TAU;
        let (from, to) = (start + gap / 2.0, start + sweep - gap / 2.0);
        start += sweep;
        if to <= from {
            continue;
        }
        let steps = ((to - from) * radius / 2.0).ceil().max(2.0) as usize;
        let points = (0..=steps)
            .map(|step| {
                let angle = from + (to - from) * step as f32 / steps as f32;
                centre + vec2(angle.cos(), angle.sin()) * radius
            })
            .collect();
        ui.painter()
            .add(Shape::line(points, Stroke::new(width, *colour)));
    }
}
