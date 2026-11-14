//! The Portfolio tab: its bar, section bar, pages and sheets over the app-wide `PortfolioHost`.
//! The tab keeps per-window presentation state (section, open sheets, the quote subscription);
//! every change to the ledger goes through the host.

mod bar;
mod import;

use crate::host::portfolio::import_flow::{self, PortfolioImportFlow};
use crate::host::portfolio::{PortfolioHost, PortfolioHostError, local_offset, now_ms};
use crate::model::portfolio;
use crate::model::portfolio::accounting::{holdings_and_closed, net_contributions};
use crate::model::portfolio::format::{PORTFOLIO_CURRENCIES, PortfolioValueFormatter};
use crate::model::portfolio::statistics::PortfolioHistoryRange;
use crate::model::portfolio::{
    PortfolioAsset, PortfolioCurrency, PortfolioHoldingsSort, PortfolioLedgerSnapshot,
    PortfolioSnapshot, PortfolioSource, PortfolioTransaction, PortfolioTransactionType,
};
use crate::model::watchlist::new_id;
use crate::service::portfolio::asset_auto_mapper::asset_from_search_result;
use crate::service::portfolio::history::{IntradayFeed, IntradayRequest};
use crate::service::portfolio::quotes::{PortfolioQuoteFeed, PortfolioQuoteState};
use crate::ui::icons::Icon;
use crate::ui::kit::header_controls;
use crate::ui::kit::sheet;
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::portfolio::asset_detail::{self, AssetDetailAction, AssetDetailInput};
use crate::ui::portfolio::asset_picker::{self, AssetPickerOutcome, AssetPickerState};
use crate::ui::portfolio::empty;
use crate::ui::portfolio::holdings::{self, HoldingsAction, HoldingsInput, HoldingsState};
use crate::ui::portfolio::overview::{
    self, OverviewAction, OverviewInput, OverviewState, PriceStatus,
};
use crate::ui::portfolio::sheets::{
    self, CreateOutcome, CreateState, ManageAction, ManageInput, ManageRow, ManageState,
};
use crate::ui::portfolio::statistics::{self, StatisticsInput};
use crate::ui::portfolio::transaction_editor::{
    self, EditorMode, TransactionEditorOutcome, TransactionEditorState,
};
use crate::ui::portfolio::transactions::{
    self, Projection, TransactionAction, TransactionsInput, TransactionsState,
};
use crate::ui::theme;
use eframe::egui;
use std::collections::{HashMap, HashSet};

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

    /// The picture and words the page shows while the portfolio has no transactions.
    fn empty_scene(self) -> empty::Scene {
        match self {
            Self::Overview => empty::Scene::Overview,
            Self::Holdings => empty::Scene::Holdings,
            Self::Transactions => empty::Scene::Transactions,
            Self::Statistics => empty::Scene::Statistics,
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
    detail: asset_detail::AssetDetailState,
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
            detail: asset_detail::AssetDetailState::default(),
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

        // With no portfolios at all the pickers, currency, privacy and import menus have nothing
        // to act on: the page is one call to action. (Everything archived keeps the header, its
        // picker is the way back to an archived portfolio.)
        if shows_header(&snapshot) {
            egui::Panel::top(egui::Id::new(("portfolio-header", self.id)))
                .exact_size(kit::bar::HEIGHT)
                .frame(egui::Frame::new().fill(theme::chrome()))
                .show(ui, |ui| self.show_bar(ui, host, &snapshot));
        }

        // With nothing in scope there is nothing to section: the page is one call to action.
        if has_portfolio_scope(&snapshot) {
            egui::Panel::top(egui::Id::new(("portfolio-sections", self.id)))
                .exact_size(64.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::background())
                        .inner_margin(egui::Margin::symmetric(0, 12)),
                )
                .show(ui, |ui| {
                    aligned(ui, |ui| portfolio_sections(ui, &mut self.section, self.id));
                });
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ui, |ui| {
                capped(ui, |ui| {
                    if scope.is_empty() {
                        empty::page(ui, empty::Scene::Portfolios, |ui| {
                            if header_controls::prominent_button(ui, Icon::Plus, "Create Portfolio")
                                .clicked()
                            {
                                self.create = Some(CreateState::default());
                            }
                        });
                        return;
                    }
                    if !has_transactions {
                        empty::page(ui, self.section.empty_scene(), |ui| {
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
                                &IMPORT_SOURCES,
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
                        });
                        // No holdings means no quotes yet: the editor opens without a market price.
                        if let Some(asset) = self.new_asset.take() {
                            self.open_new_editor(asset, &snapshot, selected, &HashMap::new());
                        }
                        return;
                    }
                    self.show_content(ui, host, &snapshot, selected);
                });
            });

        self.show_sheets(&ctx, host);
        self.show_error(&ctx, host);
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
        host.drive_history(
            ui.ctx(),
            &portfolio_ids,
            now_ms,
            &self.quote_feed.fx_rates(),
        );
        let quote_state = self.quote_feed.state();
        let projected = quote_state.projection.as_ref();
        if projected.is_none() || quote_state.is_updating || quote_state.refresh_pending {
            // Nothing else wakes the frame while a worker is still settling the prices, or
            // while price ticks wait for the next revaluation.
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
        // A ledger that cannot be replayed is not an empty portfolio: say so, and leave the
        // Transactions page usable so it can be fixed.
        let (holding_values, closed_positions, ledger_error) =
            match holdings_and_closed(&transactions, &portfolio_ids, None, &quotes, now_ms) {
                Ok((held, closed)) => (held, closed, None),
                Err(error) => (Vec::new(), Vec::new(), Some(error.message())),
            };
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
                    portfolio::csv::export_holdings(
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
                    import_flow::HOLDINGS_FILENAME,
                ),
                PendingExport::History => (
                    portfolio::csv::export_history(history),
                    import_flow::HISTORY_FILENAME,
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
        let subtitle = crate::ui::portfolio::transactions::asset_subtitle;
        let net_contributions = net_contributions(&transactions, &portfolio_ids, None);
        let total_value: f64 = holding_values
            .iter()
            .filter_map(|holding| holding.current_value)
            .sum();
        let market_value_caption = market_value_caption(&quote_state, &holding_values);

        let blocked = ledger_error
            .as_deref()
            .filter(|_| self.section != PortfolioSection::Transactions);
        if let Some(message) = blocked {
            crate::ui::portfolio::table::empty_state(
                ui,
                "These Transactions Can't Be Valued",
                &format!("{message} Review them in the Transactions tab."),
            );
        } else {
            match self.section {
                PortfolioSection::Overview => {
                    let actions = overview::show_overview(
                        ui,
                        &mut self.overview,
                        &OverviewInput {
                            portfolio_name: selected_portfolio
                                .map(|portfolio| portfolio.name.as_str()),
                            holdings: &holding_values,
                            history: overview_history,
                            total_value,
                            net_contributions,
                            formatter,
                            subtitle: &subtitle,
                            market_value_caption: &market_value_caption,
                            price_status: price_status(&quote_state, &holding_values),
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
                            OverviewAction::ShowHoldings => {
                                self.section = PortfolioSection::Holdings
                            }
                        }
                    }
                }
                PortfolioSection::Holdings => {
                    let saved_sort = selected_portfolio
                        .map_or(PortfolioHoldingsSort::CurrentValue, |portfolio| {
                            portfolio.sort
                        });
                    let actions = holdings::show(
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
                    let actions = transactions::show(
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
                            TransactionAction::Delete(id) => {
                                self.pending_delete_transaction = Some(id)
                            }
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
                PortfolioSection::Statistics => statistics::show_statistics(
                    ui,
                    &StatisticsInput {
                        holdings: &holding_values,
                        closed: &closed_positions,
                        history,
                        transactions: &transactions,
                        formatter,
                        now_ms,
                        utc_offset_secs: local_offset(),
                    },
                ),
            }
        }

        if let Some(key) = self.detail_key.clone() {
            let ledger: Vec<&PortfolioTransaction> =
                asset_detail::asset_transactions(&snapshot.transactions, &key)
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
            let actions = asset_detail::show(
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
            if let Some(outcome) = sheets::show_create(ctx, state, &names) {
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
            let actions = sheets::show_manage(
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
            match asset_picker::show(ctx, picker, host.store()) {
                Some(AssetPickerOutcome::Choose(result)) => {
                    self.new_asset = Some(asset_from_search_result(&result));
                    self.picker = None;
                }
                Some(AssetPickerOutcome::Cancel) => self.picker = None,
                None => {}
            }
        }

        if let Some((key, picker)) = &mut self.remap {
            match asset_picker::show(ctx, picker, host.store()) {
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

        import::show(ctx, &mut self.import, host, local_offset());

        if let Some(editor) = &mut self.editor {
            match transaction_editor::show(ctx, editor) {
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
                            editor.failed(error.to_string());
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
            .map(PortfolioHostError::to_string)
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

/// The empty page's import menu.
const IMPORT_SOURCES: [&str; 2] = ["From a CSV File\u{2026}", "From CoinMarketCap\u{2026}"];

/// The header's Import or Export menu, for sizing it.
const IMPORT_EXPORT_ACTIONS: [&str; 5] = [
    "Import Transactions\u{2026}",
    "Import from CoinMarketCap\u{2026}",
    "Export Transactions\u{2026}",
    "Export Current Holdings\u{2026}",
    "Export Portfolio History\u{2026}",
];

/// A currency's row in the currency menu: the code, then its name.
fn currency_choice(currency: PortfolioCurrency) -> String {
    format!("{}  \u{00B7}  {}", currency.code(), currency.display_name())
}

/// The header's controls act on portfolios; with none there is nothing for them to do.
fn shows_header(snapshot: &PortfolioLedgerSnapshot) -> bool {
    !snapshot.portfolios.is_empty()
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

/// The widest the page grows. A wider window keeps the header, the section bar and the page in one
/// centred column instead of stretching every row and table across the screen.
const CONTENT_MAX_WIDTH: f32 = 1480.0;

/// Runs `content` in the centred column at most [`CONTENT_MAX_WIDTH`] wide.
fn capped<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let full = ui.max_rect();
    let width = full.width().min(CONTENT_MAX_WIDTH);
    let column = egui::Rect::from_min_max(
        egui::pos2(full.center().x - width / 2.0, full.top()),
        egui::pos2(full.center().x + width / 2.0, full.bottom()),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(column), content)
        .inner
}

/// The side margin the pages keep inside the centred column (their cards, bands and tables).
const PAGE_MARGIN: f32 = 16.0;

/// Runs `content` in the centred column inset by [`PAGE_MARGIN`] on both sides: the same edges the
/// page below keeps, so the header's buttons and the section bar line up with its content at any
/// window width.
fn aligned<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    capped(ui, |ui| {
        let inset = ui.max_rect().shrink2(egui::vec2(PAGE_MARGIN, 0.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inset), content)
            .inner
    })
}

/// How current the prices behind the total are, for the dot beside the caption.
fn price_status(
    state: &PortfolioQuoteState,
    holdings: &[crate::model::portfolio::PortfolioHolding],
) -> PriceStatus {
    if state.error.is_some() {
        return PriceStatus::Unavailable;
    }
    let Some(projection) = &state.projection else {
        return PriceStatus::Live;
    };
    if projection.unpriced_asset_count > 0 {
        PriceStatus::Partial
    } else if projection.uses_cached_exchange_rates
        || holdings.iter().any(|holding| holding.is_price_stale)
    {
        PriceStatus::Cached
    } else {
        PriceStatus::Live
    }
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

#[cfg(test)]
mod tests;
