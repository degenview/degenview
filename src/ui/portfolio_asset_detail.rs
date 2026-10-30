//! One asset's position and its transactions, shown as a sheet over the Holdings tab. A pure
//! view: the host resolves the holding and the transactions from the ledger on every frame (so
//! editing or deleting a transaction refreshes the figures in place) and applies the actions.
//! Every amount is in the reporting currency — the same one the Holdings table shows — and says so.

// Transient: the portfolio host opens this once it is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioAsset, PortfolioHolding, PortfolioTransaction};
use crate::model::portfolio_format::PortfolioValueFormatter;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::portfolio_charts;
use crate::ui::portfolio_grid::{self, Col, MENU_RESERVE, Navigation, RowContext, TableSpec};
use crate::ui::portfolio_overview::{StatCard, grid};
use crate::ui::portfolio_transactions::{date_time_text, quantity_sign, type_name, type_tint};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};

pub const SIZE: Vec2 = Vec2::new(880.0, 660.0);

pub struct AssetDetailInput<'a> {
    /// None when the transactions net to zero.
    pub holding: Option<&'a PortfolioHolding>,
    /// The asset, from the holding or else from its first transaction.
    pub asset: Option<&'a PortfolioAsset>,
    /// "Bitcoin" or the stored label.
    pub subtitle: &'a str,
    /// This asset's transactions, newest first (`asset_transactions`).
    pub transactions: &'a [&'a PortfolioTransaction],
    /// A transaction's price in the reporting currency; the ledger's own may be in another.
    pub price_of: &'a dyn Fn(&PortfolioTransaction) -> Option<f64>,
    pub formatter: PortfolioValueFormatter,
    pub utc_offset_secs: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetDetailAction {
    Edit(u64),
    Duplicate(u64),
    /// The host confirms before deleting.
    Delete(u64),
    Close,
}

/// The ledger's transactions in one asset, newest first.
pub fn asset_transactions<'a>(
    ledger: &'a [PortfolioTransaction],
    asset_key: &str,
) -> Vec<&'a PortfolioTransaction> {
    let mut rows: Vec<&PortfolioTransaction> = ledger
        .iter()
        .filter(|transaction| transaction.asset.key == asset_key)
        .collect();
    rows.sort_by_key(|transaction| std::cmp::Reverse(transaction.timestamp));
    rows
}

// MARK: Pure text

pub fn values_note(code: &str) -> String {
    format!("Values in {code}")
}

pub const NO_POSITION: &str = "No open position — the transactions below net to zero.";

/// The eight position cards.
pub fn stat_cards<'a>(
    holding: &'a PortfolioHolding,
    f: PortfolioValueFormatter,
) -> Vec<StatCard<'a>> {
    let unavailable = "Unavailable".to_owned();
    vec![
        StatCard::new(
            "Holdings",
            if f.privacy {
                f.quantity(holding.quantity, Default::default())
            } else {
                format!(
                    "{} {}",
                    f.quantity(holding.quantity, Default::default()),
                    holding.asset.display_ticker()
                )
            },
        ),
        StatCard::new(
            "Current Value",
            holding
                .current_value
                .map_or(unavailable.clone(), |v| f.money(v)),
        )
        .placeholder(holding.current_value.is_none()),
        StatCard::new("Average Cost", f.price(holding.average_cost)),
        StatCard::new(
            "Current Price",
            holding.current_price.map_or(unavailable, |v| f.price(v)),
        )
        .placeholder(holding.current_price.is_none()),
        StatCard::new(
            "Unrealized P&L",
            holding
                .unrealized_pnl
                .map_or("—".to_owned(), |v| f.signed_money(v)),
        )
        .tone(f.tone(holding.unrealized_pnl))
        .placeholder(holding.unrealized_pnl.is_none()),
        StatCard::new("Realized P&L", f.signed_money(holding.realized_pnl))
            .tone(f.tone(Some(holding.realized_pnl))),
        StatCard::new(
            "Total P&L",
            holding
                .total_pnl()
                .map_or("—".to_owned(), |v| f.signed_money(v)),
        )
        .tone(f.tone(holding.total_pnl()))
        .caption(holding.pnl_percent().map(|p| f.signed_percent(p, 2)))
        .placeholder(holding.total_pnl().is_none()),
        StatCard::new("Allocation", f.percent(holding.allocation, 2)),
    ]
}

// MARK: View

/// What the sheet remembers between frames: the highlighted row and keyboard focus.
#[derive(Debug, Default)]
pub struct AssetDetailState {
    selected: Option<u64>,
    navigation: Navigation,
}

const HEADER_ICON: f32 = 40.0;
/// Space under the table: a gap, the divider, and the padded Done row.
const FOOTER_HEIGHT: f32 = 16.0 + 1.0 + 14.0 + 28.0 + 14.0;
const ROW_HEIGHT: f32 = 42.0;

const COLUMNS: [Col; 6] = [
    Col::new(130.0, 150.0, false),
    Col::new(112.0, 110.0, false),
    Col::new(80.0, 100.0, true),
    Col::new(84.0, 105.0, true),
    Col::new(84.0, 110.0, true),
    // Row actions sit at the end of the last column, as in the Transactions tab.
    Col::new(100.0, 140.0, false).trailing(MENU_RESERVE),
];

pub fn show(
    ctx: &egui::Context,
    state: &mut AssetDetailState,
    input: &AssetDetailInput,
) -> Vec<AssetDetailAction> {
    let mut actions = Vec::new();
    sheet::show(ctx, Id::new("portfolio-asset-detail"), SIZE.x, |ui| {
        ui.set_height(SIZE.y);
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            header(ui, input);
            ui.add_space(18.0);
            match input.holding {
                Some(holding) => {
                    ui.label(kit::text(
                        values_note(input.formatter.currency.code()),
                        kit::size::CAPTION,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    ui.add_space(8.0);
                    let cards = stat_cards(holding, input.formatter);
                    for row in cards.chunks(4) {
                        grid(ui, row);
                        ui.add_space(12.0);
                    }
                    ui.add_space(-12.0);
                }
                None => no_position(ui),
            }
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(kit::text(
                    "Transactions",
                    kit::size::HEADLINE,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.label(kit::text(
                    input.transactions.len().to_string(),
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            });
            ui.add_space(8.0);
            let height = (ui.available_height() - FOOTER_HEIGHT).max(100.0);
            let (outer, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
            portfolio_grid::paint_frame(ui, outer);
            transactions(ui, state, input, outer, &mut actions);
        });
        ui.add_space(16.0);
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::footer_button(ui, "Done", kit::FooterButton::Primary, true, 76.0).clicked()
                {
                    actions.push(AssetDetailAction::Close);
                }
            });
            ui.add_space(14.0);
        });
    });
    if actions.is_empty() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(AssetDetailAction::Close);
    }
    actions
}

fn no_position(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    ui.painter().rect_filled(rect, 10.0, theme::quaternary(0.4));
    ui.painter().rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.4)),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.left() + 26.0, rect.center().y),
            Vec2::splat(14.0),
        ),
        Icon::Layers,
        theme::text_secondary(),
    );
    let label = kit::galley(
        ui,
        NO_POSITION,
        12.0,
        kit::REGULAR,
        theme::text_secondary(),
        rect.width() - 60.0,
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 44.0, rect.center().y - label.size().y / 2.0),
        label,
        Color32::PLACEHOLDER,
    );
}

fn header(ui: &mut egui::Ui, input: &AssetDetailInput) {
    let Some(asset) = input.asset else {
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(HEADER_ICON), Sense::hover());
        crate::ui::portfolio_table::paint_asset_icon(ui, rect, asset);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(kit::text(
                asset.display_ticker(),
                22.0,
                kit::BOLD,
                theme::text_primary(),
            ));
            ui.label(kit::text(
                format!("{} · {}", input.subtitle, asset.source.display_name()),
                kit::size::BODY,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        });
    });
}

fn transactions(
    ui: &mut egui::Ui,
    state: &mut AssetDetailState,
    input: &AssetDetailInput,
    outer: Rect,
    actions: &mut Vec<AssetDetailAction>,
) {
    if input.transactions.is_empty() {
        portfolio_charts::empty_message(
            ui,
            outer,
            Icon::List,
            "No Transactions",
            "This asset has no transactions in this portfolio.",
        );
        return;
    }
    let f = input.formatter;
    let code = f.currency.code();
    let headers: Vec<String> = [
        "Date".to_owned(),
        "Type".to_owned(),
        "Quantity".to_owned(),
        format!("Price ({code})"),
        format!("Total ({code})"),
        "Notes".to_owned(),
    ]
    .into();
    let rows = input.transactions;
    let selected = state
        .selected
        .and_then(|id| rows.iter().position(|row| row.id == id));
    let output = portfolio_grid::show_table(
        ui,
        outer,
        &TableSpec {
            id: "portfolio-asset-detail",
            columns: &COLUMNS.map(|column| column.unsortable()),
            headers: &headers,
            row_height: ROW_HEIGHT,
            count: rows.len(),
            sorted: None,
            selected,
            has_footer: false,
        },
        &mut state.navigation,
        |ui, row| transaction_row(ui, rows[row.index], row, input, actions),
    );
    if let Some(index) = output.row_clicked.or(output.moved_to) {
        state.selected = Some(rows[index].id);
    }
    if let Some(index) = selected {
        if output.activated {
            actions.push(AssetDetailAction::Edit(rows[index].id));
        }
        if output.deleted {
            actions.push(AssetDetailAction::Delete(rows[index].id));
        }
    }
}

fn transaction_row(
    ui: &mut egui::Ui,
    transaction: &PortfolioTransaction,
    row: &RowContext,
    input: &AssetDetailInput,
    actions: &mut Vec<AssetDetailAction>,
) {
    let f = input.formatter;
    let cells = row.cells;
    let columns = COLUMNS;
    let primary = theme::text_primary();
    let price = (input.price_of)(transaction);
    let dash = || "\u{2014}".to_owned();

    portfolio_grid::figure(
        ui,
        cells[0],
        &columns[0],
        &date_time_text(transaction.timestamp, input.utc_offset_secs),
        primary,
    );

    let tint = type_tint(transaction.kind);
    let badge = kit::galley(
        ui,
        type_name(transaction.kind),
        kit::size::CAPTION,
        kit::MEDIUM,
        tint,
        200.0,
    );
    let capsule = Rect::from_min_size(
        Pos2::new(
            cells[1].left() + portfolio_grid::CELL_PAD,
            cells[1].center().y - (badge.size().y + 4.0) / 2.0,
        ),
        Vec2::new(badge.size().x + 16.0, badge.size().y + 4.0),
    );
    ui.painter()
        .rect_filled(capsule, capsule.height() / 2.0, tint.gamma_multiply(0.14));
    ui.painter().galley(
        Pos2::new(capsule.left() + 8.0, capsule.top() + 2.0),
        badge,
        Color32::PLACEHOLDER,
    );

    let quantity_color = if f.privacy {
        primary
    } else if transaction.kind.adds_quantity() {
        theme::UP
    } else if transaction.kind.removes_quantity() {
        theme::DOWN
    } else {
        primary
    };
    portfolio_grid::figure(
        ui,
        cells[2],
        &columns[2],
        &f.quantity(transaction.quantity, quantity_sign(transaction.kind)),
        quantity_color,
    );
    portfolio_grid::figure(
        ui,
        cells[3],
        &columns[3],
        &price.map_or_else(dash, |p| f.price(p)),
        primary,
    );
    portfolio_grid::figure(
        ui,
        cells[4],
        &columns[4],
        &price.map_or_else(dash, |p| f.money(p * transaction.quantity)),
        primary,
    );
    let notes = portfolio_grid::cell_text(
        ui,
        cells[5],
        &columns[5],
        &transaction.notes,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_secondary(),
    );
    if !transaction.notes.is_empty() {
        ui.interact(
            notes,
            Id::new(("portfolio-detail-notes", transaction.id)),
            Sense::hover(),
        )
        .on_hover_text(&transaction.notes);
    }

    let id = transaction.id;
    portfolio_grid::row_menu_button(
        ui,
        cells[5],
        Id::new(("portfolio-detail-menu", id)),
        &format!("Actions for {}", type_name(transaction.kind)),
        |ui| row_menu(ui, id, actions),
    );
    if row.response.double_clicked() {
        actions.push(AssetDetailAction::Edit(id));
    }
    egui::Popup::context_menu(row.response)
        .frame(kit::menu_frame())
        .show(|ui| row_menu(ui, id, actions));
}

fn row_menu(ui: &mut egui::Ui, id: u64, actions: &mut Vec<AssetDetailAction>) {
    ui.set_width(170.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    if kit::menu_item(ui, Icon::Pencil, "Edit", true, false) {
        actions.push(AssetDetailAction::Edit(id));
        ui.close();
    }
    if kit::menu_item(ui, Icon::CopyDoc, "Duplicate", true, false) {
        actions.push(AssetDetailAction::Duplicate(id));
        ui.close();
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, "Delete", true, true) {
        actions.push(AssetDetailAction::Delete(id));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioCurrency, PortfolioSource, PortfolioTransactionType};
    use std::collections::HashMap;

    fn asset(key: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: key.into(),
            symbol: "BTC/USDT".into(),
            name: "Bitcoin".into(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn transaction(id: u64, key: &str, timestamp: i64) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: 1,
            asset: asset(key),
            kind: PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(10.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn holding() -> PortfolioHolding {
        PortfolioHolding {
            asset: asset("Binance:BTCUSDT"),
            quantity: 0.5,
            cost_basis: 100.0,
            average_cost: 200.0,
            realized_pnl: -3.0,
            current_price: Some(260.0),
            previous_day_price: None,
            current_value: Some(130.0),
            unrealized_pnl: Some(30.0),
            allocation: 0.25,
            is_price_stale: false,
        }
    }

    #[test]
    fn an_assets_transactions_list_newest_first() {
        let ledger = vec![
            transaction(1, "Binance:BTCUSDT", 100),
            transaction(2, "Binance:ETHUSDT", 500),
            transaction(3, "Binance:BTCUSDT", 300),
        ];
        let ids: Vec<u64> = asset_transactions(&ledger, "Binance:BTCUSDT")
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, [3, 1]);
        assert!(asset_transactions(&ledger, "none").is_empty());
    }

    #[test]
    fn the_position_cards_read_like_the_holdings_table() {
        let f = PortfolioValueFormatter::new(PortfolioCurrency::Usd, false);
        let holding = holding();
        let cards = stat_cards(&holding, f);
        let titles: Vec<&str> = cards.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            [
                "Holdings",
                "Current Value",
                "Average Cost",
                "Current Price",
                "Unrealized P&L",
                "Realized P&L",
                "Total P&L",
                "Allocation"
            ]
        );
        assert_eq!(cards[0].value, "0.5 BTC");
        assert_eq!(cards[1].value, "$130.00");
        assert_eq!(cards[2].value, "$200.00");
        assert_eq!(cards[4].value, f.signed_money(30.0));
        assert_eq!(
            cards[6].value,
            f.signed_money(27.0),
            "30 unrealized − 3 realized"
        );
        assert!(cards[6].caption.is_some());
        assert_eq!(cards[7].value, "25.00%");
        assert!(!cards[1].placeholder);
    }

    #[test]
    fn missing_prices_read_unavailable_and_privacy_hides_the_ticker_suffix() {
        let f = PortfolioValueFormatter::new(PortfolioCurrency::Usd, false);
        let mut unpriced = holding();
        unpriced.current_price = None;
        unpriced.current_value = None;
        unpriced.unrealized_pnl = None;
        let cards = stat_cards(&unpriced, f);
        assert_eq!(cards[1].value, "Unavailable");
        assert!(cards[1].placeholder && cards[3].placeholder);
        assert_eq!(cards[4].value, "—");
        assert_eq!(cards[6].value, "—");
        let shown = holding();
        let masked = stat_cards(
            &shown,
            PortfolioValueFormatter::new(PortfolioCurrency::Usd, true),
        );
        assert!(!masked[0].value.contains("BTC"), "{}", masked[0].value);
        assert_eq!(values_note("EUR"), "Values in EUR");
        assert!(NO_POSITION.starts_with("No open position"));
    }
}
