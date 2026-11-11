//! The broker host against the crate's mock exchange, and the rules that keep live trading apart
//! from everything that could reach it by accident.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
use degenview_brokers::model::pending::{PendingJournal, PendingSubmission, SubmissionOutcome};
use degenview_brokers::{
    AccountId, BrokerId, ClientOrderId, Credentials, Env, MemorySecretStore, OrderDraft, OrderKind,
    ProductId, SecretStore, Side, Status, TimeInForce,
};

use super::broker_host::{BrokerHost, Connection, PENDING_KEY, PROFILE, StoreJournal};
use crate::store::{SharedStore, Store};

const ID: BrokerId = BrokerId::new("coinbase");

fn store() -> SharedStore {
    Arc::new(Mutex::new(
        Store::open_in_memory().expect("in-memory store"),
    ))
}

fn host(
    exchange: &CoinbaseExchange,
    store: &SharedStore,
) -> (Arc<BrokerHost>, Arc<MemorySecretStore>) {
    let secrets = Arc::new(MemorySecretStore::new());
    (host_over(exchange, store, secrets.clone()), secrets)
}

fn host_over(
    exchange: &CoinbaseExchange,
    store: &SharedStore,
    secrets: Arc<dyn SecretStore>,
) -> Arc<BrokerHost> {
    let env = Env::default()
        .with_loopback_base_url(exchange.rest.url())
        .with_loopback_ws_url(exchange.stream.url.clone())
        .with_stream_timing(degenview_brokers::StreamTiming::fast())
        .with_request_gap(Duration::from_millis(2));
    BrokerHost::new(
        secrets,
        Arc::new(StoreJournal::new(store.clone())),
        Some(store.clone()),
        env,
        |config| config.fast(),
    )
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

fn draft() -> OrderDraft {
    OrderDraft {
        product: ProductId::new("BTC-USD"),
        side: Side::Buy,
        kind: OrderKind::Limit {
            base_size: "0.001".parse().unwrap(),
            price: "90000.00".parse().unwrap(),
            tif: TimeInForce::GoodTilCancelled,
            post_only: false,
        },
    }
}

fn is_live(host: &BrokerHost) -> bool {
    host.snapshot(ID).is_some_and(|s| s.status == Status::Live)
}

#[test]
fn a_key_is_checked_then_saved_then_connected_and_disconnect_removes_it() {
    let exchange = CoinbaseExchange::start();
    let (host, secrets) = host(&exchange, &store());
    assert!(matches!(host.connection(ID), Connection::NotConfigured));

    let credentials = credentials(&generate_key_pem());
    let session = host
        .validate(ID, credentials.clone())
        .recv()
        .unwrap()
        .expect("a good key");
    assert!(session.permissions.can_trade);
    assert!(
        matches!(host.connection(ID), Connection::NotConfigured),
        "checking a key saves nothing and connects nothing"
    );
    assert!(!secrets.is_configured(ID, PROFILE));

    host.adopt(ID, credentials).wait().expect("saved");
    assert!(secrets.is_configured(ID, PROFILE));
    wait_for("the connection to go live", || is_live(&host));
    assert!(matches!(host.connection(ID), Connection::Running(_)));
    assert_eq!(host.connected(), vec![ID]);
    assert!(!host.snapshot(ID).unwrap().balances.is_empty());

    host.disconnect(ID).wait().expect("removed");
    assert!(host.snapshot(ID).is_none());
    assert!(
        !secrets.is_configured(ID, PROFILE),
        "the key is gone from this Mac"
    );
    assert!(matches!(host.connection(ID), Connection::NotConfigured));
}

/// The Settings window is drawn while the shared store is locked. Connecting from it must not wait
/// for that lock on the interface thread: it did, and the whole application froze.
#[test]
fn connecting_while_the_caller_holds_the_store_does_not_wait_for_it() {
    let exchange = CoinbaseExchange::start();
    let store = store();
    let (host, _) = host(&exchange, &store);
    let held = store.lock().expect("the store");
    let (sender, receiver) = std::sync::mpsc::channel();
    let ui_thread = Arc::clone(&host);
    std::thread::spawn(move || {
        let saved = ui_thread.adopt(ID, credentials(&generate_key_pem())).wait();
        let _ = sender.send(saved);
    });
    receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("adopt returned while the store was locked")
        .expect("saved");
    drop(held);
    wait_for("the connection to go live", || is_live(&host));
}

/// Changing a trading choice on the Settings page also happens under the store's lock.
#[test]
fn changing_choices_while_the_caller_holds_the_store_does_not_wait_for_it() {
    let exchange = CoinbaseExchange::start();
    let store = store();
    let (host, _) = host(&exchange, &store);
    let held = store.lock().expect("the store");
    let (sender, receiver) = std::sync::mpsc::channel();
    let ui_thread = Arc::clone(&host);
    std::thread::spawn(move || {
        let mut prefs = ui_thread.prefs(ID);
        prefs.notify = false;
        ui_thread.set_prefs(ID, prefs);
        let _ = sender.send(());
    });
    receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("set_prefs returned while the store was locked");
    assert!(!host.prefs(ID).notify, "applied at once");
    drop(held);
}

/// Quick changes made while the store is locked each save on a thread. Whichever runs last must
/// write the newest choices: a thread that read them before it had the lock wrote an older set
/// after a newer one.
#[test]
fn quick_changes_save_the_newest_choices_whatever_order_the_saves_run_in() {
    // The order the waiting saves run in is the platform's: several rounds make a wrong one likely
    // to show.
    for _ in 0..6 {
        quick_changes_round();
    }
}

fn quick_changes_round() {
    let exchange = CoinbaseExchange::start();
    let store = store();
    let (host, _) = host(&exchange, &store);
    let held = store.lock().expect("the store");
    let ui_thread = Arc::clone(&host);
    let changer = std::thread::spawn(move || {
        for step in 0..8 {
            let mut prefs = ui_thread.prefs(ID);
            match step % 4 {
                0 => prefs.notify = !prefs.notify,
                1 => prefs.show_on_charts = !prefs.show_on_charts,
                2 => prefs.confirm_single_cancel = !prefs.confirm_single_cancel,
                _ => prefs.confirm_resting_orders = !prefs.confirm_resting_orders,
            }
            ui_thread.set_prefs(ID, prefs);
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    changer.join().expect("the changes were made");
    // Let every save thread reach the lock before it is released.
    std::thread::sleep(Duration::from_millis(100));
    let newest = host.prefs(ID);
    drop(held);
    wait_for("the choices to be saved", || {
        store
            .lock()
            .unwrap()
            .setting::<crate::model::broker::prefs::AllPrefs>(crate::model::broker::prefs::KEY)
            .is_some_and(|saved| saved.of(ID.as_str()) == newest)
    });
    std::thread::sleep(Duration::from_millis(150));
    let saved = store
        .lock()
        .unwrap()
        .setting::<crate::model::broker::prefs::AllPrefs>(crate::model::broker::prefs::KEY)
        .expect("saved");
    assert_eq!(saved.of(ID.as_str()), newest, "no older save landed last");
}

#[test]
fn a_bad_key_is_refused_without_echoing_it_and_nothing_is_saved() {
    let exchange = CoinbaseExchange::start();
    let (host, secrets) = host(&exchange, &store());
    let bad = Credentials::new()
        .with("name", "organizations/o/apiKeys/SECRET-NAME-123")
        .with("private_key", "SECRET-PRIVATE-KEY-456");
    let error = host.validate(ID, bad).recv().unwrap().unwrap_err();
    let shown = format!("{error} {} {error:?}", error.detail);
    assert!(
        !shown.contains("SECRET-NAME-123") && !shown.contains("SECRET-PRIVATE-KEY-456"),
        "{shown}"
    );
    assert!(!secrets.is_configured(ID, PROFILE));
}

#[test]
fn saved_credentials_connect_at_startup_and_a_revoked_key_shows_as_needing_attention() {
    let exchange = CoinbaseExchange::start();
    let (host, secrets) = host(&exchange, &store());
    secrets
        .save(ID, PROFILE, &credentials(&generate_key_pem()))
        .unwrap();
    // The store answers on its own thread: the key shows as saved once it has.
    wait_for("the saved key to show", || host.connection(ID).is_saved());
    host.connect_all_saved();
    wait_for("the saved key to connect", || is_live(&host));

    // The exchange revokes the key; the next read finds out.
    use degenview_brokers::testing::conformance::Exchange as _;
    exchange.revoke_key();
    host.runtime(ID).unwrap().refresh();
    wait_for("the failure", || {
        host.snapshot(ID)
            .is_some_and(|s| matches!(s.status, Status::Failed(_)))
    });
    assert!(host.snapshot(ID).unwrap().orders.is_empty() || !is_live(&host));
}

#[test]
fn choices_about_trading_default_to_asking_and_survive_a_restart() {
    let exchange = CoinbaseExchange::start();
    let store = store();
    let (first, _) = host(&exchange, &store);
    assert!(first.prefs(ID).confirm_resting_orders);
    let mut prefs = first.prefs(ID);
    prefs.confirm_resting_orders = false;
    prefs.notify = false;
    first.set_prefs(ID, prefs);
    // Saved on a thread of its own; wait until it is in the database.
    wait_for("the choices to be saved", || {
        store
            .lock()
            .expect("the store")
            .setting_strict::<serde_json::Value>(crate::model::broker::prefs::KEY)
            .ok()
            .flatten()
            .is_some()
    });
    // A new process over the same database.
    let (second, _) = host(&exchange, &store);
    assert!(!second.prefs(ID).confirm_resting_orders);
    assert!(!second.prefs(ID).notify);
    assert!(second.prefs(BrokerId::new("other")).confirm_resting_orders);
}

#[test]
fn prices_are_read_only_while_something_wants_them_and_a_stale_one_is_not_shown() {
    let exchange = CoinbaseExchange::start();
    let (host, _) = host(&exchange, &store());
    host.adopt(ID, credentials(&generate_key_pem()))
        .wait()
        .unwrap();
    wait_for("live", || is_live(&host));
    let product = ProductId::new("BTC-USD");
    let reads = |exchange: &CoinbaseExchange| {
        exchange.rest.with_state(|s| {
            s.requests_seen
                .iter()
                .filter(|r| r.ends_with("/best_bid_ask"))
                .count()
        })
    };
    host.tick_quotes();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(reads(&exchange), 0, "nothing asked for them");
    assert!(host.quote(ID, &product).is_none());

    host.want_quote(ID, &product);
    wait_for("a price", || {
        host.tick_quotes();
        host.quote(ID, &product).is_some()
    });
    let quote = host.quote(ID, &product).unwrap();
    assert_eq!(quote.bid.unwrap().to_string(), "89999.50");
    assert_eq!(quote.ask.unwrap().to_string(), "90000.50");
    assert!(reads(&exchange) >= 1);
}

#[test]
fn the_revision_rises_when_a_connection_changes_state() {
    let exchange = CoinbaseExchange::start();
    let (host, _) = host(&exchange, &store());
    let start = host.revision();
    host.adopt(ID, credentials(&generate_key_pem()))
        .wait()
        .unwrap();
    wait_for("live", || is_live(&host));
    assert!(host.revision() > start, "windows are told to redraw");
}

#[test]
fn a_placed_order_is_visible_to_everyone_sharing_the_host() {
    let exchange = CoinbaseExchange::start();
    let (host, _) = host(&exchange, &store());
    host.adopt(ID, credentials(&generate_key_pem()))
        .wait()
        .unwrap();
    wait_for("live", || is_live(&host));
    let window_a = host.runtime(ID).unwrap();
    let window_b = host.runtime(ID).unwrap();
    window_a
        .submit(draft(), super::broker_host::account_of(&window_a))
        .wait()
        .expect("placed");
    // The other window reads the same snapshot: the order is there, once.
    assert_eq!(window_b.snapshot().working_orders().count(), 1);
    assert_eq!(exchange.rest.with_state(|s| s.orders.len()), 1);
}

#[test]
fn the_pending_journal_survives_a_restart_and_never_forgets_on_a_bad_record() {
    let store = store();
    let first = StoreJournal::new(store.clone());
    let entry = PendingSubmission {
        broker: ID,
        account: AccountId::new("pf-1"),
        client_id: ClientOrderId::new("c-1"),
        draft: draft(),
        created_ms: 1_000,
    };
    first.record(&entry).unwrap();
    first.record(&entry).unwrap(); // recording twice keeps one
    // A new process: a new journal over the same database.
    let second = StoreJournal::new(store.clone());
    assert_eq!(second.unresolved(ID).unwrap(), vec![entry.clone()]);
    assert!(
        second
            .unresolved(BrokerId::new("other"))
            .unwrap()
            .is_empty()
    );
    second
        .resolve(
            ID,
            &entry.account,
            &entry.client_id,
            &SubmissionOutcome::NotPlaced,
        )
        .unwrap();
    assert!(second.unresolved(ID).unwrap().is_empty());

    // A record this version cannot read is an error, never "nothing pending".
    store
        .lock()
        .unwrap()
        .set_setting(PENDING_KEY, &"garbage")
        .unwrap();
    assert!(StoreJournal::new(store).unresolved(ID).is_err());
}

#[test]
fn pending_orders_and_broker_choices_never_sync_between_macs() {
    for key in [PENDING_KEY, "broker.prefs"] {
        assert_eq!(
            crate::sync::policy::setting_policy(key),
            crate::sync::policy::SettingPolicy::Local,
            "{key} must stay on this Mac"
        );
    }
}

// ---- Boundaries ---------------------------------------------------------------------------------

fn rust_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.display().to_string(),
                std::fs::read_to_string(&path).unwrap(),
            ));
        }
    }
}

/// Live trading has no path to scripts, alerts, webhooks or MCP: nothing in them may name the
/// broker crate or its host. A future automated-trading feature needs its own design and its own
/// permission, not a side door through one of these.
#[test]
fn nothing_that_scripts_or_agents_reach_can_touch_a_broker() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for area in ["mcp", "pine", "pine_studio", "model/alert"] {
        let mut files = Vec::new();
        let path = root.join(area);
        if path.is_dir() {
            rust_files(&path, &mut files);
        } else if path.exists() {
            files.push((
                path.display().to_string(),
                std::fs::read_to_string(&path).unwrap(),
            ));
        }
        for (name, text) in files {
            assert!(
                !text.contains("degenview_brokers") && !text.contains("broker_host"),
                "{name} reaches live trading"
            );
        }
    }
    for file in [
        "host/alert_host.rs",
        "host/alert_runtime.rs",
        "host/webhook_host.rs",
        "host/mcp_host.rs",
    ] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        assert!(
            !text.contains("degenview_brokers") && !text.contains("broker_host"),
            "{file} reaches live trading"
        );
    }
}

/// The application names no particular broker where it draws or stores broker state: the list
/// comes from the crate's registry, and a new broker needs no change here.
#[test]
fn the_applications_broker_layers_name_no_broker() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in [
        "host/broker_host.rs",
        "host/trading_hub.rs",
        "service/keychain_broker_secrets.rs",
        "trading/backend.rs",
        "trading/broker_backend.rs",
        "trading/model.rs",
        "trading/paper_backend.rs",
        "ui/settings/brokers.rs",
    ] {
        let text = std::fs::read_to_string(root.join(file))
            .unwrap()
            .to_lowercase();
        for name in ["coinbase", "binance", "kraken"] {
            assert!(!text.contains(name), "{file} names {name}");
        }
    }
}

// ---- The user's last word on a connection stands -----------------------------------------------

/// A secret store that can be made to sit on a read (a Keychain prompt that nobody has answered
/// yet) and to fail a removal.
struct Gated {
    inner: MemorySecretStore,
    block_next_load: std::sync::atomic::AtomicBool,
    entered: std::sync::atomic::AtomicBool,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    fail_delete: std::sync::atomic::AtomicBool,
}

impl Gated {
    fn new() -> (Arc<Self>, std::sync::mpsc::Sender<()>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            Arc::new(Self {
                inner: MemorySecretStore::new(),
                block_next_load: false.into(),
                entered: false.into(),
                release: Mutex::new(receiver),
                fail_delete: false.into(),
            }),
            sender,
        )
    }
}

impl SecretStore for Gated {
    fn load(
        &self,
        broker: BrokerId,
        profile: &str,
    ) -> Result<Option<Credentials>, degenview_brokers::SecretStoreError> {
        use std::sync::atomic::Ordering::SeqCst;
        // The key is read, then the answer waits for the person at the prompt.
        let found = self.inner.load(broker, profile)?;
        if self.block_next_load.swap(false, SeqCst) {
            self.entered.store(true, SeqCst);
            let _ = self.release.lock().unwrap().recv();
        }
        Ok(found)
    }
    fn save(
        &self,
        broker: BrokerId,
        profile: &str,
        credentials: &Credentials,
    ) -> Result<(), degenview_brokers::SecretStoreError> {
        self.inner.save(broker, profile, credentials)
    }
    fn delete(
        &self,
        broker: BrokerId,
        profile: &str,
    ) -> Result<(), degenview_brokers::SecretStoreError> {
        if self.fail_delete.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(degenview_brokers::SecretStoreError(
                "the keychain refused".to_owned(),
            ));
        }
        self.inner.delete(broker, profile)
    }
    fn is_configured(&self, broker: BrokerId, profile: &str) -> bool {
        self.inner.is_configured(broker, profile)
    }
}

#[test]
fn a_disconnect_while_the_saved_key_is_still_being_read_is_not_undone() {
    use std::sync::atomic::Ordering::SeqCst;
    let exchange = CoinbaseExchange::start();
    let (secrets, release) = Gated::new();
    secrets
        .inner
        .save(ID, PROFILE, &credentials(&generate_key_pem()))
        .unwrap();
    let host = host_over(&exchange, &store(), secrets.clone());

    // The start is waiting on the Keychain prompt.
    secrets.block_next_load.store(true, SeqCst);
    host.connect_saved(ID);
    wait_for("the read to begin", || secrets.entered.load(SeqCst));

    // The user disconnects meanwhile. The connection stops at once; the removal of the key waits
    // its turn behind the prompt, and is reported when it has run.
    let removed = host.disconnect(ID);
    assert!(host.runtime(ID).is_none());

    // The prompt is answered. The start that was waiting must not bring the connection back.
    release.send(()).unwrap();
    removed.wait().expect("removed");
    std::thread::sleep(Duration::from_millis(400));
    assert!(host.runtime(ID).is_none(), "a removed key is running");
    assert!(matches!(host.connection(ID), Connection::NotConfigured));
    assert_eq!(
        exchange.rest.with_state(|s| s.requests_seen.len()),
        0,
        "the removed key was never even used"
    );
}

#[test]
fn a_newer_key_is_not_replaced_by_a_start_that_began_before_it() {
    use std::sync::atomic::Ordering::SeqCst;
    let exchange = CoinbaseExchange::start();
    let (secrets, release) = Gated::new();
    secrets
        .inner
        .save(ID, PROFILE, &credentials(&generate_key_pem()))
        .unwrap();
    let host = host_over(&exchange, &store(), secrets.clone());

    // The saved key is being read (slowly) when the user enters another.
    secrets.block_next_load.store(true, SeqCst);
    host.connect_saved(ID);
    wait_for("the read to begin", || secrets.entered.load(SeqCst));
    // Its save waits its turn behind the read (one queue, in the order asked).
    let adopted = host.adopt(ID, credentials(&generate_key_pem()));

    // The slow read finishes first and starts the older connection; the newer must win anyway.
    release.send(()).unwrap();
    adopted.wait().unwrap();
    wait_for("the new key to go live", || is_live(&host));
    let newer = host.runtime(ID).unwrap().generation();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        host.runtime(ID).unwrap().generation(),
        newer,
        "the older key took over from the newer"
    );
}

#[test]
fn a_key_that_could_not_be_removed_says_so_and_comes_back_at_the_next_start() {
    use std::sync::atomic::Ordering::SeqCst;
    let exchange = CoinbaseExchange::start();
    let (secrets, _release) = Gated::new();
    let store = store();
    let host = host_over(&exchange, &store, secrets.clone());
    host.adopt(ID, credentials(&generate_key_pem()))
        .wait()
        .unwrap();
    wait_for("live", || is_live(&host));

    secrets.fail_delete.store(true, SeqCst);
    let error = host.disconnect(ID).wait().expect_err("the removal failed");
    assert!(error.contains("refused"), "{error}");
    // The connection is stopped, but the key is still stored, and the host says so.
    assert!(host.runtime(ID).is_none());
    assert!(matches!(host.connection(ID), Connection::Saved));

    // The next start finds it and connects with it: the error was the only warning.
    let next = host_over(&exchange, &store, secrets);
    next.connect_all_saved();
    wait_for("the key that was left to come back", || is_live(&next));
}

#[test]
fn the_same_client_id_in_two_accounts_is_two_records_in_the_store() {
    let journal = StoreJournal::new(store());
    let mut mine = PendingSubmission {
        broker: ID,
        account: AccountId::new("portfolio-a"),
        client_id: ClientOrderId::new("same"),
        draft: draft(),
        created_ms: 1,
    };
    journal.record(&mine).unwrap();
    mine.account = AccountId::new("portfolio-b");
    journal.record(&mine).unwrap();
    assert_eq!(journal.unresolved(ID).unwrap().len(), 2);

    journal
        .resolve(
            ID,
            &AccountId::new("portfolio-a"),
            &mine.client_id,
            &SubmissionOutcome::NotPlaced,
        )
        .unwrap();
    let left = journal.unresolved(ID).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].account, AccountId::new("portfolio-b"));

    // A record from before accounts existed reads back with none, and an account cannot close it.
    let legacy = serde_json::json!([{
        "broker": "coinbase", "client_id": "old", "created_ms": 2,
        "draft": serde_json::to_value(draft()).unwrap(),
    }]);
    let store = store();
    store
        .lock()
        .unwrap()
        .set_setting(PENDING_KEY, &legacy)
        .unwrap();
    let journal = StoreJournal::new(store);
    let records = journal.unresolved(ID).unwrap();
    assert_eq!(records.len(), 1);
    assert!(records[0].account.as_str().is_empty());
    journal
        .resolve(
            ID,
            &AccountId::new("portfolio-a"),
            &ClientOrderId::new("old"),
            &SubmissionOutcome::NotPlaced,
        )
        .unwrap();
    assert_eq!(journal.unresolved(ID).unwrap().len(), 1, "still held");
}

#[test]
fn a_price_that_was_already_old_when_it_arrived_is_not_shown_as_new() {
    let exchange = CoinbaseExchange::start();
    // The exchange's own time on the price is days back: it sat on the way.
    exchange
        .rest
        .with_state(|s| s.quote_time = "2026-11-09T12:00:00Z".to_owned());
    let (host, _) = host(&exchange, &store());
    host.adopt(ID, credentials(&generate_key_pem()))
        .wait()
        .unwrap();
    wait_for("live", || is_live(&host));
    let product = ProductId::new("BTC-USD");
    host.want_quote(ID, &product);
    let reads = || {
        exchange.rest.with_state(|s| {
            s.requests_seen
                .iter()
                .filter(|r| r.ends_with("/best_bid_ask"))
                .count()
        })
    };
    wait_for("the price to have been read", || {
        host.want_quote(ID, &product);
        host.tick_quotes();
        reads() >= 1
    });
    std::thread::sleep(Duration::from_millis(100));
    host.tick_quotes();
    assert!(
        host.quote(ID, &product).is_none(),
        "a price the exchange stamped long ago was shown as fresh"
    );

    // A current one is shown.
    exchange.rest.with_state(|s| s.quote_time = String::new());
    wait_for("a current price", || {
        host.want_quote(ID, &product);
        host.tick_quotes();
        host.quote(ID, &product).is_some()
    });
}

#[test]
fn an_unconfirmed_order_of_another_account_is_held_not_shown_or_settled() {
    let exchange = CoinbaseExchange::start();
    let store = store();
    // Written by a session on another portfolio, long ago.
    let theirs = PendingSubmission {
        broker: ID,
        account: AccountId::new("portfolio-elsewhere"),
        client_id: ClientOrderId::new("theirs"),
        draft: draft(),
        created_ms: 1_000,
    };
    StoreJournal::new(store.clone()).record(&theirs).unwrap();

    let (host, _) = host(&exchange, &store);
    host.adopt(ID, credentials(&generate_key_pem()))
        .wait()
        .unwrap();
    wait_for("live", || is_live(&host));
    std::thread::sleep(Duration::from_millis(500)); // several reconcile rounds
    let snapshot = host.snapshot(ID).unwrap();
    assert_eq!(snapshot.held_elsewhere, 1);
    assert!(
        snapshot.pending.is_empty(),
        "it is not this account's order"
    );
    assert_eq!(
        StoreJournal::new(store).unresolved(ID).unwrap(),
        vec![theirs],
        "and nothing deleted it"
    );
}

// ---- The interface thread never waits on the secret store ------------------------------------

/// A secret store that does nothing until it is let go: a Keychain prompt nobody has answered.
struct Stalled {
    inner: MemorySecretStore,
    stalled: std::sync::atomic::AtomicBool,
}

impl Stalled {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: MemorySecretStore::new(),
            stalled: true.into(),
        })
    }

    fn release(&self) {
        self.stalled
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    fn wait_turn(&self) {
        while self.stalled.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl SecretStore for Stalled {
    fn load(
        &self,
        broker: BrokerId,
        profile: &str,
    ) -> Result<Option<Credentials>, degenview_brokers::SecretStoreError> {
        self.wait_turn();
        self.inner.load(broker, profile)
    }
    fn save(
        &self,
        broker: BrokerId,
        profile: &str,
        credentials: &Credentials,
    ) -> Result<(), degenview_brokers::SecretStoreError> {
        self.wait_turn();
        self.inner.save(broker, profile, credentials)
    }
    fn delete(
        &self,
        broker: BrokerId,
        profile: &str,
    ) -> Result<(), degenview_brokers::SecretStoreError> {
        self.wait_turn();
        self.inner.delete(broker, profile)
    }
    fn is_configured(&self, broker: BrokerId, profile: &str) -> bool {
        self.wait_turn();
        self.inner.is_configured(broker, profile)
    }
}

/// Runs `work` on a thread of its own and fails, instead of hanging, if it takes more than a few
/// seconds: that is what a call that waits on the secret store looks like from a test.
fn returns_promptly<T: Send + 'static>(what: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap_or_else(|_| panic!("{what} waited on the secret store"))
}

#[test]
fn saving_removing_and_asking_never_wait_on_the_secret_store() {
    let exchange = CoinbaseExchange::start();
    let secrets = Stalled::new();
    let host = host_over(&exchange, &store(), secrets.clone());

    let (adopted, removed) = {
        let host = host.clone();
        returns_promptly(
            "adopt, disconnect, connect_saved and connection",
            move || {
                let adopted = host.adopt(ID, credentials(&generate_key_pem()));
                host.connect_saved(ID);
                let _ = host.connection(ID);
                let removed = host.disconnect(ID);
                (adopted, removed)
            },
        )
    };
    secrets.release();
    adopted.wait().expect("saved");
    removed.wait().expect("removed");
    wait_for("the removal to be seen", || {
        matches!(host.connection(ID), Connection::NotConfigured)
    });
}

/// One queue, in the order asked: a removal asked after a save must not run before it, or the key
/// that was just taken back is saved again and connects at the next start.
#[test]
fn a_key_saved_and_then_removed_while_the_store_was_busy_ends_up_removed() {
    let exchange = CoinbaseExchange::start();
    let secrets = Stalled::new();
    let host = host_over(&exchange, &store(), secrets.clone());

    let adopted = host.adopt(ID, credentials(&generate_key_pem()));
    let removed = host.disconnect(ID);
    secrets.release();
    let _ = adopted.wait();
    removed.wait().expect("removed");

    assert!(
        !secrets.inner.is_configured(ID, PROFILE),
        "the key came back"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(host.snapshot(ID).is_none(), "the connection came back");
}

/// What `connection()` says comes from what the store last answered, not from asking it now.
#[test]
fn a_saved_key_shows_once_the_store_has_answered_not_before() {
    let exchange = CoinbaseExchange::start();
    let secrets = Stalled::new();
    secrets
        .inner
        .save(ID, PROFILE, &credentials(&generate_key_pem()))
        .unwrap();
    let host = host_over(&exchange, &store(), secrets.clone());

    let first = {
        let host = host.clone();
        returns_promptly("connection", move || host.connection(ID))
    };
    assert!(matches!(first, Connection::NotConfigured), "not known yet");
    secrets.release();
    wait_for("the answer", || host.connection(ID).is_saved());
}

/// The trading layer is below the screens: it does not reach into `ui`, so the dependency runs one
/// way (the icon of a tab and the words of the paper strip once lived on the wrong side).
#[test]
fn the_trading_layer_does_not_reach_into_the_screens() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/trading");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let production = text.split("#[cfg(test)]").next().unwrap();
        assert!(
            !production.contains("crate::ui") && !production.contains("egui"),
            "trading/{name} reaches into the screens"
        );
    }
}
