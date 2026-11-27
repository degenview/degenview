//! The panel beside the symbol list: the highlighted market's name, its price and change, its
//! last day on a chart, whether it is trading, the exchange it is listed on, the other dollar
//! markets of the same asset and the button that uses it. A prediction market shows its chance
//! instead of a price, and an index or a portfolio view says what it charts.

pub mod action;
mod chart;
mod layout;
mod related;

use super::dashboards::{Tile, TilePick};
use super::describe;
use super::search::{market_kind, trading_kind};
use super::{Item, PickKind, predictions};
use crate::format;
use crate::i18n::{tr, tr_n};
use crate::model::market::asset_name::AssetNames;
use crate::model::session_stats::MarketHours;
use crate::model::watchlist::quote::{Freshness, WatchlistQuote};
use crate::model::{DataSource, InstrumentId};
use crate::service::EventStats;
use crate::service::SearchResult;
use crate::ui::chart::picker_kit::artwork::{prediction_icon, result_icon};
use crate::ui::icons;
use crate::ui::kit::time_text::date_text;
use crate::ui::kit::{self, BOLD, MEDIUM, REGULAR, SEMIBOLD, galley_rows};
use crate::ui::{kit::source_logo, theme};
use action::Actions;
pub use chart::PreviewChart;
use eframe::egui::{Color32, Pos2, Rect, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};
use layout::PAD;

/// The artwork of a prediction market, and of a crypto or stock market.
const ICON: f32 = 40.0;
const MARKET_ICON: f32 = 30.0;
/// Between where a market is listed and whether it trades.
const STATUS_GAP: f32 = 14.0;
/// The room a logo or the trading dot takes before its words.
const LOGO_ROOM: f32 = 20.0;
const DOT_ROOM: f32 = 14.0;
/// The chart of a prediction market, under its chance.
const PREDICTION_CHART: f32 = 64.0;
/// The height one figure of an event takes.
const STAT_LINE: f32 = 25.0;

/// What the panel shows.
pub struct Detail<'a> {
    pub item: &'a Item,
    /// What the shown market's assets are called.
    pub names: AssetNames,
    pub quote: Option<&'a WatchlistQuote>,
    /// Other markets of the asset; `None` while they are being looked up.
    pub related: Option<&'a [SearchResult]>,
    /// The button at the foot; `None` for a row that cannot be used as it is.
    pub actions: Option<Actions>,
}

/// What the panel was asked for.
#[derive(Default)]
pub struct DetailOutput {
    /// Another market of the asset was clicked.
    pub related: Option<SearchResult>,
    /// The button or its menu was used.
    pub action: Option<PickKind>,
}

/// Draws the panel into `area`.
pub fn show(ui: &mut Ui, area: Rect, detail: &Detail, chart: &mut PreviewChart) -> DetailOutput {
    let mut out = DetailOutput::default();
    // The button keeps the foot of the panel; the rest is laid out above it.
    let above = match &detail.actions {
        Some(actions) => {
            let button = layout::button(area);
            out.action = action::show(ui, button, actions);
            Rect::from_min_max(area.min, pos2(area.right(), button.top()))
        }
        None => area,
    };
    let mut body = ui.new_child(UiBuilder::new().max_rect(above));
    body.set_clip_rect(above.intersect(ui.clip_rect()));
    match detail.item {
        Item::Market(result) if !result.source.is_prediction_market() => {
            out.related = market(&mut body, area, result, detail, chart);
        }
        Item::Market(result) | Item::Choice(result) => {
            prediction(&mut body, above, result, None, detail, chart);
        }
        Item::Event(head) => {
            prediction(&mut body, above, &head.lead, Some(head), detail, chart);
        }
        Item::Tile(tile) => tile_panel(&mut body, above, tile),
    }
    out
}

fn instrument(result: &SearchResult) -> InstrumentId {
    InstrumentId::new(result.source, &result.full_symbol)
}

/// A crypto or stock market: price, whether it trades, where, its last day and the same asset
/// elsewhere. `area` is the whole panel, whose height decides what fits.
fn market(
    ui: &mut Ui,
    area: Rect,
    result: &SearchResult,
    detail: &Detail,
    chart: &mut PreviewChart,
) -> Option<SearchResult> {
    let related = detail.related.filter(|related| !related.is_empty());
    let fit = layout::market(area, detail.actions.is_some(), related.is_some());

    let icon = Rect::from_center_size(
        pos2(fit.header.left() + MARKET_ICON / 2.0, fit.header.center().y),
        Vec2::splat(MARKET_ICON),
    );
    result_icon(ui, icon, result);
    super::list::two_lines_sized(
        ui,
        fit.header.with_min_x(icon.right() + 10.0),
        (result.ticker(), 16.0, BOLD),
        &describe::headline(result, &detail.names),
    );

    price_line(
        ui,
        fit.price.min,
        fit.price.width(),
        result.source,
        detail.quote,
    );

    listing_line(ui, fit.status, result, detail.quote);

    chart.show(ui, fit.chart, &instrument(result), detail.quote);

    related::show(ui, area, fit.related?, related?)
}

/// A prediction market: the event, the chance of the choice, where it is listed, how that chance
/// moved over the last day and the event's figures. An event with several choices shows its
/// likeliest.
fn prediction(
    ui: &mut Ui,
    area: Rect,
    result: &SearchResult,
    head: Option<&predictions::EventHead>,
    detail: &Detail,
    chart: &mut PreviewChart,
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
    let text_left = icon.right() + 12.0;
    let width = inner.right() - text_left;
    let title = galley_rows(ui, title, 16.0, BOLD, theme::text_primary(), width, 3);
    let height = title.size().y;
    let top = (icon.center().y - height / 2.0).max(inner.top());
    ui.painter()
        .galley(pos2(text_left, top), title, theme::text_primary());

    let mut y = icon.bottom().max(top + height) + 14.0;
    if let Some(chance) = result.price {
        let galley = kit::digits(
            ui,
            &format::headline(chance, true),
            30.0,
            SEMIBOLD,
            theme::text_primary(),
            inner.width(),
        );
        let height = galley.size().y;
        ui.painter()
            .galley(pos2(inner.left(), y), galley, theme::text_primary());
        y += height + 2.0;
        let caption = match head {
            Some(head) => tr_n!(
                head.choices,
                "chance of {symbol} · {count} choice",
                "chance of {symbol} · {count} choices",
                symbol = result.symbol
            ),
            None if result.symbol != result.event.as_ref().map_or("", |e| e.title.as_str()) => {
                tr!("chance of {symbol}", symbol = result.symbol)
            }
            None => tr!("chance").to_owned(),
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
        y += height + 10.0;
    }
    y += venue_line(ui, pos2(inner.left(), y), result, inner.width()).y + 14.0;

    let stats = result.event.as_ref().map(|event| &event.stats);
    // The chart comes before what the event says about itself, but never pushes a figure out.
    let figures = stats.map_or(0.0, |stats| {
        stat_rows(stats).len() as f32 * STAT_LINE + 14.0
    });
    if inner.bottom() - y >= PREDICTION_CHART + 14.0 + figures {
        chart.show(
            ui,
            Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), PREDICTION_CHART)),
            &instrument(result),
            detail.quote,
        );
        y += PREDICTION_CHART + 14.0;
    }

    if let Some(stats) = stats.filter(|stats| !stats.is_empty()) {
        ui.painter()
            .hline(area.x_range(), y, Stroke::new(1.0, theme::separator()));
        y += 14.0;
        for (label, value) in stat_rows(stats) {
            y += stat_row(ui, pos2(inner.left(), y), inner.width(), label, &value) + 8.0;
        }
        if let Some(about) = &stats.about {
            y += 8.0;
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
    money(tr!("Volume"), stats.volume);
    money(tr!("24h volume"), stats.volume_24h);
    money(tr!("Liquidity"), stats.liquidity);
    money(tr!("Open interest"), stats.open_interest);
    if let Some(ends) = stats.ends_at_ms {
        rows.push((tr!("Ends"), date_text(ends)));
    }
    if let Some(category) = &stats.category {
        rows.push((tr!("Category"), category.clone()));
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
        tr!("ABOUT"),
        11.5,
        SEMIBOLD,
        theme::text_tertiary(),
        inner.width(),
    );
    let heading_top = top;
    let top = top + heading.size().y + 8.0;
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
    let rows = (((area.bottom() - 8.0 - top) / line).floor() as usize).min(8);
    // No caption over nothing.
    if rows == 0 {
        return;
    }
    ui.painter().galley(
        pos2(inner.left(), heading_top),
        heading,
        theme::text_tertiary(),
    );
    let galley = layout(rows);
    ui.painter()
        .galley(pos2(inner.left(), top), galley, theme::text_secondary());
}

/// An index or a portfolio view: what it charts and where it comes from.
fn tile_panel(ui: &mut Ui, area: Rect, tile: &Tile) {
    let inner = area.shrink2(vec2(PAD, PAD));
    let icon = Rect::from_min_size(inner.min + vec2(0.0, 4.0), Vec2::splat(32.0));
    icons::paint(ui.painter(), icon, tile.icon, theme::text_secondary());
    let mut y = icon.bottom() + 16.0;
    let title = kit::galley(
        ui,
        &tile.title,
        20.0,
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
            venue_line(ui, pos2(inner.left(), y), result, inner.width());
        }
        TilePick::Card(_) => {
            let scope = tile.scope.as_deref().unwrap_or_default();
            let galley = kit::galley(
                ui,
                &tr!("Portfolio · {scope}", scope = scope),
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
            Some(quote) if quote.freshness == Freshness::Unsupported => {
                tr!("No quotes for this market")
            }
            _ => tr!("Loading price…"),
        };
        let galley = kit::galley(ui, text, 15.0, MEDIUM, theme::text_secondary(), width);
        let height = galley.size().y;
        ui.painter().galley(at, galley, theme::text_secondary());
        return height.max(40.0);
    };
    let price = kit::digits(
        ui,
        &amount(source, last),
        30.0,
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
            kit::digits(ui, text, 15.0, MEDIUM, color, f32::INFINITY)
                .size()
                .x
                <= room
        });
        let galley = kit::digits(ui, &text, 15.0, MEDIUM, color, room);
        let baseline = at.y + height - galley.size().y - 5.0;
        ui.painter()
            .galley(pos2(at.x + price_width + 12.0, baseline), galley, color);
    }
    height
}

/// Whether the market trades now: crypto around the clock, a share when its market is open.
/// `None` while a share's market has not said.
fn status(result: &SearchResult, quote: Option<&WatchlistQuote>) -> Option<(&'static str, bool)> {
    if MarketHours::for_source(result.source) == MarketHours::UsStock {
        return match quote.map(|quote| quote.freshness) {
            Some(Freshness::Live) => Some((tr!("Market open"), true)),
            Some(Freshness::MarketClosed) => Some((tr!("Market closed"), false)),
            _ => None,
        };
    }
    Some(match result.source {
        // Forex and the CFDs on it rest over the weekend.
        DataSource::Oanda => (tr!("Open 24/5"), true),
        _ => (tr!("Open 24/7"), true),
    })
}

/// Where the market is listed and, after it, whether it trades now, on one line across `line`. A
/// long exchange name gives way: whether it trades is never cut.
fn listing_line(ui: &Ui, line: Rect, result: &SearchResult, quote: Option<&WatchlistQuote>) {
    let trading = status(result, quote).map(|(text, open)| {
        let color = theme::text_secondary();
        (
            kit::galley(ui, text, 13.0, REGULAR, color, f32::INFINITY),
            open,
        )
    });
    let taken = trading
        .as_ref()
        .map_or(0.0, |(words, _)| STATUS_GAP + DOT_ROOM + words.size().x);
    let listed = venue_line(ui, line.min, result, line.width() - taken);
    let Some((words, open)) = trading else {
        return;
    };
    let left = line.left() + listed.x + STATUS_GAP;
    let dot = if open {
        theme::UP
    } else {
        theme::text_tertiary()
    };
    ui.painter().circle_filled(
        pos2(left + 4.0, line.top() + words.size().y / 2.0),
        4.0,
        dot,
    );
    ui.painter().galley(
        pos2(left + DOT_ROOM, line.top()),
        words,
        theme::text_secondary(),
    );
}

/// `Binance · Spot`.
fn venue_text(result: &SearchResult) -> String {
    let kind = match trading_kind(result) {
        "" => market_kind(result.source),
        kind => kind,
    };
    tr!(
        "{venue} · {kind}",
        venue = result.source.display_name(),
        kind = kind
    )
}

/// The exchange the market is listed on, with its logo and what it trades there, in at most
/// `width`. Returns the size used.
fn venue_line(ui: &Ui, at: Pos2, result: &SearchResult, width: f32) -> Vec2 {
    let galley = kit::galley(
        ui,
        &venue_text(result),
        13.0,
        REGULAR,
        theme::text_secondary(),
        width - LOGO_ROOM,
    );
    let logo = Rect::from_min_size(
        pos2(at.x - 1.0, at.y + galley.size().y / 2.0 - 7.0),
        Vec2::splat(14.0),
    );
    source_logo::paint(ui, result.source, logo);
    let used = vec2(LOGO_ROOM + galley.size().x, galley.size().y);
    ui.painter().galley(
        pos2(at.x + LOGO_ROOM, at.y),
        galley,
        theme::text_secondary(),
    );
    used
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
