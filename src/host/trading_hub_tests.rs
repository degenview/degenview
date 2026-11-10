//! Which venues are enabled, what an upgrade keeps, and that choosing from Settings never waits on
//! the lock the Settings window holds.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use degenview_brokers::coinbase::SANDBOX_ID;
use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
use degenview_brokers::{BrokerId, Env, MemorySecretStore, SecretStore, Status};

use super::broker_host::{BrokerHost, StoreJournal};
use super::paper_host::PaperHost;
use super::trading_hub::{ENABLED_KEY, TradingHub};
use crate::model::paper::{PaperAccountSettings, PaperCurrency};
use crate::model::trading_venue::TradingVenue;
use crate::store::{SharedStore, Store};
use crate::trading::model::{BackendState, VenueKind};

const COINBASE: BrokerId = BrokerId::new("coinbase");
const SANDBOX: BrokerId = SANDBOX_ID;

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("store")))
}

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

struct Rig {
    exchange: CoinbaseExchange,
    store: SharedStore,
    brokers: Arc<BrokerHost>,
    paper: Arc<Mutex<PaperHost>>,
    secrets: Arc<MemorySecretStore>,
}

impl Rig {
    fn new() -> Self {
        Self::with_store(store())
    }

    fn with_store(store: SharedStore) -> Self {
        let exchange = CoinbaseExchange::start();
        let secrets = Arc::new(MemorySecretStore::new());
        let env = Env::default()
            .with_loopback_base_url(exchange.rest.url())
            .with_loopback_ws_url(exchange.stream.url.clone())
            .with_stream_timing(degenview_brokers::StreamTiming::fast())
            .with_request_gap(Duration::from_millis(2));
        let brokers = BrokerHost::new(
            secrets.clone(),
            Arc::new(StoreJournal::new(store.clone())),
            Some(store.clone()),
            env,
            |config| config.fast(),
        );
        let paper = Arc::new(Mutex::new(PaperHost::load(store.clone())));
        Self {
            exchange,
            store,
            brokers,
            paper,
            secrets,
        }
    }

    fn hub(&self) -> Arc<TradingHub> {
        TradingHub::new(
            self.brokers.clone(),
            self.paper.clone(),
            Some(self.store.clone()),
        )
    }
}

fn saved_enabled(store: &SharedStore) -> Option<Vec<String>> {
    store.lock().unwrap().setting(ENABLED_KEY)
}

#[test]
fn a_new_profile_has_no_venue_enabled_and_lists_every_one_to_choose_from() {
    let rig = Rig::new();
    let hub = rig.hub();
    assert!(hub.enabled_venues().is_empty());
    assert_eq!(hub.default_venue(), None);
    let venues: Vec<TradingVenue> = hub.infos().iter().map(|info| info.venue).collect();
    assert_eq!(
        venues,
        vec![
            TradingVenue::Paper,
            TradingVenue::Broker(COINBASE),
            TradingVenue::Broker(SANDBOX)
        ]
    );
    let kinds: Vec<VenueKind> = hub.infos().iter().map(|info| info.kind).collect();
    assert_eq!(
        kinds,
        vec![VenueKind::Simulated, VenueKind::Live, VenueKind::Test],
        "the sandbox is not real money"
    );
    // Nothing was created behind the user's back.
    assert!(rig.paper.lock().unwrap().snapshot().accounts.is_empty());
    assert_eq!(
        hub.backend(TradingVenue::Paper).unwrap().state(),
        BackendState::NotConnected
    );
}

#[test]
fn enabling_paper_creates_its_account_and_is_remembered() {
    let rig = Rig::new();
    let hub = rig.hub();
    hub.enable(TradingVenue::Paper);
    assert!(hub.is_enabled(TradingVenue::Paper));
    wait_for("the account", || {
        hub.backend(TradingVenue::Paper).unwrap().state() == BackendState::Ready
    });
    wait_for("the choice to be saved", || {
        saved_enabled(&rig.store) == Some(vec!["paper".to_owned()])
    });
    // Another start of the app: still enabled, and it is the venue a new tab opens on.
    let again = rig.hub();
    assert_eq!(again.default_venue(), Some(TradingVenue::Paper));
    again.disable(TradingVenue::Paper);
    assert!(!again.is_enabled(TradingVenue::Paper));
    assert_eq!(
        rig.paper.lock().unwrap().snapshot().accounts.len(),
        1,
        "turning paper off keeps its data"
    );
}

/// Everyone had a paper account before this change; only those who used it keep paper switched on.
#[test]
fn an_upgrade_keeps_paper_for_those_who_used_it_and_not_for_the_rest() {
    // Untouched default account: nothing to keep.
    let untouched = Rig::new();
    untouched
        .paper
        .lock()
        .unwrap()
        .ensure_default_account()
        .unwrap();
    assert!(!untouched.hub().is_enabled(TradingVenue::Paper));
    wait_for("the decision to be saved", || {
        saved_enabled(&untouched.store).is_some()
    });

    // A renamed, differently funded account: used.
    let used = Rig::new();
    used.paper
        .lock()
        .unwrap()
        .create_account(
            "My practice",
            PaperCurrency::Usd,
            5_000.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
    assert!(used.hub().is_enabled(TradingVenue::Paper));
    wait_for("the decision to be saved", || {
        saved_enabled(&used.store) == Some(vec!["paper".to_owned()])
    });
}

#[test]
fn the_upgrade_decision_is_made_once() {
    let rig = Rig::new();
    let hub = rig.hub();
    assert!(!hub.is_enabled(TradingVenue::Paper));
    wait_for("the empty choice to be saved", || {
        saved_enabled(&rig.store) == Some(Vec::new())
    });
    // Paper data appears later (the user made an account elsewhere, say): not enabled behind their back.
    rig.paper
        .lock()
        .unwrap()
        .create_account(
            "Later",
            PaperCurrency::Usd,
            1.0,
            PaperAccountSettings::default(),
        )
        .unwrap();
    assert!(!rig.hub().is_enabled(TradingVenue::Paper));
}

#[test]
fn a_broker_with_a_saved_key_is_enabled_without_being_chosen() {
    let rig = Rig::new();
    let hub = rig.hub();
    assert!(!hub.is_enabled(TradingVenue::Broker(COINBASE)));
    rig.brokers
        .adopt(COINBASE, credentials(&generate_key_pem()))
        .expect("saved");
    assert!(hub.is_enabled(TradingVenue::Broker(COINBASE)));
    wait_for("the broker to go live", || {
        rig.brokers
            .snapshot(COINBASE)
            .is_some_and(|s| s.status == Status::Live)
    });
    assert_eq!(
        hub.backend(TradingVenue::Broker(COINBASE)).unwrap().state(),
        BackendState::Ready
    );
}

#[test]
fn a_broker_that_needs_no_key_is_enabled_by_choosing_it_and_connects_without_the_secret_store() {
    let rig = Rig::new();
    let hub = rig.hub();
    hub.enable(TradingVenue::Broker(SANDBOX));
    wait_for("the sandbox to go live", || {
        rig.brokers
            .snapshot(SANDBOX)
            .is_some_and(|s| s.status == Status::Live)
    });
    assert!(!rig.secrets.is_configured(SANDBOX, "default"));
    // Restart: it is enabled again and reconnects by itself.
    rig.brokers.disconnect(SANDBOX).ok();
    let again = rig.hub();
    assert!(again.is_enabled(TradingVenue::Broker(SANDBOX)));
    again.connect_enabled();
    wait_for("the sandbox to reconnect", || {
        rig.brokers
            .snapshot(SANDBOX)
            .is_some_and(|s| s.status == Status::Live)
    });
    // Choosing a broker that does need a key does not connect it.
    hub.enable(TradingVenue::Broker(COINBASE));
    std::thread::sleep(Duration::from_millis(200));
    assert!(rig.brokers.snapshot(COINBASE).is_none());
    let _ = &rig.exchange;
}

#[test]
fn disabling_a_broker_disconnects_it_and_removes_its_key() {
    let rig = Rig::new();
    let hub = rig.hub();
    rig.brokers
        .adopt(COINBASE, credentials(&generate_key_pem()))
        .unwrap();
    wait_for("live", || rig.brokers.snapshot(COINBASE).is_some());
    hub.disable(TradingVenue::Broker(COINBASE));
    assert!(rig.brokers.snapshot(COINBASE).is_none());
    assert!(!hub.is_enabled(TradingVenue::Broker(COINBASE)));
    assert!(!rig.secrets.is_configured(COINBASE, "default"));
}

/// Settings is drawn while the shared store is locked, and enabling a venue from it saves and
/// starts things that take that lock: on the interface thread it would wait for itself.
#[test]
fn choosing_a_venue_while_the_caller_holds_the_store_does_not_wait_for_it() {
    let rig = Rig::new();
    let hub = rig.hub();
    let held = rig.store.lock().expect("the store");
    let (sender, receiver) = std::sync::mpsc::channel();
    let ui_thread = Arc::clone(&hub);
    std::thread::spawn(move || {
        ui_thread.enable(TradingVenue::Paper);
        ui_thread.enable(TradingVenue::Broker(SANDBOX));
        ui_thread.set_last_used(TradingVenue::Paper);
        ui_thread.disable(TradingVenue::Paper);
        let _ = sender.send(());
    });
    receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("the choices returned while the store was locked");
    drop(held);
    wait_for("the last choice to be saved", || {
        saved_enabled(&rig.store).is_some_and(|ids| ids == vec!["coinbase-sandbox".to_owned()])
    });
    // And it stays the last one: no older write lands after it.
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        saved_enabled(&rig.store),
        Some(vec!["coinbase-sandbox".to_owned()])
    );
}

#[test]
fn the_settings_that_hold_the_choices_stay_on_this_mac() {
    use crate::sync::policy::{SettingPolicy, setting_policy};
    for key in [ENABLED_KEY, super::trading_hub::LAST_USED_KEY] {
        assert_eq!(setting_policy(key), SettingPolicy::Local, "{key}");
    }
}
