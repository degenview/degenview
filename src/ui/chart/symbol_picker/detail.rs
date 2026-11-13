//! The panel beside the symbol list: the highlighted market's name, its price and change, whether
//! it is trading, the exchange it is listed on, and the other dollar markets of the same asset.
//! A prediction market shows its chance instead, and an index or a portfolio view says what it
//! charts.

use super::dashboards::{Tile, TilePick};
use super::list::{related_row, subtitle};
use super::search::{market_kind, trading_kind};
use super::{Item, predictions};
use crate::format;
use crate::model::DataSource;
use crate::model::watchlist::quote::{Freshness, WatchlistQuote};
use crate::service::EventStats;
use crate::service::SearchResult;
use crate::ui::chart::picker_kit::artwork::{prediction_icon, result_icon};
use crate::ui::chart::picker_kit::text::galley_rows;
use crate::ui::icons;
use crate::ui::kit::time_text::date_text;
use crate::ui::kit::{self, BOLD, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::{kit::source_logo, theme};
use eframe::egui::{
    self, Align, Color32, Layout, Pos2, Rect, Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};

const PAD: f32 = 24.0;
const ICON: f32 = 64.0;

/// What the panel shows.
pub struct Detail<'a> {
    pub item: &'a Item,
    pub quote: Option<&'a WatchlistQuote>,
    /// Other markets of the asset; `None` while they are being looked up.
    pub related: Option<&'a [SearchResult]>,
}

/// Draws the panel into `area` and returns the related market clicked, if any.
pub fn show(ui: &mut Ui, area: Rect, detail: &Detail) -> Option<SearchResult> {
    match detail.item {
        Item::Market(result) if !result.source.is_prediction_market() => {
            market(ui, area, result, detail)
        }
        Item::Market(result) | Item::Choice(result) => {
            prediction(ui, area, result, None);
            None
        }
        Item::Event(head) => {
            prediction(ui, area, &head.lead, Some(head));
            None
        }
        Item::Tile(tile) => {
            tile_panel(ui, area, tile);
            None
        }
    }
}

/// A crypto or stock market: price, whether it trades, where, and the same asset elsewhere.
fn market(ui: &mut Ui, area: Rect, result: &SearchResult, detail: &Detail) -> Option<SearchResult> {
    let mut clicked = None;
    let inner = area.shrink2(vec2(PAD, PAD));

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

/// A prediction market: the event, the chance of the choice, and where it is listed. An event with
/// several choices shows its likeliest.
fn prediction(
    ui: &mut Ui,
    area: Rect,
    result: &SearchResult,
    head: Option<&predictions::EventHead>,
) {
    let inner = area.shrink2(vec2(PAD, PAD));
    let icon = Rect::from_min_size(inner.min, Vec2::splat(ICON));
    let title = head.map_or_else(
        || {
            result
                .event
                .as_ref()
                .map(|event| event.title.as_str())
                .filter(|title| !title.is_empty())
                .unwrap_or(&result.symbol)
        },
        |head| head.title.as_str(),
    );
    let artwork = head.map_or_else(
        || {
            result
                .event
                .as_ref()
                .and_then(|event| event.image_url.as_deref())
        },
        |head| head.image_url.as_deref(),
    );
    prediction_icon(ui, icon, title, artwork);
    let text_left = icon.right() + 16.0;
    let width = inner.right() - text_left;
    let title = crate::ui::chart::picker_kit::text::galley_rows(
        ui,
        title,
        20.0,
        BOLD,
        theme::text_primary(),
        width,
        3,
    );
    let top = icon.center().y - title.size().y.min(ICON) / 2.0;
    ui.painter().galley(
        pos2(text_left, top.max(inner.top())),
        title,
        theme::text_primary(),
    );

    let mut y = icon.bottom() + 22.0;
    if let Some(chance) = result.price {
        let galley = kit::digits(
            ui,
            &format::headline(chance, true),
            34.0,
            SEMIBOLD,
            theme::text_primary(),
            inner.width(),
        );
        let height = galley.size().y;
        ui.painter()
            .galley(pos2(inner.left(), y), galley, theme::text_primary());
        y += height + 4.0;
        let caption = match head {
            Some(head) => format!("chance of {} · {} choices", result.symbol, head.choices),
            None if result.symbol != result.event.as_ref().map_or("", |e| e.title.as_str()) => {
                format!("chance of {}", result.symbol)
            }
            None => "chance".to_owned(),
        };
        let galley = kit::galley(
            ui,
            &caption,
            14.0,
            REGULAR,
            theme::text_secondary(),
            inner.width(),
        );
        let height = galley.size().y;
        ui.painter()
            .galley(pos2(inner.left(), y), galley, theme::text_secondary());
        y += height + 18.0;
    }
    y += venue_line(ui, pos2(inner.left(), y), result) + 22.0;

    let stats = result.event.as_ref().map(|event| &event.stats);
    if let Some(stats) = stats.filter(|stats| !stats.is_empty()) {
        ui.painter()
            .hline(area.x_range(), y, Stroke::new(1.0, theme::separator()));
        y += 20.0;
        for (label, value) in stat_rows(stats) {
            y += stat_row(ui, pos2(inner.left(), y), inner.width(), label, &value) + 10.0;
        }
        if let Some(about) = &stats.about {
            y += 12.0;
            about_text(ui, area, inner, y, about);
        }
    }
}

/// The figures a prediction event shows, in the order they read: the money traded, then when it
/// ends. A figure the provider did not give is left out.
fn stat_rows(stats: &EventStats) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    let mut money = |label, value: Option<u64>| {
        if let Some(value) = value {
            rows.push((label, compact_money(value)));
        }
    };
    money("Volume", stats.volume);
    money("24h volume", stats.volume_24h);
    money("Liquidity", stats.liquidity);
    money("Open interest", stats.open_interest);
    if let Some(ends) = stats.ends_at_ms {
        rows.push(("Ends", date_text(ends)));
    }
    if let Some(category) = &stats.category {
        rows.push(("Category", category.clone()));
    }
    rows
}

/// Whole dollars the short way: `$412`, `$12.3K`, `$31.6M`, `$1.2B`.
fn compact_money(dollars: u64) -> String {
    let value = dollars as f64;
    let (scaled, suffix) = match dollars {
        0..1_000 => return format!("${dollars}"),
        1_000..1_000_000 => (value / 1e3, "K"),
        1_000_000..1_000_000_000 => (value / 1e6, "M"),
        _ => (value / 1e9, "B"),
    };
    let text = format!("{scaled:.1}");
    format!("${}{suffix}", text.trim_end_matches(".0"))
}

/// A caption at the left and its figure at the right, `width` wide. Returns the height used.
fn stat_row(ui: &Ui, at: Pos2, width: f32, label: &str, value: &str) -> f32 {
    let label = kit::galley(
        ui,
        label,
        14.0,
        REGULAR,
        theme::text_secondary(),
        width * 0.5,
    );
    let value = kit::galley(ui, value, 14.0, MEDIUM, theme::text_primary(), width * 0.5);
    let height = label.size().y.max(value.size().y);
    ui.painter().galley(at, label, theme::text_secondary());
    ui.painter().galley(
        pos2(at.x + width - value.size().x, at.y),
        value,
        theme::text_primary(),
    );
    height
}

/// How the event resolves, as the provider words it: a caption and as many lines as the panel has
/// room for, cut with an ellipsis.
fn about_text(ui: &Ui, area: Rect, inner: Rect, top: f32, about: &str) {
    let heading = kit::galley(
        ui,
        "ABOUT",
        11.5,
        SEMIBOLD,
        theme::text_tertiary(),
        inner.width(),
    );
    let heading_height = heading.size().y;
    ui.painter()
        .galley(pos2(inner.left(), top), heading, theme::text_tertiary());
    let top = top + heading_height + 8.0;
    let text = about.split_whitespace().collect::<Vec<_>>().join(" ");
    let layout = |rows| {
        galley_rows(
            ui,
            &text,
            14.0,
            REGULAR,
            theme::text_secondary(),
            inner.width(),
            rows,
        )
    };
    let line = layout(1).size().y;
    let rows = (((area.bottom() - PAD - top) / line).floor() as usize).min(8);
    if rows == 0 {
        return;
    }
    let galley = layout(rows);
    ui.painter()
        .galley(pos2(inner.left(), top), galley, theme::text_secondary());
}

/// An index or a portfolio view: what it charts and where it comes from.
fn tile_panel(ui: &mut Ui, area: Rect, tile: &Tile) {
    let inner = area.shrink2(vec2(PAD, PAD));
    let icon = Rect::from_min_size(inner.min + vec2(0.0, 12.0), Vec2::splat(40.0));
    icons::paint(ui.painter(), icon, tile.icon, theme::text_secondary());
    let mut y = icon.bottom() + 20.0;
    let title = kit::galley(
        ui,
        &tile.title,
        24.0,
        BOLD,
        theme::text_primary(),
        inner.width(),
    );
    let height = title.size().y;
    ui.painter()
        .galley(pos2(inner.left(), y), title, theme::text_primary());
    y += height + 8.0;
    let summary = kit::paragraph(
        ui,
        &tile.summary,
        14.0,
        REGULAR,
        theme::text_secondary(),
        inner.width(),
    );
    let height = summary.size().y;
    ui.painter()
        .galley(pos2(inner.left(), y), summary, theme::text_secondary());
    y += height + 22.0;
    match &tile.pick {
        TilePick::Gauge(result) => {
            venue_line(ui, pos2(inner.left(), y), result);
        }
        TilePick::Card(_) => {
            let scope = tile.scope.as_deref().unwrap_or_default();
            let galley = kit::galley(
                ui,
                &format!("Portfolio · {scope}"),
                14.0,
                REGULAR,
                theme::text_secondary(),
                inner.width(),
            );
            ui.painter()
                .galley(pos2(inner.left(), y), galley, theme::text_secondary());
        }
    }
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
        let room = (width - price_width - 12.0).max(40.0);
        let moved = quote.change.map(|change| signed(change, last, source));
        let text = change_text(moved, &signed_percent(percent), |text| {
            kit::digits(ui, text, 17.0, MEDIUM, color, f32::INFINITY)
                .size()
                .x
                <= room
        });
        let galley = kit::digits(ui, &text, 17.0, MEDIUM, color, room);
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

/// A change with its sign and only the digits it needs: `+208.12` on 76,075.70 and `+0.00059`
/// on 0.154960, never the price's full eight decimals.
fn signed(change: f64, last: f64, source: DataSource) -> String {
    let sign = if change > 0.0 { "+" } else { "" };
    let text = if in_cents(source, last) && change.abs() < 1_000.0 {
        format!("{change:.2}")
    } else {
        trim_zeros(&format::price(change))
    };
    format!("{sign}{text}")
}

/// What is written beside the price: the amount and the percent, or the percent alone when
/// `fits` says both are too wide. The percent is what is read, so it is never the part cut.
fn change_text(amount: Option<String>, percent: &str, fits: impl Fn(&str) -> bool) -> String {
    amount
        .map(|amount| format!("{amount} ({percent})"))
        .filter(|both| fits(both))
        .unwrap_or_else(|| percent.to_owned())
}

/// `text` without the zeros that end its decimals, keeping two: `-12.50`, `0.00059`.
fn trim_zeros(text: &str) -> String {
    let Some((integer, decimals)) = text.split_once('.') else {
        return text.to_owned();
    };
    format!("{integer}.{:0<2}", decimals.trim_end_matches('0'))
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
        assert_eq!(signed(-12.5, 1.0, coin), "-12.50");
        assert_eq!(signed(0.00059, 0.15496, coin), "+0.00059");
        assert_eq!(signed(-0.00123456, 0.15496, coin), "-0.00123456");
        assert_eq!(signed(0.0, 0.15496, coin), "0.00");
        assert_eq!(signed(1_234.5, 76_075.7, coin), "+1,234.50");
        assert_eq!(signed(1.15, 228.4, DataSource::Alpaca), "+1.15");
        assert_eq!(amount(DataSource::Alpaca, 228.4), "228.40");
        assert_eq!(amount(coin, 0.5), "0.500000");
        assert_eq!(signed_percent(0.2749), "+0.27%");
        assert_eq!(signed_percent(-3.0), "-3.00%");
        assert_eq!(signed_percent(0.0), "0.00%");
    }

    #[test]
    fn the_percent_is_kept_when_the_amount_does_not_fit() {
        let amount = || Some("+0.00059".to_owned());
        assert_eq!(
            change_text(amount(), "+0.38%", |_| true),
            "+0.00059 (+0.38%)"
        );
        assert_eq!(
            change_text(amount(), "+0.38%", |text| text.len() <= 8),
            "+0.38%"
        );
        assert_eq!(change_text(None, "+0.38%", |_| true), "+0.38%");
    }

    #[test]
    fn money_is_written_the_short_way() {
        assert_eq!(compact_money(0), "$0");
        assert_eq!(compact_money(412), "$412");
        assert_eq!(compact_money(12_340), "$12.3K");
        assert_eq!(compact_money(2_000), "$2K");
        assert_eq!(compact_money(31_625_167), "$31.6M");
        assert_eq!(compact_money(1_240_000_000), "$1.2B");
    }

    #[test]
    fn an_event_shows_only_the_figures_the_provider_gave() {
        let stats = EventStats {
            volume: Some(31_625_167),
            liquidity: Some(1_834_558),
            ends_at_ms: Some(1_796_000_000_000),
            category: Some("Economics".into()),
            ..EventStats::default()
        };
        let rows = stat_rows(&stats);
        let labels: Vec<&str> = rows.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels, ["Volume", "Liquidity", "Ends", "Category"]);
        assert_eq!(rows[0].1, "$31.6M");
        assert!(stat_rows(&EventStats::default()).is_empty());
    }

    #[test]
    fn a_move_up_is_green_and_down_red() {
        assert_eq!(change_color(1.0), theme::UP);
        assert_eq!(change_color(-1.0), theme::DOWN);
        assert_ne!(change_color(0.0), theme::UP);
    }
}
