//! Stateful bridge between the portfolio ledger, SQLite, and the pure portfolio views.
//!
//! Mutations are copy-on-write: validate against a candidate ledger, persist its complete
//! snapshot, and publish it only after SQLite succeeds. A failed initial load permanently blocks
//! writes for this host instance so corrupt data can never be replaced by an empty snapshot.
#![allow(dead_code)]

use crate::host::portfolio_import_flow::PortfolioImportFlow;
use crate::model::portfolio::{
    Portfolio, PortfolioAsset, PortfolioCurrency, PortfolioError, PortfolioHoldingsSort,
    PortfolioLedgerSnapshot, PortfolioSnapshot, PortfolioSource, PortfolioTransaction,
    PortfolioTransactionType,
};
use crate::model::portfolio_accounting::holdings;
use crate::model::portfolio_csv;
use crate::model::portfolio_format::{PORTFOLIO_CURRENCIES, PortfolioValueFormatter};
use crate::model::portfolio_history::rebuild_start;
use crate::model::portfolio_ledger::PortfolioLedger;
use crate::model::portfolio_statistics::PortfolioHistoryRange;
use crate::model::watchlist::new_id;
use crate::service::asset_auto_mapper::asset_from_search_result;
use crate::service::portfolio_history::{HistoryRebuilder, IntradayFeed, IntradayRequest};
use crate::service::portfolio_quotes::{PortfolioQuoteFeed, PortfolioQuoteState};
use crate::store::SharedStore;
use crate::ui::header_controls;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::portfolio_asset_detail::{self, AssetDetailAction, AssetDetailInput};
use crate::ui::portfolio_asset_picker::{self, AssetPickerOutcome, AssetPickerState};
use crate::ui::portfolio_holdings::{self, HoldingsAction, HoldingsInput, HoldingsState};
use crate::ui::portfolio_overview::{
    self, OverviewAction, OverviewInput, OverviewState, StatisticsInput,
};
use crate::ui::portfolio_sheets::{
    self, CreateOutcome, CreateState, ManageAction, ManageInput, ManageRow, ManageState,
};
use crate::ui::portfolio_transaction_editor::{
    self, EditorMode, TransactionEditorOutcome, TransactionEditorState,
};
use crate::ui::portfolio_transactions::{
    self, Projection, TransactionAction, TransactionsInput, TransactionsState,
};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use std::fmt;

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
        utc_offset_secs: i64,
    ) {
        if self.load_error.is_some() {
            return;
        }
        for result in self.history.poll(now_ms) {
            let Some(snapshots) = result.snapshots else {
                continue;
            };
            let current = rebuild_start(
                result.portfolio_id,
                self.ledger.snapshot(),
                now_ms,
                utc_offset_secs,
            );
            if current == Some(result.from) {
                let _ = self.store_snapshots(snapshots, result.portfolio_id, result.from);
            }
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
            if let Some(from) = rebuild_start(portfolio.id, snapshot, now_ms, utc_offset_secs) {
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
            self.history
                .start(ctx, &portfolio, transactions, from, now_ms, utc_offset_secs);
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

    pub fn select(&mut self, id: Option<u64>) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| ledger.select(id))
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

    pub fn set_privacy_mode(&self, on: bool) {
        if let Ok(store) = self.store.lock() {
            let _ = store.set_setting(PRIVACY_KEY, &on);
        }
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

    pub fn set_reporting_currency(&self, selection: Option<u64>, currency: PortfolioCurrency) {
        if let Ok(store) = self.store.lock() {
            let mut preferences = store
                .setting::<ReportingPreferences>(REPORTING_KEY)
                .unwrap_or_default();
            preferences
                .currencies
                .insert(preference_key(selection), currency);
            let _ = store.set_setting(REPORTING_KEY, &preferences);
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

    pub fn store_snapshots(
        &mut self,
        snapshots: Vec<PortfolioSnapshot>,
        portfolio_id: u64,
        from: i64,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.store_snapshots(snapshots, portfolio_id, from))
    }

    fn mutate<T>(
        &mut self,
        mutation: impl FnOnce(&mut PortfolioLedger) -> Result<T, PortfolioError>,
    ) -> Result<T, PortfolioHostError> {
        if let Some(error) = &self.load_error {
            self.last_error = Some(error.clone());
            return Err(error.clone());
        }

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PortfolioSection {
    #[default]
    Overview,
    Holdings,
    Transactions,
    Statistics,
}

impl PortfolioSection {
    const ALL: [Self; 4] = [
        Self::Overview,
        Self::Holdings,
        Self::Transactions,
        Self::Statistics,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Holdings => "Holdings",
            Self::Transactions => "Transactions",
            Self::Statistics => "Statistics",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Overview => Icon::PieChart,
            Self::Holdings => Icon::Layers,
            Self::Transactions => Icon::Exchange,
            Self::Statistics => Icon::BarChart,
        }
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

/// An export waiting for the data it needs (holdings and history are computed with the content).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingExport {
    Holdings,
    History,
}

/// Per-window state for the dedicated Portfolio tab. The ledger itself stays app-wide in
/// `PortfolioHost`, so opening/focusing the tab never creates a second source of truth.
pub struct PortfolioTab {
    id: u64,
    quote_feed: PortfolioQuoteFeed,
    import: PortfolioImportFlow,
    pending_export: Option<PendingExport>,
    section: PortfolioSection,
    reporting_currency: PortfolioCurrency,
    privacy: bool,
    /// Whether the saved privacy switch has been read yet.
    prefs_loaded: bool,
    /// The selection `reporting_currency` was last loaded for.
    loaded_selection: Option<Option<u64>>,
    overview: OverviewState,
    intraday: IntradayFeed,
    holdings: HoldingsState,
    transactions: TransactionsState,
    create: Option<CreateState>,
    manage: Option<ManageState>,
    pending_delete_transaction: Option<u64>,
    /// The open transaction editor (Edit / Duplicate; New arrives with the asset picker).
    editor: Option<TransactionEditorState>,
    /// The asset search that starts "Add Transaction".
    picker: Option<AssetPickerState>,
    /// The asset being remapped (its key) and the search for its new market.
    remap: Option<(String, AssetPickerState)>,
    /// An asset the picker chose, waiting for the next frame's quotes to open the editor on it.
    new_asset: Option<PortfolioAsset>,
    /// The asset whose detail sheet is open.
    detail_key: Option<String>,
    detail: portfolio_asset_detail::AssetDetailState,
    notice: Option<String>,
}

impl PortfolioTab {
    pub fn new(id: u64, quote_feed: PortfolioQuoteFeed) -> Self {
        Self {
            id,
            import: PortfolioImportFlow::new(quote_feed.fx_rates()),
            pending_export: None,
            quote_feed,
            section: PortfolioSection::Overview,
            reporting_currency: PortfolioCurrency::Usd,
            privacy: false,
            prefs_loaded: false,
            loaded_selection: None,
            overview: OverviewState::default(),
            intraday: IntradayFeed::default(),
            holdings: HoldingsState::default(),
            transactions: TransactionsState::default(),
            create: None,
            manage: None,
            pending_delete_transaction: None,
            editor: None,
            picker: None,
            remap: None,
            new_asset: None,
            detail_key: None,
            detail: portfolio_asset_detail::AssetDetailState::default(),
            notice: None,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn title(&self) -> &'static str {
        "Portfolio"
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, host: &mut PortfolioHost) {
        let ctx = ui.ctx().clone();
        let snapshot = host.snapshot().clone();
        let selected = snapshot.selected_portfolio_id;
        if !self.prefs_loaded {
            self.privacy = host.privacy_mode();
            self.prefs_loaded = true;
        }
        if self.loaded_selection != Some(selected) {
            self.reporting_currency = host.reporting_currency_for(selected);
            self.loaded_selection = Some(selected);
        }
        let scope = portfolio_scope(&snapshot);
        let has_transactions = snapshot
            .transactions
            .iter()
            .any(|transaction| scope.contains(&transaction.portfolio_id));
        if !has_transactions {
            self.quote_feed.release(self.id);
        }

        egui::Panel::top(egui::Id::new(("portfolio-header", self.id)))
            .exact_size(62.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(egui::Margin::symmetric(16, 14)),
            )
            .show(ui, |ui| self.show_header(ui, host, &snapshot));

        // With no portfolio there is nothing to section: the page is the header and one call to
        // action.
        if has_portfolio_scope(&snapshot) {
            egui::Panel::top(egui::Id::new(("portfolio-sections", self.id)))
                .exact_size(64.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::background())
                        .inner_margin(egui::Margin::symmetric(16, 12)),
                )
                .show(ui, |ui| portfolio_sections(ui, &mut self.section, self.id));
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ui, |ui| {
                if scope.is_empty() {
                    empty_state(
                        ui,
                        Icon::Briefcase,
                        "No Portfolios",
                        "Create a portfolio to start tracking your investments.",
                        |ui| {
                            if header_controls::prominent_button(ui, Icon::Plus, "Create Portfolio")
                                .clicked()
                            {
                                self.create = Some(CreateState::default());
                            }
                        },
                    );
                    return;
                }
                if !has_transactions {
                    empty_state(
                        ui,
                        Icon::PieChart,
                        "Your portfolio is empty",
                        "Add a transaction, or import a CSV or CoinMarketCap export, to begin tracking your holdings.",
                        |ui| {
                            if header_controls::prominent_button(ui, Icon::Plus, "Add Transaction")
                                .clicked()
                            {
                                self.picker = Some(add_transaction_picker());
                            }
                            // The same two sources as the header's Import or Export menu.
                            let ctx = ui.ctx().clone();
                            header_controls::menu_button(
                                ui,
                                egui::Id::new(("portfolio-empty-import", self.id)),
                                Icon::Download,
                                "Import Transactions",
                                250.0,
                                false,
                                |ui| {
                                    if header_controls::action(
                                        ui,
                                        Icon::Download,
                                        "From a CSV File\u{2026}",
                                    ) {
                                        self.import.begin_import_csv(&ctx);
                                    }
                                    if header_controls::action(
                                        ui,
                                        Icon::Download,
                                        "From CoinMarketCap\u{2026}",
                                    ) {
                                        self.import
                                            .begin_import_cmc(&ctx, cmc_destination(&snapshot));
                                    }
                                },
                            );
                        },
                    );
                    // No holdings means no quotes yet: the editor opens without a market price.
                    if let Some(asset) = self.new_asset.take() {
                        self.open_new_editor(asset, &snapshot, selected, &HashMap::new());
                    }
                    return;
                }
                self.show_content(ui, host, &snapshot, selected);
            });

        self.show_sheets(&ctx, host);
        self.show_error(&ctx, host);
    }

    fn show_header(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut PortfolioHost,
        snapshot: &PortfolioLedgerSnapshot,
    ) {
        // Converting every transaction to the new reporting currency: the valuation is being
        // rebuilt from scratch, so the currency can't be changed again until it lands.
        let scope = portfolio_scope(snapshot);
        let quote_state = self.quote_feed.state();
        let converting = snapshot
            .transactions
            .iter()
            .any(|transaction| scope.contains(&transaction.portfolio_id))
            && quote_state.projection.is_none()
            && quote_state.is_updating;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let selected_name = snapshot
                .selected_portfolio_id
                .and_then(|id| {
                    snapshot
                        .portfolios
                        .iter()
                        .find(|portfolio| portfolio.id == id)
                })
                .map_or("All Portfolios", |portfolio| portfolio.name.as_str());
            header_controls::menu_button(
                ui,
                egui::Id::new(("portfolio-selector", self.id)),
                Icon::Briefcase,
                selected_name,
                230.0,
                false,
                |ui| {
                    if header_controls::choice(
                        ui,
                        "All Portfolios",
                        snapshot.selected_portfolio_id.is_none(),
                    ) {
                        let _ = host.select(None);
                    }
                    header_controls::separator(ui);
                    for portfolio in snapshot
                        .portfolios
                        .iter()
                        .filter(|portfolio| !portfolio.is_archived)
                    {
                        if header_controls::choice(
                            ui,
                            &portfolio.name,
                            snapshot.selected_portfolio_id == Some(portfolio.id),
                        ) {
                            let _ = host.select(Some(portfolio.id));
                        }
                    }
                    header_controls::separator(ui);
                    if header_controls::action(ui, Icon::Plus, "Create Portfolio\u{2026}") {
                        self.create = Some(CreateState::default());
                    }
                    if header_controls::action(ui, Icon::Sliders, "Manage Portfolios\u{2026}") {
                        self.manage = Some(ManageState::default());
                    }
                },
            );

            let chosen_before = self.reporting_currency;
            header_controls::menu_button(
                ui,
                egui::Id::new(("portfolio-currency", self.id)),
                Icon::Banknote,
                self.reporting_currency.code(),
                230.0,
                converting,
                |ui| {
                    for currency in PORTFOLIO_CURRENCIES {
                        if header_controls::choice(
                            ui,
                            &format!("{}  \u{00B7}  {}", currency.code(), currency.display_name()),
                            currency == chosen_before,
                        ) {
                            self.reporting_currency = currency;
                        }
                    }
                },
            );
            if self.reporting_currency != chosen_before {
                host.set_reporting_currency(
                    snapshot.selected_portfolio_id,
                    self.reporting_currency,
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                // Added right to left: with no portfolio the privacy toggle is all that is left.
                if has_portfolio_scope(snapshot) {
                    if header_controls::prominent_button(ui, Icon::Plus, "Add Transaction")
                        .clicked()
                    {
                        self.picker = Some(add_transaction_picker());
                    }
                    self.show_import_menu(ui, snapshot);
                }
                let privacy = header_controls::icon_button(
                    ui,
                    if self.privacy {
                        Icon::EyeSlash
                    } else {
                        Icon::Eye
                    },
                    if self.privacy {
                        "Reveal portfolio values"
                    } else {
                        "Hide portfolio values"
                    },
                    self.privacy,
                );
                if privacy.clicked() {
                    self.privacy = !self.privacy;
                    host.set_privacy_mode(self.privacy);
                }
            });
        });
    }

    /// The "Import or Export" menu in the header.
    fn show_import_menu(&mut self, ui: &mut egui::Ui, snapshot: &PortfolioLedgerSnapshot) {
        let ctx = ui.ctx().clone();
        header_controls::menu_button(
            ui,
            egui::Id::new(("portfolio-import-export", self.id)),
            Icon::Exchange,
            "Import or Export",
            250.0,
            false,
            |ui| {
                if header_controls::action(ui, Icon::Download, "Import Transactions\u{2026}") {
                    self.import.begin_import_csv(&ctx);
                }
                if header_controls::action(ui, Icon::Download, "Import from CoinMarketCap\u{2026}")
                {
                    self.import
                        .begin_import_cmc(&ctx, cmc_destination(snapshot));
                }
                header_controls::separator(ui);
                if header_controls::action(ui, Icon::ShareUp, "Export Transactions\u{2026}") {
                    let text = portfolio_csv::export_transactions(
                        &snapshot.transactions,
                        &snapshot.portfolios,
                    );
                    self.import.begin_export(
                        &ctx,
                        text,
                        crate::host::portfolio_import_flow::CSV_FILENAME,
                    );
                }
                if header_controls::action(ui, Icon::ShareUp, "Export Current Holdings\u{2026}") {
                    self.pending_export = Some(PendingExport::Holdings);
                }
                if header_controls::action(ui, Icon::ShareUp, "Export Portfolio History\u{2026}") {
                    self.pending_export = Some(PendingExport::History);
                }
            },
        );
    }

    fn show_content(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut PortfolioHost,
        snapshot: &PortfolioLedgerSnapshot,
        selected: Option<u64>,
    ) {
        let portfolio_ids: HashSet<u64> = portfolio_scope(snapshot);
        let now_ms = now_ms();
        self.quote_feed.update(
            self.id,
            snapshot,
            &portfolio_ids,
            self.reporting_currency,
            true,
            now_ms,
        );
        host.drive_history(ui.ctx(), &portfolio_ids, now_ms, local_offset());
        let quote_state = self.quote_feed.state();
        let projected = quote_state.projection.as_ref();
        if projected.is_none() || quote_state.is_updating {
            // Nothing else wakes the frame while a worker is still settling the prices.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(400));
        }
        let transactions: Vec<PortfolioTransaction> = projected.map_or_else(
            || {
                snapshot
                    .transactions
                    .iter()
                    .filter(|transaction| portfolio_ids.contains(&transaction.portfolio_id))
                    .cloned()
                    .collect()
            },
            |projection| projection.transactions.clone(),
        );
        let quotes = projected.map_or_else(HashMap::new, |projection| projection.quotes.clone());
        if let Some(asset) = self.new_asset.take() {
            self.open_new_editor(asset, snapshot, selected, &quotes);
        }
        let holding_values =
            holdings(&transactions, &portfolio_ids, None, &quotes, now_ms).unwrap_or_default();
        let daily_history: Vec<PortfolioSnapshot> =
            projected.map_or_else(Vec::new, |projection| projection.history.clone());
        // The 1D range replays the last day at 30-minute spacing when it can; otherwise (and for
        // every other tab) the daily history stands.
        let intraday = if self.section == PortfolioSection::Overview
            && self.overview.range == PortfolioHistoryRange::OneDay
            && projected.is_some()
        {
            let request = IntradayRequest {
                portfolios: snapshot
                    .portfolios
                    .iter()
                    .filter(|portfolio| portfolio_ids.contains(&portfolio.id))
                    .map(|portfolio| (portfolio.id, portfolio.base_currency))
                    .collect(),
                reporting_currency: self.reporting_currency,
                transactions: transactions.clone(),
            };
            self.intraday
                .points(ui.ctx(), &request, &self.quote_feed.fx_rates(), now_ms)
        } else {
            Vec::new()
        };
        let overview_history = if intraday.is_empty() {
            &daily_history
        } else {
            &intraday
        };
        let history = &daily_history;
        if let Some(export) = self.pending_export.take() {
            let (text, name) = match export {
                PendingExport::Holdings => (
                    portfolio_csv::export_holdings(
                        &holding_values,
                        selected
                            .and_then(|id| {
                                snapshot
                                    .portfolios
                                    .iter()
                                    .find(|portfolio| portfolio.id == id)
                            })
                            .map_or("All Portfolios", |portfolio| portfolio.name.as_str()),
                        now_ms,
                    ),
                    crate::host::portfolio_import_flow::HOLDINGS_FILENAME,
                ),
                PendingExport::History => (
                    portfolio_csv::export_history(history),
                    crate::host::portfolio_import_flow::HISTORY_FILENAME,
                ),
            };
            self.import.begin_export(ui.ctx(), text, name);
        }
        let selected_portfolio = selected.and_then(|id| {
            snapshot
                .portfolios
                .iter()
                .find(|portfolio| portfolio.id == id)
        });
        let formatter = PortfolioValueFormatter::new(self.reporting_currency, self.privacy);
        let subtitle = |asset: &crate::model::portfolio::PortfolioAsset| {
            if asset.name.is_empty() {
                asset.embedded_name().unwrap_or(&asset.symbol).to_owned()
            } else {
                asset.name.clone()
            }
        };
        let total_value: f64 = holding_values
            .iter()
            .filter_map(|holding| holding.current_value)
            .sum();
        let market_value_caption = market_value_caption(&quote_state, &holding_values);

        match self.section {
            PortfolioSection::Overview => {
                let actions = portfolio_overview::show_overview(
                    ui,
                    &mut self.overview,
                    &OverviewInput {
                        portfolio_name: selected_portfolio.map(|portfolio| portfolio.name.as_str()),
                        holdings: &holding_values,
                        history: overview_history,
                        total_value,
                        formatter,
                        subtitle: &subtitle,
                        market_value_caption: &market_value_caption,
                        is_updating: quote_state.is_updating,
                        is_loading_initial: projected.is_none(),
                        is_loading_history: host.is_rebuilding_history(&portfolio_ids),
                        now_ms,
                        utc_offset_secs: local_offset(),
                    },
                );
                for action in actions {
                    match action {
                        OverviewAction::Select(key) => {
                            self.transactions.filters.asset_key = Some(key);
                            self.section = PortfolioSection::Transactions;
                        }
                        OverviewAction::ShowHoldings => self.section = PortfolioSection::Holdings,
                    }
                }
            }
            PortfolioSection::Holdings => {
                let saved_sort = selected_portfolio
                    .map_or(PortfolioHoldingsSort::CurrentValue, |portfolio| {
                        portfolio.sort
                    });
                let actions = portfolio_holdings::show(
                    ui,
                    &mut self.holdings,
                    &HoldingsInput {
                        holdings: &holding_values,
                        saved_sort,
                        formatter,
                        is_loading: projected.is_none(),
                        subtitle: &subtitle,
                    },
                );
                for action in actions {
                    match action {
                        HoldingsAction::SetSort(sort) => {
                            if let Some(mut portfolio) = selected_portfolio.cloned() {
                                portfolio.sort = sort;
                                let _ = host.update_portfolio(portfolio);
                            }
                        }
                        HoldingsAction::Open(key) => self.detail_key = Some(key),
                        HoldingsAction::Remap(key) => {
                            let ticker = holding_values
                                .iter()
                                .find(|holding| holding.asset.key == key)
                                .map_or_else(
                                    || key.clone(),
                                    |holding| holding.asset.display_ticker(),
                                );
                            self.remap = Some((key, remap_picker(&ticker)));
                        }
                    }
                }
            }
            PortfolioSection::Transactions => {
                let projections: HashMap<u64, Projection> = transactions
                    .iter()
                    .map(|transaction| {
                        (
                            transaction.id,
                            Projection {
                                price: transaction.price,
                                fee: transaction.fee,
                            },
                        )
                    })
                    .collect();
                let projection = |id: u64| projections.get(&id).copied();
                let actions = portfolio_transactions::show(
                    ui,
                    &mut self.transactions,
                    &TransactionsInput {
                        ledger: &transactions,
                        projection: &projection,
                        subtitle: &subtitle,
                        formatter,
                        utc_offset_secs: local_offset(),
                    },
                );
                for action in actions {
                    match action {
                        TransactionAction::Delete(id) => self.pending_delete_transaction = Some(id),
                        TransactionAction::Edit(id) => {
                            self.open_editor(EditorMode::Edit, id, snapshot, &quotes)
                        }
                        TransactionAction::Duplicate(id) => {
                            self.open_editor(EditorMode::Duplicate, id, snapshot, &quotes)
                        }
                        TransactionAction::Add => self.picker = Some(add_transaction_picker()),
                    }
                }
            }
            PortfolioSection::Statistics => portfolio_overview::show_statistics(
                ui,
                &StatisticsInput {
                    holdings: &holding_values,
                    history,
                    transactions: &transactions,
                    formatter,
                    now_ms,
                    utc_offset_secs: local_offset(),
                },
            ),
        }

        if let Some(key) = self.detail_key.clone() {
            let ledger: Vec<&PortfolioTransaction> =
                portfolio_asset_detail::asset_transactions(&snapshot.transactions, &key)
                    .into_iter()
                    .filter(|transaction| portfolio_ids.contains(&transaction.portfolio_id))
                    .collect();
            let holding = holding_values
                .iter()
                .find(|holding| holding.asset.key == key);
            let asset = holding
                .map(|holding| &holding.asset)
                .or_else(|| ledger.first().map(|transaction| &transaction.asset));
            let name = asset.map(subtitle).unwrap_or_default();
            // The reporting-currency price of each transaction comes from the projected ledger.
            let projected_price: HashMap<u64, Option<f64>> = transactions
                .iter()
                .map(|transaction| (transaction.id, transaction.price))
                .collect();
            let price_of = |transaction: &PortfolioTransaction| {
                projected_price
                    .get(&transaction.id)
                    .copied()
                    .unwrap_or(transaction.price)
            };
            let actions = portfolio_asset_detail::show(
                ui.ctx(),
                &mut self.detail,
                &AssetDetailInput {
                    holding,
                    asset,
                    subtitle: &name,
                    transactions: &ledger,
                    price_of: &price_of,
                    formatter,
                    utc_offset_secs: local_offset(),
                },
            );
            for action in actions {
                match action {
                    AssetDetailAction::Edit(id) => {
                        self.open_editor(EditorMode::Edit, id, snapshot, &quotes)
                    }
                    AssetDetailAction::Duplicate(id) => {
                        self.open_editor(EditorMode::Duplicate, id, snapshot, &quotes)
                    }
                    AssetDetailAction::Delete(id) => self.pending_delete_transaction = Some(id),
                    AssetDetailAction::Close => self.detail_key = None,
                }
            }
        }
    }

    /// Opens the editor on a blank buy of `asset`, in the selected portfolio (else the first one).
    fn open_new_editor(
        &mut self,
        asset: PortfolioAsset,
        snapshot: &PortfolioLedgerSnapshot,
        selected: Option<u64>,
        quotes: &HashMap<String, crate::model::portfolio::PortfolioQuote>,
    ) {
        let Some(portfolio_id) = selected.or_else(|| {
            snapshot
                .portfolios
                .iter()
                .find(|portfolio| !portfolio.is_archived)
                .map(|portfolio| portfolio.id)
        }) else {
            return;
        };
        let now = now_ms();
        let currency = asset.quote_currency;
        let market = quotes
            .get(&asset.key)
            .filter(|_| currency == self.reporting_currency)
            .map(|quote| quote.price);
        let transaction = PortfolioTransaction {
            id: new_id(),
            portfolio_id,
            asset,
            kind: PortfolioTransactionType::Buy,
            quantity: 0.0,
            price: None,
            price_currency: currency,
            fee: 0.0,
            fee_currency: currency,
            timestamp: now,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: now,
            updated_at: now,
        };
        let mut editor = TransactionEditorState::new(EditorMode::New, transaction, local_offset());
        if let Some(price) = market {
            editor.set_market_price(price);
        }
        self.editor = Some(editor);
    }

    /// Opens the editor on a ledger transaction (the unprojected one, so nothing is converted
    /// behind the user's back). A duplicate is a new transaction with the same details.
    fn open_editor(
        &mut self,
        mode: EditorMode,
        id: u64,
        snapshot: &PortfolioLedgerSnapshot,
        quotes: &HashMap<String, crate::model::portfolio::PortfolioQuote>,
    ) {
        let Some(original) = snapshot
            .transactions
            .iter()
            .find(|transaction| transaction.id == id)
        else {
            return;
        };
        let mut transaction = original.clone();
        if mode == EditorMode::Duplicate {
            let now = now_ms();
            transaction.id = new_id();
            transaction.external_transaction_id = None;
            transaction.created_at = now;
            transaction.updated_at = now;
        }
        let mut editor = TransactionEditorState::new(mode, transaction, local_offset());
        // The Market button is offered only when the quote is in the transaction's own currency.
        if let Some(quote) = quotes.get(&original.asset.key)
            && original.price_currency == self.reporting_currency
        {
            editor.set_market_price(quote.price);
        }
        self.editor = Some(editor);
    }

    fn show_sheets(&mut self, ctx: &egui::Context, host: &mut PortfolioHost) {
        if let Some(state) = &mut self.create {
            let names: Vec<String> = host
                .snapshot()
                .portfolios
                .iter()
                .map(|portfolio| portfolio.name.clone())
                .collect();
            if let Some(outcome) = portfolio_sheets::show_create(ctx, state, &names) {
                match outcome {
                    CreateOutcome::Create { name, currency } => {
                        if host.create_portfolio(&name, currency).is_ok() {
                            self.create = None;
                        }
                    }
                    CreateOutcome::Cancel => self.create = None,
                }
            }
        }

        if self.manage.is_some() {
            let snapshot = host.snapshot().clone();
            let rows: Vec<ManageRow<'_>> = snapshot
                .portfolios
                .iter()
                .map(|portfolio| ManageRow {
                    portfolio,
                    transactions: snapshot
                        .transactions
                        .iter()
                        .filter(|transaction| transaction.portfolio_id == portfolio.id)
                        .count(),
                    assets: snapshot
                        .transactions
                        .iter()
                        .filter(|transaction| transaction.portfolio_id == portfolio.id)
                        .map(|transaction| transaction.asset.key.as_str())
                        .collect::<HashSet<_>>()
                        .len(),
                })
                .collect();
            let input = ManageInput {
                rows: &rows,
                current: snapshot.selected_portfolio_id,
            };
            let actions = portfolio_sheets::show_manage(
                ctx,
                self.manage.as_mut().expect("manage state exists"),
                &input,
            );
            for action in actions {
                match action {
                    ManageAction::Rename { id, name } => {
                        if let Some(mut portfolio) = snapshot
                            .portfolios
                            .iter()
                            .find(|portfolio| portfolio.id == id)
                            .cloned()
                        {
                            portfolio.name = name;
                            let _ = host.update_portfolio(portfolio);
                        }
                    }
                    ManageAction::Duplicate(id) => {
                        let _ = host.duplicate_portfolio(id);
                    }
                    ManageAction::Delete(id) => {
                        let _ = host.delete_portfolio(id);
                    }
                    ManageAction::Reorder { from, destination } => {
                        let _ = host.reorder(&from, destination);
                    }
                    ManageAction::New => self.create = Some(CreateState::default()),
                    ManageAction::Done => self.manage = None,
                }
            }
        }

        if let Some(picker) = &mut self.picker {
            match portfolio_asset_picker::show(ctx, picker) {
                Some(AssetPickerOutcome::Choose(result)) => {
                    self.new_asset = Some(asset_from_search_result(&result));
                    self.picker = None;
                }
                Some(AssetPickerOutcome::Cancel) => self.picker = None,
                None => {}
            }
        }

        if let Some((key, picker)) = &mut self.remap {
            match portfolio_asset_picker::show(ctx, picker) {
                Some(AssetPickerOutcome::Choose(result)) => {
                    let scope = portfolio_scope(host.snapshot());
                    let destination = asset_from_search_result(&result);
                    // A failure lands in the tab's alert; quotes and history follow the ledger.
                    let _ = host.remap_asset(key, &destination, &scope);
                    self.remap = None;
                }
                Some(AssetPickerOutcome::Cancel) => self.remap = None,
                None => {}
            }
        }

        self.import.show(ctx, host, local_offset());

        if let Some(editor) = &mut self.editor {
            match portfolio_transaction_editor::show(ctx, editor) {
                Some(TransactionEditorOutcome::Save { transaction, mode }) => {
                    let saved = if mode == EditorMode::Edit {
                        host.update_transaction(*transaction)
                    } else {
                        host.add_transaction(*transaction)
                    };
                    match saved {
                        Ok(()) => self.editor = None,
                        Err(error) => {
                            // Shown beside the form, not in the generic portfolio alert.
                            host.clear_error();
                            editor.failed(error.message());
                        }
                    }
                }
                Some(TransactionEditorOutcome::Cancel) => self.editor = None,
                None => {}
            }
        }

        if let Some(id) = self.pending_delete_transaction {
            let result = kit::confirm(
                ctx,
                egui::Id::new(("portfolio-delete-transaction", self.id)),
                &Confirm {
                    icon: Icon::Trash,
                    title: "Delete transaction?",
                    message: "Holdings, cost basis, P&L, allocation, and later history will be recalculated.",
                    destructive: "Delete",
                },
            );
            match result {
                Some(ConfirmOutcome::Confirmed) => {
                    if host.delete_transaction(id).is_ok() {
                        self.pending_delete_transaction = None;
                    }
                }
                Some(ConfirmOutcome::Cancelled) => self.pending_delete_transaction = None,
                None => {}
            }
        }
    }

    fn show_error(&mut self, ctx: &egui::Context, host: &mut PortfolioHost) {
        let message = host
            .last_error()
            .map(PortfolioHostError::message)
            .or_else(|| self.notice.clone());
        let Some(message) = message else { return };
        sheet::show(
            ctx,
            egui::Id::new(("portfolio-error", self.id)),
            350.0,
            |ui| {
                ui.add_space(20.0);
                sheet::inset(ui, |ui| {
                    sheet::header(ui, Icon::Triangle, "Portfolio", &message, theme::ORANGE);
                    ui.add_space(20.0);
                });
                ui.separator();
                sheet::inset(ui, |ui| {
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("OK").clicked() {
                            host.clear_error();
                            self.notice = None;
                        }
                    });
                    ui.add_space(12.0);
                });
            },
        );
    }
}

impl Drop for PortfolioTab {
    fn drop(&mut self) {
        self.quote_feed.release(self.id);
    }
}

/// The portfolios the tab is looking at: the selected one, else every unarchived one.
fn portfolio_scope(snapshot: &PortfolioLedgerSnapshot) -> HashSet<u64> {
    match snapshot.selected_portfolio_id {
        Some(id) => [id].into_iter().collect(),
        None => snapshot
            .portfolios
            .iter()
            .filter(|portfolio| !portfolio.is_archived)
            .map(|portfolio| portfolio.id)
            .collect(),
    }
}

/// Where a CoinMarketCap import lands: the selected portfolio and its currency. With "All
/// Portfolios" selected there is none, and the import flow asks the user to pick one.
fn cmc_destination(snapshot: &PortfolioLedgerSnapshot) -> Option<(u64, PortfolioCurrency)> {
    let id = snapshot.selected_portfolio_id?;
    snapshot
        .portfolios
        .iter()
        .find(|portfolio| portfolio.id == id)
        .map(|portfolio| (portfolio.id, portfolio.base_currency))
}

/// Whether the tab has a portfolio to act on. Adding, importing and exporting transactions all
/// need one: with none, `open_new_editor` has nowhere to put the transaction, a CSV row can only
/// name an existing portfolio, and there is nothing to export. The tab shows its "No Portfolios"
/// page and hides those controls in exactly this state.
fn has_portfolio_scope(snapshot: &PortfolioLedgerSnapshot) -> bool {
    !portfolio_scope(snapshot).is_empty()
}

fn remap_picker(ticker: &str) -> AssetPickerState {
    AssetPickerState::new(
        &format!("Remap {ticker}"),
        "Move this asset's transactions to a different market.",
        "Use Asset",
    )
    .with_icon(crate::ui::icons::Icon::Exchange)
}

fn add_transaction_picker() -> AssetPickerState {
    AssetPickerState::new(
        "Add Asset",
        "Choose the asset this transaction is for.",
        "Continue",
    )
}

fn market_value_caption(
    state: &PortfolioQuoteState,
    holdings: &[crate::model::portfolio::PortfolioHolding],
) -> String {
    if let Some(error) = &state.error {
        return format!("Market value unavailable · {error}");
    }
    let Some(projection) = &state.projection else {
        return "Loading market value…".to_owned();
    };
    let exchange_suffix = if projection.uses_cached_exchange_rates {
        " · Cached exchange rates"
    } else {
        ""
    };
    if projection.unpriced_asset_count > 0 {
        let count = projection.unpriced_asset_count;
        return format!(
            "+ {count} unpriced asset{}{exchange_suffix}",
            if count == 1 { "" } else { "s" }
        );
    }
    let label = if holdings.iter().any(|holding| holding.is_price_stale) {
        "Cached market value"
    } else {
        "Live market value"
    };
    format!("{label}{exchange_suffix}")
}

/// The section bar: Swift's regular `IconTabBar`, hugging its titles.
fn portfolio_sections(ui: &mut egui::Ui, selected: &mut PortfolioSection, id: u64) {
    let items = PortfolioSection::ALL.map(|section| kit::TabItem {
        value: section,
        title: section.label(),
        icon: section.icon(),
        count: 0,
    });
    kit::tab_bar_regular(
        ui,
        egui::Id::new(("portfolio-sections", id)),
        &items,
        selected,
    );
}

fn empty_state(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    detail: &str,
    actions: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(((ui.available_height() - 180.0) / 2.0).max(28.0));
    ui.vertical_centered(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::hover());
        icons::paint(
            ui.painter(),
            icon_rect.shrink(6.0),
            icon,
            theme::text_secondary(),
        );
        ui.add_space(12.0);
        ui.label(RichText::new(title).strong().size(20.0));
        ui.add_space(4.0);
        ui.label(RichText::new(detail).color(theme::text_secondary()));
        ui.add_space(16.0);
        // The row's width is whatever the actions measure (one button here, two there), so centre
        // it from the width it had last pass rather than from a guess. The first pass of a new
        // row has no width yet: pad nothing and ask egui to redo the pass before painting.
        let width_id = ui.id().with(("empty-state-actions", title));
        ui.horizontal(|ui| {
            // The header's gap between controls.
            ui.spacing_mut().item_spacing.x = 10.0;
            let known: Option<f32> = ui.data(|data| data.get_temp(width_id));
            ui.add_space(((ui.available_width() - known.unwrap_or(0.0)) / 2.0).max(0.0));
            let start = ui.cursor().min.x;
            actions(ui);
            let width = ui.min_rect().right() - start;
            if known.is_none_or(|known| (known - width).abs() > 0.5) {
                ui.data_mut(|data| data.insert_temp(width_id, width));
                ui.ctx().request_discard("centre the empty-state actions");
            }
        });
    });
}

/// The viewer's UTC offset now, for showing and typing dates in local time.
pub(crate) fn local_offset() -> i64 {
    crate::service::timezone::local_utc_offset_secs(now_ms())
}

pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioAsset, PortfolioSource, PortfolioTransactionType};
    use crate::store::Store;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("open store")))
    }

    fn transaction(id: u64, portfolio_id: u64, quantity: f64) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id,
            asset: PortfolioAsset {
                key: "binance:BTCUSDT".into(),
                symbol: "BTC".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            kind: PortfolioTransactionType::Buy,
            quantity,
            price: Some(50_000.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 1_000,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 1_000,
            updated_at: 1_000,
        }
    }

    #[test]
    fn reload_picks_up_what_sync_wrote_and_repairs_a_vanished_selection() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        assert!(host.snapshot().portfolios.is_empty());
        {
            let guard = store.lock().unwrap();
            let snapshot = PortfolioLedgerSnapshot {
                portfolios: vec![Portfolio::new(7, "From iCloud", 1)],
                transactions: vec![transaction(9, 7, 1.0)],
                selected_portfolio_id: Some(99),
                ..Default::default()
            };
            guard.save_portfolio_ledger(&snapshot).unwrap();
        }
        host.reload();
        assert_eq!(host.snapshot().portfolios.len(), 1);
        assert_eq!(host.snapshot().transactions.len(), 1);
        assert_eq!(host.snapshot().selected_portfolio_id, Some(7));
    }

    #[test]
    fn mutation_persists_before_publishing() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        let id = host
            .create_portfolio("Long-term", PortfolioCurrency::Eur)
            .expect("create");
        host.add_transaction(transaction(40, id, 1.0))
            .expect("add transaction");

        assert_eq!(
            host.snapshot().portfolios[0].base_currency,
            PortfolioCurrency::Eur
        );
        let persisted = store.lock().unwrap().portfolio_ledger().expect("reload");
        assert_eq!(persisted, *host.snapshot());
    }

    /// A host with one portfolio per entry of `archived`, and "All Portfolios" selected.
    fn host_with(archived: &[bool]) -> (PortfolioHost, Vec<u64>) {
        let mut host = PortfolioHost::load(store());
        let ids: Vec<u64> = archived
            .iter()
            .enumerate()
            .map(|(index, _)| {
                host.create_portfolio(&format!("Portfolio {index}"), PortfolioCurrency::Usd)
                    .expect("create")
            })
            .collect();
        for (id, archived) in ids.iter().zip(archived) {
            let mut portfolio = host
                .snapshot()
                .portfolios
                .iter()
                .find(|portfolio| portfolio.id == *id)
                .cloned()
                .expect("portfolio");
            portfolio.is_archived = *archived;
            host.update_portfolio(portfolio).expect("update");
        }
        host.select(None).expect("select all");
        (host, ids)
    }

    #[test]
    fn transaction_controls_need_a_portfolio_in_scope() {
        let (host, _) = host_with(&[]);
        assert!(!has_portfolio_scope(host.snapshot()), "no portfolios");

        let (host, _) = host_with(&[true, true]);
        assert!(
            !has_portfolio_scope(host.snapshot()),
            "everything archived and All selected is the empty page"
        );

        let (host, _) = host_with(&[false]);
        assert!(has_portfolio_scope(host.snapshot()));

        let (host, _) = host_with(&[true, false]);
        assert!(has_portfolio_scope(host.snapshot()), "one active is enough");

        let (mut host, ids) = host_with(&[true, true]);
        host.select(Some(ids[0])).expect("select archived");
        assert!(
            has_portfolio_scope(host.snapshot()),
            "a selected portfolio is in scope even when archived"
        );
    }

    #[test]
    fn a_cmc_import_lands_in_the_selected_portfolio_only() {
        let (mut host, ids) = host_with(&[false, false]);
        assert_eq!(cmc_destination(host.snapshot()), None, "All Portfolios");
        host.select(Some(ids[1])).expect("select");
        assert_eq!(
            cmc_destination(host.snapshot()),
            Some((ids[1], PortfolioCurrency::Usd))
        );
    }

    #[test]
    fn rejected_mutation_changes_neither_memory_nor_store() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        let id = host
            .create_portfolio("Main", PortfolioCurrency::Usd)
            .expect("create");
        let baseline = host.snapshot().clone();

        let error = host
            .add_transaction(transaction(1, id, 0.0))
            .expect_err("zero quantity");
        assert!(matches!(
            error,
            PortfolioHostError::Mutation(PortfolioError::InvalidQuantity)
        ));
        assert_eq!(host.snapshot(), &baseline);
        assert_eq!(store.lock().unwrap().portfolio_ledger().unwrap(), baseline);
    }

    #[test]
    fn selection_and_delete_are_durable() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        let first = host
            .create_portfolio("First", PortfolioCurrency::Usd)
            .unwrap();
        let second = host
            .create_portfolio("Second", PortfolioCurrency::Usd)
            .unwrap();
        host.select(Some(first)).unwrap();
        host.delete_portfolio(first).unwrap();

        assert_eq!(host.snapshot().selected_portfolio_id, Some(second));
        assert_eq!(
            store
                .lock()
                .unwrap()
                .portfolio_ledger()
                .unwrap()
                .selected_portfolio_id,
            Some(second)
        );
    }

    #[test]
    fn remapping_an_asset_moves_its_transactions_durably_and_only_in_scope() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        let first = host
            .create_portfolio("First", PortfolioCurrency::Usd)
            .unwrap();
        let second = host
            .create_portfolio("Second", PortfolioCurrency::Usd)
            .unwrap();
        host.add_transaction(transaction(10, first, 1.0)).unwrap();
        host.add_transaction(transaction(11, second, 2.0)).unwrap();
        let mut destination = transaction(0, first, 0.0).asset;
        destination.key = "coinbase:BTC-USD".into();
        destination.source = DataSource::Coinbase;

        let scope: HashSet<u64> = [first].into_iter().collect();
        host.remap_asset("binance:BTCUSDT", &destination, &scope)
            .unwrap();
        let key_of = |id: u64| {
            host.snapshot()
                .transactions
                .iter()
                .find(|transaction| transaction.id == id)
                .map(|transaction| transaction.asset.key.clone())
        };
        assert_eq!(key_of(10).as_deref(), Some("coinbase:BTC-USD"));
        assert_eq!(
            key_of(11).as_deref(),
            Some("binance:BTCUSDT"),
            "other portfolios are untouched"
        );
        let saved = store.lock().unwrap().portfolio_ledger().unwrap();
        assert!(
            saved
                .transactions
                .iter()
                .any(|t| t.id == 10 && t.asset.key == "coinbase:BTC-USD"),
            "persisted before it was published"
        );
        // Remapping to the same market, or an asset nobody holds, changes nothing and is not an error.
        host.remap_asset("coinbase:BTC-USD", &destination, &scope)
            .unwrap();
        host.remap_asset("none", &destination, &scope).unwrap();
        assert!(host.last_error().is_none());
    }

    #[test]
    fn privacy_and_reporting_currency_are_remembered_per_selection() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        let euro = host
            .create_portfolio("Euros", PortfolioCurrency::Eur)
            .unwrap();
        assert!(!host.privacy_mode());
        host.set_privacy_mode(true);
        assert!(host.privacy_mode());

        assert_eq!(
            host.reporting_currency_for(Some(euro)),
            PortfolioCurrency::Eur,
            "its own currency until chosen"
        );
        assert_eq!(host.reporting_currency_for(None), PortfolioCurrency::Usd);
        host.set_reporting_currency(Some(euro), PortfolioCurrency::Gbp);
        host.set_reporting_currency(None, PortfolioCurrency::Chf);
        assert_eq!(
            host.reporting_currency_for(Some(euro)),
            PortfolioCurrency::Gbp
        );
        assert_eq!(host.reporting_currency_for(None), PortfolioCurrency::Chf);

        // A new window reads the same answers from the store.
        let again = PortfolioHost::load(store);
        assert!(again.privacy_mode());
        assert_eq!(
            again.reporting_currency_for(Some(euro)),
            PortfolioCurrency::Gbp
        );
        assert_eq!(
            again.reporting_currency_for(Some(999)),
            PortfolioCurrency::Usd,
            "unknown selection"
        );
    }
}
