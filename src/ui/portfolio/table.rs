//! The pieces every portfolio and paper-trading table shares: a sortable header cell, a
//! right-aligned figure, a tinted pill, a search box and the footer line. Painting only;
//! the sorting and filtering rules live next to each table so they can be tested.

// Transient: the portfolio and paper-trading screens use these once they are hosted.
#![allow(dead_code)]

use crate::model::DataSource;
use crate::model::portfolio::PortfolioAsset;
use crate::model::portfolio::format::Tone;
use crate::service;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, Color32, RichText, Sense, Vec2};

pub const ROW_HEIGHT: f32 = 34.0;
pub const HEADER_HEIGHT: f32 = 28.0;
/// Side margin of a table inside its container.
pub const INSET: f32 = 16.0;

pub fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Gain => theme::UP,
        Tone::Loss => theme::DOWN,
    }
}

/// `Some(color)` for a gain or loss; `None` leaves the text its normal colour.
pub fn tone_option(tone: Option<Tone>) -> Option<Color32> {
    tone.map(tone_color)
}

/// A header cell: the title, an arrow when the table is sorted by it, clickable when the
/// column can sort. Returns whether it was clicked.
pub fn header_cell(
    ui: &mut egui::Ui,
    title: &str,
    sorted: Option<bool>,
    sortable: bool,
    right_aligned: bool,
) -> bool {
    let arrow = match sorted {
        Some(true) => " ▲",
        Some(false) => " ▼",
        None => "",
    };
    let color = if sorted.is_some() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let text = RichText::new(format!("{title}{arrow}"))
        .small()
        .strong()
        .color(color);
    let layout = if right_aligned {
        egui::Layout::right_to_left(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    };
    ui.with_layout(layout, |ui| {
        if sortable {
            ui.add(egui::Button::new(text).frame(false)).clicked()
        } else {
            ui.label(text);
            false
        }
    })
    .inner
}

/// A right-aligned, fixed-width-digit figure for a table cell.
pub fn numeric_cell(ui: &mut egui::Ui, text: &str, color: Option<Color32>) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let mut rich = RichText::new(text);
        if let Some(color) = color {
            rich = rich.color(color);
        }
        ui.add(egui::Label::new(rich).truncate());
    });
}

/// The Swift ticker slot: a cached logo when available, otherwise a stable two-letter monogram.
pub fn asset_icon(ui: &mut egui::Ui, asset: &PortfolioAsset, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_asset_icon(ui, rect, asset);
}

/// `asset_icon` into a rect the caller has already placed. The artwork is looked up by the
/// asset's own market id (a CoinGecko coin id, a DEX pair address, a Binance pair) with its
/// ticker as the base symbol, like Swift's `iconURL(ticker: marketSymbol, baseSymbol:)`.
pub fn paint_asset_icon(ui: &egui::Ui, rect: egui::Rect, asset: &PortfolioAsset) {
    let size = rect.width();
    let ticker = asset.display_ticker();
    let looked_up =
        !asset.source.is_prediction_market() && asset.source != DataSource::CoinMarketCap;
    if looked_up {
        let market = service::portfolio::history::market_symbol(asset);
        if let Some(url) = service::image::icon::get_url(asset.source, &market) {
            egui::Image::new(url)
                .fit_to_exact_size(rect.size())
                .corner_radius(egui::CornerRadius::same((size / 2.0) as u8))
                .paint_at(ui, rect);
            // A hairline keeps a dark logo (XRP) visible against a dark card.
            ui.painter().circle_stroke(
                rect.center(),
                size / 2.0 - 0.5,
                egui::Stroke::new(1.0, theme::ink(34)),
            );
            return;
        }
        service::image::icon::request_with_base(asset.source, &market, &ticker);
    }
    let seed = ticker
        .to_uppercase()
        .chars()
        .fold(0u32, |sum, c| sum.wrapping_add((c as u32).wrapping_mul(31)));
    let fill: Color32 = egui::ecolor::Hsva::new((seed % 360) as f32 / 360.0, 0.55, 0.7, 1.0).into();
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    let initials: String = ticker
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials,
        egui::FontId::proportional(size * 0.42),
        Color32::WHITE,
    );
}

/// A small tinted pill, like a transaction type or a side.
pub fn pill(ui: &mut egui::Ui, text: &str, tint: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(tint.gamma_multiply(0.16))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().strong().color(tint))
        })
        .response
}

/// The search box above a table, with a Clear link while it holds text.
pub fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(246.0)
            .margin(egui::Margin {
                left: 29,
                right: 8,
                top: 6,
                bottom: 6,
            }),
    );
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(response.rect.left() + 15.0, response.rect.center().y),
            Vec2::splat(14.0),
        ),
        Icon::Magnifier,
        theme::text_secondary(),
    );
}

/// "3 assets" / "2 of 3 assets" on the left, a note on the right.
pub fn footer(ui: &mut egui::Ui, shown: usize, total: usize, noun: (&str, &str), note: &str) {
    ui.horizontal(|ui| {
        ui.add_space(INSET);
        ui.label(RichText::new(count_text(shown, total, noun)).small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(INSET);
            ui.label(RichText::new(note).small().weak());
        });
    });
    ui.add_space(6.0);
}

/// `noun` is `(singular, plural)`.
pub fn count_text(shown: usize, total: usize, noun: (&str, &str)) -> String {
    let word = |n: usize| if n == 1 { noun.0 } else { noun.1 };
    if shown == total {
        format!("{total} {}", word(total))
    } else {
        format!("{shown} of {total} {}", word(total))
    }
}

/// A centred empty state inside the table area.
pub fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(50.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(4.0);
        ui.label(RichText::new(detail).weak());
    });
}

/// The flex space the table has under a toolbar and above a footer.
pub fn table_height(ui: &egui::Ui) -> f32 {
    (ui.available_height() - 36.0).max(80.0)
}

/// A hairline-bordered frame around a table.
pub fn table_frame(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(INSET);
        let width = ui.available_width() - INSET;
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, theme::border()))
            .corner_radius(8.0)
            .show(ui, |ui| {
                ui.set_width(width - 2.0);
                content(ui);
            });
    });
}

pub fn row_sense() -> Sense {
    Sense::click()
}

pub fn cell_padding() -> Vec2 {
    Vec2::new(6.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_naturally() {
        let assets = ("asset", "assets");
        assert_eq!(count_text(1, 1, assets), "1 asset");
        assert_eq!(count_text(3, 3, assets), "3 assets");
        assert_eq!(count_text(0, 0, assets), "0 assets");
        assert_eq!(count_text(2, 5, assets), "2 of 5 assets");
        assert_eq!(
            count_text(1, 1, ("transaction", "transactions")),
            "1 transaction"
        );
    }

    #[test]
    fn tones_map_to_the_gain_and_loss_colours() {
        assert_eq!(tone_option(Some(Tone::Gain)), Some(theme::UP));
        assert_eq!(tone_option(Some(Tone::Loss)), Some(theme::DOWN));
        assert_eq!(tone_option(None), None);
    }
}
