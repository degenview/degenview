//! Paper trading on a chart card: the SELL / BUY pills in the card header, and the markers drawn
//! over the plot — one dashed rule and tag pill per open position and per working order, with a
//! close / cancel button, and the order pills draggable to a new price.
//!
//! Markers sit on the chart's own price scale (the one candles and drawings use), so a rule lies
//! exactly at its price when the chart zooms or the axis is dragged. Pure views returning actions;
//! the host applies them to the store.

use crate::i18n::tr;
use crate::model::chart::context::Target;
use crate::model::paper::format::{self as fmt, pnl_tone};
use crate::model::paper::ticket::rounded_to_tick;
use crate::model::paper::{
    PaperCurrency, PaperOrder, PaperOrderRole, PaperOrderSide, PaperOrderType, PaperPosition,
};
use crate::ui::accessibility;
use crate::ui::chart::card::PriceScale;
use crate::ui::paper::tables::{pnl_color, position_tint, side_tint};
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::sync::Arc;

pub const PILL_HEIGHT: f32 = 20.0;
const PILL_MARGIN: f32 = 8.0;

// MARK: Quick trade

/// "Sell $67,432.19" tooltips and accessibility text for the header pills.
pub fn quick_trade_help(side: PaperOrderSide, price_text: &str) -> String {
    tr!(
        "{side} {price} — opens a paper order ticket",
        side = side.label(),
        price = price_text
    )
}

/// Height of a header pill.
const QUICK_TRADE_HEIGHT: f32 = 24.0;
/// Gap between the two header pills.
const QUICK_TRADE_GAP: f32 = 4.0;

/// What a header pill says: the side and the price an order would open at.
fn quick_trade_text(side: PaperOrderSide, price_text: &str) -> String {
    format!("{}  {price_text}", side.badge_text())
}

/// The room SELL and BUY take in a card header for `price_text`, so the header can reserve exactly
/// that much instead of a guess a long price would overflow.
pub fn quick_trade_width(ui: &egui::Ui, price_text: &str) -> f32 {
    let pill = |side| {
        ui.painter()
            .layout_no_wrap(
                quick_trade_text(side, price_text),
                FontId::proportional(11.5),
                side_tint(side),
            )
            .size()
            .x
            + 18.0
    };
    pill(PaperOrderSide::Sell) + pill(PaperOrderSide::Buy) + QUICK_TRADE_GAP
}

/// SELL and BUY in a chart card's header: two tinted pills, each with the price an order would
/// open at. They open the order ticket; nothing is placed from here. Returns the side clicked.
///
/// The pills are laid out left to right in a block of their own, so SELL is always the left one,
/// even in the header's right-to-left end.
pub fn quick_trade_buttons(ui: &mut egui::Ui, price_text: &str) -> Option<PaperOrderSide> {
    let size = Vec2::new(quick_trade_width(ui, price_text), QUICK_TRADE_HEIGHT);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Min), |ui| {
        let mut clicked = None;
        ui.spacing_mut().item_spacing.x = QUICK_TRADE_GAP;
        for side in [PaperOrderSide::Sell, PaperOrderSide::Buy] {
            let tint = side_tint(side);
            let galley = ui.painter().layout_no_wrap(
                quick_trade_text(side, price_text),
                FontId::proportional(11.5),
                tint,
            );
            let size = Vec2::new(galley.size().x + 18.0, QUICK_TRADE_HEIGHT);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let alpha = if response.hovered() { 0.24 } else { 0.14 };
            ui.painter()
                .rect_filled(rect, 6.0, tint.gamma_multiply(alpha));
            ui.painter().galley(
                Pos2::new(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0),
                galley,
                tint,
            );
            // The header is drawn over the plot, whose crosshair cursor would show here.
            let response = accessibility::label(
                response,
                tr!(
                    "{side} at {price}",
                    side = side.badge_text(),
                    price = price_text
                ),
            )
            .on_hover_text(quick_trade_help(side, price_text))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                clicked = Some(side);
            }
        }
        clicked
    })
    .inner
}

// MARK: Pills

/// What one marker's tag pill says.
#[derive(Clone, Debug, PartialEq)]
pub struct Pill {
    pub tag: String,
    pub tint: Color32,
    pub text: String,
    pub pnl: Option<(String, Color32)>,
    pub close_label: String,
}

pub fn position_pill(position: &PaperPosition, pnl: f64, currency: PaperCurrency) -> Pill {
    let side = position.side();
    let size = fmt::quantity(position.quantity(), &position.instrument);
    let entry = fmt::price(position.average_entry_price, &position.instrument);
    Pill {
        tag: side.badge_text().to_owned(),
        tint: position_tint(side),
        text: format!("{size} @ {entry}"),
        pnl: Some((
            fmt::signed_money(pnl, currency),
            pnl_color(pnl_tone(pnl), theme::text_secondary()),
        )),
        close_label: if side.is_long() {
            tr!("Close paper long position, {size}", size = size)
        } else {
            tr!("Close paper short position, {size}", size = size)
        },
    }
}

/// Take profit green, stop loss and plain stops orange-red, resting limits in the accent colour.
pub fn order_tint(order: &PaperOrder) -> Color32 {
    match order.role {
        PaperOrderRole::TakeProfit => theme::UP,
        PaperOrderRole::StopLoss => theme::DOWN,
        PaperOrderRole::Entry => match order.kind {
            PaperOrderType::Stop | PaperOrderType::StopLimit => theme::REPLAY,
            _ => theme::ACCENT,
        },
    }
}

pub fn order_pill(order: &PaperOrder, displayed_price: f64) -> Pill {
    let instrument = &order.instrument;
    let price = fmt::price(displayed_price, instrument);
    let quantity = fmt::quantity(order.remaining_quantity(), instrument);
    Pill {
        tag: if order.role == PaperOrderRole::Entry {
            order.kind.badge_text().to_owned()
        } else {
            order.role.badge_text().to_owned()
        },
        tint: order_tint(order),
        text: format!("{} {quantity} @ {price}", order.side.badge_text()),
        pnl: None,
        close_label: tr!(
            "Cancel paper {side} {kind} order at {price}",
            side = order.side.label().to_lowercase(),
            kind = order.kind.label().to_lowercase(),
            price = price
        ),
    }
}

/// The price an order's marker sits at; a market order has none. A stop-limit rests at its stop
/// until it triggers, then at its limit.
pub fn marker_price(order: &PaperOrder) -> Option<f64> {
    match order.kind {
        PaperOrderType::Limit => order.limit_price,
        PaperOrderType::Stop => order.stop_price,
        PaperOrderType::StopLimit if order.stop_triggered => order.limit_price,
        PaperOrderType::StopLimit => order.stop_price,
        PaperOrderType::Market => None,
    }
}

/// Where a dragged order lands: the pointer's price on the chart scale, on the instrument's tick.
/// None for a price that would not be positive.
pub(in crate::ui) fn dropped_price(scale: &PriceScale, y: f32, order: &PaperOrder) -> Option<f64> {
    let price = rounded_to_tick(scale.price(y), &order.instrument);
    (price > 0.0 && price.is_finite()).then_some(price)
}

// MARK: Overlay

/// One position and its open P&L, as the host supplies them.
pub struct OverlayPosition<'a> {
    pub position: &'a PaperPosition,
    pub unrealized_pnl: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperOverlayAction {
    ClosePosition {
        account_id: u64,
        instrument_key: String,
    },
    /// An order marker was dropped on a new price.
    ModifyOrder {
        order_id: u64,
        price: f64,
    },
    CancelOrder(u64),
}

pub struct PaperOverlayInput<'a> {
    pub positions: &'a [OverlayPosition<'a>],
    pub orders: &'a [PaperOrder],
    pub currency: PaperCurrency,
}

/// A marker that was right-clicked, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerMenu {
    pub target: Target,
    pub at: Pos2,
}

/// Draws the markers over `plot` and returns what the user did to them, and the marker they
/// right-clicked, if any.
pub(in crate::ui) fn show(
    ui: &mut egui::Ui,
    plot: Rect,
    scale: &PriceScale,
    input: &PaperOverlayInput,
) -> (Vec<PaperOverlayAction>, Option<MarkerMenu>) {
    let mut actions = Vec::new();
    let mut menu = None;
    for entry in input.positions {
        let pill = position_pill(entry.position, entry.unrealized_pnl, input.currency);
        let y = scale.y(entry.position.average_entry_price);
        let id = egui::Id::new((
            "paper-position",
            entry.position.account_id,
            &entry.position.instrument.key,
        ));
        let result = marker(ui, plot, y, &pill, id, false);
        if let Some(at) = result.secondary {
            menu = Some(MarkerMenu {
                target: Target::Position {
                    key: entry.position.instrument.key.clone(),
                },
                at,
            });
        }
        if result.closed {
            actions.push(PaperOverlayAction::ClosePosition {
                account_id: entry.position.account_id,
                instrument_key: entry.position.instrument.key.clone(),
            });
        }
    }
    for order in input.orders {
        let Some(price) = marker_price(order) else {
            continue;
        };
        let id = egui::Id::new(("paper-order", order.id));
        let base_y = scale.y(price);
        // While dragging, the pill follows the pointer and shows the price it would land on.
        let dragging_y = ui.ctx().data(|data| data.get_temp::<f32>(id.with("y")));
        let shown_price = dragging_y
            .and_then(|y| dropped_price(scale, y, order))
            .unwrap_or(price);
        let y = dragging_y.unwrap_or(base_y);
        let pill = order_pill(order, shown_price);
        let result = marker(ui, plot, y, &pill, id, true);
        if let Some(at) = result.secondary {
            menu = Some(MarkerMenu {
                target: Target::Order { id: order.id },
                at,
            });
        }
        if result.closed {
            ui.ctx().data_mut(|data| data.remove::<f32>(id.with("y")));
            actions.push(PaperOverlayAction::CancelOrder(order.id));
            continue;
        }
        match result.drag {
            Drag::Moving(y) => ui.ctx().data_mut(|data| {
                data.insert_temp(id.with("y"), y);
            }),
            Drag::Dropped(y) => {
                ui.ctx().data_mut(|data| data.remove::<f32>(id.with("y")));
                if let Some(price) = dropped_price(scale, y, order) {
                    actions.push(PaperOverlayAction::ModifyOrder {
                        order_id: order.id,
                        price,
                    });
                }
            }
            Drag::None => {}
        }
    }
    (actions, menu)
}

pub(in crate::ui) enum Drag {
    None,
    Moving(f32),
    Dropped(f32),
}

pub(in crate::ui) struct MarkerResult {
    pub(in crate::ui) closed: bool,
    pub(in crate::ui) drag: Drag,
    /// Where the pill was right-clicked.
    pub(in crate::ui) secondary: Option<Pos2>,
    /// The pill's body was clicked (not dragged, not its close mark).
    pub(in crate::ui) clicked: bool,
}

/// A marker's pill, measured and placed.
struct PillLayout {
    y: f32,
    rect: Rect,
    close_rect: Rect,
    tag_width: f32,
    tag: Arc<egui::Galley>,
    text: Arc<egui::Galley>,
    pnl: Option<Arc<egui::Galley>>,
}

fn layout_pill(
    painter: &egui::Painter,
    plot: Rect,
    y: f32,
    pill: &Pill,
    text_color: Color32,
) -> PillLayout {
    let y = clamped_marker_y(plot, y);
    let tag = painter.layout_no_wrap(pill.tag.clone(), FontId::proportional(10.0), pill.tint);
    let text = painter.layout_no_wrap(pill.text.clone(), FontId::monospace(10.5), text_color);
    let pnl = pill
        .pnl
        .as_ref()
        .map(|(text, color)| painter.layout_no_wrap(text.clone(), FontId::monospace(10.5), *color));
    let tag_width = tag.size().x + 12.0;
    let body_width =
        text.size().x + pnl.as_ref().map_or(0.0, |pnl| pnl.size().x + 6.0) + 6.0 + 10.0 + 14.0;
    let width = tag_width + body_width;
    let rect = Rect::from_min_size(
        Pos2::new(plot.right() - PILL_MARGIN - width, y - PILL_HEIGHT / 2.0),
        Vec2::new(width, PILL_HEIGHT),
    );
    let close_rect = Rect::from_center_size(
        Pos2::new(rect.right() - 11.0, rect.center().y),
        Vec2::splat(PILL_HEIGHT - 2.0),
    );
    PillLayout {
        y,
        rect,
        close_rect,
        tag_width,
        tag,
        text,
        pnl,
    }
}

/// The dashed rule, left edge of the plot to the pill.
fn paint_rule(painter: &egui::Painter, plot: Rect, layout: &PillLayout, pill: &Pill) {
    let rule = Stroke::new(1.0, pill.tint.gamma_multiply(0.7));
    for dash in egui::Shape::dashed_line(
        &[
            Pos2::new(plot.left(), layout.y),
            Pos2::new(layout.rect.left(), layout.y),
        ],
        rule,
        4.0,
        3.0,
    ) {
        painter.add(dash);
    }
}

/// The pill itself; `hot` lights its close mark.
fn paint_pill(painter: &egui::Painter, layout: PillLayout, pill: &Pill, hot: bool) {
    let PillLayout {
        rect,
        close_rect,
        tag_width,
        tag,
        text,
        pnl,
        ..
    } = layout;
    painter.rect_filled(rect, 6.0, theme::chrome().gamma_multiply(0.94));
    let tag_rect = Rect::from_min_size(rect.min, Vec2::new(tag_width, PILL_HEIGHT));
    painter.rect_filled(tag_rect, 6.0, pill.tint.gamma_multiply(0.18));
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, pill.tint.gamma_multiply(0.8)),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        Pos2::new(tag_rect.left() + 6.0, rect.center().y - tag.size().y / 2.0),
        tag,
        pill.tint,
    );
    let mut x = tag_rect.right() + 7.0;
    let text_width = text.size().x;
    painter.galley(
        Pos2::new(x, rect.center().y - text.size().y / 2.0),
        text,
        Color32::WHITE,
    );
    x += text_width + 6.0;
    if let Some(pnl) = pnl {
        painter.galley(
            Pos2::new(x, rect.center().y - pnl.size().y / 2.0),
            pnl,
            Color32::WHITE,
        );
    }
    let alpha = if hot { 1.0 } else { 0.55 };
    painter.text(
        close_rect.center(),
        Align2::CENTER_CENTER,
        "✕",
        FontId::proportional(9.0),
        Color32::from_gray(170).gamma_multiply(alpha),
    );
}

/// A dashed rule across the plot at `y`, ending in the pill against the right edge.
pub(in crate::ui) fn marker(
    ui: &mut egui::Ui,
    plot: Rect,
    y: f32,
    pill: &Pill,
    id: egui::Id,
    draggable: bool,
) -> MarkerResult {
    let painter = ui.painter().with_clip_rect(plot);
    let layout = layout_pill(&painter, plot, y, pill, ui.visuals().text_color());
    let (y, rect, close_rect) = (layout.y, layout.rect, layout.close_rect);
    paint_rule(&painter, plot, &layout, pill);

    // Close first, so it takes the click over the draggable body.
    let close = ui.interact(close_rect, id.with("close"), Sense::click());
    let body = ui.interact(
        rect,
        id.with("body"),
        if draggable {
            Sense::click_and_drag()
        } else {
            Sense::click()
        },
    );
    paint_pill(&painter, layout, pill, body.hovered() || close.hovered());

    // The body is registered after the close mark and so lies over it: a click on the mark may be
    // reported to either, so both are read.
    let on_mark = body
        .interact_pointer_pos()
        .is_some_and(|at| close_rect.contains(at));
    let closed =
        close.clone().on_hover_text(&pill.close_label).clicked() || (body.clicked() && on_mark);
    let clicked = body.clicked() && !closed;
    let drag = if !draggable {
        Drag::None
    } else if body.dragged() {
        body.interact_pointer_pos().map_or(Drag::None, |pos| {
            Drag::Moving(clamped_marker_y(plot, pos.y))
        })
    } else if body.drag_stopped() {
        body.interact_pointer_pos().map_or(Drag::Dropped(y), |pos| {
            Drag::Dropped(clamped_marker_y(plot, pos.y))
        })
    } else {
        Drag::None
    };
    let secondary = body
        .secondary_clicked()
        .then(|| body.interact_pointer_pos())
        .flatten();
    if secondary.is_some() {
        // The card reads it next frame.
        ui.ctx().request_repaint();
    }
    if draggable && body.hovered() && !close.hovered() {
        body.on_hover_text(tr!("Drag to modify; release to commit"));
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    MarkerResult {
        closed,
        drag,
        secondary,
        clicked,
    }
}

/// Draws the markers as they rest, with nothing to point at: what a snapshot shows.
pub(in crate::ui) fn paint_markers(
    painter: &egui::Painter,
    plot: Rect,
    scale: &PriceScale,
    input: &PaperOverlayInput,
    text_color: Color32,
) {
    for entry in input.positions {
        let pill = position_pill(entry.position, entry.unrealized_pnl, input.currency);
        let y = scale.y(entry.position.average_entry_price);
        let layout = layout_pill(painter, plot, y, &pill, text_color);
        paint_rule(painter, plot, &layout, &pill);
        paint_pill(painter, layout, &pill, false);
    }
    for order in input.orders {
        let Some(price) = marker_price(order) else {
            continue;
        };
        let pill = order_pill(order, price);
        let layout = layout_pill(painter, plot, scale.y(price), &pill, text_color);
        paint_rule(painter, plot, &layout, &pill);
        paint_pill(painter, layout, &pill, false);
    }
}

fn clamped_marker_y(plot: Rect, y: f32) -> f32 {
    y.clamp(
        plot.top() + PILL_HEIGHT / 2.0,
        plot.bottom() - PILL_HEIGHT / 2.0,
    )
}

// MARK: Card state

/// What a chart card needs to draw its paper-trading controls, and what the user did to them.
/// The workspace fills the inputs each frame and drains `actions` and `quick_trade` afterwards;
/// the default (nothing enabled) draws nothing.
#[derive(Default)]
pub struct PaperCardState {
    /// In: show the SELL / BUY pills in the card header (a paper account exists).
    pub trading_enabled: bool,
    /// In: draw the position and order markers ("show trading on charts").
    pub show_markers: bool,
    /// In: this card's open positions with their unrealized P&L.
    pub positions: Vec<(PaperPosition, f64)>,
    /// In: this card's working orders.
    pub orders: Vec<PaperOrder>,
    pub currency: PaperCurrency,
    /// Out: marker actions from the last frame.
    pub actions: Vec<PaperOverlayAction>,
    /// Out: the marker right-clicked in the last frame.
    pub menu: Option<MarkerMenu>,
}

impl PaperCardState {
    /// Draws the markers when they are on and there is something to mark, and returns what the
    /// user did to them and the marker they right-clicked, if any.
    pub(in crate::ui) fn show_markers(
        &self,
        ui: &mut egui::Ui,
        plot: Rect,
        scale: &PriceScale,
    ) -> (Vec<PaperOverlayAction>, Option<MarkerMenu>) {
        if !self.show_markers || (self.positions.is_empty() && self.orders.is_empty()) {
            return (Vec::new(), None);
        }
        let positions: Vec<OverlayPosition> = self
            .positions
            .iter()
            .map(|(position, unrealized_pnl)| OverlayPosition {
                position,
                unrealized_pnl: *unrealized_pnl,
            })
            .collect();
        let input = PaperOverlayInput {
            positions: &positions,
            orders: &self.orders,
            currency: self.currency,
        };
        show(ui, plot, scale, &input)
    }

    /// What a snapshot of the chart shows of the account: the markers, when the chart shows them.
    pub(in crate::ui) fn paint_markers(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        scale: &PriceScale,
        text_color: Color32,
    ) {
        if !self.show_markers || (self.positions.is_empty() && self.orders.is_empty()) {
            return;
        }
        let positions: Vec<OverlayPosition> = self
            .positions
            .iter()
            .map(|(position, unrealized_pnl)| OverlayPosition {
                position,
                unrealized_pnl: *unrealized_pnl,
            })
            .collect();
        let input = PaperOverlayInput {
            positions: &positions,
            orders: &self.orders,
            currency: self.currency,
        };
        paint_markers(painter, plot, scale, &input, text_color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::PaperInstrument;

    fn instrument() -> PaperInstrument {
        let mut instrument = PaperInstrument::chart("BTCUSDT", "BTC/USDT", DataSource::Binance);
        instrument.tick_size = 0.01;
        instrument.quantity_increment = 0.001;
        instrument
    }

    fn order(kind: PaperOrderType, role: PaperOrderRole) -> PaperOrder {
        PaperOrder {
            id: 9,
            account_id: 1,
            instrument: instrument(),
            side: PaperOrderSide::Buy,
            kind,
            original_quantity: 1.5,
            filled_quantity: 0.5,
            average_fill_price: None,
            limit_price: Some(95.0),
            stop_price: Some(110.0),
            time_in_force: Default::default(),
            status: Default::default(),
            role,
            parent_order_id: None,
            oco_group_id: None,
            stop_triggered: false,
            reserved_margin: 0.0,
            rejection_reason: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn scale() -> PriceScale {
        PriceScale::linear(0.0, 200.0, 0.0, 200.0)
    }

    #[test]
    fn markers_sit_at_the_price_the_order_rests_at() {
        let mut stop_limit = order(PaperOrderType::StopLimit, PaperOrderRole::Entry);
        assert_eq!(
            marker_price(&order(PaperOrderType::Limit, PaperOrderRole::Entry)),
            Some(95.0)
        );
        assert_eq!(
            marker_price(&order(PaperOrderType::Stop, PaperOrderRole::Entry)),
            Some(110.0)
        );
        assert_eq!(
            marker_price(&stop_limit),
            Some(110.0),
            "waiting on its trigger"
        );
        stop_limit.stop_triggered = true;
        assert_eq!(
            marker_price(&stop_limit),
            Some(95.0),
            "triggered: now a limit"
        );
        assert_eq!(
            marker_price(&order(PaperOrderType::Market, PaperOrderRole::Entry)),
            None
        );
    }

    #[test]
    fn order_pills_name_the_role_and_remaining_size() {
        let limit = order_pill(&order(PaperOrderType::Limit, PaperOrderRole::Entry), 95.0);
        assert_eq!(limit.tag, "LMT");
        assert_eq!(limit.text, "BUY 1 @ 95");
        assert_eq!(limit.close_label, "Cancel paper buy limit order at 95");
        assert_eq!(limit.pnl, None);
        let take_profit = order_pill(
            &order(PaperOrderType::Limit, PaperOrderRole::TakeProfit),
            120.0,
        );
        assert_eq!(take_profit.tag, "TP");
        assert_eq!(take_profit.tint, theme::UP);
        let stop_loss = order_pill(&order(PaperOrderType::Stop, PaperOrderRole::StopLoss), 90.0);
        assert_eq!(
            (stop_loss.tag.as_str(), stop_loss.tint),
            ("SL", theme::DOWN)
        );
    }

    #[test]
    fn entry_stops_are_orange_and_limits_use_the_accent() {
        assert_eq!(
            order_tint(&order(PaperOrderType::Stop, PaperOrderRole::Entry)),
            theme::REPLAY
        );
        assert_eq!(
            order_tint(&order(PaperOrderType::StopLimit, PaperOrderRole::Entry)),
            theme::REPLAY
        );
        assert_eq!(
            order_tint(&order(PaperOrderType::Limit, PaperOrderRole::Entry)),
            theme::ACCENT
        );
    }

    #[test]
    fn position_pills_show_size_entry_and_pnl() {
        let position = PaperPosition {
            account_id: 1,
            instrument: instrument(),
            signed_quantity: -2.0,
            average_entry_price: 100.0,
            realized_gross_pnl: 0.0,
            commissions: 0.0,
            opened_at: 0,
            updated_at: 0,
        };
        let pill = position_pill(&position, 12.5, PaperCurrency::Usd);
        assert_eq!(pill.tag, "SHORT");
        assert_eq!(pill.tint, theme::DOWN);
        assert_eq!(pill.text, "2 @ 100");
        assert_eq!(pill.pnl, Some(("+$12.50".to_owned(), theme::UP)));
        assert_eq!(pill.close_label, "Close paper short position, 2");
        let loss = position_pill(&position, -3.0, PaperCurrency::Usd);
        assert_eq!(loss.pnl, Some(("-$3.00".to_owned(), theme::DOWN)));
    }

    #[test]
    fn a_dropped_order_lands_on_the_tick_under_the_pointer() {
        let order = order(PaperOrderType::Limit, PaperOrderRole::Entry);
        let scale = scale();
        assert_eq!(dropped_price(&scale, 100.0, &order), Some(100.0));
        assert_eq!(
            dropped_price(&scale, 99.996, &order),
            Some(100.0),
            "snapped to a tick"
        );
        assert_eq!(
            dropped_price(&scale, 200.0, &order),
            None,
            "the bottom edge is price zero"
        );
    }

    #[test]
    fn marker_drags_are_clamped_inside_the_plot() {
        let plot = Rect::from_min_max(Pos2::new(20.0, 40.0), Pos2::new(220.0, 240.0));
        assert_eq!(clamped_marker_y(plot, -100.0), 50.0);
        assert_eq!(clamped_marker_y(plot, 120.0), 120.0);
        assert_eq!(clamped_marker_y(plot, 900.0), 230.0);
    }

    #[test]
    fn an_unconfigured_card_draws_nothing() {
        let state = PaperCardState::default();
        assert!(!state.trading_enabled && !state.show_markers);
        assert!(state.positions.is_empty() && state.orders.is_empty() && state.actions.is_empty());
    }

    #[test]
    fn the_quick_trade_tooltip_names_the_side_and_price() {
        assert_eq!(
            quick_trade_help(PaperOrderSide::Buy, "67,432.19"),
            "Buy 67,432.19 — opens a paper order ticket"
        );
    }

    /// Draws the pills at the right end of a right-to-left row, the way a card header does, and
    /// returns the pills' block, the side clicked and the cursor the frame asked for.
    fn header_row(
        screen: &mut crate::ui::render_tests::Screen,
        price_text: &str,
        events: Vec<egui::Event>,
    ) -> (Rect, Option<PaperOrderSide>, egui::CursorIcon) {
        let mut block = Rect::NOTHING;
        let mut clicked = None;
        let output = screen.run(events, |ui| {
            crate::ui::theme::sync(ui.ctx());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let drawn = ui.scope(|ui| quick_trade_buttons(ui, price_text));
                block = drawn.response.rect;
                clicked = drawn.inner;
            });
        });
        (block, clicked, output.platform_output.cursor_icon)
    }

    fn press(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn sell_is_the_left_pill_and_buy_the_right_even_in_a_right_to_left_row() {
        let mut screen = crate::ui::render_tests::Screen::new(Vec2::new(600.0, 100.0));
        header_row(&mut screen, "64,348", vec![]);
        let (block, _, _) = header_row(&mut screen, "64,348", vec![]);
        for (x, side) in [
            (block.left() + 10.0, PaperOrderSide::Sell),
            (block.right() - 10.0, PaperOrderSide::Buy),
        ] {
            let at = Pos2::new(x, block.center().y);
            header_row(&mut screen, "64,348", vec![egui::Event::PointerMoved(at)]);
            header_row(&mut screen, "64,348", vec![press(at, true)]);
            let (_, clicked, _) = header_row(&mut screen, "64,348", vec![press(at, false)]);
            assert_eq!(clicked, Some(side), "a click at x = {x}");
        }
    }

    #[test]
    fn the_pills_show_a_pointing_hand() {
        let mut screen = crate::ui::render_tests::Screen::new(Vec2::new(600.0, 100.0));
        header_row(&mut screen, "64,348", vec![]);
        let (block, _, _) = header_row(&mut screen, "64,348", vec![]);
        let at = block.center();
        header_row(&mut screen, "64,348", vec![egui::Event::PointerMoved(at)]);
        let (_, _, cursor) = header_row(&mut screen, "64,348", vec![]);
        assert_eq!(cursor, egui::CursorIcon::PointingHand);
    }

    #[test]
    fn the_pills_take_exactly_the_width_they_report() {
        let mut screen = crate::ui::render_tests::Screen::new(Vec2::new(600.0, 100.0));
        let mut reported = 0.0;
        screen.run(vec![], |ui| {
            reported = quick_trade_width(ui, "64,348");
        });
        let (block, _, _) = header_row(&mut screen, "64,348", vec![]);
        assert!((block.width() - reported).abs() < 0.5);
        assert_eq!(block.height(), QUICK_TRADE_HEIGHT);
    }

    #[test]
    fn the_width_grows_with_the_price() {
        let mut screen = crate::ui::render_tests::Screen::new(Vec2::new(600.0, 100.0));
        let mut widths = Vec::new();
        screen.run(vec![], |ui| {
            for price in ["0.54", "64,348", "64,348.12", "1,234,567.891"] {
                widths.push(quick_trade_width(ui, price));
            }
        });
        assert!(
            widths.windows(2).all(|pair| pair[0] < pair[1]),
            "{widths:?}"
        );
    }
}
