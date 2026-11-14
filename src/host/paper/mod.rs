//! Stateful bridge between the paper-trading execution engine, SQLite, and the panel views in
//! `ui::paper::panel`.
//!
//! The execution engine deliberately records rejected orders before returning their error. This
//! host therefore persists and publishes every candidate engine state, including a domain-error
//! result, while still refusing all writes after an initial load failure.

use crate::model::paper::engine::PaperTradingEngine;
use crate::model::paper::{
    PaperAccountMetrics, PaperAccountSettings, PaperCurrency, PaperOrderChanges, PaperOrderRequest,
    PaperOrderSide, PaperOrderType, PaperPosition, PaperQuote, PaperTimeInForce, PaperTradingError,
    PaperTradingSnapshot,
};
use crate::store::SharedStore;
use crate::store::sync::Elsewhere;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PaperHostError {
    #[error("Could not load paper trading: {0}")]
    Load(String),
    #[error("{}", .0.message())]
    Engine(PaperTradingError),
    #[error("Could not save paper trading: {0}")]
    Persistence(String),
    /// The data disagrees with itself; nothing was saved or shown from it.
    #[error("Paper trading data failed a consistency check: {0}")]
    Integrity(String),
    #[error("The paper-trading database is unavailable.")]
    StoreUnavailable,
}

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

#[cfg(test)]
mod tests;
