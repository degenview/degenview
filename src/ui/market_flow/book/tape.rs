//! The trades tape: the latest trades, newest first, a big one set heavier and tinted by who took.

use super::ladder::taker_color;
use super::{BookViewState, MARGIN, size_text, usd};
use crate::i18n::tr;
use crate::model::depth::{DepthSnapshot, Price, Trade};
use crate::model::taker_flow::Taker;
use crate::ui::accessibility;
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Sense, Ui, pos2, vec2};

const ROW_HEIGHT: f32 = 22.0;
const HEADER_HEIGHT: f32 = 22.0;
const FONT: f32 = 12.0;
const TIME_WIDTH: f32 = 62.0;
const SIZE_WIDTH: f32 = 64.0;

pub(super) fn show(ui: &mut Ui, state: &BookViewState, snapshot: &DepthSnapshot, height: f32) {
    let width = ui.available_width();
    let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut tape = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    if snapshot.tape.trades.is_empty() {
        kit::empty_state(
            &tape,
            area,
            crate::ui::icons::Icon::List,
            tr!("Waiting for trades"),
            tr!("They are listed here as they happen."),
        );
        return;
    }
    let columns = Columns::measure(&tape);
    header(&mut tape, &columns);
    egui::ScrollArea::vertical()
        .id_salt("book-tape")
        .auto_shrink([false, false])
        .show_rows(
            &mut tape,
            ROW_HEIGHT,
            snapshot.tape.trades.len(),
            |ui, range| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for index in range {
                    row(
                        ui,
                        snapshot.tick,
                        &snapshot.tape.trades[index],
                        state,
                        &columns,
                    );
                }
            },
        );
}

/// The room the tape's time and size columns keep: [`TIME_WIDTH`] and [`SIZE_WIDTH`], or what their
/// headings need when a language writes them longer.
struct Columns {
    time: f32,
    size: f32,
}

impl Columns {
    fn measure(ui: &Ui) -> Self {
        let head = |text: &str| {
            kit::galley(
                ui,
                text,
                size::CAPTION,
                REGULAR,
                Color32::WHITE,
                f32::INFINITY,
            )
            .size()
            .x
            .ceil()
        };
        Self {
            time: TIME_WIDTH.max(head(tr!("Time")) + 8.0),
            size: SIZE_WIDTH.max(head(tr!("Size"))),
        }
    }
}

/// Who took, as the tape writes it.
fn taker_name(taker: Taker) -> &'static str {
    match taker {
        Taker::Buy => tr!("Buy"),
        Taker::Sell => tr!("Sell"),
    }
}

fn header(ui: &mut Ui, columns: &Columns) {
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
    put(tr!("Time"), rect.left() + MARGIN, Align2::LEFT_CENTER);
    put(
        tr!("Side"),
        rect.left() + MARGIN + columns.time,
        Align2::LEFT_CENTER,
    );
    put(
        tr!("Price"),
        rect.right() - MARGIN - columns.size - 8.0,
        Align2::RIGHT_CENTER,
    );
    put(tr!("Size"), rect.right() - MARGIN, Align2::RIGHT_CENTER);
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, theme::separator()),
    );
}

fn row(ui: &mut Ui, tick: Price, trade: &Trade, state: &BookViewState, columns: &Columns) {
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
    let side = taker_name(trade.taker);
    let price = trade.price.format(tick.decimals());
    put(
        &time,
        rect.left() + MARGIN,
        Align2::LEFT_CENTER,
        theme::text_tertiary(),
        REGULAR,
    );
    put(
        side,
        rect.left() + MARGIN + columns.time,
        Align2::LEFT_CENTER,
        taker,
        MEDIUM,
    );
    put(
        &price,
        rect.right() - MARGIN - columns.size - 8.0,
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
    let (amount, worth) = (size_text(trade.size), usd(notional));
    let description = match trade.taker {
        Taker::Buy => tr!(
            "{time}: buy {amount} at {price}, {worth}",
            time = time,
            amount = amount,
            price = price,
            worth = worth
        ),
        Taker::Sell => tr!(
            "{time}: sell {amount} at {price}, {worth}",
            time = time,
            amount = amount,
            price = price,
            worth = worth
        ),
    };
    accessibility::label_as(response, egui::WidgetType::Label, description.clone())
        .on_hover_text(description);
}
