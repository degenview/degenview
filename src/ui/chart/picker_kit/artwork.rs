//! Market artwork: a coin, company or prediction-market picture in a circle.

use crate::model::DataSource;
use crate::service::SearchResult;
use crate::ui::kit::{self, BOLD};
use eframe::egui::{self, Color32, CornerRadius, Rect, Ui};

/// A coin, company or market logo in a circle; a hue-seeded monogram until (or unless) the
/// shared icon cache resolves one.
pub(in crate::ui) fn ticker_icon(
    ui: &Ui,
    rect: Rect,
    symbol: &str,
    source: DataSource,
    market: &str,
) {
    let lookup = supports_ticker_icon_lookup(source);
    if lookup && kit::cached_logo(ui, rect, source, market, Some(symbol), Color32::WHITE) {
        return;
    }
    monogram(ui, rect, symbol);
}

fn supports_ticker_icon_lookup(source: DataSource) -> bool {
    matches!(
        source,
        DataSource::Binance
            | DataSource::Coinbase
            | DataSource::CoinGecko
            | DataSource::DexScreener
            | DataSource::Alpaca
    )
}

/// A hue-seeded circle with the symbol's first two letters, a touch larger than the kit's.
fn monogram(ui: &Ui, rect: Rect, symbol: &str) {
    kit::monogram_with(ui, rect, symbol, 0.42, Some(BOLD));
}

/// A prediction market's own artwork in a circle, a monogram until (or unless) it loads.
pub(in crate::ui) fn prediction_icon(ui: &Ui, rect: Rect, symbol: &str, url: Option<&str>) {
    if let Some(url) = url {
        let image = egui::Image::new(url.to_owned())
            .fit_to_exact_size(rect.size())
            .corner_radius(CornerRadius::same((rect.width() / 2.0) as u8))
            .show_loading_spinner(false);
        if image
            .load_for_size(ui.ctx(), rect.size())
            .is_ok_and(|poll| poll.is_ready())
        {
            image.paint_at(ui, rect);
            return;
        }
    }
    monogram(ui, rect, symbol);
}

/// A result's artwork. Crypto pairs share one lookup per base asset, keyed as its Binance USDT
/// market, so ETH/BTC and ETH/EUR draw the same logo as ETH/USDT without a request each.
pub(crate) fn result_icon(ui: &Ui, rect: Rect, result: &SearchResult) {
    if result.source.is_prediction_market() {
        let artwork = result
            .event
            .as_ref()
            .and_then(|event| event.image_url.as_deref());
        prediction_icon(ui, rect, &result.symbol, artwork);
        return;
    }
    let base = base_symbol(result);
    match result.source {
        DataSource::Binance | DataSource::Coinbase | DataSource::CoinGecko => {
            ticker_icon(ui, rect, &base, DataSource::Binance, &format!("{base}USDT"));
        }
        source => ticker_icon(ui, rect, &base, source, &result.full_symbol),
    }
}

/// The ticker without a trading pair or company name: "BTC/USDT" → "BTC".
fn base_symbol(result: &SearchResult) -> String {
    result.base_asset()
}
