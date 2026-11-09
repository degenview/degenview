//! The Trade Journal tab: per-window presentation over the app-wide [`JournalHost`]. The header
//! (range, account, filters), the trade table and the trade panel. Everything it shows is read
//! from the host's cached trades; nothing here touches SQLite or paper trading except to save a
//! review through the host.

use std::sync::Arc;

use eframe::egui::{self, Key};
use rust_decimal::Decimal;

use crate::host::journal_host::{JournalData, JournalHost, JournalPrefs, RangeChoice, now_ms};
use crate::model::journal::calendar::{self, days_from_civil};
use crate::model::journal::cells::{self, CellContext, Tone};
use crate::model::journal::filters::{OutcomeFilter, RangePreset, TradeFilter, range_for};
use crate::model::journal::format;
use crate::model::journal::model::Direction;
use crate::model::journal::table::{Column, Sort, SortContext, sort_indices};
use crate::service::timezone::local_utc_offset_secs;
use crate::ui::header_controls;
use crate::ui::icons::Icon;
use crate::ui::journal_detail::{self, Draft};
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
        host.flush_prefs_if_idle(ui.ctx());
        actions
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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.columns_menu(ui, host);
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
        host.rebuild_blocking();
        (store, host, JournalTab::new(1), egui::Context::default())
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
            let (_, mut host, mut tab, _) = ready();
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
