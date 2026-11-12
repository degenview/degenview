//! The last step of any CSV import: what is about to be written, with the problems found.
//! A pure view over a `CsvPreview`; the host runs the import and reports a failure back through
//! `ImportPreviewState::failed`.

// Transient: the portfolio host opens this once the importers are wired.
#![allow(dead_code)]

use crate::model::portfolio::accounting::unique_assets;
use crate::model::portfolio::csv::CsvPreview;
use crate::model::portfolio::format::{PortfolioValueFormatter, QuantitySign};
use crate::model::portfolio::{PortfolioCurrency, PortfolioTransaction};
use crate::ui::icons::Icon;
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::sheet;
use crate::ui::portfolio::empty;
use crate::ui::portfolio::grid::{self, Col, Navigation, RowContext, TableSpec};
use crate::ui::portfolio::overview::activity_caption;
use crate::ui::portfolio::sheet_parts::{card_background, problem_card, status_dot};
use crate::ui::portfolio::transactions::{
    asset_subtitle, kind_counts, paint_asset_cell, paint_date_cell, paint_type_capsule,
    paint_type_tick, quantity_sign, type_mix, type_name,
};
use crate::ui::theme;
use crate::ui::{
    chart::add::{art, sheet as add_chart_sheet},
    kit,
};
use eframe::egui::{self, Align2, Color32, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

pub const SIZE: Vec2 = Vec2::new(720.0, 680.0);

#[derive(Default)]
pub struct ImportPreviewState {
    pub is_importing: bool,
    pub failure: Option<String>,
    selected: Option<usize>,
    navigation: Navigation,
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

/// How serious one footer status is; picks its dot's colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Good,
    Warning,
    Error,
}

impl Status {
    pub fn color(self) -> Color32 {
        match self {
            Self::Good => theme::UP,
            Self::Warning => theme::REPLAY,
            Self::Error => theme::DOWN,
        }
    }
}

/// What the footer says about the file: problems that block, warnings that don't, or that it is
/// ready.
pub fn footer_status(preview: &CsvPreview) -> Vec<(Status, String)> {
    let mut status = Vec::new();
    if !preview.errors.is_empty() {
        status.push((
            Status::Error,
            format!("{} to fix", plural(preview.errors.len(), "error", "errors")),
        ));
    } else if preview.transactions.is_empty() {
        status.push((Status::Warning, "Nothing to import".to_owned()));
    } else {
        status.push((Status::Good, "Ready to import".to_owned()));
    }
    if !preview.warnings.is_empty() {
        status.push((Status::Warning, warning_title(preview.warnings.len())));
    }
    status
}

pub fn failure_detail(message: &str) -> String {
    format!(
        "{message}\n\nNo transactions were imported. Correct the source data, or restart the \
         CoinMarketCap import and skip the affected ticker or item."
    )
}

// MARK: View

const HEADER_PADDING: f32 = 24.0;
/// The footer's hairline, padding and 28 pt buttons.
const FOOTER_HEIGHT: f32 = 58.0;
const BAND_HEIGHT: f32 = 92.0;
const ROW_HEIGHT: f32 = 48.0;
/// The table keeps at least this tall, however many notices sit above it.
const TABLE_MIN_HEIGHT: f32 = 150.0;

/// Date, asset, type, quantity, price, total.
const COLUMNS: [Col; 6] = [
    Col::new(104.0, 140.0, false).unsortable(),
    Col::new(120.0, 140.0, false).unsortable(),
    Col::new(100.0, 110.0, false).unsortable(),
    Col::new(80.0, 90.0, true).unsortable(),
    Col::new(96.0, 100.0, true).unsortable(),
    Col::new(100.0, 110.0, true).unsortable(),
];

pub fn show(
    ctx: &egui::Context,
    state: &mut ImportPreviewState,
    input: &ImportPreviewInput,
) -> Option<ImportPreviewOutcome> {
    let mut outcome = None;
    let preview = input.preview;
    sheet::show(ctx, Id::new("portfolio-import-preview"), SIZE.x, |ui| {
        ui.set_height(SIZE.y);
        let top = ui.cursor().top();
        ui.add_space(HEADER_PADDING);
        sheet::inset(ui, |ui| {
            add_chart_sheet::header(
                ui,
                art::Scene::Portfolio,
                "Import Preview",
                subtitle(preview),
            );
            ui.add_space(16.0);
            summary_band(ui, preview);
            ui.add_space(12.0);
            notices(ui, preview, state);
        });
        // `set_height` made the sheet's own rect SIZE.y tall, so measure from the cursor.
        let used = ui.cursor().top() - top;
        let height = (SIZE.y - used - FOOTER_HEIGHT - 16.0).max(TABLE_MIN_HEIGHT);
        sheet::inset(ui, |ui| {
            let (outer, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
            table(ui, outer, state, preview, input.utc_offset_secs);
        });
        ui.add_space(16.0);
        let importing = state.is_importing;
        sheet::footer_split(
            ui,
            |ui| status(ui, importing, preview),
            |ui| {
                let title = import_button_title(preview.transactions.len());
                if kit::footer_button(
                    ui,
                    &title,
                    kit::FooterButton::Primary,
                    can_import(preview, state),
                    170.0,
                )
                .clicked()
                {
                    state.is_importing = true;
                    state.failure = None;
                    outcome = Some(ImportPreviewOutcome::Import);
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
                    outcome = Some(ImportPreviewOutcome::Cancel);
                }
            },
        );
    });
    if outcome.is_none()
        && !state.is_importing
        && ctx.input(|input| input.key_pressed(egui::Key::Escape))
    {
        outcome = Some(ImportPreviewOutcome::Cancel);
    }
    outcome
}

/// How many transactions come in, from how many assets and of what kind: the figure, a
/// buys-and-sells line, and one bar of the mix by type.
fn summary_band(ui: &mut Ui, preview: &CsvPreview) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAND_HEIGHT), Sense::hover());
    card_background(ui, rect, None);
    let inner = rect.shrink2(vec2(18.0, 14.0));
    let secondary = theme::text_secondary();
    let count = preview.transactions.len();
    let by_kind = kind_counts(preview.transactions.iter().map(|tx| tx.kind));
    let count_of = |kind| {
        by_kind
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map_or(0, |(_, count)| *count)
    };

    let caption = grid::put(
        ui,
        inner.left_top(),
        Align2::LEFT_TOP,
        if count == 1 {
            "Transaction"
        } else {
            "Transactions"
        },
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        secondary,
    );
    let figure = grid::put(
        ui,
        pos2(inner.left(), inner.top() + 15.0),
        Align2::LEFT_TOP,
        &count.to_string(),
        26.0,
        kit::SEMIBOLD,
        theme::text_primary(),
    );
    let activity = grid::put(
        ui,
        pos2(inner.left(), inner.bottom() - 15.0),
        Align2::LEFT_TOP,
        &format!(
            "{} \u{00B7} {}",
            activity_caption(
                count_of(crate::model::portfolio::PortfolioTransactionType::Buy),
                count_of(crate::model::portfolio::PortfolioTransactionType::Sell)
            ),
            plural(asset_count(preview), "asset", "assets")
        ),
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        secondary,
    );

    let divider = caption.right().max(figure.right()).max(activity.right()) + 26.0;
    let area = Rect::from_min_max(pos2(divider + 26.0, inner.top()), inner.right_bottom());
    if count == 0 || area.width() < 200.0 {
        return;
    }
    ui.painter().vline(
        divider,
        rect.top() + 16.0..=rect.bottom() - 16.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
    );
    grid::put(
        ui,
        area.left_top(),
        Align2::LEFT_TOP,
        "By Type",
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        secondary,
    );
    type_mix(ui, area, &by_kind, count);
}

fn notices(ui: &mut Ui, preview: &CsvPreview, state: &ImportPreviewState) {
    if !preview.errors.is_empty() {
        problem_card(
            ui,
            Icon::XmarkOctagonFill,
            theme::DOWN,
            &error_title(preview.errors.len()),
            Some("Fix the source data and import again."),
            &preview.errors,
            84.0,
        );
        ui.add_space(8.0);
    }
    if let Some(failure) = &state.failure {
        problem_card(
            ui,
            Icon::TriangleFill,
            theme::DOWN,
            "Import failed",
            Some(&failure_detail(failure)),
            &[],
            0.0,
        );
        ui.add_space(8.0);
    }
    if !preview.warnings.is_empty() {
        problem_card(
            ui,
            Icon::TriangleFill,
            theme::REPLAY,
            &warning_title(preview.warnings.len()),
            Some("Warnings don't block the import."),
            &preview.warnings,
            64.0,
        );
        ui.add_space(8.0);
    }
}

/// The transactions that will be written, newest first, in the same cells as the Transactions
/// table.
fn table(
    ui: &mut Ui,
    outer: Rect,
    state: &mut ImportPreviewState,
    preview: &CsvPreview,
    utc_offset_secs: i64,
) {
    grid::paint_frame(ui, outer);
    let rows = sorted_rows(preview);
    if rows.is_empty() {
        empty::compact(
            ui,
            outer,
            empty::Scene::Transactions,
            "No Transactions",
            "The file didn't contain any transactions that can be imported.",
        );
        return;
    }
    let headers: Vec<String> = ["Date", "Asset", "Type", "Quantity", "Price", "Total"]
        .iter()
        .map(|title| (*title).to_owned())
        .collect();
    let formatter = PortfolioValueFormatter::new(PortfolioCurrency::Usd, false);
    let output = grid::show_table(
        ui,
        outer,
        &TableSpec {
            id: "portfolio-import-preview",
            columns: &COLUMNS,
            headers: &headers,
            row_height: ROW_HEIGHT,
            count: rows.len(),
            sorted: None,
            selected: state.selected,
            has_footer: false,
        },
        &mut state.navigation,
        |ui, row| preview_row(ui, rows[row.index], row, &formatter, utc_offset_secs),
    );
    if let Some(index) = output.row_clicked.or(output.moved_to) {
        state.selected = Some(index);
    }
}

fn preview_row(
    ui: &mut Ui,
    tx: &PortfolioTransaction,
    row: &RowContext,
    formatter: &PortfolioValueFormatter,
    utc_offset_secs: i64,
) {
    let cells = row.cells;
    let primary = theme::text_primary();
    paint_type_tick(ui, cells[0], ROW_HEIGHT, tx.kind);
    paint_date_cell(ui, cells[0], &COLUMNS[0], tx.timestamp, utc_offset_secs);
    paint_asset_cell(
        ui,
        cells[1],
        &tx.asset,
        &tx.asset.display_ticker(),
        &asset_subtitle(&tx.asset),
    );
    paint_type_capsule(ui, cells[2], tx.kind, type_name(tx.kind));
    let quantity_color = match quantity_sign(tx.kind) {
        QuantitySign::Plus => theme::UP,
        QuantitySign::Minus => theme::DOWN,
        QuantitySign::None => primary,
    };
    grid::figure(
        ui,
        cells[3],
        &COLUMNS[3],
        &formatter.quantity(tx.quantity, quantity_sign(tx.kind)),
        quantity_color,
    );
    let dash = || "\u{2014}".to_owned();
    grid::figure(
        ui,
        cells[4],
        &COLUMNS[4],
        &tx.price
            .map_or_else(dash, |price| tx.price_currency.format_price(price)),
        primary,
    );
    grid::cell_text(
        ui,
        cells[5],
        &COLUMNS[5],
        &tx.price
            .map_or_else(dash, |price| tx.price_currency.format(price * tx.quantity)),
        kit::size::BODY,
        kit::MEDIUM,
        primary,
    );
}

/// The footer's leading side: busy while importing, else whether the file is ready.
fn status(ui: &mut Ui, importing: bool, preview: &CsvPreview) {
    ui.spacing_mut().item_spacing.x = 6.0;
    if importing {
        ui.add(LoadingIndicator::new().size(14.0));
        ui.label(kit::text(
            "Validating and importing\u{2026}",
            kit::size::BODY,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        return;
    }
    for (index, (kind, label)) in footer_status(preview).iter().enumerate() {
        if index > 0 {
            ui.add_space(10.0);
        }
        status_dot(
            ui,
            kind.color(),
            label,
            if index == 0 {
                kit::MEDIUM
            } else {
                kit::REGULAR
            },
            if index == 0 {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
    }
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
            ..ImportPreviewState::default()
        };
        assert!(!can_import(&preview(vec![transaction(1, "X", 1)]), &busy));
    }

    #[test]
    fn a_failure_reopens_the_button_and_shows_why() {
        let mut state = ImportPreviewState {
            is_importing: true,
            ..ImportPreviewState::default()
        };
        state.failed("Sell exceeds holdings");
        assert!(!state.is_importing);
        assert_eq!(state.failure.as_deref(), Some("Sell exceeds holdings"));
        assert!(failure_detail("x").contains("No transactions were imported."));
    }

    #[test]
    fn the_footer_says_ready_unless_something_blocks_or_warns() {
        let mut preview = preview(vec![transaction(1, "BTCUSDT", 1)]);
        assert_eq!(
            footer_status(&preview),
            vec![(Status::Good, "Ready to import".to_owned())]
        );
        preview.warnings.push("w".into());
        assert_eq!(
            footer_status(&preview),
            vec![
                (Status::Good, "Ready to import".to_owned()),
                (Status::Warning, "1 warning".to_owned())
            ]
        );
        preview.errors.extend(["e1".to_owned(), "e2".to_owned()]);
        assert_eq!(
            footer_status(&preview)[0],
            (Status::Error, "2 errors to fix".to_owned())
        );
        assert_eq!(
            footer_status(&super::tests::preview(vec![])),
            vec![(Status::Warning, "Nothing to import".to_owned())]
        );
    }

    #[test]
    fn words_agree_with_counts() {
        assert_eq!(error_title(1), "1 problem requires attention");
        assert_eq!(error_title(2), "2 problems require attention");
        assert_eq!(warning_title(1), "1 warning");
        assert_eq!(warning_title(3), "3 warnings");
        assert_eq!(import_button_title(1), "Import 1 Transaction");
        assert_eq!(import_button_title(0), "Import 0 Transactions");
        assert_eq!(subtitle(&preview(vec![])), "Nothing to import yet.");
    }
}
