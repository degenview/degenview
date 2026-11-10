//! The import sheet: which column is which, what the file's account is, and what the import will
//! do, before anything is written. The preview is recomputed only when an input changes, never
//! per frame; the sheet returns whether the user cancelled or confirmed and the tab runs it.

use eframe::egui::{self, Align, Layout, RichText, Ui, vec2};
use rust_decimal::Decimal;

use super::header_controls;
use super::icons::Icon;
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::sheet;
use super::theme;
use crate::model::DataSource;
use crate::model::journal_markets::AppMarkets;
use crate::model::portfolio_csv::format_iso8601;
use degenview_journal::calendar::Offset;
use degenview_journal::csv::{
    ColumnMap, Field, ImportOptions, NaiveTime, Plan, RowOutcome, Table, interpret, plan,
};
use degenview_journal::format;
use degenview_journal::model::CostBasis;

const WIDTH: f32 = 680.0;
const PREVIEW_ROWS: usize = 6;
const REJECTED_SHOWN: usize = 8;

/// What the sheet shows about the import as it would be done now.
#[derive(Clone, Debug, Default)]
pub struct Preview {
    pub importable: usize,
    pub rejected: Vec<(usize, String)>,
    /// Accounts the import will add to, by name, with the executions each gets.
    pub accounts: Vec<(String, usize)>,
    pub symbols: usize,
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
    /// The first rows as understood, for the table.
    pub sample: Vec<RowOutcome>,
}

pub struct ImportSheet {
    pub file_name: String,
    pub table: Table,
    pub map: ColumnMap,
    pub account_name: String,
    pub currency: String,
    pub balance_text: String,
    pub cost_basis: CostBasis,
    pub naive_time: NaiveTime,
    pub provider: Option<DataSource>,
    pub preview: Preview,
    dirty: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
    Open,
    Cancel,
    Import,
}

impl ImportSheet {
    pub fn new(file_name: String, table: Table) -> Self {
        let map = ColumnMap::detect(&table.headers);
        let stem = file_name
            .rsplit_once('.')
            .map_or(file_name.as_str(), |(stem, _)| stem)
            .trim()
            .to_owned();
        Self {
            account_name: if stem.is_empty() {
                "Imported".to_owned()
            } else {
                stem
            },
            file_name,
            table,
            map,
            currency: "USD".to_owned(),
            balance_text: String::new(),
            cost_basis: CostBasis::AverageCost,
            naive_time: NaiveTime::Utc,
            provider: None,
            preview: Preview::default(),
            dirty: true,
        }
    }

    pub fn options(&self) -> ImportOptions {
        ImportOptions {
            account_name: self.account_name.trim().to_owned(),
            currency: self.currency.trim().to_uppercase(),
            initial_balance: parse_balance(&self.balance_text),
            cost_basis: self.cost_basis,
            naive_time: self.naive_time,
            default_provider: self.provider.map(|source| source.display_name().to_owned()),
        }
    }

    pub fn balance_error(&self) -> bool {
        !self.balance_text.trim().is_empty() && parse_balance(&self.balance_text).is_none()
    }

    /// Why the import cannot run yet, if it cannot.
    pub fn blocker(&self) -> Option<String> {
        let missing = self.map.missing();
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|field| field.label()).collect();
            return Some(format!("Choose a column for: {}.", names.join(", ")));
        }
        if self.account_name.trim().is_empty() {
            return Some("Give the account a name.".to_owned());
        }
        if self.currency.trim().is_empty() {
            return Some("Enter the currency the amounts are in.".to_owned());
        }
        if self.balance_error() {
            return Some("The starting balance is not a number above zero.".to_owned());
        }
        if self.preview.importable == 0 {
            return Some("No row can be imported.".to_owned());
        }
        None
    }

    /// The plan for the rows that can be imported.
    pub fn plan(&self, offset: Offset<'_>, now_ms: i64) -> Plan {
        let options = self.options();
        let outcomes = interpret(&self.table, &self.map, &options, &AppMarkets, offset);
        plan(&outcomes, &options, &AppMarkets, now_ms)
    }

    /// Re-reads every row against the current choices.
    pub fn recompute(&mut self, offset: Offset<'_>) {
        self.dirty = false;
        if !self.map.missing().is_empty() {
            self.preview = Preview::default();
            return;
        }
        let options = self.options();
        let outcomes = interpret(&self.table, &self.map, &options, &AppMarkets, offset);
        let planned = plan(&outcomes, &options, &AppMarkets, 0);
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        let mut symbols = std::collections::BTreeSet::new();
        for execution in &planned.executions {
            let name = planned
                .accounts
                .iter()
                .find(|account| account.uid == execution.account_uid)
                .map_or_else(
                    || execution.account_uid.clone(),
                    |account| account.name.clone(),
                );
            *counts.entry(name).or_default() += 1;
            symbols.insert(execution.instrument.symbol.to_uppercase());
        }
        self.preview = Preview {
            importable: planned.executions.len(),
            rejected: planned.rejected,
            accounts: counts.into_iter().collect(),
            symbols: symbols.len(),
            first_ms: planned.executions.iter().map(|e| e.ts_ms).min(),
            last_ms: planned.executions.iter().map(|e| e.ts_ms).max(),
            sample: outcomes.into_iter().take(PREVIEW_ROWS).collect(),
        };
    }
}

fn parse_balance(text: &str) -> Option<Decimal> {
    let text = text.trim().replace(',', "");
    if text.is_empty() {
        return None;
    }
    text.parse::<Decimal>()
        .ok()
        .filter(|value| *value > Decimal::ZERO)
}

/// Draws the sheet and says what the user did.
pub fn show(ctx: &egui::Context, sheet_state: &mut ImportSheet, offset: Offset<'_>) -> Outcome {
    if sheet_state.dirty {
        sheet_state.recompute(offset);
    }
    let mut outcome = Outcome::Open;
    let before = (
        sheet_state.map.clone(),
        sheet_state.account_name.clone(),
        sheet_state.currency.clone(),
        sheet_state.balance_text.clone(),
        sheet_state.cost_basis,
        sheet_state.naive_time,
        sheet_state.provider,
    );
    let (_, outside) =
        sheet::show_dismissable(ctx, egui::Id::new("journal-import-sheet"), WIDTH, |ui| {
            sheet::header(
                ui,
                Icon::Download,
                "Import executions",
                &format!(
                    "{} \u{b7} {} rows. Nothing is added until you confirm.",
                    sheet_state.file_name,
                    sheet_state.table.rows.len()
                ),
                theme::text_secondary(),
            );
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 220.0).max(260.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    sheet::inset(ui, |ui| {
                        body(ui, sheet_state);
                    });
                });
            sheet::footer_split(
                ui,
                |ui| {
                    if let Some(reason) = sheet_state.blocker() {
                        ui.label(kit::text(
                            reason,
                            size::CAPTION + 1.0,
                            REGULAR,
                            theme::text_tertiary(),
                        ));
                    }
                },
                |ui| {
                    let ready = sheet_state.blocker().is_none();
                    if kit::sheet_button(ui, "Import", true, ready).clicked() {
                        outcome = Outcome::Import;
                    }
                    if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                        outcome = Outcome::Cancel;
                    }
                },
            );
        });
    if outside || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Outcome::Cancel;
    }
    let after = (
        sheet_state.map.clone(),
        sheet_state.account_name.clone(),
        sheet_state.currency.clone(),
        sheet_state.balance_text.clone(),
        sheet_state.cost_basis,
        sheet_state.naive_time,
        sheet_state.provider,
    );
    if before != after {
        sheet_state.dirty = true;
        ctx.request_repaint();
    }
    outcome
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(14.0);
    ui.label(kit::text(
        text.to_uppercase(),
        size::CAPTION,
        SEMIBOLD,
        theme::text_tertiary(),
    ));
    ui.add_space(6.0);
}

fn body(ui: &mut Ui, state: &mut ImportSheet) {
    heading(ui, "Columns");
    let headers: Vec<String> = state.table.headers.clone();
    egui::Grid::new("journal-import-columns")
        .num_columns(4)
        .spacing(vec2(14.0, 6.0))
        .min_col_width(110.0)
        .show(ui, |ui| {
            for (index, field) in Field::ALL.into_iter().enumerate() {
                ui.label(kit::text(
                    if field.required() {
                        format!("{} *", field.label())
                    } else {
                        field.label().to_owned()
                    },
                    size::BODY,
                    REGULAR,
                    theme::text_secondary(),
                ));
                let current = state.map.get(field);
                let title = current
                    .and_then(|index| headers.get(index))
                    .map_or("Not in the file", String::as_str)
                    .to_owned();
                let mut labels: Vec<String> = vec!["Not in the file".to_owned()];
                labels.extend(headers.iter().cloned());
                let mut chosen: Option<Option<usize>> = None;
                header_controls::menu_button(
                    ui,
                    egui::Id::new(("journal-import-column", field.name())),
                    Icon::List,
                    &title,
                    &labels,
                    false,
                    |ui| {
                        if !field.required()
                            && header_controls::choice(ui, "Not in the file", current.is_none())
                        {
                            chosen = Some(None);
                        }
                        for (index, header) in headers.iter().enumerate() {
                            if header_controls::choice(ui, header, current == Some(index)) {
                                chosen = Some(Some(index));
                            }
                        }
                    },
                );
                if let Some(column) = chosen {
                    state.map.set(field, column);
                }
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    ui.add_space(4.0);
    ui.label(kit::text(
        "* required. A column serves one field.",
        size::CAPTION,
        REGULAR,
        theme::text_tertiary(),
    ));

    heading(ui, "Account");
    egui::Grid::new("journal-import-account")
        .num_columns(2)
        .spacing(vec2(16.0, 8.0))
        .min_col_width(150.0)
        .show(ui, |ui| {
            label(ui, "Name");
            text_field(
                ui,
                "name",
                &mut state.account_name,
                "Account name",
                false,
                260.0,
            );
            ui.end_row();
            label(ui, "Currency");
            text_field(ui, "currency", &mut state.currency, "USD", false, 90.0);
            ui.end_row();
            label(ui, "Starting balance");
            let error = state.balance_error();
            text_field(
                ui,
                "balance",
                &mut state.balance_text,
                "Optional",
                error,
                160.0,
            );
            ui.end_row();
            label(ui, "Cost basis");
            kit::segmented(
                ui,
                egui::Id::new("journal-import-basis"),
                &[
                    (CostBasis::AverageCost, "Average"),
                    (CostBasis::Fifo, "FIFO"),
                    (CostBasis::Lifo, "LIFO"),
                ],
                &mut state.cost_basis,
            );
            ui.end_row();
            label(ui, "Times with no zone");
            kit::segmented(
                ui,
                egui::Id::new("journal-import-zone"),
                &[
                    (NaiveTime::Utc, "UTC"),
                    (NaiveTime::Local, "This Mac's zone"),
                ],
                &mut state.naive_time,
            );
            ui.end_row();
            label(ui, "Chart provider");
            let title = state
                .provider
                .map_or("None", |source| source.display_name())
                .to_owned();
            let mut labels = vec!["None".to_owned()];
            labels.extend(DataSource::ALL.iter().map(|s| s.display_name().to_owned()));
            let mut chosen: Option<Option<DataSource>> = None;
            header_controls::menu_button(
                ui,
                egui::Id::new("journal-import-provider"),
                Icon::ChartLine,
                &title,
                &labels,
                false,
                |ui| {
                    if header_controls::choice(ui, "None", state.provider.is_none()) {
                        chosen = Some(None);
                    }
                    for source in DataSource::ALL {
                        if header_controls::choice(
                            ui,
                            source.display_name(),
                            state.provider == Some(source),
                        ) {
                            chosen = Some(Some(source));
                        }
                    }
                },
            );
            if let Some(provider) = chosen {
                state.provider = provider;
            }
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.label(kit::text(
        "A row in a file's own account column goes to that account. The provider only links trades \
         to charts; rows that name one keep it. Importing the same file again adds nothing.",
        size::CAPTION,
        REGULAR,
        theme::text_tertiary(),
    ));

    heading(ui, "What will happen");
    preview(ui, state);
}

fn label(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        size::BODY,
        REGULAR,
        theme::text_secondary(),
    ));
}

fn text_field(ui: &mut Ui, salt: &str, text: &mut String, hint: &str, error: bool, width: f32) {
    let id = egui::Id::new(("journal-import-field", salt));
    let focused = ui.memory(|memory| memory.has_focus(id));
    let frame = if error {
        kit::field_frame(focused).stroke(egui::Stroke::new(1.0, theme::LOSS))
    } else {
        kit::field_frame(focused)
    };
    frame
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .id(id)
                    .desired_width(width)
                    .frame(egui::Frame::NONE)
                    .hint_text(RichText::new(hint).color(theme::text_tertiary())),
            )
        });
}

fn preview(ui: &mut Ui, state: &ImportSheet) {
    let preview = &state.preview;
    if !state.map.missing().is_empty() {
        ui.label(kit::text(
            "Choose the required columns to see what the file would add.",
            size::BODY,
            REGULAR,
            theme::text_secondary(),
        ));
        return;
    }
    let rows = state.table.rows.len();
    ui.label(kit::text(
        format!(
            "{} of {rows} rows can be imported, {} will be refused.",
            preview.importable,
            preview.rejected.len()
        ),
        size::BODY,
        MEDIUM,
        theme::text_primary(),
    ));
    if let (Some(first), Some(last)) = (preview.first_ms, preview.last_ms) {
        ui.label(kit::text(
            format!(
                "{} to {}, {} symbol{}.",
                &format_iso8601(first)[..10],
                &format_iso8601(last)[..10],
                preview.symbols,
                if preview.symbols == 1 { "" } else { "s" }
            ),
            size::BODY,
            REGULAR,
            theme::text_secondary(),
        ));
    }
    for (name, count) in &preview.accounts {
        ui.label(kit::text(
            format!("Account \u{201c}{name}\u{201d}: {count} executions"),
            size::BODY,
            REGULAR,
            theme::text_secondary(),
        ));
    }
    ui.add_space(8.0);
    egui::Grid::new("journal-import-sample")
        .num_columns(6)
        .spacing(vec2(14.0, 5.0))
        .striped(true)
        .show(ui, |ui| {
            for head in ["Row", "Time", "Symbol", "Side", "Quantity", "Price"] {
                ui.label(kit::text(
                    head,
                    size::CAPTION,
                    SEMIBOLD,
                    theme::text_tertiary(),
                ));
            }
            ui.end_row();
            for outcome in &preview.sample {
                let cell = |ui: &mut Ui, text: String, color: egui::Color32| {
                    ui.label(kit::text(text, size::CAPTION + 1.0, REGULAR, color));
                };
                cell(ui, outcome.row.to_string(), theme::text_tertiary());
                match &outcome.result {
                    Ok(candidate) => {
                        cell(
                            ui,
                            format_iso8601(candidate.ts_ms)[..19].replace('T', " "),
                            theme::text_primary(),
                        );
                        cell(ui, candidate.symbol.clone(), theme::text_primary());
                        let (side, color) = match candidate.side {
                            degenview_journal::model::Side::Buy => ("Buy", theme::WIN),
                            degenview_journal::model::Side::Sell => ("Sell", theme::LOSS),
                        };
                        cell(ui, side.to_owned(), color);
                        cell(
                            ui,
                            format::quantity(candidate.quantity),
                            theme::text_primary(),
                        );
                        cell(ui, format::price(candidate.price), theme::text_primary());
                    }
                    Err(reason) => {
                        cell(ui, format!("Refused: {reason}"), theme::LOSS);
                    }
                }
                ui.end_row();
            }
        });
    if !preview.rejected.is_empty() {
        ui.add_space(10.0);
        ui.label(kit::text(
            "Refused rows",
            size::CAPTION + 1.0,
            MEDIUM,
            theme::text_secondary(),
        ));
        ui.add_space(3.0);
        for (row, reason) in preview.rejected.iter().take(REJECTED_SHOWN) {
            ui.label(kit::text(
                format!("Row {row}: {reason}"),
                size::CAPTION + 1.0,
                REGULAR,
                theme::LOSS,
            ));
        }
        if preview.rejected.len() > REJECTED_SHOWN {
            ui.label(kit::text(
                format!("and {} more.", preview.rejected.len() - REJECTED_SHOWN),
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
        }
    }
    let _ = Layout::left_to_right(Align::Center);
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_journal::csv::read_table;

    fn sheet_for(text: &str) -> ImportSheet {
        let mut sheet = ImportSheet::new("fills.csv".into(), read_table(text).unwrap());
        sheet.recompute(&|_| 0);
        sheet
    }

    const FILE: &str = "time,symbol,side,qty,price\n\
        2026-11-09T10:00:00Z,BTC,buy,1,100\n\
        2026-11-09T11:00:00Z,BTC,sell,1,110\n\
        bad,BTC,buy,1,100\n";

    #[test]
    fn the_sheet_names_the_account_after_the_file_and_previews_what_the_rows_do() {
        let sheet = sheet_for(FILE);
        assert_eq!(sheet.account_name, "fills");
        assert_eq!(sheet.preview.importable, 2);
        assert_eq!(sheet.preview.rejected.len(), 1);
        assert_eq!(sheet.preview.rejected[0].0, 4);
        assert_eq!(sheet.preview.accounts, [("fills".to_owned(), 2)]);
        assert_eq!(sheet.preview.symbols, 1);
        assert_eq!(sheet.blocker(), None);
        let plan = sheet.plan(&|_| 0, 5);
        assert_eq!(plan.executions.len(), 2);
    }

    #[test]
    fn what_blocks_the_import_is_said_in_words() {
        let mut sheet = sheet_for(FILE);
        sheet.map.set(Field::Price, None);
        sheet.recompute(&|_| 0);
        assert!(sheet.blocker().unwrap().contains("Price"));
        let mut sheet = sheet_for(FILE);
        sheet.account_name = " ".into();
        assert!(sheet.blocker().unwrap().contains("name"));
        sheet.account_name = "x".into();
        sheet.balance_text = "abc".into();
        assert!(sheet.blocker().unwrap().contains("balance"));
        sheet.balance_text = "10,000".into();
        assert_eq!(
            sheet.options().initial_balance,
            Some("10000".parse().unwrap())
        );
        let all_bad = sheet_for("time,symbol,side,qty,price\nbad,BTC,buy,1,100\n");
        assert!(all_bad.blocker().unwrap().contains("No row"));
    }

    #[test]
    fn changing_an_option_changes_the_preview() {
        let mut sheet = sheet_for(
            "time,symbol,side,qty,price,currency\n2026-11-09T10:00:00Z,BTC,buy,1,100,EUR\n",
        );
        assert_eq!(sheet.preview.importable, 0);
        sheet.currency = "eur".into();
        sheet.recompute(&|_| 0);
        assert_eq!(sheet.preview.importable, 1);
    }

    #[test]
    fn the_sheet_draws_every_state_and_reports_the_buttons() {
        let ctx = egui::Context::default();
        theme::install_offscreen(&ctx);
        for text in [FILE, "time,symbol,side,qty,price\nbad,BTC,buy,1,100\n"] {
            let mut sheet = sheet_for(text);
            let mut outcome = Outcome::Cancel;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        vec2(1000.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| outcome = show(ui.ctx(), &mut sheet, &|_| 0),
            );
            out.textures_delta.clear();
            assert_eq!(outcome, Outcome::Open);
        }
        let mut missing = sheet_for(FILE);
        missing.map.set(Field::Symbol, None);
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    vec2(1000.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), &mut missing, &|_| 0);
            },
        );
        out.textures_delta.clear();
    }
}
