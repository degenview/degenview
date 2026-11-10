//! The same checks against every venue's adapter: whatever a venue is, the screens read it through
//! one trait, so they must all answer it the same way.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
use degenview_brokers::testing::conformance::Exchange as _;
use degenview_brokers::{
    BrokerId, Decimal, Env, MarketRef, MemorySecretStore, OrderDraft, OrderKind, ProductId, Side,
    Status, TimeInForce as CrateTif,
};

use super::backend::TradingBackend;
use super::model::{BackendState, MarketAccess, OrderClass, UnavailableKind, VenueKind};
use crate::host::broker_host::{BrokerHost, StoreJournal};
use crate::host::paper_host::PaperHost;
use crate::host::trading_hub::TradingHub;
use crate::model::DataSource;
use crate::model::paper::{
    PaperInstrument, PaperOrderRequest, PaperOrderSide, PaperOrderType, PaperTimeInForce,
};
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

/// What every venue must do. `buy` rests a limit buy of 0.001 at 90,000.00; `cancel` cancels
/// every working order.
fn check(backend: &dyn TradingBackend, kind: VenueKind, buy: &dyn Fn(), cancel: &dyn Fn()) {
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
    assert!(!rules.quote_asset.is_empty());

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

    // An order appears, as the venue has it, and is cancellable.
    assert!(backend.orders().iter().all(|row| !row.working));
    let before = backend.revision();
    buy();
    wait_for("the order to be listed", || {
        backend.orders().iter().any(|row| row.working)
    });
    let rows = backend.orders();
    let row = rows
        .iter()
        .find(|row| row.working)
        .expect("a working order");
    assert_eq!(row.side, Side::Buy);
    assert_eq!(row.price, Some(d("90000")));
    assert_eq!(row.size, Some(d("0.001")));
    assert_eq!(row.filled, Decimal::ZERO);
    assert_eq!(row.status.label, "Open");
    assert!(row.can_cancel && !row.id.is_empty() && !row.market.is_empty());
    assert!(
        backend.revision() > before,
        "a change is visible to a screen"
    );

    cancel();
    wait_for("the order to be cancelled", || {
        backend.orders().iter().all(|row| !row.working)
    });
    let rows = backend.orders();
    assert_eq!(rows.len(), 1, "the finished order stays in the list");
    assert_eq!(rows[0].status.label, "Cancelled");
    assert!(!rows[0].can_cancel);
}

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("store")))
}

#[test]
fn the_paper_account_answers_as_a_venue() {
    let store = store();
    let paper = Arc::new(Mutex::new(PaperHost::load(store)));
    let account = paper
        .lock()
        .unwrap()
        .ensure_default_account()
        .unwrap()
        .expect("an account");
    let backend = super::paper_backend::PaperBackend::new(paper.clone());
    let instrument = PaperInstrument::chart("BTC-USD", "BTC-USD", DataSource::Coinbase);
    let request = PaperOrderRequest {
        account_id: account,
        instrument,
        side: PaperOrderSide::Buy,
        kind: PaperOrderType::Limit,
        quantity: 0.001,
        limit_price: Some(90_000.0),
        stop_price: None,
        time_in_force: PaperTimeInForce::GoodTilCanceled,
        take_profit: None,
        stop_loss: None,
    };
    check(
        &backend,
        VenueKind::Simulated,
        &|| {
            paper
                .lock()
                .unwrap()
                .submit(request.clone())
                .expect("placed");
        },
        &|| {
            let ids: Vec<u64> = paper
                .lock()
                .unwrap()
                .snapshot()
                .orders
                .iter()
                .filter(|order| order.status.is_working())
                .map(|order| order.id)
                .collect();
            for id in ids {
                paper.lock().unwrap().cancel(id).expect("cancelled");
            }
        },
    );
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
    let hub = TradingHub::new(brokers.clone(), paper, Some(store));
    let backend = hub
        .backend(TradingVenue::Broker(COINBASE))
        .expect("the broker");
    let runtime = brokers.runtime(COINBASE).expect("running");
    let draft = OrderDraft {
        product: ProductId::new("BTC-USD"),
        side: Side::Buy,
        kind: OrderKind::Limit {
            base_size: d("0.001"),
            price: d("90000.00"),
            tif: CrateTif::GoodTilCancelled,
            post_only: false,
        },
    };
    check(
        backend.as_ref(),
        VenueKind::Live,
        &|| {
            runtime.submit(draft.clone()).wait().expect("placed");
        },
        &|| {
            let ids: Vec<_> = brokers
                .snapshot(COINBASE)
                .unwrap()
                .working_orders()
                .map(|view| view.order.id.clone())
                .collect();
            runtime.cancel(ids.clone()).wait().expect("cancelled");
            // The venue confirms on its own stream; this one is told to.
            for id in ids {
                exchange.change_order(id.as_str(), "CANCELLED", "0", false);
            }
        },
    );
}
