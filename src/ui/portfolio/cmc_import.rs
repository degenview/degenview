//! First step of a CoinMarketCap import: match each exported ticker to a market the app can
//! price, resolve foreign-currency fees, then hand the converted transactions to the preview
//! sheet. A pure view over `CmcImportState`; the host does the slow work (searching exchanges,
//! fetching historical FX rates, the "Match…" asset picker) and feeds results back through the
//! state's methods.

use crate::i18n::{tr, tr_n};
use crate::model::DataSource;
use crate::model::portfolio::csv::{CmcPreview, CmcRow, CsvPreview, Mint};
use crate::model::portfolio::{PortfolioAsset, PortfolioCurrency, PortfolioTransaction};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::matching::{self, RowAction, RowState, Tally, section_heading};
use crate::ui::kit::sheet;
use crate::ui::kit::sheet_parts::{problem_card, status_dot};
use crate::ui::portfolio::grid;
use crate::ui::portfolio::import_preview::warning_title;
use crate::ui::portfolio::table::paint_asset_icon;
use crate::ui::theme;
use crate::ui::{
    chart::picker_kit::{art, widgets},
    kit,
};
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use std::collections::{HashMap, HashSet};

pub const SIZE: Vec2 = Vec2::new(720.0, 680.0);

#[derive(Default)]
pub struct CmcImportState {
    /// Ticker → the market it maps to.
    pub mappings: HashMap<String, PortfolioAsset>,
    pub skipped_symbols: HashSet<String>,
    pub auto_mapped: HashSet<String>,
    /// Tickers the host is searching for right now.
    pub resolving: HashSet<String>,
    pub did_auto_map: bool,
    /// Row id → USD per unit of the row's fee currency, on the row's date.
    pub fee_fx_rates: HashMap<String, f64>,
    pub fee_fx_errors: HashMap<String, String>,
    pub is_resolving_fee_fx: bool,
    pub fee_fx_progress: f32,
    pub skipped_row_ids: HashSet<String>,
}

impl CmcImportState {
    // MARK: Counts

    pub fn mapped_rows(&self, preview: &CmcPreview) -> usize {
        preview
            .rows
            .iter()
            .filter(|row| {
                self.mappings.contains_key(&row.token) && !self.skipped_row_ids.contains(&row.id)
            })
            .count()
    }

    pub fn skipped_rows(&self, preview: &CmcPreview) -> usize {
        preview.rows.len() - self.mapped_rows(preview)
    }

    /// Rows that import and carry a fee in a currency other than USD.
    pub fn foreign_fee_rows<'a>(&self, preview: &'a CmcPreview) -> Vec<&'a CmcRow> {
        preview
            .rows
            .iter()
            .filter(|row| {
                self.mappings.contains_key(&row.token)
                    && !self.skipped_row_ids.contains(&row.id)
                    && row.fee != 0.0
                    && row.fee_currency.unwrap_or(PortfolioCurrency::Usd) != PortfolioCurrency::Usd
            })
            .collect()
    }

    pub fn can_continue(&self, preview: &CmcPreview) -> bool {
        preview.is_valid()
            && self.mapped_rows(preview) > 0
            && self.did_auto_map
            && !self.is_resolving_fee_fx
            && self
                .foreign_fee_rows(preview)
                .iter()
                .all(|row| self.fee_fx_rates.contains_key(&row.id))
    }

    // MARK: Mapping

    /// Reuses an asset the ledger already holds under this symbol, when there is exactly one.
    pub fn apply_existing_mappings(
        &mut self,
        preview: &CmcPreview,
        ledger: &[PortfolioTransaction],
    ) {
        for symbol in preview.symbols() {
            let mut assets: Vec<&PortfolioAsset> = Vec::new();
            for transaction in ledger {
                let asset = &transaction.asset;
                if asset.symbol.eq_ignore_ascii_case(&symbol)
                    && !assets.iter().any(|known| known.key == asset.key)
                {
                    assets.push(asset);
                }
            }
            if let [asset] = assets.as_slice() {
                self.mappings.insert(symbol, (*asset).clone());
            }
        }
    }

    /// Tickers still to search for: not mapped, not skipped.
    pub fn symbols_to_auto_map(&self, preview: &CmcPreview) -> Vec<String> {
        preview
            .symbols()
            .into_iter()
            .filter(|symbol| {
                !self.mappings.contains_key(symbol) && !self.skipped_symbols.contains(symbol)
            })
            .collect()
    }

    /// The host found a market for `symbol`. Ignored when the user mapped or skipped it meanwhile.
    pub fn auto_mapped(&mut self, symbol: &str, asset: PortfolioAsset) {
        if !self.mappings.contains_key(symbol) && !self.skipped_symbols.contains(symbol) {
            self.mappings.insert(symbol.to_owned(), asset);
            self.auto_mapped.insert(symbol.to_owned());
        }
    }

    /// The user picked a market in the "Match…" sheet.
    pub fn set_mapping(&mut self, symbol: &str, asset: PortfolioAsset) {
        self.mappings.insert(symbol.to_owned(), asset);
        self.skipped_symbols.remove(symbol);
        self.auto_mapped.remove(symbol);
    }

    pub fn skip(&mut self, symbol: &str) {
        self.skipped_symbols.insert(symbol.to_owned());
    }

    pub fn reset(&mut self, symbol: &str) {
        self.mappings.remove(symbol);
        self.skipped_symbols.remove(symbol);
        self.auto_mapped.remove(symbol);
    }

    // MARK: Fee rates

    /// Rows whose fee needs a historical rate.
    pub fn fee_rows_to_resolve<'a>(&self, preview: &'a CmcPreview) -> Vec<&'a CmcRow> {
        preview
            .rows
            .iter()
            .filter(|row| {
                row.fee != 0.0
                    && row.fee_currency.unwrap_or(PortfolioCurrency::Usd) != PortfolioCurrency::Usd
            })
            .collect()
    }

    pub fn fee_rate_resolved(&mut self, row_id: &str, result: Result<f64, String>) {
        match result {
            Ok(rate) => {
                self.fee_fx_rates.insert(row_id.to_owned(), rate);
                self.fee_fx_errors.remove(row_id);
            }
            Err(message) => {
                self.fee_fx_errors.insert(row_id.to_owned(), message);
            }
        }
    }

    // MARK: Result

    /// The converted transactions for the preview sheet.
    pub fn result(&self, preview: &CmcPreview, portfolio_id: u64, mint: &mut Mint) -> CsvPreview {
        preview.transactions(
            portfolio_id,
            &self.mappings,
            &self.fee_fx_rates,
            &self.skipped_row_ids,
            mint,
        )
    }
}

pub struct CmcImportInput<'a> {
    pub preview: &'a CmcPreview,
    pub portfolio_currency: PortfolioCurrency,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CmcImportAction {
    /// Open the asset picker for this ticker; answer with `CmcImportState::set_mapping`.
    Match(String),
    /// Convert and open the preview sheet (`CmcImportState::result`).
    Continue,
    Cancel,
}

// MARK: Pure text

pub fn subtitle(preview: &CmcPreview) -> String {
    format!(
        "{} · {}",
        tr_n!(
            preview.rows.len(),
            "{count} transaction",
            "{count} transactions"
        ),
        tr_n!(preview.symbols().len(), "{count} asset", "{count} assets")
    )
}

pub fn csv_error_title(count: usize) -> String {
    tr_n!(
        count,
        "{count} CSV error requires attention",
        "{count} CSV errors require attention"
    )
}

pub fn search_note(currency: PortfolioCurrency) -> String {
    tr!(
        "Finding {currency} markets — Binance, then Coinbase, CoinGecko and DEXScreener…",
        currency = currency.code()
    )
}

pub fn restore_title(count: usize) -> String {
    tr_n!(
        count,
        "Restore {count} individually skipped transaction",
        "Restore {count} individually skipped transactions"
    )
}

pub fn fee_row_title(row: &CmcRow) -> String {
    tr!(
        "Line {line} · {token} · {fee} {currency}",
        line = row.line,
        token = row.token,
        fee = row.fee,
        currency = row.fee_currency.map_or("", |currency| currency.code())
    )
}

pub fn progress_text(progress: f32) -> String {
    format!("{}%", (progress * 100.0).round() as i64)
}

/// What a ticker row says about its mapping.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenStatus {
    Resolving,
    Mapped {
        ticker: String,
        detail: String,
        auto: bool,
    },
    Skipped,
    NotMatched,
}

pub fn token_status(state: &CmcImportState, symbol: &str) -> TokenStatus {
    if state.resolving.contains(symbol) {
        return TokenStatus::Resolving;
    }
    if let Some(asset) = state.mappings.get(symbol) {
        let name = if asset.name.is_empty() {
            &asset.symbol
        } else {
            &asset.name
        };
        return TokenStatus::Mapped {
            ticker: asset.display_ticker(),
            detail: format!("{name} · {}", asset.source.display_name()),
            auto: state.auto_mapped.contains(symbol),
        };
    }
    if state.skipped_symbols.contains(symbol) {
        TokenStatus::Skipped
    } else {
        TokenStatus::NotMatched
    }
}

// MARK: Tally

pub fn tally(state: &CmcImportState, preview: &CmcPreview) -> Tally {
    let mut tally = Tally::default();
    for symbol in preview.symbols() {
        tally.total += 1;
        match token_status(state, &symbol) {
            TokenStatus::Resolving => tally.resolving += 1,
            TokenStatus::Mapped { .. } => tally.matched += 1,
            TokenStatus::Skipped => tally.skipped += 1,
            TokenStatus::NotMatched => tally.unmatched += 1,
        }
    }
    tally
}

// MARK: View

const HEADER_PADDING: f32 = 24.0;
/// Room under the scrolling body for the footer: its hairline, padding and 28 pt buttons.
const FOOTER_HEIGHT: f32 = 58.0;
const FEE_ROW_HEIGHT: f32 = 46.0;

pub fn show(
    ctx: &egui::Context,
    state: &mut CmcImportState,
    input: &CmcImportInput,
) -> Vec<CmcImportAction> {
    let mut actions = Vec::new();
    let preview = input.preview;
    sheet::show(ctx, Id::new("portfolio-cmc-import"), SIZE.x, |ui| {
        ui.set_height(SIZE.y);
        let top = ui.cursor().top();
        ui.add_space(HEADER_PADDING);
        sheet::inset(ui, |ui| {
            widgets::header(
                ui,
                art::Scene::CoinMarketCap,
                tr!("Import from CoinMarketCap"),
                &subtitle(preview),
            );
            ui.add_space(16.0);
            let tally = tally(state, preview);
            let caption = tr_n!(tally.total, "ticker matched", "tickers matched");
            matching::band(ui, &tally, &caption);
        });
        ui.add_space(16.0);
        // `set_height` made the sheet's own rect SIZE.y tall, so measure from the cursor.
        let height = SIZE.y - (ui.cursor().top() - top) - FOOTER_HEIGHT;
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                sheet::inset(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    body(ui, state, input, &mut actions);
                    ui.add_space(12.0);
                });
            });
        sheet::footer_split(
            ui,
            |ui| summary(ui, state, preview),
            |ui| {
                let enabled = state.can_continue(preview);
                if kit::footer_button(
                    ui,
                    tr!("Continue to Preview"),
                    kit::FooterButton::Primary,
                    enabled,
                    150.0,
                )
                .clicked()
                {
                    actions.push(CmcImportAction::Continue);
                }
                if kit::footer_button(ui, tr!("Cancel"), kit::FooterButton::Secondary, true, 0.0)
                    .clicked()
                {
                    actions.push(CmcImportAction::Cancel);
                }
            },
        );
    });
    if actions.is_empty() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        actions.push(CmcImportAction::Cancel);
    }
    actions
}

fn body(
    ui: &mut Ui,
    state: &mut CmcImportState,
    input: &CmcImportInput,
    actions: &mut Vec<CmcImportAction>,
) {
    let preview = input.preview;
    if !preview.errors.is_empty() {
        notice(
            ui,
            Icon::TriangleFill,
            theme::DOWN,
            &csv_error_title(preview.errors.len()),
            &preview.errors,
        );
        ui.add_space(16.0);
    }
    tokens_section(ui, state, input, actions);
    if !state.foreign_fee_rows(preview).is_empty() {
        ui.add_space(20.0);
        fee_section(ui, state, preview);
    }
    if !state.skipped_row_ids.is_empty() {
        ui.add_space(14.0);
        if grid::link(ui, &restore_title(state.skipped_row_ids.len())) {
            state.skipped_row_ids.clear();
        }
    }
    if !preview.warnings.is_empty() {
        ui.add_space(16.0);
        notice(
            ui,
            Icon::TriangleFill,
            theme::REPLAY,
            &warning_title(preview.warnings.len()),
            &preview.warnings,
        );
    }
}

/// A tinted callout listing `lines`, one bullet each, scrolling when long.
fn notice(ui: &mut Ui, icon: Icon, tint: Color32, title: &str, lines: &[String]) {
    problem_card(ui, icon, tint, title, None, lines, 96.0);
}

fn tokens_section(
    ui: &mut Ui,
    state: &mut CmcImportState,
    input: &CmcImportInput,
    actions: &mut Vec<CmcImportAction>,
) {
    let searching = (!state.did_auto_map).then(|| search_note(input.portfolio_currency));
    section_heading(
        ui,
        tr!("Match tokens to markets"),
        tr!(
            "Only transactions for matched tickers are imported. Skip a ticker to leave all of its \
             transactions out."
        ),
        searching
            .is_some()
            .then_some(tr!("Searching exchanges\u{2026}")),
    );
    if let Some(note) = searching {
        ui.add(egui::Label::new(kit::text(
            note,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
        )));
        ui.add_space(10.0);
    }
    for symbol in input.preview.symbols() {
        let count = input
            .preview
            .rows
            .iter()
            .filter(|row| row.token == symbol)
            .count();
        token_row(ui, state, &symbol, count, actions);
        ui.add_space(8.0);
    }
}

fn token_row(
    ui: &mut Ui,
    state: &mut CmcImportState,
    symbol: &str,
    count: usize,
    actions: &mut Vec<CmcImportAction>,
) {
    let status = token_status(state, symbol);
    let source = state.mappings.get(symbol).map(|asset| asset.source);
    let row_state = match &status {
        TokenStatus::Resolving => RowState::Searching(tr!("Searching exchanges\u{2026}")),
        TokenStatus::Mapped {
            ticker,
            detail,
            auto,
        } => RowState::Matched {
            // A mapped ticker has an asset, and so a source.
            source: source.unwrap_or(DataSource::Binance),
            ticker,
            detail,
            badge: auto.then(|| (tr!("Auto-matched"), theme::text_secondary())),
        },
        TokenStatus::Skipped => RowState::Skipped {
            note: tr!("Won\u{2019}t be imported"),
        },
        TokenStatus::NotMatched => RowState::Unmatched {
            note: tr!("Skipped unless you match it"),
        },
    };
    let caption = tr_n!(count, "{count} transaction", "{count} transactions");
    let clicked = matching::row(
        ui,
        Id::new(("cmc-row", symbol)),
        &matching::Row {
            name: symbol,
            caption: &caption,
            state: row_state,
            target_left: matching::TARGET_LEFT,
        },
        |ui, rect| match state.mappings.get(symbol) {
            Some(asset) => paint_asset_icon(ui, rect, asset),
            None => matching::placeholder_icon(ui, rect, symbol),
        },
    );
    match clicked {
        Some(RowAction::Match) => {
            state.skipped_symbols.remove(symbol);
            state.auto_mapped.remove(symbol);
            actions.push(CmcImportAction::Match(symbol.to_owned()));
        }
        Some(RowAction::Skip) => state.skip(symbol),
        Some(RowAction::Reset) => state.reset(symbol),
        None => {}
    }
}

fn fee_section(ui: &mut Ui, state: &mut CmcImportState, preview: &CmcPreview) {
    let rows = state.foreign_fee_rows(preview);
    let failed: Vec<(String, String, String)> = rows
        .iter()
        .filter_map(|row| {
            state
                .fee_fx_errors
                .get(&row.id)
                .map(|error| (row.id.clone(), fee_row_title(row), error.clone()))
        })
        .collect();
    let fees = rows.len();
    section_heading(
        ui,
        tr!("Fee exchange rates"),
        &tr_n!(
            fees,
            "{count} fee is in a currency other than USD needs a historical rate to be converted.",
            "{count} fees are in a currency other than USD need a historical rate to be converted."
        ),
        None,
    );
    let secondary = theme::text_secondary();
    egui::Frame::new()
        .fill(theme::quaternary(0.4))
        .stroke(Stroke::new(1.0, theme::separator().gamma_multiply(0.6)))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            if state.is_resolving_fee_fx {
                let (row, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                let label = kit::galley(
                    ui,
                    tr!("Fetching historical exchange rates\u{2026}"),
                    kit::size::SUBHEADLINE + 1.0,
                    kit::REGULAR,
                    secondary,
                    row.width() - 60.0,
                );
                ui.painter().galley(
                    pos2(row.left(), row.center().y - label.size().y / 2.0),
                    label,
                    Color32::PLACEHOLDER,
                );
                let percent = kit::galley(
                    ui,
                    &progress_text(state.fee_fx_progress),
                    kit::size::SUBHEADLINE + 1.0,
                    kit::MEDIUM,
                    theme::text_primary(),
                    60.0,
                );
                ui.painter().galley(
                    pos2(
                        row.right() - percent.size().x,
                        row.center().y - percent.size().y / 2.0,
                    ),
                    percent,
                    Color32::PLACEHOLDER,
                );
                ui.add_space(10.0);
                let (bar, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
                ui.painter().rect_filled(bar, 3.0, theme::quaternary(1.0));
                let mut fill = bar;
                fill.set_width((bar.width() * state.fee_fx_progress.clamp(0.0, 1.0)).max(6.0));
                ui.painter().rect_filled(fill, 3.0, theme::UP);
            } else if failed.is_empty() {
                let (row, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(
                        pos2(row.left() + 7.0, row.center().y),
                        Vec2::splat(13.0),
                    ),
                    Icon::Check,
                    theme::UP,
                );
                let label = kit::galley(
                    ui,
                    tr!("Historical exchange rates loaded"),
                    kit::size::SUBHEADLINE + 1.0,
                    kit::MEDIUM,
                    theme::UP,
                    row.width() - 24.0,
                );
                ui.painter().galley(
                    pos2(row.left() + 22.0, row.center().y - label.size().y / 2.0),
                    label,
                    Color32::PLACEHOLDER,
                );
            }
            for (index, (id, title, error)) in failed.into_iter().enumerate() {
                if index > 0 {
                    ui.add_space(4.0);
                }
                let (row, _) = ui.allocate_exact_size(
                    vec2(ui.available_width(), FEE_ROW_HEIGHT),
                    Sense::hover(),
                );
                let middle = row.center().y;
                let (left, clicked) = matching::bordered_button(
                    ui,
                    Id::new(("cmc-fee-skip", &id)),
                    row.right(),
                    middle,
                    tr!("Skip Item"),
                );
                if clicked {
                    state.skipped_row_ids.insert(id);
                }
                let width = left - 12.0 - row.left();
                let title = kit::galley(
                    ui,
                    &title,
                    kit::size::BODY,
                    kit::MEDIUM,
                    theme::text_primary(),
                    width,
                );
                let error = kit::galley(
                    ui,
                    &error,
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::DOWN,
                    width,
                );
                let top = middle - (title.size().y + 2.0 + error.size().y) / 2.0;
                ui.painter()
                    .galley(pos2(row.left(), top), title.clone(), Color32::PLACEHOLDER);
                ui.painter().galley(
                    pos2(row.left(), top + title.size().y + 2.0),
                    error,
                    Color32::PLACEHOLDER,
                );
            }
        });
}

/// The footer's leading side: how many transactions go in and how many stay out.
fn summary(ui: &mut Ui, state: &CmcImportState, preview: &CmcPreview) {
    ui.spacing_mut().item_spacing.x = 6.0;
    let mapped = state.mapped_rows(preview);
    status_dot(
        ui,
        if mapped > 0 {
            theme::UP
        } else {
            theme::text_tertiary()
        },
        &tr_n!(
            mapped,
            "{count} transaction to import",
            "{count} transactions to import"
        ),
        kit::MEDIUM,
        theme::text_primary(),
    );
    let skipped = state.skipped_rows(preview);
    if skipped > 0 {
        ui.add_space(10.0);
        status_dot(
            ui,
            theme::text_tertiary(),
            &tr_n!(skipped, "{count} skipped", "{count} skipped"),
            kit::REGULAR,
            theme::text_secondary(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::csv::preview_coinmarketcap;
    use crate::ui::kit::matching::{TallyKind, matched_title, tally_legend};

    const CMC: &str = "Date (UTC),Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes\n\
        2024-03-05 14:30:00,BTC,Buy,60000,0.5,30000,--,,\n\
        2024-03-06 10:00:00,ETH,Buy,3000,2,6000,1.5,EUR,\n\
        2024-03-07 10:00:00,ETH,Sell,3000,1,3000,0,,\n\
        2024-03-08 10:00:00,DOGE,Buy,0.1,100,10,0,,";

    fn asset(symbol: &str, key: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: key.into(),
            symbol: symbol.into(),
            name: format!("{symbol} coin"),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn ready() -> (CmcPreview, CmcImportState) {
        let preview = preview_coinmarketcap(CMC);
        assert!(preview.is_valid(), "{:?}", preview.errors);
        let state = CmcImportState {
            did_auto_map: true,
            ..CmcImportState::default()
        };
        (preview, state)
    }

    #[test]
    fn nothing_continues_until_something_is_mapped() {
        let (preview, mut state) = ready();
        assert!(!state.can_continue(&preview));
        state.set_mapping("BTC", asset("BTC", "Binance:BTCUSDT"));
        assert_eq!(state.mapped_rows(&preview), 1);
        assert_eq!(state.skipped_rows(&preview), 3);
        assert!(state.can_continue(&preview));
        state.did_auto_map = false;
        assert!(!state.can_continue(&preview), "still searching");
        state.did_auto_map = true;
        let mut invalid = preview.clone();
        invalid.errors.push("bad".into());
        assert!(!state.can_continue(&invalid));
    }

    #[test]
    fn a_foreign_fee_holds_the_button_until_its_rate_arrives() {
        let (preview, mut state) = ready();
        state.set_mapping("ETH", asset("ETH", "Binance:ETHUSDT"));
        let rows = state.foreign_fee_rows(&preview);
        assert_eq!(rows.len(), 1);
        let id = rows[0].id.clone();
        assert!(!state.can_continue(&preview));
        state.fee_rate_resolved(&id, Err("offline".into()));
        assert!(!state.can_continue(&preview));
        assert_eq!(state.fee_fx_errors[&id], "offline");
        state.is_resolving_fee_fx = true;
        state.fee_rate_resolved(&id, Ok(1.08));
        assert!(!state.can_continue(&preview), "still resolving others");
        assert!(state.fee_fx_errors.is_empty(), "a rate clears the error");
        state.is_resolving_fee_fx = false;
        assert!(state.can_continue(&preview));
        // Skipping the item instead of fetching its rate also unblocks.
        state.fee_fx_rates.clear();
        state.skipped_row_ids.insert(id);
        assert!(state.can_continue(&preview));
        assert_eq!(
            state.fee_rows_to_resolve(&preview).len(),
            1,
            "all foreign-fee rows get a lookup"
        );
        assert_eq!(
            fee_row_title(state.fee_rows_to_resolve(&preview)[0]),
            "Line 3 · ETH · 1.5 EUR"
        );
    }

    #[test]
    fn mapping_actions_keep_the_sets_consistent() {
        let (_, mut state) = ready();
        state.auto_mapped("DOGE", asset("DOGE", "Binance:DOGEUSDT"));
        assert!(state.auto_mapped.contains("DOGE"));
        state.skip("BTC");
        state.auto_mapped("BTC", asset("BTC", "Binance:BTCUSDT"));
        assert!(
            !state.mappings.contains_key("BTC"),
            "the user's skip wins over a late search result"
        );
        state.set_mapping("DOGE", asset("DOGE", "CoinGecko:dogecoin"));
        assert!(
            !state.auto_mapped.contains("DOGE"),
            "a manual pick is no longer auto"
        );
        state.set_mapping("BTC", asset("BTC", "Binance:BTCUSDT"));
        assert!(
            !state.skipped_symbols.contains("BTC"),
            "matching clears a skip"
        );
        state.reset("BTC");
        assert!(!state.mappings.contains_key("BTC") && state.skipped_symbols.is_empty());
        state.auto_mapped("DOGE", asset("DOGE", "x"));
        assert_eq!(
            state.mappings["DOGE"].key, "CoinGecko:dogecoin",
            "an existing mapping is not replaced"
        );
    }

    #[test]
    fn tickers_the_ledger_already_holds_map_themselves() {
        let (preview, mut state) = ready();
        let held =
            |key: &str, symbol: &str| {
                let mut transaction = crate::model::portfolio::csv::preview(
                &format!(
                    "{}\nMain,{key},{symbol},,Binance,Buy,1,1,USD,0,USD,2024-01-01T00:00:00Z,,x",
                    crate::model::portfolio::csv::HEADER
                ),
                &[crate::model::portfolio::Portfolio::new(1, "Main", 0)],
                &mut Mint { now_ms: 0, next_id: &mut || 1 },
            )
            .transactions;
                transaction.remove(0)
            };
        let ledger = vec![
            held("Binance:BTCUSDT", "btc"),
            held("Binance:BTCUSDT", "BTC"),
            held("Binance:ETHUSDT", "ETH"),
            held("Coinbase:ETH-USD", "ETH"),
        ];
        state.apply_existing_mappings(&preview, &ledger);
        assert_eq!(
            state.mappings["BTC"].key, "Binance:BTCUSDT",
            "one asset under the symbol"
        );
        assert!(
            !state.mappings.contains_key("ETH"),
            "two candidates: the user decides"
        );
        assert_eq!(state.symbols_to_auto_map(&preview), ["DOGE", "ETH"]);
    }

    #[test]
    fn rows_convert_through_the_preview_with_the_chosen_mappings() {
        let (preview, mut state) = ready();
        state.set_mapping("BTC", asset("BTC", "Binance:BTCUSDT"));
        let mut next = 10;
        let mut ids = || {
            next += 1;
            next
        };
        let result = state.result(
            &preview,
            4,
            &mut Mint {
                now_ms: 7,
                next_id: &mut ids,
            },
        );
        assert!(result.is_valid());
        assert_eq!(result.transactions.len(), 1);
        assert_eq!(result.transactions[0].portfolio_id, 4);
        assert_eq!(result.transactions[0].id, 11);
    }

    #[test]
    fn rows_read_in_words() {
        let (preview, mut state) = ready();
        assert_eq!(subtitle(&preview), "4 transactions · 3 assets");
        assert_eq!(csv_error_title(1), "1 CSV error requires attention");
        assert_eq!(csv_error_title(2), "2 CSV errors require attention");
        assert_eq!(
            restore_title(1),
            "Restore 1 individually skipped transaction"
        );
        assert_eq!(progress_text(0.456), "46%");
        assert_eq!(
            search_note(PortfolioCurrency::Eur),
            "Finding EUR markets — Binance, then Coinbase, CoinGecko and DEXScreener…"
        );
        assert_eq!(token_status(&state, "BTC"), TokenStatus::NotMatched);
        state.skip("BTC");
        assert_eq!(token_status(&state, "BTC"), TokenStatus::Skipped);
        state.resolving.insert("ETH".into());
        assert_eq!(token_status(&state, "ETH"), TokenStatus::Resolving);
        state.auto_mapped("DOGE", asset("DOGE", "Binance:DOGEUSDT"));
        assert_eq!(
            token_status(&state, "DOGE"),
            TokenStatus::Mapped {
                ticker: "DOGE".into(),
                detail: "DOGE coin · Binance".into(),
                auto: true
            }
        );
    }

    #[test]
    fn the_tally_counts_each_ticker_once_by_how_it_stands() {
        let (preview, mut state) = ready();
        let tally_of = |state: &CmcImportState| tally(state, &preview);
        assert_eq!(
            tally_of(&state),
            Tally {
                total: 3,
                unmatched: 3,
                ..Tally::default()
            }
        );
        state.set_mapping("BTC", asset("BTC", "Binance:BTCUSDT"));
        state.skip("DOGE");
        state.resolving.insert("ETH".into());
        let now = tally_of(&state);
        assert_eq!(
            now,
            Tally {
                total: 3,
                matched: 1,
                skipped: 1,
                resolving: 1,
                unmatched: 0
            }
        );
        assert_eq!(matched_title(&now), "1 of 3");
        assert_eq!(
            tally_legend(&now),
            vec![
                (TallyKind::Matched, "1 matched".to_owned()),
                (TallyKind::Searching, "1 searching".to_owned()),
                (TallyKind::Skipped, "1 skipped".to_owned()),
            ],
            "empty groups are left out"
        );
    }
}
