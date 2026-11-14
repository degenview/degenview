//! The pieces the page's tabs are made of: a grid, a market tile, a list row, a layout card, and
//! the note a tab shows when it has nothing yet.

use super::{EmptyStateAction, LayoutSummary};
use crate::model::market::trend::Trend;
use crate::model::{DataSource, InstrumentId};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::sparkline::{self, ChangeText};
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
/// A row narrower than this has no room for the week's change.
const CHANGE_MIN_ROW: f32 = 520.0;
/// The change column is at least this wide, so rows line up whatever the numbers.
const CHANGE_MIN_WIDTH: f32 = 84.0;
/// Narrowest a sparkline is drawn; narrower says nothing.
const SPARK_MIN: f32 = 36.0;
/// Room the name keeps before the sparkline gives way to it.
const NAME_MIN: f32 = 50.0;

/// What the page collects while it draws: the clicks, and the markets it showed.
pub struct Frame<'a> {
    pub trends: &'a HashMap<String, Trend>,
    pub actions: Vec<EmptyStateAction>,
    /// Markets drawn this frame, for the caller to fetch a week of closes for.
    pub wanted: Vec<InstrumentId>,
    /// The bottom of the page, so a long list can take the room left under it.
    pub page_bottom: f32,
}

impl<'a> Frame<'a> {
    pub fn new(trends: &'a HashMap<String, Trend>, page_bottom: f32) -> Self {
        Self {
            trends,
            actions: Vec::new(),
            wanted: Vec::new(),
            page_bottom,
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
/// sparkline is at most `spark` big at the right edge, as wide as the name leaves room for and
/// left out when that is too little; `None` leaves it out.
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
    let left = icon.right() + 12.0;
    let natural = |text: &str, font: f32, weight: f32| {
        kit::galley(ui, text, font, weight, theme::text_primary(), f32::INFINITY)
            .size()
            .x
    };
    let name_width = natural(market.title, size::BODY + 1.0, SEMIBOLD).max(
        market
            .subtitle
            .map_or(0.0, |text| natural(text, size::CAPTION + 1.0, REGULAR)),
    );
    let room = bounds.right() - left - 10.0;
    let spark_rect = spark.and_then(|size| {
        let width = (room - name_width).clamp(SPARK_MIN, size.x);
        // The name keeps its own width, or `NAME_MIN` when it is longer than that.
        (room - width >= name_width.min(NAME_MIN)).then(|| {
            Rect::from_min_size(
                pos2(bounds.right() - width, bounds.center().y - size.y / 2.0),
                vec2(width, size.y),
            )
        })
    });
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
    market_content(
        ui,
        rect.shrink2(vec2(14.0, 0.0)),
        &market,
        trend,
        30.0,
        Some(TILE_SPARK),
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
    let mut right = chevron.left() - 20.0;
    if rect.width() >= CHANGE_MIN_ROW
        && let Some(Trend::Ready(closes)) = trend
        && let Some(change) = sparkline::change_text(closes, !market.source.is_prediction_market())
    {
        right = paint_change(ui, rect, right, &change, sparkline::rising(closes)) - 18.0;
    }
    market_content(
        ui,
        Rect::from_min_max(
            pos2(rect.left() + 18.0, rect.top()),
            pos2(right, rect.bottom()),
        ),
        &market,
        trend,
        30.0,
        Some(ROW_SPARK),
    );
    response
}

/// The week's move, right-aligned to `right`: the share over the amount, in the colour of the
/// direction. Returns where it starts.
fn paint_change(ui: &Ui, row: Rect, right: f32, change: &ChangeText, up: bool) -> f32 {
    let color = if up { theme::UP } else { theme::DOWN };
    let percent = kit::galley(
        ui,
        &change.percent,
        size::BODY,
        MEDIUM,
        color,
        f32::INFINITY,
    );
    let amount = change.amount.as_deref().map(|text| {
        kit::galley(
            ui,
            text,
            size::CAPTION + 1.0,
            REGULAR,
            color.gamma_multiply(0.75),
            f32::INFINITY,
        )
    });
    let width = amount
        .as_ref()
        .map_or(percent.size().x, |amount| {
            amount.size().x.max(percent.size().x)
        })
        .max(CHANGE_MIN_WIDTH);
    let total = percent.size().y + amount.as_ref().map_or(0.0, |text| 2.0 + text.size().y);
    let top = row.center().y - total / 2.0;
    let percent_height = percent.size().y;
    let percent_x = right - percent.size().x;
    ui.painter().galley(pos2(percent_x, top), percent, color);
    if let Some(amount) = amount {
        let x = right - amount.size().x;
        ui.painter().galley(
            pos2(x, top + percent_height + 2.0),
            amount,
            color.gamma_multiply(0.75),
        );
    }
    right - width
}

/// A card holding rows, with a hairline between them. Taller than `max_height` it scrolls, and
/// only the rows in view are drawn (so only they are asked for).
pub fn list_card(
    ui: &mut Ui,
    rows: usize,
    row_height: f32,
    max_height: f32,
    mut row: impl FnMut(&mut Ui, usize, Rect),
) {
    let total = rows as f32 * row_height;
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), total.min(max_height)),
        Sense::hover(),
    );
    surface(ui, rect, kit::CARD_RADIUS, false);
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(1.0)));
    egui::ScrollArea::vertical()
        .id_salt("empty-state-list")
        .auto_shrink([false, false])
        .show_viewport(&mut inner, |ui, view| {
            let (content, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), total), Sense::hover());
            let first = (view.top() / row_height).floor().max(0.0) as usize;
            let last = ((view.bottom() / row_height).ceil() as usize).min(rows);
            for index in first..last {
                let top = content.top() + index as f32 * row_height;
                if index > 0 {
                    ui.painter().hline(
                        content.left() + 17.0..=content.right() - 17.0,
                        top,
                        Stroke::new(1.0, kit::hairline()),
                    );
                }
                row(
                    ui,
                    index,
                    Rect::from_min_size(
                        pos2(content.left(), top),
                        vec2(content.width(), row_height),
                    ),
                );
            }
        });
}

/// The tile that ends a capped list of favorites: how many did not fit, and a way to see them.
pub fn more_tile(ui: &mut Ui, rect: Rect, hidden: usize) -> Response {
    let response = tile(
        ui,
        rect,
        Id::new("empty-favorites-more"),
        "Show every favorite in the watchlist",
    );
    let response = crate::ui::accessibility::label(response, format!("{hidden} more favorites"));
    let bounds = rect.shrink2(vec2(14.0, 0.0));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(bounds.left() + 15.0, rect.center().y),
            Vec2::splat(18.0),
        ),
        Icon::Ellipsis,
        theme::text_secondary(),
    );
    let left = bounds.left() + 42.0;
    let title = kit::galley(
        ui,
        &format!("{hidden} more"),
        size::BODY + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        bounds.right() - left,
    );
    let subtitle = kit::galley(
        ui,
        "Open watchlist",
        size::CAPTION + 1.0,
        REGULAR,
        theme::text_secondary(),
        bounds.right() - left,
    );
    let top = rect.center().y - (title.size().y + 2.0 + subtitle.size().y) / 2.0;
    let title_height = title.size().y;
    ui.painter()
        .galley(pos2(left, top), title, theme::text_primary());
    ui.painter().galley(
        pos2(left, top + title_height + 2.0),
        subtitle,
        theme::text_secondary(),
    );
    response
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

/// A hairline across the page's content, with room around it, between two groups of tiles.
pub fn separator(ui: &mut Ui) {
    ui.add_space(22.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, kit::hairline()),
    );
    ui.add_space(22.0);
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
