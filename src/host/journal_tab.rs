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
use crate::model::journal::calendar::{
    self, Civil, MonthView, days_from_civil, month_view, shift_month,
};
use crate::model::journal::cells::{self, CellContext, Tone};
use crate::model::journal::filters::{
    OutcomeFilter, RangePreset, TradeFilter, outcome_of, previous_window, range_for,
};
use crate::model::journal::format;
use crate::model::journal::model::{Direction, Trade};
use crate::model::journal::overview::{
    Input as OverviewInput, Overview, accounts_in_scope, compute, currencies_of,
};
use crate::model::journal::table::{Column, Sort, SortContext, sort_indices};
use crate::service::timezone::local_utc_offset_secs;
use crate::ui::header_controls;
use crate::ui::icons::Icon;
use crate::ui::journal_calendar;
use crate::ui::journal_detail::{self, Draft};
use crate::ui::journal_overview::{self, OverviewState};
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

        egui::Panel::top(egui::Id::new(("journal-header", self.id)))
            .exact_size(92.0)
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
        match section {
            Section::Trades => self.trades_section(ui, host, &data, &reviews),
            Section::Overview => self.overview_section(ui, host, &data),
            Section::Calendar => self.calendar_section(ui, host, &data),
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
                breakeven_tolerance: BREAKEVEN_TOLERANCE,
                offset: &local_utc_offset_secs,
            });
            self.overview = Some((key, overview));
        }
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
            let text = format::iso_date(day);
            self.custom_from = text.clone();
            self.custom_to = text.clone();
            let prefs = host.prefs_mut();
            prefs.range = RangeChoice::Custom {
                from: text.clone(),
                to: text,
            };
            prefs.section = Section::Trades;
            self.range_cache = None;
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

    fn header(&mut self, ui: &mut egui::Ui, host: &mut JournalHost, data: &Arc<JournalData>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let mut section = host.prefs().section;
            let items = [
                (Section::Overview, "Overview", Icon::PieChart),
                (Section::Calendar, "Calendar", Icon::Calendar),
                (Section::Trades, "Trades", Icon::DocText),
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
            ui.add_space(6.0);
            let show_range = section != Section::Calendar;
            let mut kind = self.range_choice(host);
            let before = kind;
            if !show_range {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.account_menu(ui, host, data);
                });
                return;
            }
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

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.account_menu(ui, host, data);
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            self.filter_menus(ui, host, data);
            let active = self.filter.is_active()
                || host.prefs().account.is_some()
                || host.prefs().range != RangeChoice::Preset(RangePreset::All);
            if active && kit::sheet_button(ui, "Reset", false, true).clicked() {
                self.reset_filters(host);
            }
            let section = host.prefs().section;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if section == Section::Trades {
                    self.columns_menu(ui, host);
                }
                if section == Section::Calendar {
                    return;
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
