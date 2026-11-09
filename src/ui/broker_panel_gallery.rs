//! The live trading panel, its ticket and the Brokers settings page, drawn against the broker
//! crate's mock exchange. A smoke test always runs (every surface draws without panicking and puts
//! something on screen); the pictures are written by hand:
//! `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib broker_panel_gallery -- --ignored`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
use degenview_brokers::testing::conformance::Exchange as _;
use degenview_brokers::{
    BrokerId, ClientOrderId, Env, MemorySecretStore, OrderDraft, OrderKind, ProductId, Side,
    Status, TimeInForce,
};
use eframe::egui::{self, Vec2};

use crate::host::broker_host::{BrokerHost, StoreJournal};
use crate::model::broker_ticket::{TicketDraft, TicketKind};
use crate::store::Store;
use crate::ui::broker_panel::{self, BrokerPanelState, BrokerTab, PanelInput};
use crate::ui::broker_ticket::{self, TicketState};
use crate::ui::render_tests::Screen;
use crate::ui::{settings_brokers, theme};

const ID: BrokerId = BrokerId::new("coinbase");

struct Rig {
    exchange: CoinbaseExchange,
    host: Arc<BrokerHost>,
}

fn rig() -> Rig {
    let exchange = CoinbaseExchange::start();
    let store = Arc::new(Mutex::new(Store::open_in_memory().expect("store")));
    let env = Env::default()
        .with_loopback_base_url(exchange.rest.url())
        .with_loopback_ws_url(exchange.stream.url.clone())
        .with_stream_timing(degenview_brokers::StreamTiming::fast())
        .with_request_gap(Duration::from_millis(2));
    let host = BrokerHost::new(
        Arc::new(MemorySecretStore::new()),
        Arc::new(StoreJournal::new(store.clone())),
        Some(store),
        env,
        |config| config.fast(),
    );
    Rig { exchange, host }
}

fn wait(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn limit(side: Side, size: &str, price: &str) -> OrderDraft {
    OrderDraft {
        product: ProductId::new("BTC-USD"),
        side,
        kind: OrderKind::Limit {
            base_size: size.parse().unwrap(),
            price: price.parse().unwrap(),
            tif: TimeInForce::GoodTilCancelled,
            post_only: false,
        },
    }
}

/// A connected account holding a resting buy, a resting sell and a partly filled order.
fn live_rig() -> Rig {
    let rig = rig();
    rig.host
        .adopt(ID, credentials(&generate_key_pem()))
        .unwrap();
    wait("live", || {
        rig.host
            .snapshot(ID)
            .is_some_and(|s| s.status == Status::Live)
    });
    let runtime = rig.host.runtime(ID).unwrap();
    runtime
        .submit(limit(Side::Buy, "0.002", "89000.00"))
        .wait()
        .unwrap();
    runtime
        .submit(limit(Side::Sell, "0.0015", "93000.00"))
        .wait()
        .unwrap();
    let third = runtime
        .submit(limit(Side::Buy, "0.001", "88000.00"))
        .wait()
        .unwrap();
    rig.exchange
        .change_order(third.order_id.as_str(), "OPEN", "0.0004", true);
    wait("three orders and a fill", || {
        let snapshot = rig.host.snapshot(ID).unwrap();
        snapshot.working_orders().count() == 3 && !snapshot.fills.is_empty()
    });
    rig
}

fn frames(
    screen: &mut Screen,
    count: usize,
    mut body: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut last = None;
    for _ in 0..count {
        last = Some(screen.run(
            vec![egui::Event::PointerMoved(egui::pos2(600.0, 40.0))],
            |ui| {
                theme::sync(ui.ctx());
                body(ui);
            },
        ));
        std::thread::sleep(Duration::from_millis(15));
    }
    last.unwrap()
}

fn shapes(output: &egui::FullOutput) -> usize {
    output.shapes.len()
}

fn show_panel(
    screen: &mut Screen,
    state: &mut BrokerPanelState,
    host: &BrokerHost,
    height: f32,
) -> egui::FullOutput {
    frames(screen, 3, |ui| {
        egui::Panel::bottom(egui::Id::new("broker-gallery"))
            .exact_size(height)
            .frame(egui::Frame::new().fill(crate::ui::kit::window()))
            .show(ui, |ui| {
                broker_panel::ui(
                    ui,
                    state,
                    host,
                    ID,
                    &PanelInput {
                        market: Some(degenview_brokers::MarketRef {
                            source: "coinbase".to_owned(),
                            symbol: "BTC-USD".to_owned(),
                        }),
                    },
                );
            });
    })
}

#[test]
fn every_live_trading_surface_draws() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = live_rig();
    let mut screen = Screen::new(Vec2::new(1_100.0, 420.0));

    let mut state = BrokerPanelState::default();
    assert!(
        shapes(&show_panel(&mut screen, &mut state, &rig.host, 380.0)) > 40,
        "the orders tab"
    );
    for tab in [BrokerTab::History, BrokerTab::Fills, BrokerTab::Balances] {
        state.set_tab(tab);
        let output = show_panel(&mut screen, &mut state, &rig.host, 380.0);
        assert!(shapes(&output) > 20, "{tab:?}");
    }

    // The ticket.
    let mut ticket = TicketState::new(ID, ProductId::new("BTC-USD"), {
        let mut draft = TicketDraft::new(Side::Buy);
        draft.size_text = "0.001".to_owned();
        draft.price_text = "90000.00".to_owned();
        draft
    });
    let mut tall = Screen::new(Vec2::new(900.0, 900.0));
    let output = frames(&mut tall, 40, |ui| {
        let _ = broker_ticket::show(ui.ctx(), &mut ticket, &rig.host);
    });
    assert!(shapes(&output) > 40, "the ticket");

    // The Brokers settings page, with a connected card.
    let mut page_state = settings_brokers::BrokersState::default();
    let mut page = Screen::new(Vec2::new(600.0, 900.0));
    let output = frames(&mut page, 3, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            settings_brokers::body(ui, &mut page_state, Some(rig.host.clone()));
        });
    });
    assert!(shapes(&output) > 20, "the settings page");

    // And a broker that is not connected.
    let idle = self::rig();
    let output = frames(&mut page, 3, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            settings_brokers::body(
                ui,
                &mut settings_brokers::BrokersState::default(),
                Some(idle.host.clone()),
            );
        });
    });
    assert!(shapes(&output) > 20, "the page before connecting");

    // The panel for a broker that is not connected says how to connect.
    let mut state = BrokerPanelState::default();
    let output = show_panel(&mut screen, &mut state, &idle.host, 380.0);
    assert!(shapes(&output) > 5);
    let _ = ClientOrderId::new("x");
    let _ = TicketKind::Limit;
}

fn key(key: egui::Key) -> Vec<egui::Event> {
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        })
        .collect()
}

/// The whole path through the screen: fill in an order, review it, see that nothing has been sent,
/// approve it with the keyboard, and find the order at the exchange and in the panel.
#[test]
fn an_order_is_confirmed_before_it_is_sent_and_then_appears_everywhere() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.host
        .adopt(ID, credentials(&generate_key_pem()))
        .unwrap();
    wait("live", || {
        rig.host
            .snapshot(ID)
            .is_some_and(|s| s.status == Status::Live)
    });
    let mut ticket = TicketState::new(ID, ProductId::new("BTC-USD"), {
        let mut draft = TicketDraft::new(Side::Buy);
        draft.size_text = "0.001".to_owned();
        draft.price_text = "90000.00".to_owned();
        draft
    });
    let mut screen = Screen::new(Vec2::new(900.0, 900.0));
    let mut run = |events: Vec<egui::Event>, ticket: &mut TicketState| {
        screen.run(events, |ui| {
            theme::sync(ui.ctx());
            let _ = broker_ticket::show(ui.ctx(), ticket, &rig.host);
        })
    };
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ticket.ready_to_review() {
        assert!(
            Instant::now() < deadline,
            "the broker's preview should arrive"
        );
        run(vec![], &mut ticket);
        std::thread::sleep(Duration::from_millis(20));
    }

    // Review: the order is shown for approval and nothing has been sent.
    ticket.press_review();
    run(vec![], &mut ticket);
    run(vec![], &mut ticket);
    assert!(
        ticket.confirming(),
        "pressing Review opens the confirmation"
    );
    assert_eq!(
        rig.exchange.order_posts(),
        0,
        "nothing is sent before approval"
    );
    assert_eq!(rig.host.snapshot(ID).unwrap().working_orders().count(), 0);

    // A stray Enter approves nothing: the dialog starts with no button focused.
    run(key(egui::Key::Enter), &mut ticket);
    assert!(ticket.confirming());
    assert_eq!(rig.exchange.order_posts(), 0);

    // Tab to "Back", Tab to "Place order", Enter.
    for events in [key(egui::Key::Tab), key(egui::Key::Tab)] {
        run(events, &mut ticket);
    }
    run(key(egui::Key::Enter), &mut ticket);
    let deadline = Instant::now() + Duration::from_secs(8);
    while rig.exchange.order_posts() == 0 {
        assert!(Instant::now() < deadline, "approving should send the order");
        run(vec![], &mut ticket);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(rig.exchange.order_posts(), 1, "sent exactly once");
    wait("the order in the panel's snapshot", || {
        rig.host
            .snapshot(ID)
            .is_some_and(|s| s.working_orders().count() == 1)
    });
    assert_eq!(rig.exchange.orders_created(), 1);
}

#[test]
fn cancelling_asks_first_for_many_and_a_requested_cancel_leaves_the_order_listed() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = live_rig();
    let snapshot = rig.host.snapshot(ID).unwrap();
    let ids: Vec<_> = snapshot
        .working_orders()
        .map(|v| v.order.id.clone())
        .collect();
    assert_eq!(ids.len(), 3);
    let cancels = |rig: &Rig| {
        rig.exchange.rest.with_state(|s| {
            s.requests_seen
                .iter()
                .filter(|r| r.contains("batch_cancel"))
                .count()
        })
    };

    let mut state = BrokerPanelState::default();
    // Two orders: always asks, and nothing is sent until the user says so.
    state.request_cancel_for_test(&rig.host, ID, ids[..2].to_vec(), false);
    assert!(state.cancel_ask_open());
    assert_eq!(cancels(&rig), 0);
    // One order with the question turned off: sent at once, and the order stays until the broker says.
    let mut single = BrokerPanelState::default();
    single.request_cancel_for_test(&rig.host, ID, vec![ids[2].clone()], false);
    assert!(!single.cancel_ask_open());
    wait("the cancel to reach the exchange", || cancels(&rig) == 1);
    wait("the broker's answer", || {
        rig.host.snapshot(ID).unwrap().orders.iter().any(|v| {
            v.order.id == ids[2] && v.order.status == degenview_brokers::OrderStatus::CancelQueued
        })
    });
    let listed = rig.host.snapshot(ID).unwrap();
    assert!(
        listed.orders.iter().any(|v| v.order.id == ids[2]),
        "still listed until the broker confirms"
    );
    assert_eq!(
        listed.working_orders().count(),
        3,
        "all three are still working orders"
    );
}

#[test]
fn an_edit_is_previewed_confirmed_and_sent_as_an_edit_never_a_cancel_and_replace() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = live_rig();
    let snapshot = rig.host.snapshot(ID).unwrap();
    let order = snapshot
        .working_orders()
        .find(|v| v.order.side == Side::Sell)
        .map(|v| v.order.clone())
        .unwrap();
    let mut state = BrokerPanelState::default();
    state.begin_edit(ID, order.clone(), Some("92500.00".parse().unwrap()));
    let mut screen = Screen::new(Vec2::new(900.0, 700.0));
    let mut run = |events: Vec<egui::Event>, state: &mut BrokerPanelState| {
        screen.run(events, |ui| {
            theme::sync(ui.ctx());
            egui::Panel::bottom(egui::Id::new("edit-gallery"))
                .exact_size(300.0)
                .show(ui, |ui| {
                    broker_panel::ui(ui, state, &rig.host, ID, &PanelInput { market: None });
                });
        })
    };
    let deadline = Instant::now() + Duration::from_secs(8);
    while !state.edit_ready() {
        assert!(
            Instant::now() < deadline,
            "the broker's preview of the edit should arrive"
        );
        run(vec![], &mut state);
        std::thread::sleep(Duration::from_millis(20));
    }
    state.press_edit_review();
    run(vec![], &mut state);
    run(vec![], &mut state);
    assert!(state.edit_confirming(), "the change is shown for approval");
    let sent = |rig: &Rig| {
        rig.exchange.rest.with_state(|s| {
            s.requests_seen
                .iter()
                .filter(|r| r.ends_with("/orders/edit"))
                .count()
        })
    };
    assert_eq!(sent(&rig), 0, "nothing is changed before approval");
    for events in [key(egui::Key::Tab), key(egui::Key::Tab)] {
        run(events, &mut state);
    }
    run(key(egui::Key::Enter), &mut state);
    wait("the edit to reach the exchange", || sent(&rig) == 1);
    wait("the sheet to close", || {
        run(vec![], &mut state);
        !state.edit_open()
    });
    let seen = rig.exchange.rest.with_state(|s| s.requests_seen.clone());
    assert!(
        !seen.iter().any(|r| r.contains("batch_cancel")),
        "an edit never cancels"
    );
    assert_eq!(
        seen.iter()
            .filter(|r| r.as_str() == "POST /api/v3/brokerage/orders")
            .count(),
        3,
        "and never places a new order"
    );
    let body = rig
        .exchange
        .rest
        .server
        .requests()
        .into_iter()
        .find(|r| r.path().ends_with("/orders/edit"))
        .unwrap()
        .body;
    assert!(body.contains("\"price\":\"92500.00\""), "{body}");
}

#[test]
#[ignore = "writes pictures"]
fn gallery_of_live_trading() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = live_rig();
    for light in [false, true] {
        let suffix = if light { "light" } else { "dark" };
        let theme_pref = if light {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        };

        let mut screen = Screen::new(Vec2::new(1_100.0, 400.0));
        screen.ctx.set_theme(theme_pref);
        let mut state = BrokerPanelState::default();
        let output = show_panel(&mut screen, &mut state, &rig.host, 380.0);
        screen.save(&format!("live-panel-orders-{suffix}"), output);
        for (name, tab) in [
            ("history", BrokerTab::History),
            ("fills", BrokerTab::Fills),
            ("balances", BrokerTab::Balances),
        ] {
            state.set_tab(tab);
            std::thread::sleep(Duration::from_millis(200));
            let output = show_panel(&mut screen, &mut state, &rig.host, 380.0);
            let output = if tab == BrokerTab::History {
                std::thread::sleep(Duration::from_millis(300));
                show_panel(&mut screen, &mut state, &rig.host, 380.0)
            } else {
                output
            };
            screen.save(&format!("live-panel-{name}-{suffix}"), output);
        }

        let mut tall = Screen::new(Vec2::new(720.0, 880.0));
        tall.ctx.set_theme(theme_pref);
        let mut ticket = TicketState::new(ID, ProductId::new("BTC-USD"), {
            let mut draft = TicketDraft::new(Side::Buy);
            draft.size_text = "0.001".to_owned();
            draft.price_text = "90000.00".to_owned();
            draft
        });
        let output = frames(&mut tall, 40, |ui| {
            let _ = broker_ticket::show(ui.ctx(), &mut ticket, &rig.host);
        });
        tall.save(&format!("live-ticket-{suffix}"), output);

        let mut problem = TicketState::new(ID, ProductId::new("BTC-USD"), {
            let mut draft = TicketDraft::new(Side::Buy);
            draft.size_text = "50".to_owned();
            draft.price_text = "90000.005".to_owned();
            draft
        });
        let output = frames(&mut tall, 40, |ui| {
            let _ = broker_ticket::show(ui.ctx(), &mut problem, &rig.host);
        });
        tall.save(&format!("live-ticket-problems-{suffix}"), output);

        let mut page_state = settings_brokers::BrokersState::default();
        let mut page = Screen::new(Vec2::new(620.0, 980.0));
        page.ctx.set_theme(theme_pref);
        let output = frames(&mut page, 3, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                settings_brokers::body(ui, &mut page_state, Some(rig.host.clone()));
            });
        });
        page.save(&format!("live-settings-connected-{suffix}"), output);
        let idle = self::rig();
        let mut idle_state = settings_brokers::BrokersState::default();
        let output = frames(&mut page, 3, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                settings_brokers::body(ui, &mut idle_state, Some(idle.host.clone()));
            });
        });
        page.save(&format!("live-settings-idle-{suffix}"), output);
    }
}
