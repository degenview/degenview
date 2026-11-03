//! The last step of any CSV import: what is about to be written, with the problems found.
//! A pure view over a `CsvPreview`; the host runs the import and reports a failure back through
//! `ImportPreviewState::failed`.

// Transient: the portfolio host opens this once the importers are wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioCurrency, PortfolioTransaction};
use crate::model::portfolio_accounting::unique_assets;
use crate::model::portfolio_csv::CsvPreview;
use crate::model::portfolio_format::{PortfolioValueFormatter, QuantitySign, Tone};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::portfolio_overview::{StatCard, stat_card};
use crate::ui::portfolio_table::{HEADER_HEIGHT, ROW_HEIGHT, header_cell, numeric_cell, pill};
use crate::ui::portfolio_transactions::{date_time_text, type_name, type_tint};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, RichText, Vec2};
use egui_extras::{Column, TableBuilder};

pub const SIZE: Vec2 = Vec2::new(680.0, 600.0);

#[derive(Default)]
pub struct ImportPreviewState {
    pub is_importing: bool,
    pub failure: Option<String>,
}

impl ImportPreviewState {
    /// The host's answer to a failed `Import`: the sheet stays open with the reason.
    pub fn failed(&mut self, message: impl Into<String>) {
        self.is_importing = false;
        self.failure = Some(message.into());
    }
}

pub struct ImportPreviewInput<'a> {
    pub preview: &'a CsvPreview,
    pub utc_offset_secs: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportPreviewOutcome {
    Import,
    Cancel,
}

// MARK: Pure text

/// Newest first, as the table lists them.
pub fn sorted_rows(preview: &CsvPreview) -> Vec<&PortfolioTransaction> {
    let mut rows: Vec<&PortfolioTransaction> = preview.transactions.iter().collect();
    rows.sort_by_key(|transaction| std::cmp::Reverse(transaction.timestamp));
    rows
}

pub fn asset_count(preview: &CsvPreview) -> usize {
    unique_assets(&preview.transactions).len()
}

pub fn can_import(preview: &CsvPreview, state: &ImportPreviewState) -> bool {
    preview.is_valid() && !preview.transactions.is_empty() && !state.is_importing
}

pub fn subtitle(preview: &CsvPreview) -> &'static str {
    if preview.transactions.is_empty() {
        "Nothing to import yet."
    } else {
        "Review what will be added to your portfolio."
    }
}

pub fn plural(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

pub fn error_title(count: usize) -> String {
    format!(
        "{count} {}",
        if count == 1 {
            "problem requires attention"
        } else {
            "problems require attention"
        }
    )
}

pub fn warning_title(count: usize) -> String {
    plural(count, "warning", "warnings")
}

pub fn import_button_title(count: usize) -> String {
    format!("Import {}", plural(count, "Transaction", "Transactions"))
}

/// The third stat: errors when there are any, else warnings.
pub fn problem_card(preview: &CsvPreview) -> (&'static str, usize, Option<Tone>) {
    if preview.errors.is_empty() {
        (
            "Warnings",
            preview.warnings.len(),
            // Orange has no `Tone`; warnings are drawn in the stat's neutral accent.
            None,
        )
    } else {
        ("Errors", preview.errors.len(), Some(Tone::Loss))
    }
}

pub fn failure_detail(message: &str) -> String {
    format!(
        "{message}\n\nNo transactions were imported. Correct the source data, or restart the \
         CoinMarketCap import and skip the affected ticker or item."
    )
}

// MARK: View

pub fn show(
    ctx: &egui::Context,
    state: &mut ImportPreviewState,
    input: &ImportPreviewInput,
) -> Option<ImportPreviewOutcome> {
    let mut outcome = None;
    let preview = input.preview;
    sheet::show(ctx, Id::new("portfolio-import-preview"), SIZE.x, |ui| {
        ui.set_height(SIZE.y);
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Download,
                "Import Preview",
                subtitle(preview),
                theme::SYSTEM_ACCENT,
            );
            ui.add_space(18.0);
            summary(ui, preview);
            ui.add_space(12.0);
            notices(ui, preview, state);
        });
        let footer_height = 52.0;
        let table_height = (ui.available_height() - footer_height).max(120.0);
        ui.allocate_ui(Vec2::new(ui.available_width(), table_height), |ui| {
            content(ui, preview, input.utc_offset_secs);
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            footer(ui, state, preview, &mut outcome);
            ui.add_space(10.0);
        });
    });
    if outcome.is_none()
        && !state.is_importing
        && ctx.input(|input| input.key_pressed(egui::Key::Escape))
    {
        outcome = Some(ImportPreviewOutcome::Cancel);
    }
    outcome
}

fn summary(ui: &mut egui::Ui, preview: &CsvPreview) {
    let (title, count, tone) = problem_card(preview);
    let cards = [
        StatCard::new("Transactions", preview.transactions.len().to_string())
            .icon(crate::ui::icons::Icon::Exchange),
        StatCard::new("Assets", asset_count(preview).to_string())
            .icon(crate::ui::icons::Icon::Layers),
        StatCard::new(title, count.to_string())
            .tone(tone)
            .icon(if tone.is_some() {
                crate::ui::icons::Icon::XmarkOctagonFill
            } else {
                crate::ui::icons::Icon::Triangle
            }),
    ];
    ui.columns(3, |columns| {
        for (column, card) in columns.iter_mut().zip(&cards) {
            stat_card(column, card);
        }
    });
}

fn notice(ui: &mut egui::Ui, tint: Color32, title: &str, detail: &str, lines: &[String], max: f32) {
    egui::Frame::new()
        .fill(tint.gamma_multiply(0.12))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().color(tint));
            ui.label(RichText::new(detail).small().weak());
            if !lines.is_empty() {
                egui::ScrollArea::vertical()
                    .id_salt(title)
                    .max_height(max)
                    .show(ui, |ui| {
                        for line in lines {
                            ui.label(RichText::new(format!("• {line}")).small());
                        }
                    });
            }
        });
    ui.add_space(6.0);
}

fn notices(ui: &mut egui::Ui, preview: &CsvPreview, state: &ImportPreviewState) {
    if !preview.errors.is_empty() {
        notice(
            ui,
            theme::DOWN,
            &format!("✕ {}", error_title(preview.errors.len())),
            "Fix the source data and import again.",
            &preview.errors,
            90.0,
        );
    }
    if !preview.warnings.is_empty() {
        notice(
            ui,
            theme::REPLAY,
            &format!("⚠ {}", warning_title(preview.warnings.len())),
            "Warnings don't block the import.",
            &preview.warnings,
            70.0,
        );
    }
    if let Some(failure) = &state.failure {
        notice(
            ui,
            theme::DOWN,
            "⚠ Import failed",
            &failure_detail(failure),
            &[],
            0.0,
        );
    }
}

fn content(ui: &mut egui::Ui, preview: &CsvPreview, utc_offset_secs: i64) {
    let rows = sorted_rows(preview);
    if rows.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("No Transactions").strong().size(15.0));
            ui.label(
                RichText::new("The file didn't contain any transactions that can be imported.")
                    .weak(),
            );
        });
        return;
    }
    let formatter = PortfolioValueFormatter::new(PortfolioCurrency::Usd, false);
    let height = ui.available_height();
    TableBuilder::new(ui)
        .id_salt("portfolio-import-preview-table")
        .striped(true)
        .max_scroll_height(height)
        .column(Column::initial(150.0).at_least(120.0).clip(true))
        .column(Column::initial(70.0).at_least(55.0))
        .column(Column::initial(100.0).at_least(85.0))
        .column(Column::initial(100.0).at_least(70.0))
        .column(Column::remainder().at_least(85.0))
        .header(HEADER_HEIGHT, |mut header| {
            for (index, title) in ["Date", "Asset", "Type", "Quantity", "Price"]
                .into_iter()
                .enumerate()
            {
                header.col(|ui| {
                    header_cell(ui, title, None, false, index >= 3);
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, rows.len(), |mut row| {
                let transaction = rows[row.index()];
                row.col(|ui| {
                    ui.label(date_time_text(transaction.timestamp, utc_offset_secs));
                });
                row.col(|ui| {
                    ui.label(RichText::new(transaction.asset.display_ticker()).strong());
                });
                row.col(|ui| {
                    pill(ui, type_name(transaction.kind), type_tint(transaction.kind));
                });
                row.col(|ui| {
                    numeric_cell(
                        ui,
                        &formatter.quantity(transaction.quantity, QuantitySign::None),
                        None,
                    );
                });
                row.col(|ui| {
                    let price = transaction.price.map_or("—".to_owned(), |price| {
                        transaction.price_currency.format_price(price)
                    });
                    numeric_cell(ui, &price, None);
                });
            });
        });
}

fn footer(
    ui: &mut egui::Ui,
    state: &mut ImportPreviewState,
    preview: &CsvPreview,
    outcome: &mut Option<ImportPreviewOutcome>,
) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if state.is_importing {
            ui.spinner();
            ui.label(RichText::new("Validating and importing…").weak());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let title = import_button_title(preview.transactions.len());
            if kit::footer_button(
                ui,
                &title,
                kit::FooterButton::Prominent(theme::SYSTEM_ACCENT),
                can_import(preview, state),
                150.0,
            )
            .clicked()
            {
                state.is_importing = true;
                state.failure = None;
                *outcome = Some(ImportPreviewOutcome::Import);
            }
            if kit::footer_button(
                ui,
                "Cancel",
                kit::FooterButton::Secondary,
                !state.is_importing,
                0.0,
            )
            .clicked()
            {
                *outcome = Some(ImportPreviewOutcome::Cancel);
            }
        });
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioAsset, PortfolioSource, PortfolioTransactionType};
    use std::collections::HashMap;

    fn transaction(id: u64, symbol: &str, timestamp: i64) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: 1,
            asset: PortfolioAsset {
                key: format!("Binance:{symbol}"),
                symbol: symbol.into(),
                name: symbol.into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            kind: PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(10.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp,
            notes: String::new(),
            source: PortfolioSource::Csv,
            external_transaction_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn preview(transactions: Vec<PortfolioTransaction>) -> CsvPreview {
        CsvPreview {
            transactions,
            ..CsvPreview::default()
        }
    }

    #[test]
    fn rows_list_newest_first_and_assets_count_once() {
        let preview = preview(vec![
            transaction(1, "BTCUSDT", 100),
            transaction(2, "ETHUSDT", 300),
            transaction(3, "BTCUSDT", 200),
        ]);
        let ids: Vec<u64> = sorted_rows(&preview).iter().map(|t| t.id).collect();
        assert_eq!(ids, [2, 3, 1]);
        assert_eq!(asset_count(&preview), 2);
    }

    #[test]
    fn importing_needs_a_clean_non_empty_preview() {
        let state = ImportPreviewState::default();
        let mut good = preview(vec![transaction(1, "BTCUSDT", 1)]);
        assert!(can_import(&good, &state));
        good.warnings.push("heads up".into());
        assert!(can_import(&good, &state), "warnings don't block");
        good.errors.push("bad row".into());
        assert!(!can_import(&good, &state));
        assert!(!can_import(&preview(vec![]), &state));
        let busy = ImportPreviewState {
            is_importing: true,
            failure: None,
        };
        assert!(!can_import(&preview(vec![transaction(1, "X", 1)]), &busy));
    }

    #[test]
    fn a_failure_reopens_the_button_and_shows_why() {
        let mut state = ImportPreviewState {
            is_importing: true,
            failure: None,
        };
        state.failed("Sell exceeds holdings");
        assert!(!state.is_importing);
        assert_eq!(state.failure.as_deref(), Some("Sell exceeds holdings"));
        assert!(failure_detail("x").contains("No transactions were imported."));
    }

    #[test]
    fn the_third_stat_prefers_errors_and_words_agree_with_counts() {
        let mut preview = preview(vec![]);
        assert_eq!(problem_card(&preview), ("Warnings", 0, None));
        preview.warnings.push("w".into());
        assert_eq!(problem_card(&preview), ("Warnings", 1, None));
        preview.errors.extend(["e1".to_owned(), "e2".to_owned()]);
        assert_eq!(problem_card(&preview), ("Errors", 2, Some(Tone::Loss)));
        assert_eq!(error_title(1), "1 problem requires attention");
        assert_eq!(error_title(2), "2 problems require attention");
        assert_eq!(warning_title(1), "1 warning");
        assert_eq!(warning_title(3), "3 warnings");
        assert_eq!(import_button_title(1), "Import 1 Transaction");
        assert_eq!(import_button_title(0), "Import 0 Transactions");
        assert_eq!(subtitle(&preview), "Nothing to import yet.");
    }
}
