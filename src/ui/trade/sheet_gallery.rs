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
                symbol: "BTC-USD".to_owned(),
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
