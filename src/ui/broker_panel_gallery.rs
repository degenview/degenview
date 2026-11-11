//! The live trading panel, its ticket and the Brokers settings page, drawn against the broker
//! crate's mock exchange. A smoke test always runs (every surface draws without panicking and puts
//! something on screen); the pictures are written by hand:
//! `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib broker_panel_gallery -- --ignored`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
use degenview_brokers::testing::conformance::Exchange as _;
use degenview_brokers::{
    BrokerId, Env, MemorySecretStore, OrderDraft, OrderKind, ProductId, Side, Status, TimeInForce,
};
use eframe::egui::{self, Vec2};

use crate::host::broker_host::{BrokerHost, StoreJournal};
use crate::store::Store;
use crate::ui::broker_panel::{self, BrokerPanelState, BrokerTab};
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

/// The panel as the workspace draws it: the shared header's venue and the account strip.
fn draw(
    ui: &mut egui::Ui,
    state: &mut BrokerPanelState,
    host: &Arc<BrokerHost>,
) -> Vec<crate::ui::trade::panel_header::PanelEvent> {
    use crate::trading::backend::TradingBackend;
    let broker = degenview_brokers::registry::by_id(ID).expect("coinbase");
    let stats = crate::trading::broker_backend::BrokerBackend::new(host.clone(), broker).stats();
    let bar = crate::ui::trade::panel_header::sample_bar(
        crate::model::trading_venue::TradingVenue::Broker(ID),
        crate::trading::model::VenueKind::Live,
        "Coinbase",
    );
    broker_panel::ui(ui, state, host, ID, &bar, stats.as_ref())
}

fn show_panel(
    screen: &mut Screen,
    state: &mut BrokerPanelState,
    host: &Arc<BrokerHost>,
    height: f32,
) -> egui::FullOutput {
    frames(screen, 3, |ui| {
        egui::Panel::bottom(egui::Id::new("broker-gallery"))
            .exact_size(height)
            .frame(egui::Frame::new().fill(crate::ui::kit::window()))
            .show(ui, |ui| {
                draw(ui, state, host);
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
                    draw(ui, state, &rig.host);
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

        let idle = self::rig();
        for (name, width, height, connected, open) in [
            ("connected", 620.0, 980.0, true, true),
            ("idle", 620.0, 980.0, false, true),
            ("folded", 620.0, 300.0, true, false),
            ("narrow", 300.0, 1500.0, true, true),
            ("narrow-idle", 300.0, 1500.0, false, true),
        ] {
            let host = if connected {
                rig.host.clone()
            } else {
                idle.host.clone()
            };
            let mut page_state = settings_brokers::BrokersState::default();
            if open {
                page_state.open_for_test("coinbase");
                page_state.open_for_test("coinbase-sandbox");
            }
            let mut page = Screen::new(Vec2::new(width, height));
            page.ctx.set_theme(theme_pref);
            // The logos are images: they decode on a loader's thread, over a few frames.
            egui_extras::install_image_loaders(&page.ctx);
            let mut draw = |page: &mut Screen| {
                frames(page, 3, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        settings_brokers::body(ui, &mut page_state, Some(host.clone()));
                    });
                })
            };
            draw(&mut page);
            std::thread::sleep(Duration::from_millis(400));
            let output = draw(&mut page);
            page.save(&format!("live-settings-{name}-{suffix}"), output);
        }
    }

    // The record of unconfirmed orders cannot be read: the panel says so and orders are paused.
    let broken = live_rig();
    broken.host.corrupt_pending_record_for_test();
    wait("the unreadable record to be noticed", || {
        broken
            .host
            .snapshot(ID)
            .is_some_and(|s| s.journal_error.is_some())
    });
    let mut screen = Screen::new(Vec2::new(1_100.0, 400.0));
    let mut state = BrokerPanelState::default();
    let output = show_panel(&mut screen, &mut state, &broken.host, 380.0);
    screen.save("live-panel-journal-unreadable-dark", output);
}

fn posts(rig: &Rig, suffix: &str) -> usize {
    rig.exchange.rest.with_state(|s| {
        s.requests_seen
            .iter()
            .filter(|r| r.starts_with("POST") && r.ends_with(suffix))
            .count()
    })
}

#[test]
fn an_edit_cannot_be_reviewed_before_or_without_the_brokers_preview() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = live_rig();
    let snapshot = rig.host.snapshot(ID).unwrap();
    let order = snapshot
        .working_orders()
        .find(|v| v.order.side == Side::Sell)
        .map(|v| v.order.clone())
        .unwrap();
    rig.exchange.rest.with_state(|s| s.fail_preview = true);
    let mut state = BrokerPanelState::default();
    state.begin_edit(ID, order, Some("92500.00".parse().unwrap()));
    let mut screen = Screen::new(Vec2::new(900.0, 700.0));
    let mut run = |events: Vec<egui::Event>, state: &mut BrokerPanelState| {
        screen.run(events, |ui| {
            theme::sync(ui.ctx());
            egui::Panel::bottom(egui::Id::new("edit-gate"))
                .exact_size(300.0)
                .show(ui, |ui| {
                    draw(ui, state, &rig.host);
                });
        })
    };
    // Before any answer exists (the preview is asked for after typing pauses): not reviewable.
    run(vec![], &mut state);
    assert!(
        !state.edit_review_enabled(),
        "the Review button is off before the preview"
    );
    state.press_edit_review();
    run(vec![], &mut state);
    assert!(!state.edit_confirming(), "no review before the preview");

    // The preview fails: still not reviewable, and the edit is never sent.
    let deadline = Instant::now() + Duration::from_secs(8);
    while !state.edit_preview_failed() {
        assert!(Instant::now() < deadline, "the preview should fail");
        run(vec![], &mut state);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !state.edit_review_enabled(),
        "the Review button is off when the broker could not preview"
    );
    state.press_edit_review();
    run(vec![], &mut state);
    run(vec![], &mut state);
    assert!(
        !state.edit_confirming(),
        "no review without the broker's answer"
    );
    assert_eq!(posts(&rig, "/orders/edit"), 0, "nothing was sent");

    // Once the broker can preview and is asked again, the review opens.
    rig.exchange.rest.with_state(|s| s.fail_preview = false);
    state.retry_edit_preview();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !state.edit_ready() {
        assert!(
            Instant::now() < deadline,
            "the second preview should arrive"
        );
        run(vec![], &mut state);
        std::thread::sleep(Duration::from_millis(20));
    }
    run(vec![], &mut state);
    assert!(state.edit_review_enabled(), "and on again once it did");
    state.press_edit_review();
    run(vec![], &mut state);
    run(vec![], &mut state);
    assert!(state.edit_confirming());
    assert_eq!(posts(&rig, "/orders/edit"), 0, "approval is still to come");
}

/// Typing a key and pressing Connect must never stall a frame: the check and the save happen off
/// the interface thread or are quick, and the page after connecting draws at once.
#[test]
fn connecting_a_key_from_the_settings_page_never_stalls_a_frame() {
    let rig = rig();
    let broker = degenview_brokers::registry::by_id(ID).expect("coinbase");
    let key = credentials(&generate_key_pem());
    let mut state = settings_brokers::BrokersState::default();
    let mut screen = Screen::new(Vec2::new(600.0, 900.0));
    let mut slowest = Duration::ZERO;
    let mut draw = |state: &mut settings_brokers::BrokersState| {
        let start = Instant::now();
        frames(&mut screen, 1, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                settings_brokers::body(ui, state, Some(rig.host.clone()));
            });
        });
        slowest = slowest.max(start.elapsed() - Duration::from_millis(15));
    };
    draw(&mut state);
    state.connect_for_test(
        &rig.host,
        broker,
        &[
            ("name", key.get("name").unwrap()),
            ("private_key", key.get("private_key").unwrap()),
        ],
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        draw(&mut state);
        if rig
            .host
            .snapshot(ID)
            .is_some_and(|s| s.status == Status::Live)
        {
            break;
        }
    }
    for _ in 0..20 {
        draw(&mut state);
    }
    assert!(rig.host.snapshot(ID).is_some(), "connected");
    assert!(
        slowest < Duration::from_millis(250),
        "a frame took {slowest:?}"
    );
}

/// A base64 secret (Coinbase's Ed25519 form) typed or pasted into the secret box, character by
/// character and at once, draws every frame quickly.
#[test]
fn typing_a_base64_secret_never_stalls_a_frame() {
    let rig = rig();
    let broker = degenview_brokers::registry::by_id(ID).expect("coinbase");
    let label = broker.descriptor().credential_fields[1].label;
    let secret = "M".repeat(86) + "==";
    let mut state = settings_brokers::BrokersState::default();
    state.open_for_test("coinbase");
    let mut screen = Screen::new(Vec2::new(600.0, 900.0));
    let ctx = screen.ctx.clone();
    let slowest = std::cell::Cell::new(Duration::ZERO);
    let mut step = |events: Vec<egui::Event>, state: &mut settings_brokers::BrokersState| {
        let start = Instant::now();
        screen.run(events, |ui| {
            theme::sync(ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                settings_brokers::body(ui, state, Some(rig.host.clone()));
            });
        });
        slowest.set(slowest.get().max(start.elapsed()));
    };
    step(vec![], &mut state);
    let id = egui::Id::new(("broker-secret", label));
    // One by one, as a person types (focus is kept as long as the box stays).
    for (index, ch) in secret.chars().enumerate() {
        if index == 0 {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        step(vec![egui::Event::Text(ch.to_string())], &mut state);
    }
    for _ in 0..10 {
        step(vec![], &mut state);
    }
    assert_eq!(
        state.typed_for_test("coinbase", "private_key").as_deref(),
        Some(secret.as_str()),
        "every typed character reaches the form"
    );
    assert!(
        slowest.get() < Duration::from_millis(250),
        "a frame took {:?}",
        slowest.get()
    );
    // And all at once.
    let mut state = settings_brokers::BrokersState::default();
    state.open_for_test("coinbase");
    step(vec![], &mut state);
    ctx.memory_mut(|m| m.request_focus(id));
    step(vec![egui::Event::Paste(secret.clone())], &mut state);
    for _ in 0..10 {
        step(vec![], &mut state);
    }
    assert!(
        slowest.get() < Duration::from_millis(250),
        "a frame took {:?}",
        slowest.get()
    );
}

/// The header of a chart with live SELL and BUY pills and long prices: both pills are inside the
/// header and can be pressed, and the star stays at the right edge with the pills to its left.
#[test]
fn the_live_pills_can_be_pressed_and_the_star_stays_at_the_right_edge() {
    use crate::model::{DataSource, IndicatorConfig};
    use crate::ui::chart_card::{ChartCard, ChartCardAction};

    let _restore = theme::RestoreDefaultTheme;
    let width = 520.0;
    let mut card = ChartCard::default();
    card.live.order_entry = Some(Ok(()));
    card.live.broker_name = "Coinbase".to_owned();
    card.live.bid = Some("74,380.32".to_owned());
    card.live.ask = Some("74,396.74".to_owned());
    let mut screen = Screen::new(Vec2::new(width, 60.0));
    let indicators = IndicatorConfig::default();

    // Presses at `x` along the header's middle and reports what the header said.
    let mut press = |x: f32| -> ChartCardAction {
        let at = egui::pos2(x, 22.0);
        let mut said = ChartCardAction::default();
        for (events, last) in [
            (
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                false,
            ),
            (
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                true,
            ),
        ] {
            let mut action = ChartCardAction::default();
            screen.run(events, |ui| {
                theme::sync(ui.ctx());
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        card.header(
                            ui,
                            "BTC-USD",
                            DataSource::Coinbase,
                            &indicators,
                            &[],
                            false,
                            false,
                            &[],
                            &mut action,
                            1,
                        );
                    });
            });
            if last {
                said = action;
            }
        }
        said
    };

    let (mut sell, mut buy, mut star) = (Vec::new(), Vec::new(), Vec::new());
    let mut x = 2.0;
    while x < width {
        let said = press(x);
        match said.live_trade {
            Some(degenview_brokers::Side::Sell) => sell.push(x),
            Some(degenview_brokers::Side::Buy) => buy.push(x),
            None => {}
        }
        if said.toggle_favorite {
            star.push(x);
        }
        x += 4.0;
    }
    assert!(!sell.is_empty(), "SELL can be pressed");
    assert!(!buy.is_empty(), "BUY can be pressed");
    assert!(!star.is_empty(), "the star can be pressed");
    let leftmost_star = star.iter().copied().fold(f32::MAX, f32::min);
    let rightmost_pill = sell.iter().chain(&buy).copied().fold(0.0, f32::max);
    assert!(
        rightmost_pill < leftmost_star,
        "the pills are left of the star: pills end {rightmost_pill}, star starts {leftmost_star}"
    );
    assert!(
        star.iter().copied().fold(0.0, f32::max) > width - 60.0,
        "the star is at the right edge"
    );
}

/// The sandbox connects from the Settings page with no key at all, shows its canned account, and
/// is a broker of its own beside the real one.
#[test]
fn the_sandbox_connects_without_a_key_and_is_listed_beside_coinbase() {
    use degenview_brokers::coinbase::SANDBOX_ID;

    let rig = rig();
    let ids: Vec<_> = rig
        .host
        .brokers()
        .iter()
        .map(|b| b.descriptor().id)
        .collect();
    assert_eq!(ids, vec![ID, SANDBOX_ID]);
    let sandbox = degenview_brokers::registry::by_id(SANDBOX_ID).expect("the sandbox");
    assert!(sandbox.descriptor().credential_fields.is_empty());

    let mut state = settings_brokers::BrokersState::default();
    let mut screen = Screen::new(Vec2::new(600.0, 1200.0));
    state.connect_for_test(&rig.host, sandbox, &[]);
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        frames(&mut screen, 1, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                settings_brokers::body(ui, &mut state, Some(rig.host.clone()));
            });
        });
        if rig
            .host
            .snapshot(SANDBOX_ID)
            .is_some_and(|s| s.status == Status::Live)
        {
            break;
        }
    }
    let snapshot = rig.host.snapshot(SANDBOX_ID).expect("connected");
    assert_eq!(snapshot.status, Status::Live);
    assert!(snapshot.session.as_ref().unwrap().permissions.can_trade);
    assert!(
        rig.host.snapshot(ID).is_none(),
        "the real broker is untouched"
    );
}

/// The trading panel's top edge can be dragged to make it taller on every tab, whether the account
/// holds orders or nothing: a panel whose content is a footer short of the room it was given
/// shrinks back every frame and cannot be resized.
#[test]
fn the_trading_panel_can_be_resized_on_every_tab_with_and_without_orders() {
    let _restore = theme::RestoreDefaultTheme;
    let mut results = Vec::new();
    for (name, rig) in [("no orders", self::rig()), ("with orders", live_rig())] {
        if name == "no orders" {
            rig.host
                .adopt(ID, credentials(&generate_key_pem()))
                .unwrap();
            wait("live", || {
                rig.host
                    .snapshot(ID)
                    .is_some_and(|s| s.status == Status::Live)
            });
        }
        for tab in [
            BrokerTab::Orders,
            BrokerTab::History,
            BrokerTab::Fills,
            BrokerTab::Balances,
        ] {
            let mut screen = Screen::new(Vec2::new(1_100.0, 760.0));
            let mut state = BrokerPanelState::default();
            state.set_tab(tab);
            let height = std::cell::Cell::new(0.0_f32);
            let mut draw = |screen: &mut Screen, events: Vec<egui::Event>| {
                screen.run(events, |ui| {
                    theme::sync(ui.ctx());
                    let shown = egui::Panel::bottom(egui::Id::new("resize-gallery"))
                        .resizable(true)
                        .default_size(340.0)
                        .min_size(280.0)
                        .frame(egui::Frame::new().fill(crate::ui::kit::window()))
                        .show(ui, |ui| {
                            draw(ui, &mut state, &rig.host);
                        });
                    height.set(shown.response.rect.height());
                });
            };
            for _ in 0..6 {
                draw(
                    &mut screen,
                    vec![egui::Event::PointerMoved(egui::pos2(600.0, 100.0))],
                );
                std::thread::sleep(Duration::from_millis(15));
            }
            let before = height.get();
            let edge = 760.0 - before;
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let start = egui::pos2(600.0, edge);
            draw(&mut screen, vec![egui::Event::PointerMoved(start)]);
            draw(&mut screen, vec![button(start, true)]);
            for step in 1..=8 {
                let at = egui::pos2(600.0, edge - 10.0 * step as f32);
                draw(&mut screen, vec![egui::Event::PointerMoved(at)]);
            }
            let end = egui::pos2(600.0, edge - 80.0);
            draw(&mut screen, vec![button(end, false)]);
            for _ in 0..3 {
                draw(&mut screen, vec![egui::Event::PointerMoved(end)]);
            }
            results.push((name, tab, before, height.get()));
        }
    }
    for (name, tab, before, after) in &results {
        assert!(
            *after > *before + 40.0,
            "{name} / {tab:?}: dragging the top edge up by 80 should make the panel taller: {before} -> {after} ({results:?})"
        );
    }
}
