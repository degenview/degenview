//! The rows of the watchlist panel: column header, symbol rows, section headings and
//! the empty-section placeholder. Each is painted by hand so the left and right edges
//! line up exactly (see `watchlist_style`), and each returns the `Response` the
//! sidebar uses for clicks, drags and menus.

use crate::model::market::trend::Trend;
use crate::model::watchlist;
use crate::model::watchlist::layout::Flags;
use crate::model::watchlist::quote::{Direction, WatchlistQuote};
use crate::model::watchlist::{
    WatchlistColumn, WatchlistDisplaySettings, WatchlistInstrument, WatchlistSection,
    WatchlistSortKey,
};
use crate::model::{DataSource, InstrumentId};
use crate::ui::accessibility;
use crate::ui::kit::{self, size, sparkline};
use crate::ui::theme;
use crate::ui::watchlist::style::{
    self as style, CHART_GAP, CHART_WIDTH, COLUMN_GAP, LEADING_INSET, LOGO_GAP, ROW_HEIGHT_COMPACT,
    ROW_HEIGHT_WITH_SUBTITLE, TRAILING_INSET,
};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Pos2, Rect, Response, Sense, Stroke, Vec2, text::LayoutJob,
};

pub type QuoteLookup<'a> = &'a dyn Fn(&InstrumentId) -> Option<WatchlistQuote>;

/// Strength of the hairline between two symbol rows: there to part them, not to be seen.
const DIVIDER_ALPHA: u8 = 9;
/// Space above and below a row's sparkline.
const CHART_VERTICAL_INSET: f32 = 12.0;

/// Text clipped to one line with an ellipsis, so a long name gives way to the values.
fn fitted(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    painter.layout_job(job)
}

/// The asset whose artwork to look up, matching what a chart card uses.
pub fn icon_base_symbol(item: &WatchlistInstrument) -> String {
    let symbol = &item.instrument.symbol;
    match item.instrument.source {
        DataSource::Binance => {
            let upper = symbol.to_uppercase();
            ["USDT", "USDC", "BUSD", "FDUSD", "USD"]
                .iter()
                .find_map(|quote| upper.strip_suffix(quote).filter(|base| !base.is_empty()))
                .unwrap_or(&upper)
                .to_owned()
        }
        DataSource::Coinbase => symbol.split('-').next().unwrap_or(symbol).to_uppercase(),
        DataSource::CoinGecko | DataSource::DexScreener => item
            .label
            .split('/')
            .next()
            .unwrap_or(&item.label)
            .to_uppercase(),
        DataSource::Polymarket | DataSource::Kalshi => item
            .display_name
            .clone()
            .unwrap_or_else(|| item.name.clone()),
        DataSource::Alpaca => symbol.to_uppercase(),
        DataSource::CoinMarketCap => "CMC".to_owned(),
    }
}

/// The line under the name: the provider ("Binance"). Where the short label says
/// something the name does not (a stock's ticker, a prediction market's outcome) it
/// comes first: "AAPL · Alpaca (IEX)".
pub fn subtitle(item: &WatchlistInstrument) -> String {
    // A portfolio card's label is the portfolio it covers; there is no provider to name.
    if item.card.is_some() {
        return item.label.clone();
    }
    let source = item.instrument.source.display_name();
    // A prediction market's label is the provider's opaque token (rows saved before it was
    // dropped still carry it), so only the provider names the row.
    if item.instrument.source.is_prediction_market()
        || item.label.is_empty()
        || item.label.eq_ignore_ascii_case(&item.name)
    {
        source.to_owned()
    } else {
        format!("{} · {source}", item.label)
    }
}

fn direction_color(direction: Direction, flat: Color32) -> Color32 {
    match direction {
        Direction::Up => theme::UP,
        Direction::Down => theme::DOWN,
        Direction::Flat => flat,
    }
}

// MARK: Column header

/// Column titles. Clicking one sorts by it; returns the key clicked.
pub fn column_header(
    ui: &mut egui::Ui,
    display: &WatchlistDisplaySettings,
) -> Option<WatchlistSortKey> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
    let weak = ui.visuals().weak_text_color();
    let strong = ui.visuals().strong_text_color();
    let font = FontId::proportional(10.5);
    let mut clicked = None;

    let paint_title = |ui: &mut egui::Ui,
                       id: &str,
                       title: &str,
                       key: Option<WatchlistSortKey>,
                       anchor: Align2,
                       x: f32|
     -> Option<WatchlistSortKey> {
        let active = key.is_some_and(|key| display.sort.key == key);
        let color = if active { strong } else { weak };
        let arrow = if active {
            if display.sort.ascending {
                " ▲"
            } else {
                " ▼"
            }
        } else {
            ""
        };
        let galley = ui
            .painter()
            .layout_no_wrap(format!("{title}{arrow}"), font.clone(), color);
        let hit = match anchor {
            Align2::RIGHT_CENTER => Rect::from_min_size(
                Pos2::new(x - galley.size().x, rect.top()),
                Vec2::new(galley.size().x, rect.height()),
            ),
            _ => Rect::from_min_size(
                Pos2::new(x, rect.top()),
                Vec2::new(galley.size().x, rect.height()),
            ),
        };
        ui.painter().galley(
            Pos2::new(hit.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        let response = ui.interact(
            hit.expand2(Vec2::new(3.0, 0.0)),
            Id::new(("wl-col", id)),
            if key.is_some() {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if response.hovered() && key.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        (response.clicked()).then_some(key).flatten()
    };

    if let Some(key) = paint_title(
        ui,
        "symbol",
        "Symbol",
        Some(WatchlistSortKey::Symbol),
        Align2::LEFT_CENTER,
        rect.left() + LEADING_INSET,
    ) {
        clicked = Some(key);
    }
    let (edges, values_left) = column_edges(display, rect.right() - TRAILING_INSET);
    for (column, right) in edges {
        if let Some(key) = paint_title(
            ui,
            column.title(),
            column.title(),
            column.sort_key(),
            Align2::RIGHT_CENTER,
            right,
        ) {
            clicked = Some(key);
        }
    }
    if display.shows_chart {
        paint_title(ui, "chart", "1M", None, Align2::RIGHT_CENTER, values_left);
    }
    clicked
}

// MARK: Symbol row

pub struct RowStyle<'a> {
    pub display: &'a WatchlistDisplaySettings,
    pub flags: &'a Flags,
    pub quote: QuoteLookup<'a>,
    /// The market's month of closes; `None` while it loads.
    pub trend: Option<&'a Trend>,
    pub selected: bool,
    /// The row being dragged is drawn quieter.
    pub dragged: bool,
    /// A hairline at the top parts this row from the symbol row above. Never drawn while the
    /// row is lit.
    pub divider: bool,
}

pub fn row_height(display: &WatchlistDisplaySettings) -> f32 {
    if display.shows_description {
        ROW_HEIGHT_WITH_SUBTITLE
    } else {
        ROW_HEIGHT_COMPACT
    }
}

pub fn instrument_row(ui: &mut egui::Ui, item: &WatchlistInstrument, row: &RowStyle) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, row_height(row.display)),
        Sense::click_and_drag(),
    );
    let quote = (row.quote)(&item.instrument);
    let quote = quote.as_ref();
    let dimmed = quote.is_some_and(|quote| quote.last.is_some() && !quote.freshness.is_current());
    let alpha = if row.dragged {
        0.4
    } else if dimmed {
        0.6
    } else {
        1.0
    };
    let painter = ui.painter_at(rect);
    let hovered = response.hovered() && !row.dragged;

    if row.selected {
        painter.rect_filled(
            rect.shrink2(Vec2::new(2.0, 1.0)),
            5.0,
            theme::surface_selected(),
        );
    } else if hovered {
        painter.rect_filled(rect.shrink2(Vec2::new(2.0, 1.0)), 5.0, theme::ink(10));
    } else if row.divider {
        painter.hline(
            (rect.left() + LEADING_INSET)..=(rect.right() - TRAILING_INSET),
            rect.top() + 0.5,
            Stroke::new(1.0, theme::ink(DIVIDER_ALPHA)),
        );
    }
    if let Some(flag) = row.flags.get(&item.instrument.key()) {
        style::paint_flag(&painter, rect.left(), rect.center().y, *flag);
    }

    // Logo.
    let logo_size = style::logo_size(row.display);
    let logo = Rect::from_center_size(
        Pos2::new(
            rect.left() + LEADING_INSET + logo_size / 2.0,
            rect.center().y,
        ),
        Vec2::splat(logo_size),
    );
    paint_logo(ui, &painter, logo, item, alpha);

    // Value columns, from the right edge.
    let weak = ui.visuals().weak_text_color().gamma_multiply(alpha);
    let text_color = ui.visuals().text_color().gamma_multiply(alpha);
    // A portfolio card has no price, so it shows dashes, not the "no quote" warning.
    let quote = quote.filter(|_| item.card.is_none());
    let direction = watchlist::quote::direction(quote);
    let source = Some(item.instrument.source);
    let change_color =
        direction_color(direction, ui.visuals().weak_text_color()).gamma_multiply(alpha);
    // Two-line rows carry the change under the price, unless it has a column of its own.
    let change_under_price =
        row.display.shows_description && !row.display.columns.contains(&WatchlistColumn::Change);
    let (edges, values_left) = column_edges(row.display, rect.right() - TRAILING_INSET);
    let mut warning = None;
    for (column, right) in edges {
        let column_width = style::column_width(column);
        let (text, color) = match column {
            WatchlistColumn::Last => {
                if quote.is_some_and(|quote| quote.last.is_none() && quote.note.is_some()) {
                    warning = quote.and_then(|quote| quote.note.clone());
                    (
                        "⚠".to_owned(),
                        Color32::from_rgb(255, 159, 10).gamma_multiply(alpha),
                    )
                } else {
                    let price = watchlist::quote::last_text(quote, source);
                    if change_under_price {
                        let price = kit::digits(
                            ui,
                            &price,
                            size::BODY,
                            kit::MEDIUM,
                            text_color,
                            column_width,
                        );
                        let change = kit::digits(
                            ui,
                            &watchlist::quote::change_text(quote, source),
                            size::SUBHEADLINE,
                            kit::REGULAR,
                            change_color,
                            column_width,
                        );
                        let top = rect.center().y - (price.size().y + change.size().y) / 2.0;
                        let change_top = top + price.size().y;
                        painter.galley(Pos2::new(right - price.size().x, top), price, text_color);
                        painter.galley(
                            Pos2::new(right - change.size().x, change_top),
                            change,
                            change_color,
                        );
                        continue;
                    }
                    (price, text_color)
                }
            }
            WatchlistColumn::Change => (watchlist::quote::change_text(quote, source), change_color),
            WatchlistColumn::ChangePercent => {
                (watchlist::quote::percent_text(quote, source), change_color)
            }
            WatchlistColumn::Volume => (watchlist::quote::volume_text(quote), weak),
            WatchlistColumn::Exchange => (item.instrument.source.display_name().to_owned(), weak),
            WatchlistColumn::Status => (
                quote
                    .map_or(watchlist::quote::Freshness::Unavailable, |quote| {
                        quote.freshness
                    })
                    .label()
                    .to_owned(),
                weak,
            ),
        };
        let galley = kit::digits(ui, &text, size::BODY, kit::REGULAR, color, column_width);
        painter.galley(
            Pos2::new(
                right - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
    }

    // The month's sparkline sits left of the values; a portfolio card has none.
    let mut name_right = values_left;
    if let Some(chart) = chart_rect(row.display, rect, values_left) {
        if item.card.is_none() {
            sparkline::paint_trail(&painter.with_clip_rect(rect), chart, row.trend);
        }
        name_right = chart.left() - CHART_GAP;
    }

    // Name and subtitle fill what is left.
    let name_left = logo.right() + LOGO_GAP;
    let name_width = (name_right - name_left).max(24.0);
    let name = kit::galley(
        ui,
        &item.name,
        size::BODY + 1.0,
        kit::SEMIBOLD,
        text_color,
        name_width,
    );
    if row.display.shows_description {
        let sub = kit::galley(
            ui,
            &subtitle(item),
            size::SUBHEADLINE + 0.5,
            kit::REGULAR,
            weak,
            name_width,
        );
        let total = name.size().y + 1.0 + sub.size().y;
        let top = rect.center().y - total / 2.0;
        painter.galley(Pos2::new(name_left, top + name.size().y + 1.0), sub, weak);
        painter.galley(Pos2::new(name_left, top), name, text_color);
    } else {
        painter.galley(
            Pos2::new(name_left, rect.center().y - name.size().y / 2.0),
            name,
            text_color,
        );
    }

    let hover = match warning {
        Some(note) => note,
        None => {
            let mut text = format!(
                "{}\n{}:{}",
                item.name,
                item.instrument.source.watchlist_slug(),
                item.instrument.symbol
            );
            // Providers disagree on what a change is measured against, so say which.
            if let Some(basis) = quote.and_then(|quote| quote.change_basis) {
                text.push('\n');
                text.push_str(basis.label());
            }
            text
        }
    };
    let spoken = if item.card.is_some() {
        format!("{} {}", item.name, item.label)
    } else {
        format!(
            "{} {}:{}",
            item.name,
            item.instrument.source.watchlist_slug(),
            item.instrument.symbol
        )
    };
    accessibility::label(response, spoken).on_hover_text(hover)
}

/// Where each value column's text ends, right to left from `right`, and where the space left of
/// them ends. The column header and every row lay out from this one function.
fn column_edges(
    display: &WatchlistDisplaySettings,
    right: f32,
) -> (Vec<(WatchlistColumn, f32)>, f32) {
    let mut x = right;
    let edges = display
        .columns
        .iter()
        .rev()
        .map(|column| {
            let edge = (*column, x);
            x -= style::column_width(*column) + COLUMN_GAP;
            edge
        })
        .collect();
    (edges, x)
}

/// The month's sparkline in `row`, ending where the value columns leave off; `None` when the
/// list hides it.
fn chart_rect(display: &WatchlistDisplaySettings, row: Rect, values_left: f32) -> Option<Rect> {
    display.shows_chart.then(|| {
        let height = (row.height() - 2.0 * CHART_VERTICAL_INSET).max(8.0);
        Rect::from_min_max(
            Pos2::new(values_left - CHART_WIDTH, row.center().y - height / 2.0),
            Pos2::new(values_left, row.center().y + height / 2.0),
        )
    })
}

/// The coin artwork, or a monogram while it resolves (or when there is none).
fn paint_logo(
    ui: &egui::Ui,
    painter: &egui::Painter,
    logo: Rect,
    item: &WatchlistInstrument,
    alpha: f32,
) {
    if let Some(card) = &item.card {
        let painter = painter.clone();
        crate::ui::portfolio::cards::paint_badge(&painter, logo, card.kind);
        return;
    }
    let base = icon_base_symbol(item);
    let source = item.instrument.source;
    let fallback_url;
    let url = if let Some(url) = item.image_url.as_deref() {
        Some(url)
    } else {
        fallback_url = crate::service::image::icon::get_url(source, &item.instrument.symbol);
        fallback_url.as_deref()
    };
    if let Some(url) = url {
        egui::Image::new(url)
            .fit_to_exact_size(logo.size())
            .corner_radius(egui::CornerRadius::same((logo.width() / 2.0) as u8))
            .tint(Color32::WHITE.gamma_multiply(alpha))
            .paint_at(ui, logo);
        return;
    }
    crate::service::image::icon::request(source, &item.instrument.symbol);
    painter.circle_filled(
        logo.center(),
        logo.width() / 2.0,
        theme::pick(Color32::from_gray(70), Color32::from_gray(222)).gamma_multiply(alpha),
    );
    painter.text(
        logo.center(),
        Align2::CENTER_CENTER,
        base.chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default(),
        FontId::proportional(11.0),
        theme::pick(Color32::from_gray(210), Color32::from_gray(90)).gamma_multiply(alpha),
    );
}

// MARK: Section row

/// A section heading: a small-caps title, a hairline to the right, then the count and
/// a fold chevron. Sections are flat: the title starts on the same left edge as every
/// symbol and the chevron ends on the same right edge as every value.
pub fn section_row(
    ui: &mut egui::Ui,
    section: &WatchlistSection,
    count: usize,
    is_first: bool,
    dragged: bool,
) -> Response {
    let top_pad = if is_first { 6.0 } else { 16.0 };
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, top_pad + 14.0 + 4.0),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    let hovered = response.hovered() && !dragged;
    let title_color = if dragged {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    } else if hovered {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let y = rect.top() + top_pad + 7.0;

    let chevron_x = rect.right() - TRAILING_INSET - 5.0;
    let count_text = count.to_string();
    let count_galley = painter.layout_no_wrap(
        count_text,
        FontId::monospace(10.0),
        ui.visuals().weak_text_color().gamma_multiply(0.7),
    );
    let count_right = chevron_x - 12.0;
    let count_left = count_right - count_galley.size().x;
    painter.galley(
        Pos2::new(count_left, y - count_galley.size().y / 2.0),
        count_galley,
        Color32::PLACEHOLDER,
    );

    let title_left = rect.left() + LEADING_INSET;
    let title = fitted(
        &painter,
        &section.title.to_uppercase(),
        FontId::proportional(10.5),
        title_color,
        (count_left - title_left - 24.0).max(40.0),
    );
    let title_width = title.size().x;
    painter.galley(
        Pos2::new(title_left, y - title.size().y / 2.0),
        title,
        title_color,
    );
    let rule_left = title_left + title_width + 8.0;
    let rule_right = count_left - 8.0;
    if rule_right > rule_left {
        painter.line_segment(
            [Pos2::new(rule_left, y), Pos2::new(rule_right, y)],
            Stroke::new(1.0, theme::border()),
        );
    }

    // Chevron: down when open, right when folded.
    let chevron_color = if hovered {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let c = Pos2::new(chevron_x, y);
    let points = if section.is_collapsed {
        [
            c + Vec2::new(-2.0, -3.5),
            c + Vec2::new(2.5, 0.0),
            c + Vec2::new(-2.0, 3.5),
        ]
    } else {
        [
            c + Vec2::new(-3.5, -2.0),
            c + Vec2::new(0.0, 2.5),
            c + Vec2::new(3.5, -2.0),
        ]
    };
    painter.line_segment([points[0], points[1]], Stroke::new(1.6, chevron_color));
    painter.line_segment([points[1], points[2]], Stroke::new(1.6, chevron_color));

    accessibility::label(
        response,
        format!("{} section, {} symbols", section.title, count),
    )
    .on_hover_text(if section.is_collapsed {
        "Expand section"
    } else {
        "Collapse section"
    })
}

/// What an open section with no symbols shows: a quiet dashed area on the same edges
/// as every row. Dropping a symbol anywhere under the heading puts it in the section.
pub fn empty_section_row(ui: &mut egui::Ui) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
    let inner = Rect::from_min_max(
        Pos2::new(rect.left() + LEADING_INSET, rect.top() + 2.0),
        Pos2::new(rect.right() - TRAILING_INSET, rect.bottom() - 2.0),
    );
    let painter = ui.painter_at(rect);
    let color = ui.visuals().weak_text_color().gamma_multiply(0.35);
    let corners = [
        inner.left_top(),
        inner.right_top(),
        inner.right_bottom(),
        inner.left_bottom(),
        inner.left_top(),
    ];
    painter.add(egui::Shape::dashed_line(
        &corners,
        Stroke::new(1.0, color),
        4.0,
        3.0,
    ));
    painter.text(
        inner.center(),
        Align2::CENTER_CENTER,
        "Drag symbols here",
        FontId::proportional(11.0),
        ui.visuals().weak_text_color().gamma_multiply(0.7),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, InstrumentId};

    fn item(source: DataSource, symbol: &str, name: &str, label: &str) -> WatchlistInstrument {
        WatchlistInstrument::new(InstrumentId::new(source, symbol), name, label)
    }

    #[test]
    fn icon_base_per_source() {
        let cases = [
            (
                item(DataSource::Binance, "BTCUSDT", "BTCUSDT", "BTC"),
                "BTC",
            ),
            (item(DataSource::Binance, "ethusdc", "ETH", "ETH"), "ETH"),
            (item(DataSource::Binance, "USDT", "USDT", "USDT"), "USDT"),
            (
                item(DataSource::Coinbase, "eth-usd", "ETH/USD", "ETH-USD"),
                "ETH",
            ),
            (
                item(DataSource::CoinGecko, "bitcoin", "Bitcoin", "btc/usd"),
                "BTC",
            ),
            (item(DataSource::Alpaca, "aapl", "Apple", "AAPL"), "AAPL"),
        ];
        for (item, expected) in cases {
            assert_eq!(icon_base_symbol(&item), expected, "{:?}", item.instrument);
        }
        let mut market = item(DataSource::Polymarket, "123", "Q", "123");
        market.display_name = Some("Will it rain?".to_owned());
        assert_eq!(icon_base_symbol(&market), "Will it rain?");
    }

    #[test]
    fn subtitle_leads_with_the_label_only_when_it_adds_something() {
        let same = item(DataSource::Binance, "BTCUSDT", "BTCUSDT", "btcusdt");
        assert_eq!(subtitle(&same), "Binance");
        let stock = item(DataSource::Alpaca, "AAPL", "Apple Inc.", "AAPL");
        assert_eq!(
            subtitle(&stock),
            format!("AAPL · {}", DataSource::Alpaca.display_name())
        );
        let blank = item(DataSource::Binance, "X", "X coin", "");
        assert_eq!(subtitle(&blank), "Binance");
        let prediction = item(DataSource::Polymarket, "opaque-token", "Will it rain?", "");
        assert_eq!(subtitle(&prediction), "Polymarket");
        let saved = item(
            DataSource::Polymarket,
            "opaque-token",
            "Will it rain?",
            "0xdeadbeef",
        );
        assert_eq!(subtitle(&saved), "Polymarket");
    }

    #[test]
    fn row_height_follows_the_subtitle_setting() {
        let mut display = WatchlistDisplaySettings::default();
        assert_eq!(row_height(&display), ROW_HEIGHT_WITH_SUBTITLE);
        display.shows_description = false;
        assert_eq!(row_height(&display), ROW_HEIGHT_COMPACT);
    }
}
