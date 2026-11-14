//! Stateful bridge between the portfolio ledger, SQLite, and the Portfolio tab in
//! `ui::portfolio::tab`.
//!
//! Mutations are copy-on-write: validate against a candidate ledger, persist its complete
//! snapshot, and publish it only after SQLite succeeds. A failed initial load permanently blocks
//! writes for this host instance so corrupt data can never be replaced by an empty snapshot.

pub mod card;
pub mod import_flow;

use crate::model::portfolio::history::{rebuild_start, transactions_fingerprint};
use crate::model::portfolio::ledger::PortfolioLedger;
use crate::model::portfolio::{
    Portfolio, PortfolioAsset, PortfolioCurrency, PortfolioError, PortfolioLedgerSnapshot,
    PortfolioSnapshot, PortfolioTransaction,
};
use crate::service::portfolio::history::{HistoryRebuilder, HistoryResult};
use crate::service::portfolio::quotes::PortfolioFxRates;
use crate::service::system::timezone::local_utc_offset_secs;
use crate::store::{SharedStore, Store};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum PortfolioHostError {
    Load(String),
    Mutation(PortfolioError),
    Persistence(String),
    StoreUnavailable,
}

impl PortfolioHostError {
    pub fn message(&self) -> String {
        match self {
            Self::Load(message) => format!("Could not load portfolios: {message}"),
            Self::Mutation(error) => error.message(),
            Self::Persistence(message) => format!("Could not save portfolios: {message}"),
            Self::StoreUnavailable => "The portfolio database is unavailable.".to_owned(),
        }
    }
}

impl fmt::Display for PortfolioHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for PortfolioHostError {}

pub struct PortfolioHost {
    store: SharedStore,
    ledger: PortfolioLedger,
    load_error: Option<PortfolioHostError>,
    last_error: Option<PortfolioHostError>,
    history: HistoryRebuilder,
}

impl PortfolioHost {
    /// The store the host reads and writes, for the sheets that keep their own settings.
    pub fn store(&self) -> &SharedStore {
        &self.store
    }

    pub fn load(store: SharedStore) -> Self {
        let loaded = store
            .lock()
            .map_err(|_| PortfolioHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .portfolio_ledger()
                    .map_err(|error| PortfolioHostError::Load(error.to_string()))
            });
        match loaded {
            Ok(snapshot) => Self {
                store,
                ledger: PortfolioLedger::new(snapshot),
                load_error: None,
                last_error: None,
                history: HistoryRebuilder::default(),
            },
            Err(error) => Self {
                store,
                ledger: PortfolioLedger::new(PortfolioLedgerSnapshot::default()),
                load_error: Some(error.clone()),
                last_error: Some(error),
                history: HistoryRebuilder::default(),
            },
        }
    }

    pub fn snapshot(&self) -> &PortfolioLedgerSnapshot {
        self.ledger.snapshot()
    }

    /// Re-reads the ledger after iCloud sync changed it underneath. A ledger that cannot be read
    /// leaves the one in memory alone; a selection that no longer exists falls back to the first
    /// portfolio.
    pub fn reload(&mut self) {
        let loaded = self
            .store
            .lock()
            .map_err(|_| PortfolioHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .portfolio_ledger()
                    .map_err(|error| PortfolioHostError::Load(error.to_string()))
            });
        match loaded {
            Ok(mut snapshot) => {
                let selected_exists = snapshot
                    .selected_portfolio_id
                    .is_some_and(|id| snapshot.portfolios.iter().any(|p| p.id == id));
                if !selected_exists {
                    snapshot.selected_portfolio_id = snapshot.portfolios.first().map(|p| p.id);
                }
                self.ledger = PortfolioLedger::new(snapshot);
                self.load_error = None;
            }
            Err(error) => self.last_error = Some(error),
        }
    }

    /// Keeps the daily value history of `scope` current: stores what finished rebuilding and
    /// starts a rebuild for any portfolio whose history is missing days. A result is dropped
    /// when the ledger changed under it (the next frame starts a fresh one).
    pub fn drive_history(
        &mut self,
        ctx: &egui::Context,
        scope: &HashSet<u64>,
        now_ms: i64,
        fx: &Arc<dyn PortfolioFxRates>,
    ) {
        if self.load_error.is_some() {
            return;
        }
        for result in self.history.poll(now_ms) {
            self.accept_rebuilt(result, now_ms);
        }
        let snapshot = self.ledger.snapshot();
        let mut starts = Vec::new();
        for portfolio in snapshot
            .portfolios
            .iter()
            .filter(|portfolio| scope.contains(&portfolio.id) && !portfolio.is_archived)
        {
            if !self.history.can_start(portfolio.id, now_ms) {
                continue;
            }
            let from = rebuild_start(portfolio.id, snapshot, now_ms, local_utc_offset_secs)
                .or_else(|| self.history.partial_due(portfolio.id, now_ms));
            if let Some(from) = from {
                let transactions: Vec<PortfolioTransaction> = snapshot
                    .transactions
                    .iter()
                    .filter(|transaction| transaction.portfolio_id == portfolio.id)
                    .cloned()
                    .collect();
                starts.push((portfolio.clone(), transactions, from));
            }
        }
        for (portfolio, transactions, from) in starts {
            self.history.start(
                ctx,
                &portfolio,
                transactions,
                from,
                now_ms,
                local_utc_offset_secs,
                Arc::clone(fx),
            );
        }
    }

    /// Whether any of `scope`'s history is being rebuilt right now.
    pub fn is_rebuilding_history(&self, scope: &HashSet<u64>) -> bool {
        scope.iter().any(|id| self.history.is_rebuilding(*id))
    }

    pub fn load_failed(&self) -> bool {
        self.load_error.is_some()
    }

    pub fn last_error(&self) -> Option<&PortfolioHostError> {
        self.last_error.as_ref()
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn create_portfolio(
        &mut self,
        name: &str,
        currency: PortfolioCurrency,
    ) -> Result<u64, PortfolioHostError> {
        self.mutate(|ledger| ledger.create_portfolio(name, currency))
    }

    /// Changing the selection writes the one setting, not the whole ledger: it happens on a
    /// click, on the UI thread, with the store locked.
    pub fn select(&mut self, id: Option<u64>) -> Result<(), PortfolioHostError> {
        self.ensure_loaded()?;
        if let Some(id) = id
            && !self.ledger.snapshot().portfolios.iter().any(|p| p.id == id)
        {
            return self.fail(PortfolioHostError::Mutation(
                PortfolioError::PortfolioNotFound,
            ));
        }
        if let Err(error) = self.save_setting(|store| store.set_selected_portfolio(id)) {
            return self.fail(error);
        }
        // Checked above: this only moves the in-memory selection to match what was saved.
        let _ = self.ledger.select(id);
        self.last_error = None;
        Ok(())
    }

    pub fn update_portfolio(&mut self, portfolio: Portfolio) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.update_portfolio(portfolio))
    }

    pub fn duplicate_portfolio(&mut self, id: u64) -> Result<u64, PortfolioHostError> {
        self.mutate(|ledger| ledger.duplicate_portfolio(id))
    }

    pub fn reorder(
        &mut self,
        from: &[usize],
        destination: usize,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| {
            ledger.reorder(from, destination);
            Ok(())
        })
    }

    pub fn delete_portfolio(&mut self, id: u64) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| ledger.delete_portfolio(id))
    }

    pub fn add_transaction(
        &mut self,
        transaction: PortfolioTransaction,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.add_transaction(transaction))
    }

    pub fn update_transaction(
        &mut self,
        transaction: PortfolioTransaction,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.update_transaction(transaction))
    }

    // MARK: Preferences

    /// Whether amounts are hidden. Stored on its own, so it outlives any ledger change.
    pub fn privacy_mode(&self) -> bool {
        self.store
            .lock()
            .ok()
            .and_then(|store| store.setting::<bool>(PRIVACY_KEY))
            .unwrap_or(false)
    }

    /// A choice that cannot be saved would come back reverted after a restart: say so.
    pub fn set_privacy_mode(&mut self, on: bool) {
        let saved = self.save_setting(|store| store.set_setting(PRIVACY_KEY, &on));
        self.report(saved);
    }

    /// The currency amounts were last shown in for a selection (one portfolio, or all of them);
    /// before any choice, the portfolio's own currency, or dollars for all.
    pub fn reporting_currency_for(&self, selection: Option<u64>) -> PortfolioCurrency {
        let saved = self
            .store
            .lock()
            .ok()
            .and_then(|store| store.setting::<ReportingPreferences>(REPORTING_KEY))
            .and_then(|preferences| {
                preferences
                    .currencies
                    .get(&preference_key(selection))
                    .copied()
            });
        saved.unwrap_or_else(|| native_currency(self.ledger.snapshot(), selection))
    }

    pub fn set_reporting_currency(&mut self, selection: Option<u64>, currency: PortfolioCurrency) {
        let saved = self.save_setting(|store| {
            let mut preferences = store
                .setting::<ReportingPreferences>(REPORTING_KEY)
                .unwrap_or_default();
            preferences
                .currencies
                .insert(preference_key(selection), currency);
            store.set_setting(REPORTING_KEY, &preferences)
        });
        self.report(saved);
    }

    fn save_setting(
        &self,
        write: impl FnOnce(&Store) -> rusqlite::Result<()>,
    ) -> Result<(), PortfolioHostError> {
        let store = self
            .store
            .lock()
            .map_err(|_| PortfolioHostError::StoreUnavailable)?;
        write(&store).map_err(|error| PortfolioHostError::Persistence(error.to_string()))
    }

    /// Keeps a failure for the error sheet; a success leaves any earlier one alone.
    fn report(&mut self, result: Result<(), PortfolioHostError>) {
        if let Err(error) = result {
            self.last_error = Some(error);
        }
    }

    /// Moves every transaction of one asset (in `portfolio_ids`) to another market. All or
    /// nothing: the ledger re-checks each affected portfolio's holdings before it commits.
    pub fn remap_asset(
        &mut self,
        source_key: &str,
        destination: &PortfolioAsset,
        portfolio_ids: &HashSet<u64>,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| ledger.remap_asset(source_key, destination, portfolio_ids))
    }

    pub fn delete_transaction(&mut self, id: u64) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| ledger.delete_transaction(id))
    }

    pub fn import_transactions(
        &mut self,
        transactions: Vec<PortfolioTransaction>,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.import_transactions(transactions))
    }

    /// Stores a finished rebuild, if the portfolio still has the transactions it was computed
    /// from. The result is for those: if the portfolio has changed since, it is dropped and the
    /// next frame starts a fresh rebuild.
    fn accept_rebuilt(&mut self, result: HistoryResult, now_ms: i64) {
        let Some(snapshots) = result.snapshots else {
            return;
        };
        let basis = transactions_fingerprint(
            &self
                .ledger
                .snapshot()
                .transactions
                .iter()
                .filter(|transaction| transaction.portfolio_id == result.portfolio_id)
                .cloned()
                .collect::<Vec<_>>(),
        );
        if basis == result.basis
            && self
                .store_snapshots(snapshots, result.portfolio_id, result.from)
                .is_err()
        {
            // `store_snapshots` has told the error sheet. Without a pause the next frame
            // would rebuild the same days and fail the same way.
            self.history.defer(result.portfolio_id, now_ms);
        }
    }

    /// Stores a rebuilt stretch of one portfolio's history. Only that stretch and the stale
    /// marks are written; a background save does not dismiss an error the user has not seen.
    pub fn store_snapshots(
        &mut self,
        snapshots: Vec<PortfolioSnapshot>,
        portfolio_id: u64,
        from: i64,
    ) -> Result<(), PortfolioHostError> {
        self.ensure_loaded()?;
        let mut invalidated = self.ledger.snapshot().invalidated_after.clone();
        invalidated.remove(&portfolio_id);
        if let Err(error) = self.save_setting(|store| {
            store.replace_portfolio_history(portfolio_id, from, &snapshots, &invalidated)
        }) {
            return self.fail(error);
        }
        // Cannot fail: it only mirrors in memory what was just written.
        let _ = self.ledger.store_snapshots(snapshots, portfolio_id, from);
        Ok(())
    }

    fn ensure_loaded(&mut self) -> Result<(), PortfolioHostError> {
        if let Some(error) = self.load_error.clone() {
            self.last_error = Some(error.clone());
            return Err(error);
        }
        Ok(())
    }

    fn mutate<T>(
        &mut self,
        mutation: impl FnOnce(&mut PortfolioLedger) -> Result<T, PortfolioError>,
    ) -> Result<T, PortfolioHostError> {
        self.ensure_loaded()?;

        let mut candidate = PortfolioLedger::new(self.ledger.snapshot().clone());
        let value = match mutation(&mut candidate) {
            Ok(value) => value,
            Err(error) => return self.fail(PortfolioHostError::Mutation(error)),
        };
        let saved = self
            .store
            .lock()
            .map_err(|_| PortfolioHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .save_portfolio_ledger(candidate.snapshot())
                    .map_err(|error| PortfolioHostError::Persistence(error.to_string()))
            });
        if let Err(error) = saved {
            return self.fail(error);
        }

        self.ledger = candidate;
        self.last_error = None;
        Ok(value)
    }

    fn fail<T>(&mut self, error: PortfolioHostError) -> Result<T, PortfolioHostError> {
        self.last_error = Some(error.clone());
        Err(error)
    }
}

const PRIVACY_KEY: &str = "portfolio.privacyMode";
const REPORTING_KEY: &str = "portfolio.reportingCurrencies";

/// The currency chosen per selection, keyed by `preference_key`.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
struct ReportingPreferences {
    #[serde(default)]
    currencies: HashMap<String, PortfolioCurrency>,
}

fn preference_key(selection: Option<u64>) -> String {
    selection.map_or_else(|| "all".to_owned(), |id| id.to_string())
}

/// The portfolio's own currency; dollars when looking at all of them.
fn native_currency(
    snapshot: &PortfolioLedgerSnapshot,
    selection: Option<u64>,
) -> PortfolioCurrency {
    selection
        .and_then(|id| {
            snapshot
                .portfolios
                .iter()
                .find(|portfolio| portfolio.id == id)
        })
        .map_or(PortfolioCurrency::Usd, |portfolio| portfolio.base_currency)
}

/// The viewer's UTC offset now, for showing and typing dates in local time.
pub(crate) fn local_offset() -> i64 {
    crate::service::system::timezone::local_utc_offset_secs(now_ms())
}

pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests;
