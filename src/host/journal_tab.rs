//! The Trade Journal tab: per-window presentation over the app-wide [`JournalHost`]. The header
//! (range, account, filters), the trade table and the trade panel. Everything it shows is read
//! from the host's cached trades; nothing here touches SQLite or paper trading except to save a
//! review through the host.

use std::sync::Arc;

use eframe::egui::{self, Key};
use rust_decimal::Decimal;

use crate::host::journal_excursion::{CandleFetcher, Excursions, State as ExcursionState};
use crate::host::journal_host::{
    JournalData, JournalHost, JournalPrefs, RangeChoice, Section, now_ms,
};
use crate::host::journal_io::{Io, Outcome as IoOutcome};
use crate::host::journal_shots::{Shots, Source};
use crate::service::timezone::local_utc_offset_secs;
use crate::ui::header_controls;
use crate::ui::icons::Icon;
use crate::ui::journal_analytics;
use crate::ui::journal_calendar;
use crate::ui::journal_compare;
use crate::ui::journal_daily;
use crate::ui::journal_detail::{self, Draft};
use crate::ui::journal_import::{self, ImportSheet};
use crate::ui::journal_overview::{self, OverviewState};
use crate::ui::journal_playbooks;
use crate::ui::journal_shots as shots_ui;
use crate::ui::journal_table::{self, TableSpec};
use crate::ui::kit::{self, REGULAR, SEMIBOLD, size};
use crate::ui::paper_grid;
use crate::ui::theme;
use degenview_journal::backup::Backup;
use degenview_journal::breakdown::{self, Dimension, Slice};
use degenview_journal::calendar::{
    self, Civil, MonthView, days_from_civil, month_view, shift_month,
};
use degenview_journal::cells::{self, CellContext, Tone};
use degenview_journal::compare::{self, AccountColumn};
#[cfg(test)]
use degenview_journal::daily::DailyEntry;
use degenview_journal::filters::{
    OutcomeFilter, RangePreset, TradeFilter, outcome_of, previous_window, range_for,
};
use degenview_journal::format;
use degenview_journal::model::{Direction, Trade};
use degenview_journal::overview::{
    Input as OverviewInput, Overview, accounts_in_scope, compute, currencies_of,
};
use degenview_journal::playbook::Playbook;
use degenview_journal::process::{ProcessScore, process_score};
use degenview_journal::table::{Column, Sort, SortContext, sort_indices};

const PAGE_MARGIN: f32 = 16.0;
const CONTENT_MAX_WIDTH: f32 = 1480.0;
const DETAIL_WIDTH: f32 = 440.0;
/// Weeks start on Sunday, as the calendar the journal is modelled on does.
const WEEK_START: u8 = 0;
/// A review saves this long after the last keystroke, or when its field loses focus.
const SAVE_AFTER: f64 = 0.8;
const BREAKEVEN_TOLERANCE: Decimal = Decimal::ZERO;

/// What the Data menu can do.
enum DataAction {
    Import,
    ExportTrades,
    ExportExecutions,
    ExportSummary,
    ExportNotes,
    Backup,
    Restore,
    RemoveAccount(String, String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum JournalAction {
    /// Run every paper fill through the journal again.
    RetryIngest,
    /// Show a trade on a chart: its market, and the span from its first fill to its last (or to
    /// now while it is open).
    ViewOnChart {
        instrument: crate::model::InstrumentId,
        from_ms: i64,
        to_ms: i64,
    },
}

/// The chart market a trade was made on, when its provider is one the app charts.
fn chart_market(trade: &Trade) -> Option<crate::model::InstrumentId> {
    let source = trade
        .instrument
        .provider
        .as_deref()
        .and_then(crate::model::DataSource::from_display_name)?;
    Some(crate::model::InstrumentId::new(
        source,
        &trade.instrument.symbol,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RangeKind {
    Preset(RangePreset),
    Custom,
}

/// What a cached overview or month was computed from.
#[derive(Clone, PartialEq)]
struct StatsKey {
    build_id: u64,
    reviews_revision: u64,
    filter: TradeFilter,
    currency: Option<Option<String>>,
    month: Option<(i32, u8)>,
}

/// What the cached row order was computed from.
#[derive(Clone, PartialEq)]
struct RowsKey {
    build_id: u64,
    reviews_revision: u64,
    filter: TradeFilter,
    sort: Sort,
}

pub struct JournalTab {
    id: u64,
    filter: TradeFilter,
    search: String,
    custom_from: String,
    custom_to: String,
    selected: Option<String>,
    rows: Vec<usize>,
    rows_key: Option<RowsKey>,
    draft: Option<Draft>,
    scroll_to: Option<usize>,
    prefs_applied: bool,
    /// The resolved range is refreshed every so often, not every frame.
    range_cache: Option<(i64, Option<i64>, Option<i64>)>,
    overview_state: OverviewState,
    overview: Option<(StatsKey, Overview)>,
    breakdown_dimension: Dimension,
    breakdown: Option<(StatsKey, Dimension, Vec<Slice>)>,
    compare: Option<(StatsKey, Vec<AccountColumn>)>,
    process: Option<(StatsKey, ProcessScore)>,
    playbook_stats: Option<(StatsKey, Vec<Slice>, std::collections::HashMap<String, f64>)>,
    playbook_selected: Option<String>,
    playbook_draft: Option<journal_playbooks::Draft>,
    playbook_delete: Option<String>,
    daily_date: Option<Civil>,
    daily_draft: Option<journal_daily::Draft>,
    month: Option<(StatsKey, MonthView)>,
    /// The month the calendar shows; `None` is the current one.
    calendar_month: Option<(i32, u8)>,
    /// The currency the overview and calendar describe when the selection has several.
    currency: Option<Option<String>>,
    shots: Shots,
    /// Actions for the app raised while drawing, handed over at the end of the frame.
    outbox: Vec<JournalAction>,
    io: Io,
    import: Option<ImportSheet>,
    /// Rows the import sheet already refused, for the result message.
    import_skipped: usize,
    restore: Option<(String, Backup)>,
    remove_account: Option<(String, String)>,
    /// A result or an error to tell the user: (is an error, text).
    notice: Option<(bool, String)>,
    excursions: Excursions,
    fetcher: std::sync::Arc<dyn CandleFetcher>,
}

impl JournalTab {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            filter: TradeFilter::default(),
            search: String::new(),
            custom_from: String::new(),
            custom_to: String::new(),
            selected: None,
            rows: Vec::new(),
            rows_key: None,
            draft: None,
            scroll_to: None,
            prefs_applied: false,
            range_cache: None,
            overview_state: OverviewState::default(),
            overview: None,
            breakdown_dimension: Dimension::Weekday,
            breakdown: None,
            compare: None,
            process: None,
            playbook_stats: None,
            playbook_selected: None,
            playbook_draft: None,
            playbook_delete: None,
            daily_date: None,
            daily_draft: None,
            month: None,
            calendar_month: None,
            currency: None,
            shots: Shots::default(),
            outbox: Vec::new(),
            io: Io::default(),
            import: None,
            import_skipped: 0,
            restore: None,
            remove_account: None,
            notice: None,
            excursions: Excursions::default(),
            fetcher: crate::host::journal_excursion::default_fetcher(),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn title(&self) -> &'static str {
        "Trade Journal"
    }

    /// Shows one trade: clears the range and filters that could hide it and opens its panel. What
    /// a click on a chart's trade does.
    pub fn show_trade(&mut self, key: &str, host: &mut JournalHost) {
        self.reset_filters(host);
        host.prefs_mut().section = Section::Trades;
        self.selected = Some(key.to_owned());
    }

    /// Opens the trade panel on one trade.
    #[cfg(test)]
    pub fn select_trade(&mut self, key: &str) {
        self.selected = Some(key.to_owned());
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        paper_revision: u64,
    ) -> Vec<JournalAction> {
        let mut actions = Vec::new();
        host.sync(paper_revision);
        host.poll();
        if !self.prefs_applied {
            self.apply_prefs(host.prefs());
            self.prefs_applied = true;
        }

        let Some(data) = host.data().cloned() else {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::background()))
                .show(ui, |ui| match host.load_error() {
                    Some(error) => {
                        let error = error.to_owned();
                        if message_page(
                            ui,
                            Icon::TriangleFill,
                            "The journal could not be read",
                            &error,
                            Some("Retry"),
                        ) {
                            host.mark_dirty();
                        }
                    }
                    None => loading_page(ui),
                });
            return actions;
        };

        // The ledger moved under an open panel, or the trade vanished: drop a stale selection.
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !data.trades.iter().any(|trade| &trade.key == key))
        {
            self.close_trade(host);
        }

        let (range_row, filter_row) = Self::header_rows(host.prefs().section);
        let header_height =
            24.0 + 36.0 + 44.0 * f32::from(u8::from(range_row) + u8::from(filter_row));
        egui::Panel::top(egui::Id::new(("journal-header", self.id)))
            .exact_size(header_height)
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(egui::Margin::symmetric(0, 12)),
            )
            .show(ui, |ui| aligned(ui, |ui| self.header(ui, host, &data)));

        if let Some(error) = data.ingest_error.clone() {
            egui::Panel::top(egui::Id::new(("journal-error", self.id)))
                .frame(egui::Frame::new().fill(theme::background()))
                .show(ui, |ui| {
                    aligned(ui, |ui| {
                        ui.add_space(4.0);
                        kit::notice_card(
                            ui,
                            Icon::TriangleFill,
                            kit::RED,
                            "Some paper fills could not be added to the journal",
                            Some(&error),
                            None,
                        );
                        if kit::sheet_button(ui, "Retry", false, true).clicked() {
                            actions.push(JournalAction::RetryIngest);
                        }
                        ui.add_space(6.0);
                    })
                });
        }

        let reviews = host.reviews().clone();
        self.refresh_rows(host, &data, &reviews);
        let section = host.prefs().section;
        if section != Section::Trades && self.selected.is_some() {
            self.close_trade(host);
        }
        if section != Section::Playbooks {
            self.flush_playbook(host);
            self.playbook_draft = None;
        }
        if section != Section::Daily {
            self.flush_daily(host);
            self.daily_draft = None;
        }
        self.io_ui(ui.ctx(), host);
        match section {
            Section::Trades => self.trades_section(ui, host, &data, &reviews),
            Section::Overview => self.overview_section(ui, host, &data),
            Section::Calendar => self.calendar_section(ui, host, &data),
            Section::Analytics => self.analytics_section(ui, host, &data),
            Section::Compare => self.compare_section(ui, host, &data),
            Section::Playbooks => self.playbooks_section(ui, host, &data),
            Section::Daily => self.daily_section(ui, host, &data),
        }
        host.flush_prefs_if_idle(ui.ctx());
        actions.append(&mut self.outbox);
        actions
    }

    fn trades_section(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
        reviews: &Arc<std::collections::HashMap<String, degenview_journal::reviews::Review>>,
    ) {
        let data = data.clone();
        let reviews = reviews.clone();

        // The trade panel, on the right.
        let selected_row = self.selected.as_ref().and_then(|key| {
            self.rows
                .iter()
                .position(|&index| &data.trades[index].key == key)
        });
        if let Some(key) = self.selected.clone()
            && let Some(trade) = data.trades.iter().find(|trade| trade.key == key)
        {
            if self
                .draft
                .as_ref()
                .is_none_or(|draft| draft.review.trade_key != key)
            {
                self.flush_draft(host);
                self.draft = Some(Draft::new(trade, reviews.get(&key)));
            }
            let mut nav = None;
            let mut close = false;
            let emotions = degenview_journal::reviews::suggestions(
                &degenview_journal::reviews::DEFAULT_EMOTIONS,
                reviews.values().flat_map(|review| review.emotions.iter()),
            );
            let mistakes = degenview_journal::reviews::suggestions(
                &degenview_journal::reviews::DEFAULT_MISTAKES,
                reviews.values().flat_map(|review| review.mistakes.iter()),
            );
            if let Some(hash) = self.shots.poll(ui.ctx()) {
                let draft = self.draft.as_mut().expect("draft");
                if !draft.review.images.contains(&hash) {
                    draft.review.images.push(hash);
                    // Attaching is a deliberate act: it is saved at once, not after a pause.
                    draft.edited_at = Some(ui.input(|input| input.time) - SAVE_AFTER);
                }
            }
            let shot_list: Vec<shots_ui::Shot> = {
                let hashes = self.draft.as_ref().expect("draft").review.images.clone();
                self.shots.keep_only(&hashes);
                hashes
                    .into_iter()
                    .map(|hash| shots_ui::Shot {
                        thumb: self.shots.thumb(ui.ctx(), &hash),
                        hash,
                    })
                    .collect()
            };
            self.excursions.poll();
            if !trade.is_open() {
                let repaint = ui.ctx().clone();
                self.excursions.request(
                    trade,
                    &host.store(),
                    self.fetcher.clone(),
                    move || repaint.request_repaint(),
                    false,
                );
            }
            let excursion_state = self.excursions.state(trade).cloned();
            let mut retry_excursion = false;
            let mut view_on_chart = false;
            let (shots_busy, shots_error) = (self.shots.busy(), self.shots.error.clone());
            let mut shot_action = shots_ui::Action::default();
            egui::Panel::right(egui::Id::new(("journal-detail", self.id)))
                .default_size(DETAIL_WIDTH)
                .size_range(360.0..=680.0)
                .resizable(true)
                .frame(
                    egui::Frame::new()
                        .fill(theme::background())
                        .inner_margin(egui::Margin::symmetric(16, 14)),
                )
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let account_name = data
                                .account(&trade.account_uid)
                                .map_or(trade.account_uid.as_str(), |account| {
                                    account.name.as_str()
                                });
                            let draft = self.draft.as_mut().expect("draft");
                            let mut output = journal_detail::show(
                                ui,
                                &journal_detail::Input {
                                    trade,
                                    account_name,
                                    breakeven_tolerance: BREAKEVEN_TOLERANCE,
                                    offset: &local_utc_offset_secs,
                                    position: (
                                        selected_row.map_or(0, |row| row + 1),
                                        self.rows.len(),
                                    ),
                                    playbooks: host.playbooks(),
                                    emotion_suggestions: &emotions,
                                    mistake_suggestions: &mistakes,
                                    account_balance: data
                                        .account(&trade.account_uid)
                                        .and_then(|account| account.initial_balance),
                                    can_view_on_chart: chart_market(trade).is_some(),
                                    shots: &shot_list,
                                    shots_busy,
                                    shots_error: shots_error.as_deref(),
                                    excursion: match &excursion_state {
                                        _ if trade.is_open() => {
                                            journal_detail::ExcursionView::Hidden
                                        }
                                        None | Some(ExcursionState::Loading) => {
                                            journal_detail::ExcursionView::Loading
                                        }
                                        Some(ExcursionState::Done(Err(message))) => {
                                            journal_detail::ExcursionView::Failed(message)
                                        }
                                        Some(ExcursionState::Done(Ok(metric))) => {
                                            journal_detail::ExcursionView::Ready(metric)
                                        }
                                    },
                                },
                                draft,
                            );
                            shot_action = std::mem::take(&mut output.shots);
                            retry_excursion = output.retry_excursion;
                            view_on_chart = output.view_on_chart;
                            let now = ui.input(|input| input.time);
                            if output.changed {
                                draft.edited_at = Some(now);
                                draft.error = None;
                            }
                            let due = draft.edited_at.is_some_and(|at| now - at >= SAVE_AFTER);
                            if draft.is_dirty()
                                && (due || output.blurred && draft.edited_at.is_some())
                            {
                                match host.save_review(&draft.review) {
                                    Ok(()) => {
                                        draft.saved = draft.review.clone();
                                        draft.edited_at = None;
                                        draft.saved_until = Some(now + 2.0);
                                        draft.error = None;
                                        ui.ctx().request_repaint_after(
                                            std::time::Duration::from_secs(3),
                                        );
                                    }
                                    Err(error) => {
                                        draft.error = Some(error);
                                        draft.edited_at = None;
                                    }
                                }
                            } else if draft.is_dirty() {
                                ui.ctx()
                                    .request_repaint_after(std::time::Duration::from_millis(900));
                            }
                            if output.previous {
                                nav = Some(-1);
                            }
                            if output.next {
                                nav = Some(1);
                            }
                            close = output.close;
                        });
                });
            if retry_excursion {
                let repaint = ui.ctx().clone();
                self.excursions.request(
                    trade,
                    &host.store(),
                    self.fetcher.clone(),
                    move || repaint.request_repaint(),
                    true,
                );
            }
            if view_on_chart && let Some(instrument) = chart_market(trade) {
                self.outbox.push(JournalAction::ViewOnChart {
                    instrument,
                    from_ms: trade.open_ts_ms,
                    to_ms: trade.close_ts_ms.unwrap_or_else(now_ms),
                });
            }
            self.handle_shots(ui, host, shot_action, &shot_list);
            if let Some(step) = nav {
                self.step(step, &data);
            }
            if close {
                self.close_trade(host);
            }
        } else {
            self.flush_draft(host);
            self.draft = None;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| {
                aligned(ui, |ui| {
                    self.keyboard(ui, &data);
                    if data.trades.is_empty() {
                        message_page(
                            ui,
                            Icon::Journal,
                            "Your trades appear here",
                            "Place a paper trade on a chart. Every execution is recorded \
                             automatically and grouped into trades you can review.",
                            None,
                        );
                    } else if self.rows.is_empty() {
                        if message_page(
                            ui,
                            Icon::Filter,
                            "No trades match",
                            "Nothing in the journal fits the range and filters.",
                            Some("Reset filters"),
                        ) {
                            self.reset_filters(host);
                        }
                    } else {
                        self.table(ui, host, &data, &reviews);
                    }
                })
            });
    }

    // MARK: Playbooks and the daily journal

    /// The currency chips' choice for `selected`.
    fn choose_currency(&self, selected: &[&Trade]) -> (Vec<Option<String>>, Option<String>) {
        let currencies = currencies_of(selected);
        let current = self
            .currency
            .clone()
            .filter(|chosen| currencies.contains(chosen))
            .or_else(|| currencies.first().cloned())
            .unwrap_or(None);
        (currencies, current)
    }

    fn playbooks_section(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
    ) {
        let key = self.overview_key(host, data);
        let selected: Vec<&Trade> = self.rows.iter().map(|&i| &data.trades[i]).collect();
        let (currencies, current) = self.choose_currency(&selected);
        if self
            .playbook_stats
            .as_ref()
            .is_none_or(|(cached, _, _)| cached != &key)
        {
            let mine: Vec<&Trade> = selected
                .iter()
                .copied()
                .filter(|trade| trade.currency == current)
                .collect();
            let reviews = host.reviews().clone();
            let slices = breakdown::breakdown(
                &mine,
                Dimension::Strategy,
                &breakdown::Context {
                    reviews: &reviews,
                    playbook_names: host.playbook_names(),
                    offset: &local_utc_offset_secs,
                    breakeven_tolerance: BREAKEVEN_TOLERANCE,
                },
            );
            let mut adherence: std::collections::HashMap<String, f64> = Default::default();
            for playbook in host.playbooks().iter() {
                let shares: Vec<f64> = mine
                    .iter()
                    .filter(|t| t.status == degenview_journal::model::TradeStatus::Closed)
                    .filter_map(|t| reviews.get(&t.key))
                    .filter_map(|review| {
                        degenview_journal::playbook::rule_adherence(review, playbook)
                    })
                    .collect();
                if !shares.is_empty() {
                    adherence.insert(
                        playbook.id.clone(),
                        shares.iter().sum::<f64>() / shares.len() as f64,
                    );
                }
            }
            self.playbook_stats = Some((key, slices, adherence));
        }

        // The draft follows the selection.
        if let Some(id) = self.playbook_selected.clone()
            && host.playbooks().iter().all(|p| p.id != id)
        {
            self.playbook_selected = None;
        }
        match self.playbook_selected.clone() {
            Some(id) => {
                if self
                    .playbook_draft
                    .as_ref()
                    .is_none_or(|draft| draft.playbook.id != id)
                {
                    self.flush_playbook(host);
                    self.playbook_draft = host
                        .playbooks()
                        .iter()
                        .find(|p| p.id == id)
                        .map(journal_playbooks::Draft::new);
                }
            }
            None => {
                self.flush_playbook(host);
                self.playbook_draft = None;
            }
        }

        let playbooks = host.playbooks().clone();
        let mut output = journal_playbooks::Output::default();
        let mut chosen = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| {
                aligned(ui, |ui| {
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            chosen = journal_overview::currency_chips(ui, &currencies, &current);
                            let Some((_, slices, adherence)) = self.playbook_stats.as_ref() else {
                                return;
                            };
                            let rows: Vec<journal_playbooks::Row<'_>> = playbooks
                                .iter()
                                .map(|playbook| journal_playbooks::Row {
                                    playbook,
                                    slice: slices.iter().find(|s| s.label == playbook.name),
                                    adherence: adherence.get(&playbook.id).copied(),
                                })
                                .collect();
                            output = journal_playbooks::show(
                                ui,
                                &rows,
                                self.playbook_selected.as_deref(),
                                self.playbook_draft.as_mut(),
                                current.as_deref().unwrap_or(""),
                            );
                        });
                })
            });
        if let Some(chosen) = chosen {
            self.currency = Some(chosen);
        }
        let now = ui.input(|input| input.time);
        if let Some(draft) = self.playbook_draft.as_mut() {
            if output.changed {
                draft.edited_at = Some(now);
                draft.error = None;
            }
            let due = draft.edited_at.is_some_and(|at| now - at >= SAVE_AFTER);
            if draft.is_dirty() && (due || output.blurred && draft.edited_at.is_some()) {
                match host.save_playbook(&draft.playbook) {
                    Ok(saved) => {
                        draft.saved = saved;
                        draft.edited_at = None;
                        draft.saved_until = Some(now + 2.0);
                        draft.error = None;
                    }
                    Err(error) => {
                        draft.error = Some(error);
                        draft.edited_at = None;
                    }
                }
            } else if draft.is_dirty() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(900));
            }
        }
        if let Some(id) = output.select {
            self.playbook_selected = Some(id);
        }
        if output.create {
            let playbook = Playbook::new(JournalHost::new_playbook_id(), "New playbook");
            if let Ok(saved) = host.save_playbook(&playbook) {
                self.playbook_selected = Some(saved.id);
            }
        }
        if let Some(id) = output.delete {
            self.playbook_delete = Some(id);
        }
        if let Some(id) = self.playbook_delete.clone() {
            match kit::confirm(
                ui.ctx(),
                egui::Id::new(("journal-delete-playbook", self.id)),
                &kit::Confirm {
                    icon: Icon::Trash,
                    title: "Delete this playbook?",
                    message: "Reviews that used it keep their other notes but show no playbook. \
                              This can't be undone.",
                    destructive: "Delete Playbook",
                },
            ) {
                Some(kit::ConfirmOutcome::Confirmed) => {
                    let _ = host.delete_playbook(&id);
                    if self.playbook_selected.as_deref() == Some(id.as_str()) {
                        self.playbook_selected = None;
                        self.playbook_draft = None;
                    }
                    self.playbook_delete = None;
                }
                Some(kit::ConfirmOutcome::Cancelled) => self.playbook_delete = None,
                None => {}
            }
        }
    }

    fn flush_playbook(&mut self, host: &mut JournalHost) {
        if let Some(draft) = self.playbook_draft.as_mut()
            && draft.is_dirty()
            && let Ok(saved) = host.save_playbook(&draft.playbook)
        {
            draft.saved = saved;
            draft.edited_at = None;
        }
    }

    fn flush_daily(&mut self, host: &mut JournalHost) {
        if let Some(draft) = self.daily_draft.as_mut()
            && draft.is_dirty()
            && host.save_day(&draft.entry).is_ok()
        {
            draft.saved = draft.entry.clone();
            draft.edited_at = None;
        }
    }

    fn daily_section(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
    ) {
        let today = journal_calendar::civil_today(now_ms(), &local_utc_offset_secs);
        let date = self.daily_date.unwrap_or(today);
        let date_key = format::iso_date(date);
        let day = days_from_civil(date);

        let scope = self.scope_indices(data);
        let on_day: Vec<&Trade> = scope
            .iter()
            .map(|&i| &data.trades[i])
            .filter(|trade| {
                trade
                    .close_ts_ms
                    .is_some_and(|close| calendar::day_number(close, &local_utc_offset_secs) == day)
            })
            .collect();
        let (currencies, current) = self.choose_currency(&on_day);
        let mut rows: Vec<journal_daily::DayTrade> = on_day
            .iter()
            .map(|trade| journal_daily::DayTrade {
                key: trade.key.clone(),
                symbol: trade.instrument.display_name.clone(),
                direction: trade.direction,
                net: trade.net_pnl(),
                close_ts_ms: trade.close_ts_ms.unwrap_or(trade.open_ts_ms),
            })
            .collect();
        rows.sort_by_key(|row| (row.close_ts_ms, row.key.clone()));
        let mine: Vec<&Trade> = on_day
            .iter()
            .copied()
            .filter(|trade| trade.currency == current)
            .collect();
        let summary = journal_daily::Summary {
            trades: mine.len(),
            wins: mine
                .iter()
                .filter(|t| outcome_of(t, BREAKEVEN_TOLERANCE) == OutcomeFilter::Win)
                .count(),
            losses: mine
                .iter()
                .filter(|t| outcome_of(t, BREAKEVEN_TOLERANCE) == OutcomeFilter::Loss)
                .count(),
            net: mine.iter().map(|t| t.net_pnl()).sum(),
            currency: current.clone().unwrap_or_default(),
        };

        if self
            .daily_draft
            .as_ref()
            .is_none_or(|draft| draft.entry.date != date_key)
        {
            self.flush_daily(host);
            self.daily_draft = Some(journal_daily::Draft::new(
                &date_key,
                host.days().get(&date_key),
            ));
        }
        let mood: Vec<String> = degenview_journal::reviews::suggestions(
            &degenview_journal::reviews::DEFAULT_EMOTIONS,
            host.days()
                .values()
                .flat_map(|d| d.mood.iter())
                .chain(host.reviews().values().flat_map(|r| r.emotions.iter())),
        );

        let mut output = journal_daily::Output::default();
        let mut chosen = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| {
                aligned(ui, |ui| {
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            chosen = journal_overview::currency_chips(ui, &currencies, &current);
                            if let Some(draft) = self.daily_draft.as_mut() {
                                output = journal_daily::show(
                                    ui,
                                    date,
                                    &summary,
                                    &rows,
                                    draft,
                                    &mood,
                                    &local_utc_offset_secs,
                                );
                            }
                        });
                })
            });
        if let Some(chosen) = chosen {
            self.currency = Some(chosen);
        }
        let now = ui.input(|input| input.time);
        if let Some(draft) = self.daily_draft.as_mut() {
            if output.changed {
                draft.edited_at = Some(now);
                draft.error = None;
            }
            let due = draft.edited_at.is_some_and(|at| now - at >= SAVE_AFTER);
            if draft.is_dirty() && (due || output.blurred && draft.edited_at.is_some()) {
                match host.save_day(&draft.entry) {
                    Ok(()) => {
                        draft.saved = draft.entry.clone();
                        draft.edited_at = None;
                        draft.saved_until = Some(now + 2.0);
                        draft.error = None;
                    }
                    Err(error) => {
                        draft.error = Some(error);
                        draft.edited_at = None;
                    }
                }
            } else if draft.is_dirty() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(900));
            }
        }
        if let Some(shift) = output.shift {
            self.daily_date = Some(calendar::civil_from_days(day + shift));
        }
        if output.today {
            self.daily_date = None;
        }
        if let Some(key) = output.open_trade {
            self.flush_daily(host);
            host.prefs_mut().section = Section::Trades;
            self.selected = Some(key);
        }
    }

    // MARK: Overview and calendar

    /// The trades the filters pick, ignoring the range.
    fn scope_indices(&self, data: &JournalData) -> Vec<usize> {
        let mut filter = self.filter.clone();
        filter.from_ms = None;
        filter.to_ms = None;
        filter.matching_indices(&data.trades, BREAKEVEN_TOLERANCE)
    }

    fn overview_section(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
    ) {
        let key = StatsKey {
            build_id: data.build_id,
            reviews_revision: host.reviews_revision(),
            filter: self.filter.clone(),
            currency: self.currency.clone(),
            month: None,
        };
        if data.trades.is_empty() {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::background()))
                .show(ui, |ui| {
                    message_page(
                        ui,
                        Icon::Journal,
                        "Your trades appear here",
                        "Place a paper trade on a chart. Every execution is recorded automatically \
                         and grouped into trades you can review.",
                        None,
                    );
                });
            return;
        }
        self.ensure_overview(host, data, key);
        let mut currency = None;
        let mut open_trade = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| {
                aligned(ui, |ui| {
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.rows.is_empty() {
                                if message_page(
                                    ui,
                                    Icon::Filter,
                                    "No trades match",
                                    "Nothing in the journal fits the range and filters.",
                                    Some("Reset filters"),
                                ) {
                                    self.reset_filters(host);
                                }
                                return;
                            }
                            let Some((_, overview)) = self.overview.as_ref() else {
                                return;
                            };
                            let output = journal_overview::show(
                                ui,
                                overview,
                                &mut self.overview_state,
                                &local_utc_offset_secs,
                                &data.trades,
                            );
                            currency = output.currency;
                            open_trade = output.open_trade;
                        });
                })
            });
        if let Some(chosen) = currency {
            self.currency = Some(chosen);
        }
        if let Some(key) = open_trade {
            host.prefs_mut().section = Section::Trades;
            self.selected = Some(key);
            self.scroll_to = None;
        }
    }

    /// Computes the overview for the current selection unless it is already cached under `key`.
    fn ensure_overview(&mut self, host: &JournalHost, data: &Arc<JournalData>, key: StatsKey) {
        if self
            .overview
            .as_ref()
            .is_none_or(|(cached, _)| cached != &key)
        {
            let scope = self.scope_indices(data);
            let selected: Vec<&Trade> = self.rows.iter().map(|&i| &data.trades[i]).collect();
            let scope_refs: Vec<&Trade> = scope.iter().map(|&i| &data.trades[i]).collect();
            let (from, to) = (self.filter.from_ms, self.filter.to_ms);
            let before: Vec<&Trade> = match from {
                Some(from) => scope_refs
                    .iter()
                    .copied()
                    .filter(|t| t.close_ts_ms.is_some_and(|close| close < from))
                    .collect(),
                None => Vec::new(),
            };
            let previous: Option<Vec<&Trade>> = previous_window(from, to).map(|(start, end)| {
                scope_refs
                    .iter()
                    .copied()
                    .filter(|trade| {
                        let at = trade.close_ts_ms.unwrap_or(trade.open_ts_ms);
                        at >= start && at < end
                    })
                    .collect()
            });
            let accounts =
                accounts_in_scope(&data.accounts, host.prefs().account.as_deref(), &scope_refs);
            let overview = compute(&OverviewInput {
                trades: &selected,
                previous: previous.as_deref(),
                before: &before,
                accounts: &accounts,
                from_ms: from,
                to_ms: to,
                currency: self.currency.clone(),
                reviews: host.reviews(),
                breakeven_tolerance: BREAKEVEN_TOLERANCE,
                offset: &local_utc_offset_secs,
            });
            self.overview = Some((key, overview));
        }
    }

    fn overview_key(&self, host: &JournalHost, data: &JournalData) -> StatsKey {
        StatsKey {
            build_id: data.build_id,
            reviews_revision: host.reviews_revision(),
            filter: self.filter.clone(),
            currency: self.currency.clone(),
            month: None,
        }
    }

    fn analytics_section(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
    ) {
        if data.trades.is_empty() {
            self.empty_central(ui);
            return;
        }
        let key = self.overview_key(host, data);
        self.ensure_overview(host, data, key.clone());
        let Some((_, overview)) = self.overview.as_ref() else {
            return;
        };
        let currency = overview.currency.clone();
        let dimension = self.breakdown_dimension;
        let stale = self
            .breakdown
            .as_ref()
            .is_none_or(|(cached, dim, _)| cached != &key || *dim != dimension);
        if stale {
            let reviews = host.reviews().clone();
            let selected: Vec<&Trade> = self
                .rows
                .iter()
                .map(|&i| &data.trades[i])
                .filter(|trade| trade.currency == currency)
                .collect();
            let slices = breakdown::breakdown(
                &selected,
                dimension,
                &breakdown::Context {
                    reviews: &reviews,
                    playbook_names: host.playbook_names(),
                    offset: &local_utc_offset_secs,
                    breakeven_tolerance: BREAKEVEN_TOLERANCE,
                },
            );
            self.breakdown = Some((key.clone(), dimension, slices));
        }
        if self
            .process
            .as_ref()
            .is_none_or(|(cached, _)| cached != &key)
        {
            let selected: Vec<&Trade> = self
                .rows
                .iter()
                .map(|&i| &data.trades[i])
                .filter(|trade| trade.currency == currency)
                .collect();
            let score = process_score(&selected, host.reviews(), host.playbooks());
            self.process = Some((key.clone(), score));
        }
        let mut picked = None;
        let mut chosen = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| {
                aligned(ui, |ui| {
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.rows.is_empty() {
                                if message_page(
                                    ui,
                                    Icon::Filter,
                                    "No trades match",
                                    "Nothing in the journal fits the range and filters.",
                                    Some("Reset filters"),
                                ) {
                                    self.reset_filters(host);
                                }
                                return;
                            }
                            let (Some((_, overview)), Some((_, _, slices)), Some((_, process))) = (
                                self.overview.as_ref(),
                                self.breakdown.as_ref(),
                                self.process.as_ref(),
                            ) else {
                                return;
                            };
                            chosen = journal_overview::currency_chips(
                                ui,
                                &overview.currencies,
                                &overview.currency,
                            );
                            picked = journal_analytics::show(
                                ui,
                                overview,
                                process,
                                dimension,
                                slices,
                                overview.currency.as_deref().unwrap_or(""),
                            );
                        });
                })
            });
        if let Some(chosen) = chosen {
            self.currency = Some(chosen);
        }
        if let Some(picked) = picked {
            self.breakdown_dimension = picked;
        }
    }

    fn compare_section(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
    ) {
        if data.trades.is_empty() {
            self.empty_central(ui);
            return;
        }
        let key = self.overview_key(host, data);
        let selected: Vec<&Trade> = self.rows.iter().map(|&i| &data.trades[i]).collect();
        let currencies = currencies_of(&selected);
        let current = self
            .currency
            .clone()
            .filter(|chosen| currencies.contains(chosen))
            .or_else(|| currencies.first().cloned())
            .unwrap_or(None);
        if self
            .compare
            .as_ref()
            .is_none_or(|(cached, _)| cached != &key)
        {
            let columns = compare::compare(&compare::Input {
                accounts: &data.accounts,
                trades: &selected,
                currency: current.clone(),
                reviews: host.reviews(),
                breakeven_tolerance: BREAKEVEN_TOLERANCE,
                offset: &local_utc_offset_secs,
            });
            self.compare = Some((key, columns));
        }
        let mut chosen = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| {
                aligned(ui, |ui| {
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            chosen = journal_overview::currency_chips(ui, &currencies, &current);
                            let columns = self.compare.as_ref().map_or(&[][..], |(_, c)| &c[..]);
                            if columns.len() < 2 {
                                message_page(
                                    ui,
                                    Icon::SplitColumns,
                                    "Compare needs two accounts",
                                    "Pick All accounts and a range in which at least two accounts \
                                     closed trades in the same currency.",
                                    None,
                                );
                            } else {
                                journal_compare::show(
                                    ui,
                                    columns,
                                    current.as_deref().unwrap_or(""),
                                    &local_utc_offset_secs,
                                );
                            }
                        });
                })
            });
        if let Some(chosen) = chosen {
            self.currency = Some(chosen);
        }
    }

    fn empty_central(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| {
                message_page(
                    ui,
                    Icon::Journal,
                    "Your trades appear here",
                    "Place a paper trade on a chart. Every execution is recorded automatically \
                     and grouped into trades you can review.",
                    None,
                );
            });
    }

    fn calendar_section(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
    ) {
        let today = journal_calendar::civil_today(now_ms(), &local_utc_offset_secs);
        let (year, month) = self.calendar_month.unwrap_or((today.year, today.month));
        let key = StatsKey {
            build_id: data.build_id,
            reviews_revision: host.reviews_revision(),
            filter: {
                let mut filter = self.filter.clone();
                filter.from_ms = None;
                filter.to_ms = None;
                filter
            },
            currency: self.currency.clone(),
            month: Some((year, month)),
        };
        let scope = self.scope_indices(data);
        let scope_refs: Vec<&Trade> = scope.iter().map(|&i| &data.trades[i]).collect();
        let currencies = currencies_of(&scope_refs);
        let current = self
            .currency
            .clone()
            .filter(|chosen| currencies.contains(chosen))
            .or_else(|| currencies.first().cloned())
            .unwrap_or(None);
        if self.month.as_ref().is_none_or(|(cached, _)| cached != &key) {
            let mine: Vec<&Trade> = scope_refs
                .iter()
                .copied()
                .filter(|trade| trade.currency == current)
                .collect();
            let view =
                month_view(
                    &mine,
                    year,
                    month,
                    &local_utc_offset_secs,
                    &|trade| match outcome_of(trade, BREAKEVEN_TOLERANCE) {
                        OutcomeFilter::Win => calendar::Outcome::Win,
                        OutcomeFilter::Loss => calendar::Outcome::Loss,
                        _ => calendar::Outcome::Breakeven,
                    },
                );
            self.month = Some((key, view));
        }
        let mut chosen = None;
        let mut output = journal_calendar::Output::default();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| {
                aligned(ui, |ui| {
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if data.trades.is_empty() {
                                message_page(
                                    ui,
                                    Icon::Journal,
                                    "Your trades appear here",
                                    "Place a paper trade on a chart. Every execution is recorded \
                                 automatically and grouped into trades you can review.",
                                    None,
                                );
                                return;
                            }
                            chosen = journal_overview::currency_chips(ui, &currencies, &current);
                            if let Some((_, view)) = self.month.as_ref() {
                                output = journal_calendar::show(
                                    ui,
                                    view,
                                    WEEK_START,
                                    current.as_deref().unwrap_or(""),
                                    Some(today),
                                );
                            }
                        });
                })
            });
        if let Some(chosen) = chosen {
            self.currency = Some(chosen);
        }
        if let Some(delta) = output.shift {
            self.calendar_month = Some(shift_month(year, month, delta));
        }
        if output.today {
            self.calendar_month = None;
        }
        if let Some(day) = output.open_day {
            self.daily_date = Some(day);
            host.prefs_mut().section = Section::Daily;
        }
        let _ = Civil {
            year,
            month,
            day: 1,
        };
    }

    // MARK: Header

    fn apply_prefs(&mut self, prefs: &JournalPrefs) {
        if let RangeChoice::Custom { from, to } = &prefs.range {
            self.custom_from = from.clone();
            self.custom_to = to.clone();
        }
    }

    fn range_choice(&self, host: &JournalHost) -> RangeKind {
        match host.prefs().range {
            RangeChoice::Preset(preset) => RangeKind::Preset(preset),
            RangeChoice::Custom { .. } => RangeKind::Custom,
        }
    }

    /// The `[from, to)` of the chosen range, refreshed about twice a minute.
    fn resolved_range(&mut self, host: &JournalHost) -> (Option<i64>, Option<i64>) {
        let now = now_ms();
        let bucket = now / 30_000;
        let custom = matches!(host.prefs().range, RangeChoice::Custom { .. });
        if !custom
            && let Some((cached, from, to)) = self.range_cache
            && cached == bucket
        {
            return (from, to);
        }
        let range = match &host.prefs().range {
            RangeChoice::Preset(preset) => {
                range_for(*preset, now, &local_utc_offset_secs, WEEK_START)
            }
            RangeChoice::Custom { from, to } => {
                let start = format::parse_iso_date(from).map(|civil| {
                    calendar::local_midnight_ms(days_from_civil(civil), &local_utc_offset_secs)
                });
                // The end date is included: the range stops at the start of the day after it.
                let end = format::parse_iso_date(to).map(|civil| {
                    calendar::local_midnight_ms(days_from_civil(civil) + 1, &local_utc_offset_secs)
                });
                (start, end)
            }
        };
        self.range_cache = Some((bucket, range.0, range.1));
        range
    }

    /// How many rows the header has on this page: the page bar, the range, the filters.
    fn header_rows(section: Section) -> (bool, bool) {
        match section {
            Section::Daily => (false, false),
            Section::Calendar => (false, true),
            _ => (true, true),
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, host: &mut JournalHost, data: &Arc<JournalData>) {
        let (show_range, show_filters) = Self::header_rows(host.prefs().section);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let mut section = host.prefs().section;
            let items = [
                (Section::Overview, "Overview", Icon::PieChart),
                (Section::Calendar, "Calendar", Icon::Calendar),
                (Section::Trades, "Trades", Icon::DocText),
                (Section::Analytics, "Analytics", Icon::ChartLine),
                (Section::Compare, "Compare", Icon::SplitColumns),
                (Section::Playbooks, "Playbooks", Icon::Books),
                (Section::Daily, "Daily", Icon::Journal),
            ]
            .map(|(value, title, icon)| kit::TabItem {
                value,
                title,
                icon,
                count: 0,
            });
            kit::tab_bar_slim(
                ui,
                egui::Id::new(("journal-sections", self.id)),
                &items,
                &mut section,
            );
            if section != host.prefs().section {
                host.prefs_mut().section = section;
            }
            if section != Section::Daily {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.account_menu(ui, host, data);
                    self.data_menu(ui, host, data);
                });
            } else {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.data_menu(ui, host, data);
                });
            }
        });
        if show_range {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                self.range_controls(ui, host);
                let section = host.prefs().section;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if section == Section::Trades {
                        self.columns_menu(ui, host);
                    }
                    let open = self
                        .rows
                        .iter()
                        .filter(|&&index| data.trades[index].is_open())
                        .count();
                    let mut text = format!(
                        "{} trade{}",
                        self.rows.len(),
                        if self.rows.len() == 1 { "" } else { "s" }
                    );
                    if open > 0 {
                        text.push_str(&format!(" \u{00b7} {open} open"));
                    }
                    ui.label(kit::text(
                        text,
                        size::BODY,
                        REGULAR,
                        theme::text_secondary(),
                    ))
                    .on_hover_text(format!("Built from {} executions", data.executions));
                });
            });
        }
        if show_filters {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                self.filter_menus(ui, host, data);
                let active = self.filter.is_active()
                    || host.prefs().account.is_some()
                    || host.prefs().range != RangeChoice::Preset(RangePreset::All);
                if active && kit::sheet_button(ui, "Reset", false, true).clicked() {
                    self.reset_filters(host);
                }
            });
        }
    }

    fn range_controls(&mut self, ui: &mut egui::Ui, host: &mut JournalHost) {
        let mut kind = self.range_choice(host);
        let before = kind;
        kit::segmented(
            ui,
            egui::Id::new(("journal-range", self.id)),
            &[
                (RangeKind::Preset(RangePreset::Today), "Today"),
                (RangeKind::Preset(RangePreset::Week), "Week"),
                (RangeKind::Preset(RangePreset::Month), "Month"),
                (RangeKind::Preset(RangePreset::Year), "Year"),
                (RangeKind::Preset(RangePreset::YearToDate), "YTD"),
                (RangeKind::Preset(RangePreset::All), "All"),
                (RangeKind::Custom, "Custom"),
            ],
            &mut kind,
        );
        if kind != before {
            host.prefs_mut().range = match kind {
                RangeKind::Preset(preset) => RangeChoice::Preset(preset),
                RangeKind::Custom => RangeChoice::Custom {
                    from: self.custom_from.clone(),
                    to: self.custom_to.clone(),
                },
            };
            self.range_cache = None;
        }
        if kind == RangeKind::Custom {
            let mut changed = false;
            for (salt, text) in [("from", &mut self.custom_from), ("to", &mut self.custom_to)] {
                let bad = !text.trim().is_empty() && format::parse_iso_date(text).is_none();
                let response = kit::field_frame(false)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .stroke(egui::Stroke::new(
                        1.0,
                        if bad { theme::LOSS } else { kit::primary(0.12) },
                    ))
                    .show(ui, |ui| {
                        kit::plain_field(
                            ui,
                            egui::Id::new(("journal-custom", self.id, salt)),
                            text,
                            if salt == "from" {
                                "From YYYY-MM-DD"
                            } else {
                                "To YYYY-MM-DD"
                            },
                            size::BODY,
                            REGULAR,
                            110.0,
                        )
                    })
                    .inner;
                changed |= response.changed();
            }
            if changed {
                host.prefs_mut().range = RangeChoice::Custom {
                    from: self.custom_from.clone(),
                    to: self.custom_to.clone(),
                };
            }
        }
    }

    // MARK: Import, export and backup

    fn data_menu(&mut self, ui: &mut egui::Ui, host: &mut JournalHost, data: &Arc<JournalData>) {
        use degenview_journal::model::TradingSourceKind;
        let imported: Vec<(String, String)> = data
            .accounts
            .iter()
            .filter(|account| account.kind == TradingSourceKind::Imported)
            .map(|account| (account.uid.clone(), account.name.clone()))
            .collect();
        let removals: Vec<String> = imported
            .iter()
            .map(|(_, name)| format!("Remove imported account \u{201c}{name}\u{201d}\u{2026}"))
            .collect();
        let mut labels: Vec<String> = [
            "Import executions (CSV)\u{2026}",
            "Export trades (CSV)\u{2026}",
            "Export executions (CSV)\u{2026}",
            "Export summary (CSV)\u{2026}",
            "Export notes (Markdown)\u{2026}",
            "Back up journal\u{2026}",
            "Restore from backup\u{2026}",
        ]
        .map(str::to_owned)
        .to_vec();
        labels.extend(removals.iter().cloned());
        let mut chosen: Option<DataAction> = None;
        header_controls::menu_button(
            ui,
            egui::Id::new(("journal-data", self.id)),
            Icon::Download,
            "Data",
            &labels,
            self.io.is_busy(),
            |ui| {
                if header_controls::action(ui, Icon::Download, &labels[0]) {
                    chosen = Some(DataAction::Import);
                }
                header_controls::separator(ui);
                for (index, action) in [
                    DataAction::ExportTrades,
                    DataAction::ExportExecutions,
                    DataAction::ExportSummary,
                    DataAction::ExportNotes,
                ]
                .into_iter()
                .enumerate()
                {
                    if header_controls::action(ui, Icon::ArrowUpRightSquare, &labels[1 + index]) {
                        chosen = Some(action);
                    }
                }
                header_controls::separator(ui);
                if header_controls::action(ui, Icon::Briefcase, &labels[5]) {
                    chosen = Some(DataAction::Backup);
                }
                if header_controls::action(ui, Icon::Download, &labels[6]) {
                    chosen = Some(DataAction::Restore);
                }
                if !imported.is_empty() {
                    header_controls::separator(ui);
                    for ((uid, name), label) in imported.iter().zip(&removals) {
                        if header_controls::action(ui, Icon::Trash, label) {
                            chosen = Some(DataAction::RemoveAccount(uid.clone(), name.clone()));
                        }
                    }
                }
            },
        );
        if let Some(action) = chosen {
            self.run_data_action(ui.ctx(), host, data, action);
        }
    }

    fn run_data_action(
        &mut self,
        ctx: &egui::Context,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
        action: DataAction,
    ) {
        use degenview_journal::{csv as journal_csv, report};
        if self.io.is_busy() {
            return;
        }
        let names = data.account_names();
        let account_name =
            move |uid: &str| names.get(uid).cloned().unwrap_or_else(|| uid.to_owned());
        let playbooks = host.playbook_names().clone();
        let playbook_name =
            move |id: &str| playbooks.get(id).cloned().unwrap_or_else(|| id.to_owned());
        let in_view: Vec<&Trade> = self.rows.iter().map(|&index| &data.trades[index]).collect();
        match action {
            DataAction::Import => self.io.pick_csv(ctx),
            DataAction::ExportTrades => {
                if in_view.is_empty() {
                    self.notice = Some((true, "There are no trades in view to export.".to_owned()));
                    return;
                }
                let text = journal_csv::export_trades(
                    &in_view,
                    host.reviews(),
                    &playbook_name,
                    &account_name,
                );
                self.io
                    .save_text(ctx, "the trades", "journal-trades.csv", text);
            }
            DataAction::ExportExecutions => {
                let store = host.store();
                let account = host.prefs().account.clone();
                self.io
                    .save_built(ctx, "the executions", "journal-executions.csv", move || {
                        let executions = store
                            .lock()
                            .map_err(|_| "The journal's store is unavailable.".to_owned())?
                            .journal_executions()
                            .map_err(|error| {
                                format!("The executions could not be read: {error}")
                            })?;
                        let chosen: Vec<&degenview_journal::model::Execution> = executions
                            .iter()
                            .filter(|execution| {
                                account
                                    .as_deref()
                                    .is_none_or(|uid| uid == execution.account_uid)
                            })
                            .collect();
                        if chosen.is_empty() {
                            return Err("There are no executions to export.".to_owned());
                        }
                        Ok(journal_csv::export_executions(&chosen, &account_name))
                    });
            }
            DataAction::ExportSummary => {
                let reviews = host.reviews().clone();
                let r_of = |trade: &Trade| {
                    degenview_journal::risk::r_multiple(
                        trade,
                        reviews.get(&trade.key).and_then(|review| review.stop),
                    )
                };
                let summaries = degenview_journal::analytics::summarize_by_currency(
                    &in_view,
                    &degenview_journal::analytics::Options {
                        initial_balance: None,
                        breakeven_tolerance: BREAKEVEN_TOLERANCE,
                        offset: &local_utc_offset_secs,
                        r_of: &r_of,
                    },
                );
                if summaries.is_empty() {
                    self.notice =
                        Some((true, "There are no trades in view to summarise.".to_owned()));
                    return;
                }
                self.io.save_text(
                    ctx,
                    "the summary",
                    "journal-summary.csv",
                    report::summary_csv(&summaries),
                );
            }
            DataAction::ExportNotes => {
                let all: Vec<&Trade> = data.trades.iter().collect();
                let days: Vec<&degenview_journal::daily::DailyEntry> =
                    host.days().values().collect();
                let text = report::notes_markdown(
                    &all,
                    host.reviews(),
                    &days,
                    &playbook_name,
                    &account_name,
                    &local_utc_offset_secs,
                    &format::iso_date(journal_calendar::civil_today(
                        now_ms(),
                        &local_utc_offset_secs,
                    )),
                );
                self.io
                    .save_text(ctx, "the notes", "journal-notes.md", text);
            }
            DataAction::Backup => {
                let store = host.store();
                self.io.save_built(
                    ctx,
                    "the backup",
                    "degenview-journal-backup.json",
                    move || {
                        let backup = store
                            .lock()
                            .map_err(|_| "The journal's store is unavailable.".to_owned())?
                            .journal_backup(
                                crate::model::portfolio_csv::format_iso8601(now_ms()),
                                env!("CARGO_PKG_VERSION"),
                            )
                            .map_err(|error| format!("The journal could not be read: {error}"))?;
                        backup.to_json()
                    },
                );
            }
            DataAction::Restore => self.io.pick_backup(ctx),
            DataAction::RemoveAccount(uid, name) => self.remove_account = Some((uid, name)),
        }
    }

    /// Runs the import the sheet describes.
    fn start_import(&mut self, ctx: &egui::Context, host: &mut JournalHost) {
        let Some(sheet) = self.import.take() else {
            return;
        };
        let planned = sheet.plan(&local_utc_offset_secs, now_ms());
        self.import_skipped = planned.rejected.len();
        let sources = degenview_journal::csv::ImportSource::split(planned);
        self.io.import(ctx, host.store(), sources);
    }

    /// Restores the backup that was asked about.
    fn start_restore(&mut self, ctx: &egui::Context, host: &mut JournalHost) {
        if let Some((_, backup)) = self.restore.take() {
            self.io.restore(ctx, host.store(), backup);
        }
    }

    /// Takes in a finished job and draws the sheet and dialogs the import and restore need.
    fn io_ui(&mut self, ctx: &egui::Context, host: &mut JournalHost) {
        if let Some(outcome) = self.io.poll(ctx) {
            match outcome {
                IoOutcome::CsvPicked(Ok(Some((name, table)))) => {
                    self.import = Some(ImportSheet::new(name, table));
                }
                IoOutcome::CsvPicked(Ok(None)) | IoOutcome::BackupPicked(Ok(None)) => {}
                IoOutcome::CsvPicked(Err(error)) | IoOutcome::BackupPicked(Err(error)) => {
                    self.notice = Some((true, error));
                }
                IoOutcome::Saved { what, result } => match result {
                    Ok(Some(path)) => {
                        self.notice = Some((false, format!("Saved {what} to {}.", path.display())));
                    }
                    Ok(None) => {}
                    Err(error) => self.notice = Some((true, error)),
                },
                IoOutcome::Imported(Ok(summary)) => {
                    host.mark_dirty();
                    self.notice = Some((false, summary.message(self.import_skipped)));
                }
                IoOutcome::Imported(Err(error)) | IoOutcome::Restored(Err(error)) => {
                    self.notice = Some((true, error));
                }
                IoOutcome::BackupPicked(Ok(Some(picked))) => self.restore = Some(picked),
                IoOutcome::Restored(Ok(report)) => {
                    host.mark_dirty();
                    self.notice = Some((false, format!("Restored: {}.", report.summary())));
                }
            }
        }

        if let Some(sheet) = self.import.as_mut() {
            match journal_import::show(ctx, sheet, &local_utc_offset_secs) {
                journal_import::Outcome::Open => {}
                journal_import::Outcome::Cancel => self.import = None,
                journal_import::Outcome::Import => self.start_import(ctx, host),
            }
        }

        if let Some((name, backup)) = &self.restore {
            let message = format!(
                "{name} holds {}. Restoring adds what the journal does not have. Nothing is deleted, and anything you edited after the backup was made is kept as it is.",
                backup.summary()
            );
            match kit::alert(
                ctx,
                egui::Id::new(("journal-restore", self.id)),
                &kit::Alert {
                    icon: Icon::Download,
                    tint: theme::text_secondary(),
                    title: "Restore this backup?",
                    message: &message,
                    accept: "Restore",
                    dismiss: "Cancel",
                    cautious: false,
                },
            ) {
                Some(kit::ConfirmOutcome::Confirmed) => self.start_restore(ctx, host),
                Some(kit::ConfirmOutcome::Cancelled) => self.restore = None,
                None => {}
            }
        }

        if let Some((uid, name)) = self.remove_account.clone() {
            let message = format!(
                "This removes \u{201c}{name}\u{201d} from the journal with its executions, trades and reviews. Screenshots stay in the library. It cannot be undone, and importing the file again brings the executions back without the reviews."
            );
            match kit::confirm(
                ctx,
                egui::Id::new(("journal-remove-account", self.id)),
                &kit::Confirm {
                    icon: Icon::Trash,
                    title: "Remove this account?",
                    message: &message,
                    destructive: "Remove",
                },
            ) {
                Some(kit::ConfirmOutcome::Confirmed) => {
                    self.remove_account = None;
                    if let Err(error) = host.remove_account(&uid) {
                        self.notice = Some((true, error));
                    }
                }
                Some(kit::ConfirmOutcome::Cancelled) => self.remove_account = None,
                None => {}
            }
        }

        if let Some((error, text)) = &self.notice {
            let dismissed = kit::notice(
                ctx,
                egui::Id::new(("journal-notice", self.id)),
                &kit::Notice {
                    icon: if *error {
                        Icon::XmarkOctagonFill
                    } else {
                        Icon::CheckCircleFill
                    },
                    tint: if *error { theme::LOSS } else { theme::WIN },
                    title: if *error { "That did not work" } else { "Done" },
                    message: text,
                },
            );
            if dismissed {
                self.notice = None;
            }
        }
    }

    fn account_menu(&mut self, ui: &mut egui::Ui, host: &mut JournalHost, data: &Arc<JournalData>) {
        let selected = host.prefs().account.clone();
        let title = selected
            .as_deref()
            .and_then(|uid| data.account(uid))
            .map_or("All accounts".to_owned(), |account| account.name.clone());
        let labels: Vec<String> = std::iter::once("All accounts".to_owned())
            .chain(data.accounts.iter().map(account_label))
            .collect();
        let mut chosen: Option<Option<String>> = None;
        header_controls::menu_button(
            ui,
            egui::Id::new(("journal-account", self.id)),
            Icon::Briefcase,
            &title,
            &labels,
            false,
            |ui| {
                if header_controls::choice(ui, "All accounts", selected.is_none()) {
                    chosen = Some(None);
                }
                if !data.accounts.is_empty() {
                    header_controls::separator(ui);
                }
                for account in &data.accounts {
                    if header_controls::choice(
                        ui,
                        &account_label(account),
                        selected.as_deref() == Some(account.uid.as_str()),
                    ) {
                        chosen = Some(Some(account.uid.clone()));
                    }
                }
            },
        );
        if let Some(account) = chosen {
            host.prefs_mut().account = account;
        }
    }

    fn filter_menus(&mut self, ui: &mut egui::Ui, host: &mut JournalHost, data: &Arc<JournalData>) {
        let _ = host;
        // Instrument.
        let markets = data.markets();
        let current = self.filter.markets.iter().next().cloned();
        let title = current
            .as_ref()
            .and_then(|key| markets.iter().find(|(market, _)| market == key))
            .map_or("All instruments".to_owned(), |(_, label)| label.clone());
        let labels: Vec<&str> = std::iter::once("All instruments")
            .chain(markets.iter().map(|(_, label)| label.as_str()))
            .collect();
        header_controls::menu_button(
            ui,
            egui::Id::new(("journal-instrument", self.id)),
            Icon::Filter,
            &title,
            &labels,
            false,
            |ui| {
                if header_controls::choice(ui, "All instruments", current.is_none()) {
                    self.filter.markets.clear();
                }
                for (key, label) in &markets {
                    if header_controls::choice(ui, label, current.as_ref() == Some(key)) {
                        self.filter.markets.clear();
                        self.filter.markets.insert(key.clone());
                    }
                }
            },
        );
        // Side.
        let title = match self.filter.direction {
            None => "Both sides",
            Some(Direction::Long) => "Long",
            Some(Direction::Short) => "Short",
        };
        header_controls::menu_button(
            ui,
            egui::Id::new(("journal-side", self.id)),
            Icon::Filter,
            title,
            &["Both sides", "Long", "Short"],
            false,
            |ui| {
                for (label, value) in [
                    ("Both sides", None),
                    ("Long", Some(Direction::Long)),
                    ("Short", Some(Direction::Short)),
                ] {
                    if header_controls::choice(ui, label, self.filter.direction == value) {
                        self.filter.direction = value;
                    }
                }
            },
        );
        // Status.
        let status_options = [
            ("All statuses", None),
            ("Open", Some(OutcomeFilter::Open)),
            ("Wins", Some(OutcomeFilter::Win)),
            ("Losses", Some(OutcomeFilter::Loss)),
            ("Breakeven", Some(OutcomeFilter::Breakeven)),
        ];
        let title = status_options
            .iter()
            .find(|(_, value)| *value == self.filter.outcome)
            .map_or("All statuses", |(label, _)| label);
        let labels: Vec<&str> = status_options.iter().map(|(label, _)| *label).collect();
        header_controls::menu_button(
            ui,
            egui::Id::new(("journal-status", self.id)),
            Icon::Filter,
            title,
            &labels,
            false,
            |ui| {
                for (label, value) in status_options {
                    if header_controls::choice(ui, label, self.filter.outcome == value) {
                        self.filter.outcome = value;
                    }
                }
            },
        );
        // Search.
        let response = kit::search_field(
            ui,
            egui::Id::new(("journal-search", self.id)),
            &mut self.search,
            "Search symbol",
            180.0,
        );
        if response.changed() {
            self.filter.search = self.search.clone();
        }
    }

    fn columns_menu(&mut self, ui: &mut egui::Ui, host: &mut JournalHost) {
        let labels: Vec<&str> = Column::ALL.iter().map(|column| column.label()).collect();
        let mut toggled = None;
        let visible = host.prefs().columns.clone();
        header_controls::menu_button(
            ui,
            egui::Id::new(("journal-columns", self.id)),
            Icon::Sliders,
            "Columns",
            &labels,
            false,
            |ui| {
                for column in Column::ALL {
                    if kit::menu_item_marked(
                        ui,
                        Icon::Check,
                        column.label(),
                        visible.contains(&column),
                    ) {
                        toggled = Some(column);
                    }
                }
            },
        );
        if let Some(column) = toggled {
            let prefs = host.prefs_mut();
            if prefs.columns.contains(&column) {
                // At least one column stays.
                if prefs.columns.len() > 1 {
                    prefs.columns.retain(|c| *c != column);
                }
            } else {
                prefs.columns.push(column);
            }
        }
    }

    fn reset_filters(&mut self, host: &mut JournalHost) {
        self.filter.reset();
        self.search.clear();
        let prefs = host.prefs_mut();
        prefs.account = None;
        prefs.range = RangeChoice::Preset(RangePreset::All);
        self.range_cache = None;
    }

    // MARK: Rows

    fn refresh_rows(
        &mut self,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
        reviews: &Arc<std::collections::HashMap<String, degenview_journal::reviews::Review>>,
    ) {
        let (from, to) = self.resolved_range(host);
        self.filter.from_ms = from;
        self.filter.to_ms = to;
        self.filter.accounts.clear();
        if let Some(uid) = host.prefs().account.clone() {
            if data.account(&uid).is_some() {
                self.filter.accounts.insert(uid);
            } else {
                host.prefs_mut().account = None;
            }
        }
        let key = RowsKey {
            build_id: data.build_id,
            reviews_revision: host.reviews_revision(),
            filter: self.filter.clone(),
            sort: host.prefs().sort,
        };
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        let names = data.account_names();
        let mut rows = self
            .filter
            .matching_indices(&data.trades, BREAKEVEN_TOLERANCE);
        sort_indices(
            &mut rows,
            &data.trades,
            host.prefs().sort,
            &SortContext {
                reviews,
                playbook_names: host.playbook_names(),
                account_names: &names,
                breakeven_tolerance: BREAKEVEN_TOLERANCE,
            },
        );
        self.rows = rows;
        self.rows_key = Some(key);
    }

    /// Does what the user asked of the screenshots: add, paste, drop, view, remove.
    fn handle_shots(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        action: shots_ui::Action,
        list: &[shots_ui::Shot],
    ) {
        let ctx = ui.ctx().clone();
        if action.attach {
            self.shots.start(&ctx, host.store(), Source::Panel);
        }
        if action.paste {
            self.shots.start(&ctx, host.store(), Source::Clipboard);
        }
        // A picture dropped on the window while a trade is open is attached to it.
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_path_buf())
        });
        if let Some(path) = dropped {
            self.shots.start(&ctx, host.store(), Source::Path(path));
        }
        if let Some(hash) = action.open {
            self.shots.viewing = Some(hash);
        }
        if let Some(hash) = action.remove {
            if let Some(draft) = self.draft.as_mut() {
                draft.review.images.retain(|image| *image != hash);
                draft.edited_at = Some(ctx.input(|input| input.time) - SAVE_AFTER);
            }
            if self.shots.viewing.as_deref() == Some(hash.as_str()) {
                self.shots.viewing = None;
            }
        }
        if let Some(hash) = self.shots.viewing.clone() {
            let shot = list
                .iter()
                .find(|shot| shot.hash == hash)
                .cloned()
                .unwrap_or(shots_ui::Shot {
                    hash,
                    thumb: shots_ui::Thumb::Missing,
                });
            if shots_ui::viewer(&ctx, &shot) {
                self.shots.viewing = None;
            }
        }
    }

    fn step(&mut self, delta: isize, data: &Arc<JournalData>) {
        if self.rows.is_empty() {
            return;
        }
        let current = self.selected.as_ref().and_then(|key| {
            self.rows
                .iter()
                .position(|&index| &data.trades[index].key == key)
        });
        let next = match current {
            Some(position) => {
                (position as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize
            }
            None => 0,
        };
        self.selected = Some(data.trades[self.rows[next]].key.clone());
        self.scroll_to = Some(next);
    }

    fn keyboard(&mut self, ui: &mut egui::Ui, data: &Arc<JournalData>) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (down, up, escape) = ui.input(|input| {
            (
                input.key_pressed(Key::ArrowDown) || input.key_pressed(Key::J),
                input.key_pressed(Key::ArrowUp) || input.key_pressed(Key::K),
                input.key_pressed(Key::Escape),
            )
        });
        if down {
            self.step(1, data);
        } else if up {
            self.step(-1, data);
        } else if escape && self.selected.is_some() {
            self.selected = None;
        }
    }

    // MARK: Table

    fn table(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
        reviews: &Arc<std::collections::HashMap<String, degenview_journal::reviews::Review>>,
    ) {
        let names = data.account_names();
        let context = CellContext {
            reviews,
            playbook_names: host.playbook_names(),
            account_names: &names,
            breakeven_tolerance: BREAKEVEN_TOLERANCE,
            offset: &local_utc_offset_secs,
        };
        let columns = host.prefs().visible();
        let prefs = host.prefs().clone();
        let width_of = |column: Column| prefs.width(column);
        let selected_row = self.selected.as_ref().and_then(|key| {
            self.rows
                .iter()
                .position(|&index| &data.trades[index].key == key)
        });
        let rows = self.rows.clone();
        let output = journal_table::show(
            ui,
            &TableSpec {
                salt: "journal-trades",
                columns: &columns,
                width_of: &width_of,
                sort: host.prefs().sort,
                rows: rows.len(),
                selected: selected_row,
                scroll_to: self.scroll_to.take(),
                height: ui.available_height() - 12.0,
            },
            |ui, row, column| {
                let trade = &data.trades[rows[row]];
                let cell = cells::cell(column, trade, &context);
                let color = tone_color(cell.tone);
                if cell.pill {
                    paper_grid::tag(ui, &cell.text, color);
                } else if matches!(column, Column::Net | Column::Gross) {
                    paper_grid::figure_bold(ui, &cell.text, color);
                } else {
                    paper_grid::figure(ui, &cell.text, color);
                }
            },
        );
        if let Some(column) = output.sort_by {
            let prefs = host.prefs_mut();
            prefs.sort = prefs.sort.toggled(column);
        }
        if let Some((column, width)) = output.resizing {
            host.prefs_mut().widths.insert(column, width);
        }
        if let Some(row) = output.clicked {
            let key = data.trades[rows[row]].key.clone();
            self.selected = if self.selected.as_ref() == Some(&key) {
                None
            } else {
                Some(key)
            };
        }
    }

    // MARK: Review drafts

    fn flush_draft(&mut self, host: &mut JournalHost) {
        if let Some(draft) = self.draft.as_mut()
            && draft.is_dirty()
            && host.save_review(&draft.review).is_ok()
        {
            draft.saved = draft.review.clone();
            draft.edited_at = None;
        }
    }

    fn close_trade(&mut self, host: &mut JournalHost) {
        self.flush_draft(host);
        self.draft = None;
        self.selected = None;
    }
}

fn account_label(account: &degenview_journal::model::JournalAccount) -> String {
    let mut label = account.name.clone();
    if account.archived {
        label.push_str(" (archived)");
    }
    label
}

fn tone_color(tone: Tone) -> egui::Color32 {
    match tone {
        Tone::Normal => theme::text_primary(),
        Tone::Positive => theme::WIN,
        Tone::Negative => theme::LOSS,
        Tone::Muted => theme::text_secondary(),
    }
}

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

fn aligned<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    capped(ui, |ui| {
        let inset = ui.max_rect().shrink2(egui::vec2(PAGE_MARGIN, 0.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inset), content)
            .inner
    })
}

fn loading_page(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 30.0).max(40.0));
        ui.add(egui::Spinner::new().size(20.0));
        ui.add_space(10.0);
        ui.label(kit::text(
            "Building the journal\u{2026}",
            size::BODY,
            REGULAR,
            theme::text_secondary(),
        ));
    });
}

/// A centred message with an optional button; true when the button was pressed.
fn message_page(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    message: &str,
    button: Option<&str>,
) -> bool {
    let mut pressed = false;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 90.0).max(30.0));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
        crate::ui::icons::paint(ui.painter(), rect, icon, theme::text_tertiary());
        ui.add_space(14.0);
        ui.label(kit::text(title, 18.0, SEMIBOLD, theme::text_primary()));
        ui.add_space(6.0);
        ui.scope(|ui| {
            ui.set_max_width(440.0);
            ui.label(kit::text(
                message,
                size::BODY,
                REGULAR,
                theme::text_secondary(),
            ));
        });
        if let Some(label) = button {
            ui.add_space(16.0);
            pressed = header_controls::prominent_button(ui, Icon::Check, label).clicked();
        }
    });
    pressed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{
        PaperAccountSettings, PaperCurrency, PaperInstrument, PaperOrderRequest, PaperOrderSide,
        PaperOrderType, PaperQuote, PaperTimeInForce,
    };
    use crate::store::{SharedStore, Store};
    use std::sync::Mutex;

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    /// Two closed trades and one open, through the real paper host.
    fn traded(store: &SharedStore) {
        let mut paper = crate::host::paper_host::PaperHost::load(store.clone());
        let id = paper
            .create_account(
                "Main",
                PaperCurrency::Usd,
                100_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        let instrument = PaperInstrument::chart("BTCUSDT", "BTCUSDT", DataSource::Binance);
        let order = |side, quantity| PaperOrderRequest {
            account_id: id,
            instrument: instrument.clone(),
            side,
            kind: PaperOrderType::Market,
            quantity,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        };
        let quote = |last: f64| PaperQuote {
            instrument_key: instrument.key.clone(),
            bid: None,
            ask: None,
            last: Some(last),
            timestamp: now_ms(),
            is_market_open: true,
        };
        for (buy, sell) in [(100.0, 110.0), (120.0, 115.0)] {
            paper.process_quote(quote(buy)).unwrap();
            paper.submit(order(PaperOrderSide::Buy, 1.0)).unwrap();
            paper.process_quote(quote(sell)).unwrap();
            paper.submit(order(PaperOrderSide::Sell, 1.0)).unwrap();
        }
        paper.process_quote(quote(130.0)).unwrap();
        paper.submit(order(PaperOrderSide::Buy, 1.0)).unwrap();
    }

    fn frame(
        ctx: &egui::Context,
        tab: &mut JournalTab,
        host: &mut JournalHost,
        time: f64,
    ) -> Vec<JournalAction> {
        let mut actions = Vec::new();
        let mut out = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| actions = tab.ui(ui, host, 1),
        );
        out.textures_delta.clear();
        actions
    }

    fn ready() -> (SharedStore, JournalHost, JournalTab, egui::Context) {
        let store = store();
        traded(&store);
        let mut host = JournalHost::new(store.clone(), Arc::new(|| {}));
        host.prefs_mut().section = Section::Trades;
        host.rebuild_blocking();
        // Settle the first background build so a later blocking rebuild is not overwritten by it.
        host.sync(1);
        while host.is_building() {
            host.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        (store, host, JournalTab::new(1), egui::Context::default())
    }

    /// Sixty trades over about six weeks, wins and losses, three symbols, both directions.
    struct Synthetic;

    impl degenview_journal::source::TradingActivitySource for Synthetic {
        fn source_id(&self) -> degenview_journal::model::SourceId {
            degenview_journal::model::SourceId("synthetic".into())
        }
        fn kind(&self) -> degenview_journal::model::TradingSourceKind {
            degenview_journal::model::TradingSourceKind::Imported
        }
        fn capabilities(&self) -> degenview_journal::source::SourceCapabilities {
            Default::default()
        }
        fn accounts(&self) -> Vec<degenview_journal::model::JournalAccount> {
            vec![degenview_journal::model::JournalAccount {
                uid: "synthetic:1".into(),
                source: self.source_id(),
                kind: self.kind(),
                external_id: "1".into(),
                name: "Swing account".into(),
                currency: "USD".into(),
                initial_balance: Some("10000".parse().unwrap()),
                cost_basis: Default::default(),
                archived: false,
                created_at_ms: 1,
            }]
        }
        fn executions(
            &self,
            _: &str,
            _: Option<&degenview_journal::source::Cursor>,
        ) -> Result<degenview_journal::source::ExecutionBatch, degenview_journal::source::SourceError>
        {
            use degenview_journal::model::Side;
            use degenview_journal::testing::exec;
            const DAY: i64 = 86_400_000;
            let now = now_ms();
            let mut state = 12_345u64;
            let mut next = || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 33) as f64 / (1u64 << 31) as f64
            };
            let mut executions = Vec::new();
            for index in 0..60i64 {
                let symbol = ["BTC", "ETH", "SOL"][(index % 3) as usize];
                let long = next() > 0.35;
                let entry = 100.0 + next() * 20.0;
                let move_pct = (next() - 0.42) * 0.09;
                let exit = if long {
                    entry * (1.0 + move_pct)
                } else {
                    entry * (1.0 - move_pct)
                };
                let open = now - (42 * DAY) + index * 42 * DAY / 60 + (next() * 4.0e6) as i64;
                let close = open + 600_000 + (next() * 20_000_000.0) as i64;
                let (first, second) = if long {
                    (Side::Buy, Side::Sell)
                } else {
                    (Side::Sell, Side::Buy)
                };
                let mut buy = exec(
                    &format!("{index}a"),
                    first,
                    "10",
                    &format!("{entry:.2}"),
                    open,
                )
                .of(symbol)
                .in_account("synthetic:1")
                .fee("0.5");
                if index % 2 == 0 {
                    let stop = if long { entry * 0.97 } else { entry * 1.03 };
                    buy = buy.stop(&format!("{stop:.2}"));
                }
                let sell = exec(
                    &format!("{index}b"),
                    second,
                    "10",
                    &format!("{exit:.2}"),
                    close,
                )
                .of(symbol)
                .in_account("synthetic:1")
                .fee("0.5");
                executions.push(buy);
                executions.push(sell);
            }
            Ok(degenview_journal::source::ExecutionBatch {
                executions,
                ..Default::default()
            })
        }
    }

    #[test]
    fn an_empty_journal_shows_the_guidance_not_a_table() {
        let store = store();
        let mut host = JournalHost::new(store, Arc::new(|| {}));
        host.rebuild_blocking();
        let ctx = egui::Context::default();
        let mut tab = JournalTab::new(1);
        for step in 0..3 {
            frame(&ctx, &mut tab, &mut host, step as f64);
        }
        assert!(tab.rows.is_empty());
    }

    #[test]
    fn the_first_frames_before_the_build_finishes_draw_a_loading_page() {
        let store = store();
        let mut host = JournalHost::new(store, Arc::new(|| {}));
        let ctx = egui::Context::default();
        let mut tab = JournalTab::new(1);
        frame(&ctx, &mut tab, &mut host, 0.0);
        // Either the worker has not finished (the loading page was drawn) or it already has: the
        // first frame must draw without it either way.
        assert!(host.data().is_none() || !host.is_building());
    }

    #[test]
    fn trades_list_newest_first_and_filters_narrow_them() {
        let (_, mut host, mut tab, ctx) = ready();
        for step in 0..3 {
            frame(&ctx, &mut tab, &mut host, step as f64);
        }
        let data = host.data().unwrap().clone();
        assert_eq!(tab.rows.len(), 3);
        let first = &data.trades[tab.rows[0]];
        assert!(first.is_open(), "newest first: the open trade");

        tab.filter.outcome = Some(OutcomeFilter::Win);
        frame(&ctx, &mut tab, &mut host, 4.0);
        assert_eq!(tab.rows.len(), 1);

        tab.filter.outcome = None;
        tab.filter.direction = Some(Direction::Short);
        frame(&ctx, &mut tab, &mut host, 5.0);
        assert!(tab.rows.is_empty());
        tab.reset_filters(&mut host);
        frame(&ctx, &mut tab, &mut host, 6.0);
        assert_eq!(tab.rows.len(), 3);
    }

    #[test]
    fn sorting_by_a_header_reorders_and_is_remembered() {
        let (store, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        host.prefs_mut().sort = Sort {
            column: Column::Net,
            descending: false,
        };
        frame(&ctx, &mut tab, &mut host, 1.0);
        let data = host.data().unwrap().clone();
        let nets: Vec<_> = tab.rows.iter().map(|&i| data.trades[i].net_pnl()).collect();
        assert!(nets.windows(2).all(|pair| pair[0] <= pair[1]), "{nets:?}");
        host.flush_prefs();
        let again = JournalHost::new(store, Arc::new(|| {}));
        assert_eq!(again.prefs().sort.column, Column::Net);
    }

    #[test]
    fn selecting_a_trade_opens_the_panel_and_arrows_step_through_the_rows() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let data = host.data().unwrap().clone();
        let key = data.trades[tab.rows[1]].key.clone();
        tab.select_trade(&key);
        frame(&ctx, &mut tab, &mut host, 1.0);
        assert_eq!(tab.draft.as_ref().unwrap().review.trade_key, key);

        tab.step(1, &data);
        frame(&ctx, &mut tab, &mut host, 2.0);
        assert_eq!(tab.selected, Some(data.trades[tab.rows[2]].key.clone()));
        tab.step(1, &data);
        assert_eq!(
            tab.selected,
            Some(data.trades[tab.rows[2]].key.clone()),
            "stops at the end"
        );
        tab.step(-1, &data);
        tab.step(-1, &data);
        tab.step(-1, &data);
        assert_eq!(
            tab.selected,
            Some(data.trades[tab.rows[0]].key.clone()),
            "stops at the start"
        );

        tab.close_trade(&mut host);
        frame(&ctx, &mut tab, &mut host, 3.0);
        assert!(tab.selected.is_none() && tab.draft.is_none());
    }

    #[test]
    fn a_review_typed_in_the_panel_is_saved_after_a_pause_and_survives_a_restart() {
        let (store, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let data = host.data().unwrap().clone();
        let trade = data.trades[tab.rows[1]].clone();
        tab.select_trade(&trade.key);
        frame(&ctx, &mut tab, &mut host, 1.0);

        let draft = tab.draft.as_mut().unwrap();
        draft.review.notes = "Took profit too early.".into();
        draft.review.rating = Some(4);
        draft.tags_text = "early-exit".into();
        draft.edited_at = Some(1.0);
        // Not yet due.
        frame(&ctx, &mut tab, &mut host, 1.2);
        assert!(host.reviews().is_empty());
        // Due.
        frame(&ctx, &mut tab, &mut host, 3.0);
        let saved = &host.reviews()[&trade.key];
        assert_eq!(saved.notes, "Took profit too early.");
        assert_eq!(saved.rating, Some(4));
        assert_eq!(saved.tags, ["early-exit"]);
        assert!(!tab.draft.as_ref().unwrap().is_dirty());

        let mut again = JournalHost::new(store, Arc::new(|| {}));
        again.rebuild_blocking();
        assert_eq!(again.reviews()[&trade.key].notes, "Took profit too early.");
    }

    #[test]
    fn an_attached_screenshot_is_stored_once_saved_with_the_review_and_survives_a_restart() {
        let (store, mut host, mut tab, ctx) = ready();
        crate::service::asset_host::install(
            &ctx,
            crate::service::asset_host::AssetHost::new(store.clone()),
        );
        frame(&ctx, &mut tab, &mut host, 0.0);
        let data = host.data().unwrap().clone();
        let trade = data.trades[tab.rows[0]].clone();
        tab.select_trade(&trade.key);
        frame(&ctx, &mut tab, &mut host, 1.0);

        let path =
            std::env::temp_dir().join(format!("degenview-journal-shot-{}.png", std::process::id()));
        let image =
            image::RgbaImage::from_fn(60, 40, |x, y| image::Rgba([x as u8, y as u8, 90, 255]));
        image.save(&path).unwrap();
        tab.shots
            .start(&ctx, host.store(), Source::Path(path.clone()));
        let mut time = 1.1;
        while tab.draft.as_ref().unwrap().review.images.is_empty() {
            assert!(time < 8.0, "the picture never arrived");
            frame(&ctx, &mut tab, &mut host, time);
            std::thread::sleep(std::time::Duration::from_millis(20));
            time += 0.1;
        }
        std::fs::remove_file(&path).ok();
        // Saved at once, not after the pause.
        frame(&ctx, &mut tab, &mut host, time + 0.1);
        let hash = host.reviews()[&trade.key].images[0].clone();
        assert!(store.lock().unwrap().asset(&hash).unwrap().is_some());
        assert!(!tab.draft.as_ref().unwrap().is_dirty());

        // The viewer opens on it and draws.
        tab.shots.viewing = Some(hash.clone());
        frame(&ctx, &mut tab, &mut host, time + 0.2);
        frame(&ctx, &mut tab, &mut host, time + 0.3);

        let mut again = JournalHost::new(store, Arc::new(|| {}));
        again.rebuild_blocking();
        assert_eq!(again.reviews()[&trade.key].images, [hash]);
    }

    #[test]
    fn a_file_that_is_not_a_picture_is_reported_and_attaches_nothing() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let data = host.data().unwrap().clone();
        tab.select_trade(&data.trades[tab.rows[0]].key.clone());
        frame(&ctx, &mut tab, &mut host, 1.0);
        let path = std::env::temp_dir().join(format!(
            "degenview-journal-notshot-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, b"not a picture").unwrap();
        tab.shots
            .start(&ctx, host.store(), Source::Path(path.clone()));
        let mut time = 1.1;
        while tab.shots.error.is_none() {
            assert!(time < 8.0, "no error came back");
            frame(&ctx, &mut tab, &mut host, time);
            std::thread::sleep(std::time::Duration::from_millis(20));
            time += 0.1;
        }
        std::fs::remove_file(&path).ok();
        assert!(tab.draft.as_ref().unwrap().review.images.is_empty());
    }

    #[test]
    fn the_excursion_block_reports_a_failed_fetch_and_a_retry_that_gets_an_answer() {
        struct OneBar;
        impl CandleFetcher for OneBar {
            fn candles(
                &self,
                _: crate::model::DataSource,
                _: &str,
                _: &str,
                start: i64,
                _: i64,
                _: usize,
            ) -> Result<Vec<crate::model::Kline>, String> {
                Ok(vec![crate::model::Kline {
                    open_time: start,
                    close_time: start + 59_999,
                    open: 1.0,
                    high: 2.0,
                    low: 1.0,
                    close: 2.0,
                    volume: 1.0,
                }])
            }
        }
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let data = host.data().unwrap().clone();
        let closed = data
            .trades
            .iter()
            .find(|trade| !trade.is_open())
            .unwrap()
            .clone();
        tab.select_trade(&closed.key);
        let mut time = 1.0;
        let mut wait =
            |tab: &mut JournalTab, host: &mut JournalHost, done: fn(&ExcursionState) -> bool| {
                for _ in 0..200 {
                    frame(&ctx, tab, host, time);
                    time += 0.1;
                    if tab.excursions.state(&closed).is_some_and(done) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                panic!("no answer");
            };
        // The test fetcher is offline.
        wait(&mut tab, &mut host, |state| {
            matches!(state, ExcursionState::Done(Err(_)))
        });
        tab.fetcher = Arc::new(OneBar);
        tab.excursions
            .request(&closed, &host.store(), tab.fetcher.clone(), || {}, true);
        wait(&mut tab, &mut host, |state| {
            matches!(state, ExcursionState::Done(Ok(_)))
        });
        // Fills a few milliseconds apart leave no complete candle between them.
        assert!(matches!(
            tab.excursions.state(&closed),
            Some(ExcursionState::Done(Ok(
                degenview_journal::metric::Metric::Unavailable(
                    degenview_journal::metric::Reason::NoCompleteBar
                )
            )))
        ));
        frame(&ctx, &mut tab, &mut host, time);
    }

    #[test]
    fn showing_a_trade_from_a_chart_clears_what_could_hide_it_and_opens_its_panel() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let data = host.data().unwrap().clone();
        let key = data.trades[tab.rows[1]].key.clone();
        // A range and a search that exclude everything.
        host.prefs_mut().range = RangeChoice::Custom {
            from: "2001-01-01".into(),
            to: "2001-12-31".into(),
        };
        tab.search = "nothing-matches".into();
        tab.range_cache = None;
        frame(&ctx, &mut tab, &mut host, 1.0);
        assert!(tab.rows.is_empty());
        tab.show_trade(&key, &mut host);
        frame(&ctx, &mut tab, &mut host, 2.0);
        assert_eq!(tab.selected.as_deref(), Some(key.as_str()));
        assert_eq!(tab.draft.as_ref().unwrap().review.trade_key, key);
        assert_eq!(host.prefs().section, Section::Trades);
    }

    #[test]
    fn the_host_offers_marks_for_the_selected_account_and_caches_them() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let all = host.marks();
        assert_eq!(all.len(), 3);
        assert!(Arc::ptr_eq(&all, &host.marks()), "cached");
        host.prefs_mut().account = Some("paper:none".into());
        assert!(host.marks().is_empty());
    }

    fn wait_for_io(tab: &mut JournalTab, host: &mut JournalHost, ctx: &egui::Context) {
        let mut time = 50.0;
        while tab.io.is_busy() {
            assert!(time < 60.0, "the job never finished");
            frame(ctx, tab, host, time);
            std::thread::sleep(std::time::Duration::from_millis(10));
            time += 0.05;
        }
        frame(ctx, tab, host, time);
    }

    #[test]
    fn an_import_adds_an_account_and_its_trades_and_a_repeat_adds_nothing() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let before = host.data().unwrap().trades.len();
        let text = "time,symbol,side,qty,price,fee\n\
            2026-11-09T10:00:00Z,SOL,buy,10,100,1\n\
            2026-11-09T11:00:00Z,SOL,sell,10,110,1\n\
            not-a-time,SOL,buy,1,100,\n";
        for round in 0..2 {
            let table = degenview_journal::csv::read_table(text).unwrap();
            tab.import = Some(ImportSheet::new("broker-fills.csv".into(), table));
            tab.start_import(&ctx, &mut host);
            assert!(tab.import.is_none());
            wait_for_io(&mut tab, &mut host, &ctx);
            let (error, message) = tab.notice.clone().expect("a result");
            assert!(!error, "{message}");
            if round == 0 {
                assert!(
                    message.starts_with("2 new executions added to 1 account"),
                    "{message}"
                );
                assert!(message.contains("1 rows were refused"), "{message}");
            } else {
                assert!(message.starts_with("0 new executions"), "{message}");
                assert!(
                    message.contains("2 were already in the journal"),
                    "{message}"
                );
            }
            tab.notice = None;
            // The journal rebuilds on its worker.
            let mut waited = 0;
            while host.is_building() && waited < 500 {
                host.poll();
                std::thread::sleep(std::time::Duration::from_millis(5));
                waited += 1;
            }
            host.sync(1);
            while host.is_building() {
                host.poll();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            frame(&ctx, &mut tab, &mut host, 100.0 + f64::from(round));
        }
        let data = host.data().unwrap().clone();
        assert_eq!(data.trades.len(), before + 1);
        let imported = data
            .accounts
            .iter()
            .find(|account| account.name == "broker-fills")
            .expect("the imported account");
        assert_eq!(
            imported.kind,
            degenview_journal::model::TradingSourceKind::Imported
        );

        // Removing it takes the trade with it.
        let uid = imported.uid.clone();
        host.remove_account(&uid).unwrap();
        host.sync(2);
        while host.is_building() {
            host.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        host.poll();
        assert_eq!(host.data().unwrap().trades.len(), before);
    }

    #[test]
    fn a_backup_made_here_restores_into_another_journal_through_the_tab() {
        let (store_a, mut host_a, mut tab_a, ctx) = ready();
        frame(&ctx, &mut tab_a, &mut host_a, 0.0);
        let key = host_a.data().unwrap().trades[0].key.clone();
        let mut review = degenview_journal::reviews::Review::new(
            &key,
            &host_a.data().unwrap().trades[0].account_uid,
        );
        review.notes = "Backed up.".into();
        host_a.save_review(&review).unwrap();
        let backup = store_a
            .lock()
            .unwrap()
            .journal_backup("2026-11-10T00:00:00.000Z".into(), "test")
            .unwrap();

        let store_b = store();
        let mut host_b = JournalHost::new(store_b.clone(), Arc::new(|| {}));
        let mut tab_b = JournalTab::new(2);
        tab_b.restore = Some(("backup.json".into(), backup));
        tab_b.start_restore(&ctx, &mut host_b);
        wait_for_io(&mut tab_b, &mut host_b, &ctx);
        let (error, message) = tab_b.notice.clone().expect("a result");
        assert!(!error, "{message}");
        assert!(message.starts_with("Restored:"), "{message}");
        host_b.rebuild_blocking();
        assert_eq!(
            host_b.data().unwrap().trades.len(),
            host_a.data().unwrap().trades.len()
        );
        assert_eq!(host_b.reviews()[&key].notes, "Backed up.");
    }

    #[test]
    fn the_data_menu_and_sheet_draw_in_every_state() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let table = degenview_journal::csv::read_table(
            "time,symbol,side,qty,price\n2026-11-09T10:00:00Z,SOL,buy,1,100\n",
        )
        .unwrap();
        tab.import = Some(ImportSheet::new("f.csv".into(), table));
        frame(&ctx, &mut tab, &mut host, 1.0);
        frame(&ctx, &mut tab, &mut host, 1.1);
        tab.import = None;
        tab.notice = Some((true, "That did not work.".into()));
        frame(&ctx, &mut tab, &mut host, 2.0);
        tab.notice = None;
        tab.remove_account = Some(("import:x".into(), "X".into()));
        frame(&ctx, &mut tab, &mut host, 3.0);
        tab.remove_account = None;
        tab.restore = Some((
            "b.json".into(),
            Backup::empty("2026-11-10T00:00:00.000Z".into(), "test"),
        ));
        frame(&ctx, &mut tab, &mut host, 4.0);
    }

    #[test]
    fn switching_trades_saves_what_was_typed_instead_of_losing_it() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let data = host.data().unwrap().clone();
        let (a, b) = (
            data.trades[tab.rows[0]].key.clone(),
            data.trades[tab.rows[1]].key.clone(),
        );
        tab.select_trade(&a);
        frame(&ctx, &mut tab, &mut host, 1.0);
        tab.draft.as_mut().unwrap().review.notes = "unsaved".into();
        tab.select_trade(&b);
        frame(&ctx, &mut tab, &mut host, 1.1);
        assert_eq!(host.reviews()[&a].notes, "unsaved");
        assert_eq!(tab.draft.as_ref().unwrap().review.trade_key, b);
    }

    #[test]
    fn a_selection_that_vanishes_closes_the_panel() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        tab.select_trade("no-such-trade");
        frame(&ctx, &mut tab, &mut host, 1.0);
        assert!(tab.selected.is_none());
    }

    #[test]
    fn the_range_choice_and_account_filter_apply_to_the_rows() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        // Everything was traded just now: a range ending long ago has nothing in it.
        host.prefs_mut().range = RangeChoice::Custom {
            from: "2001-01-01".into(),
            to: "2001-12-31".into(),
        };
        frame(&ctx, &mut tab, &mut host, 1.0);
        assert!(tab.rows.is_empty());
        host.prefs_mut().range = RangeChoice::Preset(RangePreset::All);
        tab.range_cache = None;
        frame(&ctx, &mut tab, &mut host, 2.0);
        assert_eq!(tab.rows.len(), 3);
        // An account that no longer exists is dropped, not shown as an empty journal.
        host.prefs_mut().account = Some("paper:gone".into());
        frame(&ctx, &mut tab, &mut host, 3.0);
        assert_eq!(host.prefs().account, None);
        assert_eq!(tab.rows.len(), 3);
        let uid = host.data().unwrap().accounts[0].uid.clone();
        host.prefs_mut().account = Some(uid);
        frame(&ctx, &mut tab, &mut host, 4.0);
        assert_eq!(tab.rows.len(), 3);
    }

    #[test]
    fn the_overview_describes_the_selection_and_follows_the_filters() {
        let (_, mut host, mut tab, ctx) = ready();
        host.prefs_mut().section = Section::Overview;
        for step in 0..3 {
            frame(&ctx, &mut tab, &mut host, step as f64);
        }
        let (_, overview) = tab.overview.as_ref().expect("overview computed");
        assert_eq!(overview.summary.closed, 2);
        assert_eq!(overview.summary.open, 1);
        assert_eq!(overview.summary.net_pnl, "5".parse().unwrap());
        assert_eq!(overview.initial_balance, Some("100000".parse().unwrap()));

        tab.filter.outcome = Some(OutcomeFilter::Loss);
        frame(&ctx, &mut tab, &mut host, 4.0);
        assert_eq!(tab.overview.as_ref().unwrap().1.summary.closed, 1);
        // Nothing matching draws the guidance, not an empty dashboard.
        tab.filter.direction = Some(Direction::Short);
        frame(&ctx, &mut tab, &mut host, 5.0);
    }

    #[test]
    fn the_analytics_page_breaks_results_down_and_follows_the_chosen_dimension() {
        let (_, mut host, mut tab, ctx) = ready();
        host.prefs_mut().section = Section::Analytics;
        for step in 0..3 {
            frame(&ctx, &mut tab, &mut host, step as f64);
        }
        let (_, dim, slices) = tab.breakdown.as_ref().expect("breakdown computed");
        assert_eq!(*dim, Dimension::Weekday);
        assert_eq!(slices.iter().map(|s| s.trades).sum::<usize>(), 2);
        tab.breakdown_dimension = Dimension::Side;
        frame(&ctx, &mut tab, &mut host, 4.0);
        let (_, dim, slices) = tab.breakdown.as_ref().unwrap();
        assert_eq!(*dim, Dimension::Side);
        assert_eq!(slices[0].label, "Long");
        // A review's strategy becomes a slice after it is saved.
        let trade = host.data().unwrap().trades[0].clone();
        let mut review = degenview_journal::reviews::Review::new(&trade.key, &trade.account_uid);
        review.playbook = Some("pb1".into());
        host.save_review(&review).unwrap();
        tab.breakdown_dimension = Dimension::Strategy;
        frame(&ctx, &mut tab, &mut host, 5.0);
        let labels: Vec<_> = tab
            .breakdown
            .as_ref()
            .unwrap()
            .2
            .iter()
            .map(|s| s.label.clone())
            .collect();
        assert!(labels.contains(&"(none)".to_owned()), "{labels:?}");
    }

    #[test]
    fn compare_needs_two_accounts_and_lines_them_up_when_there_are() {
        let (store, mut host, mut tab, ctx) = ready();
        host.prefs_mut().section = Section::Compare;
        frame(&ctx, &mut tab, &mut host, 0.0);
        assert_eq!(
            tab.compare.as_ref().unwrap().1.len(),
            1,
            "one account is not a comparison"
        );
        store
            .lock()
            .unwrap()
            .journal_ingest(&Synthetic, true)
            .unwrap();
        host.rebuild_blocking();
        frame(&ctx, &mut tab, &mut host, 1.0);
        let columns = &tab.compare.as_ref().unwrap().1;
        assert_eq!(columns.len(), 2);
        assert!(columns.iter().any(|c| c.name == "Swing account"));
    }

    #[test]
    fn a_playbook_is_created_edited_saved_and_deleted() {
        let (store, mut host, mut tab, ctx) = ready();
        host.prefs_mut().section = Section::Playbooks;
        frame(&ctx, &mut tab, &mut host, 0.0);
        assert!(host.playbooks().is_empty());

        // Create (what the New playbook button does), then it is selected and has a draft.
        let created = host
            .save_playbook(&Playbook::new(JournalHost::new_playbook_id(), "Breakout"))
            .unwrap();
        tab.playbook_selected = Some(created.id.clone());
        frame(&ctx, &mut tab, &mut host, 1.0);
        let draft = tab.playbook_draft.as_mut().expect("draft");
        assert_eq!(draft.playbook.name, "Breakout");
        draft.playbook.description = "Trade the break.".into();
        draft.checklist_text = "Volume\nStop placed".into();
        draft.sync_fields();
        draft.edited_at = Some(1.0);
        frame(&ctx, &mut tab, &mut host, 3.0);
        let saved = &host.playbooks()[0];
        assert_eq!(saved.description, "Trade the break.");
        assert_eq!(saved.checklist, ["Volume", "Stop placed"]);

        // It survives a restart.
        let mut again = JournalHost::new(store, Arc::new(|| {}));
        again.rebuild_blocking();
        assert_eq!(again.playbooks()[0].name, "Breakout");

        // An emptied name is refused and the old one stays stored.
        let draft = tab.playbook_draft.as_mut().unwrap();
        draft.playbook.name.clear();
        draft.edited_at = Some(3.0);
        frame(&ctx, &mut tab, &mut host, 5.0);
        assert!(tab.playbook_draft.as_ref().unwrap().error.is_some());
        assert_eq!(host.playbooks()[0].name, "Breakout");

        // Leaving the page flushes a good draft instead of losing it.
        let draft = tab.playbook_draft.as_mut().unwrap();
        draft.playbook.name = "Breakout v2".into();
        draft.error = None;
        host.prefs_mut().section = Section::Trades;
        frame(&ctx, &mut tab, &mut host, 6.0);
        assert_eq!(host.playbooks()[0].name, "Breakout v2");
        assert!(tab.playbook_draft.is_none());

        host.delete_playbook(&created.id).unwrap();
        assert!(host.playbooks().is_empty());
    }

    #[test]
    fn a_trade_follows_a_playbook_and_adherence_reaches_the_pages() {
        let (_, mut host, mut tab, ctx) = ready();
        let mut playbook = Playbook::new(JournalHost::new_playbook_id(), "Breakout");
        playbook.checklist = vec!["Volume".into(), "Stop".into()];
        let playbook = host.save_playbook(&playbook).unwrap();
        let trade = host
            .data()
            .unwrap()
            .trades
            .iter()
            .find(|t| !t.is_open())
            .unwrap()
            .clone();
        let mut review = degenview_journal::reviews::Review::new(&trade.key, &trade.account_uid);
        review.playbook = Some(playbook.id.clone());
        review.checks = ["Volume".to_owned()].into();
        host.save_review(&review).unwrap();
        host.prefs_mut().section = Section::Playbooks;
        frame(&ctx, &mut tab, &mut host, 0.0);
        let (_, slices, adherence) = tab.playbook_stats.as_ref().unwrap();
        assert_eq!(
            slices
                .iter()
                .find(|s| s.label == "Breakout")
                .unwrap()
                .trades,
            1
        );
        assert_eq!(adherence[&playbook.id], 0.5);
        // The Strategy column and breakdown use its name.
        host.prefs_mut().section = Section::Trades;
        frame(&ctx, &mut tab, &mut host, 1.0);
        host.prefs_mut().section = Section::Analytics;
        tab.breakdown_dimension = Dimension::Strategy;
        frame(&ctx, &mut tab, &mut host, 2.0);
        let labels: Vec<_> = tab
            .breakdown
            .as_ref()
            .unwrap()
            .2
            .iter()
            .map(|s| s.label.clone())
            .collect();
        assert!(labels.contains(&"Breakout".to_owned()), "{labels:?}");
        let (_, process) = tab.process.as_ref().unwrap();
        assert_eq!(process.reviewed, 1);
    }

    #[test]
    fn the_daily_journal_saves_per_day_and_follows_the_date() {
        let (store, mut host, mut tab, ctx) = ready();
        host.prefs_mut().section = Section::Daily;
        frame(&ctx, &mut tab, &mut host, 0.0);
        let today = journal_calendar::civil_today(now_ms(), &local_utc_offset_secs);
        let key = format::iso_date(today);
        let draft = tab.daily_draft.as_mut().expect("draft");
        assert_eq!(draft.entry.date, key);
        draft.entry.plan = "Trade the open.".into();
        draft.mood_text = "calm".into();
        draft.sync_fields();
        draft.edited_at = Some(0.0);
        frame(&ctx, &mut tab, &mut host, 2.0);
        assert_eq!(host.days()[&key].plan, "Trade the open.");
        assert_eq!(host.days()[&key].mood, ["calm"]);

        // Another day has its own empty entry; typing and moving on saves the first.
        tab.daily_draft.as_mut().unwrap().entry.lessons = "Wait.".into();
        tab.daily_date = Some(calendar::civil_from_days(days_from_civil(today) - 1));
        frame(&ctx, &mut tab, &mut host, 3.0);
        assert_eq!(host.days()[&key].lessons, "Wait.");
        assert!(tab.daily_draft.as_ref().unwrap().entry.plan.is_empty());

        let mut again = JournalHost::new(store, Arc::new(|| {}));
        again.rebuild_blocking();
        assert_eq!(again.days()[&key].lessons, "Wait.");

        // A day with no entry and no trades is allowed to stay empty and stores nothing.
        tab.daily_date = Some(calendar::civil_from_days(days_from_civil(today) - 400));
        frame(&ctx, &mut tab, &mut host, 4.0);
        assert_eq!(host.days().len(), 1);
    }

    #[test]
    fn the_days_trades_are_listed_on_the_daily_page_and_open_in_trades() {
        let (_, mut host, mut tab, ctx) = ready();
        host.prefs_mut().section = Section::Daily;
        frame(&ctx, &mut tab, &mut host, 0.0);
        // The two closed trades were made today: they are on the page (it drew without panic) and
        // a click would select one.
        let key = host.data().unwrap().trades[0].key.clone();
        tab.selected = Some(key.clone());
        host.prefs_mut().section = Section::Trades;
        frame(&ctx, &mut tab, &mut host, 1.0);
        assert_eq!(tab.draft.as_ref().unwrap().review.trade_key, key);
    }

    #[test]
    fn the_calendar_shows_the_current_month_and_can_move() {
        let (_, mut host, mut tab, ctx) = ready();
        host.prefs_mut().section = Section::Calendar;
        frame(&ctx, &mut tab, &mut host, 0.0);
        let today = journal_calendar::civil_today(now_ms(), &local_utc_offset_secs);
        let (_, view) = tab.month.as_ref().expect("month computed");
        assert_eq!((view.year, view.month), (today.year, today.month));
        assert_eq!(view.trades, 2, "the two closed trades were made today");
        // The range filter does not hide a month's days.
        host.prefs_mut().range = RangeChoice::Custom {
            from: "2001-01-01".into(),
            to: "2001-01-02".into(),
        };
        frame(&ctx, &mut tab, &mut host, 1.0);
        assert_eq!(tab.month.as_ref().unwrap().1.trades, 2);
        tab.calendar_month = Some(shift_month(today.year, today.month, -1));
        frame(&ctx, &mut tab, &mut host, 2.0);
        assert_eq!(tab.month.as_ref().unwrap().1.trades, 0);
    }

    #[test]
    fn leaving_the_trades_page_closes_the_panel() {
        let (_, mut host, mut tab, ctx) = ready();
        frame(&ctx, &mut tab, &mut host, 0.0);
        let key = host.data().unwrap().trades[tab.rows[0]].key.clone();
        tab.select_trade(&key);
        frame(&ctx, &mut tab, &mut host, 1.0);
        assert!(tab.selected.is_some());
        host.prefs_mut().section = Section::Overview;
        frame(&ctx, &mut tab, &mut host, 2.0);
        assert!(tab.selected.is_none() && tab.draft.is_none());
    }

    #[test]
    fn the_chosen_page_is_remembered() {
        let (store, mut host, mut tab, ctx) = ready();
        host.prefs_mut().section = Section::Calendar;
        frame(&ctx, &mut tab, &mut host, 0.0);
        host.flush_prefs();
        let again = JournalHost::new(store, Arc::new(|| {}));
        assert_eq!(again.prefs().section, Section::Calendar);
    }

    #[test]
    fn an_invalid_custom_date_leaves_that_end_open() {
        let (_, mut host, mut tab, ctx) = ready();
        host.prefs_mut().range = RangeChoice::Custom {
            from: "garbage".into(),
            to: "".into(),
        };
        frame(&ctx, &mut tab, &mut host, 0.0);
        assert_eq!(tab.filter.from_ms, None);
        assert_eq!(tab.filter.to_ms, None);
        assert_eq!(tab.rows.len(), 3);
    }

    /// Playbooks, reviews and a daily entry for the pictures.
    fn seed_gallery(host: &mut JournalHost) {
        let mut breakout = Playbook::new(JournalHost::new_playbook_id(), "Breakout");
        breakout.description = "Trade the break of a well-tested level.".into();
        breakout.checklist = vec![
            "Volume spike".into(),
            "Above VWAP".into(),
            "Stop placed".into(),
        ];
        let breakout = host.save_playbook(&breakout).unwrap();
        let mut reversion = Playbook::new(JournalHost::new_playbook_id(), "Mean reversion");
        reversion.checklist = vec!["Extended from VWAP".into(), "Stop placed".into()];
        let reversion = host.save_playbook(&reversion).unwrap();
        let trades = host.data().unwrap().trades.clone();
        for (index, trade) in trades.iter().filter(|t| !t.is_open()).enumerate().take(44) {
            let mut review =
                degenview_journal::reviews::Review::new(&trade.key, &trade.account_uid);
            let playbook = if index % 2 == 0 {
                &breakout
            } else {
                &reversion
            };
            review.playbook = Some(playbook.id.clone());
            review.checks = playbook
                .checklist
                .iter()
                .take(1 + index % playbook.checklist.len())
                .cloned()
                .collect();
            review.discipline = Some(2 + (index % 4) as u8);
            if index % 5 == 0 {
                review.mistakes = vec!["Moved stop".into()];
            }
            review.notes = "Entered on the retest.".into();
            host.save_review(&review).unwrap();
        }
        let today = journal_calendar::civil_today(now_ms(), &local_utc_offset_secs);
        let mut day = DailyEntry::new(&format::iso_date(today));
        day.plan = "Trade the open only. No trades after the first hour.".into();
        day.mood = vec!["Calm".into()];
        day.lessons = "Waiting for the retest worked.".into();
        host.save_day(&day).unwrap();
    }

    /// The tab as pictures: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib journal_tab::tests::gallery -- --ignored`.
    #[test]
    #[ignore = "writes pictures"]
    fn gallery_of_the_tab() {
        use crate::ui::render_tests::Screen;
        let _restore = crate::ui::theme::RestoreDefaultTheme;
        for light in [false, true] {
            let (store, mut host, mut tab, _) = ready();
            store
                .lock()
                .unwrap()
                .journal_ingest(&Synthetic, true)
                .unwrap();
            host.rebuild_blocking();
            seed_gallery(&mut host);
            let mut screen = Screen::new(egui::vec2(1400.0, 860.0));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let suffix = if light { "light" } else { "dark" };
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(vec![], |ui| {
                    crate::ui::theme::sync(ui.ctx());
                    tab.ui(ui, &mut host, 1);
                }));
            }
            screen.save(
                &format!("journal-table-{suffix}"),
                last.take().expect("ran"),
            );

            let data = host.data().unwrap().clone();
            let key = data.trades[tab.rows[1]].key.clone();
            crate::service::asset_host::install(
                &screen.ctx,
                crate::service::asset_host::AssetHost::new(store.clone()),
            );
            let account = data.trades[tab.rows[1]].account_uid.clone();
            let mut review = host
                .reviews()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| degenview_journal::reviews::Review::new(&key, &account));
            for seed in [30u8, 160] {
                let image = image::RgbaImage::from_fn(320, 200, |x, y| {
                    image::Rgba([seed, (x / 2) as u8, (y + u32::from(seed)) as u8, 255])
                });
                let mut png = Vec::new();
                image::ImageEncoder::write_image(
                    image::codecs::png::PngEncoder::new(&mut png),
                    image.as_raw(),
                    320,
                    200,
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
                let picture = crate::service::image_ingest::ingest(&png).unwrap();
                store
                    .lock()
                    .unwrap()
                    .put_asset(&crate::store::asset_store::StoredAsset::from_ingested(
                        &picture,
                    ))
                    .unwrap();
                review.images.push(picture.id);
            }
            host.save_review(&review).unwrap();
            let trade = data.trades[tab.rows[1]].clone();
            tab.excursions.put(
                &trade,
                Ok(degenview_journal::metric::Metric::Value(
                    degenview_journal::excursion::Excursion {
                        mfe: trade.net_pnl().abs() * rust_decimal::Decimal::TWO,
                        mae: trade.net_pnl().abs() / rust_decimal::Decimal::TWO,
                        gross: trade.gross_pnl,
                        exit_efficiency: Some(0.5),
                        bars: 42,
                        interval: "5m".into(),
                    },
                )),
            );
            tab.select_trade(&key);
            for _ in 0..4 {
                last = Some(screen.run(vec![], |ui| {
                    crate::ui::theme::sync(ui.ctx());
                    tab.ui(ui, &mut host, 1);
                }));
            }
            screen.save(
                &format!("journal-detail-{suffix}"),
                last.take().expect("ran"),
            );

            if !light {
                let mut tall = Screen::new(egui::vec2(1400.0, 2500.0));
                // Textures belong to the context that loaded them.
                tab.shots = Shots::default();
                crate::service::asset_host::install(
                    &tall.ctx,
                    crate::service::asset_host::AssetHost::new(store.clone()),
                );
                for _ in 0..8 {
                    last = Some(tall.run(vec![], |ui| {
                        crate::ui::theme::sync(ui.ctx());
                        tab.ui(ui, &mut host, 1);
                    }));
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
                tall.save("journal-detail-tall-dark", last.take().expect("ran"));
            }

            {
                let table = degenview_journal::csv::read_table(
                    "Date,Ticker,Action,Shares,Fill Price,Commission,Exchange\n\
                     2026-11-09 09:31:02,AAPL,BOT,100,212.40,1.00,alpaca\n\
                     2026-11-09 10:14:45,AAPL,SLD,100,214.05,1.00,alpaca\n\
                     2026-11-09 10:40:10,NVDA,BOT,20,141.2,1.00,alpaca\n\
                     2026-11-09 15:55:00,NVDA,SLD,20,139.75,,alpaca\n\
                     yesterday,TSLA,BOT,5,330,1.00,alpaca\n\
                     2026-11-10 09:35:00,TSLA,HOLD,5,330,1.00,alpaca\n\
                     2026-11-10 09:36:00,TSLA,BOT,0,330,1.00,alpaca\n",
                )
                .unwrap();
                tab.import = Some(ImportSheet::new("ibkr-fills.csv".into(), table));
                let mut sheet_screen = Screen::new(egui::vec2(1400.0, 1000.0));
                for _ in 0..4 {
                    last = Some(sheet_screen.run(vec![], |ui| {
                        crate::ui::theme::sync(ui.ctx());
                        tab.ui(ui, &mut host, 1);
                    }));
                }
                sheet_screen.save("journal-import-dark", last.take().expect("ran"));
                tab.import = None;
            }

            for (section, name) in [
                (Section::Overview, "overview"),
                (Section::Calendar, "calendar"),
                (Section::Analytics, "analytics"),
                (Section::Compare, "compare"),
                (Section::Playbooks, "playbooks"),
                (Section::Daily, "daily"),
            ] {
                host.prefs_mut().section = section;
                for _ in 0..4 {
                    last = Some(screen.run(vec![], |ui| {
                        crate::ui::theme::sync(ui.ctx());
                        tab.ui(ui, &mut host, 1);
                    }));
                }
                screen.save(
                    &format!("journal-{name}-{suffix}"),
                    last.take().expect("ran"),
                );
            }
            host.prefs_mut().section = Section::Trades;
            tab.filter.direction = Some(Direction::Short);
            tab.close_trade(&mut host);
            for _ in 0..4 {
                last = Some(screen.run(vec![], |ui| {
                    crate::ui::theme::sync(ui.ctx());
                    tab.ui(ui, &mut host, 1);
                }));
            }
            screen.save(
                &format!("journal-nomatch-{suffix}"),
                last.take().expect("ran"),
            );
        }
        let store = store();
        let mut host = JournalHost::new(store, Arc::new(|| {}));
        host.rebuild_blocking();
        let mut tab = JournalTab::new(1);
        let mut screen = Screen::new(egui::vec2(1400.0, 600.0));
        let mut last = None;
        for _ in 0..4 {
            last = Some(screen.run(vec![], |ui| {
                crate::ui::theme::sync(ui.ctx());
                tab.ui(ui, &mut host, 1);
            }));
        }
        screen.save("journal-empty-dark", last.take().expect("ran"));
    }
}
