//! The P&L by asset panel: one bar per asset.

use super::{PNL_COMPACT, PNL_ROW_MAX, PNL_ROW_MIN};
use crate::model::portfolio::PortfolioAsset;
use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::model::portfolio::statistics::{AssetResult, pnl_bars};
use crate::ui::accessibility;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, MEDIUM, REGULAR};
use crate::ui::portfolio::blocks::{caption_color, draw, panel, tone_or};
use crate::ui::portfolio::empty;
use crate::ui::portfolio::table::{self, tone_option};
use crate::ui::theme;
use eframe::egui::{self, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

pub(super) fn pnl_panel(
    ui: &mut Ui,
    results: &[AssetResult],
    f: PortfolioValueFormatter,
    content_height: f32,
) {
    panel(
        ui,
        content_height,
        Icon::BarChart,
        "P&L by Asset",
        |header, ui| {
            header.label(kit::text(
                format!("Total, in {}", f.currency.code()),
                kit::size::SUBHEADLINE,
                REGULAR,
                caption_color(),
            ));
            pnl_bars_view(ui, results, f, content_height);
        },
    );
}

/// Horizontal bars of each asset's total P&L around a zero line: gains grow right, losses left.
/// Rows share `content_height` between them, within a comfortable range.
fn pnl_bars_view(
    ui: &mut egui::Ui,
    results: &[AssetResult],
    f: PortfolioValueFormatter,
    content_height: f32,
) {
    if results.is_empty() {
        let area = ui.max_rect();
        empty::compact(
            ui,
            area,
            empty::Scene::Results,
            "No Results Yet",
            "Priced holdings appear here.",
        );
        return;
    }
    let (bars, zero) = pnl_bars(results);
    let secondary = caption_color();
    let row_height = (content_height / results.len() as f32).clamp(PNL_ROW_MIN, PNL_ROW_MAX);
    ui.spacing_mut().item_spacing.y = 0.0;
    let compact = ui.available_width() < PNL_COMPACT;
    for (result, bar) in results.iter().zip(&bars) {
        let (row, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::hover());
        if response.hovered() {
            ui.painter()
                .rect_filled(row.expand2(vec2(8.0, -1.0)), 8.0, theme::ink(10));
        }
        let middle = row.center().y;

        // Asset: icon and ticker in a 96 pt column.
        paint_asset_icon_at(ui, pos2(row.left() + 11.0, middle), &result.asset, 22.0);
        let ticker = kit::galley(
            ui,
            &result.asset.display_ticker(),
            kit::size::SUBHEADLINE,
            MEDIUM,
            theme::text_primary(),
            62.0,
        );
        draw(
            ui,
            pos2(row.left() + 30.0, middle - ticker.size().y / 2.0),
            &ticker,
        );

        // Figures, right-aligned at the row's end: the amount, then the percentage.
        let tone = f.tone(Some(result.total_pnl));
        let mut figures_left = row.right();
        if !compact {
            let percent = kit::galley(
                ui,
                &result
                    .pnl_percent
                    .map_or("—".to_owned(), |p| f.signed_percent(p, 2)),
                kit::size::SUBHEADLINE,
                REGULAR,
                tone_or(f.tone(result.pnl_percent), secondary),
                76.0,
            );
            draw(
                ui,
                pos2(
                    row.right() - percent.size().x,
                    middle - percent.size().y / 2.0,
                ),
                &percent,
            );
            figures_left -= 76.0 + 10.0;
        }
        let amount = kit::galley(
            ui,
            &f.signed_money(result.total_pnl),
            kit::size::SUBHEADLINE,
            REGULAR,
            tone_or(tone, theme::text_primary()),
            110.0,
        );
        draw(
            ui,
            pos2(
                figures_left - amount.size().x,
                middle - amount.size().y / 2.0,
            ),
            &amount,
        );

        // The bar track fills what the other columns leave.
        let track = Rect::from_min_max(
            pos2(row.left() + 96.0 + 14.0, middle - 5.0),
            pos2(figures_left - 110.0 - 14.0, middle + 5.0),
        );
        if track.width() > 20.0 {
            ui.painter().rect_filled(track, 5.0, theme::ink(8));
            let zero_x = track.left() + track.width() * zero as f32;
            // Neutral while values are hidden: red and green would give each sign away.
            let colour = tone_option(tone).map_or(theme::ink(80), |c| c.gamma_multiply(0.9));
            let from = track.left() + track.width() * bar.start as f32;
            let length = (track.width() * bar.length as f32).max(3.0);
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(from, track.top()), vec2(length, track.height())),
                5.0,
                colour,
            );
            ui.painter().vline(
                zero_x,
                egui::Rangef::new(track.top() - 3.0, track.bottom() + 3.0),
                Stroke::new(1.0, theme::ink(70)),
            );
        }
        accessibility::label_as(
            response,
            egui::WidgetType::Other,
            format!(
                "{}: {} ({})",
                result.asset.display_ticker(),
                f.signed_money(result.total_pnl),
                result
                    .pnl_percent
                    .map_or("—".to_owned(), |p| f.signed_percent(p, 2)),
            ),
        );
    }
}

/// An asset's icon centred at `centre`, painted without moving the layout cursor.
fn paint_asset_icon_at(ui: &Ui, centre: Pos2, asset: &PortfolioAsset, size: f32) {
    table::paint_asset_icon(ui, Rect::from_center_size(centre, Vec2::splat(size)), asset);
}
