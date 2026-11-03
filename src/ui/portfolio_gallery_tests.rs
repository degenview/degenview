//! Pictures of the Portfolio pages at several window sizes, for a person to look at. Run by hand:
//! `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib portfolio_gallery -- --ignored`.

use crate::model::DataSource;
use crate::model::portfolio::{
    PortfolioAsset, PortfolioCurrency, PortfolioHolding, PortfolioSnapshot,
};
use crate::model::portfolio_format::PortfolioValueFormatter;
use crate::ui::portfolio_empty::{self, Scene};
use crate::ui::portfolio_holdings::{self, HoldingsInput, HoldingsState};
use crate::ui::portfolio_overview::{
    self, OverviewInput, OverviewState, PriceStatus, StatisticsInput,
};
use crate::ui::portfolio_transactions::{self, Projection, TransactionsInput, TransactionsState};
use crate::ui::render_tests::Screen;
use crate::ui::{header_controls, icons::Icon, theme};
use eframe::egui::{self, vec2};
use std::collections::HashMap;

const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 86_400_000;

fn holding(ticker: &str, name: &str, value: f64, cost: f64, allocation: f64) -> PortfolioHolding {
    PortfolioHolding {
        asset: PortfolioAsset {
            key: format!("coinmarketcap:{ticker}"),
            symbol: ticker.to_owned(),
            name: name.to_owned(),
            source: DataSource::CoinMarketCap,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        },
        quantity: 1.0,
        cost_basis: cost,
        average_cost: cost,
        realized_pnl: 0.0,
        current_price: Some(value),
        previous_day_price: Some(value * 0.98),
        current_value: Some(value),
        unrealized_pnl: Some(value - cost),
        allocation,
        is_price_stale: false,
    }
}

fn holdings() -> Vec<PortfolioHolding> {
    vec![
        holding("BTC", "Bitcoin", 8_120.5, 5_900.0, 0.52),
        holding("ETH", "Ethereum", 3_410.0, 3_050.0, 0.22),
        holding("SOL", "Solana", 1_850.0, 1_990.0, 0.12),
        holding("LINK", "Chainlink", 1_240.0, 800.0, 0.08),
        holding("ARB", "Arbitrum", 640.0, 790.0, 0.04),
        holding("DOGE", "Dogecoin", 320.0, 400.0, 0.02),
    ]
}

fn history() -> Vec<PortfolioSnapshot> {
    (0..120)
        .map(|day| {
            let wave = (day as f64 * 0.21).sin() * 520.0 + (day as f64 * 0.07).cos() * 380.0;
            let value = 12_000.0 + day as f64 * 28.0 + wave;
            PortfolioSnapshot {
                portfolio_id: 1,
                timestamp: NOW - (120 - day) * DAY,
                value,
                net_contributions: 10_400.0,
                realized_pnl: 0.0,
                unrealized_pnl: value - 10_400.0,
                is_complete: true,
            }
        })
        .collect()
}

fn frame(
    name: &str,
    size: egui::Vec2,
    theme_name: &str,
    app_theme: theme::AppTheme,
    body: impl Fn(&mut egui::Ui),
) {
    let mut screen = Screen::new(size);
    egui_extras::install_image_loaders(&screen.ctx);
    theme::apply(&screen.ctx, app_theme);
    let mut last = None;
    for _ in 0..4 {
        last = Some(screen.run(Vec::new(), |ui| {
            let full = ui.max_rect();
            ui.painter().rect_filled(full, 0.0, theme::background());
            body(ui);
        }));
    }
    screen.save(&format!("{name}-{theme_name}"), last.expect("a frame"));
}

fn both_themes(name: &str, size: egui::Vec2, body: impl Fn(&mut egui::Ui) + Copy) {
    let _theme = theme::RestoreDefaultTheme;
    for (theme_name, app_theme) in [
        ("dark", theme::AppTheme::Dark),
        ("light", theme::AppTheme::Light),
    ] {
        frame(name, size, theme_name, app_theme, body);
    }
}

fn overview(ui: &mut egui::Ui, state: &mut OverviewState, empty: bool) {
    let holdings = if empty { Vec::new() } else { holdings() };
    let history = if empty { Vec::new() } else { history() };
    let total: f64 = holdings.iter().filter_map(|h| h.current_value).sum();
    let subtitle = |asset: &PortfolioAsset| asset.name.clone();
    portfolio_overview::show_overview(
        ui,
        state,
        &OverviewInput {
            portfolio_name: Some("Main"),
            holdings: &holdings,
            history: &history,
            total_value: total,
            formatter: PortfolioValueFormatter::new(PortfolioCurrency::Usd, false),
            subtitle: &subtitle,
            market_value_caption: "Live market value",
            price_status: PriceStatus::Live,
            is_loading_initial: false,
            is_loading_history: false,
            now_ms: NOW,
            utc_offset_secs: 0,
        },
    );
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_overview() {
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    for (name, size) in [
        ("portfolio-overview-1100", vec2(1100.0, 1000.0)),
        ("portfolio-overview-1480", vec2(1480.0, 980.0)),
        ("portfolio-overview-760", vec2(760.0, 1100.0)),
    ] {
        both_themes(name, size, |ui| {
            let mut state = OverviewState::default();
            overview(ui, &mut state, false);
        });
    }
    // The first frames, before any price: the holdings exist, the valuation does not.
    for (name, size) in [
        ("portfolio-overview-loading", vec2(1100.0, 1000.0)),
        ("portfolio-overview-loading-760", vec2(760.0, 1100.0)),
    ] {
        both_themes(name, size, |ui| {
            let mut state = OverviewState::default();
            let mut holdings = holdings();
            for holding in &mut holdings {
                holding.current_value = None;
                holding.current_price = None;
            }
            let subtitle = |asset: &PortfolioAsset| asset.name.clone();
            portfolio_overview::show_overview(
                ui,
                &mut state,
                &OverviewInput {
                    portfolio_name: Some("Main"),
                    holdings: &holdings,
                    history: &[],
                    total_value: 0.0,
                    formatter: PortfolioValueFormatter::new(PortfolioCurrency::Usd, false),
                    subtitle: &subtitle,
                    market_value_caption: "Live market value",
                    price_status: PriceStatus::Live,
                    is_loading_initial: true,
                    is_loading_history: true,
                    now_ms: NOW,
                    utc_offset_secs: 0,
                },
            );
        });
    }
    both_themes("portfolio-overview-empty", vec2(1100.0, 1000.0), |ui| {
        let mut state = OverviewState::default();
        overview(ui, &mut state, true);
    });
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_statistics() {
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    for (name, empty, size) in [
        ("portfolio-statistics", false, vec2(1100.0, 1000.0)),
        ("portfolio-statistics-1480", false, vec2(1480.0, 900.0)),
        ("portfolio-statistics-760", false, vec2(760.0, 1250.0)),
        ("portfolio-statistics-420", false, vec2(420.0, 1500.0)),
        ("portfolio-statistics-empty", true, vec2(1100.0, 900.0)),
        ("portfolio-statistics-empty-420", true, vec2(420.0, 1200.0)),
    ] {
        both_themes(name, size, move |ui| {
            let holdings = if empty { Vec::new() } else { holdings() };
            let history = if empty { Vec::new() } else { history() };
            portfolio_overview::show_statistics(
                ui,
                &StatisticsInput {
                    holdings: &holdings,
                    history: &history,
                    transactions: &[],
                    formatter: PortfolioValueFormatter::new(PortfolioCurrency::Usd, false),
                    now_ms: NOW,
                    utc_offset_secs: 0,
                },
            );
        });
    }
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_empty_pages() {
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    for (name, scene) in [
        ("portfolio-page-portfolios", Scene::Portfolios),
        ("portfolio-page-overview", Scene::Overview),
        ("portfolio-page-holdings", Scene::Holdings),
        ("portfolio-page-transactions", Scene::Transactions),
        ("portfolio-page-statistics", Scene::Statistics),
    ] {
        both_themes(name, vec2(900.0, 520.0), move |ui| {
            portfolio_empty::page(ui, scene, |ui| {
                header_controls::prominent_button(ui, Icon::Plus, "Add Transaction");
                header_controls::prominent_button(ui, Icon::Download, "Import Transactions");
            });
        });
    }
    both_themes("portfolio-page-narrow", vec2(420.0, 640.0), |ui| {
        portfolio_empty::page(ui, Scene::Overview, |ui| {
            header_controls::prominent_button(ui, Icon::Plus, "Add Transaction");
        });
    });
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_holdings() {
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    let mut with_unpriced = holdings();
    let mut unpriced = holding("PEPE", "Pepe", 0.0, 120.0, 0.0);
    unpriced.current_price = None;
    unpriced.previous_day_price = None;
    unpriced.current_value = None;
    unpriced.unrealized_pnl = None;
    with_unpriced.push(unpriced);
    let subtitle = |asset: &PortfolioAsset| asset.name.clone();
    for (name, size, list, loading, search) in [
        (
            "portfolio-holdings-1100",
            vec2(1100.0, 640.0),
            &with_unpriced,
            false,
            "",
        ),
        (
            "portfolio-holdings-1480",
            vec2(1480.0, 560.0),
            &with_unpriced,
            false,
            "",
        ),
        (
            "portfolio-holdings-760",
            vec2(760.0, 640.0),
            &with_unpriced,
            false,
            "",
        ),
        (
            "portfolio-holdings-loading",
            vec2(1100.0, 460.0),
            &with_unpriced,
            true,
            "",
        ),
        (
            "portfolio-holdings-nomatch",
            vec2(1100.0, 460.0),
            &with_unpriced,
            false,
            "zzz",
        ),
        (
            "portfolio-holdings-empty",
            vec2(1100.0, 460.0),
            &Vec::new(),
            false,
            "",
        ),
    ] {
        both_themes(name, size, |ui| {
            let mut state = HoldingsState::default();
            state.search = search.to_owned();
            portfolio_holdings::show(
                ui,
                &mut state,
                &HoldingsInput {
                    holdings: list,
                    saved_sort: crate::model::portfolio::PortfolioHoldingsSort::CurrentValue,
                    formatter: PortfolioValueFormatter::new(PortfolioCurrency::Usd, false),
                    is_loading: loading,
                    subtitle: &subtitle,
                },
            );
        });
    }
}

fn ledger() -> Vec<crate::model::portfolio::PortfolioTransaction> {
    use crate::model::portfolio::{
        PortfolioSource, PortfolioTransaction, PortfolioTransactionType,
    };
    let kinds = [
        (
            PortfolioTransactionType::Buy,
            "BTC",
            0.25,
            31_200.0,
            "Weekly DCA",
        ),
        (PortfolioTransactionType::Buy, "ETH", 2.0, 1_700.0, ""),
        (
            PortfolioTransactionType::Sell,
            "SOL",
            14.0,
            140.0,
            "Took profit",
        ),
        (
            PortfolioTransactionType::StakingReward,
            "ETH",
            0.012,
            1_850.0,
            "",
        ),
        (
            PortfolioTransactionType::TransferIn,
            "LINK",
            120.0,
            14.5,
            "From cold wallet",
        ),
        (PortfolioTransactionType::Buy, "BTC", 0.1, 30_100.0, ""),
        (PortfolioTransactionType::Airdrop, "ARB", 400.0, 1.2, ""),
        (PortfolioTransactionType::Sell, "DOGE", 2_500.0, 0.09, ""),
        (PortfolioTransactionType::Buy, "SOL", 20.0, 98.0, "Dip"),
        (PortfolioTransactionType::Fee, "ETH", 0.002, 1_900.0, "Gas"),
        (PortfolioTransactionType::Buy, "LINK", 60.0, 12.8, ""),
        (
            PortfolioTransactionType::Interest,
            "BTC",
            0.0004,
            32_000.0,
            "",
        ),
    ];
    kinds
        .iter()
        .enumerate()
        .map(
            |(index, (kind, ticker, quantity, price, notes))| PortfolioTransaction {
                id: index as u64 + 1,
                portfolio_id: 1,
                asset: holding(
                    ticker,
                    match *ticker {
                        "BTC" => "Bitcoin",
                        "ETH" => "Ethereum",
                        "SOL" => "Solana",
                        "LINK" => "Chainlink",
                        "ARB" => "Arbitrum",
                        _ => "Dogecoin",
                    },
                    *price,
                    *price,
                    0.0,
                )
                .asset,
                kind: *kind,
                quantity: *quantity,
                price: Some(*price),
                price_currency: PortfolioCurrency::Usd,
                fee: if *kind == PortfolioTransactionType::Buy {
                    4.5
                } else {
                    0.0
                },
                fee_currency: PortfolioCurrency::Usd,
                timestamp: NOW - (index as i64 + 1) * 9 * DAY + index as i64 * 3_700_000,
                notes: (*notes).to_owned(),
                source: PortfolioSource::Manual,
                external_transaction_id: None,
                created_at: NOW,
                updated_at: NOW,
            },
        )
        .collect()
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_transactions() {
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    let full = ledger();
    let subtitle = |asset: &PortfolioAsset| asset.name.clone();
    // The same ledger ten times over, so the footer has pages to offer.
    let long: Vec<_> = (0..10u64)
        .flat_map(|copy| {
            full.iter().cloned().map(move |mut tx| {
                tx.id += copy * 100;
                tx.timestamp -= copy as i64 * 120 * DAY;
                tx
            })
        })
        .collect();
    let projection = {
        let long = long.clone();
        move |id: u64| {
            long.iter().find(|tx| tx.id == id).map(|tx| Projection {
                price: tx.price,
                fee: tx.fee,
            })
        }
    };
    for (name, size, list, search) in [
        (
            "portfolio-transactions-1100",
            vec2(1100.0, 760.0),
            &full,
            "",
        ),
        (
            "portfolio-transactions-1480",
            vec2(1480.0, 620.0),
            &full,
            "",
        ),
        ("portfolio-transactions-760", vec2(760.0, 760.0), &full, ""),
        (
            "portfolio-transactions-paged",
            vec2(1100.0, 760.0),
            &long,
            "",
        ),
        (
            "portfolio-transactions-paged-760",
            vec2(760.0, 760.0),
            &long,
            "",
        ),
        (
            "portfolio-transactions-nomatch",
            vec2(1100.0, 460.0),
            &full,
            "zzz",
        ),
        (
            "portfolio-transactions-empty",
            vec2(1100.0, 460.0),
            &Vec::new(),
            "",
        ),
    ] {
        both_themes(name, size, |ui| {
            let mut state = TransactionsState::default();
            state.filters.search = search.to_owned();
            portfolio_transactions::show(
                ui,
                &mut state,
                &TransactionsInput {
                    ledger: list,
                    projection: &projection,
                    subtitle: &subtitle,
                    formatter: PortfolioValueFormatter::new(PortfolioCurrency::Usd, false),
                    utc_offset_secs: 0,
                },
            );
        });
    }
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_editor() {
    use crate::ui::portfolio_transaction_editor::{EditorMode, TransactionEditorState, show};
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    for (name, index, mode) in [
        ("portfolio-editor-edit", 0, EditorMode::Edit),
        ("portfolio-editor-sell", 2, EditorMode::Edit),
        ("portfolio-editor-income", 3, EditorMode::Duplicate),
    ] {
        let state = std::cell::RefCell::new(TransactionEditorState::new(
            mode,
            ledger()[index].clone(),
            0,
        ));
        state.borrow_mut().set_market_price(31_450.0);
        both_themes(name, vec2(700.0, 900.0), |ui| {
            show(ui.ctx(), &mut state.borrow_mut());
        });
    }
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_asset_picker() {
    use crate::ui::portfolio_asset_picker::{AssetPickerState, PickerTab, show};
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    for (name, tab) in [
        ("portfolio-picker-crypto", PickerTab::Crypto),
        ("portfolio-picker-stocks", PickerTab::Stocks),
    ] {
        let state = std::cell::RefCell::new(AssetPickerState::new(
            "Add Asset",
            "Choose the asset this transaction is for.",
            "Continue",
        ));
        state.borrow_mut().select_tab(tab);
        let store: crate::store::SharedStore = std::sync::Arc::new(std::sync::Mutex::new(
            crate::store::Store::open_in_memory().expect("open store"),
        ));
        {
            let mut recents = Vec::new();
            for (symbol, full, source) in [
                ("BTC/USDT", "BTCUSDT", DataSource::Binance),
                ("ETH/USDT", "ETHUSDT", DataSource::Binance),
                ("AAPL", "AAPL", DataSource::Alpaca),
                ("SOL/USDT", "SOLUSDT", DataSource::Binance),
            ] {
                crate::ui::add_chart_sheet::record_recent(
                    &mut recents,
                    &crate::service::SearchResult {
                        symbol: symbol.into(),
                        full_symbol: full.into(),
                        source,
                        price: None,
                        event: None,
                    },
                );
            }
            crate::ui::add_chart_sheet::store_recents(&store.lock().unwrap(), &recents);
        }
        both_themes(name, vec2(1000.0, 720.0), |ui| {
            show(ui.ctx(), &mut state.borrow_mut(), &store);
        });
    }
}
