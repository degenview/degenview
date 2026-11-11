//! Pictures of the Portfolio pages at several window sizes, for a person to look at. Run by hand:
//! `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib portfolio_gallery -- --ignored`.

use crate::model::DataSource;
use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::model::portfolio::{
    PortfolioAsset, PortfolioCurrency, PortfolioHolding, PortfolioSnapshot,
};
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
            net_contributions: 0.0,
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
                    net_contributions: 0.0,
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
                    closed: &[],
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
                        base: None,
                        quote: None,
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

const CMC_FILE: &str = "Date (UTC),Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes\n\
    2024-03-05 14:30:00,BTC,Buy,60000,0.5,30000,--,,\n\
    2024-03-06 10:00:00,ETH,Buy,3000,2,6000,1.5,EUR,\n\
    2024-03-07 10:00:00,ETH,Sell,3000,1,3000,0,,\n\
    2024-03-08 10:00:00,DOGE,Buy,0.1,100,10,0,,\n\
    2024-03-09 10:00:00,SOL,Buy,140,10,1400,0,,\n\
    2024-03-10 10:00:00,PEPE,Buy,0.00001,1000000,10,0,,\n\
    2024-03-11 10:00:00,ARB,Buy,1.2,400,480,2,GBP,";

const CMC_SMALL: &str = "Date (UTC),Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes\n\
    2024-03-05 14:30:00,BTC,Buy,60000,0.5,30000,--,,\n\
    2024-03-06 10:00:00,ETH,Buy,3000,2,6000,1.5,EUR,\n\
    2024-03-11 10:00:00,ARB,Buy,1.2,400,480,2,GBP,";

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_cmc_import() {
    use crate::model::portfolio::csv::preview_coinmarketcap;
    use crate::ui::portfolio_cmc_import::{CmcImportInput, CmcImportState, show};
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    let asset = |ticker: &str, name: &str| {
        let mut asset = holding(ticker, name, 1.0, 1.0, 0.0).asset;
        asset.symbol = format!("{ticker}/USDT");
        asset
    };
    let preview = preview_coinmarketcap(CMC_FILE);
    assert!(preview.is_valid(), "{:?}", preview.errors);
    // Searching: nothing decided, the host still looking.
    let small = preview_coinmarketcap(CMC_SMALL);
    assert!(small.is_valid(), "{:?}", small.errors);
    let searching = || {
        let mut state = CmcImportState::default();
        for symbol in preview.symbols() {
            state.resolving.insert(symbol);
        }
        state
    };
    // Mixed: every kind of row, a fee rate that failed and one still loading.
    let mixed = |preview: &crate::model::portfolio::csv::CmcPreview| {
        let mut state = CmcImportState {
            did_auto_map: true,
            ..CmcImportState::default()
        };
        state.auto_mapped("BTC", asset("BTC", "Bitcoin"));
        state.set_mapping("ETH", asset("ETH", "Ethereum"));
        state.skip("SOL");
        state.resolving.insert("DOGE".into());
        state.is_resolving_fee_fx = false;
        let ids: Vec<String> = preview.rows.iter().map(|row| row.id.clone()).collect();
        state.fee_rate_resolved(&ids[1], Err("No rate found for EUR on Mar 6, 2024.".into()));
        state
    };
    // Ready: everything matched, the fee rates loaded.
    let ready = |preview: &crate::model::portfolio::csv::CmcPreview| {
        let mut state = CmcImportState {
            did_auto_map: true,
            ..CmcImportState::default()
        };
        for (ticker, name) in [
            ("BTC", "Bitcoin"),
            ("ETH", "Ethereum"),
            ("DOGE", "Dogecoin"),
            ("SOL", "Solana"),
            ("ARB", "Arbitrum"),
        ] {
            state.auto_mapped(ticker, asset(ticker, name));
        }
        state.skip("PEPE");
        for row in &preview.rows {
            if row.fee != 0.0 {
                state.fee_rate_resolved(&row.id, Ok(1.08));
            }
        }
        state
    };
    let loading_fees = |preview: &crate::model::portfolio::csv::CmcPreview| {
        let mut state = mixed(preview);
        state.fee_fx_errors.clear();
        state.is_resolving_fee_fx = true;
        state.fee_fx_progress = 0.45;
        state
    };
    let mut broken = small.clone();
    broken.errors = vec![
        "Line 4: unknown transaction type \"Swap\".".into(),
        "Line 9: the amount is missing.".into(),
    ];
    broken.warnings = vec!["PEPE has no market yet; its 1 transaction will be skipped.".into()];
    let cases: Vec<(&str, CmcImportState, &_)> = vec![
        ("portfolio-cmc-searching", searching(), &preview),
        ("portfolio-cmc-mixed", mixed(&preview), &preview),
        ("portfolio-cmc-fees-loading", loading_fees(&small), &small),
        ("portfolio-cmc-fees-failed", mixed(&small), &small),
        ("portfolio-cmc-ready", ready(&small), &small),
        ("portfolio-cmc-errors", mixed(&small), &broken),
    ];
    for (name, state, file) in cases {
        let state = std::cell::RefCell::new(state);
        both_themes(name, vec2(900.0, 780.0), |ui| {
            show(
                ui.ctx(),
                &mut state.borrow_mut(),
                &CmcImportInput {
                    preview: file,
                    portfolio_currency: PortfolioCurrency::Usd,
                },
            );
        });
    }
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_header_menus() {
    use crate::ui::header_controls::{self, open_for_gallery};
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    let currencies: Vec<String> = [
        ("USD", "US Dollar"),
        ("EUR", "Euro"),
        ("GBP", "British Pound"),
    ]
    .iter()
    .map(|(code, name)| format!("{code}  \u{00B7}  {name}"))
    .collect();
    let transfers = [
        "Import Transactions\u{2026}",
        "Import from CoinMarketCap\u{2026}",
        "Export Transactions\u{2026}",
        "Export Current Holdings\u{2026}",
        "Export Portfolio History\u{2026}",
    ];
    let portfolios = [
        "All Portfolios",
        "Main",
        "Create Portfolio\u{2026}",
        "Manage Portfolios\u{2026}",
    ];
    for (name, which) in [
        ("portfolio-menu-transfer", 0),
        ("portfolio-menu-currency", 1),
        ("portfolio-menu-selector", 2),
    ] {
        both_themes(name, vec2(700.0, 330.0), |ui| {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let ids = [
                    egui::Id::new("gallery-portfolio"),
                    egui::Id::new("gallery-currency"),
                    egui::Id::new("gallery-transfer"),
                ];
                open_for_gallery(ui.ctx(), ids[[2, 1, 0][which]]);
                header_controls::menu_button(
                    ui,
                    ids[0],
                    Icon::Briefcase,
                    "Main",
                    &portfolios,
                    false,
                    |ui| {
                        header_controls::choice(ui, "All Portfolios", false);
                        header_controls::separator(ui);
                        header_controls::choice(ui, "Main", true);
                        header_controls::separator(ui);
                        header_controls::action(ui, Icon::Plus, "Create Portfolio\u{2026}");
                        header_controls::action(ui, Icon::Sliders, "Manage Portfolios\u{2026}");
                    },
                );
                header_controls::menu_button(
                    ui,
                    ids[1],
                    Icon::Banknote,
                    "USD",
                    &currencies,
                    false,
                    |ui| {
                        for (index, label) in currencies.iter().enumerate() {
                            header_controls::choice(ui, label, index == 0);
                        }
                    },
                );
                header_controls::menu_button(
                    ui,
                    ids[2],
                    Icon::Exchange,
                    "Import or Export",
                    &transfers,
                    false,
                    |ui| {
                        for (index, label) in transfers.iter().enumerate() {
                            if index == 2 {
                                header_controls::separator(ui);
                            }
                            header_controls::action(ui, Icon::Download, label);
                        }
                    },
                );
            });
        });
    }
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_import_preview() {
    use crate::model::portfolio::csv::CsvPreview;
    use crate::ui::portfolio_import_preview::{ImportPreviewInput, ImportPreviewState, show};
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    let ready = CsvPreview {
        transactions: ledger(),
        ..CsvPreview::default()
    };
    let warned = CsvPreview {
        warnings: vec![
            "Line 7: the fee currency is missing; USD is assumed.".into(),
            "Line 11: the notes were cut to 200 characters.".into(),
        ],
        ..ready.clone()
    };
    let broken = CsvPreview {
        errors: vec![
            "Line 4: a Sell of 20 SOL exceeds the 14 SOL held on that date.".into(),
            "Line 9: the amount is missing.".into(),
        ],
        warnings: vec!["PEPE has no market yet; its 1 transaction was left out.".into()],
        ..ready.clone()
    };
    for (name, preview, importing, failure) in [
        ("portfolio-preview-ready", ready.clone(), false, None),
        ("portfolio-preview-warned", warned, false, None),
        ("portfolio-preview-errors", broken, false, None),
        ("portfolio-preview-importing", ready.clone(), true, None),
        (
            "portfolio-preview-failed",
            ready,
            false,
            Some("A sell on Mar 8, 2024 exceeds the holding."),
        ),
        (
            "portfolio-preview-empty",
            CsvPreview::default(),
            false,
            None,
        ),
    ] {
        let state = std::cell::RefCell::new(ImportPreviewState::default());
        state.borrow_mut().is_importing = importing;
        if let Some(message) = failure {
            state.borrow_mut().failed(message);
        }
        both_themes(name, vec2(900.0, 780.0), |ui| {
            show(
                ui.ctx(),
                &mut state.borrow_mut(),
                &ImportPreviewInput {
                    preview: &preview,
                    utc_offset_secs: 0,
                },
            );
        });
    }
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_manage() {
    use crate::model::portfolio::Portfolio;
    use crate::ui::portfolio_sheets::{ManageInput, ManageRow, ManageState, show_manage};
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    let mut portfolios: Vec<Portfolio> = ["Main", "Long-term holds", "Trading account", "DeFi"]
        .iter()
        .enumerate()
        .map(|(index, name)| Portfolio::new(index as u64 + 1, *name, 0))
        .collect();
    portfolios[1].base_currency = PortfolioCurrency::Eur;
    portfolios[3].is_archived = true;
    let counts = [(42, 6), (118, 11), (7, 2), (0, 0)];
    let rows: Vec<ManageRow> = portfolios
        .iter()
        .zip(counts)
        .map(|(portfolio, (transactions, assets))| ManageRow {
            portfolio,
            transactions,
            assets,
        })
        .collect();
    let many: Vec<Portfolio> = (0..9)
        .map(|index| Portfolio::new(index + 1, format!("Portfolio {}", index + 1), 0))
        .collect();
    let many_rows: Vec<ManageRow> = many
        .iter()
        .map(|portfolio| ManageRow {
            portfolio,
            transactions: 3,
            assets: 2,
        })
        .collect();
    for (name, list, current) in [
        ("portfolio-manage", &rows, Some(1)),
        ("portfolio-manage-many", &many_rows, None),
        ("portfolio-manage-empty", &Vec::new(), None),
    ] {
        let state = std::cell::RefCell::new(ManageState::default());
        both_themes(name, vec2(900.0, 780.0), |ui| {
            show_manage(
                ui.ctx(),
                &mut state.borrow_mut(),
                &ManageInput {
                    rows: list,
                    current,
                },
            );
        });
    }
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_create() {
    use crate::ui::portfolio_sheets::{CreateState, show_create};
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    let existing = vec!["Main".to_owned()];
    for (name, typed, currency) in [
        ("portfolio-create", "", PortfolioCurrency::Usd),
        (
            "portfolio-create-filled",
            "Long-term holds",
            PortfolioCurrency::Eur,
        ),
        ("portfolio-create-duplicate", "main", PortfolioCurrency::Usd),
    ] {
        let state = std::cell::RefCell::new(CreateState::default());
        state.borrow_mut().name = typed.to_owned();
        state.borrow_mut().currency = currency;
        both_themes(name, vec2(800.0, 820.0), |ui| {
            show_create(ui.ctx(), &mut state.borrow_mut(), &existing);
        });
    }
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_row_menus() {
    use crate::ui::portfolio_grid;
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    both_themes("portfolio-row-menus", vec2(600.0, 240.0), |ui| {
        let cell = egui::Rect::from_min_size(egui::pos2(300.0, 40.0), vec2(280.0, 40.0));
        let id = egui::Id::new("gallery-row-menu");
        egui::Popup::open_id(ui.ctx(), id.with("popup"));
        portfolio_grid::row_menu_button(ui, cell, id, "Actions", |ui| {
            portfolio_grid::fit_menu(ui, &["View Transactions", "Remap Asset\u{2026}"]);
            ui.spacing_mut().item_spacing.y = 0.0;
            crate::ui::kit::menu_item(ui, Icon::List, "View Transactions", true, false);
            crate::ui::kit::menu_item(ui, Icon::ArrowLeftRight, "Remap Asset\u{2026}", true, false);
        });
    });
}

#[test]
#[ignore = "writes PNGs for a person to look at"]
fn portfolio_gallery_asset_detail() {
    use crate::ui::portfolio_asset_detail::{AssetDetailInput, AssetDetailState, show};
    if crate::ui::render_tests::gallery_dir().is_none() {
        return;
    }
    let all = holdings();
    let holding = all[0].clone();
    let ledger = ledger();
    let mine: Vec<&crate::model::portfolio::PortfolioTransaction> = ledger
        .iter()
        .filter(|tx| tx.asset.display_ticker() == "BTC" || tx.asset.display_ticker() == "ETH")
        .collect();
    let price_of = |tx: &crate::model::portfolio::PortfolioTransaction| tx.price;
    let none: Vec<&crate::model::portfolio::PortfolioTransaction> = Vec::new();
    for (name, with_holding, rows) in [
        ("portfolio-detail", true, &mine),
        ("portfolio-detail-closed", false, &mine),
        ("portfolio-detail-empty", false, &none),
    ] {
        let state = std::cell::RefCell::new(AssetDetailState::default());
        both_themes(name, vec2(1000.0, 800.0), |ui| {
            show(
                ui.ctx(),
                &mut state.borrow_mut(),
                &AssetDetailInput {
                    holding: with_holding.then_some(&holding),
                    asset: Some(&holding.asset),
                    subtitle: &holding.asset.name,
                    transactions: rows,
                    price_of: &price_of,
                    formatter: PortfolioValueFormatter::new(PortfolioCurrency::Usd, false),
                    utc_offset_secs: 0,
                },
            );
        });
    }
}
