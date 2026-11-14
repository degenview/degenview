//! The pieces the portfolio tables share: gain and loss colours, the asset icon, the count line
//! and the empty state. Painting only; the sorting and filtering rules live next to each table so
//! they can be tested.

use crate::model::DataSource;
use crate::model::portfolio::PortfolioAsset;
use crate::model::portfolio::format::Tone;
use crate::service;
use crate::ui::{kit, theme};
use eframe::egui::{self, Color32, RichText, Sense, Vec2};

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

/// The Swift ticker slot: a cached logo when available, otherwise a stable two-letter monogram.
pub fn asset_icon(ui: &mut egui::Ui, asset: &PortfolioAsset, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_asset_icon(ui, rect, asset);
}

/// `asset_icon` into a rect the caller has already placed. The artwork is looked up by the
/// asset's own market id (a CoinGecko coin id, a DEX pair address, a Binance pair) with its
/// ticker as the base symbol, like Swift's `iconURL(ticker: marketSymbol, baseSymbol:)`.
pub fn paint_asset_icon(ui: &egui::Ui, rect: egui::Rect, asset: &PortfolioAsset) {
    let ticker = asset.display_ticker();
    let looked_up =
        !asset.source.is_prediction_market() && asset.source != DataSource::CoinMarketCap;
    if looked_up {
        let market = service::portfolio::history::market_symbol(asset);
        if kit::cached_logo(
            ui,
            rect,
            asset.source,
            &market,
            Some(&ticker),
            Color32::WHITE,
        ) {
            // A hairline keeps a dark logo (XRP) visible against a dark card.
            ui.painter().circle_stroke(
                rect.center(),
                rect.width() / 2.0 - 0.5,
                egui::Stroke::new(1.0, theme::ink(34)),
            );
            return;
        }
    }
    kit::monogram_with(ui, rect, &ticker, 0.42, None);
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
