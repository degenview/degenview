//! Stateful bridge between the paper-trading execution engine, SQLite, and the pure panel views.
//!
//! The execution engine deliberately records rejected orders before returning their error. This
//! host therefore persists and publishes every candidate engine state, including a domain-error
//! result, while still refusing all writes after an initial load failure.
#![allow(dead_code)]

use crate::model::paper::{
    PaperAccount, PaperAccountMetrics, PaperAccountSettings, PaperClosedTrade, PaperCurrency,
    PaperInstrument, PaperJournalEntry, PaperOrder, PaperOrderChanges, PaperOrderEvent,
    PaperOrderRequest, PaperOrderSide, PaperOrderType, PaperPosition, PaperQuote, PaperTimeInForce,
    PaperTradingError, PaperTradingSnapshot,
};
use crate::model::paper_engine::PaperTradingEngine;
use crate::model::paper_export;
use crate::model::paper_ticket::TicketDraft;
use crate::store::SharedStore;
use crate::store::sync::Elsewhere;
use crate::ui::icons::{self, Icon};
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
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum PaperHostError {
    Load(String),
    Engine(PaperTradingError),
    Persistence(String),
    /// The data disagrees with itself; nothing was saved or shown from it.
    Integrity(String),
    StoreUnavailable,
}

impl PaperHostError {
    pub fn message(&self) -> String {
        match self {
            Self::Load(message) => format!("Could not load paper trading: {message}"),
            Self::Engine(error) => error.message(),
            Self::Persistence(message) => format!("Could not save paper trading: {message}"),
            Self::Integrity(message) => {
                format!("Paper trading data failed a consistency check: {message}")
            }
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
    /// Accounts another Mac runs through iCloud Sync. They are shown but never filled here, so a
    /// working order is not filled twice; acting on one takes it over.
    elsewhere: HashMap<u64, Elsewhere>,
    /// Counts every change to what the panel lists (orders, positions, history, accounts), not a
    /// quote tick, so a view built from the snapshot is rebuilt only when it could differ.
    revision: u64,
}

/// Whether two snapshots hold the same accounts and history, ignoring the transient quotes.
fn same_ledger(a: &PaperTradingSnapshot, b: &PaperTradingSnapshot) -> bool {
    a.selected_account_id == b.selected_account_id
        && a.accounts == b.accounts
        && a.orders == b.orders
        && a.fills == b.fills
        && a.positions == b.positions
        && a.order_events == b.order_events
        && a.closed_trades == b.closed_trades
        && a.journal == b.journal
}

/// Refuses a snapshot whose rows disagree with each other.
fn check_integrity(snapshot: &PaperTradingSnapshot) -> Result<(), PaperHostError> {
    let issues = snapshot.integrity_issues();
    match issues.first() {
        None => Ok(()),
        Some(first) => Err(PaperHostError::Integrity(if issues.len() > 1 {
            format!("{first} (and {} more)", issues.len() - 1)
        } else {
            first.clone()
        })),
    }
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
        match loaded.and_then(|snapshot| check_integrity(&snapshot).map(|()| snapshot)) {
            Ok(mut snapshot) => {
                // A quote restored from an earlier process is stale by definition. Keeping it
                // would put the previous session's price into a new order ticket until the first
                // live update arrives.
                snapshot.quotes.clear();
                // Everything the ledger already holds reaches the journal, whether it predates
                // the journal, arrived from another Mac, or missed an earlier pass. Idempotent;
                // a failure is kept by the journal and does not stop paper trading.
                if let Ok(store) = store.lock() {
                    let _ = store.journal_reconcile_paper(&snapshot);
                }
                let mut host = Self {
                    store,
                    engine: PaperTradingEngine::new(snapshot),
                    load_error: None,
                    last_error: None,
                    elsewhere: HashMap::new(),
                    revision: 0,
                };
                host.refresh_ownership();
                host
            }
            Err(error) => Self {
                store,
                engine: PaperTradingEngine::new(PaperTradingSnapshot::default()),
                load_error: Some(error.clone()),
                last_error: Some(error),
                elsewhere: HashMap::new(),
                revision: 0,
            },
        }
    }

    fn refresh_ownership(&mut self) {
        self.elsewhere = self
            .store
            .lock()
            .ok()
            .and_then(|store| store.paper_elsewhere().ok())
            .unwrap_or_default();
    }

    /// Re-reads everything after iCloud Sync changed it underneath. Live quotes stay; an account
    /// that vanished is not selected any more. A snapshot that cannot be read leaves the one in
    /// memory alone.
    pub fn reload(&mut self) {
        if self.load_error.is_some() {
            return;
        }
        let loaded = self
            .store
            .lock()
            .map_err(|_| PaperHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .paper_trading()
                    .map_err(|error| PaperHostError::Load(error.to_string()))
            });
        match loaded.and_then(|snapshot| check_integrity(&snapshot).map(|()| snapshot)) {
            Ok(mut snapshot) => {
                snapshot.quotes = self.engine.snapshot().quotes.clone();
                let selected_exists = snapshot
                    .selected_account_id
                    .is_some_and(|id| snapshot.accounts.iter().any(|a| a.id == id));
                if !selected_exists {
                    snapshot.selected_account_id = snapshot.accounts.first().map(|a| a.id);
                }
                // Fills another Mac made reach the journal here: they came through the bundle,
                // not through this Mac's save.
                if let Ok(store) = self.store.lock() {
                    store.journal_ingest_paper(&snapshot);
                }
                self.engine = PaperTradingEngine::new(snapshot);
                self.revision = self.revision.wrapping_add(1);
                self.refresh_ownership();
            }
            Err(error) => self.last_error = Some(error),
        }
    }

    /// Whether this Mac has a working order or an open position on the instrument, in an account
    /// it runs. Quotes for anything else matter only to an open panel.
    pub fn tracks(&self, instrument_key: &str) -> bool {
        let snapshot = self.snapshot();
        snapshot.orders.iter().any(|order| {
            order.instrument.key == instrument_key
                && order.status.is_working()
                && !self.elsewhere.contains_key(&order.account_id)
        }) || snapshot.positions.iter().any(|position| {
            position.instrument.key == instrument_key
                && !self.elsewhere.contains_key(&position.account_id)
        })
    }

    /// Who runs the account when it is not this Mac.
    pub fn owned_elsewhere(&self, account_id: u64) -> Option<&Elsewhere> {
        self.elsewhere.get(&account_id)
    }

    pub fn snapshot(&self) -> &PaperTradingSnapshot {
        self.engine.snapshot()
    }

    /// Changes whenever the lists the panel shows could differ; not on a quote tick.
    pub fn revision(&self) -> u64 {
        self.revision
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
        self.apply_for(Some(account_id), move |engine| {
            engine.update_settings(account_id, settings)
        })
    }

    pub fn reset_account(
        &mut self,
        id: u64,
        currency: PaperCurrency,
        initial_balance: f64,
        settings: PaperAccountSettings,
    ) -> Result<(), PaperHostError> {
        self.apply_for(Some(id), move |engine| {
            engine.reset_account(id, currency, initial_balance, settings)
        })
    }

    pub fn delete_account(&mut self, id: u64) -> Result<(), PaperHostError> {
        self.apply_for(Some(id), |engine| engine.delete_account(id))
    }

    /// The journal's uid for a paper account (it changes when the account is reset).
    pub fn journal_account_uid(&self, id: u64) -> Option<String> {
        crate::model::journal::sources::paper::PaperSource::new(self.snapshot())
            .account_uid(id)
            .map(str::to_owned)
    }

    /// `reset_account`, and when `keep_history` is false the old account's Trade Journal history is
    /// removed too. The journal keeps it by default: a caller that cannot ask (MCP) passes true.
    pub fn reset_account_keeping(
        &mut self,
        id: u64,
        currency: PaperCurrency,
        initial_balance: f64,
        settings: PaperAccountSettings,
        keep_history: bool,
    ) -> Result<(), PaperHostError> {
        let old_uid = self.journal_account_uid(id);
        self.reset_account(id, currency, initial_balance, settings)?;
        if !keep_history {
            self.purge_journal(old_uid);
        }
        Ok(())
    }

    /// `delete_account`, with the same choice about the Trade Journal's copy of its history.
    pub fn delete_account_keeping(
        &mut self,
        id: u64,
        keep_history: bool,
    ) -> Result<(), PaperHostError> {
        let uid = self.journal_account_uid(id);
        self.delete_account(id)?;
        if !keep_history {
            self.purge_journal(uid);
        }
        Ok(())
    }

    fn purge_journal(&mut self, uid: Option<String>) {
        if let Some(uid) = uid
            && let Ok(store) = self.store.lock()
        {
            let _ = store.journal_purge_account(&uid);
        }
        // The journal changed under the paper ledger; whoever watches the revision refreshes.
        self.revision = self.revision.wrapping_add(1);
    }

    /// Runs every paper fill through the journal again (idempotent). What the journal's "retry"
    /// does after a failed pass.
    pub fn reconcile_journal(&self) -> Result<(), String> {
        let snapshot = self.snapshot().clone();
        let store = self
            .store
            .lock()
            .map_err(|_| "the store is unavailable".to_owned())?;
        store
            .journal_reconcile_paper(&snapshot)
            .map(drop)
            .map_err(|error| error.to_string())
    }

    pub fn submit(&mut self, request: PaperOrderRequest) -> Result<u64, PaperHostError> {
        let account = request.account_id;
        self.apply_for(Some(account), move |engine| engine.submit(request))
    }

    fn account_of_order(&self, order_id: u64) -> Option<u64> {
        self.snapshot()
            .orders
            .iter()
            .find(|order| order.id == order_id)
            .map(|order| order.account_id)
    }

    /// Whether `request` would be accepted right now, decided on a throwaway copy: nothing is
    /// saved, no order is recorded (not even a refused one) and no account changes hands.
    pub fn check_order(&self, request: PaperOrderRequest) -> Result<(), PaperHostError> {
        if let Some(error) = &self.load_error {
            return Err(error.clone());
        }
        let mut candidate = PaperTradingEngine::new(self.engine.snapshot().clone());
        candidate
            .submit(request)
            .map(|_| ())
            .map_err(PaperHostError::Engine)
    }

    pub fn cancel(&mut self, order_id: u64) -> Result<(), PaperHostError> {
        let account = self.account_of_order(order_id);
        self.apply_for(account, |engine| engine.cancel(order_id))
    }

    pub fn modify(
        &mut self,
        order_id: u64,
        changes: PaperOrderChanges,
    ) -> Result<(), PaperHostError> {
        let account = self.account_of_order(order_id);
        self.apply_for(account, move |engine| engine.modify(order_id, changes))
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
        let skip: HashSet<u64> = self.elsewhere.keys().copied().collect();
        let should_persist = !is_outdated
            && self.snapshot().orders.iter().any(|order| {
                order.instrument.key == quote.instrument_key
                    && order.status.is_working()
                    && !skip.contains(&order.account_id)
            });
        if !should_persist {
            // Nothing can fill: record the mark in place. Cloning the whole history to do that
            // cost more than everything else a tick does.
            let _ = self.engine.process_except(quote, &skip);
            self.last_error = None;
            return Ok(());
        }
        let mut candidate = PaperTradingEngine::new(self.engine.snapshot().clone());
        let result = candidate.process_except(quote, &skip);
        if let Err(error) = check_integrity(candidate.snapshot()) {
            self.last_error = Some(error.clone());
            return Err(error);
        }
        // A mark that filled, rejected or changed nothing is not worth a rewrite of every table
        // (80 ms at 5,000 fills): quotes are cleared on load anyway, so only a changed ledger is
        // saved.
        if !same_ledger(candidate.snapshot(), self.engine.snapshot()) {
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
            self.revision = self.revision.wrapping_add(1);
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

    /// Runs an engine operation not tied to one account.
    fn apply<T>(
        &mut self,
        operation: impl FnOnce(&mut PaperTradingEngine) -> Result<T, PaperTradingError>,
    ) -> Result<T, PaperHostError> {
        self.apply_for(None, operation)
    }

    /// Runs an engine operation, persists whatever state it produced (including rejected-order
    /// audit rows), and publishes only after persistence succeeds. A successful operation on an
    /// account another Mac runs takes that account over in the same save.
    fn apply_for<T>(
        &mut self,
        account: Option<u64>,
        operation: impl FnOnce(&mut PaperTradingEngine) -> Result<T, PaperTradingError>,
    ) -> Result<T, PaperHostError> {
        if let Some(error) = &self.load_error {
            self.last_error = Some(error.clone());
            return Err(error.clone());
        }

        let mut candidate = PaperTradingEngine::new(self.engine.snapshot().clone());
        let result = operation(&mut candidate);
        if let Err(error) = check_integrity(candidate.snapshot()) {
            self.last_error = Some(error.clone());
            return Err(error);
        }
        // Acting on a mirrored account takes it over, and so does a refused order the engine
        // recorded for audit: that already changed the account, which must not stay owned by the
        // Mac that did not see it. A refusal that changed nothing (an unknown order, an invalid
        // ticket) leaves the owner alone.
        let changed = candidate.snapshot() != self.engine.snapshot();
        let claim = account
            .filter(|id| self.elsewhere.contains_key(id))
            .filter(|_| result.is_ok() || changed);
        let saved = self
            .store
            .lock()
            .map_err(|_| PaperHostError::StoreUnavailable)
            .and_then(|store| {
                let persistence =
                    |error: rusqlite::Error| PaperHostError::Persistence(error.to_string());
                let previous = match claim {
                    Some(id) => {
                        let key = id.to_string();
                        let before = store.paper_owner(&key).map_err(persistence)?;
                        store.set_paper_owner(&key, None).map_err(persistence)?;
                        Some((key, before))
                    }
                    None => None,
                };
                store
                    .save_paper_trading(candidate.snapshot())
                    .map_err(|error| {
                        if let Some((key, Some(owner))) = &previous {
                            let _ = store.set_paper_owner(key, Some(owner));
                        }
                        persistence(error)
                    })
            });
        if let Err(error) = saved {
            self.last_error = Some(error.clone());
            return Err(error);
        }
        if let Some(id) = claim {
            self.elsewhere.remove(&id);
        }

        self.engine = candidate;
        self.revision = self.revision.wrapping_add(1);
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
}

/// What a finished export tells the panel: the folder it wrote to, or why it did not.
type ExportOutcome = Result<Option<(std::path::PathBuf, String)>, String>;

/// What the panel lists for the selected account.
struct PanelView {
    revision: u64,
    account_id: Option<u64>,
    account: Option<PaperAccount>,
    accounts: Vec<AccountChoice>,
    positions: Vec<PaperPosition>,
    working_orders: Vec<PaperOrder>,
    events: Vec<PaperOrderEvent>,
    trades: Vec<PaperClosedTrade>,
    journal: Vec<PaperJournalEntry>,
}

impl PanelView {
    fn build(host: &PaperHost) -> Self {
        let snapshot = host.snapshot();
        let account_id = snapshot.selected_account_id;
        let mine = |owner: u64| Some(owner) == account_id;
        Self {
            revision: host.revision(),
            account_id,
            account: account_id
                .and_then(|id| snapshot.accounts.iter().find(|item| item.id == id))
                .cloned(),
            accounts: snapshot
                .accounts
                .iter()
                .map(|account| AccountChoice {
                    id: account.id,
                    name: account.name.clone(),
                })
                .collect(),
            positions: snapshot
                .positions
                .iter()
                .filter(|position| mine(position.account_id))
                .cloned()
                .collect(),
            working_orders: snapshot
                .orders
                .iter()
                .filter(|order| mine(order.account_id) && order.status.is_working())
                .cloned()
                .collect(),
            events: snapshot
                .order_events
                .iter()
                .filter(|event| mine(event.account_id))
                .cloned()
                .collect(),
            trades: snapshot
                .closed_trades
                .iter()
                .filter(|trade| mine(trade.account_id))
                .cloned()
                .collect(),
            journal: snapshot
                .journal
                .iter()
                .filter(|entry| mine(entry.account_id))
                .cloned()
                .collect(),
        }
    }
}

/// A line under the header that says what an action did (or why it did nothing), until it is
/// dismissed or, for good news and plain facts, a few seconds pass.
struct PanelNotice {
    text: String,
    is_error: bool,
    since: std::time::Instant,
}

impl PanelNotice {
    const LINGER: std::time::Duration = std::time::Duration::from_secs(8);

    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
            since: std::time::Instant::now(),
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::info(text)
        }
    }

    fn expired(&self) -> bool {
        !self.is_error && self.since.elapsed() >= Self::LINGER
    }
}

/// Per-chart-window presentation state over the app-wide `PaperHost`.
pub struct PaperPanel {
    header: PaperHeaderState,
    footer: PaperFooterState,
    positions: PositionsState,
    account_sheet: Option<PaperAccountSheetState>,
    /// The account whose deletion is waiting for confirmation.
    pending_delete: Option<u64>,
    /// The delete dialog's switch: keep the account's history in the Trade Journal.
    keep_history_on_delete: bool,
    ticket: Option<PaperTicketState>,
    show_trading_on_charts: bool,
    notice: Option<PanelNotice>,
    /// The lists the panel shows for the selected account, rebuilt only when the host's data
    /// changed. Filtering and cloning the whole history every frame cost far more than drawing it.
    cached_view: Option<std::sync::Arc<PanelView>>,
    /// The folder dialog and the writing behind it, on a worker thread.
    export_job: Option<std::sync::mpsc::Receiver<ExportOutcome>>,
    /// Set by the header menu and the footer; taken at the end of the frame.
    export_requested: bool,
}

impl Default for PaperPanel {
    fn default() -> Self {
        Self {
            header: PaperHeaderState::default(),
            footer: PaperFooterState::default(),
            positions: PositionsState::default(),
            account_sheet: None,
            pending_delete: None,
            keep_history_on_delete: true,
            ticket: None,
            show_trading_on_charts: true,
            notice: None,
            cached_view: None,
            export_job: None,
            export_requested: false,
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
        let view = self.view(host);
        let account_id = view.account_id;
        let account = view.account.as_ref();
        let currency = account.map_or(PaperCurrency::Usd, |account| account.base_currency);
        let accounts = &view.accounts;
        let positions = &view.positions;
        let working_orders = &view.working_orders;
        let events = &view.events;
        let trades = &view.trades;
        let journal = &view.journal;
        let metrics = account_id.map(|id| host.metrics(id));
        let unrealized_total = positions
            .iter()
            .map(|position| host.unrealized_pnl(position))
            .sum();

        let header_actions = paper_header::header(
            ui,
            &mut self.header,
            &PaperHeaderInput {
                accounts,
                selected: account_id,
                currency: account.map(|account| account.base_currency),
                position_count: positions.len(),
                working_order_count: working_orders.len(),
                show_trading_on_charts: self.show_trading_on_charts,
            },
        );
        divider(ui);
        self.poll_export(ui.ctx());
        self.show_notice(ui);
        if let Some(elsewhere) = account_id.and_then(|id| host.owned_elsewhere(id)) {
            owner_note(ui, elsewhere);
        }
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
                // The offset now, as the portfolio tables use: rows from the other side of a
                // daylight-saving change show an hour off.
                utc_offset_secs: crate::service::timezone::local_utc_offset_secs(
                    crate::model::watchlist::now_ms(),
                ),
            };
            match self.header.tab {
                PaperTab::Positions => {
                    table_actions = paper_tables::positions_table(
                        ui,
                        &mut self.positions,
                        &PositionsInput {
                            positions,
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
                            orders: working_orders,
                            ctx: context,
                        },
                    );
                }
                PaperTab::History => paper_tables::history_table(ui, events, context),
                PaperTab::Trades => paper_tables::trades_table(ui, trades, context),
                PaperTab::Journal => paper_tables::journal_table(ui, journal, context),
            }
            footer_actions = paper_header::footer(
                ui,
                &mut self.footer,
                self.header.tab,
                &FooterFacts {
                    positions,
                    unrealized_total,
                    working_orders: working_orders.len(),
                    events: events.len(),
                    trades,
                    journal_entries: journal.len(),
                    currency,
                },
            );
        }

        self.apply_header_actions(header_actions, host, account, &mut panel_actions);
        self.apply_table_actions(table_actions, host);
        self.apply_footer_actions(footer_actions, host, account_id);
        if std::mem::take(&mut self.export_requested) {
            self.begin_export(ui.ctx(), host, account_id);
        }
        self.show_account_sheet(ui.ctx(), host, account_id);
        self.show_delete_confirm(ui.ctx(), host);
        self.show_ticket(ui.ctx(), host, currency);
        panel_actions
    }

    /// "Export Trading Data": with nothing to write, say so instead of opening a dialog; otherwise
    /// ask for a folder and write the three CSV files there, off the UI thread.
    fn begin_export(&mut self, ctx: &egui::Context, host: &PaperHost, account_id: Option<u64>) {
        if self.export_job.is_some() {
            return;
        }
        let Some(account_id) = account_id else {
            self.notice = Some(PanelNotice::info(
                "Create a paper account first; there is nothing to export yet.",
            ));
            return;
        };
        let export = paper_export::export_account(host.snapshot(), account_id);
        if export.is_empty() {
            self.notice = Some(PanelNotice::info(
                "Nothing to export yet. Orders, fills and closed trades show up here once you trade.",
            ));
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let outcome = crate::service::file_dialog::choose_folder("Export").and_then(|folder| {
                let Some(folder) = folder else {
                    return Ok(None);
                };
                for (name, text) in export.files() {
                    crate::service::file_dialog::write_text(&folder.join(name), text)?;
                }
                Ok(Some((folder, export.summary())))
            });
            let _ = sender.send(outcome);
            ctx.request_repaint();
        });
        self.export_job = Some(receiver);
        self.notice = None;
    }

    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.export_job else {
            return;
        };
        match job.try_recv() {
            Ok(Ok(Some((folder, summary)))) => {
                let name = folder.file_name().map_or_else(
                    || folder.display().to_string(),
                    |n| n.to_string_lossy().into(),
                );
                self.notice = Some(PanelNotice::info(format!(
                    "Exported {summary} to \u{201c}{name}\u{201d}."
                )));
                self.export_job = None;
            }
            Ok(Ok(None)) => self.export_job = None,
            Ok(Err(message)) => {
                self.notice = Some(PanelNotice::error(message));
                self.export_job = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.export_job = None,
        }
    }

    /// The selected account's lists, from the cache while the host's data has not changed.
    fn view(&mut self, host: &PaperHost) -> std::sync::Arc<PanelView> {
        let selected = host.snapshot().selected_account_id;
        if let Some(view) = &self.cached_view
            && view.revision == host.revision()
            && view.account_id == selected
        {
            return std::sync::Arc::clone(view);
        }
        let view = std::sync::Arc::new(PanelView::build(host));
        self.cached_view = Some(std::sync::Arc::clone(&view));
        view
    }

    /// Puts what failed in a notice that stays until dismissed. The host's own banner holds only
    /// the last error and a later success clears it, so a failure in a batch would be lost.
    fn report_failures(&mut self, host: &mut PaperHost, failures: &[PaperHostError]) {
        let Some(first) = failures.first() else {
            return;
        };
        let text = match failures.len() {
            1 => first.message(),
            more => format!(
                "{} failed: {} (and {} more)",
                more,
                first.message(),
                more - 1
            ),
        };
        self.notice = Some(PanelNotice::error(text));
        host.clear_error();
    }

    /// Failures from actions taken on a chart, shown the next time the panel is.
    pub fn report_messages(&mut self, messages: &[String]) {
        let Some(first) = messages.first() else {
            return;
        };
        let text = match messages.len() {
            1 => first.clone(),
            more => format!("{more} failed: {first} (and {} more)", more - 1),
        };
        self.notice = Some(PanelNotice::error(text));
    }

    fn show_notice(&mut self, ui: &mut egui::Ui) {
        if self.notice.as_ref().is_some_and(PanelNotice::expired) {
            self.notice = None;
        }
        let Some(notice) = &self.notice else {
            return;
        };
        if !notice.is_error {
            ui.ctx()
                .request_repaint_after(PanelNotice::LINGER.saturating_sub(notice.since.elapsed()));
        }
        let (tint, icon) = if notice.is_error {
            (crate::ui::theme::DOWN, Icon::TriangleFill)
        } else {
            (crate::ui::theme::text_secondary(), Icon::InfoCircle)
        };
        let mut dismissed = false;
        egui::Frame::new()
            .fill(if notice.is_error {
                tint.gamma_multiply(0.14)
            } else {
                crate::ui::theme::quaternary(0.5)
            })
            .inner_margin(egui::Margin::symmetric(12, 7))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    icons::inline(ui, icon, 12.0, tint);
                    ui.label(crate::ui::kit::text(
                        &notice.text,
                        12.0,
                        crate::ui::kit::REGULAR,
                        if notice.is_error {
                            tint
                        } else {
                            crate::ui::theme::text_primary()
                        },
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                        dismissed = crate::ui::kit::hover_button(
                            ui,
                            rect,
                            egui::Id::new("paper-notice-dismiss"),
                            Icon::Xmark,
                            "Dismiss",
                            true,
                            true,
                        )
                        .clicked();
                    });
                });
            });
        if dismissed {
            self.notice = None;
        }
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
                    if let Err(error) = host.select_account(id) {
                        self.report_failures(host, &[error]);
                    }
                }
                PaperHeaderAction::NewAccount => {
                    self.account_sheet = Some(PaperAccountSheetState::create());
                }
                PaperHeaderAction::ExportData => self.export_requested = true,
                PaperHeaderAction::ResetAccount => {
                    if let Some(account) = account {
                        self.account_sheet = Some(PaperAccountSheetState::reset(account));
                    }
                }
                PaperHeaderAction::DeleteAccount => {
                    self.keep_history_on_delete = true;
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
        let mut failures = Vec::new();
        for action in actions {
            let result = match action {
                PaperTableAction::ClosePosition {
                    account_id,
                    instrument_key,
                } => host.close_position(account_id, &instrument_key).map(drop),
                PaperTableAction::ReversePosition {
                    account_id,
                    instrument_key,
                } => host.reverse_position(account_id, &instrument_key).map(drop),
                PaperTableAction::CancelOrder(id) => host.cancel(id),
            };
            failures.extend(result.err());
        }
        self.report_failures(host, &failures);
    }

    fn apply_footer_actions(
        &mut self,
        actions: Vec<PaperFooterAction>,
        host: &mut PaperHost,
        account_id: Option<u64>,
    ) {
        for action in actions {
            match (action, account_id) {
                (PaperFooterAction::CloseAllPositions, Some(id)) => {
                    let results = host.close_all_positions(id);
                    if let Some(text) = bulk_failure_text("Closed", "positions", &results) {
                        self.notice = Some(PanelNotice::error(text));
                        host.clear_error();
                    }
                }
                (PaperFooterAction::CancelAllOrders, Some(id)) => {
                    let results = host.cancel_all_orders(id);
                    if let Some(text) = bulk_failure_text("Canceled", "orders", &results) {
                        self.notice = Some(PanelNotice::error(text));
                        host.clear_error();
                    }
                }
                (PaperFooterAction::Export, _) => self.export_requested = true,
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
                let outcome = if is_reset {
                    match selected_account_id {
                        Some(id) => host.reset_account_keeping(
                            id,
                            result.currency,
                            result.balance,
                            result.settings,
                            result.keep_journal_history,
                        ),
                        None => Ok(()),
                    }
                } else {
                    host.create_account(
                        &result.name,
                        result.currency,
                        result.balance,
                        result.settings,
                    )
                    .map(drop)
                };
                match outcome {
                    Ok(()) => self.account_sheet = None,
                    // Nothing was saved: the sheet stays, with what went wrong, so the user can
                    // retry or cancel instead of losing what they typed.
                    Err(error) => {
                        if let Some(sheet) = self.account_sheet.as_mut() {
                            sheet.submit_error = Some(error.message());
                        }
                        host.clear_error();
                    }
                }
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
        match crate::ui::paper_header::delete_confirm(
            context,
            &name,
            &mut self.keep_history_on_delete,
        ) {
            Some(crate::ui::kit::ConfirmOutcome::Confirmed) => {
                self.pending_delete = None;
                if let Err(error) = host.delete_account_keeping(id, self.keep_history_on_delete) {
                    self.report_failures(host, &[error]);
                }
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

/// What a mirrored account says about itself, in one quiet line.
pub fn owner_text(elsewhere: &Elsewhere) -> String {
    if elsewhere.paused {
        "Paused: two Macs changed this account. Placing an order here takes it over.".to_owned()
    } else if elsewhere.name.is_empty() {
        "Running on another Mac. Placing an order here takes it over.".to_owned()
    } else {
        format!(
            "Running on {}. Placing an order here takes it over.",
            elsewhere.name
        )
    }
}

fn owner_note(ui: &mut egui::Ui, elsewhere: &Elsewhere) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(owner_text(elsewhere))
                    .small()
                    .color(crate::ui::theme::text_secondary()),
            );
        });
}

/// What a bulk action did not manage, or `None` when every item went through.
fn bulk_failure_text<T>(
    verb: &str,
    noun: &str,
    results: &[Result<T, PaperHostError>],
) -> Option<String> {
    let failures: Vec<&PaperHostError> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    let first = failures.first()?;
    let more = if failures.len() > 1 {
        format!(" (and {} more)", failures.len() - 1)
    } else {
        String::new()
    };
    Some(format!(
        "{verb} {} of {} {noun}. {}{more}",
        results.len() - failures.len(),
        results.len(),
        first.message()
    ))
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
    fn a_bulk_action_reports_what_it_could_not_do() {
        let ok: Result<u64, PaperHostError> = Ok(1);
        let failed: Result<u64, PaperHostError> =
            Err(PaperHostError::Engine(PaperTradingError::NoMarketData));
        assert_eq!(
            bulk_failure_text("Closed", "positions", &[ok.clone(), ok.clone()]),
            None
        );
        let text = bulk_failure_text(
            "Closed",
            "positions",
            &[ok.clone(), failed.clone(), failed.clone()],
        )
        .unwrap();
        assert!(text.starts_with("Closed 1 of 3 positions."), "{text}");
        assert!(text.ends_with("(and 1 more)"), "{text}");
    }

    #[test]
    fn a_tick_that_fills_nothing_changes_no_list_and_saves_nothing() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let account_id = account(&mut host);
        let key = "Binance:BTCUSDT";
        let tick = |last: f64, timestamp: i64| PaperQuote {
            instrument_key: key.to_owned(),
            bid: None,
            ask: None,
            last: Some(last),
            timestamp,
            is_market_open: true,
        };
        let now = crate::model::watchlist::now_ms();
        let before = host.revision();
        host.process_quote(tick(100.0, now)).unwrap();
        host.process_quote(tick(101.0, now + 1)).unwrap();
        assert_eq!(
            host.revision(),
            before,
            "no order to evaluate: only the mark moves"
        );
        assert_eq!(host.snapshot().quotes[key].last, Some(101.0));
        assert!(
            store
                .lock()
                .unwrap()
                .paper_trading()
                .unwrap()
                .quotes
                .is_empty(),
            "a bare tick is not saved"
        );

        // With a resting order the tick is evaluated, but while nothing fills nothing is saved
        // or counted either.
        let mut request = market_request(account_id);
        request.kind = PaperOrderType::Limit;
        request.limit_price = Some(50.0);
        host.submit(request).unwrap();
        let after_submit = host.revision();
        host.process_quote(tick(102.0, now + 2)).unwrap();
        assert_eq!(host.revision(), after_submit);
        assert_eq!(host.snapshot().quotes[key].last, Some(102.0));

        // A tick that fills it is saved and counted.
        host.process_quote(tick(40.0, now + 3)).unwrap();
        assert!(host.revision() > after_submit);
        assert_eq!(host.snapshot().positions.len(), 1);
        assert_eq!(
            store
                .lock()
                .unwrap()
                .paper_trading()
                .unwrap()
                .positions
                .len(),
            1
        );
    }

    #[test]
    fn data_that_disagrees_with_itself_is_not_loaded() {
        let store = store();
        {
            let mut host = PaperHost::load(store.clone());
            let id = account(&mut host);
            host.submit(market_request(id)).ok();
        }
        {
            let guard = store.lock().unwrap();
            let mut snapshot = guard.paper_trading().unwrap();
            snapshot.accounts[0].id = 424_242;
            guard.save_paper_trading(&snapshot).unwrap();
        }
        let mut host = PaperHost::load(store);
        assert!(host.load_failed(), "orders now point at a missing account");
        assert!(matches!(
            host.last_error(),
            Some(PaperHostError::Integrity(_))
        ));
        assert!(host.submit(market_request(424_242)).is_err());
    }

    /// `cargo test --lib paper_host_cost -- --ignored --nocapture`
    #[test]
    #[ignore = "timing"]
    fn paper_host_cost() {
        use std::time::Instant;
        for fills in [100usize, 1_000, 5_000] {
            let store = store();
            let mut host = PaperHost::load(store.clone());
            let id = account(&mut host);
            let key = "Binance:BTCUSDT";
            let now = crate::model::watchlist::now_ms();
            let tick = |last: f64, n: i64| PaperQuote {
                instrument_key: key.to_owned(),
                bid: Some(last - 0.5),
                ask: Some(last + 0.5),
                last: Some(last),
                timestamp: now + n,
                is_market_open: true,
            };
            host.process_quote(tick(100.0, 0)).unwrap();
            // Build the history inside one engine, then save it once.
            let mut engine = PaperTradingEngine::new(host.snapshot().clone());
            for index in 0..fills {
                let mut request = market_request(id);
                request.side = if index % 2 == 0 {
                    PaperOrderSide::Buy
                } else {
                    PaperOrderSide::Sell
                };
                engine.submit(request).unwrap();
            }
            store
                .lock()
                .unwrap()
                .save_paper_trading(engine.snapshot())
                .unwrap();
            let mut host = PaperHost::load(store.clone());
            host.process_quote(tick(100.0, 1_000_000)).unwrap();

            let started = Instant::now();
            for n in 0..100 {
                host.process_quote(tick(100.0 + n as f64, 2_000_000 + n))
                    .unwrap();
            }
            let bare_tick = started.elapsed() / 100;

            let mut request = market_request(id);
            request.kind = PaperOrderType::Limit;
            request.limit_price = Some(50.0);
            let started = Instant::now();
            host.submit(request).unwrap();
            let submit = started.elapsed();

            let started = Instant::now();
            for n in 0..10 {
                host.process_quote(tick(100.0 + n as f64, 3_000_000 + n))
                    .unwrap();
            }
            let evaluated_tick = started.elapsed() / 10;

            let started = Instant::now();
            let view = PanelView::build(&host);
            let build_view = started.elapsed();
            println!(
                "{fills:>5} fills: bare tick {bare_tick:?}, tick with a working order {evaluated_tick:?}, submit {submit:?}, panel view build {build_view:?} ({} events)",
                view.events.len()
            );
        }
    }

    #[test]
    fn tracks_only_instruments_with_a_working_order_or_a_position() {
        let mut host = PaperHost::load(store());
        let account_id = account(&mut host);
        let instrument = PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance);
        assert!(!host.tracks(&instrument.key));

        host.process_quote(PaperQuote {
            instrument_key: instrument.key.clone(),
            bid: None,
            ask: None,
            last: Some(100.0),
            timestamp: crate::model::watchlist::now_ms(),
            is_market_open: true,
        })
        .unwrap();
        let mut request = market_request(account_id);
        request.kind = PaperOrderType::Limit;
        request.limit_price = Some(50.0);
        let order = host.submit(request).unwrap();
        assert!(host.tracks(&instrument.key), "a resting order is tracked");
        assert!(!host.tracks("binance:ETHUSDT"));

        host.cancel(order).unwrap();
        assert!(!host.tracks(&instrument.key));

        host.submit(market_request(account_id)).unwrap();
        assert!(host.tracks(&instrument.key), "an open position is tracked");
    }

    #[test]
    fn the_note_for_a_mirrored_account_names_the_mac_or_says_it_is_paused() {
        let named = Elsewhere {
            name: "Studio".into(),
            paused: false,
        };
        assert_eq!(
            owner_text(&named),
            "Running on Studio. Placing an order here takes it over."
        );
        let unknown = Elsewhere {
            name: String::new(),
            paused: false,
        };
        assert!(owner_text(&unknown).starts_with("Running on another Mac."));
        let paused = Elsewhere {
            name: String::new(),
            paused: true,
        };
        assert!(owner_text(&paused).starts_with("Paused"));
    }

    #[test]
    fn reload_picks_up_what_sync_wrote_and_keeps_live_quotes() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = host
            .create_account(
                "Mine",
                PaperCurrency::Usd,
                1_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        host.process_quote(PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: Some(1.0),
            ask: Some(2.0),
            last: Some(1.5),
            timestamp: 5,
            is_market_open: true,
        })
        .unwrap();
        {
            let guard = store.lock().unwrap();
            let mut snapshot = guard.paper_trading().unwrap();
            snapshot
                .accounts
                .push(crate::model::paper::PaperAccount::new(77, "From iCloud", 1));
            snapshot.selected_account_id = Some(999);
            guard.save_paper_trading(&snapshot).unwrap();
            guard.set_paper_owner("77", Some("other-mac")).unwrap();
        }
        host.reload();
        assert_eq!(host.snapshot().accounts.len(), 2);
        assert_eq!(host.snapshot().selected_account_id, Some(id));
        assert!(host.snapshot().quotes.contains_key("Binance:BTCUSDT"));
        assert!(host.owned_elsewhere(77).is_some());
        assert!(host.owned_elsewhere(id).is_none());
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

    // MARK: Trading journal capture

    fn btc_quote(last: f64) -> PaperQuote {
        PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: None,
            ask: None,
            last: Some(last),
            timestamp: now_ms(),
            is_market_open: true,
        }
    }

    fn journal_executions(store: &SharedStore) -> Vec<crate::model::journal::model::Execution> {
        store.lock().unwrap().journal_executions().unwrap()
    }

    #[test]
    fn a_ticket_fill_reaches_the_journal() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(btc_quote(50_000.0)).unwrap();
        host.submit(market_request(id)).unwrap();

        let executions = journal_executions(&store);
        assert_eq!(executions.len(), 1);
        assert_eq!(executions[0].quantity, rust_decimal::Decimal::ONE);
        assert_eq!(executions[0].instrument.symbol, "BTCUSDT");
    }

    #[test]
    fn a_fill_made_by_a_quote_reaches_the_journal() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(btc_quote(50_000.0)).unwrap();
        let mut request = market_request(id);
        request.kind = PaperOrderType::Limit;
        request.limit_price = Some(40_000.0);
        host.submit(request).unwrap();
        assert!(
            journal_executions(&store).is_empty(),
            "a working order is not a fill"
        );

        host.process_quote(btc_quote(39_000.0)).unwrap();
        let executions = journal_executions(&store);
        assert_eq!(executions.len(), 1);
        assert_eq!(
            executions[0].order_ref.as_deref(),
            Some(&*host.snapshot().orders[0].id.to_string())
        );
    }

    #[test]
    fn opening_and_closing_gives_the_journal_one_completed_trade() {
        use crate::model::journal::model::{CostBasis, TradeStatus};
        use crate::model::journal::reconstruct::reconstruct;
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(btc_quote(50_000.0)).unwrap();
        host.submit(market_request(id)).unwrap();
        host.process_quote(btc_quote(51_000.0)).unwrap();
        host.close_position(id, "Binance:BTCUSDT").unwrap();

        let executions = journal_executions(&store);
        assert_eq!(executions.len(), 2);
        let trades = reconstruct(&executions, |_| CostBasis::AverageCost);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].status, TradeStatus::Closed);
        let engine_net: f64 = host
            .snapshot()
            .closed_trades
            .iter()
            .map(|t| t.net_pnl())
            .sum();
        assert_eq!(
            trades[0].net_pnl().to_string().parse::<f64>().unwrap(),
            engine_net
        );
    }

    #[test]
    fn checking_an_order_does_not_touch_the_journal() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(btc_quote(50_000.0)).unwrap();
        host.check_order(market_request(id)).unwrap();
        assert!(journal_executions(&store).is_empty());
        assert!(host.snapshot().fills.is_empty());
    }

    #[test]
    fn a_restart_does_not_duplicate_what_the_journal_holds() {
        let store = store();
        {
            let mut host = PaperHost::load(store.clone());
            let id = account(&mut host);
            host.process_quote(btc_quote(50_000.0)).unwrap();
            host.submit(market_request(id)).unwrap();
        }
        let first = journal_executions(&store);
        assert_eq!(first.len(), 1);
        let _restarted = PaperHost::load(store.clone());
        let _again = PaperHost::load(store.clone());
        assert_eq!(journal_executions(&store), first);
    }

    #[test]
    fn existing_history_is_backfilled_when_the_host_loads() {
        let store = store();
        {
            let mut host = PaperHost::load(store.clone());
            let id = account(&mut host);
            host.process_quote(btc_quote(50_000.0)).unwrap();
            host.submit(market_request(id)).unwrap();
        }
        {
            let guard = store.lock().unwrap();
            guard
                .raw_connection()
                .execute("DELETE FROM journal_execution", [])
                .unwrap();
            guard
                .raw_connection()
                .execute("DELETE FROM journal_cursor", [])
                .unwrap();
            guard
                .raw_connection()
                .execute("DELETE FROM journal_account", [])
                .unwrap();
        }
        assert!(journal_executions(&store).is_empty());
        let _host = PaperHost::load(store.clone());
        assert_eq!(journal_executions(&store).len(), 1);
        assert_eq!(store.lock().unwrap().journal_accounts().unwrap().len(), 1);
    }

    #[test]
    fn fills_that_arrive_through_the_ledger_reach_the_journal_on_reload() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(btc_quote(50_000.0)).unwrap();
        host.submit(market_request(id)).unwrap();
        // What an iCloud apply does: the paper tables change without this Mac's save path.
        let mut other = host.snapshot().clone();
        let mut fill = other.fills[0].clone();
        fill.id += 1_000;
        fill.timestamp += 5;
        other.fills.push(fill);
        {
            let guard = store.lock().unwrap();
            guard
                .raw_connection()
                .execute(
                    "INSERT INTO paper_fill (account_id, payload) VALUES (?1, ?2)",
                    rusqlite::params![
                        id.to_string(),
                        serde_json::to_string(&other.fills[1]).unwrap()
                    ],
                )
                .unwrap();
        }
        assert_eq!(journal_executions(&store).len(), 1);
        host.reload();
        assert_eq!(journal_executions(&store).len(), 2);
    }

    fn host_with_a_fill(store: &SharedStore) -> (PaperHost, u64) {
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.create_account(
            "Second",
            PaperCurrency::Usd,
            1_000.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
        host.process_quote(btc_quote(50_000.0)).unwrap();
        host.submit(market_request(id)).unwrap();
        (host, id)
    }

    #[test]
    fn deleting_an_account_can_keep_its_journal_history() {
        let store = store();
        let (mut host, id) = host_with_a_fill(&store);
        host.delete_account_keeping(id, true).unwrap();
        assert_eq!(journal_executions(&store).len(), 1);
        let accounts = store.lock().unwrap().journal_accounts().unwrap();
        assert!(
            accounts
                .iter()
                .any(|account| account.archived && account.name == "Main")
        );
    }

    #[test]
    fn deleting_an_account_can_remove_its_journal_history() {
        let store = store();
        let (mut host, id) = host_with_a_fill(&store);
        let revision = host.revision();
        host.delete_account_keeping(id, false).unwrap();
        assert!(journal_executions(&store).is_empty());
        let accounts = store.lock().unwrap().journal_accounts().unwrap();
        assert_eq!(accounts.len(), 1, "only the other account is left");
        assert_eq!(accounts[0].name, "Second");
        assert!(host.revision() > revision, "watchers refresh");
        // A later save does not bring it back.
        host.process_quote(btc_quote(50_100.0)).unwrap();
        assert!(journal_executions(&store).is_empty());
    }

    #[test]
    fn a_reset_keeps_or_removes_only_the_old_generation() {
        for keep in [true, false] {
            let store = store();
            let (mut host, id) = host_with_a_fill(&store);
            std::thread::sleep(std::time::Duration::from_millis(3));
            host.reset_account_keeping(
                id,
                PaperCurrency::Usd,
                100_000.0,
                PaperAccountSettings::default(),
                keep,
            )
            .unwrap();
            host.process_quote(btc_quote(51_000.0)).unwrap();
            host.submit(market_request(id)).unwrap();
            let executions = journal_executions(&store);
            assert_eq!(executions.len(), if keep { 2 } else { 1 }, "keep={keep}");
        }
    }

    #[test]
    fn retrying_the_journal_pass_is_safe_to_repeat() {
        let store = store();
        let (host, _) = host_with_a_fill(&store);
        host.reconcile_journal().unwrap();
        host.reconcile_journal().unwrap();
        assert_eq!(journal_executions(&store).len(), 1);
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

    #[test]
    fn exporting_with_nothing_to_export_says_so_and_opens_no_dialog() {
        let mut host = PaperHost::load(store());
        let ctx = egui::Context::default();
        let mut panel = PaperPanel::default();

        panel.begin_export(&ctx, &host, None);
        assert!(
            panel
                .notice
                .as_ref()
                .is_some_and(|notice| notice.text.contains("Create a paper account"))
        );

        let id = account(&mut host);
        panel.notice = None;
        panel.begin_export(&ctx, &host, Some(id));
        let notice = panel.notice.as_ref().expect("the user is told");
        assert!(
            notice.text.starts_with("Nothing to export yet"),
            "{}",
            notice.text
        );
        assert!(!notice.is_error);
        assert!(
            panel.export_job.is_none(),
            "no folder dialog for an empty account"
        );
    }

    #[test]
    fn an_info_notice_fades_but_an_error_stays_until_dismissed() {
        let mut notice = PanelNotice::info("done");
        assert!(!notice.expired());
        notice.since -= PanelNotice::LINGER;
        assert!(notice.expired());
        let mut error = PanelNotice::error("disk full");
        error.since -= PanelNotice::LINGER * 10;
        assert!(!error.expired());
    }

    /// The notice line, as pictures: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib paper_host::tests::gallery -- --ignored`.
    #[test]
    #[ignore = "writes pictures"]
    fn gallery_of_the_notice() {
        use crate::ui::render_tests::Screen;
        let _restore = crate::ui::theme::RestoreDefaultTheme;
        let mut host = PaperHost::load(store());
        account(&mut host);
        for (name, notice) in [
            (
                "empty",
                PanelNotice::info(
                    "Nothing to export yet. Orders, fills and closed trades show up here once you trade.",
                ),
            ),
            (
                "done",
                PanelNotice::info(
                    "Exported 3 orders, 2 fills and 1 closed trade to \u{201c}Documents\u{201d}.",
                ),
            ),
            (
                "error",
                PanelNotice::error(
                    "Could not write /Volumes/x/paper-orders.csv: Read-only file system",
                ),
            ),
        ] {
            for light in [false, true] {
                let mut screen = Screen::new(egui::vec2(900.0, 260.0));
                screen.ctx.set_theme(if light {
                    egui::ThemePreference::Light
                } else {
                    egui::ThemePreference::Dark
                });
                let mut panel = PaperPanel {
                    notice: Some(PanelNotice {
                        text: notice.text.clone(),
                        ..notice
                    }),
                    ..PaperPanel::default()
                };
                let mut last = None;
                for _ in 0..4 {
                    last = Some(screen.run(vec![], |ui| {
                        crate::ui::theme::sync(ui.ctx());
                        egui::Panel::bottom(egui::Id::new("g"))
                            .exact_size(260.0)
                            .frame(egui::Frame::new().fill(crate::ui::kit::window()))
                            .show(ui, |ui| {
                                panel.ui(ui, &mut host);
                            });
                    }));
                }
                screen.save(
                    &format!("notice-{name}-{}", if light { "light" } else { "dark" }),
                    last.expect("ran"),
                );
            }
        }
    }

    fn now_ms() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64)
    }
}
