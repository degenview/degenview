//! The order sheet drawn against real venues: the paper account, and a broker against the crate's
//! mock exchange. Always-run tests type orders and press the button; the pictures are written by
//! hand: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib sheet_gallery`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
use degenview_brokers::testing::conformance::Exchange as _;
use degenview_brokers::{BrokerId, Env, MarketRef, MemorySecretStore, Side, Status};
use eframe::egui::{self, Vec2};

use super::order_sheet::{self, SheetAction, SheetInput, SheetState};
use crate::host::broker_host::{BrokerHost, StoreJournal};
use crate::host::paper_host::PaperHost;
use crate::host::trading_hub::TradingHub;
use crate::model::trading_venue::TradingVenue;
use crate::store::{SharedStore, Store};
use crate::trading::model::{OrderClass, VenueInfo};
use crate::ui::render_tests::Screen;
use crate::ui::theme;

const COINBASE: BrokerId = BrokerId::new("coinbase");

struct Rig {
    exchange: CoinbaseExchange,
    hub: Arc<TradingHub>,
    paper: Arc<Mutex<PaperHost>>,
    brokers: Arc<BrokerHost>,
    /// The symbol of the focused chart; `BTC-USD` unless a test changes it.
    symbol: std::cell::RefCell<String>,
    /// Whether the focused chart's logo source, price and move are given to the sheet.
    with_face: std::cell::Cell<bool>,
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

fn rig() -> Rig {
    let exchange = CoinbaseExchange::start();
    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().expect("store")));
    let env = Env::default()
        .with_loopback_base_url(exchange.rest.url())
        .with_loopback_ws_url(exchange.stream.url.clone())
        .with_stream_timing(degenview_brokers::StreamTiming::fast())
        .with_request_gap(Duration::from_millis(2));
    let brokers = BrokerHost::new(
        Arc::new(MemorySecretStore::new()),
        Arc::new(StoreJournal::new(store.clone())),
        Some(store.clone()),
        env,
        |config| config.fast(),
    );
    let paper = Arc::new(Mutex::new(PaperHost::load(store.clone())));
    let hub = TradingHub::new(brokers.clone(), paper.clone(), Some(store));
    Rig {
        exchange,
        hub,
        paper,
        brokers,
        symbol: std::cell::RefCell::new("BTC-USD".to_owned()),
        with_face: std::cell::Cell::new(true),
    }
}

impl Rig {
    fn enable_paper(&self) {
        self.hub.enable(TradingVenue::Paper);
        wait("the paper account", || {
            self.paper.lock().unwrap().snapshot().accounts.len() == 1
        });
    }

    fn connect_broker(&self) {
        self.brokers
            .adopt(COINBASE, credentials(&generate_key_pem()))
            .unwrap();
        wait("the broker to go live", || {
            self.brokers
                .snapshot(COINBASE)
                .is_some_and(|s| s.status == Status::Live)
        });
    }

    /// One frame of the sheet for `venue` (none: nothing is enabled), with `events`.
    fn frame(
        &self,
        screen: &mut Screen,
        state: &mut SheetState,
        venue: Option<TradingVenue>,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Vec<SheetAction>) {
        let all: Vec<VenueInfo> = self.hub.infos();
        let enabled: Vec<VenueInfo> = all
            .iter()
            .filter(|info| self.hub.is_enabled(info.venue))
            .cloned()
            .collect();
        let backend = venue.and_then(|venue| self.hub.backend(venue));
        let current = venue
            .and_then(|venue| all.iter().find(|info| info.venue == venue))
            .zip(backend.as_deref());
        let input = SheetInput {
            enabled: &enabled,
            all: &all,
            current,
            market: Some(MarketRef {
                source: "coinbase".to_owned(),
                symbol: self.symbol.borrow().clone(),
            }),
            face: self.with_face.get().then_some(order_sheet::MarketFace {
                source: crate::model::DataSource::Coinbase,
                last: Some(90_000.0),
                change: Some((1_350.0, 1.52)),
            }),
            confirm_resting: true,
            replay: false,
        };
        let mut actions = Vec::new();
        let output = screen.run(events, |ui| {
            theme::sync(ui.ctx());
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(crate::ui::kit::window()))
                .show(ui, |ui| {
                    actions = order_sheet::show(ui, state, &input);
                });
        });
        (output, actions)
    }

    fn frames(
        &self,
        screen: &mut Screen,
        state: &mut SheetState,
        venue: Option<TradingVenue>,
        count: usize,
    ) -> egui::FullOutput {
        let mut last = None;
        for _ in 0..count {
            last = Some(
                self.frame(
                    screen,
                    state,
                    venue,
                    vec![egui::Event::PointerMoved(egui::pos2(5.0, 5.0))],
                )
                .0,
            );
            std::thread::sleep(Duration::from_millis(15));
        }
        last.unwrap()
    }

    /// Frames until the button can be pressed (the venue has answered its preview).
    fn until_sendable(
        &self,
        screen: &mut Screen,
        state: &mut SheetState,
        venue: Option<TradingVenue>,
    ) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.frames(screen, state, venue, 2);
            if state.button_enabled_for_test() {
                return;
            }
            assert!(Instant::now() < deadline, "the order never became sendable");
        }
    }

    fn press(
        &self,
        screen: &mut Screen,
        state: &mut SheetState,
        venue: Option<TradingVenue>,
        at: egui::Pos2,
    ) {
        for pressed in [true, false] {
            self.frame(
                screen,
                state,
                venue,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
}

fn limit_order(state: &mut SheetState, size: &str, price: &str) {
    state.start(Side::Buy, Some(OrderClass::Limit), None);
    state.draft.size_text = size.to_owned();
    state.draft.price_text = price.to_owned();
    // The sheet fills the market's price in on its own once there is one; here the price is typed.
    state.mark_prefilled_for_test();
}

#[test]
fn with_nothing_enabled_the_sheet_offers_every_venue_to_choose_from() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    let mut screen = Screen::new(Vec2::new(340.0, 640.0));
    let mut state = SheetState::default();
    // Pressing a venue's button reports it; nothing is connected by drawing the list.
    let output = rig.frames(&mut screen, &mut state, None, 3);
    assert!(output.shapes.len() > 20, "the list is drawn");
    screen.save("sheet-first-run", output);
    assert!(rig.hub.enabled_venues().is_empty());
    assert!(rig.paper.lock().unwrap().snapshot().accounts.is_empty());
}

#[test]
fn a_paper_order_is_typed_checked_and_placed_by_the_button() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.enable_paper();
    let venue = Some(TradingVenue::Paper);
    let mut screen = Screen::new(Vec2::new(340.0, 900.0));
    let mut state = SheetState::default();
    let output = rig.frames(&mut screen, &mut state, venue, 3);
    screen.save("sheet-paper-empty", output);

    limit_order(&mut state, "0.5", "90000");
    rig.until_sendable(&mut screen, &mut state, venue);
    let output = rig.frames(&mut screen, &mut state, venue, 2);
    screen.save("sheet-paper-ready", output);
    let button = state.button.expect("the button was drawn");
    rig.press(&mut screen, &mut state, venue, button.center());
    wait("the order to be placed", || {
        rig.paper
            .lock()
            .unwrap()
            .snapshot()
            .orders
            .iter()
            .any(|order| order.status.is_working())
    });
    rig.frames(&mut screen, &mut state, venue, 3);
    assert_eq!(state.notice_text(), Some("Order placed."));
    assert!(
        state.draft.size_text.is_empty(),
        "what was typed is done with"
    );
    let output = rig.frames(&mut screen, &mut state, venue, 2);
    screen.save("sheet-paper-placed", output);
}

#[test]
fn an_order_the_account_cannot_afford_cannot_be_sent() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.enable_paper();
    let venue = Some(TradingVenue::Paper);
    let mut screen = Screen::new(Vec2::new(340.0, 900.0));
    let mut state = SheetState::default();
    // 10,000 BTC at 90,000 is far beyond 100,000 of funds.
    limit_order(&mut state, "10000", "90000");
    rig.frames(&mut screen, &mut state, venue, 6);
    assert!(!state.button_enabled_for_test(), "the button is off");
    let button = state.button.expect("the button was drawn");
    rig.press(&mut screen, &mut state, venue, button.center());
    rig.frames(&mut screen, &mut state, venue, 3);
    assert!(rig.paper.lock().unwrap().snapshot().orders.is_empty());
    let output = rig.frames(&mut screen, &mut state, venue, 2);
    screen.save("sheet-paper-too-big", output);
}

#[test]
fn a_live_order_is_reviewed_before_it_is_sent_and_nothing_goes_without_it() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.connect_broker();
    let venue = Some(TradingVenue::Broker(COINBASE));
    let mut screen = Screen::new(Vec2::new(340.0, 900.0));
    let mut state = SheetState::default();
    let output = rig.frames(&mut screen, &mut state, venue, 3);
    screen.save("sheet-live-empty", output);

    limit_order(&mut state, "0.001", "90000.00");
    rig.until_sendable(&mut screen, &mut state, venue);
    let output = rig.frames(&mut screen, &mut state, venue, 2);
    screen.save("sheet-live-ready", output);
    let posts = || rig.exchange.order_posts();
    let before = posts();
    let button = state.button.expect("the button was drawn");
    rig.press(&mut screen, &mut state, venue, button.center());
    rig.frames(&mut screen, &mut state, venue, 3);
    assert!(state.is_reviewing(), "the review opens");
    assert_eq!(posts(), before, "nothing was sent by pressing the button");
    let output = rig.frames(&mut screen, &mut state, venue, 2);
    screen.save("sheet-live-review", output);
    // Backing out sends nothing either.
    assert!(state.close_review());
    rig.frames(&mut screen, &mut state, venue, 3);
    assert_eq!(posts(), before);
    assert!(!state.is_sending());
}

#[test]
fn a_broker_that_is_not_connected_says_so_instead_of_offering_a_form() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    let venue = Some(TradingVenue::Broker(COINBASE));
    let mut screen = Screen::new(Vec2::new(340.0, 640.0));
    let mut state = SheetState::default();
    let output = rig.frames(&mut screen, &mut state, venue, 3);
    assert!(state.button.is_none(), "no order can be started");
    screen.save("sheet-not-connected", output);
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

/// The whole path through the sheet: type an order, review it, see that nothing has been sent,
/// approve it with the keyboard, and find the order at the exchange and in the list.
#[test]
fn a_live_order_is_confirmed_before_it_is_sent_and_then_appears_everywhere() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.connect_broker();
    let venue = Some(TradingVenue::Broker(COINBASE));
    let mut screen = Screen::new(Vec2::new(900.0, 900.0));
    let mut state = SheetState::default();
    limit_order(&mut state, "0.001", "90000.00");
    rig.until_sendable(&mut screen, &mut state, venue);
    let button = state.button.expect("the button was drawn");
    rig.press(&mut screen, &mut state, venue, button.center());
    rig.frames(&mut screen, &mut state, venue, 2);
    assert!(state.is_reviewing(), "pressing the button opens the review");
    assert_eq!(
        rig.exchange.order_posts(),
        0,
        "nothing is sent before approval"
    );
    assert_eq!(
        rig.brokers
            .snapshot(COINBASE)
            .unwrap()
            .working_orders()
            .count(),
        0
    );

    // A stray Enter approves nothing: the dialog starts with no button focused.
    rig.frame(&mut screen, &mut state, venue, key(egui::Key::Enter));
    assert!(state.is_reviewing());
    assert_eq!(rig.exchange.order_posts(), 0);

    // Tab to "Back", Tab to "Place order", Enter.
    for events in [
        key(egui::Key::Tab),
        key(egui::Key::Tab),
        key(egui::Key::Enter),
    ] {
        rig.frame(&mut screen, &mut state, venue, events);
    }
    wait("the order to be sent", || {
        rig.frames(&mut screen, &mut state, venue, 1);
        rig.exchange.order_posts() > 0
    });
    assert_eq!(rig.exchange.order_posts(), 1, "sent exactly once");
    wait("the order in the snapshot", || {
        rig.brokers
            .snapshot(COINBASE)
            .is_some_and(|s| s.working_orders().count() == 1)
    });
    assert_eq!(rig.exchange.orders_created(), 1);
    rig.frames(&mut screen, &mut state, venue, 3);
    assert_eq!(state.notice_text(), Some("Order placed."));
}

#[test]
fn an_order_cannot_be_reviewed_or_sent_without_the_venues_preview() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.connect_broker();
    let venue = Some(TradingVenue::Broker(COINBASE));
    rig.exchange.rest.with_state(|s| s.fail_preview = true);
    let mut screen = Screen::new(Vec2::new(900.0, 900.0));
    let mut state = SheetState::default();
    limit_order(&mut state, "0.001", "90000.00");
    wait("the preview to fail", || {
        rig.frames(&mut screen, &mut state, venue, 1);
        state.preview_failed()
    });
    assert!(
        !state.button_enabled_for_test(),
        "the button is off when the venue could not preview"
    );

    // The button, and Enter, and the keyboard path: nothing opens, nothing is sent.
    let button = state.button.expect("the button was drawn");
    rig.press(&mut screen, &mut state, venue, button.center());
    rig.frame(&mut screen, &mut state, venue, key(egui::Key::Enter));
    rig.frames(&mut screen, &mut state, venue, 2);
    assert!(
        !state.is_reviewing(),
        "no review without the venue's answer"
    );
    assert_eq!(rig.exchange.order_posts(), 0);

    // The venue can preview again, and the user asks again.
    rig.exchange.rest.with_state(|s| s.fail_preview = false);
    state.retry_preview();
    rig.until_sendable(&mut screen, &mut state, venue);
    let button = state.button.expect("the button was drawn");
    rig.press(&mut screen, &mut state, venue, button.center());
    rig.frames(&mut screen, &mut state, venue, 2);
    assert!(state.is_reviewing(), "and the review opens once it did");
    assert_eq!(rig.exchange.order_posts(), 0, "and still nothing is sent");
}

#[test]
fn an_unreadable_record_of_unconfirmed_orders_stops_new_orders_at_the_sheet() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.connect_broker();
    let venue = Some(TradingVenue::Broker(COINBASE));
    let mut screen = Screen::new(Vec2::new(900.0, 900.0));
    let mut state = SheetState::default();
    limit_order(&mut state, "0.001", "90000.00");
    rig.until_sendable(&mut screen, &mut state, venue);

    rig.brokers.corrupt_pending_record_for_test();
    wait("the unreadable record to be noticed", || {
        rig.brokers
            .snapshot(COINBASE)
            .is_some_and(|s| s.journal_error.is_some())
    });
    assert!(!rig.brokers.snapshot(COINBASE).unwrap().can_send_orders());
    rig.frames(&mut screen, &mut state, venue, 3);
    assert!(
        !state.button_enabled_for_test(),
        "the button is off while the record is unreadable"
    );
    // The form is not even offered: the venue is paused, and the sheet says so.
    assert!(state.button.is_none(), "there is no button to press");
    let output = rig.frames(&mut screen, &mut state, venue, 2);
    screen.save("sheet-live-paused", output);
    assert!(
        !state.is_reviewing(),
        "no review while the record is unreadable"
    );
    assert_eq!(rig.exchange.order_posts(), 0);
    // And the runtime itself refuses, should anything get past the screen.
    let runtime = rig.brokers.runtime(COINBASE).unwrap();
    let error = runtime
        .submit(degenview_brokers::OrderDraft {
            product: degenview_brokers::ProductId::new("BTC-USD"),
            side: Side::Buy,
            kind: degenview_brokers::OrderKind::Limit {
                base_size: "0.001".parse().unwrap(),
                price: "90000.00".parse().unwrap(),
                tif: degenview_brokers::TimeInForce::GoodTilCancelled,
                post_only: false,
            },
        })
        .wait()
        .unwrap_err();
    assert!(error.to_string().contains("No order was sent"), "{error}");
    assert_eq!(rig.exchange.order_posts(), 0);
}

/// The sandbox has no prices and no money: an order is checked and sent with no review, and the
/// pill says it is a test.
#[test]
fn an_order_can_be_placed_on_the_sandbox_without_prices() {
    use degenview_brokers::coinbase::SANDBOX_ID;
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    let sandbox = TradingVenue::Broker(SANDBOX_ID);
    rig.hub.enable(sandbox);
    wait("the sandbox to go live", || {
        rig.brokers
            .snapshot(SANDBOX_ID)
            .is_some_and(|s| s.status == Status::Live)
    });
    let venue = Some(sandbox);
    let mut screen = Screen::new(Vec2::new(340.0, 900.0));
    let mut state = SheetState::default();
    limit_order(&mut state, "0.001", "60000.00");
    rig.until_sendable(&mut screen, &mut state, venue);
    let output = rig.frames(&mut screen, &mut state, venue, 2);
    screen.save("sheet-sandbox-ready", output);
    let button = state.button.expect("the button was drawn");
    rig.press(&mut screen, &mut state, venue, button.center());
    rig.frames(&mut screen, &mut state, venue, 2);
    assert!(!state.is_reviewing(), "no real money, so no review");
    wait("the order to reach the exchange", || {
        rig.frames(&mut screen, &mut state, venue, 1);
        rig.exchange.order_posts() == 1
    });
}

/// A chart whose market the broker does not list is told so in a card, not trade-able and not
/// not a bare line of red text.
#[test]
fn a_market_the_broker_does_not_list_is_explained_in_a_card() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.connect_broker();
    *rig.symbol.borrow_mut() = "NOPE-USD".to_owned();
    let venue = Some(TradingVenue::Broker(COINBASE));
    let mut screen = Screen::new(Vec2::new(340.0, 420.0));
    let mut state = SheetState::default();
    let output = rig.frames(&mut screen, &mut state, venue, 3);
    assert!(state.button.is_none(), "no order can be started");
    screen.save("sheet-market-not-offered", output);
}

/// A chart with no candles gives the sheet no face: the symbol is there, the price is not, and the
/// order can still be typed and sent.
#[test]
fn the_sheet_draws_without_a_chart_face() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.enable_paper();
    rig.with_face.set(false);
    let venue = Some(TradingVenue::Paper);
    let mut screen = Screen::new(Vec2::new(340.0, 900.0));
    let mut state = SheetState::default();
    limit_order(&mut state, "0.5", "90000");
    rig.until_sendable(&mut screen, &mut state, venue);
    assert!(state.button_enabled_for_test());
}

/// The sheet's other shapes, light and dark: a stop-limit with take profit and stop loss, and a
/// market sell.
#[test]
fn the_sheet_in_other_shapes_light_and_dark() {
    let _restore = theme::RestoreDefaultTheme;
    let rig = rig();
    rig.enable_paper();
    let venue = Some(TradingVenue::Paper);
    for light in [false, true] {
        let tag = if light { "light" } else { "dark" };
        let pref = if light {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        };
        for (name, side, class) in [
            ("stop-limit-bracket", Side::Buy, OrderClass::StopLimit),
            ("market-sell", Side::Sell, OrderClass::Market),
        ] {
            let mut screen = Screen::new(Vec2::new(380.0, 1100.0));
            screen.ctx.set_theme(pref);
            let mut state = SheetState::default();
            state.start(side, Some(class), None);
            state.mark_prefilled_for_test();
            state.draft.size_text = "0.5".to_owned();
            if class == OrderClass::StopLimit {
                state.draft.stop_text = "91000".to_owned();
                state.draft.price_text = "91500".to_owned();
                state.draft.bracket = true;
                state.draft.take_profit_text = "95000".to_owned();
            }
            let output = rig.frames(&mut screen, &mut state, venue, 4);
            screen.save(&format!("sheet-{name}-{tag}"), output);
        }
    }
}
