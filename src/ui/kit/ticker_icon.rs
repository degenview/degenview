//! Market artwork: coin, company and chart logos from the shared icon cache, clipped to a circle,
//! the monogram that stands in for one, and the data source's corner badge.

use super::{BOLD, galley, source_logo, window};
use crate::model::DataSource;
use eframe::egui::{self, Color32, CornerRadius, Rect, Ui, Vec2};

/// A coin, company or market logo in a circle; a hue-seeded monogram until (or unless) the
/// shared icon cache resolves one. Crypto shares one lookup per base asset.
pub fn ticker_icon(ui: &Ui, rect: Rect, symbol: &str, source: DataSource) {
    if source == DataSource::CoinMarketCap {
        // Its charts are indices, not assets: the CoinMarketCap mark stands in for a coin logo.
        source_logo::paint(ui, source, rect);
        return;
    }
    let base = source.base_asset(symbol);
    let lookup = match source {
        DataSource::Binance | DataSource::Coinbase | DataSource::CoinGecko => {
            Some((DataSource::Binance, format!("{base}USDT")))
        }
        DataSource::Alpaca => Some((DataSource::Alpaca, base.clone())),
        _ => None,
    };
    if let Some((source, market)) = &lookup {
        if let Some(url) = crate::service::image::icon::get_url(*source, market) {
            egui::Image::new(url)
                .fit_to_exact_size(rect.size())
                .corner_radius(CornerRadius::same((rect.width() / 2.0) as u8))
                .paint_at(ui, rect);
            return;
        }
        crate::service::image::icon::request(*source, market);
    }
    monogram(ui, rect, &base);
}

/// `ChartIconView`: the artwork for a chart's own market, resolved per (source, symbol) as the
/// chart does, clipped to a circle, with a monogram while it loads or when none exists and, when
/// `shows_source`, the data source's logo on the bottom-right corner. CoinMarketCap's charts are
/// indices, not assets: its mark stands in for the artwork and needs no second badge.
pub fn chart_icon(ui: &Ui, rect: Rect, symbol: &str, source: DataSource, shows_source: bool) {
    chart_icon_with_label(ui, rect, symbol, symbol, source, shows_source);
}

/// [`chart_icon`] with a separate human-readable fallback label. This keeps provider ids such as
/// DEX pair addresses as the lookup key while the unresolved monogram still uses the ticker.
pub fn chart_icon_with_label(
    ui: &Ui,
    rect: Rect,
    symbol: &str,
    label: &str,
    source: DataSource,
    shows_source: bool,
) {
    if source == DataSource::CoinMarketCap {
        source_logo::paint(ui, source, rect);
        return;
    }
    match crate::service::image::icon::get_url(source, symbol) {
        Some(url) => {
            egui::Image::new(url)
                .fit_to_exact_size(rect.size())
                .corner_radius(CornerRadius::same((rect.width() / 2.0) as u8))
                .paint_at(ui, rect);
        }
        None => {
            let base = source.base_asset(label);
            crate::service::image::icon::request_with_base(source, symbol, &base);
            monogram(ui, rect, &base);
        }
    }
    if shows_source {
        source_badge(ui, rect, source);
    }
}

/// `TickerIconView`'s fallback: two letters on a hue derived from the symbol.
pub fn monogram(ui: &Ui, rect: Rect, symbol: &str) {
    let seed = symbol
        .to_uppercase()
        .chars()
        .fold(0u32, |sum, c| sum.wrapping_add((c as u32).wrapping_mul(31)));
    let fill: Color32 = egui::ecolor::Hsva::new((seed % 360) as f32 / 360.0, 0.55, 0.7, 1.0).into();
    ui.painter()
        .circle_filled(rect.center(), rect.width() / 2.0, fill);
    let initials: String = symbol
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let label = galley(
        ui,
        &initials,
        rect.width() * 0.4,
        BOLD,
        Color32::WHITE,
        f32::INFINITY,
    );
    ui.painter()
        .galley(rect.center() - label.size() / 2.0, label, Color32::WHITE);
}

/// The source's logo badged half outside an icon's bottom-right corner, ringed in the window
/// colour so it reads against any artwork.
pub fn source_badge(ui: &Ui, icon: Rect, source: DataSource) {
    let side = (icon.width() * 0.55).round();
    let center = icon.right_bottom() - Vec2::splat(side * 0.5) + Vec2::splat(side * 0.3);
    let rect = Rect::from_center_size(center, Vec2::splat(side));
    ui.painter()
        .rect_filled(rect.expand(1.0), side * 0.3, window());
    source_logo::paint(ui, source, rect);
}
