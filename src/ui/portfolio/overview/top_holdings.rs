//! The Top Holdings panel and its rows.

use super::{
    OverviewAction, OverviewInput, PANEL_CONTENT_HEIGHT, TOP_COUNT, TOP_ROW_GAP, TOP_ROW_HEIGHT,
    top_holdings,
};
use crate::model::portfolio::PortfolioHolding;
use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::ui::accessibility;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::portfolio::blocks::{caption_color, draw, line, panel};
use crate::ui::portfolio::empty;
use crate::ui::portfolio::table;
use crate::ui::theme;
use eframe::egui::{self, CursorIcon, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};

pub(super) fn top_holdings_panel(
    ui: &mut Ui,
    input: &OverviewInput,
    actions: &mut Vec<OverviewAction>,
) {
    let f = input.formatter;
    let show_all = input.holdings.len() > TOP_COUNT;
    panel(
        ui,
        PANEL_CONTENT_HEIGHT,
        Icon::Layers,
        "Top Holdings",
        |header, ui| {
            if show_all {
                let link = header
                    .add(
                        egui::Label::new(kit::text(
                            "View All",
                            kit::size::SUBHEADLINE,
                            MEDIUM,
                            theme::SYSTEM_ACCENT,
                        ))
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(CursorIcon::PointingHand);
                if link.clicked() {
                    actions.push(OverviewAction::ShowHoldings);
                }
            }
            if input.holdings.is_empty() {
                empty::compact(
                    ui,
                    ui.max_rect(),
                    empty::Scene::Holdings,
                    "No Open Positions",
                    "Assets you hold appear here.",
                );
                return;
            }
            let width = ui.available_width();
            let top = ui.max_rect().top();
            let left = ui.max_rect().left();
            for (index, holding) in top_holdings(input.holdings).into_iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(left, top + index as f32 * (TOP_ROW_HEIGHT + TOP_ROW_GAP)),
                    vec2(width, TOP_ROW_HEIGHT),
                );
                top_holding_row(ui, row, holding, input, f, actions);
            }
        },
    );
}

fn top_holding_row(
    ui: &mut Ui,
    row: Rect,
    holding: &PortfolioHolding,
    input: &OverviewInput,
    f: PortfolioValueFormatter,
    actions: &mut Vec<OverviewAction>,
) {
    let response = ui
        .interact(
            row,
            egui::Id::new(("portfolio-top-holding", &holding.asset.key)),
            Sense::click(),
        )
        .on_hover_text("Shows this asset's transactions")
        .on_hover_cursor(CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(row, 8.0, theme::ink(18));
    }
    let secondary = caption_color();
    let icon = Rect::from_center_size(
        pos2(row.left() + 8.0 + 13.0, row.center().y),
        Vec2::splat(26.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(icon), |ui| {
        table::asset_icon(ui, &holding.asset, 26.0);
    });

    // Right column: the value over an allocation bar and its percentage.
    let right = row.right() - 8.0;
    let value = line(
        ui,
        &holding.current_value.map_or("—".to_owned(), |v| f.money(v)),
        kit::size::SUBHEADLINE,
        REGULAR,
        theme::text_primary(),
    );
    let percent = line(
        ui,
        &f.percent(holding.allocation, 1),
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let right_height = value.size().y + 3.0 + percent.size().y;
    let right_top = row.center().y - right_height / 2.0;
    draw(ui, pos2(right - value.size().x, right_top), &value);
    let line_y = right_top + value.size().y + 3.0;
    draw(ui, pos2(right - percent.size().x, line_y), &percent);
    // The bar grows with the row, so a wide page does not leave it a stub far from its name.
    let bar_width = (row.width() * 0.22).clamp(64.0, 240.0);
    let bar = Rect::from_center_size(
        pos2(
            right - 44.0 - 6.0 - bar_width / 2.0,
            line_y + percent.size().y / 2.0,
        ),
        vec2(bar_width, 4.0),
    );
    ui.painter().rect_filled(bar, 2.0, theme::ink(28));
    let filled = Rect::from_min_size(
        bar.min,
        vec2(bar_width * holding.allocation.clamp(0.0, 1.0) as f32, 4.0),
    );
    ui.painter().rect_filled(filled, 2.0, theme::SYSTEM_ACCENT);

    let text_left = icon.right() + 10.0;
    let text_width = (bar.left() - 12.0 - text_left)
        .min(right - value.size().x - 12.0 - text_left)
        .max(30.0);
    let ticker = kit::galley(
        ui,
        &holding.asset.display_ticker(),
        kit::size::SUBHEADLINE,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let name = kit::galley(
        ui,
        &(input.subtitle)(&holding.asset),
        kit::size::CAPTION,
        REGULAR,
        secondary,
        text_width,
    );
    let top = row.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    draw(ui, pos2(text_left, top), &ticker);
    draw(ui, pos2(text_left, top + ticker.size().y + 1.0), &name);

    if response.clicked() {
        actions.push(OverviewAction::Select(holding.asset.key.clone()));
    }
    let spoken = format!(
        "{}: {} ({} of portfolio)",
        holding.asset.display_ticker(),
        holding.current_value.map_or("—".to_owned(), |v| f.money(v)),
        f.percent(holding.allocation, 1),
    );
    accessibility::label_as(response, egui::WidgetType::Button, spoken);
}
