//! The broker host against the crate's mock exchange, and the rules that keep live trading apart
//! from everything that could reach it by accident.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use degenview_brokers::coinbase::fake::{CoinbaseExchange, credentials, generate_key_pem};
use degenview_brokers::model::pending::{PendingJournal, PendingSubmission, SubmissionOutcome};
use degenview_brokers::{
    BrokerId, ClientOrderId, Credentials, Env, MemorySecretStore, OrderDraft, OrderKind, ProductId,
    SecretStore, Side, Status, TimeInForce,
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
    let env = Env::default()
        .with_loopback_base_url(exchange.rest.url())
        .with_loopback_ws_url(exchange.stream.url.clone())
        .with_stream_timing(degenview_brokers::StreamTiming::fast())
        .with_request_gap(Duration::from_millis(2));
    let host = BrokerHost::new(
        secrets.clone(),
        Arc::new(StoreJournal::new(store.clone())),
        Some(store.clone()),
        env,
        |config| config.fast(),
    );
    (host, secrets)
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

    host.adopt(ID, credentials).expect("saved");
    assert!(secrets.is_configured(ID, PROFILE));
    wait_for("the connection to go live", || is_live(&host));
    assert!(matches!(host.connection(ID), Connection::Running(_)));
    assert_eq!(host.connected(), vec![ID]);
    assert!(!host.snapshot(ID).unwrap().balances.is_empty());

    host.disconnect(ID).expect("removed");
    assert!(host.snapshot(ID).is_none());
    assert!(
        !secrets.is_configured(ID, PROFILE),
        "the key is gone from this Mac"
    );
    assert!(matches!(host.connection(ID), Connection::NotConfigured));
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
    assert!(matches!(host.connection(ID), Connection::Saved));
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
    first.set_prefs(ID, prefs).unwrap();
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
    host.adopt(ID, credentials(&generate_key_pem())).unwrap();
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
    host.adopt(ID, credentials(&generate_key_pem())).unwrap();
    wait_for("live", || is_live(&host));
    assert!(host.revision() > start, "windows are told to redraw");
}

#[test]
fn a_placed_order_is_visible_to_everyone_sharing_the_host() {
    let exchange = CoinbaseExchange::start();
    let (host, _) = host(&exchange, &store());
    host.adopt(ID, credentials(&generate_key_pem())).unwrap();
    wait_for("live", || is_live(&host));
    let window_a = host.runtime(ID).unwrap();
    let window_b = host.runtime(ID).unwrap();
    window_a.submit(draft()).wait().expect("placed");
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
        .resolve(ID, &entry.client_id, &SubmissionOutcome::NotPlaced)
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
    for area in ["mcp", "pine", "pine_studio", "model/alert.rs"] {
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
        "service/keychain_broker_secrets.rs",
        "ui/settings_brokers.rs",
    ] {
        let text = std::fs::read_to_string(root.join(file))
            .unwrap()
            .to_lowercase();
        for name in ["coinbase", "binance", "kraken"] {
            assert!(!text.contains(name), "{file} names {name}");
        }
    }
}
