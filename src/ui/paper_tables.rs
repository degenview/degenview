//! The paper-trading panel's tables: open positions, working orders, order history, closed
//! trades and the journal, plus the account metrics strip. Pure views over plain values that
//! return actions; the host owns the store and the engine.

// Transient: the paper-trading panel hosts these once the host is wired.
#![allow(dead_code)]

use crate::model::paper::{
    PaperClosedTrade, PaperCurrency, PaperInstrument, PaperJournalEntry, PaperOrder,
    PaperOrderEvent, PaperOrderRole, PaperOrderSide, PaperPosition, PaperPositionSide,
};
use crate::model::paper_format::{self as fmt, BadgeTone, BufferLevel, PnlTone, pnl_tone};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::paper_grid::{self as grid, Col, col};
use crate::ui::theme;
use eframe::egui::{self, Color32};

// MARK: Colours

pub fn pnl_color(tone: PnlTone, flat: Color32) -> Color32 {
    match tone {
        PnlTone::Profit => theme::UP,
        PnlTone::Loss => theme::DOWN,
        PnlTone::Flat => flat,
    }
}

pub fn side_tint(side: PaperOrderSide) -> Color32 {
    if side.is_buy() {
        theme::UP
    } else {
        theme::DOWN
    }
}

pub fn position_tint(side: PaperPositionSide) -> Color32 {
    if side.is_long() {
        theme::UP
    } else {
        theme::DOWN
    }
}

pub fn badge_color(tone: BadgeTone) -> Color32 {
    match tone {
        BadgeTone::Good => theme::UP,
        BadgeTone::Warning => theme::REPLAY,
        BadgeTone::Bad => theme::DOWN,
        BadgeTone::Neutral => theme::text_secondary(),
    }
}

fn buffer_color(level: BufferLevel, normal: Color32) -> Color32 {
    match level {
        BufferLevel::Critical => theme::DOWN,
        BufferLevel::Thin => theme::REPLAY,
        BufferLevel::Normal => normal,
    }
}

// MARK: Sections

/// The five sections of the Paper Trading panel.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PaperTab {
    #[default]
    Positions,
    Orders,
    History,
    Trades,
    Journal,
}

impl PaperTab {
    pub const ALL: [Self; 5] = [
        Self::Positions,
        Self::Orders,
        Self::History,
        Self::Trades,
        Self::Journal,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Positions => "Positions",
            Self::Orders => "Orders",
            Self::History => "History",
            Self::Trades => "Trades",
            Self::Journal => "Journal",
        }
    }
}

/// Everything a table needs to know about the account it shows.
#[derive(Clone, Copy)]
pub struct Context {
    pub currency: PaperCurrency,
    pub utc_offset_secs: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperTableAction {
    /// Close at market. Keyed by account and instrument key.
    ClosePosition {
        account_id: u64,
        instrument_key: String,
    },
    /// Close and open an equal position the other way, at market.
    ReversePosition {
        account_id: u64,
        instrument_key: String,
    },
    CancelOrder(u64),
}

// MARK: Metrics strip

#[cfg(test)]
pub use crate::trading::paper_stats::stats;

// MARK: Table plumbing

fn tone_of(tone: BadgeTone) -> kit::Tone {
    match tone {
        BadgeTone::Good => kit::Tone::Good,
        BadgeTone::Warning => kit::Tone::Warning,
        BadgeTone::Bad => kit::Tone::Bad,
        BadgeTone::Neutral => kit::Tone::Neutral,
    }
}

/// What a section shows when it has nothing to list: a glyph on a disc, a title and a line of
/// advice, centred above the footer. Compact on purpose; the panel is a short pane.
pub fn empty(ui: &mut egui::Ui, icon: Icon, title: &str, message: &str) {
    let height = grid::table_height(ui);
    empty_in(ui, height, icon, title, message, None);
}

/// `empty`, with an optional button; true when it was clicked. In a panel with a footer the
/// room is the table's; without one (no account) it is whatever is left.
pub fn empty_with_action(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    message: &str,
    action: Option<&str>,
) -> bool {
    let height = ui.available_height().max(120.0);
    empty_in(ui, height, icon, title, message, action)
}

fn empty_in(
    ui: &mut egui::Ui,
    height: f32,
    icon: Icon,
    title: &str,
    message: &str,
    action: Option<&str>,
) -> bool {
    let (area, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(area.shrink2(egui::vec2(20.0, 0.0)))
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    content.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    // Centre the stack vertically: disc, title, message (two lines at most) and the button.
    let stack = 42.0 + 10.0 + 16.0 + 3.0 + 30.0 + if action.is_some() { 10.0 + 22.0 } else { 0.0 };
    content.add_space(((area.height() - stack) / 2.0).max(0.0));
    empty_stack(&mut content, icon, title, message, action)
}

/// The stack itself, in whatever space the caller made: a glyph on a disc, a title, a line of
/// advice and an optional button (true when it was clicked).
fn empty_stack(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    message: &str,
    action: Option<&str>,
) -> bool {
    let (disc, _) = ui.allocate_exact_size(egui::Vec2::splat(42.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(disc.center(), 21.0, theme::quaternary(0.6));
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(disc.center(), egui::Vec2::splat(18.0)),
        icon,
        theme::text_secondary(),
    );
    ui.add_space(10.0);
    ui.label(kit::text(title, 12.5, kit::SEMIBOLD, theme::text_primary()));
    ui.add_space(3.0);
    ui.add(
        egui::Label::new(kit::text(
            message,
            11.0,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
    let Some(label) = action else {
        return false;
    };
    ui.add_space(10.0);
    small_button(ui, label, true).clicked()
}

/// `.controlSize(.small)` push button.
pub fn small_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    let color = if enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let galley = kit::galley(ui, label, 11.5, kit::MEDIUM, color, f32::INFINITY);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(galley.size().x + 20.0, 22.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let response = crate::ui::accessibility::label(response, label);
    let fill = kit::primary(
        match (
            enabled,
            response.is_pointer_button_down_on(),
            response.hovered(),
        ) {
            (false, _, _) => 0.05,
            (true, true, _) => 0.2,
            (true, false, true) => 0.14,
            _ => 0.1,
        },
    );
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, kit::primary(0.06)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response
}

/// The symbol cell: the market's artwork, then its ticker.
fn symbol_cell(ui: &mut egui::Ui, instrument: &PaperInstrument) {
    ui.spacing_mut().item_spacing.x = 8.0;
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(20.0), egui::Sense::hover());
    kit::chart_icon(ui, rect, &instrument.symbol, instrument.source, false);
    ui.add(
        egui::Label::new(kit::text(
            &instrument.symbol,
            12.5,
            kit::SEMIBOLD,
            theme::text_primary(),
        ))
        .truncate(),
    )
    .on_hover_text(&instrument.display_name);
}

fn chip(ui: &mut egui::Ui, text: &str, tint: Color32) {
    grid::tag(ui, text, tint);
}

// MARK: Positions

pub struct PositionsInput<'a> {
    pub positions: &'a [PaperPosition],
    pub ctx: Context,
    /// The live price a position is marked at, by position.
    pub mark: &'a dyn Fn(&PaperPosition) -> Option<f64>,
    pub unrealized_pnl: &'a dyn Fn(&PaperPosition) -> f64,
    /// Unrealized P&L as a fraction of what the position cost, if known.
    pub return_ratio: &'a dyn Fn(&PaperPosition) -> Option<f64>,
}

#[derive(Default)]
pub struct PositionsState {
    pending_reverse: Option<(u64, String)>,
}

pub fn positions_table(
    ui: &mut egui::Ui,
    state: &mut PositionsState,
    input: &PositionsInput,
) -> Vec<PaperTableAction> {
    let mut actions = Vec::new();
    if input.positions.is_empty() {
        empty(
            ui,
            Icon::ChartUptrend,
            "No open positions",
            "Use BUY or SELL in a chart's header to open one.",
        );
        return actions;
    }
    let currency = input.ctx.currency;
    const COLS: [Col; 9] = [
        col("Symbol", 130.0, 1.4, false),
        col("Side", 72.0, 0.0, false),
        col("Size", 90.0, 1.0, true),
        col("Entry", 100.0, 1.0, true),
        col("Mark", 100.0, 1.0, true),
        col("Unrealized", 110.0, 1.0, true),
        col("Return", 80.0, 0.8, true),
        col("Realized", 100.0, 1.0, true),
        col("", 104.0, 0.0, true),
    ];
    grid::show(
        ui,
        "paper-positions",
        &COLS,
        input.positions.len(),
        |ui, row, column| {
            let position = &input.positions[row];
            let weak = theme::text_secondary();
            let primary = theme::text_primary();
            match column {
                0 => symbol_cell(ui, &position.instrument),
                1 => chip(
                    ui,
                    position.side().badge_text(),
                    position_tint(position.side()),
                ),
                2 => grid::figure(
                    ui,
                    &fmt::quantity(position.quantity(), &position.instrument),
                    primary,
                ),
                3 => grid::figure(
                    ui,
                    &fmt::price(position.average_entry_price, &position.instrument),
                    primary,
                ),
                4 => {
                    let text = (input.mark)(position).map_or("—".to_owned(), |mark| {
                        fmt::price(mark, &position.instrument)
                    });
                    grid::figure(ui, &text, weak);
                }
                5 => {
                    let pnl = (input.unrealized_pnl)(position);
                    grid::figure_bold(
                        ui,
                        &fmt::signed_money(pnl, currency),
                        pnl_color(pnl_tone(pnl), weak),
                    );
                }
                6 => match (input.return_ratio)(position) {
                    Some(ratio) => grid::figure(
                        ui,
                        &fmt::signed_percent(ratio),
                        pnl_color(pnl_tone(ratio), weak),
                    ),
                    None => grid::figure(ui, "—", weak),
                },
                7 => {
                    let realized = position.realized_gross_pnl - position.commissions;
                    grid::figure(
                        ui,
                        &fmt::signed_money(realized, currency),
                        pnl_color(pnl_tone(realized), weak),
                    );
                }
                _ => {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let key = (position.account_id, &position.instrument.key);
                    let more = egui::Id::new(("paper-position-more", key));
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                    let response = kit::hover_button(
                        ui,
                        rect,
                        more,
                        Icon::Ellipsis,
                        "More actions",
                        true,
                        true,
                    );
                    kit::fitted_menu(
                        ui,
                        &response,
                        egui::RectAlign::BOTTOM_END,
                        &["Reverse\u{2026}"],
                        |ui| {
                            if kit::menu_item(
                                ui,
                                Icon::ArrowLeftRight,
                                "Reverse\u{2026}",
                                true,
                                false,
                            ) {
                                state.pending_reverse =
                                    Some((position.account_id, position.instrument.key.clone()));
                                ui.close();
                            }
                        },
                    );
                    if small_button(ui, "Close", true)
                        .on_hover_text("Close at market")
                        .clicked()
                    {
                        actions.push(PaperTableAction::ClosePosition {
                            account_id: position.account_id,
                            instrument_key: position.instrument.key.clone(),
                        });
                    }
                }
            }
        },
    );
    reverse_dialog(ui.ctx(), state, input.positions, &mut actions);
    actions
}

fn reverse_dialog(
    ctx: &egui::Context,
    state: &mut PositionsState,
    positions: &[PaperPosition],
    actions: &mut Vec<PaperTableAction>,
) {
    let Some((account_id, key)) = state.pending_reverse.clone() else {
        return;
    };
    let Some(position) = positions
        .iter()
        .find(|p| p.account_id == account_id && p.instrument.key == key)
    else {
        state.pending_reverse = None;
        return;
    };
    let title = format!("Reverse {} position?", position.instrument.symbol);
    let message = reverse_message(position.side());
    let result = kit::confirm(
        ctx,
        egui::Id::new("paper-reverse"),
        &Confirm {
            icon: Icon::ArrowLeftRight,
            title: &title,
            message: &message,
            destructive: "Reverse Position",
        },
    );
    if result == Some(ConfirmOutcome::Confirmed) {
        actions.push(PaperTableAction::ReversePosition {
            account_id,
            instrument_key: key.clone(),
        });
    }
    if result.is_some() {
        state.pending_reverse = None;
    }
}

/// "Closes the long and opens an equal short at market."
pub fn reverse_message(side: PaperPositionSide) -> String {
    format!(
        "Closes the {} and opens an equal {} at market.",
        side.label().to_lowercase(),
        if side.is_long() { "short" } else { "long" }
    )
}

// MARK: Working orders

/// TP in green, SL in red; entries have no role chip.
pub fn role_chip(role: PaperOrderRole) -> Option<(&'static str, Color32)> {
    match role {
        PaperOrderRole::Entry => None,
        PaperOrderRole::TakeProfit => Some((role.badge_text(), theme::UP)),
        PaperOrderRole::StopLoss => Some((role.badge_text(), theme::DOWN)),
    }
}

/// "0.5 / 2".
pub fn filled_over_quantity(order: &PaperOrder) -> String {
    format!(
        "{} / {}",
        fmt::quantity(order.filled_quantity, &order.instrument),
        fmt::quantity(order.original_quantity, &order.instrument)
    )
}

// MARK: Order history

/// Every order event, newest first.
pub fn newest_first_events(events: &[PaperOrderEvent]) -> Vec<&PaperOrderEvent> {
    let mut sorted: Vec<&PaperOrderEvent> = events.iter().collect();
    sorted.sort_by_key(|event| std::cmp::Reverse(event.timestamp));
    sorted
}

pub fn history_table(ui: &mut egui::Ui, events: &[PaperOrderEvent], ctx: Context) {
    if events.is_empty() {
        empty(
            ui,
            Icon::Clock,
            "No order history",
            "Placed, filled and canceled orders are logged here.",
        );
        return;
    }
    let sorted = newest_first_events(events);
    const COLS: [Col; 4] = [
        col("Time", 140.0, 0.0, false),
        col("Symbol", 130.0, 0.0, false),
        col("Event", 130.0, 0.0, false),
        col("Details", 200.0, 1.0, false),
    ];
    grid::show(
        ui,
        "paper-history",
        &COLS,
        sorted.len(),
        |ui, row, column| {
            let event = sorted[row];
            match column {
                0 => grid::text_cell(
                    ui,
                    &fmt::timestamp(event.timestamp, ctx.utc_offset_secs),
                    theme::text_secondary(),
                ),
                1 => symbol_cell(ui, &event.order.instrument),
                2 => {
                    kit::status_badge(ui, event.kind.label(), tone_of(event.kind.tone()));
                }
                _ => {
                    ui.add(
                        egui::Label::new(kit::text(
                            &event.message,
                            12.0,
                            kit::REGULAR,
                            theme::text_primary(),
                        ))
                        .truncate(),
                    )
                    .on_hover_text(format!("{}\nOrder {}", event.message, event.order_id));
                }
            }
        },
    );
}

// MARK: Closed trades

/// Closed trades, newest exit first.
pub fn newest_first_trades(trades: &[PaperClosedTrade]) -> Vec<&PaperClosedTrade> {
    let mut sorted: Vec<&PaperClosedTrade> = trades.iter().collect();
    sorted.sort_by_key(|trade| std::cmp::Reverse(trade.exit_timestamp));
    sorted
}

/// Share of closed trades with a positive net result, or `None` with no trades.
pub fn win_rate(trades: &[PaperClosedTrade]) -> Option<f64> {
    (!trades.is_empty()).then(|| {
        trades.iter().filter(|trade| trade.net_pnl() > 0.0).count() as f64 / trades.len() as f64
    })
}

pub fn net_pnl(trades: &[PaperClosedTrade]) -> f64 {
    trades.iter().map(PaperClosedTrade::net_pnl).sum()
}

pub fn trades_table(ui: &mut egui::Ui, trades: &[PaperClosedTrade], ctx: Context) {
    if trades.is_empty() {
        empty(
            ui,
            Icon::CheckmarkSeal,
            "No closed trades",
            "A trade appears here once a position is closed.",
        );
        return;
    }
    let sorted = newest_first_trades(trades);
    let currency = ctx.currency;
    const COLS: [Col; 9] = [
        col("Symbol", 130.0, 1.2, false),
        col("Side", 72.0, 0.0, false),
        col("Size", 90.0, 1.0, true),
        col("Entry", 100.0, 1.0, true),
        col("Exit", 100.0, 1.0, true),
        col("Gross", 100.0, 1.0, true),
        col("Fees", 80.0, 0.8, true),
        col("Net", 105.0, 1.0, true),
        col("Closed", 140.0, 0.0, true),
    ];
    grid::show(
        ui,
        "paper-trades",
        &COLS,
        sorted.len(),
        |ui, row, column| {
            let trade = sorted[row];
            let weak = theme::text_secondary();
            let primary = theme::text_primary();
            match column {
                0 => symbol_cell(ui, &trade.instrument),
                1 => chip(ui, trade.side.badge_text(), position_tint(trade.side)),
                2 => grid::figure(
                    ui,
                    &fmt::quantity(trade.quantity, &trade.instrument),
                    primary,
                ),
                3 => grid::figure(
                    ui,
                    &fmt::price(trade.entry_price, &trade.instrument),
                    primary,
                ),
                4 => grid::figure(
                    ui,
                    &fmt::price(trade.exit_price, &trade.instrument),
                    primary,
                ),
                5 => grid::figure(
                    ui,
                    &fmt::signed_money(trade.gross_pnl, currency),
                    pnl_color(pnl_tone(trade.gross_pnl), weak),
                ),
                6 => {
                    let text = if trade.commission == 0.0 {
                        "—".to_owned()
                    } else {
                        fmt::money(trade.commission, currency)
                    };
                    grid::figure(ui, &text, weak);
                }
                7 => {
                    let net = trade.net_pnl();
                    grid::figure_bold(
                        ui,
                        &fmt::signed_money(net, currency),
                        pnl_color(pnl_tone(net), weak),
                    );
                }
                _ => grid::text_cell(
                    ui,
                    &fmt::timestamp(trade.exit_timestamp, ctx.utc_offset_secs),
                    weak,
                ),
            }
        },
    );
}

// MARK: Journal

pub fn newest_first_journal(entries: &[PaperJournalEntry]) -> Vec<&PaperJournalEntry> {
    let mut sorted: Vec<&PaperJournalEntry> = entries.iter().collect();
    sorted.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp));
    sorted
}

pub fn journal_table(ui: &mut egui::Ui, entries: &[PaperJournalEntry], ctx: Context) {
    if entries.is_empty() {
        empty(
            ui,
            Icon::Books,
            "The journal is empty",
            "Fills, rejections and account changes are written here.",
        );
        return;
    }
    let sorted = newest_first_journal(entries);
    const COLS: [Col; 2] = [
        col("Time", 140.0, 0.0, false),
        col("Entry", 240.0, 1.0, false),
    ];
    grid::show(
        ui,
        "paper-journal",
        &COLS,
        sorted.len(),
        |ui, row, column| {
            let entry = sorted[row];
            if column == 0 {
                grid::text_cell(
                    ui,
                    &fmt::timestamp(entry.timestamp, ctx.utc_offset_secs),
                    theme::text_secondary(),
                );
            } else {
                ui.add(
                    egui::Label::new(kit::text(
                        &entry.message,
                        12.0,
                        kit::REGULAR,
                        theme::text_primary(),
                    ))
                    .truncate(),
                )
                .on_hover_text(&entry.message);
            }
        },
    );
}

// MARK: Panel footer

/// What the footer needs to count.
pub struct FooterFacts<'a> {
    pub positions: &'a [PaperPosition],
    pub unrealized_total: f64,
    pub working_orders: usize,
    pub events: usize,
    pub trades: &'a [PaperClosedTrade],
    pub journal_entries: usize,
    pub currency: PaperCurrency,
}

/// The one-line summary under a section's table ("2 open · Unrealized +$12.00").
pub fn footer_note(tab: PaperTab, facts: &FooterFacts) -> String {
    let count = |value: usize, singular: &str, plural: &str| {
        format!("{value} {}", if value == 1 { singular } else { plural })
    };
    let currency = facts.currency;
    match tab {
        PaperTab::Positions => {
            if facts.positions.is_empty() {
                "No open positions".to_owned()
            } else {
                format!(
                    "{} open · Unrealized {}",
                    facts.positions.len(),
                    fmt::signed_money(facts.unrealized_total, currency)
                )
            }
        }
        PaperTab::Orders => {
            if facts.working_orders == 0 {
                "No working orders".to_owned()
            } else {
                format!("{} working", facts.working_orders)
            }
        }
        PaperTab::History => {
            if facts.events == 0 {
                "No order events".to_owned()
            } else {
                count(facts.events, "event", "events")
            }
        }
        PaperTab::Trades => match win_rate(facts.trades) {
            None => "No closed trades".to_owned(),
            Some(rate) => format!(
                "{} · Win rate {}% · Net {}",
                count(facts.trades.len(), "trade", "trades"),
                (rate * 100.0).round() as i64,
                fmt::signed_money(net_pnl(facts.trades), currency)
            ),
        },
        PaperTab::Journal => {
            if facts.journal_entries == 0 {
                "No journal entries".to_owned()
            } else {
                count(facts.journal_entries, "entry", "entries")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{PaperOrderEventKind, PaperOrderStatus};

    fn instrument() -> PaperInstrument {
        let mut instrument = PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance);
        instrument.tick_size = 0.01;
        instrument.quantity_increment = 0.001;
        instrument
    }

    fn order(filled: f64, original: f64) -> PaperOrder {
        PaperOrder {
            id: 1,
            account_id: 1,
            instrument: instrument(),
            side: PaperOrderSide::Buy,
            kind: Default::default(),
            original_quantity: original,
            filled_quantity: filled,
            average_fill_price: None,
            limit_price: None,
            stop_price: None,
            time_in_force: Default::default(),
            status: PaperOrderStatus::Working,
            role: PaperOrderRole::Entry,
            parent_order_id: None,
            oco_group_id: None,
            stop_triggered: false,
            reserved_margin: 0.0,
            rejection_reason: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn trade(id: u64, exit: i64, gross: f64, commission: f64) -> PaperClosedTrade {
        PaperClosedTrade {
            id,
            account_id: 1,
            instrument: instrument(),
            side: PaperPositionSide::Long,
            entry_timestamp: 0,
            exit_timestamp: exit,
            entry_price: 100.0,
            exit_price: 110.0,
            quantity: 1.0,
            gross_pnl: gross,
            commission,
        }
    }

    #[test]
    fn tabs_have_the_short_swift_titles() {
        let titles: Vec<&str> = PaperTab::ALL.iter().map(|t| t.title()).collect();
        assert_eq!(
            titles,
            ["Positions", "Orders", "History", "Trades", "Journal"]
        );
        assert_eq!(PaperTab::default(), PaperTab::Positions);
    }

    #[test]
    fn working_orders_show_filled_over_quantity_at_the_instruments_precision() {
        assert_eq!(filled_over_quantity(&order(0.0, 2.0)), "0 / 2");
        assert_eq!(filled_over_quantity(&order(0.5, 2.0)), "0.5 / 2");
        assert_eq!(
            filled_over_quantity(&order(1.2345, 2.0)),
            "1.234 / 2",
            "half-even at three places"
        );
    }

    #[test]
    fn only_protective_orders_get_a_role_chip() {
        assert_eq!(role_chip(PaperOrderRole::Entry), None);
        assert_eq!(
            role_chip(PaperOrderRole::TakeProfit),
            Some(("TP", theme::UP))
        );
        assert_eq!(
            role_chip(PaperOrderRole::StopLoss),
            Some(("SL", theme::DOWN))
        );
    }

    #[test]
    fn reversing_says_what_it_will_do() {
        assert_eq!(
            reverse_message(PaperPositionSide::Long),
            "Closes the long and opens an equal short at market."
        );
        assert_eq!(
            reverse_message(PaperPositionSide::Short),
            "Closes the short and opens an equal long at market."
        );
    }

    #[test]
    fn history_trades_and_journal_list_newest_first() {
        let events: Vec<PaperOrderEvent> = [10, 30, 20]
            .iter()
            .map(|&timestamp| PaperOrderEvent {
                id: timestamp as u64,
                order_id: 1,
                account_id: 1,
                kind: PaperOrderEventKind::Placed,
                timestamp,
                message: String::new(),
                order: order(0.0, 1.0),
            })
            .collect();
        let stamps: Vec<i64> = newest_first_events(&events)
            .iter()
            .map(|e| e.timestamp)
            .collect();
        assert_eq!(stamps, [30, 20, 10]);

        let trades = vec![
            trade(1, 100, 10.0, 0.0),
            trade(2, 300, -5.0, 0.0),
            trade(3, 200, 1.0, 0.0),
        ];
        let exits: Vec<i64> = newest_first_trades(&trades)
            .iter()
            .map(|t| t.exit_timestamp)
            .collect();
        assert_eq!(exits, [300, 200, 100]);

        let journal: Vec<PaperJournalEntry> = [5, 9, 7]
            .iter()
            .map(|&timestamp| PaperJournalEntry {
                id: timestamp as u64,
                account_id: 1,
                timestamp,
                message: String::new(),
            })
            .collect();
        let at: Vec<i64> = newest_first_journal(&journal)
            .iter()
            .map(|e| e.timestamp)
            .collect();
        assert_eq!(at, [9, 7, 5]);
    }

    #[test]
    fn win_rate_and_net_result_count_after_fees() {
        let trades = vec![
            trade(1, 1, 10.0, 1.0), // +9
            trade(2, 2, 1.0, 2.0),  // -1 after fees
            trade(3, 3, -4.0, 0.0), // -4
            trade(4, 4, 6.0, 0.0),  // +6
        ];
        assert_eq!(win_rate(&trades), Some(0.5));
        assert_eq!(net_pnl(&trades), 10.0);
        assert_eq!(win_rate(&[]), None);
        assert_eq!(net_pnl(&[]), 0.0);
    }

    #[test]
    fn the_footer_summarises_each_section() {
        let usd = PaperCurrency::Usd;
        let positions = vec![PaperPosition {
            account_id: 1,
            instrument: instrument(),
            signed_quantity: 1.0,
            average_entry_price: 100.0,
            realized_gross_pnl: 0.0,
            commissions: 0.0,
            opened_at: 0,
            updated_at: 0,
        }];
        let trades = vec![
            trade(1, 1, 10.0, 0.0),
            trade(2, 2, -2.0, 0.0),
            trade(3, 3, 4.0, 0.0),
        ];
        let note = |tab, p: &[PaperPosition], orders, events, t: &[PaperClosedTrade], journal| {
            footer_note(
                tab,
                &FooterFacts {
                    positions: p,
                    unrealized_total: 12.0,
                    working_orders: orders,
                    events,
                    trades: t,
                    journal_entries: journal,
                    currency: usd,
                },
            )
        };
        assert_eq!(
            note(PaperTab::Positions, &[], 0, 0, &[], 0),
            "No open positions"
        );
        assert_eq!(
            note(PaperTab::Positions, &positions, 0, 0, &[], 0),
            "1 open · Unrealized +$12.00"
        );
        assert_eq!(
            note(PaperTab::Orders, &[], 0, 0, &[], 0),
            "No working orders"
        );
        assert_eq!(note(PaperTab::Orders, &[], 3, 0, &[], 0), "3 working");
        assert_eq!(
            note(PaperTab::History, &[], 0, 0, &[], 0),
            "No order events"
        );
        assert_eq!(note(PaperTab::History, &[], 0, 1, &[], 0), "1 event");
        assert_eq!(note(PaperTab::History, &[], 0, 8, &[], 0), "8 events");
        assert_eq!(
            note(PaperTab::Trades, &[], 0, 0, &[], 0),
            "No closed trades"
        );
        assert_eq!(
            note(PaperTab::Trades, &[], 0, 0, &trades, 0),
            "3 trades · Win rate 67% · Net +$12.00"
        );
        assert_eq!(
            note(PaperTab::Journal, &[], 0, 0, &[], 0),
            "No journal entries"
        );
        assert_eq!(note(PaperTab::Journal, &[], 0, 0, &[], 1), "1 entry");
        assert_eq!(note(PaperTab::Journal, &[], 0, 0, &[], 4), "4 entries");
    }

    #[test]
    fn the_colour_rules_map_tones_to_the_theme() {
        let flat = Color32::from_gray(1);
        assert_eq!(pnl_color(PnlTone::Profit, flat), theme::UP);
        assert_eq!(pnl_color(PnlTone::Loss, flat), theme::DOWN);
        assert_eq!(pnl_color(PnlTone::Flat, flat), flat);
        assert_eq!(side_tint(PaperOrderSide::Buy), theme::UP);
        assert_eq!(position_tint(PaperPositionSide::Short), theme::DOWN);
        assert_eq!(badge_color(BadgeTone::Good), theme::UP);
        assert_eq!(buffer_color(BufferLevel::Critical, flat), theme::DOWN);
        assert_eq!(buffer_color(BufferLevel::Normal, flat), flat);
    }
}
