//! The panel beside the symbol list: the highlighted market's name, its price and change, whether
//! it is trading, the exchange it is listed on, and the other dollar markets of the same asset.

use super::list::{related_row, subtitle};
use super::search::{market_kind, trading_kind};
use crate::format;
use crate::model::DataSource;
use crate::model::watchlist::quote::{Freshness, WatchlistQuote};
use crate::service::SearchResult;
use crate::ui::chart::add::sheet::result_icon;
use crate::ui::kit::{self, BOLD, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::{kit::source_logo, theme};
use eframe::egui::{
    self, Align, Color32, Layout, Pos2, Rect, Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};

const PAD: f32 = 24.0;
const ICON: f32 = 64.0;

/// What the panel shows.
pub struct Detail<'a> {
    pub result: &'a SearchResult,
    pub quote: Option<&'a WatchlistQuote>,
    /// Other markets of the asset; `None` while they are being looked up.
    pub related: Option<&'a [SearchResult]>,
}

/// Draws the panel into `area` and returns the related market clicked, if any.
pub fn show(ui: &mut Ui, area: Rect, detail: &Detail) -> Option<SearchResult> {
    let mut clicked = None;
    let inner = area.shrink2(vec2(PAD, PAD));
    let result = detail.result;

    let icon = Rect::from_min_size(inner.min, Vec2::splat(ICON));
    result_icon(ui, icon, result);
    let text_left = icon.right() + 16.0;
    let width = inner.right() - text_left;
    let title = kit::galley(
        ui,
        result.ticker(),
        24.0,
        BOLD,
        theme::text_primary(),
        width,
    );
    let sub = subtitle(result);
    let sub = kit::galley(ui, &sub, 14.0, REGULAR, theme::text_secondary(), width);
    let block = title.size().y + 4.0 + sub.size().y;
    let top = icon.center().y - block / 2.0;
    let sub_top = top + title.size().y + 4.0;
    ui.painter()
        .galley(pos2(text_left, top), title, theme::text_primary());
    ui.painter()
        .galley(pos2(text_left, sub_top), sub, theme::text_secondary());

    // Price and change.
    let mut y = icon.bottom() + 22.0;
    y += price_line(
        ui,
        pos2(inner.left(), y),
        inner.width(),
        result.source,
        detail.quote,
    ) + 12.0;

    // Whether it trades now, and where.
    y += status_line(ui, pos2(inner.left(), y), result, detail.quote) + 6.0;
    y += venue_line(ui, pos2(inner.left(), y), result) + 22.0;

    ui.painter()
        .hline(area.x_range(), y, Stroke::new(1.0, theme::separator()));
    y += 22.0;

    // Other markets of the same asset.
    match detail.related {
        Some([]) | None => {}
        Some(related) => {
            let heading = kit::galley(
                ui,
                "ALSO TRADES AS",
                11.5,
                SEMIBOLD,
                theme::text_tertiary(),
                inner.width(),
            );
            ui.painter().galley(
                pos2(inner.left(), y),
                heading.clone(),
                theme::text_tertiary(),
            );
            y += heading.size().y + 8.0;
            // The rows scroll inside what is left of the panel instead of running over its edge.
            let room = Rect::from_min_max(pos2(inner.left() - 8.0, y), area.max);
            let mut list = ui.new_child(
                UiBuilder::new()
                    .max_rect(room)
                    .layout(Layout::top_down(Align::Min)),
            );
            list.set_clip_rect(room.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt("symbol_picker_related")
                .auto_shrink([false, false])
                .show(&mut list, |list| {
                    list.spacing_mut().item_spacing = Vec2::ZERO;
                    for hit in related {
                        if related_row(list, hit, inner.width() + 16.0) {
                            clicked = Some(hit.clone());
                        }
                    }
                });
        }
    }
    clicked
}

/// The last price and its change. Returns the height used.
fn price_line(
    ui: &Ui,
    at: Pos2,
    width: f32,
    source: DataSource,
    quote: Option<&WatchlistQuote>,
) -> f32 {
    let Some(last) = quote.and_then(|quote| quote.last) else {
        let text = match quote {
            Some(quote) if quote.note.is_some() => quote.note.as_deref().unwrap_or_default(),
            Some(quote) if quote.freshness == Freshness::Unsupported => "No quotes for this market",
            _ => "Loading price…",
        };
        let galley = kit::galley(ui, text, 15.0, MEDIUM, theme::text_secondary(), width);
        let height = galley.size().y;
        ui.painter().galley(at, galley, theme::text_secondary());
        return height.max(40.0);
    };
    let price = kit::digits(
        ui,
        &amount(source, last),
        34.0,
        SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let price_width = price.size().x;
    let height = price.size().y;
    ui.painter().galley(at, price, theme::text_primary());
    if let Some(quote) = quote
        && let Some(percent) = quote.change_percent
    {
        let color = change_color(percent);
        let text = match quote.change {
            Some(change) => format!(
                "{} ({})",
                signed(change, last, source),
                signed_percent(percent)
            ),
            None => signed_percent(percent),
        };
        let galley = kit::digits(
            ui,
            &text,
            17.0,
            MEDIUM,
            color,
            (width - price_width - 12.0).max(40.0),
        );
        let baseline = at.y + height - galley.size().y - 6.0;
        ui.painter()
            .galley(pos2(at.x + price_width + 12.0, baseline), galley, color);
    }
    height
}

/// A dot and a line: crypto trades around the clock; a share says whether its market is open.
fn status_line(ui: &Ui, at: Pos2, result: &SearchResult, quote: Option<&WatchlistQuote>) -> f32 {
    let (text, open) = match result.source {
        DataSource::Alpaca => match quote.map(|quote| quote.freshness) {
            Some(Freshness::Live) => ("Market open", true),
            Some(Freshness::MarketClosed) => ("Market closed", false),
            _ => return 0.0,
        },
        _ => ("Open 24/7", true),
    };
    let galley = kit::galley(
        ui,
        text,
        14.0,
        REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let dot = if open {
        theme::UP
    } else {
        theme::text_tertiary()
    };
    ui.painter()
        .circle_filled(pos2(at.x + 6.0, at.y + galley.size().y / 2.0), 4.5, dot);
    let height = galley.size().y;
    ui.painter()
        .galley(pos2(at.x + 20.0, at.y), galley, theme::text_secondary());
    height
}

/// The exchange the market is listed on, with its logo and what it trades there.
fn venue_line(ui: &Ui, at: Pos2, result: &SearchResult) -> f32 {
    let kind = match trading_kind(result.source) {
        "" => market_kind(result.source),
        kind => kind,
    };
    let text = format!("{} · {kind}", result.source.display_name());
    let galley = kit::galley(
        ui,
        &text,
        14.0,
        REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let logo = Rect::from_min_size(
        pos2(at.x, at.y + galley.size().y / 2.0 - 8.0),
        Vec2::splat(16.0),
    );
    source_logo::paint(ui, result.source, logo);
    let height = galley.size().y;
    ui.painter()
        .galley(pos2(at.x + 24.0, at.y), galley, theme::text_secondary());
    height
}

fn change_color(change: f64) -> Color32 {
    if change > 0.0 {
        theme::UP
    } else if change < 0.0 {
        theme::DOWN
    } else {
        theme::text_secondary()
    }
}

/// Whether a price is quoted to the cent: a share, or anything from a thousand up.
fn in_cents(source: DataSource, price: f64) -> bool {
    price.abs() >= 1_000.0 || (source == DataSource::Alpaca && price.abs() >= 1.0)
}

/// A price as the app writes one, but to the cent for a share (`228.40`, not `228.4000`).
fn amount(source: DataSource, price: f64) -> String {
    if in_cents(source, price) && price.abs() < 1_000.0 {
        format!("{price:.2}")
    } else {
        format::price(price)
    }
}

/// A change with its sign, to the precision of the price it moved: `+208.12` on 76,075.70.
fn signed(change: f64, last: f64, source: DataSource) -> String {
    let sign = if change > 0.0 { "+" } else { "" };
    let text = if in_cents(source, last) && change.abs() < 1_000.0 {
        format!("{change:.2}")
    } else {
        format::price(change)
    };
    format!("{sign}{text}")
}

fn signed_percent(percent: f64) -> String {
    let sign = if percent > 0.0 { "+" } else { "" };
    format!("{sign}{percent:.2}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_carry_their_sign() {
        let coin = DataSource::Binance;
        assert_eq!(signed(208.124, 76_075.7, coin), "+208.12");
        assert_eq!(signed(-12.5, 1.0, coin), "-12.5000");
        assert_eq!(signed(1_234.5, 76_075.7, coin), "+1,234.50");
        assert_eq!(signed(1.15, 228.4, DataSource::Alpaca), "+1.15");
        assert_eq!(amount(DataSource::Alpaca, 228.4), "228.40");
        assert_eq!(amount(coin, 0.5), "0.500000");
        assert_eq!(signed_percent(0.2749), "+0.27%");
        assert_eq!(signed_percent(-3.0), "-3.00%");
        assert_eq!(signed_percent(0.0), "0.00%");
    }

    #[test]
    fn a_move_up_is_green_and_down_red() {
        assert_eq!(change_color(1.0), theme::UP);
        assert_eq!(change_color(-1.0), theme::DOWN);
        assert_ne!(change_color(0.0), theme::UP);
    }
}
