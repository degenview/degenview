//! Quick-fill chips and the starting markets offered before anything is typed.

use super::artwork::ticker_icon;
use super::widgets::{caption_label, card};
use crate::model::DataSource;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{MEDIUM, galley_words, size};
use crate::ui::theme;
use eframe::egui::{self, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

pub(crate) const CRYPTO_SUGGESTIONS: [&str; 10] = [
    "BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK",
];

pub(crate) const STOCK_SUGGESTIONS: [&str; 10] = [
    "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "SPY", "QQQ", "AMD",
];

/// Starting points for the Predictions section; each searches the selected provider for its query.
pub(crate) const PREDICTION_TOPICS: [(&str, &str, Icon); 10] = [
    ("Fed decision", "fed", Icon::BuildingColumns),
    ("Bitcoin price", "bitcoin", Icon::BitcoinCircle),
    ("Ethereum price", "ethereum", Icon::Diamond),
    ("Inflation", "inflation", Icon::ChartUptrend),
    ("Recession", "recession", Icon::ArrowDownRightCircle),
    ("Election", "election", Icon::CheckmarkSeal),
    ("Trump", "trump", Icon::PersonCircle),
    ("Oil & gold", "oil", Icon::Flame),
    ("Super Bowl", "super bowl", Icon::SportsCourt),
    ("Weather", "temperature", Icon::CloudSun),
];

pub(crate) enum Chip {
    /// A coin or company logo, looked up for the chip's title as a ticker of the source.
    Logo(&'static str, DataSource),
    Symbol(&'static str, Icon),
}

/// Quick-fill chips use five columns at the Swift sheet width and three in a narrow window.
pub(crate) fn suggestion_grid(ui: &mut Ui, caption: &str, items: &[Chip]) -> Option<usize> {
    let mut clicked = None;
    card(ui, |ui| {
        ui.horizontal(|ui| caption_label(ui, Icon::Sparkles, caption));
        ui.add_space(10.0);
        let spacing = 8.0;
        let columns = if ui.available_width() < 650.0 { 3 } else { 5 };
        let width = (ui.available_width() - spacing * (columns - 1) as f32) / columns as f32;
        for (row, chunk) in items.chunks(columns).enumerate() {
            if row > 0 {
                ui.add_space(spacing);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spacing;
                for (offset, item) in chunk.iter().enumerate() {
                    if suggestion_chip(ui, item, width).clicked() {
                        clicked = Some(row * columns + offset);
                    }
                }
            });
        }
    });
    clicked
}

fn suggestion_chip(ui: &mut Ui, item: &Chip, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let title = match item {
        Chip::Logo(title, _) | Chip::Symbol(title, _) => *title,
    };
    let response = crate::ui::accessibility::label(response, title);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        8.0,
        theme::ink(if response.hovered() { 23 } else { 13 }),
    );
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, theme::ink(20)),
        StrokeKind::Inside,
    );
    let label = galley_words(
        ui,
        title,
        size::SUBHEADLINE + 1.0,
        MEDIUM,
        theme::text_primary(),
        width - 20.0 - 25.0,
    );
    let content_width = 18.0 + 7.0 + label.size().x;
    let left = rect.center().x - content_width / 2.0;
    let icon_rect = Rect::from_min_size(pos2(left, rect.center().y - 9.0), Vec2::splat(18.0));
    match item {
        Chip::Logo(ticker, source) => {
            // The id the source knows the ticker by: Binance lists "BTC" as "BTCUSDT".
            let market = if *source == DataSource::Binance {
                format!("{ticker}USDT")
            } else {
                (*ticker).to_owned()
            };
            ticker_icon(ui, icon_rect, ticker, *source, &market);
        }
        Chip::Symbol(_, icon) => {
            icons::paint(painter, icon_rect.shrink(2.5), *icon, theme::text_primary())
        }
    }
    painter.galley(
        pos2(
            icon_rect.right() + 7.0,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        theme::text_primary(),
    );
    response
}
