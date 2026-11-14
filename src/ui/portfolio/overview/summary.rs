//! The row of summary cards under the hero, and the best and worst performer cards.

use super::OverviewInput;
use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::model::portfolio::statistics::{AssetResult, PortfolioStatistics};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::portfolio::blocks::{
    CARD_HEIGHT, StatCard, caption_color, card_frame, draw, grid, line, text_at, tone_or,
};
use crate::ui::portfolio::table::{self, tone_color};
use crate::ui::theme;
use eframe::egui::{self, Align2, Rect, UiBuilder, Vec2, pos2};

pub(super) fn summary_cards(
    ui: &mut egui::Ui,
    f: PortfolioValueFormatter,
    stats: &PortfolioStatistics,
) {
    grid(
        ui,
        &[
            StatCard::new("Cost Basis", f.money(stats.cost_basis)).icon(Icon::Banknote),
            StatCard::new("All-time P&L", f.signed_money(stats.total_pnl()))
                .tone(f.tone(Some(stats.total_pnl())))
                .icon(Icon::ChartUptrend),
            StatCard::new("Unrealized P&L", f.signed_money(stats.unrealized_pnl))
                .tone(f.tone(Some(stats.unrealized_pnl)))
                .caption(stats.unrealized_percent.map(|p| f.signed_percent(p, 2)))
                .icon(Icon::Clock),
            StatCard::new("Realized P&L", f.signed_money(stats.realized_pnl))
                .tone(f.tone(Some(stats.realized_pnl)))
                .icon(Icon::CheckmarkSeal),
        ],
    );
}

pub(super) fn performer_card(
    ui: &mut egui::Ui,
    title: &str,
    result: Option<&AssetResult>,
    input: &OverviewInput,
    height: f32,
) {
    if height > CARD_HEIGHT * 1.5 {
        return performer_tile(ui, title, result, input, height);
    }
    let f = input.formatter;
    let rect = card_frame(ui, height, None);
    let left = rect.left() + 14.0;
    let right = rect.right() - 14.0;
    let secondary = caption_color();
    let top = rect.top() + 12.0;
    let heading = text_at(
        ui,
        pos2(left, top),
        Align2::LEFT_TOP,
        title,
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let row = Rect::from_min_max(
        pos2(left, heading.bottom() + 8.0),
        pos2(right, rect.bottom() - 12.0),
    );
    let Some(result) = result else {
        text_at(
            ui,
            pos2(left, row.center().y),
            Align2::LEFT_CENTER,
            "—",
            15.0,
            SEMIBOLD,
            secondary,
        );
        return;
    };

    let icon = Rect::from_min_size(row.min, Vec2::splat(row.height().min(30.0)));
    ui.scope_builder(UiBuilder::new().max_rect(icon), |ui| {
        table::asset_icon(ui, &result.asset, icon.width());
    });

    let percent = result
        .pnl_percent
        .map_or("—".to_owned(), |p| f.signed_percent(p, 2));
    let big = line(
        ui,
        &percent,
        15.0,
        SEMIBOLD,
        tone_or(f.tone(result.pnl_percent), theme::text_primary()),
    );
    let small = line(
        ui,
        &f.signed_money(result.total_pnl),
        kit::size::CAPTION,
        REGULAR,
        tone_or(f.tone(Some(result.total_pnl)), secondary),
    );
    let figures_height = big.size().y + 1.0 + small.size().y;
    let figures_top = row.center().y - figures_height / 2.0;
    draw(ui, pos2(right - big.size().x, figures_top), &big);
    draw(
        ui,
        pos2(right - small.size().x, figures_top + big.size().y + 1.0),
        &small,
    );
    let figures_width = big.size().x.max(small.size().x);

    let text_left = icon.right() + 10.0;
    let text_width = (right - figures_width - 12.0 - text_left).max(30.0);
    let ticker = kit::galley(
        ui,
        &result.asset.display_ticker(),
        kit::size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let name = kit::galley(
        ui,
        &(input.subtitle)(&result.asset),
        kit::size::CAPTION,
        REGULAR,
        secondary,
        text_width,
    );
    let names_top = row.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    draw(ui, pos2(text_left, names_top), &ticker);
    draw(
        ui,
        pos2(text_left, names_top + ticker.size().y + 1.0),
        &name,
    );
}

/// A performer as a tall tile: which it is, what it is, then how far it moved in large type.
fn performer_tile(
    ui: &mut egui::Ui,
    title: &str,
    result: Option<&AssetResult>,
    input: &OverviewInput,
    height: f32,
) {
    let f = input.formatter;
    let best = title.starts_with("Best");
    let tone = result.and_then(|result| f.tone(Some(result.total_pnl)));
    let rect = card_frame(ui, height, tone.map(tone_color));
    let (left, right) = (rect.left() + 18.0, rect.right() - 18.0);
    let secondary = caption_color();
    let accent = if best { kit::GREEN } else { kit::RED };
    let badge = Rect::from_center_size(pos2(left + 10.0, rect.top() + 24.0), Vec2::splat(20.0));
    ui.painter()
        .rect_filled(badge, 6.0, accent.gamma_multiply(0.16));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(badge.center(), Vec2::splat(11.0)),
        if best {
            Icon::ArrowUpRight
        } else {
            Icon::ArrowDownRight
        },
        accent,
    );
    text_at(
        ui,
        pos2(badge.right() + 8.0, badge.center().y),
        Align2::LEFT_CENTER,
        title,
        kit::size::SUBHEADLINE,
        MEDIUM,
        secondary,
    );
    let Some(result) = result else {
        text_at(
            ui,
            pos2(left, rect.center().y + 10.0),
            Align2::LEFT_CENTER,
            "No priced holdings yet",
            kit::size::SUBHEADLINE,
            REGULAR,
            secondary,
        );
        return;
    };

    let icon = Rect::from_min_size(pos2(left, badge.bottom() + 14.0), Vec2::splat(36.0));
    // Painted in place: a scope would move the column's cursor back up to the icon.
    table::paint_asset_icon(ui, icon, &result.asset);
    let text_left = icon.right() + 12.0;
    let width = (right - text_left).max(30.0);
    let ticker = kit::galley(
        ui,
        &result.asset.display_ticker(),
        kit::size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let name = kit::galley(
        ui,
        &(input.subtitle)(&result.asset),
        kit::size::CAPTION,
        REGULAR,
        secondary,
        width,
    );
    let top = icon.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    draw(ui, pos2(text_left, top), &ticker);
    draw(ui, pos2(text_left, top + ticker.size().y + 1.0), &name);

    let amount = text_at(
        ui,
        pos2(left, rect.bottom() - 16.0),
        Align2::LEFT_BOTTOM,
        &f.signed_money(result.total_pnl),
        kit::size::SUBHEADLINE,
        MEDIUM,
        tone_or(tone, secondary),
    );
    text_at(
        ui,
        pos2(left, amount.top() - 2.0),
        Align2::LEFT_BOTTOM,
        &result
            .pnl_percent
            .map_or("\u{2014}".to_owned(), |p| f.signed_percent(p, 2)),
        26.0,
        SEMIBOLD,
        tone_or(tone, theme::text_primary()),
    );
}
