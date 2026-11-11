//! First step of a CoinMarketCap import: match each exported ticker to a market the app can
//! price, resolve foreign-currency fees, then hand the converted transactions to the preview
//! sheet. A pure view over `CmcImportState`; the host does the slow work (searching exchanges,
//! fetching historical FX rates, the "Match…" asset picker) and feeds results back through the
//! state's methods.

// Transient: the portfolio host opens this once the importers are wired.
#![allow(dead_code)]

use crate::model::portfolio::csv::{CmcPreview, CmcRow, CsvPreview, Mint};
use crate::model::portfolio::{PortfolioAsset, PortfolioCurrency, PortfolioTransaction};
use crate::ui::icons::{self, Icon};
use crate::ui::portfolio_grid;
use crate::ui::portfolio_import_preview::{plural, warning_title};
use crate::ui::portfolio_sheet_parts::{card_background, pill, problem_card, status_dot};
use crate::ui::portfolio_table::paint_asset_icon;
use crate::ui::sheet;
use crate::ui::theme;
use crate::ui::{add_chart_art, add_chart_sheet, kit, source_logo};
use eframe::egui::{
    self, Color32, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
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
        plural(preview.rows.len(), "transaction", "transactions"),
        plural(preview.symbols().len(), "asset", "assets")
    )
}

pub fn csv_error_title(count: usize) -> String {
    format!(
        "{count} CSV {}",
        if count == 1 {
            "error requires attention"
        } else {
            "errors require attention"
        }
    )
}

pub fn search_note(currency: PortfolioCurrency) -> String {
    format!(
        "Finding {} markets — Binance, then Coinbase, CoinGecko and DEXScreener…",
        currency.code()
    )
}

pub fn restore_title(count: usize) -> String {
    format!(
        "Restore {count} individually skipped {}",
        if count == 1 {
            "transaction"
        } else {
            "transactions"
        }
    )
}

pub fn fee_row_title(row: &CmcRow) -> String {
    format!(
        "Line {} · {} · {} {}",
        row.line,
        row.token,
        row.fee,
        row.fee_currency.map_or("", |currency| currency.code())
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

/// How the file's tickers stand: the one summary the progress band draws.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Tally {
    pub total: usize,
    /// Matched to a market.
    pub matched: usize,
    /// Left out on purpose.
    pub skipped: usize,
    /// Still being searched for.
    pub resolving: usize,
    /// Neither matched nor skipped: left out unless the user matches them.
    pub unmatched: usize,
}

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

/// The big figure of the progress band: how many of the tickers have a market.
pub fn matched_title(tally: &Tally) -> String {
    format!("{} of {}", tally.matched, tally.total)
}

/// The legend under the progress bar; only the groups that have tickers.
pub fn tally_legend(tally: &Tally) -> Vec<(TallyKind, String)> {
    [
        (TallyKind::Matched, tally.matched, "matched"),
        (TallyKind::Searching, tally.resolving, "searching"),
        (TallyKind::Unmatched, tally.unmatched, "to match"),
        (TallyKind::Skipped, tally.skipped, "skipped"),
    ]
    .into_iter()
    .filter(|(_, count, _)| *count > 0)
    .map(|(kind, count, word)| (kind, format!("{count} {word}")))
    .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TallyKind {
    Matched,
    Searching,
    Unmatched,
    Skipped,
}

impl TallyKind {
    fn color(self) -> Color32 {
        match self {
            Self::Matched => theme::UP,
            Self::Searching => theme::text_secondary(),
            Self::Unmatched => theme::REPLAY,
            Self::Skipped => theme::text_tertiary(),
        }
    }
}

// MARK: View

const HEADER_PADDING: f32 = 24.0;
/// Room under the scrolling body for the footer: its hairline, padding and 28 pt buttons.
const FOOTER_HEIGHT: f32 = 58.0;
const BAND_HEIGHT: f32 = 76.0;
const ROW_HEIGHT: f32 = 60.0;
const FEE_ROW_HEIGHT: f32 = 46.0;
/// Where a ticker row's market starts, from its left edge.
const TARGET_LEFT: f32 = 196.0;

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
            add_chart_sheet::header(
                ui,
                add_chart_art::Scene::CoinMarketCap,
                "Import from CoinMarketCap",
                &subtitle(preview),
            );
            ui.add_space(16.0);
            progress_band(ui, &tally(state, preview));
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
                    "Continue to Preview",
                    kit::FooterButton::Primary,
                    enabled,
                    150.0,
                )
                .clicked()
                {
                    actions.push(CmcImportAction::Continue);
                }
                if kit::footer_button(ui, "Cancel", kit::FooterButton::Secondary, true, 0.0)
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

/// How many tickers have a market, as a figure and one bar split by state, with its legend.
fn progress_band(ui: &mut Ui, tally: &Tally) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAND_HEIGHT), Sense::hover());
    card_background(ui, rect, None);
    let inner = rect.shrink2(vec2(18.0, 14.0));
    let primary = theme::text_primary();
    let secondary = theme::text_secondary();

    let figure = kit::galley(
        ui,
        &matched_title(tally),
        22.0,
        kit::SEMIBOLD,
        primary,
        140.0,
    );
    let caption = kit::galley(
        ui,
        if tally.total == 1 {
            "ticker matched"
        } else {
            "tickers matched"
        },
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        secondary,
        140.0,
    );
    let block = figure.size().y + 1.0 + caption.size().y;
    let top = rect.center().y - block / 2.0;
    let figure_width = figure.size().x.max(caption.size().x);
    ui.painter().galley(
        pos2(inner.left(), top),
        figure.clone(),
        Color32::PLACEHOLDER,
    );
    ui.painter().galley(
        pos2(inner.left(), top + figure.size().y + 1.0),
        caption,
        Color32::PLACEHOLDER,
    );

    // The bar and its legend, to the right of a hairline.
    let left = inner.left() + figure_width.max(96.0) + 22.0;
    ui.painter().vline(
        left - 12.0,
        rect.top() + 16.0..=rect.bottom() - 16.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
    );
    let bar = Rect::from_min_size(
        pos2(left, inner.top() + 8.0),
        vec2(inner.right() - left, 8.0),
    );
    ui.painter().rect_filled(bar, 4.0, theme::quaternary(1.0));
    if tally.total > 0 {
        let mut x = bar.left();
        let segments = [
            (TallyKind::Matched, tally.matched),
            (TallyKind::Searching, tally.resolving),
            (TallyKind::Unmatched, tally.unmatched),
            (TallyKind::Skipped, tally.skipped),
        ];
        let visible: Vec<_> = segments.iter().filter(|(_, count)| *count > 0).collect();
        let gap = 2.0;
        let usable = bar.width() - gap * visible.len().saturating_sub(1) as f32;
        for (kind, count) in visible {
            let width = usable * *count as f32 / tally.total as f32;
            let piece = Rect::from_min_size(pos2(x, bar.top()), vec2(width, bar.height()));
            ui.painter().rect_filled(piece, 4.0, kind.color());
            x += width + gap;
        }
    }
    let mut x = left;
    let legend_y = bar.bottom() + 14.0;
    for (kind, label) in tally_legend(tally) {
        ui.painter()
            .circle_filled(pos2(x + 4.0, legend_y), 3.5, kind.color());
        let text = kit::galley(
            ui,
            &label,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            secondary,
            200.0,
        );
        ui.painter().galley(
            pos2(x + 14.0, legend_y - text.size().y / 2.0),
            text.clone(),
            Color32::PLACEHOLDER,
        );
        x += 14.0 + text.size().x + 18.0;
    }
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
        if portfolio_grid::link(ui, &restore_title(state.skipped_row_ids.len())) {
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

/// A section's title with an optional status at its trailing edge, and a line of explanation.
fn section_heading(ui: &mut Ui, title: &str, detail: &str, busy: Option<&str>) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let heading = kit::galley(
        ui,
        title,
        kit::size::HEADLINE + 2.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        row.width(),
    );
    ui.painter().galley(
        pos2(row.left(), row.center().y - heading.size().y / 2.0),
        heading,
        Color32::PLACEHOLDER,
    );
    if let Some(busy) = busy {
        let label = kit::galley(
            ui,
            busy,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
            300.0,
        );
        let left = row.right() - label.size().x;
        ui.painter().galley(
            pos2(left, row.center().y - label.size().y / 2.0),
            label,
            Color32::PLACEHOLDER,
        );
        egui::Spinner::new().size(13.0).paint_at(
            ui,
            Rect::from_center_size(pos2(left - 14.0, row.center().y), Vec2::splat(13.0)),
        );
    }
    ui.add_space(3.0);
    ui.add(
        egui::Label::new(kit::text(
            detail,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
    ui.add_space(12.0);
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
        "Match tokens to markets",
        "Only transactions for matched tickers are imported. Skip a ticker to leave all of its \
         transactions out.",
        searching.is_some().then_some("Searching exchanges\u{2026}"),
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

#[derive(Clone, Copy, PartialEq)]
enum ButtonStyle {
    /// The step that moves the row forward.
    Prominent,
    Bordered,
    /// A quiet text button.
    Quiet,
}

/// A 26 pt button ending at `right`, centred on `middle`; returns its left edge and whether it
/// was clicked. Painted and hit-tested by hand so a row stays one fixed-height block.
fn row_button(
    ui: &Ui,
    id: Id,
    right: f32,
    middle: f32,
    label: &str,
    style: ButtonStyle,
) -> (f32, bool) {
    let color = match style {
        // The filled button is the inverse of the sheet: light on dark, dark on light.
        ButtonStyle::Prominent => theme::chrome(),
        ButtonStyle::Bordered => theme::text_primary(),
        ButtonStyle::Quiet => theme::text_secondary(),
    };
    let text = kit::galley(
        ui,
        label,
        kit::size::SUBHEADLINE + 1.0,
        kit::MEDIUM,
        color,
        200.0,
    );
    let rect = Rect::from_min_max(
        pos2(right - text.size().x - 24.0, middle - 13.0),
        pos2(right, middle + 13.0),
    );
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let hovered = response.hovered();
    match style {
        ButtonStyle::Prominent => {
            let fill = theme::text_primary();
            ui.painter().rect_filled(
                rect,
                7.0,
                if hovered {
                    fill.gamma_multiply(0.85)
                } else {
                    fill
                },
            );
        }
        ButtonStyle::Bordered => {
            ui.painter().rect_filled(
                rect,
                7.0,
                theme::quaternary(if hovered { 0.9 } else { 0.6 }),
            );
            ui.painter().rect_stroke(
                rect,
                7.0,
                Stroke::new(1.0, theme::separator()),
                StrokeKind::Inside,
            );
        }
        ButtonStyle::Quiet => {
            if hovered {
                ui.painter().rect_filled(rect, 7.0, theme::quaternary(0.6));
            }
        }
    }
    let color = if style == ButtonStyle::Quiet && hovered {
        theme::text_primary()
    } else {
        color
    };
    ui.painter().galley_with_override_text_color(
        pos2(
            rect.center().x - text.size().x / 2.0,
            rect.center().y - text.size().y / 2.0,
        ),
        text,
        color,
    );
    (rect.left(), response.clicked())
}

/// The ticker's initials on a neutral disc, for a ticker with no market yet.
fn placeholder_icon(ui: &Ui, rect: Rect, symbol: &str) {
    ui.painter()
        .circle_filled(rect.center(), rect.width() / 2.0, theme::quaternary(1.0));
    let initials: String = symbol.chars().take(2).collect();
    let text = kit::galley(
        ui,
        &initials.to_uppercase(),
        13.0,
        kit::SEMIBOLD,
        theme::text_secondary(),
        rect.width(),
    );
    ui.painter().galley(
        rect.center() - text.size() / 2.0,
        text,
        Color32::PLACEHOLDER,
    );
}

fn token_row(
    ui: &mut Ui,
    state: &mut CmcImportState,
    symbol: &str,
    count: usize,
    actions: &mut Vec<CmcImportAction>,
) {
    let status = token_status(state, symbol);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let outline = (status == TokenStatus::NotMatched).then(|| theme::REPLAY.gamma_multiply(0.45));
    card_background(ui, rect, outline);
    let middle = rect.center().y;
    let primary = theme::text_primary();
    let secondary = theme::text_secondary();

    // The ticker as the file spells it, with how many transactions it carries.
    let icon = Rect::from_center_size(pos2(rect.left() + 32.0, middle), Vec2::splat(36.0));
    match state.mappings.get(symbol) {
        Some(asset) => paint_asset_icon(ui, icon, asset),
        None => placeholder_icon(ui, icon, symbol),
    }
    let name = kit::galley(
        ui,
        symbol,
        kit::size::BODY + 1.0,
        kit::SEMIBOLD,
        primary,
        100.0,
    );
    let caption = kit::galley(
        ui,
        &plural(count, "transaction", "transactions"),
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        secondary,
        120.0,
    );
    let stack = name.size().y + 2.0 + caption.size().y;
    let top = middle - stack / 2.0;
    let text_left = icon.right() + 12.0;
    ui.painter()
        .galley(pos2(text_left, top), name.clone(), Color32::PLACEHOLDER);
    ui.painter().galley(
        pos2(text_left, top + name.size().y + 2.0),
        caption,
        Color32::PLACEHOLDER,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.left() + TARGET_LEFT - 22.0, middle),
            Vec2::splat(11.0),
        ),
        Icon::ChevronRight,
        theme::text_tertiary(),
    );

    // The actions, from the trailing edge in.
    let mut right = rect.right() - 12.0;
    let button = |ui: &Ui, name: &str, right: f32, label: &str, style| {
        row_button(
            ui,
            Id::new(("cmc-row", symbol, name)),
            right,
            middle,
            label,
            style,
        )
    };
    let mut clicked_match = false;
    let mut skip = false;
    let mut reset = false;
    match &status {
        TokenStatus::Resolving => {}
        TokenStatus::Mapped { .. } => {
            let (left, clicked) =
                button(ui, "change", right, "Change\u{2026}", ButtonStyle::Bordered);
            clicked_match = clicked;
            let (left, clicked) = button(ui, "reset", left - 6.0, "Reset", ButtonStyle::Quiet);
            reset = clicked;
            right = left;
        }
        TokenStatus::Skipped => {
            let (left, clicked) =
                button(ui, "match", right, "Match\u{2026}", ButtonStyle::Bordered);
            clicked_match = clicked;
            let (left, clicked) = button(ui, "reset", left - 6.0, "Reset", ButtonStyle::Quiet);
            reset = clicked;
            right = left;
        }
        TokenStatus::NotMatched => {
            let (left, clicked) =
                button(ui, "match", right, "Match\u{2026}", ButtonStyle::Prominent);
            clicked_match = clicked;
            let (left, clicked) = button(ui, "skip", left - 6.0, "Skip", ButtonStyle::Quiet);
            skip = clicked;
            right = left;
        }
    }

    // What the ticker maps to, between the arrow and the actions.
    let target = rect.left() + TARGET_LEFT;
    let room = (right - 14.0 - target).max(60.0);
    match &status {
        TokenStatus::Resolving => {
            egui::Spinner::new().size(14.0).paint_at(
                ui,
                Rect::from_center_size(pos2(target + 7.0, middle), Vec2::splat(14.0)),
            );
            let label = kit::galley(
                ui,
                "Searching exchanges\u{2026}",
                kit::size::BODY,
                kit::REGULAR,
                secondary,
                room - 24.0,
            );
            ui.painter().galley(
                pos2(target + 24.0, middle - label.size().y / 2.0),
                label,
                Color32::PLACEHOLDER,
            );
        }
        TokenStatus::Mapped {
            ticker,
            detail,
            auto,
        } => {
            let logo = Rect::from_center_size(pos2(target + 11.0, middle), Vec2::splat(22.0));
            if let Some(asset) = state.mappings.get(symbol) {
                source_logo::paint(ui, asset.source, logo);
            }
            let ticker = kit::galley(
                ui,
                ticker,
                kit::size::BODY,
                kit::MEDIUM,
                primary,
                room - 34.0,
            );
            let line = kit::galley(
                ui,
                detail,
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                secondary,
                room - 34.0,
            );
            let stack = ticker.size().y + 2.0 + line.size().y;
            let top = middle - stack / 2.0;
            let ticker_right = target + 34.0 + ticker.size().x;
            ui.painter().galley(
                pos2(target + 34.0, top),
                ticker.clone(),
                Color32::PLACEHOLDER,
            );
            ui.painter().galley(
                pos2(target + 34.0, top + ticker.size().y + 2.0),
                line,
                Color32::PLACEHOLDER,
            );
            if *auto {
                pill(
                    ui,
                    ticker_right + 8.0,
                    top + ticker.size().y / 2.0,
                    "Auto-matched",
                    theme::text_secondary(),
                );
            }
        }
        TokenStatus::Skipped => {
            let end = pill(ui, target, middle - 9.0, "Skipped", theme::text_tertiary());
            let _ = end;
            let line = kit::galley(
                ui,
                "Won\u{2019}t be imported",
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                secondary,
                room,
            );
            ui.painter().galley(
                pos2(target, middle + 9.0 - line.size().y / 2.0),
                line,
                Color32::PLACEHOLDER,
            );
        }
        TokenStatus::NotMatched => {
            pill(ui, target, middle - 9.0, "Not matched", theme::REPLAY);
            let line = kit::galley(
                ui,
                "Skipped unless you match it",
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                secondary,
                room,
            );
            ui.painter().galley(
                pos2(target, middle + 9.0 - line.size().y / 2.0),
                line,
                Color32::PLACEHOLDER,
            );
        }
    }

    if clicked_match {
        state.skipped_symbols.remove(symbol);
        state.auto_mapped.remove(symbol);
        actions.push(CmcImportAction::Match(symbol.to_owned()));
    }
    if skip {
        state.skip(symbol);
    }
    if reset {
        state.reset(symbol);
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
        "Fee exchange rates",
        &format!(
            "{} in a currency other than USD {} a historical rate to be converted.",
            plural(fees, "fee is", "fees are"),
            if fees == 1 { "needs" } else { "need" }
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
                    "Fetching historical exchange rates\u{2026}",
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
                    "Historical exchange rates loaded",
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
                let (left, clicked) = row_button(
                    ui,
                    Id::new(("cmc-fee-skip", &id)),
                    row.right(),
                    middle,
                    "Skip Item",
                    ButtonStyle::Bordered,
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
        &format!(
            "{} to import",
            plural(mapped, "transaction", "transactions")
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
            &format!("{skipped} skipped"),
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
