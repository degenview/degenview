//! Stateful bridge between the paper-trading execution engine, SQLite, and the pure panel views.
//!
//! The execution engine deliberately records rejected orders before returning their error. This
//! host therefore persists and publishes every candidate engine state, including a domain-error
//! result, while still refusing all writes after an initial load failure.
#![allow(dead_code)]

use crate::model::paper::{
    PaperAccountMetrics, PaperAccountSettings, PaperCurrency, PaperInstrument, PaperOrderChanges,
    PaperOrderRequest, PaperOrderSide, PaperOrderType, PaperPosition, PaperQuote, PaperTimeInForce,
    PaperTradingError, PaperTradingSnapshot,
};
use crate::model::paper_engine::PaperTradingEngine;
use crate::model::paper_ticket::TicketDraft;
use crate::store::SharedStore;
use crate::ui::paper_account_sheet::{
    AccountSheetOutcome, PaperAccountSheetState, show as show_account_sheet,
};
use crate::ui::paper_header::{
    self, AccountChoice, PaperFooterAction, PaperFooterState, PaperHeaderAction, PaperHeaderInput,
    PaperHeaderState,
};
use crate::ui::paper_order_ticket::{
    PaperTicketAction, PaperTicketInput, PaperTicketState, show as show_ticket,
};
use crate::ui::paper_tables::{
    self, Context as TableContext, FooterFacts, OrdersInput, PaperTab, PaperTableAction,
    PositionsInput, PositionsState,
};
use eframe::egui;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum PaperHostError {
    Load(String),
    Engine(PaperTradingError),
    Persistence(String),
    StoreUnavailable,
}

impl PaperHostError {
    pub fn message(&self) -> String {
        match self {
            Self::Load(message) => format!("Could not load paper trading: {message}"),
            Self::Engine(error) => error.message(),
            Self::Persistence(message) => format!("Could not save paper trading: {message}"),
            Self::StoreUnavailable => "The paper-trading database is unavailable.".to_owned(),
        }
    }
}

impl fmt::Display for PaperHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for PaperHostError {}

pub struct PaperHost {
    store: SharedStore,
    engine: PaperTradingEngine,
    load_error: Option<PaperHostError>,
    last_error: Option<PaperHostError>,
}

impl PaperHost {
    pub fn load(store: SharedStore) -> Self {
        let loaded = store
            .lock()
            .map_err(|_| PaperHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .paper_trading()
                    .map_err(|error| PaperHostError::Load(error.to_string()))
            });
        match loaded {
            Ok(mut snapshot) => {
                // A quote restored from an earlier process is stale by definition. Keeping it
                // would put the previous session's price into a new order ticket until the first
                // live update arrives.
                snapshot.quotes.clear();
                Self {
                    store,
                    engine: PaperTradingEngine::new(snapshot),
                    load_error: None,
                    last_error: None,
                }
            }
            Err(error) => Self {
                store,
                engine: PaperTradingEngine::new(PaperTradingSnapshot::default()),
                load_error: Some(error.clone()),
                last_error: Some(error),
            },
        }
    }

    pub fn snapshot(&self) -> &PaperTradingSnapshot {
        self.engine.snapshot()
    }

    pub fn load_failed(&self) -> bool {
        self.load_error.is_some()
    }

    pub fn last_error(&self) -> Option<&PaperHostError> {
        self.last_error.as_ref()
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn metrics(&self, account_id: u64) -> PaperAccountMetrics {
        self.engine.metrics(account_id)
    }

    pub fn ensure_default_account(&mut self) -> Result<Option<u64>, PaperHostError> {
        if !self.snapshot().accounts.is_empty() {
            return Ok(None);
        }
        self.create_account(
            "Paper Trading",
            PaperCurrency::Usd,
            100_000.0,
            PaperAccountSettings::default(),
        )
        .map(Some)
    }

    /// The executable side used to mark a position: bid for a long and ask for a short, with the
    /// last trade as the fallback.
    pub fn mark(&self, position: &PaperPosition) -> Option<f64> {
        let quote = self.snapshot().quotes.get(&position.instrument.key)?;
        if position.signed_quantity >= 0.0 {
            quote.bid.or(quote.last)
        } else {
            quote.ask.or(quote.last)
        }
    }

    pub fn unrealized_pnl(&self, position: &PaperPosition) -> f64 {
        self.engine.unrealized_pnl(position)
    }

    pub fn return_ratio(&self, position: &PaperPosition) -> Option<f64> {
        let cost =
            position.average_entry_price * position.quantity() * position.instrument.point_value;
        (cost > 0.0 && self.mark(position).is_some()).then(|| self.unrealized_pnl(position) / cost)
    }

    pub fn create_account(
        &mut self,
        name: &str,
        currency: PaperCurrency,
        initial_balance: f64,
        settings: PaperAccountSettings,
    ) -> Result<u64, PaperHostError> {
        self.apply(|engine| engine.create_account(name, currency, initial_balance, settings))
    }

    pub fn select_account(&mut self, id: u64) -> Result<(), PaperHostError> {
        self.apply(|engine| engine.select_account(id))
    }

    pub fn update_settings(
        &mut self,
        account_id: u64,
        settings: PaperAccountSettings,
    ) -> Result<(), PaperHostError> {
        self.apply(move |engine| engine.update_settings(account_id, settings))
    }

    pub fn reset_account(
        &mut self,
        id: u64,
        currency: PaperCurrency,
        initial_balance: f64,
        settings: PaperAccountSettings,
    ) -> Result<(), PaperHostError> {
        self.apply(move |engine| engine.reset_account(id, currency, initial_balance, settings))
    }

    pub fn delete_account(&mut self, id: u64) -> Result<(), PaperHostError> {
        self.apply(|engine| engine.delete_account(id))
    }

    pub fn submit(&mut self, request: PaperOrderRequest) -> Result<u64, PaperHostError> {
        self.apply(move |engine| engine.submit(request))
    }

    pub fn cancel(&mut self, order_id: u64) -> Result<(), PaperHostError> {
        self.apply(|engine| engine.cancel(order_id))
    }

    pub fn modify(
        &mut self,
        order_id: u64,
        changes: PaperOrderChanges,
    ) -> Result<(), PaperHostError> {
        self.apply(move |engine| engine.modify(order_id, changes))
    }

    pub fn move_order(&mut self, order_id: u64, price: f64) -> Result<(), PaperHostError> {
        let order = self
            .snapshot()
            .orders
            .iter()
            .find(|order| order.id == order_id)
            .ok_or(PaperHostError::Engine(PaperTradingError::OrderNotFound))?;
        let changes = if order.kind == PaperOrderType::Stop
            || (order.kind == PaperOrderType::StopLimit && !order.stop_triggered)
        {
            PaperOrderChanges {
                stop_price: Some(price),
                ..PaperOrderChanges::default()
            }
        } else {
            PaperOrderChanges {
                limit_price: Some(price),
                ..PaperOrderChanges::default()
            }
        };
        self.modify(order_id, changes)
    }

    pub fn process_quote(&mut self, quote: PaperQuote) -> Result<(), PaperHostError> {
        if let Some(error) = &self.load_error {
            self.last_error = Some(error.clone());
            return Err(error.clone());
        }

        // Quotes are transient marks. Match the Swift actor: publish all of them in memory, but
        // rewrite the paper tables only when this quote could evaluate a working order.
        let is_outdated = self
            .snapshot()
            .quotes
            .get(&quote.instrument_key)
            .is_some_and(|previous| quote.timestamp < previous.timestamp);
        let should_persist = !is_outdated
            && self.snapshot().orders.iter().any(|order| {
                order.instrument.key == quote.instrument_key && order.status.is_working()
            });
        let mut candidate = PaperTradingEngine::new(self.engine.snapshot().clone());
        let result = candidate.process(quote);
        if should_persist {
            let saved = self
                .store
                .lock()
                .map_err(|_| PaperHostError::StoreUnavailable)
                .and_then(|store| {
                    store
                        .save_paper_trading(candidate.snapshot())
                        .map_err(|error| PaperHostError::Persistence(error.to_string()))
                });
            if let Err(error) = saved {
                self.last_error = Some(error.clone());
                return Err(error);
            }
        }

        self.engine = candidate;
        match result {
            Ok(()) => {
                self.last_error = None;
                Ok(())
            }
            Err(error) => {
                let error = PaperHostError::Engine(error);
                self.last_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub fn close_position(
        &mut self,
        account_id: u64,
        instrument_key: &str,
    ) -> Result<u64, PaperHostError> {
        self.market_against_position(account_id, instrument_key, 1.0)
    }

    pub fn reverse_position(
        &mut self,
        account_id: u64,
        instrument_key: &str,
    ) -> Result<u64, PaperHostError> {
        self.market_against_position(account_id, instrument_key, 2.0)
    }

    pub fn close_all_positions(&mut self, account_id: u64) -> Vec<Result<u64, PaperHostError>> {
        let keys: Vec<String> = self
            .snapshot()
            .positions
            .iter()
            .filter(|position| position.account_id == account_id)
            .map(|position| position.instrument.key.clone())
            .collect();
        keys.into_iter()
            .map(|key| self.close_position(account_id, &key))
            .collect()
    }

    pub fn cancel_all_orders(&mut self, account_id: u64) -> Vec<Result<(), PaperHostError>> {
        let ids: Vec<u64> = self
            .snapshot()
            .orders
            .iter()
            .filter(|order| order.account_id == account_id && order.status.is_working())
            .map(|order| order.id)
            .collect();
        ids.into_iter().map(|id| self.cancel(id)).collect()
    }

    fn market_against_position(
        &mut self,
        account_id: u64,
        instrument_key: &str,
        quantity_multiplier: f64,
    ) -> Result<u64, PaperHostError> {
        let position = self
            .snapshot()
            .positions
            .iter()
            .find(|position| {
                position.account_id == account_id && position.instrument.key == instrument_key
            })
            .cloned()
            .ok_or(PaperHostError::Engine(PaperTradingError::UnsupportedSymbol))?;
        let side = if position.signed_quantity > 0.0 {
            PaperOrderSide::Sell
        } else {
            PaperOrderSide::Buy
        };
        let quantity = position.quantity() * quantity_multiplier;
        self.submit(PaperOrderRequest {
            account_id,
            instrument: position.instrument,
            side,
            kind: PaperOrderType::Market,
            quantity,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        })
    }

    /// Runs an engine operation, persists whatever state it produced (including rejected-order
    /// audit rows), and publishes only after persistence succeeds.
    fn apply<T>(
        &mut self,
        operation: impl FnOnce(&mut PaperTradingEngine) -> Result<T, PaperTradingError>,
    ) -> Result<T, PaperHostError> {
        if let Some(error) = &self.load_error {
            self.last_error = Some(error.clone());
            return Err(error.clone());
        }

        let mut candidate = PaperTradingEngine::new(self.engine.snapshot().clone());
        let result = operation(&mut candidate);
        let saved = self
            .store
            .lock()
            .map_err(|_| PaperHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .save_paper_trading(candidate.snapshot())
                    .map_err(|error| PaperHostError::Persistence(error.to_string()))
            });
        if let Err(error) = saved {
            self.last_error = Some(error.clone());
            return Err(error);
        }

        self.engine = candidate;
        match result {
            Ok(value) => {
                self.last_error = None;
                Ok(value)
            }
            Err(error) => {
                let error = PaperHostError::Engine(error);
                self.last_error = Some(error.clone());
                Err(error)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperPanelAction {
    Close,
    Export,
}

/// Per-chart-window presentation state over the app-wide `PaperHost`.
pub struct PaperPanel {
    header: PaperHeaderState,
    footer: PaperFooterState,
    positions: PositionsState,
    account_sheet: Option<PaperAccountSheetState>,
    /// The account whose deletion is waiting for confirmation.
    pending_delete: Option<u64>,
    ticket: Option<PaperTicketState>,
    show_trading_on_charts: bool,
}

impl Default for PaperPanel {
    fn default() -> Self {
        Self {
            header: PaperHeaderState::default(),
            footer: PaperFooterState::default(),
            positions: PositionsState::default(),
            account_sheet: None,
            pending_delete: None,
            ticket: None,
            show_trading_on_charts: true,
        }
    }
}

impl PaperPanel {
    pub fn tab(&self) -> PaperTab {
        self.header.tab
    }

    pub fn show_trading_on_charts(&self) -> bool {
        self.show_trading_on_charts
    }

    pub fn open_ticket(
        &mut self,
        host: &mut PaperHost,
        instrument: PaperInstrument,
        reference_price: Option<f64>,
        side: PaperOrderSide,
    ) {
        host.clear_error();
        self.ticket = Some(PaperTicketState::new(TicketDraft::opening(
            instrument,
            reference_price,
            side,
        )));
    }

    /// The draft the open ticket holds, for tests of what opened it.
    #[cfg(test)]
    pub fn ticket_draft(&self) -> Option<&TicketDraft> {
        self.ticket.as_ref().map(|ticket| &ticket.draft)
    }

    /// Opens the ticket on a pending order (a limit or stop) at `price`, for the user to review
    /// and submit. Nothing is placed by opening it.
    pub fn open_pending_ticket(
        &mut self,
        host: &mut PaperHost,
        instrument: PaperInstrument,
        side: PaperOrderSide,
        kind: PaperOrderType,
        price: f64,
    ) {
        host.clear_error();
        self.ticket = Some(PaperTicketState::new(TicketDraft::pending(
            instrument, side, kind, price,
        )));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, host: &mut PaperHost) -> Vec<PaperPanelAction> {
        let mut panel_actions = Vec::new();
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        let _ = host.ensure_default_account();
        let snapshot = host.snapshot().clone();
        let account_id = snapshot.selected_account_id;
        let account = account_id.and_then(|id| snapshot.accounts.iter().find(|item| item.id == id));
        let currency = account.map_or(PaperCurrency::Usd, |account| account.base_currency);
        let accounts: Vec<AccountChoice> = snapshot
            .accounts
            .iter()
            .map(|account| AccountChoice {
                id: account.id,
                name: account.name.clone(),
            })
            .collect();
        let positions: Vec<_> = snapshot
            .positions
            .iter()
            .filter(|position| Some(position.account_id) == account_id)
            .cloned()
            .collect();
        let working_orders: Vec<_> = snapshot
            .orders
            .iter()
            .filter(|order| Some(order.account_id) == account_id && order.status.is_working())
            .cloned()
            .collect();
        let events: Vec<_> = snapshot
            .order_events
            .iter()
            .filter(|event| Some(event.account_id) == account_id)
            .cloned()
            .collect();
        let trades: Vec<_> = snapshot
            .closed_trades
            .iter()
            .filter(|trade| Some(trade.account_id) == account_id)
            .cloned()
            .collect();
        let journal: Vec<_> = snapshot
            .journal
            .iter()
            .filter(|entry| Some(entry.account_id) == account_id)
            .cloned()
            .collect();
        let metrics = account_id.map(|id| host.metrics(id));
        let unrealized_total = positions
            .iter()
            .map(|position| host.unrealized_pnl(position))
            .sum();

        let header_actions = paper_header::header(
            ui,
            &mut self.header,
            &PaperHeaderInput {
                accounts: &accounts,
                selected: account_id,
                currency: account.map(|account| account.base_currency),
                position_count: positions.len(),
                working_order_count: working_orders.len(),
                show_trading_on_charts: self.show_trading_on_charts,
            },
        );
        divider(ui);
        if let Some(error) = host.last_error() {
            error_banner(ui, &error.message());
        }

        let mut table_actions = Vec::new();
        let mut footer_actions = Vec::new();
        if account.is_none() {
            if paper_header::no_account_state(ui) {
                self.account_sheet = Some(PaperAccountSheetState::create());
            }
        } else {
            paper_tables::metrics_strip(ui, metrics.as_ref(), currency);
            divider(ui);
            let context = TableContext {
                currency,
                utc_offset_secs: 0,
            };
            match self.header.tab {
                PaperTab::Positions => {
                    table_actions = paper_tables::positions_table(
                        ui,
                        &mut self.positions,
                        &PositionsInput {
                            positions: &positions,
                            ctx: context,
                            mark: &|position| host.mark(position),
                            unrealized_pnl: &|position| host.unrealized_pnl(position),
                            return_ratio: &|position| host.return_ratio(position),
                        },
                    );
                }
                PaperTab::Orders => {
                    table_actions = paper_tables::orders_table(
                        ui,
                        &OrdersInput {
                            orders: &working_orders,
                            ctx: context,
                        },
                    );
                }
                PaperTab::History => paper_tables::history_table(ui, &events, context),
                PaperTab::Trades => paper_tables::trades_table(ui, &trades, context),
                PaperTab::Journal => paper_tables::journal_table(ui, &journal, context),
            }
            footer_actions = paper_header::footer(
                ui,
                &mut self.footer,
                self.header.tab,
                &FooterFacts {
                    positions: &positions,
                    unrealized_total,
                    working_orders: working_orders.len(),
                    events: events.len(),
                    trades: &trades,
                    journal_entries: journal.len(),
                    currency,
                },
            );
        }

        self.apply_header_actions(header_actions, host, account, &mut panel_actions);
        self.apply_table_actions(table_actions, host);
        self.apply_footer_actions(footer_actions, host, account_id, &mut panel_actions);
        self.show_account_sheet(ui.ctx(), host, account_id);
        self.show_delete_confirm(ui.ctx(), host);
        self.show_ticket(ui.ctx(), host, currency);
        panel_actions
    }

    fn apply_header_actions(
        &mut self,
        actions: Vec<PaperHeaderAction>,
        host: &mut PaperHost,
        account: Option<&crate::model::paper::PaperAccount>,
        panel_actions: &mut Vec<PaperPanelAction>,
    ) {
        for action in actions {
            match action {
                PaperHeaderAction::SelectAccount(id) => {
                    let _ = host.select_account(id);
                }
                PaperHeaderAction::NewAccount => {
                    self.account_sheet = Some(PaperAccountSheetState::create());
                }
                PaperHeaderAction::ExportData => panel_actions.push(PaperPanelAction::Export),
                PaperHeaderAction::ResetAccount => {
                    if let Some(account) = account {
                        self.account_sheet = Some(PaperAccountSheetState::reset(account));
                    }
                }
                PaperHeaderAction::DeleteAccount => {
                    self.pending_delete = account.map(|account| account.id);
                }
                PaperHeaderAction::ToggleShowOnCharts => {
                    self.show_trading_on_charts = !self.show_trading_on_charts;
                }
                PaperHeaderAction::Close => panel_actions.push(PaperPanelAction::Close),
            }
        }
    }

    fn apply_table_actions(&mut self, actions: Vec<PaperTableAction>, host: &mut PaperHost) {
        for action in actions {
            match action {
                PaperTableAction::ClosePosition {
                    account_id,
                    instrument_key,
                } => {
                    let _ = host.close_position(account_id, &instrument_key);
                }
                PaperTableAction::ReversePosition {
                    account_id,
                    instrument_key,
                } => {
                    let _ = host.reverse_position(account_id, &instrument_key);
                }
                PaperTableAction::CancelOrder(id) => {
                    let _ = host.cancel(id);
                }
            }
        }
    }

    fn apply_footer_actions(
        &mut self,
        actions: Vec<PaperFooterAction>,
        host: &mut PaperHost,
        account_id: Option<u64>,
        panel_actions: &mut Vec<PaperPanelAction>,
    ) {
        for action in actions {
            match (action, account_id) {
                (PaperFooterAction::CloseAllPositions, Some(id)) => {
                    let _ = host.close_all_positions(id);
                }
                (PaperFooterAction::CancelAllOrders, Some(id)) => {
                    let _ = host.cancel_all_orders(id);
                }
                (PaperFooterAction::Export, _) => panel_actions.push(PaperPanelAction::Export),
                _ => {}
            }
        }
    }

    fn show_account_sheet(
        &mut self,
        context: &egui::Context,
        host: &mut PaperHost,
        selected_account_id: Option<u64>,
    ) {
        let Some(sheet) = self.account_sheet.as_mut() else {
            return;
        };
        let is_reset = sheet.is_reset;
        match show_account_sheet(context, sheet) {
            Some(AccountSheetOutcome::Submit(result)) => {
                if is_reset {
                    if let Some(id) = selected_account_id {
                        let _ = host.reset_account(
                            id,
                            result.currency,
                            result.balance,
                            result.settings,
                        );
                    }
                } else {
                    let _ = host.create_account(
                        &result.name,
                        result.currency,
                        result.balance,
                        result.settings,
                    );
                }
                self.account_sheet = None;
            }
            Some(AccountSheetOutcome::Cancel) => self.account_sheet = None,
            None => {}
        }
    }

    fn show_delete_confirm(&mut self, context: &egui::Context, host: &mut PaperHost) {
        let Some(id) = self.pending_delete else {
            return;
        };
        let Some(name) = host
            .snapshot()
            .accounts
            .iter()
            .find(|account| account.id == id)
            .map(|account| account.name.clone())
        else {
            self.pending_delete = None;
            return;
        };
        match crate::ui::paper_header::delete_confirm(context, &name) {
            Some(crate::ui::kit::ConfirmOutcome::Confirmed) => {
                let _ = host.delete_account(id);
                self.pending_delete = None;
            }
            Some(crate::ui::kit::ConfirmOutcome::Cancelled) => self.pending_delete = None,
            None => {}
        }
    }

    fn show_ticket(
        &mut self,
        context: &egui::Context,
        host: &mut PaperHost,
        currency: PaperCurrency,
    ) {
        let Some(ticket) = self.ticket.as_mut() else {
            return;
        };
        let account_id = host.snapshot().selected_account_id;
        let instrument_key = ticket.draft.instrument.key.clone();
        let quote = host.snapshot().quotes.get(&instrument_key).cloned();
        let existing = host
            .snapshot()
            .positions
            .iter()
            .find(|position| {
                Some(position.account_id) == account_id && position.instrument.key == instrument_key
            })
            .map_or(0.0, |position| position.signed_quantity);
        let available = account_id.map(|id| host.metrics(id).available_funds);
        if let Some(account) = account_id.and_then(|id| {
            host.snapshot()
                .accounts
                .iter()
                .find(|account| account.id == id)
        }) {
            ticket.draft.leverage = account
                .settings
                .leverage
                .leverage(ticket.draft.instrument.asset_class);
            ticket.draft.commission = account.settings.commission;
        }
        ticket.refresh_market(
            quote.as_ref().and_then(|quote| quote.bid),
            quote.as_ref().and_then(|quote| quote.ask),
            quote.as_ref().and_then(|quote| quote.last),
            available,
            existing,
        );
        let error = host.last_error().map(PaperHostError::message);
        let actions = show_ticket(
            context,
            ticket,
            &PaperTicketInput {
                account_id,
                currency,
                error: error.as_deref(),
            },
        );
        for action in actions {
            match action {
                PaperTicketAction::Submit(request) => match host.submit(*request) {
                    Ok(_) => self.ticket = None,
                    Err(_) => {
                        if let Some(ticket) = &mut self.ticket {
                            ticket.finish();
                        }
                    }
                },
                PaperTicketAction::Cancel => {
                    host.clear_error();
                    self.ticket = None;
                }
            }
        }
    }
}

/// A one-pixel hairline across the panel.
fn divider(ui: &mut egui::Ui) {
    let (line, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(line, 0.0, crate::ui::theme::separator());
}

fn error_banner(ui: &mut egui::Ui, message: &str) {
    egui::Frame::new()
        .fill(crate::ui::theme::DOWN.gamma_multiply(0.14))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(message)
                    .small()
                    .color(crate::ui::theme::DOWN),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{PaperInstrument, PaperOrderStatus};
    use crate::store::Store;
    use std::sync::{Arc, Mutex};

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("open store")))
    }

    fn account(host: &mut PaperHost) -> u64 {
        host.create_account(
            "Main",
            PaperCurrency::Usd,
            100_000.0,
            PaperAccountSettings::default(),
        )
        .expect("account")
    }

    fn market_request(account_id: u64) -> PaperOrderRequest {
        PaperOrderRequest {
            account_id,
            instrument: PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance),
            side: PaperOrderSide::Buy,
            kind: PaperOrderType::Market,
            quantity: 1.0,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        }
    }

    #[test]
    fn account_mutations_are_durable() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let first = account(&mut host);
        let second = host
            .create_account(
                "Second",
                PaperCurrency::Eur,
                50_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        host.select_account(first).unwrap();
        host.delete_account(first).unwrap();

        assert_eq!(host.snapshot().selected_account_id, Some(second));
        assert_eq!(
            store.lock().unwrap().paper_trading().unwrap(),
            *host.snapshot()
        );
    }

    #[test]
    fn default_account_is_created_once_and_persisted() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = host.ensure_default_account().unwrap().expect("new account");
        assert_eq!(host.ensure_default_account().unwrap(), None);
        assert_eq!(host.snapshot().selected_account_id, Some(id));
        assert_eq!(host.snapshot().accounts[0].name, "Paper Trading");
        assert_eq!(
            store
                .lock()
                .unwrap()
                .paper_trading()
                .unwrap()
                .accounts
                .len(),
            1
        );
    }

    #[test]
    fn a_rejected_order_is_saved_before_its_error_is_returned() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        let error = host.submit(market_request(id)).expect_err("no quote");
        assert!(matches!(
            error,
            PaperHostError::Engine(PaperTradingError::NoMarketData)
        ));
        assert_eq!(host.snapshot().orders.len(), 1);
        assert_eq!(host.snapshot().orders[0].status, PaperOrderStatus::Rejected);
        assert_eq!(
            store.lock().unwrap().paper_trading().unwrap().orders[0].status,
            PaperOrderStatus::Rejected
        );
    }

    #[test]
    fn quotes_and_fills_persist_together() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: Some(49_999.0),
            ask: Some(50_001.0),
            last: Some(50_000.0),
            timestamp: now_ms(),
            is_market_open: true,
        })
        .unwrap();
        host.submit(market_request(id)).unwrap();

        assert_eq!(host.snapshot().positions.len(), 1);
        assert_eq!(host.snapshot().fills.len(), 1);
        assert_eq!(
            store.lock().unwrap().paper_trading().unwrap(),
            *host.snapshot()
        );
    }

    #[test]
    fn a_quote_that_crosses_a_working_order_persists_the_fill() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        let initial_timestamp = now_ms();
        host.process_quote(PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: None,
            ask: None,
            last: Some(50_000.0),
            timestamp: initial_timestamp,
            is_market_open: true,
        })
        .unwrap();
        let mut request = market_request(id);
        request.kind = PaperOrderType::Limit;
        request.limit_price = Some(40_000.0);
        host.submit(request).expect("working limit");
        assert!(host.snapshot().orders[0].status.is_working());

        host.process_quote(PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: None,
            ask: None,
            last: Some(39_000.0),
            timestamp: now_ms() + 1_000,
            is_market_open: true,
        })
        .expect("crossing quote");

        assert_eq!(host.snapshot().orders[0].status, PaperOrderStatus::Filled);
        assert_eq!(
            store.lock().unwrap().paper_trading().unwrap(),
            *host.snapshot()
        );
    }

    #[test]
    fn marks_use_the_executable_side_and_returns_use_entry_value() {
        let store = store();
        let mut host = PaperHost::load(store);
        let id = account(&mut host);
        host.process_quote(PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: Some(90.0),
            ask: Some(110.0),
            last: Some(100.0),
            timestamp: now_ms(),
            is_market_open: true,
        })
        .unwrap();
        let instrument = PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance);
        let long = PaperPosition {
            account_id: id,
            instrument: instrument.clone(),
            signed_quantity: 2.0,
            average_entry_price: 100.0,
            realized_gross_pnl: 0.0,
            commissions: 0.0,
            opened_at: now_ms(),
            updated_at: now_ms(),
        };
        let short = PaperPosition {
            signed_quantity: -2.0,
            ..long.clone()
        };

        assert_eq!(host.mark(&long), Some(90.0));
        assert_eq!(host.mark(&short), Some(110.0));
        assert_eq!(host.return_ratio(&long), Some(-0.1));
        assert_eq!(host.return_ratio(&short), Some(-0.1));
    }

    #[test]
    fn restored_quotes_are_discarded_as_stale() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        host.process_quote(PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: Some(49_999.0),
            ask: Some(50_001.0),
            last: Some(50_000.0),
            timestamp: now_ms(),
            is_market_open: true,
        })
        .unwrap();
        assert!(!host.snapshot().quotes.is_empty());
        assert!(
            store
                .lock()
                .unwrap()
                .paper_trading()
                .unwrap()
                .quotes
                .is_empty(),
            "a mark with no working order stays transient"
        );

        let restored = PaperHost::load(store);
        assert!(restored.snapshot().quotes.is_empty());
    }

    fn now_ms() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64)
    }
}
