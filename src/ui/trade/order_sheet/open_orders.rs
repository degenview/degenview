//! The working orders at the foot of the sheet.

use degenview_brokers::Side;
use eframe::egui::{self, vec2};

use super::state::SheetState;
use crate::trading::backend::TradingBackend;
use crate::trading::model::OrderRow;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};

/// The working orders, compact: the full lists are in the orders panel.
pub(super) fn show(ui: &mut egui::Ui, state: &mut SheetState, backend: &dyn TradingBackend) {
    let rows: Vec<OrderRow> = backend
        .orders()
        .into_iter()
        .filter(|row| row.working)
        .collect();
    // A header that folds the list: open while there is something working, shut when there is not,
    // until the user says otherwise.
    let open = state.orders_open.unwrap_or(!rows.is_empty());
    let (head, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::click());
    let response = crate::ui::accessibility::label(response, "Open orders")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        state.orders_open = Some(!open);
    }
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(head.left() + 9.0, head.center().y),
            vec2(16.0, 16.0),
        ),
        Icon::DocText,
        theme::text_secondary(),
    );
    let title = kit::galley(
        ui,
        "Open orders",
        kit::size::BODY,
        kit::SEMIBOLD,
        theme::text_primary(),
        f32::INFINITY,
    );
    let title_width = title.size().x;
    ui.painter().galley(
        egui::pos2(head.left() + 26.0, head.center().y - title.size().y / 2.0),
        title,
        theme::text_primary(),
    );
    let count = kit::galley(
        ui,
        &rows.len().to_string(),
        kit::size::BODY,
        kit::REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    ui.painter().galley(
        egui::pos2(
            head.left() + 26.0 + title_width + 8.0,
            head.center().y - count.size().y / 2.0,
        ),
        count,
        theme::text_secondary(),
    );
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(head.right() - 8.0, head.center().y),
            vec2(12.0, 12.0),
        ),
        if open {
            Icon::ChevronUp
        } else {
            Icon::ChevronDown
        },
        theme::text_secondary(),
    );
    if !open {
        return;
    }
    ui.add_space(6.0);
    if rows.is_empty() {
        ui.label(kit::text(
            "Working orders rest here until they fill or are cancelled.",
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
        return;
    }
    const SHOWN: usize = 6;
    for row in rows.iter().take(SHOWN) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let color = match row.side {
                Side::Buy => theme::UP,
                Side::Sell => theme::DOWN,
            };
            ui.label(kit::text(
                match row.side {
                    Side::Buy => "Buy",
                    Side::Sell => "Sell",
                },
                kit::size::CAPTION + 1.0,
                kit::SEMIBOLD,
                color,
            ));
            let price = row.price.map_or_else(
                || "market".to_owned(),
                crate::model::broker::ticket::display,
            );
            let size = row
                .size
                .map_or_else(|| "—".to_owned(), crate::model::broker::ticket::display);
            ui.add(
                egui::Label::new(kit::text(
                    format!("{size} @ {price}"),
                    kit::size::CAPTION + 1.0,
                    kit::REGULAR,
                    theme::text_primary(),
                ))
                .truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(row.can_cancel, |ui| {
                    if icons::button(
                        ui,
                        Icon::Xmark,
                        20.0,
                        theme::text_secondary(),
                        "Cancel this order",
                    )
                    .clicked()
                    {
                        state.cancels.push(backend.cancel(&row.id));
                    }
                });
                kit::status_badge(ui, row.status.label, tone(row.status.tone));
            });
        });
        ui.add_space(6.0);
    }
    if rows.len() > SHOWN {
        ui.label(kit::text(
            format!("{} more in the orders panel", rows.len() - SHOWN),
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    }
}

fn tone(tone: crate::trading::model::Tone) -> kit::Tone {
    use crate::trading::model::Tone;
    match tone {
        Tone::Good => kit::Tone::Good,
        Tone::Warning => kit::Tone::Warning,
        Tone::Bad => kit::Tone::Bad,
        Tone::Neutral => kit::Tone::Neutral,
    }
}
