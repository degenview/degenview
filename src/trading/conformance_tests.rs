//! The same checks against every venue's adapter: whatever a venue is, the screens read it through
//! one trait, so they must all answer it the same way.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
use degenview_brokers::testing::conformance::Exchange as _;
use degenview_brokers::{BrokerId, Decimal, Env, MarketRef, MemorySecretStore, Side, Status};

use super::backend::TradingBackend;
use super::draft::{DraftField, TradeDraft};
use super::model::{BackendState, MarketAccess, OrderClass, UnavailableKind, VenueKind};
use super::task::Task;
use crate::host::broker_host::{BrokerHost, StoreJournal};
use crate::host::paper_host::PaperHost;
use crate::host::trading_hub::TradingHub;
use crate::model::trading_venue::TradingVenue;
use crate::store::{SharedStore, Store};

const COINBASE: BrokerId = BrokerId::new("coinbase");

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn market() -> MarketRef {
    MarketRef {
        source: "coinbase".to_owned(),
        symbol: "BTC-USD".to_owned(),
    }
}

fn d(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn answer<T: Clone>(mut task: Task<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(answer) = task.poll() {
            return answer.clone();
        }
        assert!(Instant::now() < deadline, "the venue never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn limit_buy() -> TradeDraft {
    let mut draft = TradeDraft::new(Side::Buy);
    draft.size_text = "0.001".to_owned();
    draft.price_text = "90000.00".to_owned();
    draft
}

/// What every venue must do. `settle_cancel` is told which order was cancelled, for a venue whose
/// confirmation arrives on its own stream (the test plays that part).
fn check(backend: &dyn TradingBackend, kind: VenueKind, settle_cancel: &dyn Fn(&str)) {
    let info = backend.info();
    assert_eq!(info.kind, kind);
    assert!(!info.name.is_empty() && !info.summary.is_empty());
    assert_eq!(backend.state(), BackendState::Ready);
    assert!(backend.tabs().contains(&super::model::TabKind::Orders));

    // What can be asked of the market, in the venue's own increments.
    let access = backend.market(&market());
    let rules = access.rules().expect("a market of this venue");
    assert!(rules.caps.allows(OrderClass::Market) && rules.caps.allows(OrderClass::Limit));
    assert!(rules.price_increment > Decimal::ZERO && rules.size_increment > Decimal::ZERO);
    assert_eq!(rules.base_asset, "BTC");
    assert_eq!(rules.quote_asset, "USD");

    // A chart of a source no venue prices is nobody's market: nothing is offered or substituted.
    let other = MarketRef {
        source: "cmc".to_owned(),
        symbol: "BTC".to_owned(),
    };
    match backend.market(&other) {
        MarketAccess::Unavailable(reason) => {
            assert_eq!(
                reason.kind,
                UnavailableKind::NotThisVenue,
                "{}",
                reason.text
            );
            assert!(!reason.text.is_empty());
        }
        MarketAccess::Ready(_) => panic!("a CoinMarketCap chart is not tradable anywhere"),
    }

    // Funds to spend, in the market's quote asset.
    let funds = backend
        .available(&market(), Side::Buy)
        .expect("the account has funds");
    assert!(funds.value > Decimal::ZERO && !funds.asset.is_empty());
    assert!(backend.account().is_some());

    // The strip: a big figure and quieter ones, every one with a title and a value to draw.
    let stats = backend.stats().expect("an account with numbers");
    assert_eq!(stats.hero.title, "Equity");
    assert!(!stats.hero.value.is_empty());
    assert!(stats.cells.len() >= 4);
    assert!(
        stats
            .cells
            .iter()
            .all(|cell| !cell.title.is_empty() && !cell.value.is_empty() && !cell.help.is_empty())
    );
    let most = backend
        .max_size(&market(), Side::Buy, Some(d("90000")))
        .expect("the account can afford some");
    assert!(most > Decimal::ZERO);

    // A fresh sheet is incomplete, not wrong; what is typed wrongly is wrong, field by field.
    let fresh = backend.check(&market(), &TradeDraft::new(Side::Buy));
    assert!(fresh.issues.is_empty(), "{:?}", fresh.issues);
    assert_eq!(fresh.missing, vec![DraftField::Size, DraftField::Price]);
    let mut typo = limit_buy();
    typo.size_text = "abc".to_owned();
    let found = backend.check(&market(), &typo);
    assert!(
        found.issue_for(DraftField::Size).is_some(),
        "{:?}",
        found.issues
    );
    let mut nothing = limit_buy();
    nothing.size_text = "0".to_owned();
    assert!(
        backend
            .check(&market(), &nothing)
            .issue_for(DraftField::Size)
            .is_some()
    );

    // A good order is complete and its cost is known before anything is asked.
    let draft = limit_buy();
    let good = backend.check(&market(), &draft);
    assert!(good.is_complete(), "{:?} {:?}", good.issues, good.missing);
    assert_eq!(good.estimate.notional, Some(d("90")));

    // The venue is asked what it would do, and says it would take it.
    let preview = answer(backend.preview(&market(), &draft)).expect("a preview");
    assert!(preview.accepted, "{:?}", preview.problems);

    // Sending it lists it, as the venue has it.
    assert!(backend.orders().iter().all(|row| !row.working));
    let before = backend.revision();
    let sent = answer(backend.submit(&market(), &draft)).expect("the order was taken");
    wait_for("the order to be listed", || {
        backend.orders().iter().any(|row| row.working)
    });
    let rows = backend.orders();
    let row = rows
        .iter()
        .find(|row| row.working)
        .expect("a working order");
    assert_eq!(row.id, sent.id);
    assert_eq!(row.side, Side::Buy);
    assert_eq!(row.price, Some(d("90000")));
    assert_eq!(row.size, Some(d("0.001")));
    assert_eq!(row.filled, Decimal::ZERO);
    assert_eq!(row.status.label, "Open");
    assert!(row.can_cancel && !row.market.is_empty());
    assert!(
        backend.revision() > before,
        "a change is visible to a screen"
    );

    // And cancelling it ends it, leaving the finished order in the list.
    answer(backend.cancel(&sent.id)).expect("the cancel was taken");
    settle_cancel(&sent.id);
    wait_for("the order to be cancelled", || {
        backend.orders().iter().all(|row| !row.working)
    });
    let rows = backend.orders();
    assert_eq!(rows.len(), 1, "the finished order stays in the list");
    assert_eq!(rows[0].status.label, "Cancelled");
    assert!(!rows[0].can_cancel);

    // A cancel of an order that is not there is an answer, not a hang.
    assert!(answer(backend.cancel("no-such-order")).is_err());
}

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("store")))
}

#[test]
fn the_paper_account_answers_as_a_venue() {
    let store = store();
    let paper = Arc::new(Mutex::new(PaperHost::load(store)));
    paper
        .lock()
        .unwrap()
        .ensure_default_account()
        .unwrap()
        .expect("an account");
    let backend = super::paper_backend::PaperBackend::new(paper);
    check(&backend, VenueKind::Simulated, &|_| {});
}

#[test]
fn a_connected_broker_answers_as_a_venue() {
    let exchange = CoinbaseExchange::start();
    let store = store();
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
    brokers
        .adopt(COINBASE, credentials(&generate_key_pem()))
        .unwrap();
    wait_for("live", || {
        brokers
            .snapshot(COINBASE)
            .is_some_and(|s| s.status == Status::Live)
    });
    let paper = Arc::new(Mutex::new(PaperHost::load(store.clone())));
    let hub = TradingHub::new(brokers, paper, Some(store));
    let backend = hub
        .backend(TradingVenue::Broker(COINBASE))
        .expect("the broker");
    check(backend.as_ref(), VenueKind::Live, &|id| {
        // The venue confirms on its own stream; this one is told to.
        exchange.change_order(id, "CANCELLED", "0", false);
    });
}
