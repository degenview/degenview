//! The portfolio's front page: what it is worth, how it has moved, and where the money sits.
//! And, beside it, the Statistics tab: performance, value range, activity and P&L by asset.
//! Pure views over the holdings, history and statistics the host computes.
//!
//! Every block here has a fixed height and paints its text at measured positions, so nothing
//! reflows while prices stream in and cards in one row always end level.

// Transient: the Portfolio window hosts these once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{
    PortfolioAsset, PortfolioHolding, PortfolioSnapshot, PortfolioTransaction,
};
use crate::model::portfolio_format::{PortfolioValueFormatter, Tone};
use crate::model::portfolio_statistics::{
    AssetResult, PortfolioHistoryRange, PortfolioPeriodChange, PortfolioStatistics, pnl_bars,
};
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::portfolio_charts::{self, HistoryInput};
use crate::ui::portfolio_empty;
use crate::ui::portfolio_table::{self, tone_color, tone_option};
use crate::ui::replay_bar::date_text;
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Galley, Margin, Pos2, Rect, Sense, Shape, Stroke, Ui,
    UiBuilder, Vec2, pos2, vec2,
};
use std::sync::Arc;

/// How many holdings the "Top Holdings" panel lists.
pub const TOP_COUNT: usize = 6;

/// Space around the page and between its blocks.
const PAGE_INSET: i8 = 16;
const SECTION_GAP: f32 = 16.0;
const GRID_GAP: f32 = 12.0;
/// A stat or performer card: two 12 pt paddings around a 50 pt text block.
const CARD_HEIGHT: f32 = 76.0;
const HERO_HEIGHT: f32 = 136.0;
const HERO_PADDING: f32 = 22.0;
/// From this content width the Overview puts a side column beside the charts and the lists.
const WIDE_MIN: f32 = 1_000.0;
const SIDE_WIDTH: f32 = 400.0;
/// The value chart's content height beside the allocation, a little taller than when stacked.
const WIDE_CHART_HEIGHT: f32 = 300.0;
/// A panel's padding, its title row and the gap under it.
const PANEL_PADDING: f32 = 16.0;
const PANEL_HEADER: f32 = 27.0;
const PANEL_GAP: f32 = 12.0;
const HISTORY_CHART_HEIGHT: f32 = 250.0;
/// Both lower panels size their content to this, so they always end level: six Top Holdings
/// rows fill it exactly.
const PANEL_CONTENT_HEIGHT: f32 = 270.0;
const TOP_ROW_HEIGHT: f32 = 43.0;
const TOP_ROW_GAP: f32 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub enum OverviewAction {
    /// Open this asset's transactions.
    Select(String),
    /// "View All" in Top Holdings.
    ShowHoldings,
}

pub struct OverviewInput<'a> {
    /// `None` is "All Portfolios".
    pub portfolio_name: Option<&'a str>,
    pub holdings: &'a [PortfolioHolding],
    /// The value history for the selected portfolio (intraday for 1D when there is some).
    pub history: &'a [PortfolioSnapshot],
    pub total_value: f64,
    pub formatter: PortfolioValueFormatter,
    pub subtitle: &'a dyn Fn(&PortfolioAsset) -> String,
    /// "Prices as of 14:03" and the like.
    pub market_value_caption: &'a str,
    pub price_status: PriceStatus,
    pub is_loading_initial: bool,
    pub is_loading_history: bool,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

#[derive(Default)]
pub struct OverviewState {
    pub range: PortfolioHistoryRange,
    hovered_slice: Option<usize>,
}

/// The top holdings by value, largest first; unpriced ones last. Equal values keep a fixed
/// order (by market), so rows never swap places between frames.
pub fn top_holdings(holdings: &[PortfolioHolding]) -> Vec<&PortfolioHolding> {
    let mut ranked: Vec<&PortfolioHolding> = holdings.iter().collect();
    ranked.sort_by(|a, b| {
        b.current_value
            .unwrap_or(0.0)
            .total_cmp(&a.current_value.unwrap_or(0.0))
            .then_with(|| a.asset.key.cmp(&b.asset.key))
    });
    ranked.truncate(TOP_COUNT);
    ranked
}

/// "Portfolio · 3 assets".
pub fn asset_count_text(count: usize) -> String {
    format!("{count} {}", if count == 1 { "asset" } else { "assets" })
}

/// The caption beside a change figure: "▲ 3.21%" (the glyph hidden in privacy mode, and for a
/// move of exactly zero).
pub fn change_caption(formatter: &PortfolioValueFormatter, amount: f64, percentage: f64) -> String {
    let glyph = if formatter.privacy || amount == 0.0 {
        ""
    } else if amount > 0.0 {
        "▲ "
    } else {
        "▼ "
    };
    format!("{glyph}{}", formatter.percent(percentage.abs(), 2))
}

// MARK: Painting helpers

/// One line of text at an SF weight, laid out so it can be measured before it is placed.
fn line(ui: &Ui, text: &str, size: f32, weight: f32, color: Color32) -> Arc<Galley> {
    kit::galley(ui, text, size, weight, color, 4_000.0)
}

fn draw(ui: &Ui, at: Pos2, galley: &Arc<Galley>) {
    ui.painter()
        .galley(at, galley.clone(), Color32::PLACEHOLDER);
}

/// Paints `text` anchored at `pos`; returns the rect it covers.
fn text_at(
    ui: &Ui,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    size: f32,
    weight: f32,
    color: Color32,
) -> Rect {
    let galley = line(ui, text, size, weight, color);
    let rect = anchor.anchor_size(pos, galley.size());
    draw(ui, rect.min, &galley);
    rect
}

fn caption_color() -> Color32 {
    theme::text_secondary()
}

/// The tone's colour, or `fallback` when there is none (zero, unknown, or values hidden).
fn tone_or(tone: Option<Tone>, fallback: Color32) -> Color32 {
    tone_option(tone).unwrap_or(fallback)
}

/// A small filled triangle: the direction mark beside a change percentage.
fn paint_arrow(ui: &Ui, center: Pos2, up: bool, color: Color32) {
    let (half_width, half_height) = (3.5, 3.0);
    let points = if up {
        vec![
            pos2(center.x - half_width, center.y + half_height),
            pos2(center.x + half_width, center.y + half_height),
            pos2(center.x, center.y - half_height),
        ]
    } else {
        vec![
            pos2(center.x - half_width, center.y - half_height),
            pos2(center.x + half_width, center.y - half_height),
            pos2(center.x, center.y + half_height),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A raised surface: a soft shadow under the fill, and a hairline around it.
pub(crate) fn lift(ui: &Ui, rect: Rect, radius: f32) {
    let painter = ui.painter();
    painter.add(
        egui::Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(30),
        }
        .as_shape(rect, radius),
    );
    painter.rect_filled(rect, radius, theme::chrome());
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
}

/// A soft light of `color` rising from the bottom of `rect`, strongest right of centre and
/// gone before the corners, so it needs no clipping to the rounded shape.
fn glow(ui: &Ui, rect: Rect, color: Color32) {
    const COLUMNS: usize = 20;
    let from = rect.left() + rect.width() * 0.3;
    let to = rect.right() - 20.0;
    let (top, bottom) = (rect.top() + rect.height() * 0.35, rect.bottom() - 1.0);
    let mut mesh = egui::Mesh::default();
    for column in 0..=COLUMNS {
        let t = column as f32 / COLUMNS as f32;
        let x = from + (to - from) * t;
        let bell = (std::f32::consts::PI * t).sin().powi(2);
        mesh.colored_vertex(pos2(x, top), color.gamma_multiply(0.0));
        mesh.colored_vertex(pos2(x, bottom), color.gamma_multiply(0.13 * bell));
    }
    for column in 0..COLUMNS as u32 {
        let base = column * 2;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    ui.painter().add(Shape::mesh(mesh));
}

/// A panel: fixed outer height, a title row with an optional trailing control, then content.
/// Because the height is given, two panels in a row end level whatever they hold.
fn panel(
    ui: &mut Ui,
    content_height: f32,
    icon: Icon,
    title: &str,
    content: impl FnOnce(&mut Ui, &mut Ui),
) {
    let height = panel_height(content_height);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    lift(ui, rect, 14.0);
    let inner = rect.shrink(PANEL_PADDING);
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), PANEL_HEADER));
    let chip = Rect::from_center_size(
        pos2(header.left() + 11.0, header.center().y),
        Vec2::splat(22.0),
    );
    ui.painter()
        .rect_filled(chip, 7.0, theme::SYSTEM_ACCENT.gamma_multiply(0.14));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(chip.center(), Vec2::splat(13.0)),
        icon,
        theme::SYSTEM_ACCENT,
    );
    text_at(
        ui,
        pos2(chip.right() + 9.0, header.center().y),
        Align2::LEFT_CENTER,
        title,
        kit::size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
    );
    let mut header_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(header)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let body = Rect::from_min_size(
        pos2(inner.left(), header.bottom() + PANEL_GAP),
        vec2(inner.width(), content_height),
    );
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body));
    body_ui.set_clip_rect(
        body.expand2(vec2(PANEL_PADDING, 0.0))
            .intersect(ui.clip_rect()),
    );
    content(&mut header_ui, &mut body_ui);
}

/// The outer height of a [`panel`] whose content is `content_height` tall.
fn panel_height(content_height: f32) -> f32 {
    PANEL_PADDING * 2.0 + PANEL_HEADER + PANEL_GAP + content_height
}

/// A tinted, bordered card of a fixed height; `inside` is the content rect. `wash` lays a faint
/// light of the card's tone over it.
fn card_frame(ui: &mut Ui, height: f32, wash: Option<Color32>) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, 12.0, theme::chrome());
    if let Some(wash) = wash {
        ui.painter()
            .rect_filled(rect, 12.0, wash.gamma_multiply(0.07));
    }
    ui.painter().rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    rect
}

/// Two columns of the given widths, tops level, with `gap` between. Both share `shared`, which
/// each takes in turn.
fn split<T>(
    ui: &mut Ui,
    widths: [f32; 2],
    gap: f32,
    shared: &mut T,
    left: impl FnOnce(&mut Ui, &mut T),
    right: impl FnOnce(&mut Ui, &mut T),
) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(
            vec2(widths[0], 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(widths[0]);
                left(ui, shared);
            },
        );
        ui.allocate_ui_with_layout(
            vec2(widths[1], 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(widths[1]);
                right(ui, shared);
            },
        );
    });
}

// MARK: Overview

/// How current the prices behind the total are; colours the dot beside the caption.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PriceStatus {
    /// Every holding has a live price.
    #[default]
    Live,
    /// Some price is a cached one.
    Cached,
    /// Some holding has no price.
    Partial,
    /// The valuation failed.
    Unavailable,
}

impl PriceStatus {
    fn color(self) -> Color32 {
        match self {
            Self::Live => kit::GREEN,
            Self::Cached | Self::Partial => kit::ORANGE,
            Self::Unavailable => kit::RED,
        }
    }
}

pub fn show_overview(
    ui: &mut egui::Ui,
    state: &mut OverviewState,
    input: &OverviewInput,
) -> Vec<OverviewAction> {
    let mut actions = Vec::new();
    let stats = PortfolioStatistics::new(input.holdings, input.history, &[]);
    let ranged = state.range.filter(input.history, input.now_ms);
    let period = PortfolioPeriodChange::new(&ranged, input.total_value);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(PAGE_INSET))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(GRID_GAP, SECTION_GAP);
                    let wide = ui.available_width() >= WIDE_MIN;
                    hero(ui, state, input, &stats, period);
                    summary_cards(ui, input.formatter, &stats);
                    if wide {
                        let width = ui.available_width();
                        let widths = [width - SIDE_WIDTH - SECTION_GAP, SIDE_WIDTH];
                        split(
                            ui,
                            widths,
                            SECTION_GAP,
                            state,
                            |ui, state| history_card(ui, state, input, WIDE_CHART_HEIGHT),
                            |ui, state| allocation_panel(ui, state, input, WIDE_CHART_HEIGHT),
                        );
                        split(
                            ui,
                            widths,
                            SECTION_GAP,
                            &mut actions,
                            |ui, actions| top_holdings_panel(ui, input, actions),
                            |ui, _| {
                                ui.spacing_mut().item_spacing.y = GRID_GAP;
                                let each = (panel_height(PANEL_CONTENT_HEIGHT) - GRID_GAP) / 2.0;
                                performer_card(
                                    ui,
                                    "Best Performer",
                                    stats.best.as_ref(),
                                    input,
                                    each,
                                );
                                performer_card(
                                    ui,
                                    "Worst Performer",
                                    stats.worst.as_ref(),
                                    input,
                                    each,
                                );
                            },
                        );
                    } else {
                        ui.columns(2, |columns| {
                            performer_card(
                                &mut columns[0],
                                "Best Performer",
                                stats.best.as_ref(),
                                input,
                                CARD_HEIGHT,
                            );
                            performer_card(
                                &mut columns[1],
                                "Worst Performer",
                                stats.worst.as_ref(),
                                input,
                                CARD_HEIGHT,
                            );
                        });
                        history_card(ui, state, input, HISTORY_CHART_HEIGHT);
                        ui.spacing_mut().item_spacing.x = SECTION_GAP;
                        ui.columns(2, |columns| {
                            allocation_panel(&mut columns[0], state, input, PANEL_CONTENT_HEIGHT);
                            top_holdings_panel(&mut columns[1], input, &mut actions);
                        });
                    }
                });
        });
    actions
}

fn allocation_panel(
    ui: &mut Ui,
    state: &mut OverviewState,
    input: &OverviewInput,
    content_height: f32,
) {
    panel(ui, content_height, Icon::PieChart, "Allocation", |_, ui| {
        portfolio_charts::show_allocation(
            ui,
            input.holdings,
            input.formatter,
            &mut state.hovered_slice,
            content_height,
        );
    });
}

fn hero(
    ui: &mut Ui,
    state: &OverviewState,
    input: &OverviewInput,
    stats: &PortfolioStatistics,
    period: Option<PortfolioPeriodChange>,
) {
    let f = input.formatter;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HERO_HEIGHT), Sense::hover());
    let tint = f
        .tone(period.map(|p| p.profit))
        .map_or(theme::SYSTEM_ACCENT, tone_color);
    lift(ui, rect, 16.0);
    if !input.is_loading_initial && !input.holdings.is_empty() {
        glow(ui, rect, tint);
    }
    let inner = rect.shrink(HERO_PADDING);

    // The portfolio's name on a chip of its own.
    let chip = Rect::from_center_size(
        pos2(inner.left() + 11.0, inner.top() + 11.0),
        Vec2::splat(22.0),
    );
    ui.painter()
        .rect_filled(chip, 7.0, theme::SYSTEM_ACCENT.gamma_multiply(0.14));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(chip.center(), Vec2::splat(13.0)),
        Icon::Briefcase,
        theme::SYSTEM_ACCENT,
    );
    let name = text_at(
        ui,
        pos2(chip.right() + 9.0, chip.center().y),
        Align2::LEFT_CENTER,
        input.portfolio_name.unwrap_or("All Portfolios"),
        15.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    let count = text_at(
        ui,
        pos2(name.right() + 6.0, chip.center().y),
        Align2::LEFT_CENTER,
        &format!("\u{00B7} {}", asset_count_text(input.holdings.len())),
        15.0,
        REGULAR,
        caption_color(),
    );

    let balance_top = inner.top() + 32.0;
    let balance_middle = balance_top + 24.0;
    let mut left_end = count.right();
    if input.is_loading_initial {
        ui.put(
            Rect::from_center_size(pos2(inner.left() + 8.0, balance_middle), Vec2::splat(16.0)),
            egui::Spinner::new().size(16.0),
        );
        let loading = text_at(
            ui,
            pos2(inner.left() + 26.0, balance_middle),
            Align2::LEFT_CENTER,
            "Loading portfolio value\u{2026}",
            12.0,
            REGULAR,
            caption_color(),
        );
        left_end = left_end.max(loading.right());
    } else {
        let balance = text_at(
            ui,
            pos2(inner.left(), balance_top),
            Align2::LEFT_TOP,
            &f.money(input.total_value),
            40.0,
            SEMIBOLD,
            theme::text_primary(),
        );
        left_end = left_end.max(balance.right());
    }

    // Where the prices stand: a dot in the state's colour, then the caption.
    let caption_middle = inner.bottom() - 6.0;
    if !input.is_loading_initial {
        ui.painter().circle_filled(
            pos2(inner.left() + 4.0, caption_middle),
            3.5,
            input.price_status.color(),
        );
    }
    let caption = text_at(
        ui,
        pos2(
            inner.left() + if input.is_loading_initial { 0.0 } else { 14.0 },
            caption_middle,
        ),
        Align2::LEFT_CENTER,
        input.market_value_caption,
        kit::size::CAPTION,
        REGULAR,
        caption_color(),
    );
    left_end = left_end.max(caption.right());

    if input.is_loading_initial {
        return;
    }
    let middle = rect.center().y;
    let pnl_label = format!("P&L \u{00B7} {}", state.range.period_name());
    let edge = change_block(
        ui,
        f,
        &pnl_label,
        period.map(|p| p.profit),
        period.and_then(|p| p.percentage),
        pos2(inner.right(), middle),
    );
    let day_edge = change_block(
        ui,
        f,
        "24h",
        stats.day_change.map(|d| d.amount),
        stats.day_change.and_then(|d| d.percentage),
        pos2(edge - 36.0, middle),
    );
    ui.painter().vline(
        edge - 18.0,
        middle - 26.0..=middle + 26.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
    );

    // Between the two, the shape of the range.
    let room = Rect::from_min_max(
        pos2(left_end + 36.0, inner.top() + 6.0),
        pos2(day_edge - 36.0, inner.bottom() - 6.0),
    );
    if room.width() >= 120.0 && !f.privacy {
        sparkline(
            ui,
            room,
            &portfolio_charts::prepared_points(
                input.history,
                input.total_value,
                state.range,
                input.now_ms,
                false,
            ),
        );
    }
}

/// The value over the range as a line with a fade under it; green when it ends above where it
/// began, red when below.
fn sparkline(ui: &Ui, rect: Rect, points: &[PortfolioSnapshot]) {
    let Some((minimum, maximum)) = portfolio_charts::value_bounds(points) else {
        return;
    };
    if points.len() < 2 {
        return;
    }
    let color = if points[points.len() - 1].value >= points[0].value {
        theme::UP
    } else {
        theme::DOWN
    };
    let spread = (maximum - minimum).max(f64::EPSILON);
    let line: Vec<Pos2> = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            pos2(
                rect.left() + rect.width() * index as f32 / (points.len() - 1) as f32,
                rect.bottom() - rect.height() * ((point.value - minimum) / spread) as f32,
            )
        })
        .collect();
    let mut mesh = egui::Mesh::default();
    for point in &line {
        mesh.colored_vertex(*point, color.gamma_multiply(0.20));
        mesh.colored_vertex(pos2(point.x, rect.bottom()), color.gamma_multiply(0.0));
    }
    for step in 0..line.len() as u32 - 1 {
        let base = step * 2;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    ui.painter().add(Shape::mesh(mesh));
    let end = line[line.len() - 1];
    ui.painter().add(Shape::line(line, Stroke::new(2.0, color)));
    ui.painter().circle_filled(end, 3.5, color);
    ui.painter()
        .circle_stroke(end, 3.5, Stroke::new(2.0, theme::chrome()));
}

/// A period label over a signed amount over a pill with the percentage, left-aligned in a block
/// that ends at `anchor.x` and is centred on `anchor.y`. Returns the block's left edge.
fn change_block(
    ui: &Ui,
    f: PortfolioValueFormatter,
    label: &str,
    amount: Option<f64>,
    percentage: Option<f64>,
    anchor: Pos2,
) -> f32 {
    let secondary = caption_color();
    let label = line(ui, label, kit::size::CAPTION, REGULAR, secondary);
    let tone = f.tone(amount);
    let value = match amount {
        Some(amount) => line(
            ui,
            &f.signed_money(amount),
            17.0,
            SEMIBOLD,
            tone_or(tone, theme::text_primary()),
        ),
        None => line(ui, "\u{2014}", 17.0, SEMIBOLD, secondary),
    };
    let percent = percentage.filter(|_| amount.is_some()).map(|percentage| {
        line(
            ui,
            &f.percent(percentage.abs(), 2),
            kit::size::CAPTION,
            MEDIUM,
            tone_or(tone, secondary),
        )
    });
    let arrow = amount.is_some_and(|amount| !f.privacy && amount != 0.0);
    let pill_width = percent.as_ref().map_or(0.0, |galley| {
        galley.size().x + 16.0 + if arrow { 12.0 } else { 0.0 }
    });
    let width = label.size().x.max(value.size().x).max(pill_width);
    let pill_height = if percent.is_some() { 20.0 + 5.0 } else { 0.0 };
    let height = label.size().y + 3.0 + value.size().y + pill_height;
    let left = anchor.x - width;
    let mut y = anchor.y - height / 2.0;
    draw(ui, pos2(left, y), &label);
    y += label.size().y + 3.0;
    draw(ui, pos2(left, y), &value);
    y += value.size().y + 5.0;
    if let Some(percent) = percent {
        let pill = Rect::from_min_size(pos2(left, y), vec2(pill_width, 20.0));
        let fill = tone_option(tone).unwrap_or_else(theme::text_tertiary);
        ui.painter()
            .rect_filled(pill, 10.0, fill.gamma_multiply(0.16));
        let mut x = pill.left() + 8.0;
        if arrow {
            paint_arrow(
                ui,
                pos2(x + 3.5, pill.center().y),
                amount.is_some_and(|amount| amount > 0.0),
                tone_or(tone, secondary),
            );
            x += 12.0;
        }
        draw(
            ui,
            pos2(x, pill.center().y - percent.size().y / 2.0),
            &percent,
        );
    }
    left
}

pub struct StatCard<'a> {
    pub title: &'a str,
    pub value: String,
    pub tone: Option<Tone>,
    pub caption: Option<String>,
    /// Quieter value when there is nothing to report yet.
    pub placeholder: bool,
    pub icon: Option<Icon>,
}

impl<'a> StatCard<'a> {
    pub fn new(title: &'a str, value: String) -> Self {
        Self {
            title,
            value,
            tone: None,
            caption: None,
            placeholder: false,
            icon: None,
        }
    }

    pub fn tone(mut self, tone: Option<Tone>) -> Self {
        self.tone = tone;
        self
    }

    pub fn caption(mut self, caption: Option<String>) -> Self {
        self.caption = caption;
        self
    }

    pub fn placeholder(mut self, placeholder: bool) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

pub fn stat_card(ui: &mut egui::Ui, card: &StatCard) {
    let tint = card.tone.map_or(theme::SYSTEM_ACCENT, tone_color);
    let rect = card_frame(ui, CARD_HEIGHT, card.tone.map(tone_color));
    let mut x = rect.left() + 14.0;
    if let Some(icon) = card.icon {
        let badge = Rect::from_center_size(pos2(x + 17.0, rect.center().y), Vec2::splat(34.0));
        ui.painter()
            .rect_filled(badge, 9.0, tint.gamma_multiply(0.14));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(badge.center(), Vec2::splat(17.0)),
            icon,
            tint,
        );
        x = badge.right() + 12.0;
    }
    let max_width = (rect.right() - 14.0 - x).max(20.0);
    let secondary = caption_color();
    let title = line(ui, card.title, kit::size::CAPTION, REGULAR, secondary);
    let value_color = if card.placeholder {
        secondary
    } else {
        tone_or(card.tone, theme::text_primary())
    };
    let value = kit::galley(ui, &card.value, 17.0, SEMIBOLD, value_color, max_width);
    // Always laid out, so cards with and without a caption share one text block.
    let caption = card.caption.as_deref().map(|caption| {
        kit::galley(
            ui,
            caption,
            kit::size::CAPTION,
            REGULAR,
            tone_or(card.tone, secondary),
            max_width,
        )
    });
    let block = title.size().y + 4.0 + value.size().y + 4.0 + title.size().y;
    let mut y = rect.center().y - block / 2.0;
    draw(ui, pos2(x, y), &title);
    y += title.size().y + 4.0;
    draw(ui, pos2(x, y), &value);
    y += value.size().y + 4.0;
    if let Some(caption) = caption {
        draw(ui, pos2(x, y), &caption);
    }
}

pub fn grid(ui: &mut egui::Ui, cards: &[StatCard]) {
    ui.spacing_mut().item_spacing.x = GRID_GAP;
    // Two across where four would squeeze a figure out of its card.
    let across = if ui.available_width() < 640.0 { 2 } else { 4 };
    ui.columns(across, |columns| {
        for (index, card) in cards.iter().enumerate() {
            stat_card(&mut columns[index % across], card);
        }
    });
}

fn summary_cards(ui: &mut egui::Ui, f: PortfolioValueFormatter, stats: &PortfolioStatistics) {
    grid(
        ui,
        &[
            StatCard::new("Cost Basis", f.money(stats.cost_basis)).icon(Icon::Banknote),
            StatCard::new("All-time P&L", f.signed_money(stats.total_pnl()))
                .tone(f.tone(Some(stats.total_pnl())))
                .icon(Icon::ChartUptrend),
            StatCard::new("Unrealized P&L", f.signed_money(stats.unrealized_pnl))
                .tone(f.tone(Some(stats.unrealized_pnl)))
                .caption(stats.unrealized_percent.map(|p| f.signed_percent(p, 2)))
                .icon(Icon::Clock),
            StatCard::new("Realized P&L", f.signed_money(stats.realized_pnl))
                .tone(f.tone(Some(stats.realized_pnl)))
                .icon(Icon::CheckmarkSeal),
        ],
    );
}

fn performer_card(
    ui: &mut egui::Ui,
    title: &str,
    result: Option<&AssetResult>,
    input: &OverviewInput,
    height: f32,
) {
    if height > CARD_HEIGHT * 1.5 {
        return performer_tile(ui, title, result, input, height);
    }
    let f = input.formatter;
    let rect = card_frame(ui, height, None);
    let left = rect.left() + 14.0;
    let right = rect.right() - 14.0;
    let secondary = caption_color();
    let top = rect.top() + 12.0;
    let heading = text_at(
        ui,
        pos2(left, top),
        Align2::LEFT_TOP,
        title,
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let row = Rect::from_min_max(
        pos2(left, heading.bottom() + 8.0),
        pos2(right, rect.bottom() - 12.0),
    );
    let Some(result) = result else {
        text_at(
            ui,
            pos2(left, row.center().y),
            Align2::LEFT_CENTER,
            "—",
            15.0,
            SEMIBOLD,
            secondary,
        );
        return;
    };

    let icon = Rect::from_min_size(row.min, Vec2::splat(row.height().min(30.0)));
    ui.scope_builder(UiBuilder::new().max_rect(icon), |ui| {
        portfolio_table::asset_icon(ui, &result.asset, icon.width());
    });

    let percent = result
        .pnl_percent
        .map_or("—".to_owned(), |p| f.signed_percent(p, 2));
    let big = line(
        ui,
        &percent,
        15.0,
        SEMIBOLD,
        tone_or(f.tone(result.pnl_percent), theme::text_primary()),
    );
    let small = line(
        ui,
        &f.signed_money(result.total_pnl),
        kit::size::CAPTION,
        REGULAR,
        tone_or(f.tone(Some(result.total_pnl)), secondary),
    );
    let figures_height = big.size().y + 1.0 + small.size().y;
    let figures_top = row.center().y - figures_height / 2.0;
    draw(ui, pos2(right - big.size().x, figures_top), &big);
    draw(
        ui,
        pos2(right - small.size().x, figures_top + big.size().y + 1.0),
        &small,
    );
    let figures_width = big.size().x.max(small.size().x);

    let text_left = icon.right() + 10.0;
    let text_width = (right - figures_width - 12.0 - text_left).max(30.0);
    let ticker = kit::galley(
        ui,
        &result.asset.display_ticker(),
        kit::size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let name = kit::galley(
        ui,
        &(input.subtitle)(&result.asset),
        kit::size::CAPTION,
        REGULAR,
        secondary,
        text_width,
    );
    let names_top = row.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    draw(ui, pos2(text_left, names_top), &ticker);
    draw(
        ui,
        pos2(text_left, names_top + ticker.size().y + 1.0),
        &name,
    );
}

/// A performer as a tall tile: which it is, what it is, then how far it moved in large type.
fn performer_tile(
    ui: &mut egui::Ui,
    title: &str,
    result: Option<&AssetResult>,
    input: &OverviewInput,
    height: f32,
) {
    let f = input.formatter;
    let best = title.starts_with("Best");
    let tone = result.and_then(|result| f.tone(Some(result.total_pnl)));
    let rect = card_frame(ui, height, tone.map(tone_color));
    let (left, right) = (rect.left() + 18.0, rect.right() - 18.0);
    let secondary = caption_color();
    let accent = if best { kit::GREEN } else { kit::RED };
    let badge = Rect::from_center_size(pos2(left + 10.0, rect.top() + 24.0), Vec2::splat(20.0));
    ui.painter()
        .rect_filled(badge, 6.0, accent.gamma_multiply(0.16));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(badge.center(), Vec2::splat(11.0)),
        if best {
            Icon::ArrowUpRight
        } else {
            Icon::ArrowDownRight
        },
        accent,
    );
    text_at(
        ui,
        pos2(badge.right() + 8.0, badge.center().y),
        Align2::LEFT_CENTER,
        title,
        kit::size::SUBHEADLINE,
        MEDIUM,
        secondary,
    );
    let Some(result) = result else {
        text_at(
            ui,
            pos2(left, rect.center().y + 10.0),
            Align2::LEFT_CENTER,
            "No priced holdings yet",
            kit::size::SUBHEADLINE,
            REGULAR,
            secondary,
        );
        return;
    };

    let icon = Rect::from_min_size(pos2(left, badge.bottom() + 14.0), Vec2::splat(36.0));
    // Painted in place: a scope would move the column's cursor back up to the icon.
    portfolio_table::paint_asset_icon(ui, icon, &result.asset);
    let text_left = icon.right() + 12.0;
    let width = (right - text_left).max(30.0);
    let ticker = kit::galley(
        ui,
        &result.asset.display_ticker(),
        kit::size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let name = kit::galley(
        ui,
        &(input.subtitle)(&result.asset),
        kit::size::CAPTION,
        REGULAR,
        secondary,
        width,
    );
    let top = icon.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    draw(ui, pos2(text_left, top), &ticker);
    draw(ui, pos2(text_left, top + ticker.size().y + 1.0), &name);

    let amount = text_at(
        ui,
        pos2(left, rect.bottom() - 16.0),
        Align2::LEFT_BOTTOM,
        &f.signed_money(result.total_pnl),
        kit::size::SUBHEADLINE,
        MEDIUM,
        tone_or(tone, secondary),
    );
    text_at(
        ui,
        pos2(left, amount.top() - 2.0),
        Align2::LEFT_BOTTOM,
        &result
            .pnl_percent
            .map_or("\u{2014}".to_owned(), |p| f.signed_percent(p, 2)),
        26.0,
        SEMIBOLD,
        tone_or(tone, theme::text_primary()),
    );
}

fn history_card(ui: &mut egui::Ui, state: &mut OverviewState, input: &OverviewInput, height: f32) {
    panel(
        ui,
        height,
        Icon::ChartUptrend,
        "Portfolio Value",
        |header, ui| {
            history_range_control(header, &mut state.range);
            let loading =
                input.is_loading_initial || (input.is_loading_history && input.history.is_empty());
            portfolio_charts::show_history(
                ui,
                &HistoryInput {
                    snapshots: input.history,
                    current_value: input.total_value,
                    currency: input.formatter.currency,
                    range: state.range,
                    privacy: input.formatter.privacy,
                    is_loading: loading,
                    loading_message: if input.is_loading_initial {
                        "Loading market data\u{2026}"
                    } else {
                        "Building history\u{2026}"
                    },
                    now_ms: input.now_ms,
                    utc_offset_secs: input.utc_offset_secs,
                },
                height,
            );
        },
    );
}

fn history_range_control(ui: &mut egui::Ui, selected: &mut PortfolioHistoryRange) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(195.0, 27.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 7.0, theme::quaternary(0.6));
    painter.rect_stroke(
        rect,
        7.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    for (index, range) in PortfolioHistoryRange::ALL.into_iter().enumerate() {
        let segment = Rect::from_min_size(
            Pos2::new(rect.left() + 2.0 + index as f32 * 38.2, rect.top() + 2.0),
            Vec2::new(38.2, 23.0),
        );
        let response = ui
            .interact(
                segment,
                egui::Id::new(("portfolio-history-range", range.title())),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() {
            *selected = range;
        }
        let is_selected = *selected == range;
        if is_selected {
            painter.rect_filled(segment, 5.0, theme::chrome());
            painter.rect_stroke(
                segment,
                5.0,
                Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(segment, 5.0, theme::ink(12));
        }
        text_at(
            ui,
            segment.center(),
            Align2::CENTER_CENTER,
            range.title(),
            11.5,
            if is_selected { MEDIUM } else { REGULAR },
            if is_selected {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
    }
}

fn top_holdings_panel(ui: &mut Ui, input: &OverviewInput, actions: &mut Vec<OverviewAction>) {
    let f = input.formatter;
    let show_all = input.holdings.len() > TOP_COUNT;
    panel(
        ui,
        PANEL_CONTENT_HEIGHT,
        Icon::Layers,
        "Top Holdings",
        |header, ui| {
            if show_all {
                let link = header
                    .add(
                        egui::Label::new(kit::text(
                            "View All",
                            kit::size::SUBHEADLINE,
                            MEDIUM,
                            theme::SYSTEM_ACCENT,
                        ))
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(CursorIcon::PointingHand);
                if link.clicked() {
                    actions.push(OverviewAction::ShowHoldings);
                }
            }
            if input.holdings.is_empty() {
                portfolio_empty::compact(
                    ui,
                    ui.max_rect(),
                    portfolio_empty::Scene::Holdings,
                    "No Open Positions",
                    "Assets you hold appear here.",
                );
                return;
            }
            let width = ui.available_width();
            let top = ui.max_rect().top();
            let left = ui.max_rect().left();
            for (index, holding) in top_holdings(input.holdings).into_iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(left, top + index as f32 * (TOP_ROW_HEIGHT + TOP_ROW_GAP)),
                    vec2(width, TOP_ROW_HEIGHT),
                );
                top_holding_row(ui, row, holding, input, f, actions);
            }
        },
    );
}

fn top_holding_row(
    ui: &mut Ui,
    row: Rect,
    holding: &PortfolioHolding,
    input: &OverviewInput,
    f: PortfolioValueFormatter,
    actions: &mut Vec<OverviewAction>,
) {
    let response = ui
        .interact(
            row,
            egui::Id::new(("portfolio-top-holding", &holding.asset.key)),
            Sense::click(),
        )
        .on_hover_text("Shows this asset's transactions")
        .on_hover_cursor(CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(row, 8.0, theme::ink(18));
    }
    let secondary = caption_color();
    let icon = Rect::from_center_size(
        pos2(row.left() + 8.0 + 13.0, row.center().y),
        Vec2::splat(26.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(icon), |ui| {
        portfolio_table::asset_icon(ui, &holding.asset, 26.0);
    });

    // Right column: the value over an allocation bar and its percentage.
    let right = row.right() - 8.0;
    let value = line(
        ui,
        &holding.current_value.map_or("—".to_owned(), |v| f.money(v)),
        kit::size::SUBHEADLINE,
        REGULAR,
        theme::text_primary(),
    );
    let percent = line(
        ui,
        &f.percent(holding.allocation, 1),
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let right_height = value.size().y + 3.0 + percent.size().y;
    let right_top = row.center().y - right_height / 2.0;
    draw(ui, pos2(right - value.size().x, right_top), &value);
    let line_y = right_top + value.size().y + 3.0;
    draw(ui, pos2(right - percent.size().x, line_y), &percent);
    // The bar grows with the row, so a wide page does not leave it a stub far from its name.
    let bar_width = (row.width() * 0.22).clamp(64.0, 240.0);
    let bar = Rect::from_center_size(
        pos2(
            right - 44.0 - 6.0 - bar_width / 2.0,
            line_y + percent.size().y / 2.0,
        ),
        vec2(bar_width, 4.0),
    );
    ui.painter().rect_filled(bar, 2.0, theme::ink(28));
    let filled = Rect::from_min_size(
        bar.min,
        vec2(bar_width * holding.allocation.clamp(0.0, 1.0) as f32, 4.0),
    );
    ui.painter().rect_filled(filled, 2.0, theme::SYSTEM_ACCENT);

    let text_left = icon.right() + 10.0;
    let text_width = (bar.left() - 12.0 - text_left)
        .min(right - value.size().x - 12.0 - text_left)
        .max(30.0);
    let ticker = kit::galley(
        ui,
        &holding.asset.display_ticker(),
        kit::size::SUBHEADLINE,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let name = kit::galley(
        ui,
        &(input.subtitle)(&holding.asset),
        kit::size::CAPTION,
        REGULAR,
        secondary,
        text_width,
    );
    let top = row.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    draw(ui, pos2(text_left, top), &ticker);
    draw(ui, pos2(text_left, top + ticker.size().y + 1.0), &name);

    if response.clicked() {
        actions.push(OverviewAction::Select(holding.asset.key.clone()));
    }
    let spoken = format!(
        "{}: {} ({} of portfolio)",
        holding.asset.display_ticker(),
        holding.current_value.map_or("—".to_owned(), |v| f.money(v)),
        f.percent(holding.allocation, 1),
    );
    accessibility::label_as(response, egui::WidgetType::Button, spoken);
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::chrome())
        .stroke(Stroke::new(1.0, theme::separator().gamma_multiply(0.6)))
        .corner_radius(12.0)
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(30),
        })
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

// MARK: Statistics tab

pub struct StatisticsInput<'a> {
    pub holdings: &'a [PortfolioHolding],
    pub history: &'a [PortfolioSnapshot],
    /// The converted transactions, for fees and activity.
    pub transactions: &'a [PortfolioTransaction],
    pub formatter: PortfolioValueFormatter,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

/// "3 buys · 1 sell".
pub fn activity_caption(buys: usize, sells: usize) -> String {
    format!(
        "{buys} {} · {sells} {}",
        if buys == 1 { "buy" } else { "buys" },
        if sells == 1 { "sell" } else { "sells" }
    )
}

/// "Tracking since Jan 15, 2024" caption: how long ago, in words.
pub fn tracking_caption(since_ms: i64, now_ms: i64) -> String {
    let days = ((now_ms - since_ms) / 86_400_000).max(0);
    match days {
        0 => "today".to_owned(),
        1 => "1 day ago".to_owned(),
        2..=59 => format!("{days} days ago"),
        60..=729 => format!("{} months ago", days / 30),
        _ => format!("{} years ago", days / 365),
    }
}

/// The Win Rate panel's content height when it stands alone; beside the P&L panel it takes that
/// panel's height instead, so the two end level.
const WIN_CONTENT: f32 = 190.0;
const RANGE_HEIGHT: f32 = 96.0;
/// The P&L rows share their panel's height, kept within these.
const PNL_ROW_MIN: f32 = 30.0;
const PNL_ROW_MAX: f32 = 40.0;
/// Below this row width the P&L percentage is dropped, so the bars keep some room.
const PNL_COMPACT: f32 = 520.0;

pub fn show_statistics(ui: &mut egui::Ui, input: &StatisticsInput) {
    let f = input.formatter;
    let stats = PortfolioStatistics::new(input.holdings, input.history, input.transactions);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(PAGE_INSET))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(GRID_GAP, SECTION_GAP);
                    let width = ui.available_width();
                    statistics_hero(ui, f, &stats);
                    value_cards(ui, input, &stats);
                    range_strip(ui, input, &stats);
                    let rows_height = stats.asset_results.len().max(1) as f32 * PNL_ROW_MIN;
                    if width >= WIDE_MIN {
                        // Results on the left, the win rate beside them, both as tall as the
                        // longer of the bars and the ring needs.
                        let content = rows_height.max(WIN_CONTENT);
                        split(
                            ui,
                            [width - SIDE_WIDTH - SECTION_GAP, SIDE_WIDTH],
                            SECTION_GAP,
                            &mut (),
                            |ui, _| pnl_panel(ui, &stats.asset_results, f, content),
                            |ui, _| win_panel(ui, &stats, content),
                        );
                    } else {
                        win_panel(ui, &stats, WIN_CONTENT);
                        pnl_panel(ui, &stats.asset_results, f, rows_height.max(WIN_CONTENT));
                    }
                    activity_cards(ui, input, &stats);
                });
        });
}

fn value_cards(ui: &mut Ui, input: &StatisticsInput, stats: &PortfolioStatistics) {
    let f = input.formatter;
    let on = |ms: i64| date_text(ms + input.utc_offset_secs * 1_000);
    grid(
        ui,
        &[
            StatCard::new("Current Value", f.money(stats.total_value)).icon(Icon::Briefcase),
            StatCard::new("Cost Basis", f.money(stats.cost_basis)).icon(Icon::Banknote),
            StatCard::new(
                "Portfolio High",
                stats.high.map_or("—".to_owned(), |e| f.money(e.value)),
            )
            .caption(stats.high.map(|e| on(e.timestamp)))
            .placeholder(stats.high.is_none())
            .icon(Icon::ArrowUp),
            StatCard::new(
                "Portfolio Low",
                stats.low.map_or("—".to_owned(), |e| f.money(e.value)),
            )
            .caption(stats.low.map(|e| on(e.timestamp)))
            .placeholder(stats.low.is_none())
            .icon(Icon::ArrowDown),
        ],
    );
}

fn activity_cards(ui: &mut Ui, input: &StatisticsInput, stats: &PortfolioStatistics) {
    let f = input.formatter;
    let on = |ms: i64| date_text(ms + input.utc_offset_secs * 1_000);
    grid(
        ui,
        &[
            StatCard::new("Transactions", stats.transaction_count.to_string())
                .caption(Some(activity_caption(stats.buy_count, stats.sell_count)))
                .icon(Icon::Exchange),
            StatCard::new("Assets", stats.asset_count.to_string())
                .caption(Some(format!(
                    "{} in profit · {} at a loss",
                    stats.winners, stats.losers
                )))
                .icon(Icon::Layers),
            StatCard::new("Fees Paid", f.money(stats.fees_paid)).icon(Icon::Percent),
            StatCard::new(
                "Tracking Since",
                stats.first_transaction.map_or("—".to_owned(), on),
            )
            .caption(
                stats
                    .first_transaction
                    .map(|t| tracking_caption(t, input.now_ms)),
            )
            .placeholder(stats.first_transaction.is_none())
            .icon(Icon::List),
        ],
    );
}

// MARK: Statistics hero

/// How the profit divides between what is banked and what is still open, as shares of the two
/// magnitudes. `None` when there is neither.
pub fn composition_shares(unrealized: f64, realized: f64) -> Option<[f32; 2]> {
    let total = unrealized.abs() + realized.abs();
    (total.is_finite() && total > 0.0).then(|| {
        [
            (unrealized.abs() / total) as f32,
            (realized.abs() / total) as f32,
        ]
    })
}

/// Shares of the priced assets that are in profit, at a loss and level, summing to one. All
/// zero when nothing is priced.
pub fn win_shares(winners: usize, losers: usize, priced: usize) -> [f32; 3] {
    if priced == 0 {
        return [0.0; 3];
    }
    let share = |count: usize| count as f32 / priced as f32;
    let level = priced.saturating_sub(winners + losers);
    [share(winners), share(losers), share(level)]
}

/// Total P&L large, how it splits between realized and unrealized in the middle, and the last
/// day's move at the edge.
fn statistics_hero(ui: &mut Ui, f: PortfolioValueFormatter, stats: &PortfolioStatistics) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HERO_HEIGHT), Sense::hover());
    let total = stats.total_pnl();
    let tone = f.tone(Some(total));
    let tint = tone.map_or(theme::SYSTEM_ACCENT, tone_color);
    lift(ui, rect, 16.0);
    if stats.asset_count > 0 {
        glow(ui, rect, tint);
    }
    let inner = rect.shrink(HERO_PADDING);
    let secondary = caption_color();

    let chip = Rect::from_center_size(
        pos2(inner.left() + 11.0, inner.top() + 11.0),
        Vec2::splat(22.0),
    );
    ui.painter()
        .rect_filled(chip, 7.0, theme::SYSTEM_ACCENT.gamma_multiply(0.14));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(chip.center(), Vec2::splat(13.0)),
        Icon::ChartUptrend,
        theme::SYSTEM_ACCENT,
    );
    let name = text_at(
        ui,
        pos2(chip.right() + 9.0, chip.center().y),
        Align2::LEFT_CENTER,
        "Total P&L",
        15.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    let all_time = text_at(
        ui,
        pos2(name.right() + 6.0, chip.center().y),
        Align2::LEFT_CENTER,
        "\u{00B7} All time",
        15.0,
        REGULAR,
        secondary,
    );
    let balance = text_at(
        ui,
        pos2(inner.left(), inner.top() + 32.0),
        Align2::LEFT_TOP,
        &f.signed_money(total),
        40.0,
        SEMIBOLD,
        tone_or(tone, theme::text_primary()),
    );

    // The return on what was put in, in the figure's own colour.
    let caption_middle = inner.bottom() - 6.0;
    let mut left_end = all_time.right().max(balance.right());
    if stats.cost_basis > 0.0 {
        let percent = total / stats.cost_basis;
        let figure = text_at(
            ui,
            pos2(inner.left(), caption_middle),
            Align2::LEFT_CENTER,
            &f.signed_percent(percent, 2),
            kit::size::CAPTION,
            MEDIUM,
            tone_or(f.tone(Some(percent)), secondary),
        );
        let rest = text_at(
            ui,
            pos2(figure.right() + 5.0, caption_middle),
            Align2::LEFT_CENTER,
            "on cost basis",
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
        left_end = left_end.max(rest.right());
    } else {
        let none = text_at(
            ui,
            pos2(inner.left(), caption_middle),
            Align2::LEFT_CENTER,
            "No cost basis yet",
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
        left_end = left_end.max(none.right());
    }

    let middle = rect.center().y;
    let edge = change_block(
        ui,
        f,
        "24h",
        stats.day_change.map(|d| d.amount),
        stats.day_change.and_then(|d| d.percentage),
        pos2(inner.right(), middle),
    );
    ui.painter().vline(
        edge - 18.0,
        middle - 26.0..=middle + 26.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
    );
    let room = Rect::from_min_max(
        pos2(left_end + 36.0, inner.top() + 4.0),
        pos2(edge - 36.0, inner.bottom() - 4.0),
    );
    if room.width() >= 220.0 {
        composition(ui, f, room, stats);
    }
}

/// A bar of realized against unrealized profit, with a line each beneath it.
fn composition(ui: &Ui, f: PortfolioValueFormatter, room: Rect, stats: &PortfolioStatistics) {
    let secondary = caption_color();
    let room = Rect::from_min_size(room.min, vec2(room.width().min(420.0), room.height()));
    let top = room.center().y - 37.0;
    text_at(
        ui,
        pos2(room.left(), top),
        Align2::LEFT_TOP,
        "Breakdown",
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let open = (stats.unrealized_pnl, 1.0);
    let banked = (stats.realized_pnl, 0.55);
    let colour = |(amount, strength): (f64, f32)| {
        tone_option(f.tone(Some(amount))).map_or(theme::ink(80), |c| c.gamma_multiply(strength))
    };

    let bar = Rect::from_min_size(pos2(room.left(), top + 22.0), vec2(room.width(), 10.0));
    ui.painter().rect_filled(bar, 5.0, theme::ink(10));
    if let Some([open_share, banked_share]) =
        composition_shares(stats.unrealized_pnl, stats.realized_pnl)
    {
        let gap = if open_share > 0.0 && banked_share > 0.0 {
            3.0
        } else {
            0.0
        };
        let usable = bar.width() - gap;
        let open_width = usable * open_share;
        let paint = |left: f32, width: f32, colour: Color32| {
            if width > 0.0 {
                ui.painter().rect_filled(
                    Rect::from_min_size(pos2(left, bar.top()), vec2(width, bar.height())),
                    5.0,
                    colour,
                );
            }
        };
        paint(bar.left(), open_width, colour(open));
        paint(
            bar.left() + open_width + gap,
            usable * banked_share,
            colour(banked),
        );
    }

    for (index, (label, figure)) in [("Unrealized", open), ("Realized", banked)]
        .into_iter()
        .enumerate()
    {
        let y = top + 46.0 + index as f32 * 20.0;
        ui.painter()
            .circle_filled(pos2(room.left() + 4.0, y), 3.5, colour(figure));
        text_at(
            ui,
            pos2(room.left() + 14.0, y),
            Align2::LEFT_CENTER,
            label,
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
        text_at(
            ui,
            pos2(room.right(), y),
            Align2::RIGHT_CENTER,
            &f.signed_money(figure.0),
            kit::size::CAPTION,
            MEDIUM,
            tone_or(f.tone(Some(figure.0)), theme::text_primary()),
        );
    }
}

// MARK: Value range

/// Where the value stands between its lowest and highest point: a track filled to today's value.
fn range_strip(ui: &mut Ui, input: &StatisticsInput, stats: &PortfolioStatistics) {
    let f = input.formatter;
    let on = |ms: i64| date_text(ms + input.utc_offset_secs * 1_000);
    let rect = card_frame(ui, RANGE_HEIGHT, None);
    let (left, right) = (rect.left() + 16.0, rect.right() - 16.0);
    let secondary = caption_color();
    text_at(
        ui,
        pos2(left, rect.top() + 13.0),
        Align2::LEFT_TOP,
        "Value Range",
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );

    let ends = stats
        .high
        .zip(stats.low)
        .filter(|(high, low)| high.value > low.value);
    let position = ends.map(|(high, low)| {
        ((stats.total_value - low.value) / (high.value - low.value)).clamp(0.0, 1.0) as f32
    });
    if let Some(position) = position {
        text_at(
            ui,
            pos2(right, rect.top() + 13.0),
            Align2::RIGHT_TOP,
            &format!("{:.0}% of the way up", position * 100.0),
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
    }

    let y = rect.top() + 44.0;
    let track = Rect::from_min_max(pos2(left, y - 4.0), pos2(right, y + 4.0));
    ui.painter().rect_filled(track, 4.0, theme::ink(10));
    if let Some(position) = position {
        let colour = tone_option(f.tone(Some(stats.total_pnl()))).unwrap_or(theme::SYSTEM_ACCENT);
        // The marker keeps its whole width on the card at either end.
        let x = (track.left() + track.width() * position)
            .clamp(track.left() + 8.0, track.right() - 8.0);
        ui.painter().rect_filled(
            Rect::from_min_max(track.min, pos2(x.max(track.left() + 8.0), track.bottom())),
            4.0,
            colour.gamma_multiply(0.85),
        );
        ui.painter().circle_filled(pos2(x, y), 8.0, theme::chrome());
        ui.painter()
            .circle_filled(pos2(x, y), 5.5, theme::text_primary());
    }

    let with_date = rect.width() >= 480.0;
    for (extreme, label, anchor, align) in [
        (stats.low, "Low", left, Align2::LEFT_TOP),
        (stats.high, "High", right, Align2::RIGHT_TOP),
    ] {
        let money = extreme.map_or("—".to_owned(), |e| f.money(e.value));
        let note = match extreme {
            Some(e) if with_date => format!("{label} \u{00B7} {}", on(e.timestamp)),
            _ => label.to_owned(),
        };
        let top = rect.top() + 58.0;
        let figure = text_at(
            ui,
            pos2(anchor, top),
            align,
            &money,
            13.0,
            SEMIBOLD,
            if extreme.is_some() {
                theme::text_primary()
            } else {
                secondary
            },
        );
        text_at(
            ui,
            pos2(anchor, figure.bottom() + 1.0),
            align,
            &note,
            kit::size::CAPTION,
            REGULAR,
            secondary,
        );
    }
}

// MARK: Win rate

fn win_panel(ui: &mut Ui, stats: &PortfolioStatistics, content_height: f32) {
    panel(ui, content_height, Icon::Gauge, "Win Rate", |header, ui| {
        let priced = stats.asset_results.len();
        let area = ui.max_rect();
        if priced == 0 {
            portfolio_empty::compact(
                ui,
                area,
                portfolio_empty::Scene::Allocation,
                "No Win Rate Yet",
                "Priced holdings appear here.",
            );
            return;
        }
        header.label(kit::text(
            format!("{priced} priced"),
            kit::size::SUBHEADLINE,
            REGULAR,
            caption_color(),
        ));
        paint_win_rate(ui, area, stats);
    });
}

/// A ring of in profit, at a loss and level, with its legend beside it, the pair centred in
/// `area`.
fn paint_win_rate(ui: &Ui, area: Rect, stats: &PortfolioStatistics) {
    let secondary = caption_color();
    let priced = stats.asset_results.len();
    let shares = win_shares(stats.winners, stats.losers, priced);
    let level = priced.saturating_sub(stats.winners + stats.losers);
    let unpriced = stats.asset_count.saturating_sub(priced);

    let diameter = area.height().min(132.0).min(area.width() * 0.45);
    let legend_width = (area.width() - diameter - 28.0).clamp(120.0, 210.0);
    let block = diameter + 28.0 + legend_width;
    let left = (area.center().x - block / 2.0).max(area.left());
    let centre = pos2(left + diameter / 2.0, area.center().y);
    let width = 14.0;
    let radius = diameter / 2.0 - width / 2.0;

    ring(
        ui,
        centre,
        radius,
        width,
        &[
            (shares[0], theme::UP),
            (shares[1], theme::DOWN),
            (shares[2], theme::text_tertiary()),
        ],
    );
    let rate = line(
        ui,
        &format!("{:.0}%", shares[0] * 100.0),
        24.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    let caption = line(ui, "win rate", kit::size::CAPTION, REGULAR, secondary);
    let stack = rate.size().y + caption.size().y;
    let top = centre.y - stack / 2.0;
    draw(ui, pos2(centre.x - rate.size().x / 2.0, top), &rate);
    draw(
        ui,
        pos2(centre.x - caption.size().x / 2.0, top + rate.size().y),
        &caption,
    );

    let mut rows = vec![
        (theme::UP, "In profit", stats.winners),
        (theme::DOWN, "At a loss", stats.losers),
        (theme::text_tertiary(), "Break-even", level),
    ];
    if unpriced > 0 {
        rows.push((theme::ink(80), "No price", unpriced));
    }
    let row_height = 34.0;
    let x0 = left + diameter + 28.0;
    let first = area.center().y - rows.len() as f32 * row_height / 2.0;
    for (index, (colour, label, count)) in rows.iter().enumerate() {
        let middle = first + (index as f32 + 0.5) * row_height;
        ui.painter()
            .circle_filled(pos2(x0 + 5.0, middle), 4.5, *colour);
        text_at(
            ui,
            pos2(x0 + 18.0, middle),
            Align2::LEFT_CENTER,
            label,
            kit::size::SUBHEADLINE,
            REGULAR,
            secondary,
        );
        text_at(
            ui,
            pos2(x0 + legend_width, middle),
            Align2::RIGHT_CENTER,
            &count.to_string(),
            15.0,
            SEMIBOLD,
            theme::text_primary(),
        );
        if index + 1 < rows.len() {
            ui.painter().hline(
                x0..=x0 + legend_width,
                middle + row_height / 2.0,
                Stroke::new(1.0, theme::separator().gamma_multiply(0.5)),
            );
        }
    }
}

/// Segments of a ring, each a share of the whole in its colour, on a faint track. Several
/// segments are kept apart by a small gap.
fn ring(ui: &Ui, centre: Pos2, radius: f32, width: f32, segments: &[(f32, Color32)]) {
    use std::f32::consts::{FRAC_PI_2, TAU};
    ui.painter()
        .circle_stroke(centre, radius, Stroke::new(width, theme::ink(10)));
    let shown: Vec<&(f32, Color32)> = segments.iter().filter(|(share, _)| *share > 0.0).collect();
    if let [(_, colour)] = shown.as_slice() {
        ui.painter()
            .circle_stroke(centre, radius, Stroke::new(width, *colour));
        return;
    }
    let gap = 0.06;
    let mut start = -FRAC_PI_2;
    for (share, colour) in shown {
        let sweep = share * TAU;
        let (from, to) = (start + gap / 2.0, start + sweep - gap / 2.0);
        start += sweep;
        if to <= from {
            continue;
        }
        let steps = ((to - from) * radius / 2.0).ceil().max(2.0) as usize;
        let points = (0..=steps)
            .map(|step| {
                let angle = from + (to - from) * step as f32 / steps as f32;
                centre + vec2(angle.cos(), angle.sin()) * radius
            })
            .collect();
        ui.painter()
            .add(Shape::line(points, Stroke::new(width, *colour)));
    }
}

// MARK: P&L by asset

fn pnl_panel(
    ui: &mut Ui,
    results: &[AssetResult],
    f: PortfolioValueFormatter,
    content_height: f32,
) {
    panel(
        ui,
        content_height,
        Icon::BarChart,
        "P&L by Asset",
        |header, ui| {
            header.label(kit::text(
                format!("Total, in {}", f.currency.code()),
                kit::size::SUBHEADLINE,
                REGULAR,
                caption_color(),
            ));
            pnl_bars_view(ui, results, f, content_height);
        },
    );
}

/// Horizontal bars of each asset's total P&L around a zero line: gains grow right, losses left.
/// Rows share `content_height` between them, within a comfortable range.
fn pnl_bars_view(
    ui: &mut egui::Ui,
    results: &[AssetResult],
    f: PortfolioValueFormatter,
    content_height: f32,
) {
    if results.is_empty() {
        let area = ui.max_rect();
        portfolio_empty::compact(
            ui,
            area,
            portfolio_empty::Scene::Results,
            "No Results Yet",
            "Priced holdings appear here.",
        );
        return;
    }
    let (bars, zero) = pnl_bars(results);
    let secondary = caption_color();
    let row_height = (content_height / results.len() as f32).clamp(PNL_ROW_MIN, PNL_ROW_MAX);
    ui.spacing_mut().item_spacing.y = 0.0;
    let compact = ui.available_width() < PNL_COMPACT;
    for (result, bar) in results.iter().zip(&bars) {
        let (row, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::hover());
        if response.hovered() {
            ui.painter()
                .rect_filled(row.expand2(vec2(8.0, -1.0)), 8.0, theme::ink(10));
        }
        let middle = row.center().y;

        // Asset: icon and ticker in a 96 pt column.
        paint_asset_icon_at(ui, pos2(row.left() + 11.0, middle), &result.asset, 22.0);
        let ticker = kit::galley(
            ui,
            &result.asset.display_ticker(),
            kit::size::SUBHEADLINE,
            MEDIUM,
            theme::text_primary(),
            62.0,
        );
        draw(
            ui,
            pos2(row.left() + 30.0, middle - ticker.size().y / 2.0),
            &ticker,
        );

        // Figures, right-aligned at the row's end: the amount, then the percentage.
        let tone = f.tone(Some(result.total_pnl));
        let mut figures_left = row.right();
        if !compact {
            let percent = kit::galley(
                ui,
                &result
                    .pnl_percent
                    .map_or("—".to_owned(), |p| f.signed_percent(p, 2)),
                kit::size::SUBHEADLINE,
                REGULAR,
                tone_or(f.tone(result.pnl_percent), secondary),
                76.0,
            );
            draw(
                ui,
                pos2(
                    row.right() - percent.size().x,
                    middle - percent.size().y / 2.0,
                ),
                &percent,
            );
            figures_left -= 76.0 + 10.0;
        }
        let amount = kit::galley(
            ui,
            &f.signed_money(result.total_pnl),
            kit::size::SUBHEADLINE,
            REGULAR,
            tone_or(tone, theme::text_primary()),
            110.0,
        );
        draw(
            ui,
            pos2(
                figures_left - amount.size().x,
                middle - amount.size().y / 2.0,
            ),
            &amount,
        );

        // The bar track fills what the other columns leave.
        let track = Rect::from_min_max(
            pos2(row.left() + 96.0 + 14.0, middle - 5.0),
            pos2(figures_left - 110.0 - 14.0, middle + 5.0),
        );
        if track.width() > 20.0 {
            ui.painter().rect_filled(track, 5.0, theme::ink(8));
            let zero_x = track.left() + track.width() * zero as f32;
            // Neutral while values are hidden: red and green would give each sign away.
            let colour = tone_option(tone).map_or(theme::ink(80), |c| c.gamma_multiply(0.9));
            let from = track.left() + track.width() * bar.start as f32;
            let length = (track.width() * bar.length as f32).max(3.0);
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(from, track.top()), vec2(length, track.height())),
                5.0,
                colour,
            );
            ui.painter().vline(
                zero_x,
                egui::Rangef::new(track.top() - 3.0, track.bottom() + 3.0),
                Stroke::new(1.0, theme::ink(70)),
            );
        }
        accessibility::label_as(
            response,
            egui::WidgetType::Other,
            format!(
                "{}: {} ({})",
                result.asset.display_ticker(),
                f.signed_money(result.total_pnl),
                result
                    .pnl_percent
                    .map_or("—".to_owned(), |p| f.signed_percent(p, 2)),
            ),
        );
    }
}

/// An asset's icon centred at `centre`, painted without moving the layout cursor.
fn paint_asset_icon_at(ui: &Ui, centre: Pos2, asset: &PortfolioAsset, size: f32) {
    portfolio_table::paint_asset_icon(ui, Rect::from_center_size(centre, Vec2::splat(size)), asset);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::PortfolioCurrency;
    use std::collections::HashMap;

    fn holding(ticker: &str, value: Option<f64>) -> PortfolioHolding {
        PortfolioHolding {
            asset: PortfolioAsset {
                key: format!("binance:{ticker}"),
                symbol: format!("{ticker}/USDT"),
                name: ticker.to_owned(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            quantity: 1.0,
            cost_basis: 1.0,
            average_cost: 1.0,
            realized_pnl: 0.0,
            current_price: value,
            previous_day_price: None,
            current_value: value,
            unrealized_pnl: None,
            allocation: 0.0,
            is_price_stale: false,
        }
    }

    #[test]
    fn top_holdings_are_the_six_largest_with_unpriced_last() {
        let holdings: Vec<PortfolioHolding> = (1..=8)
            .map(|n| holding(&format!("A{n}"), Some(f64::from(n) * 10.0)))
            .chain([holding("NONE", None)])
            .collect();
        let top = top_holdings(&holdings);
        let tickers: Vec<String> = top.iter().map(|h| h.asset.display_ticker()).collect();
        assert_eq!(tickers, ["A8", "A7", "A6", "A5", "A4", "A3"]);
        let few = vec![holding("X", None), holding("Y", Some(1.0))];
        let ranked: Vec<String> = top_holdings(&few)
            .iter()
            .map(|h| h.asset.display_ticker())
            .collect();
        assert_eq!(ranked, ["Y", "X"]);
    }

    #[test]
    fn asset_counts_pluralise() {
        assert_eq!(asset_count_text(0), "0 assets");
        assert_eq!(asset_count_text(1), "1 asset");
        assert_eq!(asset_count_text(12), "12 assets");
    }

    #[test]
    fn the_change_caption_shows_direction_unless_hidden_or_flat() {
        let open = PortfolioValueFormatter::new(PortfolioCurrency::Usd, false);
        let hidden = PortfolioValueFormatter::new(PortfolioCurrency::Usd, true);
        assert_eq!(change_caption(&open, 120.0, 0.0321), "▲ 3.21%");
        assert_eq!(change_caption(&open, -120.0, -0.0321), "▼ 3.21%");
        assert_eq!(change_caption(&open, 0.0, 0.0), "0.00%");
        assert_eq!(
            change_caption(&hidden, 120.0, 0.0321),
            "3.21%",
            "no glyph to give the sign away"
        );
    }

    #[test]
    fn activity_and_tracking_captions_read_naturally() {
        assert_eq!(activity_caption(1, 1), "1 buy · 1 sell");
        assert_eq!(activity_caption(3, 0), "3 buys · 0 sells");
        let day = 86_400_000;
        assert_eq!(tracking_caption(0, 0), "today");
        assert_eq!(tracking_caption(0, day), "1 day ago");
        assert_eq!(tracking_caption(0, 30 * day), "30 days ago");
        assert_eq!(tracking_caption(0, 90 * day), "3 months ago");
        assert_eq!(tracking_caption(0, 800 * day), "2 years ago");
        assert_eq!(
            tracking_caption(10 * day, 0),
            "today",
            "a future first transaction is not negative"
        );
    }

    #[test]
    fn the_breakdown_splits_by_magnitude_and_is_empty_without_profit() {
        assert_eq!(composition_shares(300.0, 100.0), Some([0.75, 0.25]));
        assert_eq!(composition_shares(-100.0, 100.0), Some([0.5, 0.5]));
        assert_eq!(composition_shares(0.0, 0.0), None);
        assert_eq!(composition_shares(f64::NAN, 1.0), None);
    }

    #[test]
    fn win_shares_sum_to_one_and_count_the_level_ones() {
        assert_eq!(win_shares(0, 0, 0), [0.0; 3]);
        assert_eq!(win_shares(3, 3, 6), [0.5, 0.5, 0.0]);
        assert_eq!(win_shares(1, 1, 4), [0.25, 0.25, 0.5]);
        let shares = win_shares(5, 2, 9);
        assert!((shares.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_default_range_is_a_month() {
        assert_eq!(
            OverviewState::default().range,
            PortfolioHistoryRange::OneMonth
        );
    }
}
