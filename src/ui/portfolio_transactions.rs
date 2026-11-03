//! Every transaction in the selected portfolio, searchable, filterable and sortable.
//!
//! Rows come from the unprojected ledger (so Edit opens exactly what was entered); the price
//! and fee shown are the reporting-currency projection of the same transaction.

// Transient: the Portfolio window hosts this once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioAsset, PortfolioTransaction, PortfolioTransactionType};
use crate::model::portfolio_format::{PortfolioValueFormatter, QuantitySign};
use crate::ui::accessibility;
use crate::ui::alert_center::clock_time;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::portfolio_charts;
use crate::ui::portfolio_empty;
use crate::ui::portfolio_grid::{
    self, Col, INSET, MENU_RESERVE, Navigation, RowContext, TableSpec, put,
};
use crate::ui::portfolio_overview::{activity_caption, lift};
use crate::ui::portfolio_table as table;
use crate::ui::replay_bar::date_text;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionColumn {
    Date,
    Asset,
    Type,
    Quantity,
    Price,
    Total,
    Fee,
    Notes,
}

impl TransactionColumn {
    pub const ALL: [Self; 8] = [
        Self::Date,
        Self::Asset,
        Self::Type,
        Self::Quantity,
        Self::Price,
        Self::Total,
        Self::Fee,
        Self::Notes,
    ];

    /// Money columns name the reporting currency.
    pub fn title(self, currency_code: &str) -> String {
        match self {
            Self::Date => "Date".to_owned(),
            Self::Asset => "Asset".to_owned(),
            Self::Type => "Type".to_owned(),
            Self::Quantity => "Quantity".to_owned(),
            Self::Price => format!("Price ({currency_code})"),
            Self::Total => format!("Total ({currency_code})"),
            Self::Fee => "Fee".to_owned(),
            Self::Notes => "Notes".to_owned(),
        }
    }

    /// The fee column does not sort, as in the Swift table.
    pub fn is_sortable(self) -> bool {
        self != Self::Fee
    }

    fn is_numeric(self) -> bool {
        matches!(self, Self::Quantity | Self::Price | Self::Total | Self::Fee)
    }

    /// A first click sorts text A to Z and everything else (dates, figures) newest/largest first.
    pub fn default_ascending(self) -> bool {
        matches!(self, Self::Asset | Self::Type | Self::Notes)
    }
}

/// The reporting-currency view of one transaction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    pub price: Option<f64>,
    pub fee: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransactionRow {
    pub transaction: PortfolioTransaction,
    pub price: Option<f64>,
    pub fee: f64,
}

impl TransactionRow {
    pub fn ticker(&self) -> String {
        self.transaction.asset.display_ticker()
    }

    pub fn total(&self) -> Option<f64> {
        self.price.map(|price| price * self.transaction.quantity)
    }

    pub fn type_name(&self) -> &'static str {
        type_name(self.transaction.kind)
    }

    pub fn source_name(&self) -> &'static str {
        use crate::model::portfolio::PortfolioSource::*;
        match self.transaction.source {
            Manual => "Manual",
            Wallet => "Wallet",
            Exchange => "Exchange",
            Csv => "CSV",
            CoinMarketCap => "CoinMarketCap",
        }
    }
}

pub fn type_name(kind: PortfolioTransactionType) -> &'static str {
    use PortfolioTransactionType::*;
    match kind {
        Buy => "Buy",
        Sell => "Sell",
        TransferIn => "Transfer In",
        TransferOut => "Transfer Out",
        Reward => "Reward",
        StakingReward => "Staking Reward",
        Airdrop => "Airdrop",
        Mining => "Mining",
        Interest => "Interest",
        Fee => "Fee",
        Adjustment => "Adjustment",
    }
}

pub const ALL_TYPES: [PortfolioTransactionType; 11] = {
    use PortfolioTransactionType::*;
    [
        Buy,
        Sell,
        TransferIn,
        TransferOut,
        Reward,
        StakingReward,
        Airdrop,
        Mining,
        Interest,
        Fee,
        Adjustment,
    ]
};

/// Buys and sells read at a glance in a long table.
pub fn type_tint(kind: PortfolioTransactionType) -> Color32 {
    use PortfolioTransactionType::*;
    match kind {
        Buy => theme::UP,
        Sell => theme::DOWN,
        TransferIn | TransferOut => Color32::from_rgb(10, 132, 255),
        Reward | StakingReward | Airdrop | Mining | Interest => Color32::from_rgb(191, 90, 242),
        Fee => theme::REPLAY,
        Adjustment => Color32::from_gray(150),
    }
}

/// The sign shown on a quantity: inflows "+", outflows "−".
pub fn quantity_sign(kind: PortfolioTransactionType) -> QuantitySign {
    if kind.adds_quantity() {
        QuantitySign::Plus
    } else if kind.removes_quantity() {
        QuantitySign::Minus
    } else {
        QuantitySign::None
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filters {
    pub search: String,
    pub kind: Option<PortfolioTransactionType>,
    pub asset_key: Option<String>,
}

impl Filters {
    pub fn is_active(&self) -> bool {
        !self.search.trim().is_empty() || self.kind.is_some() || self.asset_key.is_some()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Rows matching the filters, sorted by `column`.
pub fn rows(
    ledger: &[PortfolioTransaction],
    projection: &dyn Fn(u64) -> Option<Projection>,
    subtitle: &dyn Fn(&PortfolioAsset) -> String,
    filters: &Filters,
    column: TransactionColumn,
    ascending: bool,
) -> Vec<TransactionRow> {
    let needle = filters.search.trim().to_lowercase();
    let mut rows: Vec<TransactionRow> = ledger
        .iter()
        .filter(|tx| filters.kind.is_none_or(|kind| tx.kind == kind))
        .filter(|tx| {
            filters
                .asset_key
                .as_ref()
                .is_none_or(|key| &tx.asset.key == key)
        })
        .filter(|tx| {
            needle.is_empty()
                || [
                    tx.asset.display_ticker(),
                    tx.asset.symbol.clone(),
                    tx.asset.name.clone(),
                    subtitle(&tx.asset),
                    tx.notes.clone(),
                    source_label(tx),
                    type_name(tx.kind).to_owned(),
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&needle))
        })
        .map(|tx| {
            let projected = projection(tx.id);
            TransactionRow {
                transaction: tx.clone(),
                price: projected.and_then(|p| p.price),
                fee: projected.map_or(0.0, |p| p.fee),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        let order = compare(a, b, column);
        if ascending { order } else { order.reverse() }
    });
    rows
}

fn source_label(tx: &PortfolioTransaction) -> String {
    TransactionRow {
        transaction: tx.clone(),
        price: None,
        fee: 0.0,
    }
    .source_name()
    .to_owned()
}

fn compare(a: &TransactionRow, b: &TransactionRow, column: TransactionColumn) -> Ordering {
    let (x, y) = (&a.transaction, &b.transaction);
    match column {
        TransactionColumn::Date => x.timestamp.cmp(&y.timestamp),
        TransactionColumn::Asset => a.ticker().to_lowercase().cmp(&b.ticker().to_lowercase()),
        TransactionColumn::Type => a.type_name().cmp(b.type_name()),
        TransactionColumn::Quantity => x.quantity.total_cmp(&y.quantity),
        // An unpriced transaction sorts as zero, as in the Swift table.
        TransactionColumn::Price => a.price.unwrap_or(0.0).total_cmp(&b.price.unwrap_or(0.0)),
        TransactionColumn::Total => a
            .total()
            .unwrap_or(0.0)
            .total_cmp(&b.total().unwrap_or(0.0)),
        TransactionColumn::Fee => a.fee.total_cmp(&b.fee),
        TransactionColumn::Notes => x.notes.to_lowercase().cmp(&y.notes.to_lowercase()),
    }
}

/// The assets that appear in the ledger, once each, A to Z: `(key, ticker)`.
pub fn asset_choices(ledger: &[PortfolioTransaction]) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut choices: Vec<(String, String)> = ledger
        .iter()
        .filter(|tx| seen.insert(tx.asset.key.clone()))
        .map(|tx| (tx.asset.key.clone(), tx.asset.display_ticker()))
        .collect();
    choices.sort_by_key(|(_, ticker)| ticker.to_lowercase());
    choices
}

/// "Oct 9, 2026 14:03" in the viewer's time zone.
pub fn date_time_text(ms: i64, utc_offset_secs: i64) -> String {
    format!(
        "{} {}",
        date_text(ms + utc_offset_secs * 1_000),
        clock_time(ms, utc_offset_secs)
    )
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransactionAction {
    Add,
    Edit(u64),
    Duplicate(u64),
    Delete(u64),
}

pub struct TransactionsInput<'a> {
    pub ledger: &'a [PortfolioTransaction],
    pub projection: &'a dyn Fn(u64) -> Option<Projection>,
    pub subtitle: &'a dyn Fn(&PortfolioAsset) -> String,
    pub formatter: PortfolioValueFormatter,
    pub utc_offset_secs: i64,
}

pub struct TransactionsState {
    pub filters: Filters,
    column: TransactionColumn,
    ascending: bool,
    selected: Option<u64>,
    navigation: Navigation,
    /// The page shown, from zero.
    page: usize,
    /// Rows per page; one of [`PAGE_SIZES`].
    page_size: usize,
    /// The filters and sort the page was last chosen under; a change of either returns to page one.
    view: Option<(Filters, TransactionColumn, bool)>,
}

/// The page sizes the footer offers.
pub const PAGE_SIZES: [usize; 5] = [25, 50, 100, 250, 500];
/// Rows per page unless the user picks another.
pub const DEFAULT_PAGE_SIZE: usize = 100;

/// How many pages `total` rows make at `size` per page; one even when there are no rows.
pub fn page_count(total: usize, size: usize) -> usize {
    total.div_ceil(size.max(1)).max(1)
}

/// The rows of `page` (from zero) out of `total`, the page clamped to the last.
pub fn page_range(total: usize, size: usize, page: usize) -> std::ops::Range<usize> {
    let size = size.max(1);
    let page = page.min(page_count(total, size) - 1);
    (page * size).min(total)..((page + 1) * size).min(total)
}

impl Default for TransactionsState {
    fn default() -> Self {
        Self {
            filters: Filters::default(),
            column: TransactionColumn::Date,
            ascending: false,
            selected: None,
            navigation: Navigation::default(),
            page: 0,
            page_size: DEFAULT_PAGE_SIZE,
            view: None,
        }
    }
}

impl TransactionsState {
    pub fn column(&self) -> TransactionColumn {
        self.column
    }

    pub fn is_ascending(&self) -> bool {
        self.ascending
    }

    pub fn click(&mut self, column: TransactionColumn) {
        if !column.is_sortable() {
            return;
        }
        if self.column == column {
            self.ascending = !self.ascending;
        } else {
            self.column = column;
            self.ascending = column.default_ascending();
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut TransactionsState,
    input: &TransactionsInput,
) -> Vec<TransactionAction> {
    let mut actions = Vec::new();
    if !input.ledger.is_empty() {
        ui.add_space(12.0);
        summary_band(ui, input);
    }
    toolbar(ui, state, input, &mut actions);
    let shown = rows(
        input.ledger,
        input.projection,
        input.subtitle,
        &state.filters,
        state.column,
        state.ascending,
    );
    let view = (state.filters.clone(), state.column, state.ascending);
    if state.view.as_ref() != Some(&view) {
        state.page = 0;
        state.view = Some(view);
    }
    let pages = page_count(shown.len(), state.page_size);
    state.page = state.page.min(pages - 1);
    let on_page = page_range(shown.len(), state.page_size, state.page);
    let outer = portfolio_grid::allocate_table(ui);
    portfolio_grid::paint_page_frame(ui, outer);
    if shown.is_empty() {
        if state.filters.is_active() {
            portfolio_charts::empty_message(
                ui,
                portfolio_grid::body_rect(outer),
                Icon::Magnifier,
                "No Matching Transactions",
                "Nothing matches the current search and filters.",
            );
            let button = Rect::from_center_size(
                pos2(
                    outer.center().x,
                    portfolio_grid::body_rect(outer).center().y + 62.0,
                ),
                vec2(110.0, 24.0),
            );
            if ui
                .put(button, |ui: &mut egui::Ui| {
                    kit::push_button(ui, None, "Clear Filters", true)
                })
                .clicked()
            {
                state.filters.clear();
            }
        } else {
            portfolio_empty::compact(
                ui,
                portfolio_grid::body_rect(outer),
                portfolio_empty::Scene::Transactions,
                "No Transactions",
                "Add a transaction or import a CSV to get started.",
            );
        }
    } else {
        table_view(
            ui,
            state,
            input,
            outer,
            &shown[on_page.clone()],
            &mut actions,
        );
    }
    portfolio_grid::footer(
        ui,
        outer,
        &table::count_text(
            shown.len(),
            input.ledger.len(),
            ("transaction", "transactions"),
        ),
        &format!("Prices and fees in {}", input.formatter.currency.code()),
    );
    if shown.len() > PAGE_SIZES[0] {
        pager(ui, state, outer, shown.len(), on_page);
    }
    actions
}

/// The page size and the previous and next buttons, centred in the footer strip.
fn pager(
    ui: &mut egui::Ui,
    state: &mut TransactionsState,
    outer: Rect,
    total: usize,
    on_page: std::ops::Range<usize>,
) {
    const BUTTON: f32 = 24.0;
    const SIZE_WIDTH: f32 = 104.0;
    const GAP: f32 = 8.0;
    let strip = Rect::from_min_max(
        pos2(outer.left(), outer.bottom() - portfolio_grid::FOOTER_STRIP),
        outer.max,
    );
    let pages = page_count(total, state.page_size);
    let label = format!("{}\u{2013}{} of {total}", on_page.start + 1, on_page.end);
    let text = kit::galley(
        ui,
        &label,
        kit::size::CAPTION,
        REGULAR,
        theme::text_secondary(),
        240.0,
    );
    let pager_width = BUTTON + GAP + text.size().x + GAP + BUTTON;
    let block = SIZE_WIDTH + 16.0 + pager_width;
    let mut x = strip.center().x - block / 2.0;
    let middle = strip.center().y;

    let size_rect = Rect::from_min_size(pos2(x, middle - 11.0), vec2(SIZE_WIDTH, 22.0));
    let size_labels: Vec<String> = PAGE_SIZES
        .iter()
        .map(|size| format!("{size} per page"))
        .collect();
    ui.put(size_rect, |ui: &mut egui::Ui| {
        portfolio_grid::dropdown_inline(
            ui,
            egui::Id::new("portfolio-transactions-page-size"),
            &format!("{} per page", state.page_size),
            &size_labels,
            |ui| {
                for (size, name) in PAGE_SIZES.iter().zip(&size_labels) {
                    if kit::menu_item_marked(ui, Icon::Check, name, *size == state.page_size) {
                        state.page_size = *size;
                        state.page = 0;
                        ui.close();
                    }
                }
            },
        )
    });
    x += SIZE_WIDTH + 16.0;

    let step = |ui: &mut egui::Ui, rect: Rect, icon: Icon, enabled: bool, name: &str| {
        let response = ui
            .interact(
                rect,
                egui::Id::new(("portfolio-transactions-page", name)),
                Sense::click(),
            )
            .on_hover_cursor(if enabled {
                egui::CursorIcon::PointingHand
            } else {
                egui::CursorIcon::Default
            });
        if enabled && response.hovered() {
            ui.painter().rect_filled(rect, 6.0, theme::quaternary(0.75));
        }
        let color = if enabled {
            theme::text_primary()
        } else {
            theme::text_tertiary().gamma_multiply(0.6)
        };
        crate::ui::icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), egui::Vec2::splat(12.0)),
            icon,
            color,
        );
        enabled && response.clicked()
    };
    let previous = Rect::from_min_size(pos2(x, middle - BUTTON / 2.0), vec2(BUTTON, BUTTON));
    if step(ui, previous, Icon::ChevronLeft, state.page > 0, "previous") {
        state.page -= 1;
    }
    x += BUTTON + GAP;
    ui.painter().galley(
        pos2(x, middle - text.size().y / 2.0),
        text.clone(),
        Color32::PLACEHOLDER,
    );
    x += text.size().x + GAP;
    let next = Rect::from_min_size(pos2(x, middle - BUTTON / 2.0), vec2(BUTTON, BUTTON));
    if step(ui, next, Icon::ChevronRight, state.page + 1 < pages, "next") {
        state.page += 1;
    }
}

fn toolbar(
    ui: &mut egui::Ui,
    state: &mut TransactionsState,
    input: &TransactionsInput,
    actions: &mut Vec<TransactionAction>,
) {
    let search_id = egui::Id::new("portfolio-transactions-search");
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(INSET);
        ui.spacing_mut().item_spacing.x = 10.0;
        kit::search_field(
            ui,
            search_id,
            &mut state.filters.search,
            "Search asset, notes, or source",
            246.0,
        );

        let kind_label = state.filters.kind.map_or("All Types", type_name);
        let kind_choices: Vec<&str> = std::iter::once("All Types")
            .chain(ALL_TYPES.iter().map(|kind| type_name(*kind)))
            .collect();
        portfolio_grid::dropdown(
            ui,
            egui::Id::new("portfolio-type-filter"),
            kind_label,
            state.filters.kind.is_some(),
            &kind_choices,
            |ui| {
                let filters = &mut state.filters;
                if kit::menu_item_marked(ui, Icon::Check, "All Types", filters.kind.is_none()) {
                    filters.kind = None;
                    ui.close();
                }
                kit::menu_separator(ui);
                for kind in ALL_TYPES {
                    if kit::menu_item_marked(
                        ui,
                        Icon::Check,
                        type_name(kind),
                        filters.kind == Some(kind),
                    ) {
                        filters.kind = Some(kind);
                        ui.close();
                    }
                }
            },
        );

        let choices = asset_choices(input.ledger);
        let asset_label = state
            .filters
            .asset_key
            .as_ref()
            .and_then(|key| choices.iter().find(|(candidate, _)| candidate == key))
            .map_or("All Assets", |(_, ticker)| ticker.as_str());
        let asset_names: Vec<&str> = std::iter::once("All Assets")
            .chain(choices.iter().map(|(_, ticker)| ticker.as_str()))
            .collect();
        portfolio_grid::dropdown(
            ui,
            egui::Id::new("portfolio-asset-filter"),
            asset_label,
            state.filters.asset_key.is_some(),
            &asset_names,
            |ui| {
                let filters = &mut state.filters;
                if kit::menu_item_marked(ui, Icon::Check, "All Assets", filters.asset_key.is_none())
                {
                    filters.asset_key = None;
                    ui.close();
                }
                kit::menu_separator(ui);
                for (key, ticker) in &choices {
                    if kit::menu_item_marked(
                        ui,
                        Icon::Check,
                        ticker,
                        filters.asset_key.as_ref() == Some(key),
                    ) {
                        filters.asset_key = Some(key.clone());
                        ui.close();
                    }
                }
            },
        );
        if state.filters.is_active() && portfolio_grid::link(ui, "Clear") {
            state.filters.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(INSET);
            if kit::push_button(ui, Some(Icon::Plus), "Add Transaction", true).clicked() {
                actions.push(TransactionAction::Add);
            }
        });
    });
    ui.add_space(12.0);
    if ui.memory(|memory| memory.has_focus(search_id)) {
        state.navigation.focused = false;
    }
}

// MARK: Summary band

const BAND_HEIGHT: f32 = 108.0;

/// What the band above the table says about the whole ledger, whatever the filters show.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub buys: usize,
    pub sells: usize,
    /// Fees in the reporting currency.
    pub fees: f64,
    /// How many of each type, most common first (ties in the order of `ALL_TYPES`).
    pub by_kind: Vec<(PortfolioTransactionType, usize)>,
}

pub fn summarise(
    ledger: &[PortfolioTransaction],
    projection: &dyn Fn(u64) -> Option<Projection>,
) -> Summary {
    let count_of = |kind| ledger.iter().filter(|tx| tx.kind == kind).count();
    let by_kind = kind_counts(ledger.iter().map(|tx| tx.kind));
    Summary {
        count: ledger.len(),
        buys: count_of(PortfolioTransactionType::Buy),
        sells: count_of(PortfolioTransactionType::Sell),
        fees: ledger
            .iter()
            .filter_map(|tx| projection(tx.id))
            .map(|p| p.fee)
            .sum(),
        by_kind,
    }
}

/// How many transactions there are and of what kind, with one bar of the mix in the colours of
/// the type capsules below.
fn summary_band(ui: &mut egui::Ui, input: &TransactionsInput) {
    let f = input.formatter;
    let (full, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAND_HEIGHT), Sense::hover());
    let rect = egui::Rect::from_min_max(
        pos2(full.left() + INSET, full.top()),
        pos2(full.right() - INSET, full.bottom()),
    );
    lift(ui, rect, 14.0);
    let inner = rect.shrink2(vec2(20.0, 16.0));
    let secondary = theme::text_secondary();
    let summary = summarise(input.ledger, input.projection);

    let caption = put(
        ui,
        inner.left_top(),
        Align2::LEFT_TOP,
        "Transactions",
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let count = put(
        ui,
        pos2(inner.left(), inner.top() + 14.0),
        Align2::LEFT_TOP,
        &summary.count.to_string(),
        26.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    let galley = kit::galley(
        ui,
        &activity_caption(summary.buys, summary.sells),
        kit::size::CAPTION,
        MEDIUM,
        theme::SYSTEM_ACCENT,
        400.0,
    );
    let pill = Rect::from_min_size(
        pos2(inner.left(), inner.bottom() - 22.0),
        vec2(galley.size().x + 18.0, 22.0),
    );
    ui.painter()
        .rect_filled(pill, 11.0, theme::SYSTEM_ACCENT.gamma_multiply(0.16));
    ui.painter().galley(
        pill.center() - galley.size() / 2.0,
        galley,
        Color32::PLACEHOLDER,
    );

    let divider = caption.right().max(count.right()).max(pill.right()) + 28.0;
    let area = Rect::from_min_max(pos2(divider + 28.0, inner.top()), inner.right_bottom());
    if area.width() < 200.0 {
        return;
    }
    ui.painter().vline(
        divider,
        inner.y_range(),
        Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
    );
    put(
        ui,
        area.left_top(),
        Align2::LEFT_TOP,
        "By Type",
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    put(
        ui,
        area.right_top(),
        Align2::RIGHT_TOP,
        &if summary.fees > 0.0 {
            format!("Fees paid \u{00B7} {}", f.money(summary.fees))
        } else {
            "No fees paid".to_owned()
        },
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );

    type_mix(ui, area, &summary.by_kind, summary.count);
}

/// One bar of how many transactions there are of each type, in the colours of the type capsules,
/// with its legend under it. `area` is the block that also holds a caption row above (24 pt).
pub(crate) fn type_mix(
    ui: &mut egui::Ui,
    area: Rect,
    by_kind: &[(PortfolioTransactionType, usize)],
    total: usize,
) {
    let secondary = theme::text_secondary();
    let bar = Rect::from_min_size(
        pos2(area.left(), area.top() + 24.0),
        vec2(area.width(), 12.0),
    );
    ui.painter().rect_filled(bar, 6.0, theme::ink(10));
    let mut start = 0.0_f32;
    for (index, (kind, count)) in by_kind.iter().enumerate() {
        let from = bar.left() + bar.width() * start;
        start += *count as f32 / total as f32;
        let to = bar.left() + bar.width() * start;
        let gap = if index + 1 < by_kind.len() { 2.0 } else { 0.0 };
        let width = (to - from - gap).max(2.0);
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(from, bar.top()), vec2(width, bar.height())),
            (width / 2.0).min(6.0),
            type_tint(*kind),
        );
    }

    let middle = area.top() + 56.0;
    let mut x = area.left();
    for (kind, count) in by_kind {
        let galley = kit::galley(
            ui,
            &format!("{} {count}", type_name(*kind)),
            kit::size::CAPTION,
            REGULAR,
            secondary,
            300.0,
        );
        let width = 14.0 + galley.size().x;
        if x + width > area.right() {
            break;
        }
        ui.painter()
            .circle_filled(pos2(x + 3.5, middle), 3.5, type_tint(*kind));
        ui.painter().galley(
            pos2(x + 14.0, middle - galley.size().y / 2.0),
            galley,
            Color32::PLACEHOLDER,
        );
        x += width + 16.0;
    }
}

// MARK: Shared cells

/// A tick in the type's colour at the row's edge, so buys and sells read down a long table.
pub(crate) fn paint_type_tick(
    ui: &egui::Ui,
    first_cell: Rect,
    row_height: f32,
    kind: PortfolioTransactionType,
) {
    ui.painter().rect_filled(
        Rect::from_center_size(
            pos2(first_cell.left() + 3.0, first_cell.center().y),
            vec2(3.0, row_height - 22.0),
        ),
        1.5,
        type_tint(kind),
    );
}

/// The date over the time of day, in `cell`.
pub(crate) fn paint_date_cell(
    ui: &egui::Ui,
    cell: Rect,
    column: &Col,
    timestamp: i64,
    utc_offset_secs: i64,
) {
    let span = portfolio_grid::text_span(cell, column);
    let date = kit::galley(
        ui,
        &date_text(timestamp + utc_offset_secs * 1_000),
        kit::size::BODY,
        MEDIUM,
        theme::text_primary(),
        span.width(),
    );
    let time = kit::galley(
        ui,
        &clock_time(timestamp, utc_offset_secs),
        kit::size::CAPTION,
        REGULAR,
        theme::text_secondary(),
        span.width(),
    );
    let top = cell.center().y - (date.size().y + 1.0 + time.size().y) / 2.0;
    let time_top = top + date.size().y + 1.0;
    ui.painter()
        .galley(pos2(span.left() + 4.0, top), date, Color32::PLACEHOLDER);
    ui.painter().galley(
        pos2(span.left() + 4.0, time_top),
        time,
        Color32::PLACEHOLDER,
    );
}

/// The asset's icon, its ticker over its name, in `cell`.
pub(crate) fn paint_asset_cell(
    ui: &egui::Ui,
    cell: Rect,
    asset: &PortfolioAsset,
    ticker: &str,
    name: &str,
) {
    let icon = Rect::from_center_size(
        pos2(
            cell.left() + portfolio_grid::CELL_PAD + 12.0,
            cell.center().y,
        ),
        egui::Vec2::splat(24.0),
    );
    table::paint_asset_icon(ui, icon, asset);
    let text_left = icon.right() + 9.0;
    let text_width = (cell.right() - text_left - 6.0).max(20.0);
    let ticker = kit::galley(
        ui,
        ticker,
        kit::size::BODY,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let name = kit::galley(
        ui,
        name,
        kit::size::CAPTION,
        REGULAR,
        theme::text_secondary(),
        text_width,
    );
    let top = cell.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    let name_y = top + ticker.size().y + 1.0;
    ui.painter()
        .galley(pos2(text_left, top), ticker, Color32::PLACEHOLDER);
    ui.painter()
        .galley(pos2(text_left, name_y), name, Color32::PLACEHOLDER);
}

/// The type as a tinted capsule in `cell`; returns the capsule's rect.
pub(crate) fn paint_type_capsule(
    ui: &egui::Ui,
    cell: Rect,
    kind: PortfolioTransactionType,
    label: &str,
) -> Rect {
    let tint = type_tint(kind);
    let badge = kit::galley(ui, label, kit::size::CAPTION, kit::MEDIUM, tint, 200.0);
    let capsule = Rect::from_min_size(
        pos2(
            cell.left() + portfolio_grid::CELL_PAD,
            cell.center().y - (badge.size().y + 4.0) / 2.0,
        ),
        vec2(badge.size().x + 16.0, badge.size().y + 4.0),
    );
    ui.painter()
        .rect_filled(capsule, capsule.height() / 2.0, tint.gamma_multiply(0.14));
    ui.painter().galley(
        pos2(capsule.left() + 8.0, capsule.top() + 2.0),
        badge,
        Color32::PLACEHOLDER,
    );
    capsule
}

/// An asset's second line: its name, else the name embedded in its key, else its symbol.
pub fn asset_subtitle(asset: &PortfolioAsset) -> String {
    if asset.name.is_empty() {
        asset.embedded_name().unwrap_or(&asset.symbol).to_owned()
    } else {
        asset.name.clone()
    }
}

/// How many transactions of each type, most common first (ties keep the order of `ALL_TYPES`).
pub fn kind_counts(
    kinds: impl Iterator<Item = PortfolioTransactionType> + Clone,
) -> Vec<(PortfolioTransactionType, usize)> {
    let mut by_kind: Vec<(PortfolioTransactionType, usize)> = ALL_TYPES
        .iter()
        .map(|kind| {
            (
                *kind,
                kinds.clone().filter(|candidate| candidate == kind).count(),
            )
        })
        .filter(|(_, count)| *count > 0)
        .collect();
    by_kind.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    by_kind
}

// MARK: Table

const ROW_HEIGHT: f32 = 48.0;

/// The columns, in `TransactionColumn::ALL` order. Widths follow the Swift table's.
const COLUMNS: [Col; 8] = [
    Col::new(104.0, 145.0, false),
    Col::new(100.0, 120.0, false),
    Col::new(100.0, 110.0, false),
    Col::new(68.0, 90.0, true),
    Col::new(96.0, 95.0, true),
    Col::new(96.0, 100.0, true),
    Col::new(52.0, 75.0, true).unsortable(),
    // Row actions sit at the end of the last column, as in the Holdings table.
    Col::new(72.0, 120.0, false).trailing(MENU_RESERVE),
];

fn table_view(
    ui: &mut egui::Ui,
    state: &mut TransactionsState,
    input: &TransactionsInput,
    outer: Rect,
    shown: &[TransactionRow],
    actions: &mut Vec<TransactionAction>,
) {
    let f = input.formatter;
    let code = f.currency.code();
    let headers: Vec<String> = TransactionColumn::ALL
        .iter()
        .map(|column| column.title(code))
        .collect();
    let selected = state
        .selected
        .and_then(|id| shown.iter().position(|row| row.transaction.id == id));
    let sorted_column = TransactionColumn::ALL
        .iter()
        .position(|column| *column == state.column);
    let output = portfolio_grid::show_table(
        ui,
        outer,
        &TableSpec {
            id: "portfolio-transactions",
            columns: &COLUMNS,
            headers: &headers,
            row_height: ROW_HEIGHT,
            count: shown.len(),
            sorted: sorted_column.map(|column| (column, state.ascending)),
            selected,
            has_footer: true,
        },
        &mut state.navigation,
        |ui, row| transaction_row(ui, &shown[row.index], row, input, actions),
    );
    if let Some(index) = output.row_clicked.or(output.moved_to) {
        state.selected = Some(shown[index].transaction.id);
    }
    if let Some(index) = selected {
        let id = shown[index].transaction.id;
        if output.activated {
            actions.push(TransactionAction::Edit(id));
        }
        if output.deleted {
            actions.push(TransactionAction::Delete(id));
        }
    }
    if let Some(column) = output.header_clicked {
        state.click(TransactionColumn::ALL[column]);
    }
}

fn transaction_row(
    ui: &mut egui::Ui,
    item: &TransactionRow,
    row: &RowContext,
    input: &TransactionsInput,
    actions: &mut Vec<TransactionAction>,
) {
    let f = input.formatter;
    let tx = &item.transaction;
    let cells = row.cells;
    let primary = theme::text_primary();
    let secondary = theme::text_secondary();
    let figure = |ui: &egui::Ui, index: usize, text: &str, color: Color32| {
        portfolio_grid::figure(ui, cells[index], &COLUMNS[index], text, color)
    };

    paint_type_tick(ui, cells[0], ROW_HEIGHT, tx.kind);

    paint_date_cell(
        ui,
        cells[0],
        &COLUMNS[0],
        tx.timestamp,
        input.utc_offset_secs,
    );

    paint_asset_cell(
        ui,
        cells[1],
        &tx.asset,
        &item.ticker(),
        &(input.subtitle)(&tx.asset),
    );

    // Type: a tinted capsule; hovering says where the transaction came from.
    let capsule = paint_type_capsule(ui, cells[2], tx.kind, item.type_name());
    ui.interact(
        capsule,
        egui::Id::new(("portfolio-transaction-type", tx.id)),
        Sense::hover(),
    )
    .on_hover_text(format!("Source: {}", item.source_name()));

    let quantity_color = if f.privacy {
        primary
    } else {
        match quantity_sign(tx.kind) {
            QuantitySign::Plus => theme::UP,
            QuantitySign::Minus => theme::DOWN,
            QuantitySign::None => primary,
        }
    };
    let dash = || "\u{2014}".to_owned();
    figure(
        ui,
        3,
        &f.quantity(tx.quantity, quantity_sign(tx.kind)),
        quantity_color,
    );
    figure(
        ui,
        4,
        &item.price.map_or_else(dash, |v| f.price(v)),
        primary,
    );
    portfolio_grid::cell_text(
        ui,
        cells[5],
        &COLUMNS[5],
        &item.total().map_or_else(dash, |v| f.money(v)),
        kit::size::BODY,
        MEDIUM,
        primary,
    );
    figure(
        ui,
        6,
        &if item.fee == 0.0 {
            dash()
        } else {
            f.money(item.fee)
        },
        secondary,
    );

    let notes = portfolio_grid::cell_text(
        ui,
        cells[7],
        &COLUMNS[7],
        &tx.notes,
        kit::size::BODY,
        kit::REGULAR,
        secondary,
    );
    if !tx.notes.is_empty() {
        ui.interact(
            notes,
            egui::Id::new(("portfolio-transaction-notes", tx.id)),
            Sense::hover(),
        )
        .on_hover_text(&tx.notes);
    }

    portfolio_grid::row_menu_button(
        ui,
        cells[7],
        egui::Id::new(("portfolio-transaction-menu", tx.id)),
        &format!("Actions for {} {}", item.ticker(), item.type_name()),
        |ui| row_menu(ui, tx.id, actions),
    );
    if row.response.double_clicked() {
        actions.push(TransactionAction::Edit(tx.id));
    }
    egui::Popup::context_menu(row.response)
        .frame(kit::menu_frame())
        .show(|ui| row_menu(ui, tx.id, actions));
    let spoken = format!(
        "{} {}, {}, {}",
        item.type_name(),
        item.ticker(),
        f.quantity(tx.quantity, QuantitySign::None),
        date_time_text(tx.timestamp, input.utc_offset_secs),
    );
    accessibility::label_as(row.response.clone(), egui::WidgetType::Button, spoken);
}

fn row_menu(ui: &mut egui::Ui, id: u64, actions: &mut Vec<TransactionAction>) {
    ui.set_width(170.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    if kit::menu_item(ui, Icon::Pencil, "Edit", true, false) {
        actions.push(TransactionAction::Edit(id));
        ui.close();
    }
    if kit::menu_item(ui, Icon::CopyDoc, "Duplicate", true, false) {
        actions.push(TransactionAction::Duplicate(id));
        ui.close();
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, "Delete", true, true) {
        actions.push(TransactionAction::Delete(id));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioCurrency, PortfolioSource};
    use std::collections::HashMap;

    fn asset(ticker: &str, name: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: format!("binance:{ticker}USDT"),
            symbol: format!("{ticker}/USDT"),
            name: name.to_owned(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn tx(
        id: u64,
        ticker: &str,
        name: &str,
        kind: PortfolioTransactionType,
        qty: f64,
        at: i64,
        notes: &str,
    ) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: 1,
            asset: asset(ticker, name),
            kind,
            quantity: qty,
            price: Some(10.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: at,
            notes: notes.to_owned(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn ledger() -> Vec<PortfolioTransaction> {
        use PortfolioTransactionType::*;
        vec![
            tx(1, "BTC", "Bitcoin", Buy, 2.0, 100, "first buy"),
            tx(2, "ETH", "Ethereum", Buy, 10.0, 300, ""),
            tx(3, "BTC", "Bitcoin", Sell, 1.0, 200, "took profit"),
            tx(4, "ETH", "Ethereum", Reward, 0.5, 400, "staking"),
        ]
    }

    fn projection(id: u64) -> Option<Projection> {
        // Transaction 2 has no price (unpriced); 3 carries a fee.
        match id {
            2 => Some(Projection {
                price: None,
                fee: 0.0,
            }),
            3 => Some(Projection {
                price: Some(30.0),
                fee: 1.5,
            }),
            _ => Some(Projection {
                price: Some(10.0),
                fee: 0.0,
            }),
        }
    }

    fn subtitle(asset: &PortfolioAsset) -> String {
        asset.name.clone()
    }

    fn ids(rows: &[TransactionRow]) -> Vec<u64> {
        rows.iter().map(|r| r.transaction.id).collect()
    }

    fn query(filters: &Filters, column: TransactionColumn, ascending: bool) -> Vec<TransactionRow> {
        rows(
            &ledger(),
            &projection,
            &subtitle,
            filters,
            column,
            ascending,
        )
    }

    #[test]
    fn pages_split_the_rows_and_the_last_one_keeps_the_remainder() {
        assert_eq!(page_count(0, 100), 1);
        assert_eq!(page_count(100, 100), 1);
        assert_eq!(page_count(101, 100), 2);
        assert_eq!(page_range(250, 100, 0), 0..100);
        assert_eq!(page_range(250, 100, 2), 200..250);
        // A page past the end is the last one; no rows is an empty first page.
        assert_eq!(page_range(250, 100, 9), 200..250);
        assert_eq!(page_range(0, 100, 0), 0..0);
        assert_eq!(DEFAULT_PAGE_SIZE, 100);
        assert!(PAGE_SIZES.contains(&DEFAULT_PAGE_SIZE));
    }

    #[test]
    fn the_summary_counts_the_whole_ledger_by_type_and_adds_up_the_fees() {
        use PortfolioTransactionType::*;
        let summary = summarise(&ledger(), &projection);
        assert_eq!((summary.count, summary.buys, summary.sells), (4, 2, 1));
        assert_eq!(summary.fees, 1.5);
        // Most common first; the rest keep their order in `ALL_TYPES`.
        assert_eq!(summary.by_kind, [(Buy, 2), (Sell, 1), (Reward, 1)]);
        assert_eq!(
            summary
                .by_kind
                .iter()
                .map(|(_, count)| count)
                .sum::<usize>(),
            summary.count
        );
        assert_eq!(summarise(&[], &projection).by_kind, []);
    }

    #[test]
    fn the_default_view_is_newest_first() {
        let found = query(&Filters::default(), TransactionColumn::Date, false);
        assert_eq!(ids(&found), [4, 2, 3, 1]);
        let oldest = query(&Filters::default(), TransactionColumn::Date, true);
        assert_eq!(ids(&oldest), [1, 3, 2, 4]);
    }

    #[test]
    fn prices_totals_and_fees_are_the_projection_not_the_ledger() {
        let found = query(&Filters::default(), TransactionColumn::Date, true);
        let sell = found.iter().find(|r| r.transaction.id == 3).unwrap();
        assert_eq!((sell.price, sell.fee), (Some(30.0), 1.5));
        assert_eq!(sell.total(), Some(30.0), "price × quantity");
        let unpriced = found.iter().find(|r| r.transaction.id == 2).unwrap();
        assert_eq!((unpriced.price, unpriced.total()), (None, None));
    }

    #[test]
    fn unpriced_rows_sort_as_zero() {
        let by_price = query(&Filters::default(), TransactionColumn::Price, true);
        assert_eq!(by_price[0].transaction.id, 2);
        let by_total = query(&Filters::default(), TransactionColumn::Total, false);
        assert_eq!(
            by_total[0].transaction.id, 3,
            "30 × 1 beats 10 × 2 and 10 × 0.5"
        );
    }

    #[test]
    fn type_asset_and_note_filters_combine() {
        use PortfolioTransactionType::*;
        let buys = Filters {
            kind: Some(Buy),
            ..Default::default()
        };
        assert_eq!(ids(&query(&buys, TransactionColumn::Date, true)), [1, 2]);
        let btc = Filters {
            asset_key: Some("binance:BTCUSDT".into()),
            ..Default::default()
        };
        assert_eq!(ids(&query(&btc, TransactionColumn::Date, true)), [1, 3]);
        let btc_buys = Filters {
            kind: Some(Buy),
            ..btc.clone()
        };
        assert_eq!(ids(&query(&btc_buys, TransactionColumn::Date, true)), [1]);
        assert!(btc.is_active());
        let mut cleared = btc;
        cleared.clear();
        assert!(!cleared.is_active());
    }

    #[test]
    fn search_reaches_notes_names_sources_and_types() {
        for (needle, expected) in [
            ("profit", vec![3]),
            ("ethereum", vec![2, 4]),
            ("MANUAL", vec![1, 2, 3, 4]),
            ("reward", vec![4]),
            ("  sell ", vec![3]),
            ("nope", vec![]),
        ] {
            let filters = Filters {
                search: needle.into(),
                ..Default::default()
            };
            let mut found = ids(&query(&filters, TransactionColumn::Date, true));
            found.sort_unstable();
            assert_eq!(found, expected, "{needle:?}");
        }
        assert!(
            !Filters {
                search: "   ".into(),
                ..Default::default()
            }
            .is_active()
        );
    }

    #[test]
    fn text_columns_sort_a_to_z_ignoring_case() {
        let by_asset = query(&Filters::default(), TransactionColumn::Asset, true);
        assert_eq!(by_asset[0].ticker(), "BTC");
        let by_notes = query(&Filters::default(), TransactionColumn::Notes, true);
        assert_eq!(ids(&by_notes)[0], 2, "the empty note sorts first");
        let by_type = query(&Filters::default(), TransactionColumn::Type, true);
        assert_eq!(by_type[0].type_name(), "Buy");
    }

    #[test]
    fn clicking_headers_flips_or_switches_and_the_fee_column_does_not_sort() {
        let mut state = TransactionsState::default();
        assert_eq!(
            (state.column(), state.is_ascending()),
            (TransactionColumn::Date, false)
        );
        state.click(TransactionColumn::Date);
        assert!(state.is_ascending());
        state.click(TransactionColumn::Asset);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (TransactionColumn::Asset, true)
        );
        state.click(TransactionColumn::Total);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (TransactionColumn::Total, false)
        );
        state.click(TransactionColumn::Fee);
        assert_eq!(
            state.column(),
            TransactionColumn::Total,
            "fees are not a sort key"
        );
    }

    #[test]
    fn asset_choices_list_each_asset_once_a_to_z() {
        let mut more = ledger();
        more.push(tx(
            5,
            "ada",
            "Cardano",
            PortfolioTransactionType::Buy,
            1.0,
            500,
            "",
        ));
        let choices = asset_choices(&more);
        let tickers: Vec<&str> = choices.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(tickers, ["ADA", "BTC", "ETH"]);
    }

    #[test]
    fn quantities_carry_inflow_and_outflow_signs() {
        use PortfolioTransactionType::*;
        assert_eq!(quantity_sign(Buy), QuantitySign::Plus);
        assert_eq!(quantity_sign(Reward), QuantitySign::Plus);
        assert_eq!(quantity_sign(Sell), QuantitySign::Minus);
        assert_eq!(quantity_sign(Fee), QuantitySign::Minus);
        assert_eq!(quantity_sign(Adjustment), QuantitySign::None);
        assert_eq!(type_name(StakingReward), "Staking Reward");
        assert_eq!(ALL_TYPES.len(), 11);
    }

    #[test]
    fn dates_read_in_the_viewers_zone() {
        // 2024-01-15 23:30 UTC is already the 16th in UTC+2.
        let ms = 1_705_361_400_000;
        assert_eq!(date_time_text(ms, 0), "Jan 15, 2024 23:30");
        assert_eq!(date_time_text(ms, 2 * 3_600), "Jan 16, 2024 01:30");
        assert_eq!(TransactionColumn::Price.title("USD"), "Price (USD)");
        assert_eq!(TransactionColumn::Fee.title("USD"), "Fee");
    }
}
