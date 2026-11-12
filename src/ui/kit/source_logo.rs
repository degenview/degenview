//! A data source's logo, the same artwork the Swift app ships in its asset catalog, bundled into
//! the binary so it draws without a network round trip.

use crate::model::DataSource;
use eframe::egui::{self, ImageSource, Rect, Ui, Vec2};

fn source(source: DataSource) -> ImageSource<'static> {
    match source {
        DataSource::Binance => egui::include_image!("../../../assets/source_logos/binance.svg"),
        DataSource::Coinbase => egui::include_image!("../../../assets/source_logos/coinbase.svg"),
        DataSource::CoinGecko => egui::include_image!("../../../assets/source_logos/coingecko.png"),
        DataSource::DexScreener => {
            egui::include_image!("../../../assets/source_logos/dexscreener.png")
        }
        DataSource::Alpaca => egui::include_image!("../../../assets/source_logos/alpaca.png"),
        DataSource::Polymarket => {
            egui::include_image!("../../../assets/source_logos/polymarket.png")
        }
        DataSource::Kalshi => egui::include_image!("../../../assets/source_logos/kalshi.png"),
        DataSource::CoinMarketCap => {
            egui::include_image!("../../../assets/source_logos/coinmarketcap.svg")
        }
    }
}

/// The logo as an image, rounded like the Swift `SourceLogoView` (22% of its side).
pub fn image(data_source: DataSource, size: f32) -> egui::Image<'static> {
    egui::Image::new(source(data_source))
        .fit_to_exact_size(Vec2::splat(size))
        .corner_radius(size * 0.22)
}

/// Paints the logo into `rect`.
pub fn paint(ui: &Ui, data_source: DataSource, rect: Rect) {
    image(data_source, rect.width().min(rect.height())).paint_at(ui, rect);
}

/// Allocates a `size` square and draws the logo in it.
pub fn show(ui: &mut Ui, data_source: DataSource, size: f32) -> egui::Response {
    ui.add(image(data_source, size))
        .on_hover_text(data_source.display_name())
}
