//! The Trade Journal tab: per-window presentation over the app-wide [`JournalHost`]. The header
//! (range, account, filters), the trade table and the trade panel. Everything it shows is read
//! from the host's cached trades; nothing here touches SQLite or paper trading except to save a
//! review through the host.

use std::sync::Arc;

use eframe::egui::{self, Key};
use rust_decimal::Decimal;

use crate::host::journal_host::{
    JournalData, JournalHost, JournalPrefs, RangeChoice, Section, now_ms,
};
use crate::model::journal::breakdown::{self, Dimension, Slice};
use crate::model::journal::calendar::{
    self, Civil, MonthView, days_from_civil, month_view, shift_month,
};
use crate::model::journal::cells::{self, CellContext, Tone};
use crate::model::journal::compare::{self, AccountColumn};
#[cfg(test)]
use crate::model::journal::daily::DailyEntry;
use crate::model::journal::filters::{
    OutcomeFilter, RangePreset, TradeFilter, outcome_of, previous_window, range_for,
};
use crate::model::journal::format;
use crate::model::journal::model::{Direction, Trade};
use crate::model::journal::overview::{
    Input as OverviewInput, Overview, accounts_in_scope, compute, currencies_of,
};
use crate::model::journal::playbook::Playbook;
use crate::model::journal::process::{ProcessScore, process_score};
use crate::model::journal::table::{Column, Sort, SortContext, sort_indices};
use crate::service::timezone::local_utc_offset_secs;
use crate::ui::header_controls;
use crate::ui::icons::Icon;
use crate::ui::journal_analytics;
use crate::ui::journal_calendar;
use crate::ui::journal_compare;
use crate::ui::journal_daily;
use crate::ui::journal_detail::{self, Draft};
use crate::ui::journal_overview::{self, OverviewState};
use crate::ui::journal_playbooks;
use crate::ui::journal_table::{self, TableSpec};
use crate::ui::kit::{self, REGULAR, SEMIBOLD, size};
use crate::ui::paper_grid;
use crate::ui::theme;

const PAGE_MARGIN: f32 = 16.0;
const CONTENT_MAX_WIDTH: f32 = 1480.0;
const DETAIL_WIDTH: f32 = 440.0;
/// Weeks start on Sunday, as the calendar the journal is modelled on does.
const WEEK_START: u8 = 0;
/// A review saves this long after the last keystroke, or when its field loses focus.
const SAVE_AFTER: f64 = 0.8;
const BREAKEVEN_TOLERANCE: Decimal = Decimal::ZERO;

#[derive(Clone, Debug, PartialEq)]
pub enum JournalAction {
    /// Run every paper fill through the journal again.
    RetryIngest,
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
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn title(&self) -> &'static str {
        "Trade Journal"
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
        actions
    }

    fn trades_section(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut JournalHost,
        data: &Arc<JournalData>,
        reviews: &Arc<std::collections::HashMap<String, crate::model::journal::reviews::Review>>,
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
            let emotions = crate::model::journal::reviews::suggestions(
                &crate::model::journal::reviews::DEFAULT_EMOTIONS,
                reviews.values().flat_map(|review| review.emotions.iter()),
            );
            let mistakes = crate::model::journal::reviews::suggestions(
                &crate::model::journal::reviews::DEFAULT_MISTAKES,
                reviews.values().flat_map(|review| review.mistakes.iter()),
            );
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
                            let output = journal_detail::show(
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
                                },
                                draft,
                            );
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
                    .filter(|t| t.status == crate::model::journal::model::TradeStatus::Closed)
                    .filter_map(|t| reviews.get(&t.key))
                    .filter_map(|review| {
                        crate::model::journal::playbook::rule_adherence(review, playbook)
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
        let mood: Vec<String> = crate::model::journal::reviews::suggestions(
            &crate::model::journal::reviews::DEFAULT_EMOTIONS,
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
        reviews: &Arc<std::collections::HashMap<String, crate::model::journal::reviews::Review>>,
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
        reviews: &Arc<std::collections::HashMap<String, crate::model::journal::reviews::Review>>,
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

fn account_label(account: &crate::model::journal::model::JournalAccount) -> String {
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

    impl crate::model::journal::source::TradingActivitySource for Synthetic {
        fn source_id(&self) -> crate::model::journal::model::SourceId {
            crate::model::journal::model::SourceId("synthetic".into())
        }
        fn kind(&self) -> crate::model::journal::model::TradingSourceKind {
            crate::model::journal::model::TradingSourceKind::Imported
        }
        fn capabilities(&self) -> crate::model::journal::source::SourceCapabilities {
            Default::default()
        }
        fn accounts(&self) -> Vec<crate::model::journal::model::JournalAccount> {
            vec![crate::model::journal::model::JournalAccount {
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
            _: Option<&crate::model::journal::source::Cursor>,
        ) -> Result<
            crate::model::journal::source::ExecutionBatch,
            crate::model::journal::source::SourceError,
        > {
            use crate::model::journal::model::Side;
            use crate::model::journal::testing::exec;
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
            Ok(crate::model::journal::source::ExecutionBatch {
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
        assert!(host.data().is_none());
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
        let mut review =
            crate::model::journal::reviews::Review::new(&trade.key, &trade.account_uid);
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
        let mut review =
            crate::model::journal::reviews::Review::new(&trade.key, &trade.account_uid);
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
                crate::model::journal::reviews::Review::new(&trade.key, &trade.account_uid);
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
