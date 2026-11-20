//! The trades tape: the latest trades, newest first, a big one set heavier and tinted by who took.

use super::BookViewState;
use super::ladder::taker_color;
use super::size_text;
use crate::model::depth::{Ladder, Trade};
use crate::model::taker_flow::Taker;
use crate::ui::accessibility;
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::market_flow::{MARGIN, usd};
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Sense, Ui, pos2, vec2};

const ROW_HEIGHT: f32 = 22.0;
const HEADER_HEIGHT: f32 = 22.0;
const FONT: f32 = 12.0;
const TIME_WIDTH: f32 = 62.0;
const SIZE_WIDTH: f32 = 64.0;

pub(super) fn show(ui: &mut Ui, state: &BookViewState, ladder: &Ladder, height: f32) {
    let width = ui.available_width();
    let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut tape = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    if ladder.trades.is_empty() {
        kit::empty_state(
            &tape,
            area,
            crate::ui::icons::Icon::List,
            "Waiting for trades",
            "They are listed here as they happen.",
        );
        return;
    }
    header(&mut tape);
    egui::ScrollArea::vertical()
        .id_salt("book-tape")
        .auto_shrink([false, false])
        .show_rows(&mut tape, ROW_HEIGHT, ladder.trades.len(), |ui, range| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for index in range {
                row(ui, ladder, &ladder.trades[index], state);
            }
        });
}

fn header(ui: &mut Ui) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
    let painter = ui.painter();
    let color = theme::text_tertiary();
    let y = rect.center().y;
    let put = |text: &str, x: f32, align: Align2| {
        painter.text(
            pos2(x, y),
            align,
            text,
            egui::FontId::proportional(size::CAPTION),
            color,
        );
    };
    put("Time", rect.left() + MARGIN, Align2::LEFT_CENTER);
    put(
        "Side",
        rect.left() + MARGIN + TIME_WIDTH,
        Align2::LEFT_CENTER,
    );
    put(
        "Price",
        rect.right() - MARGIN - SIZE_WIDTH - 8.0,
        Align2::RIGHT_CENTER,
    );
    put("Size", rect.right() - MARGIN, Align2::RIGHT_CENTER);
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, theme::separator()),
    );
}

fn row(ui: &mut Ui, ladder: &Ladder, trade: &Trade, state: &BookViewState) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let notional = trade.notional();
    let big = notional >= state.big.threshold();
    let taker = taker_color(trade.taker);
    if big {
        ui.painter()
            .rect_filled(rect, 0.0, theme::with_alpha(taker, 0.12));
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, kit::primary(0.06));
    }
    let weight = if big { SEMIBOLD } else { REGULAR };
    let y = rect.center().y;
    let put = |content: &str, x: f32, align: Align2, color: Color32, weight: f32| {
        let galley = kit::galley(ui, content, FONT, weight, color, f32::INFINITY);
        let left = match align {
            Align2::RIGHT_CENTER => x - galley.size().x,
            _ => x,
        };
        ui.painter().galley(
            pos2(left, y - galley.size().y / 2.0),
            galley,
            Color32::PLACEHOLDER,
        );
    };
    let time = kit::time_text::clock_time_seconds(Some(trade.time_ms));
    let side = match trade.taker {
        Taker::Buy => "Buy",
        Taker::Sell => "Sell",
    };
    let price = trade.price.format(ladder.tick.decimals());
    put(
        &time,
        rect.left() + MARGIN,
        Align2::LEFT_CENTER,
        theme::text_tertiary(),
        REGULAR,
    );
    put(
        side,
        rect.left() + MARGIN + TIME_WIDTH,
        Align2::LEFT_CENTER,
        taker,
        MEDIUM,
    );
    put(
        &price,
        rect.right() - MARGIN - SIZE_WIDTH - 8.0,
        Align2::RIGHT_CENTER,
        theme::text_primary(),
        weight,
    );
    put(
        &size_text(trade.size),
        rect.right() - MARGIN,
        Align2::RIGHT_CENTER,
        theme::text_primary(),
        weight,
    );
    let description = format!(
        "{time}: {} {} at {price}, {}",
        side.to_lowercase(),
        size_text(trade.size),
        usd(notional)
    );
    accessibility::label_as(response, egui::WidgetType::Label, description.clone())
        .on_hover_text(description);
}
