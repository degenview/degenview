//! The Overview's hero: the total, its change and the sparkline beside it.

use super::{OverviewInput, OverviewState, asset_count_text};
use crate::model::portfolio::PortfolioSnapshot;
use crate::model::portfolio::statistics::{PortfolioPeriodChange, PortfolioStatistics};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::{self, REGULAR, SEMIBOLD};
use crate::ui::portfolio::blocks::{
    HERO_HEIGHT, HERO_PADDING, caption_color, change_block, glow, lift, text_at,
};
use crate::ui::portfolio::charts;
use crate::ui::portfolio::table::tone_color;
use crate::ui::theme;
use eframe::egui::{self, Align2, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

pub(super) fn hero(
    ui: &mut Ui,
    state: &OverviewState,
    input: &OverviewInput,
    stats: &PortfolioStatistics,
    period: Option<PortfolioPeriodChange>,
) {
    let f = input.formatter;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HERO_HEIGHT), Sense::hover());
    let tint = f
        .tone(period.map(|p| p.profit))
        .map_or(theme::SYSTEM_ACCENT, tone_color);
    lift(ui, rect, 16.0);
    if !input.is_loading_initial && !input.holdings.is_empty() {
        glow(ui, rect, tint);
    }
    let inner = rect.shrink(HERO_PADDING);

    // The portfolio's name on a chip of its own.
    let chip = Rect::from_center_size(
        pos2(inner.left() + 11.0, inner.top() + 11.0),
        Vec2::splat(22.0),
    );
    ui.painter()
        .rect_filled(chip, 7.0, theme::SYSTEM_ACCENT.gamma_multiply(0.14));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(chip.center(), Vec2::splat(13.0)),
        Icon::Briefcase,
        theme::SYSTEM_ACCENT,
    );
    let name = text_at(
        ui,
        pos2(chip.right() + 9.0, chip.center().y),
        Align2::LEFT_CENTER,
        input.portfolio_name.unwrap_or("All Portfolios"),
        15.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    let count = text_at(
        ui,
        pos2(name.right() + 6.0, chip.center().y),
        Align2::LEFT_CENTER,
        &format!("\u{00B7} {}", asset_count_text(input.holdings.len())),
        15.0,
        REGULAR,
        caption_color(),
    );

    let balance_top = inner.top() + 32.0;
    let balance_middle = balance_top + 24.0;
    let mut left_end = count.right();
    if input.is_loading_initial {
        // Painted, not `put`: `put` would move the cursor back up to the spinner, and the cards
        // below would start under the hero.
        LoadingIndicator::new().size(16.0).paint_at(
            ui,
            Rect::from_center_size(pos2(inner.left() + 8.0, balance_middle), Vec2::splat(16.0)),
        );
        let loading = text_at(
            ui,
            pos2(inner.left() + 26.0, balance_middle),
            Align2::LEFT_CENTER,
            "Loading portfolio value\u{2026}",
            12.0,
            REGULAR,
            caption_color(),
        );
        left_end = left_end.max(loading.right());
    } else {
        let balance = text_at(
            ui,
            pos2(inner.left(), balance_top),
            Align2::LEFT_TOP,
            &f.money(input.total_value),
            40.0,
            SEMIBOLD,
            theme::text_primary(),
        );
        left_end = left_end.max(balance.right());
    }

    // Where the prices stand: a dot in the state's colour, then the caption.
    let caption_middle = inner.bottom() - 6.0;
    if !input.is_loading_initial {
        ui.painter().circle_filled(
            pos2(inner.left() + 4.0, caption_middle),
            3.5,
            input.price_status.color(),
        );
    }
    let caption = text_at(
        ui,
        pos2(
            inner.left() + if input.is_loading_initial { 0.0 } else { 14.0 },
            caption_middle,
        ),
        Align2::LEFT_CENTER,
        input.market_value_caption,
        kit::size::CAPTION,
        REGULAR,
        caption_color(),
    );
    left_end = left_end.max(caption.right());

    if input.is_loading_initial {
        return;
    }
    let middle = rect.center().y;
    let pnl_label = format!("P&L \u{00B7} {}", state.range.period_name());
    let edge = change_block(
        ui,
        f,
        &pnl_label,
        period.map(|p| p.profit),
        period.and_then(|p| p.percentage),
        pos2(inner.right(), middle),
    );
    let day_edge = change_block(
        ui,
        f,
        "24h",
        stats.day_change.map(|d| d.amount),
        stats.day_change.and_then(|d| d.percentage),
        pos2(edge - 36.0, middle),
    );
    ui.painter().vline(
        edge - 18.0,
        middle - 26.0..=middle + 26.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
    );

    // Between the two, the shape of the range.
    let room = Rect::from_min_max(
        pos2(left_end + 36.0, inner.top() + 6.0),
        pos2(day_edge - 36.0, inner.bottom() - 6.0),
    );
    if room.width() >= 120.0 && !f.privacy {
        sparkline(
            ui,
            room,
            &charts::prepared_points(
                input.history,
                input.total_value,
                state.range,
                input.now_ms,
                false,
            ),
        );
    }
}

/// The value over the range as a line with a fade under it; green when it ends above where it
/// began, red when below.
fn sparkline(ui: &Ui, rect: Rect, points: &[PortfolioSnapshot]) {
    let Some((minimum, maximum)) = charts::value_bounds(points) else {
        return;
    };
    if points.len() < 2 {
        return;
    }
    let color = if points[points.len() - 1].value >= points[0].value {
        theme::UP
    } else {
        theme::DOWN
    };
    let spread = (maximum - minimum).max(f64::EPSILON);
    let line: Vec<Pos2> = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            pos2(
                rect.left() + rect.width() * index as f32 / (points.len() - 1) as f32,
                rect.bottom() - rect.height() * ((point.value - minimum) / spread) as f32,
            )
        })
        .collect();
    let mut mesh = egui::Mesh::default();
    for point in &line {
        mesh.colored_vertex(*point, color.gamma_multiply(0.20));
        mesh.colored_vertex(pos2(point.x, rect.bottom()), color.gamma_multiply(0.0));
    }
    for step in 0..line.len() as u32 - 1 {
        let base = step * 2;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    ui.painter().add(Shape::mesh(mesh));
    let end = line[line.len() - 1];
    ui.painter().add(Shape::line(line, Stroke::new(2.0, color)));
    ui.painter().circle_filled(end, 3.5, color);
    ui.painter()
        .circle_stroke(end, 3.5, Stroke::new(2.0, theme::chrome()));
}
