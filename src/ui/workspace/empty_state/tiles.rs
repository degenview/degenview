//! The pieces the page's tabs are made of: a grid, a market tile, a list row, a layout card, and
//! the note a tab shows when it has nothing yet.

use super::sparkline::{self, Trend};
use super::{EmptyStateAction, LayoutSummary};
use crate::model::{DataSource, InstrumentId};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{self, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use std::collections::HashMap;

/// Space between tiles.
const GAP: f32 = 10.0;
/// Ticker icons drawn on a saved-layout card.
const PREVIEW_LIMIT: usize = 3;
/// The sparkline's size on a tile and on a list row.
const TILE_SPARK: Vec2 = vec2(88.0, 28.0);
const ROW_SPARK: Vec2 = vec2(112.0, 28.0);
/// A tile or row narrower than this has no room for a sparkline beside its name.
const SPARK_MIN_TILE: f32 = 150.0;
const ROW_SPARK_MIN: f32 = 320.0;

/// What the page collects while it draws: the clicks, and the markets it showed.
pub struct Frame<'a> {
    pub trends: &'a HashMap<String, Trend>,
    pub actions: Vec<EmptyStateAction>,
    /// Markets drawn this frame, for the caller to fetch a week of closes for.
    pub wanted: Vec<InstrumentId>,
}

impl<'a> Frame<'a> {
    pub fn new(trends: &'a HashMap<String, Trend>) -> Self {
        Self {
            trends,
            actions: Vec::new(),
            wanted: Vec::new(),
        }
    }

    /// Notes that `market` is on screen and returns what is known of its week.
    fn trend(&mut self, market: &Market) -> Option<&'a Trend> {
        let instrument = InstrumentId::new(market.source, market.api_symbol);
        let trends = self.trends;
        let trend = trends.get(&instrument.key());
        self.wanted.push(instrument);
        trend
    }
}

/// What a market shortcut shows.
pub struct Market<'a> {
    /// The ticker the artwork is looked up by.
    pub icon_symbol: &'a str,
    /// The id the provider takes, to read its week by.
    pub api_symbol: &'a str,
    pub source: DataSource,
    pub title: &'a str,
    pub subtitle: Option<&'a str>,
    /// Whether the data source sits on the artwork's corner.
    pub badged: bool,
}

/// A surface of the page: opaque, so the animated line never shows through, with the card fill and
/// hairline of the rest of the app on top.
pub fn surface(ui: &Ui, rect: Rect, radius: f32, hovered: bool) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::background());
    painter.rect_filled(rect, radius, theme::card_fill(hovered));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::card_border(hovered)),
        StrokeKind::Inside,
    );
}

/// Lays `count` tiles in as many columns of at least `min_width` as fit (never more than
/// `max_columns`); `tile` draws one into its rect.
pub fn grid(
    ui: &mut Ui,
    count: usize,
    min_width: f32,
    max_columns: usize,
    height: f32,
    mut tile: impl FnMut(&mut Ui, usize, Rect),
) {
    let width = ui.available_width();
    let columns = (((width + GAP) / (min_width + GAP)).floor() as usize).clamp(1, max_columns);
    let tile_width = (width - GAP * (columns - 1) as f32) / columns as f32;
    let rows = count.div_ceil(columns);
    let (area, _) = ui.allocate_exact_size(
        vec2(
            width,
            rows as f32 * height + rows.saturating_sub(1) as f32 * GAP,
        ),
        Sense::hover(),
    );
    for index in 0..count {
        let (row, column) = (index / columns, index % columns);
        let rect = Rect::from_min_size(
            pos2(
                area.left() + column as f32 * (tile_width + GAP),
                area.top() + row as f32 * (height + GAP),
            ),
            vec2(tile_width, height),
        );
        tile(ui, index, rect);
    }
}

/// The button every tile is: a flat card that firms its outline under the pointer.
fn tile(ui: &mut Ui, rect: Rect, id: Id, tooltip: &str) -> Response {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    surface(ui, rect, kit::CARD_RADIUS, response.hovered());
    response
}

/// Artwork, ticker, a second line and the week's sparkline, laid out inside `bounds`. The
/// sparkline is `spark` big at the right edge; `None` leaves it out.
fn market_content(
    ui: &Ui,
    bounds: Rect,
    market: &Market,
    trend: Option<&Trend>,
    icon_side: f32,
    spark: Option<Vec2>,
) {
    let icon = Rect::from_min_size(
        pos2(bounds.left(), bounds.center().y - icon_side / 2.0),
        Vec2::splat(icon_side),
    );
    kit::ticker_icon(ui, icon, market.icon_symbol, market.source);
    if market.badged {
        kit::source_badge(ui, icon, market.source);
    }
    let spark_rect = spark.map(|size| {
        Rect::from_min_size(
            pos2(bounds.right() - size.x, bounds.center().y - size.y / 2.0),
            size,
        )
    });
    let left = icon.right() + 12.0;
    let right = spark_rect.map_or(bounds.right(), |rect| rect.left() - 10.0);
    let title = kit::galley(
        ui,
        market.title,
        size::BODY + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        (right - left).max(20.0),
    );
    let subtitle = market.subtitle.map(|text| {
        kit::galley(
            ui,
            text,
            size::CAPTION + 1.0,
            REGULAR,
            theme::text_secondary(),
            (right - left).max(20.0),
        )
    });
    let total = title.size().y + subtitle.as_ref().map_or(0.0, |text| 2.0 + text.size().y);
    let top = bounds.center().y - total / 2.0;
    let title_height = title.size().y;
    ui.painter()
        .galley(pos2(left, top), title, theme::text_primary());
    if let Some(subtitle) = subtitle {
        ui.painter().galley(
            pos2(left, top + title_height + 2.0),
            subtitle,
            theme::text_secondary(),
        );
    }
    if let Some(rect) = spark_rect {
        sparkline::paint(ui.painter(), rect, trend);
    }
}

/// One market as a tile: artwork, ticker, name and its week.
pub fn market_tile(ui: &mut Ui, rect: Rect, id: Id, market: Market, frame: &mut Frame) -> Response {
    let response = tile(ui, rect, id, &format!("Add {} to this tab", market.title));
    let response = crate::ui::accessibility::label(
        response,
        market.subtitle.map_or_else(
            || market.title.to_owned(),
            |subtitle| format!("{}, {subtitle}", market.title),
        ),
    );
    let trend = frame.trend(&market);
    let spark = (rect.width() >= SPARK_MIN_TILE)
        .then(|| vec2(TILE_SPARK.x.min(rect.width() * 0.3), TILE_SPARK.y));
    market_content(
        ui,
        rect.shrink2(vec2(14.0, 0.0)),
        &market,
        trend,
        30.0,
        spark,
    );
    response
}

/// One market as a row of a list: the same, with a chevron at the end.
pub fn market_row(ui: &mut Ui, rect: Rect, id: Id, market: Market, frame: &mut Frame) -> Response {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_text(format!("Add {} to this tab", market.title))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(
        response,
        market.subtitle.map_or_else(
            || market.title.to_owned(),
            |subtitle| format!("{}, {subtitle}", market.title),
        ),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(4.0, 2.0)), 8.0, theme::ink(8));
    }
    let trend = frame.trend(&market);
    let chevron = Rect::from_center_size(
        pos2(rect.right() - 20.0, rect.center().y),
        Vec2::splat(11.0),
    );
    icons::paint(
        ui.painter(),
        chevron,
        Icon::ChevronRight,
        theme::text_tertiary(),
    );
    let spark = (rect.width() >= ROW_SPARK_MIN).then_some(ROW_SPARK);
    market_content(
        ui,
        Rect::from_min_max(
            pos2(rect.left() + 18.0, rect.top()),
            pos2(chevron.left() - 20.0, rect.bottom()),
        ),
        &market,
        trend,
        30.0,
        spark,
    );
    response
}

/// A card holding rows, with a hairline between them.
pub fn list_card(
    ui: &mut Ui,
    rows: usize,
    row_height: f32,
    mut row: impl FnMut(&mut Ui, usize, Rect),
) {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), rows as f32 * row_height),
        Sense::hover(),
    );
    surface(ui, rect, kit::CARD_RADIUS, false);
    for index in 0..rows {
        let top = rect.top() + index as f32 * row_height;
        if index > 0 {
            ui.painter().hline(
                rect.left() + 18.0..=rect.right() - 18.0,
                top,
                Stroke::new(1.0, kit::hairline()),
            );
        }
        row(
            ui,
            index,
            Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), row_height)),
        );
    }
}

/// One saved layout: its name, what is in it, and its first few markets as artwork.
pub fn layout_card(ui: &mut Ui, rect: Rect, layout: &LayoutSummary) -> Response {
    let response = tile(
        ui,
        rect,
        Id::new(("empty-layout", layout.id)),
        &format!("Open {}", layout.name),
    );
    let subtitle = format!(
        "{} · {}",
        if layout.chart_count == 1 {
            "1 chart".to_owned()
        } else {
            format!("{} charts", layout.chart_count)
        },
        layout.timeframe
    );
    let response =
        crate::ui::accessibility::label(response, format!("{}, {subtitle}", layout.name));
    let inner = rect.shrink2(vec2(14.0, 12.0));
    let name = kit::galley(
        ui,
        &layout.name,
        size::BODY + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        inner.width() - 20.0,
    );
    let name_height = name.size().y;
    ui.painter().galley(inner.min, name, theme::text_primary());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(inner.right() - 5.0, inner.top() + name_height / 2.0),
            Vec2::splat(10.0),
        ),
        Icon::ChevronRight,
        theme::text_tertiary(),
    );
    let detail = kit::galley(
        ui,
        &subtitle,
        size::CAPTION + 1.0,
        REGULAR,
        theme::text_secondary(),
        inner.width() - 70.0,
    );
    ui.painter().galley(
        pos2(inner.left(), inner.bottom() - detail.size().y),
        detail,
        theme::text_secondary(),
    );
    // Overlapping artwork, each ringed in the page colour.
    let mut x = inner.right() - 20.0;
    for (source, symbol) in layout.previews.iter().take(PREVIEW_LIMIT).rev() {
        let icon = Rect::from_min_size(pos2(x, inner.bottom() - 20.0), Vec2::splat(20.0));
        ui.painter()
            .circle_filled(icon.center(), 11.5, theme::background());
        kit::ticker_icon(ui, icon, &source.display_symbol(symbol), *source);
        x -= 14.0;
    }
    response
}

/// A link on a line of its own at the right, above or under what it acts on: gray and underlined,
/// as every link is. Says whether it was clicked.
pub fn link_row(ui: &mut Ui, text: &str) -> bool {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 16.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.add(
                egui::Label::new(
                    kit::text(text, size::CAPTION + 1.0, REGULAR, theme::text_secondary())
                        .underline(),
                )
                .sense(Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        },
    )
    .inner
}

/// A line of small gray text above a group of tiles.
pub fn caption(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        size::SUBHEADLINE + 1.0,
        MEDIUM,
        theme::text_secondary(),
    ));
    ui.add_space(8.0);
}

/// What a tab says when it has nothing to show yet.
pub fn note(ui: &mut Ui, icon: Icon, text: &str) {
    let height = 132.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    surface(ui, rect, kit::CARD_RADIUS, false);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.center().x, rect.center().y - 16.0),
            Vec2::splat(20.0),
        ),
        icon,
        theme::text_tertiary(),
    );
    let galley = kit::galley(
        ui,
        text,
        size::BODY,
        REGULAR,
        theme::text_secondary(),
        rect.width() - 48.0,
    );
    ui.painter().galley(
        pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y + 6.0,
        ),
        galley,
        theme::text_secondary(),
    );
}
