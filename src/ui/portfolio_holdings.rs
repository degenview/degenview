//! The selected portfolio's positions as a sortable table.
//!
//! The sort column follows the portfolio's saved `PortfolioHoldingsSort`; clicking a header
//! that the saved sort has a case for writes it back. Double-click (or Return) opens the
//! asset's transactions. Pure over `PortfolioHolding`s: the host prices them and persists.

// Transient: the Portfolio window hosts this once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioAsset, PortfolioHolding, PortfolioHoldingsSort};
use crate::model::portfolio_format::PortfolioValueFormatter;
use crate::ui::accessibility;
use crate::ui::icons::Icon;
use crate::ui::kit::{self, SEMIBOLD};
use crate::ui::portfolio_charts;
use crate::ui::portfolio_empty;
use crate::ui::portfolio_grid::{
    self, Col, INSET, MENU_RESERVE, Navigation, RowContext, TableSpec,
};
use crate::ui::portfolio_table as table;
use crate::ui::theme;
use eframe::egui::{self, Color32, Rect, Vec2, pos2};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoldingsColumn {
    Asset,
    Price,
    DayChange,
    Quantity,
    AverageCost,
    Value,
    Allocation,
    ProfitLoss,
}

impl HoldingsColumn {
    pub const ALL: [Self; 8] = [
        Self::Asset,
        Self::Price,
        Self::DayChange,
        Self::Quantity,
        Self::AverageCost,
        Self::Value,
        Self::Allocation,
        Self::ProfitLoss,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Asset => "Asset",
            Self::Price => "Price",
            Self::DayChange => "24h %",
            Self::Quantity => "Quantity",
            Self::AverageCost => "Avg Cost",
            Self::Value => "Value",
            Self::Allocation => "Allocation",
            Self::ProfitLoss => "P&L",
        }
    }

    /// The saved sort a click on this column writes, for the columns the portfolio remembers.
    pub fn saved_sort(self) -> Option<PortfolioHoldingsSort> {
        match self {
            Self::Asset => Some(PortfolioHoldingsSort::Asset),
            Self::DayChange => Some(PortfolioHoldingsSort::DayChange),
            Self::Value => Some(PortfolioHoldingsSort::CurrentValue),
            Self::Allocation => Some(PortfolioHoldingsSort::Allocation),
            Self::ProfitLoss => Some(PortfolioHoldingsSort::ProfitLoss),
            Self::Price | Self::Quantity | Self::AverageCost => None,
        }
    }

    /// The column a saved sort shows. The table has one P&L column; it sorts by amount.
    pub fn for_saved(sort: PortfolioHoldingsSort) -> Self {
        match sort {
            PortfolioHoldingsSort::Asset => Self::Asset,
            PortfolioHoldingsSort::CurrentValue => Self::Value,
            PortfolioHoldingsSort::Allocation => Self::Allocation,
            PortfolioHoldingsSort::DayChange => Self::DayChange,
            PortfolioHoldingsSort::ProfitLoss | PortfolioHoldingsSort::ProfitLossPercent => {
                Self::ProfitLoss
            }
        }
    }

    /// A first click sorts names A to Z and numbers largest first.
    pub fn default_ascending(self) -> bool {
        self == Self::Asset
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HoldingRow {
    pub holding: PortfolioHolding,
    pub subtitle: String,
}

impl HoldingRow {
    pub fn ticker(&self) -> String {
        self.holding.asset.display_ticker()
    }

    pub fn key(&self) -> &str {
        &self.holding.asset.key
    }
}

/// Rows matching the search (ticker, symbol, name, subtitle), sorted by `column`. An
/// unpriced value sorts as the lowest: last when largest-first.
pub fn rows(
    holdings: &[PortfolioHolding],
    subtitle: &dyn Fn(&PortfolioAsset) -> String,
    search: &str,
    column: HoldingsColumn,
    ascending: bool,
) -> Vec<HoldingRow> {
    let needle = search.trim().to_lowercase();
    let mut rows: Vec<HoldingRow> = holdings
        .iter()
        .map(|holding| HoldingRow {
            subtitle: subtitle(&holding.asset),
            holding: holding.clone(),
        })
        .filter(|row| {
            needle.is_empty()
                || [
                    row.ticker(),
                    row.holding.asset.symbol.clone(),
                    row.holding.asset.name.clone(),
                    row.subtitle.clone(),
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&needle))
        })
        .collect();
    rows.sort_by(|a, b| {
        let order = compare(a, b, column);
        if ascending { order } else { order.reverse() }
    });
    rows
}

fn number(value: Option<f64>) -> f64 {
    value.unwrap_or(f64::NEG_INFINITY)
}

fn compare(a: &HoldingRow, b: &HoldingRow, column: HoldingsColumn) -> Ordering {
    let (x, y) = (&a.holding, &b.holding);
    match column {
        HoldingsColumn::Asset => a.ticker().to_lowercase().cmp(&b.ticker().to_lowercase()),
        HoldingsColumn::Price => number(x.current_price).total_cmp(&number(y.current_price)),
        HoldingsColumn::DayChange => {
            number(x.day_change_percent()).total_cmp(&number(y.day_change_percent()))
        }
        HoldingsColumn::Quantity => x.quantity.total_cmp(&y.quantity),
        HoldingsColumn::AverageCost => x.average_cost.total_cmp(&y.average_cost),
        HoldingsColumn::Value => number(x.current_value).total_cmp(&number(y.current_value)),
        HoldingsColumn::Allocation => x.allocation.total_cmp(&y.allocation),
        HoldingsColumn::ProfitLoss => number(x.total_pnl()).total_cmp(&number(y.total_pnl())),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HoldingsAction {
    /// Write the portfolio's saved sort.
    SetSort(PortfolioHoldingsSort),
    /// Open this asset's transactions (double-click, or the menu).
    Open(String),
    /// Remap the asset to another market.
    Remap(String),
}

pub struct HoldingsInput<'a> {
    pub holdings: &'a [PortfolioHolding],
    pub saved_sort: PortfolioHoldingsSort,
    pub formatter: PortfolioValueFormatter,
    /// Prices have not arrived yet: rows would be unpriced and reorder when they do.
    pub is_loading: bool,
    /// "Bitcoin", or the stored label.
    pub subtitle: &'a dyn Fn(&PortfolioAsset) -> String,
}

pub struct HoldingsState {
    pub search: String,
    column: HoldingsColumn,
    ascending: bool,
    selected: Option<String>,
    navigation: Navigation,
    /// The saved sort the table last adopted, so switching portfolios re-applies it.
    adopted: Option<PortfolioHoldingsSort>,
}

impl Default for HoldingsState {
    fn default() -> Self {
        Self {
            search: String::new(),
            column: HoldingsColumn::Value,
            ascending: false,
            selected: None,
            navigation: Navigation::default(),
            adopted: None,
        }
    }
}

impl HoldingsState {
    pub fn column(&self) -> HoldingsColumn {
        self.column
    }

    pub fn is_ascending(&self) -> bool {
        self.ascending
    }

    /// Follows the portfolio's saved sort when it changes (another portfolio, a write).
    pub fn adopt(&mut self, saved: PortfolioHoldingsSort) {
        if self.adopted == Some(saved) {
            return;
        }
        self.adopted = Some(saved);
        self.column = HoldingsColumn::for_saved(saved);
        self.ascending = self.column.default_ascending();
    }

    /// A click on a header: flips the direction on the sorted column, otherwise sorts by it.
    /// Returns the saved sort to write when this column is one the portfolio remembers and
    /// differs from what is saved.
    pub fn click(
        &mut self,
        column: HoldingsColumn,
        saved: PortfolioHoldingsSort,
    ) -> Option<PortfolioHoldingsSort> {
        if self.column == column {
            self.ascending = !self.ascending;
        } else {
            self.column = column;
            self.ascending = column.default_ascending();
        }
        let sort = column.saved_sort()?;
        (HoldingsColumn::for_saved(saved) != column).then(|| {
            self.adopted = Some(sort);
            sort
        })
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut HoldingsState,
    input: &HoldingsInput,
) -> Vec<HoldingsAction> {
    let mut actions = Vec::new();
    state.adopt(input.saved_sort);

    // Toolbar: the search field, padded like the table beneath it.
    let search_id = egui::Id::new("portfolio-holdings-search");
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(INSET);
        kit::search_field(
            ui,
            search_id,
            &mut state.search,
            "Search asset or name",
            246.0,
        );
    });
    ui.add_space(12.0);
    if ui.memory(|memory| memory.has_focus(search_id)) {
        state.navigation.focused = false;
    }

    let shown = rows(
        input.holdings,
        input.subtitle,
        &state.search,
        state.column,
        state.ascending,
    );
    let outer = portfolio_grid::allocate_table(ui);
    portfolio_grid::paint_page_frame(ui, outer);
    if input.is_loading && !input.holdings.is_empty() {
        portfolio_grid::loading_message(
            ui,
            portfolio_grid::body_rect(outer),
            "Loading prices\u{2026}",
        );
    } else if shown.is_empty() {
        let body = portfolio_grid::body_rect(outer);
        if input.holdings.is_empty() {
            portfolio_empty::compact(
                ui,
                body,
                portfolio_empty::Scene::Holdings,
                "No Holdings",
                "Positions appear once a transaction leaves you holding an asset.",
            );
        } else {
            portfolio_charts::empty_message(
                ui,
                body,
                Icon::Magnifier,
                &format!("No Results for \u{201C}{}\u{201D}", state.search.trim()),
                "Check the spelling or try a new search.",
            );
        }
    } else {
        table_view(ui, state, input, outer, &shown, &mut actions);
    }

    portfolio_grid::footer(
        ui,
        outer,
        &table::count_text(shown.len(), input.holdings.len(), ("asset", "assets")),
        &format!(
            "Values in {} \u{00B7} Double-click a row for its transactions",
            input.formatter.currency.code()
        ),
    );
    actions
}

const ROW_HEIGHT: f32 = 46.0;

/// The columns, in `HoldingsColumn::ALL` order. Widths follow the Swift table's ideal widths.
const COLUMNS: [Col; 8] = [
    Col::new(130.0, 150.0, false),
    Col::new(84.0, 100.0, true),
    Col::new(64.0, 70.0, true),
    Col::new(70.0, 90.0, true),
    Col::new(84.0, 100.0, true),
    Col::new(84.0, 105.0, true),
    Col::new(76.0, 76.0, true),
    // Row actions sit at the end of the last column, as in the Transactions table.
    Col::new(130.0, 140.0, true).trailing(MENU_RESERVE),
];

fn table_view(
    ui: &mut egui::Ui,
    state: &mut HoldingsState,
    input: &HoldingsInput,
    outer: Rect,
    shown: &[HoldingRow],
    actions: &mut Vec<HoldingsAction>,
) {
    let headers: Vec<String> = HoldingsColumn::ALL
        .iter()
        .map(|column| column.title().to_owned())
        .collect();
    let selected = state
        .selected
        .as_deref()
        .and_then(|key| shown.iter().position(|row| row.key() == key));
    let sorted_column = HoldingsColumn::ALL
        .iter()
        .position(|column| *column == state.column);
    let output = portfolio_grid::show_table(
        ui,
        outer,
        &TableSpec {
            id: "portfolio-holdings",
            columns: &COLUMNS,
            headers: &headers,
            row_height: ROW_HEIGHT,
            count: shown.len(),
            sorted: sorted_column.map(|column| (column, state.ascending)),
            selected,
            has_footer: true,
        },
        &mut state.navigation,
        |ui, row| holding_row(ui, &shown[row.index], row, input.formatter, actions),
    );
    if let Some(index) = output.row_clicked.or(output.moved_to) {
        state.selected = Some(shown[index].key().to_owned());
    }
    if output.activated
        && let Some(index) = selected
    {
        actions.push(HoldingsAction::Open(shown[index].key().to_owned()));
    }
    if let Some(column) = output.header_clicked
        && let Some(sort) = state.click(HoldingsColumn::ALL[column], input.saved_sort)
    {
        actions.push(HoldingsAction::SetSort(sort));
    }
}

fn holding_row(
    ui: &mut egui::Ui,
    item: &HoldingRow,
    row: &RowContext,
    f: PortfolioValueFormatter,
    actions: &mut Vec<HoldingsAction>,
) {
    let holding = &item.holding;
    let cells = row.cells;
    let primary = theme::text_primary();
    let secondary = theme::text_secondary();

    // Asset: icon, ticker over name.
    let asset = cells[0];
    let icon = Rect::from_center_size(
        pos2(
            asset.left() + portfolio_grid::CELL_PAD + 12.0,
            asset.center().y,
        ),
        Vec2::splat(24.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(icon), |ui| {
        table::asset_icon(ui, &holding.asset, 24.0);
    });
    let text_left = icon.right() + 10.0;
    let text_width = (asset.right() - text_left - 6.0).max(20.0);
    let ticker = kit::galley(
        ui,
        &item.ticker(),
        kit::size::BODY,
        SEMIBOLD,
        primary,
        text_width,
    );
    let name = kit::galley(
        ui,
        &item.subtitle,
        kit::size::CAPTION,
        kit::REGULAR,
        secondary,
        text_width,
    );
    let top = asset.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    let name_y = top + ticker.size().y + 1.0;
    ui.painter()
        .galley(pos2(text_left, top), ticker, Color32::PLACEHOLDER);
    ui.painter()
        .galley(pos2(text_left, name_y), name, Color32::PLACEHOLDER);

    let dash = || "\u{2014}".to_owned();
    let figure = |index: usize, text: &str, color: Color32| {
        portfolio_grid::figure(ui, cells[index], &COLUMNS[index], text, color)
    };
    figure(
        1,
        &holding.current_price.map_or_else(dash, |v| f.price(v)),
        primary,
    );
    let day = holding.day_change_percent();
    figure(
        2,
        &day.map_or_else(dash, |v| f.signed_percent(v, 2)),
        table::tone_option(f.tone(day)).unwrap_or(secondary),
    );
    figure(
        3,
        &f.quantity(holding.quantity, Default::default()),
        primary,
    );
    figure(4, &f.price(holding.average_cost), primary);
    portfolio_grid::cell_text(
        ui,
        cells[5],
        &COLUMNS[5],
        &holding.current_value.map_or_else(dash, |v| f.money(v)),
        kit::size::BODY,
        kit::MEDIUM,
        primary,
    );
    figure(6, &f.percent(holding.allocation, 1), secondary);
    let pnl = holding.total_pnl();
    portfolio_grid::cell_text(
        ui,
        cells[7],
        &COLUMNS[7],
        &pnl.map_or_else(dash, |v| f.signed_money(v)),
        kit::size::BODY,
        kit::MEDIUM,
        table::tone_option(f.tone(pnl)).unwrap_or(primary),
    );

    let ticker_text = item.ticker();
    portfolio_grid::row_menu_button(
        ui,
        cells[7],
        egui::Id::new(("portfolio-holding-menu", item.key())),
        &format!("Actions for {ticker_text}"),
        |ui| row_menu(ui, item.key(), actions),
    );
    if row.response.double_clicked() {
        actions.push(HoldingsAction::Open(item.key().to_owned()));
    }
    egui::Popup::context_menu(row.response)
        .frame(kit::menu_frame())
        .show(|ui| row_menu(ui, item.key(), actions));
    let spoken = format!(
        "{ticker_text}, {}, {}",
        item.subtitle,
        holding.current_value.map_or_else(dash, |v| f.money(v)),
    );
    accessibility::label_as(row.response.clone(), egui::WidgetType::Button, spoken);
}

fn row_menu(ui: &mut egui::Ui, key: &str, actions: &mut Vec<HoldingsAction>) {
    ui.set_width(190.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    if kit::menu_item(ui, Icon::List, "View Transactions", true, false) {
        actions.push(HoldingsAction::Open(key.to_owned()));
        ui.close();
    }
    if kit::menu_item(ui, Icon::ArrowLeftRight, "Remap Asset\u{2026}", true, false) {
        actions.push(HoldingsAction::Remap(key.to_owned()));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::PortfolioCurrency;
    use std::collections::HashMap;

    fn holding(
        ticker: &str,
        name: &str,
        value: Option<f64>,
        cost: f64,
        qty: f64,
    ) -> PortfolioHolding {
        PortfolioHolding {
            asset: PortfolioAsset {
                key: format!("binance:{ticker}USDT"),
                symbol: format!("{ticker}/USDT"),
                name: name.to_owned(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            quantity: qty,
            cost_basis: cost,
            average_cost: if qty == 0.0 { 0.0 } else { cost / qty },
            realized_pnl: 0.0,
            current_price: value.map(|v| v / qty),
            previous_day_price: value.map(|v| v / qty / 1.1),
            current_value: value,
            unrealized_pnl: value.map(|v| v - cost),
            allocation: value.unwrap_or(0.0) / 1_000.0,
            is_price_stale: false,
        }
    }

    fn fixtures() -> Vec<PortfolioHolding> {
        vec![
            holding("BTC", "Bitcoin", Some(600.0), 500.0, 0.01),
            holding("ETH", "Ethereum", Some(300.0), 350.0, 0.1),
            holding("DOGE", "Dogecoin", None, 50.0, 400.0),
            holding("ADA", "Cardano", Some(100.0), 60.0, 200.0),
        ]
    }

    fn tickers(rows: &[HoldingRow]) -> Vec<String> {
        rows.iter().map(HoldingRow::ticker).collect()
    }

    fn subtitle(asset: &PortfolioAsset) -> String {
        asset.name.clone()
    }

    #[test]
    fn value_sorts_largest_first_with_unpriced_assets_last() {
        let found = rows(&fixtures(), &subtitle, "", HoldingsColumn::Value, false);
        assert_eq!(tickers(&found), ["BTC", "ETH", "ADA", "DOGE"]);
        let reversed = rows(&fixtures(), &subtitle, "", HoldingsColumn::Value, true);
        assert_eq!(tickers(&reversed), ["DOGE", "ADA", "ETH", "BTC"]);
    }

    #[test]
    fn assets_sort_a_to_z_ignoring_case() {
        let found = rows(&fixtures(), &subtitle, "", HoldingsColumn::Asset, true);
        assert_eq!(tickers(&found), ["ADA", "BTC", "DOGE", "ETH"]);
    }

    #[test]
    fn profit_and_loss_sorts_by_amount_with_unknown_last() {
        let found = rows(
            &fixtures(),
            &subtitle,
            "",
            HoldingsColumn::ProfitLoss,
            false,
        );
        assert_eq!(tickers(&found), ["BTC", "ADA", "ETH", "DOGE"]);
    }

    #[test]
    fn quantity_and_allocation_sort_numerically() {
        let by_quantity = rows(&fixtures(), &subtitle, "", HoldingsColumn::Quantity, false);
        assert_eq!(tickers(&by_quantity), ["DOGE", "ADA", "ETH", "BTC"]);
        let by_allocation = rows(
            &fixtures(),
            &subtitle,
            "",
            HoldingsColumn::Allocation,
            false,
        );
        assert_eq!(by_allocation[0].ticker(), "BTC");
    }

    #[test]
    fn search_matches_ticker_pair_name_and_subtitle() {
        let all = fixtures();
        for (needle, expected) in [
            ("btc", vec!["BTC"]),
            ("COIN", vec!["BTC", "DOGE"]),
            ("usdt", vec!["BTC", "ETH", "DOGE", "ADA"]),
            ("  card ", vec!["ADA"]),
            ("zzz", vec![]),
        ] {
            let mut found = tickers(&rows(&all, &subtitle, needle, HoldingsColumn::Value, false));
            found.sort();
            let mut want: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
            want.sort();
            assert_eq!(found, want, "{needle:?}");
        }
    }

    #[test]
    fn saved_sorts_map_onto_columns_and_back() {
        use PortfolioHoldingsSort::*;
        for (sort, column) in [
            (Asset, HoldingsColumn::Asset),
            (CurrentValue, HoldingsColumn::Value),
            (Allocation, HoldingsColumn::Allocation),
            (DayChange, HoldingsColumn::DayChange),
            (ProfitLoss, HoldingsColumn::ProfitLoss),
            (ProfitLossPercent, HoldingsColumn::ProfitLoss),
        ] {
            assert_eq!(HoldingsColumn::for_saved(sort), column);
        }
        assert_eq!(HoldingsColumn::Price.saved_sort(), None);
        assert_eq!(HoldingsColumn::Quantity.saved_sort(), None);
        assert_eq!(HoldingsColumn::AverageCost.saved_sort(), None);
        assert_eq!(HoldingsColumn::Value.saved_sort(), Some(CurrentValue));
    }

    #[test]
    fn a_header_click_flips_or_switches_and_writes_back_only_remembered_columns() {
        let mut state = HoldingsState::default();
        state.adopt(PortfolioHoldingsSort::CurrentValue);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (HoldingsColumn::Value, false)
        );

        // Clicking the sorted column flips direction; the saved sort already names it.
        assert_eq!(
            state.click(HoldingsColumn::Value, PortfolioHoldingsSort::CurrentValue),
            None
        );
        assert!(state.is_ascending());

        // A new remembered column is written back, names A to Z, numbers largest first.
        assert_eq!(
            state.click(HoldingsColumn::Asset, PortfolioHoldingsSort::CurrentValue),
            Some(PortfolioHoldingsSort::Asset)
        );
        assert!(state.is_ascending());
        assert_eq!(
            state.click(HoldingsColumn::Allocation, PortfolioHoldingsSort::Asset),
            Some(PortfolioHoldingsSort::Allocation)
        );
        assert!(!state.is_ascending());

        // A transient column sorts the view but writes nothing.
        assert_eq!(
            state.click(HoldingsColumn::Price, PortfolioHoldingsSort::Allocation),
            None
        );
        assert_eq!(state.column(), HoldingsColumn::Price);
    }

    #[test]
    fn switching_portfolios_adopts_the_other_ones_saved_sort_once() {
        let mut state = HoldingsState::default();
        state.adopt(PortfolioHoldingsSort::Asset);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (HoldingsColumn::Asset, true)
        );
        state.click(HoldingsColumn::Price, PortfolioHoldingsSort::Asset);
        // The same saved sort again must not undo the user's transient choice...
        state.adopt(PortfolioHoldingsSort::Asset);
        assert_eq!(state.column(), HoldingsColumn::Price);
        // ...but a different one (another portfolio) is followed.
        state.adopt(PortfolioHoldingsSort::DayChange);
        assert_eq!(state.column(), HoldingsColumn::DayChange);
        assert!(!state.is_ascending());
    }
}
