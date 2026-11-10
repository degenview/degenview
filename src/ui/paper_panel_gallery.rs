//! The Paper Trading panel as pictures, composed the way `PaperPanel::ui` composes it. Run by
//! hand: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib paper_panel_gallery -- --ignored`.

use crate::model::DataSource;
use crate::model::paper::{
    PaperAccountMetrics, PaperCurrency, PaperInstrument, PaperOrder, PaperOrderRole,
    PaperOrderSide, PaperOrderStatus, PaperOrderType, PaperPosition,
};
use crate::ui::paper_header::{
    self, AccountChoice, PaperFooterState, PaperHeaderInput, PaperHeaderState,
};
use crate::ui::paper_tables::{
    self, Context, FooterFacts, OrdersInput, PaperTab, PositionsInput, PositionsState,
};
use crate::ui::render_tests::Screen;
use crate::ui::theme;
use eframe::egui::{self, Vec2};

fn instrument(symbol: &str, name: &str, source: DataSource, tick: f64) -> PaperInstrument {
    let mut instrument = PaperInstrument::chart(symbol, name, source);
    instrument.tick_size = tick;
    instrument.quantity_increment = 0.001;
    instrument
}

fn position(symbol: &str, source: DataSource, quantity: f64, entry: f64) -> PaperPosition {
    PaperPosition {
        account_id: 1,
        instrument: instrument(symbol, symbol, source, 0.01),
        signed_quantity: quantity,
        average_entry_price: entry,
        realized_gross_pnl: 12.5,
        commissions: 1.2,
        opened_at: 0,
        updated_at: 0,
    }
}

fn order(id: u64, symbol: &str, side: PaperOrderSide, kind: PaperOrderType) -> PaperOrder {
    PaperOrder {
        id,
        account_id: 1,
        instrument: instrument(symbol, symbol, DataSource::Binance, 0.01),
        side,
        kind,
        original_quantity: 2.0,
        filled_quantity: 0.5,
        average_fill_price: None,
        limit_price: Some(61_250.0),
        stop_price: None,
        time_in_force: Default::default(),
        status: PaperOrderStatus::Working,
        role: PaperOrderRole::Entry,
        parent_order_id: None,
        oco_group_id: None,
        stop_triggered: false,
        reserved_margin: 0.0,
        rejection_reason: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn metrics() -> PaperAccountMetrics {
    PaperAccountMetrics {
        balance: 10_000.0,
        equity: 10_120.0,
        realized_pnl: 50.0,
        unrealized_pnl: 120.0,
        position_margin: 800.0,
        orders_margin: 200.0,
        available_funds: 9_120.0,
        margin_buffer: 0.9,
    }
}

fn divider(ui: &mut egui::Ui) {
    let (line, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, theme::separator());
}

#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_panel() {
    let _restore = theme::RestoreDefaultTheme;
    let accounts = [
        AccountChoice {
            id: 1,
            name: "Main".into(),
        },
        AccountChoice {
            id: 2,
            name: "Swing".into(),
        },
    ];
    let positions = vec![
        position("BTCUSDT", DataSource::Binance, 0.25, 61_000.0),
        position("ETHUSDT", DataSource::Binance, -3.0, 3_050.0),
        position("AAPL", DataSource::Alpaca, 40.0, 188.2),
    ];
    let orders = vec![
        order(1, "BTCUSDT", PaperOrderSide::Buy, PaperOrderType::Limit),
        order(2, "ETHUSDT", PaperOrderSide::Sell, PaperOrderType::Stop),
    ];
    // `click` is a point to press and release on, to open the menu there.
    let cases = [
        (
            "positions",
            PaperTab::Positions,
            1_240.0,
            300.0,
            false,
            None,
        ),
        ("orders", PaperTab::Orders, 1_240.0, 300.0, false, None),
        ("empty", PaperTab::Orders, 1_240.0, 300.0, true, None),
        ("compact", PaperTab::Positions, 760.0, 340.0, false, None),
        (
            "menu-account",
            PaperTab::Positions,
            1_240.0,
            300.0,
            false,
            Some(egui::pos2(130.0, 24.0)),
        ),
        (
            "menu-more",
            PaperTab::Positions,
            1_240.0,
            300.0,
            false,
            Some(egui::pos2(1_184.0, 24.0)),
        ),
        (
            "menu-row",
            PaperTab::Positions,
            1_240.0,
            300.0,
            false,
            Some(egui::pos2(1_204.0, 197.0)),
        ),
    ];
    for (name, tab, width, height, empty, click) in cases {
        for light in [false, true] {
            let mut screen = Screen::new(Vec2::new(width, height));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let mut header_state = PaperHeaderState { tab };
            let mut footer_state = PaperFooterState::default();
            let mut positions_state = PositionsState::default();
            let shown_positions: &[PaperPosition] = if empty { &[] } else { &positions };
            let shown_orders: &[PaperOrder] = if empty { &[] } else { &orders };
            let mut last = None;
            let hover = egui::pos2(width * 0.5, 150.0);
            for frame in 0..8 {
                let button = |at, pressed| egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                let events = match (click, frame) {
                    (Some(at), 1..=2) => vec![egui::Event::PointerMoved(at)],
                    (Some(at), 3) => vec![button(at, true)],
                    (Some(at), 4) => vec![button(at, false)],
                    (Some(_), _) => vec![],
                    (None, _) => vec![egui::Event::PointerMoved(hover)],
                };
                last = Some(screen.run(events, |ui| {
                    theme::sync(ui.ctx());
                    let context = Context {
                        currency: PaperCurrency::Usd,
                        utc_offset_secs: 0,
                    };
                    egui::Panel::bottom(egui::Id::new("gallery-panel"))
                        .exact_size(height)
                        .frame(egui::Frame::new().fill(crate::ui::kit::window()))
                        .show(ui, |ui| {
                            paper_header::header(
                                ui,
                                &mut header_state,
                                &PaperHeaderInput {
                                    accounts: &accounts,
                                    selected: Some(1),
                                    currency: Some(PaperCurrency::Usd),
                                    position_count: shown_positions.len(),
                                    working_order_count: shown_orders.len(),
                                    show_trading_on_charts: true,
                                },
                            );
                            divider(ui);
                            paper_tables::metrics_strip(ui, Some(&metrics()), PaperCurrency::Usd);
                            divider(ui);
                            match tab {
                                PaperTab::Orders => {
                                    paper_tables::orders_table(
                                        ui,
                                        &OrdersInput {
                                            orders: shown_orders,
                                            ctx: context,
                                        },
                                    );
                                }
                                _ => {
                                    paper_tables::positions_table(
                                        ui,
                                        &mut positions_state,
                                        &PositionsInput {
                                            positions: shown_positions,
                                            ctx: context,
                                            mark: &|p| Some(p.average_entry_price * 1.01),
                                            unrealized_pnl: &|p| {
                                                p.signed_quantity * p.average_entry_price * 0.01
                                            },
                                            return_ratio: &|_| Some(0.01),
                                        },
                                    );
                                }
                            }
                            paper_header::footer(
                                ui,
                                &mut footer_state,
                                tab,
                                &FooterFacts {
                                    positions: shown_positions,
                                    unrealized_total: 120.0,
                                    working_orders: shown_orders.len(),
                                    events: 0,
                                    trades: &[],
                                    journal_entries: 0,
                                    currency: PaperCurrency::Usd,
                                },
                            );
                        });
                }));
            }
            screen.save(
                &format!("paper-{name}-{}", if light { "light" } else { "dark" }),
                last.expect("ran"),
            );
        }
    }
}

#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_sheets() {
    use crate::ui::paper_account_sheet::{self, PaperAccountSheetState};
    let _restore = theme::RestoreDefaultTheme;
    for light in [false, true] {
        let tag = if light { "light" } else { "dark" };
        let theme_pref = if light {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        };
        for (name, kind) in [("create", 0), ("reset", 3)] {
            let mut screen = Screen::new(Vec2::new(760.0, 900.0));
            screen.ctx.set_theme(theme_pref);
            let mut sheet = if kind == 3 {
                let mut account = crate::model::paper::PaperAccount::new(3, "Swing", 0);
                account.initial_balance = 25_000.5;
                PaperAccountSheetState::reset(&account)
            } else {
                PaperAccountSheetState::create()
            };
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(vec![], |ui| {
                    theme::sync(ui.ctx());
                    ui.painter()
                        .rect_filled(ui.max_rect(), 0.0, theme::background());
                    let ctx = ui.ctx().clone();
                    paper_account_sheet::show(&ctx, &mut sheet);
                }));
            }
            screen.save(&format!("sheet-{name}-{tag}"), last.expect("ran"));
        }
    }
}
