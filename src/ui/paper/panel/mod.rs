//! The Paper Trading panel under a chart window: header, notices, tables and footer over the
//! app-wide `PaperHost`, plus the account sheets. The panel keeps only presentation state; every
//! change to an account goes through the host.

use crate::host::paper::{PaperHost, PaperHostError, owner_text};
use crate::model::paper;
use crate::model::paper::{
    PaperAccount, PaperClosedTrade, PaperCurrency, PaperJournalEntry, PaperOrder, PaperOrderEvent,
    PaperPosition,
};
use crate::store::sync::Elsewhere;
use crate::ui::icons::{self, Icon};
use crate::ui::paper::account_sheet::{
    AccountSheetOutcome, PaperAccountSheetState, show as show_account_sheet,
};
use crate::ui::paper::header::{
    self, AccountChoice, PaperFooterAction, PaperFooterState, PaperHeaderAction, PaperHeaderInput,
    PaperHeaderState,
};
use crate::ui::paper::tables::{
    self, Context as TableContext, FooterFacts, PaperTab, PaperTableAction, PositionsInput,
    PositionsState,
};
use crate::ui::trade::panel_header::{self, ChipPick, Header, HeaderEvent, PanelEvent, VenueBar};
use crate::ui::trade::stats_strip;
use eframe::egui;

/// What a finished export tells the panel: the folder it wrote to, or why it did not.
type ExportOutcome = Result<Option<(std::path::PathBuf, String)>, String>;

/// What the panel lists for the selected account. Crate-visible for the host's timing test.
pub(crate) struct PanelView {
    revision: u64,
    account_id: Option<u64>,
    account: Option<PaperAccount>,
    accounts: Vec<AccountChoice>,
    positions: Vec<PaperPosition>,
    working_orders: Vec<PaperOrder>,
    pub(crate) events: Vec<PaperOrderEvent>,
    trades: Vec<PaperClosedTrade>,
    journal: Vec<PaperJournalEntry>,
}

impl PanelView {
    pub(crate) fn build(host: &PaperHost) -> Self {
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
    /// The working orders ticked in the table, by id.
    selected_orders: std::collections::BTreeSet<String>,
    account_sheet: Option<PaperAccountSheetState>,
    /// The account whose deletion is waiting for confirmation.
    pending_delete: Option<u64>,
    show_trading_on_charts: bool,
    notice: Option<PanelNotice>,
    /// The lists the panel shows for the selected account, rebuilt only when the host's data
    /// changed. Filtering and cloning the whole history every frame cost far more than drawing it.
    cached_view: Option<std::sync::Arc<PanelView>>,
    /// The folder dialog and the writing behind it, on a worker thread.
    export_job: Option<crate::service::work_queue::Pending<ExportOutcome>>,
    /// Set by the header menu and the footer; taken at the end of the frame.
    export_requested: bool,
}

impl Default for PaperPanel {
    fn default() -> Self {
        Self {
            header: PaperHeaderState::default(),
            footer: PaperFooterState::default(),
            positions: PositionsState::default(),
            selected_orders: std::collections::BTreeSet::new(),
            account_sheet: None,
            pending_delete: None,
            show_trading_on_charts: true,
            notice: None,
            cached_view: None,
            export_job: None,
            export_requested: false,
        }
    }
}

impl PaperPanel {
    pub fn show_trading_on_charts(&self) -> bool {
        self.show_trading_on_charts
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut PaperHost,
        bar: &VenueBar,
    ) -> Vec<PanelEvent> {
        let mut panel_actions = Vec::new();
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
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

        let input = PaperHeaderInput {
            accounts,
            selected: account_id,
            currency: account.map(|account| account.base_currency),
            position_count: positions.len(),
            working_order_count: working_orders.len(),
        };
        let parts = header::header_parts(&input);
        let header_events = panel_header::show(
            ui,
            &Header {
                bar,
                chip: &parts.chip,
                tabs: &parts.tabs,
                overlay: Some(self.show_trading_on_charts),
                can_order: account.is_some(),
                more: &parts.more,
                settings_tip: "Paper trading settings",
            },
            &mut self.header.tab,
        );
        let mut header_actions = Vec::new();
        for event in header_events {
            match event {
                HeaderEvent::Panel(event) => panel_actions.push(event),
                HeaderEvent::Chip(ChipPick::Row(index)) => {
                    if let Some(choice) = accounts.get(index) {
                        header_actions.push(PaperHeaderAction::SelectAccount(choice.id));
                    }
                }
                HeaderEvent::Chip(ChipPick::Add) => {
                    header_actions.push(PaperHeaderAction::NewAccount)
                }
                HeaderEvent::ToggleOverlay => {
                    header_actions.push(PaperHeaderAction::ToggleShowOnCharts);
                }
                HeaderEvent::More(index) => {
                    header_actions.extend(parts.more_actions.get(index).cloned())
                }
            }
        }
        divider(ui);
        self.poll_export();
        self.show_notice(ui);
        if let Some(elsewhere) = account_id.and_then(|id| host.owned_elsewhere(id)) {
            owner_note(ui, elsewhere);
        }
        if let Some(error) = host.last_error() {
            error_banner(ui, &error.to_string());
        }

        let mut table_actions = Vec::new();
        let mut footer_actions = Vec::new();
        if account.is_none() {
            if header::no_account_state(ui) {
                self.account_sheet = Some(PaperAccountSheetState::create());
            }
        } else {
            let strip = crate::trading::paper::stats::stats(metrics.as_ref(), currency);
            stats_strip::show(ui, Some(&strip));
            divider(ui);
            let context = TableContext {
                currency,
                // The offset now, as the portfolio tables use: rows from the other side of a
                // daylight-saving change show an hour off.
                utc_offset_secs: crate::service::system::timezone::local_utc_offset_secs(
                    crate::model::watchlist::now_ms(),
                ),
            };
            match self.header.tab {
                PaperTab::Positions => {
                    table_actions = tables::positions_table(
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
                    let rows: Vec<crate::trading::model::OrderRow> = working_orders
                        .iter()
                        .map(crate::trading::paper::backend::order_row)
                        .collect();
                    let asked = crate::ui::trade::tables::orders(
                        ui,
                        "paper-orders",
                        &rows,
                        &mut self.selected_orders,
                        false,
                    );
                    table_actions = asked
                        .into_iter()
                        .filter_map(|action| match action {
                            crate::ui::trade::tables::OrderAction::Cancel(id) => {
                                id.parse().ok().map(PaperTableAction::CancelOrder)
                            }
                            _ => None,
                        })
                        .collect();
                }
                PaperTab::History => tables::history_table(ui, events, context),
                PaperTab::Trades => tables::trades_table(ui, trades, context),
                PaperTab::Journal => tables::journal_table(ui, journal, context),
            }
            footer_actions = header::footer(
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
                self.selected_orders.len(),
            );
        }

        self.apply_header_actions(header_actions, host, account);
        self.apply_table_actions(table_actions, host);
        self.apply_footer_actions(footer_actions, host, account_id);
        if std::mem::take(&mut self.export_requested) {
            self.begin_export(ui.ctx(), host, account_id);
        }
        self.show_account_sheet(ui.ctx(), host, account_id);
        self.show_delete_confirm(ui.ctx(), host);
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
        let export = paper::export::export_account(host.snapshot(), account_id);
        if export.is_empty() {
            self.notice = Some(PanelNotice::info(
                "Nothing to export yet. Orders, fills and closed trades show up here once you trade.",
            ));
            return;
        }
        let job = crate::service::work_queue::SNAPSHOTS.start(Some(ctx), move || {
            crate::service::system::file_dialog::choose_folder("Export").and_then(|folder| {
                let Some(folder) = folder else {
                    return Ok(None);
                };
                for (name, text) in export.files() {
                    crate::service::system::file_dialog::write_text(&folder.join(name), text)?;
                }
                Ok(Some((folder, export.summary())))
            })
        });
        self.export_job = Some(job);
        self.notice = None;
    }

    fn poll_export(&mut self) {
        let Some(job) = &mut self.export_job else {
            return;
        };
        match job.poll() {
            Some(Ok(Some((folder, summary)))) => {
                let name = folder.file_name().map_or_else(
                    || folder.display().to_string(),
                    |n| n.to_string_lossy().into(),
                );
                self.notice = Some(PanelNotice::info(format!(
                    "Exported {summary} to \u{201c}{name}\u{201d}."
                )));
                self.export_job = None;
            }
            Some(Ok(None)) => self.export_job = None,
            Some(Err(message)) => {
                self.notice = Some(PanelNotice::error(message));
                self.export_job = None;
            }
            // Still open: the job repaints when it ends.
            None if job.is_pending() => {}
            None => self.export_job = None,
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
            1 => first.to_string(),
            more => format!("{more} failed: {first} (and {} more)", more - 1),
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
                    self.pending_delete = account.map(|account| account.id);
                }
                PaperHeaderAction::ToggleShowOnCharts => {
                    self.show_trading_on_charts = !self.show_trading_on_charts;
                }
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
                (PaperFooterAction::CancelSelectedOrders, _) => {
                    let ids = std::mem::take(&mut self.selected_orders);
                    let failures: Vec<PaperHostError> = ids
                        .iter()
                        .filter_map(|id| id.parse::<u64>().ok())
                        .filter_map(|id| host.cancel(id).err())
                        .collect();
                    self.report_failures(host, &failures);
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
                        Some(id) => {
                            host.reset_account(id, result.currency, result.balance, result.settings)
                        }
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
                            sheet.submit_error = Some(error.to_string());
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
        match crate::ui::paper::header::delete_confirm(context, &name) {
            Some(crate::ui::kit::ConfirmOutcome::Confirmed) => {
                self.pending_delete = None;
                if let Err(error) = host.delete_account(id) {
                    self.report_failures(host, &[error]);
                }
            }
            Some(crate::ui::kit::ConfirmOutcome::Cancelled) => self.pending_delete = None,
            None => {}
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
        "{verb} {} of {} {noun}. {first}{more}",
        results.len() - failures.len(),
        results.len(),
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
mod tests;
