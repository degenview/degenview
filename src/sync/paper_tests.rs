//! Paper-trading accounts across two Macs: one Mac runs an account, the other mirrors it.

use super::core_tests::{Mac, mac};
use super::mock::MockCloud;
use crate::host::paper_host::PaperHost;
use crate::model::paper::{
    PaperAccountSettings, PaperCurrency, PaperInstrument, PaperOrderRequest, PaperOrderSide,
    PaperOrderStatus, PaperOrderType, PaperQuote, PaperTimeInForce,
};
use crate::store::sync::OutboxState;

fn pair() -> (MockCloud, Mac, Mac) {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "Mac A", "acct", Some("acct"));
    let mut b = mac(&cloud, "Mac B", "acct", Some("acct"));
    a.apply = true;
    b.apply = true;
    a.core.start();
    b.core.start();
    (cloud, a, b)
}

/// Lets both Macs upload and fetch until nothing moves, after the upload debounce has passed.
fn settle(a: &mut Mac, b: &mut Mac) {
    a.advance(6_000);
    b.advance(6_000);
    for _ in 0..6 {
        a.core.request_sync();
        a.run();
        b.core.request_sync();
        b.run();
    }
}

fn device(mac: &Mac) -> String {
    mac.store
        .lock()
        .unwrap()
        .sync_state()
        .unwrap()
        .unwrap()
        .device_id
}

fn host(mac: &Mac) -> PaperHost {
    PaperHost::load(mac.store.clone())
}

fn open(host: &mut PaperHost, name: &str) -> u64 {
    host.create_account(
        name,
        PaperCurrency::Usd,
        100_000.0,
        PaperAccountSettings::default(),
    )
    .expect("account")
}

pub(super) fn limit_buy(account: u64, price: f64) -> PaperOrderRequest {
    PaperOrderRequest {
        account_id: account,
        instrument: PaperInstrument::chart("BTCUSDT", "BTCUSDT", crate::model::DataSource::Binance),
        side: PaperOrderSide::Buy,
        kind: PaperOrderType::Limit,
        quantity: 1.0,
        limit_price: Some(price),
        stop_price: None,
        time_in_force: PaperTimeInForce::GoodTilCanceled,
        take_profit: None,
        stop_loss: None,
    }
}

fn quote(last: f64) -> PaperQuote {
    PaperQuote {
        instrument_key: "Binance:BTCUSDT".to_owned(),
        bid: Some(last - 0.5),
        ask: Some(last + 0.5),
        last: Some(last),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64),
        is_market_open: true,
    }
}

fn snapshot(mac: &Mac) -> crate::model::paper::PaperTradingSnapshot {
    mac.store.lock().unwrap().paper_trading().unwrap()
}

#[test]
fn an_account_made_on_one_mac_is_mirrored_on_the_other_without_echoing_back() {
    let (_cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let id = open(&mut on_a, "Main");
    settle(&mut a, &mut b);
    let shared = a.store.lock().unwrap().id_space().to_global(id);
    let on_b = snapshot(&b);
    assert_eq!(on_b.accounts.len(), 1);
    assert_eq!(on_b.accounts[0].id, shared);
    assert_eq!(on_b.accounts[0].name, "Main");
    assert_eq!(a.outbox().len() + b.outbox().len(), 0, "no echo");
    // B shows it as A's.
    let mirrored = host(&b);
    assert!(mirrored.owned_elsewhere(shared).is_some());
    assert!(host(&a).owned_elsewhere(id).is_none());
    assert_eq!(
        b.store
            .lock()
            .unwrap()
            .paper_owner(&shared.to_string())
            .unwrap(),
        Some(device(&a))
    );
}

#[test]
fn only_the_owner_fills_a_working_order() {
    let (_cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let id = open(&mut on_a, "Main");
    on_a.process_quote(quote(110.0)).unwrap();
    on_a.submit(limit_buy(id, 100.0)).unwrap();
    settle(&mut a, &mut b);
    let shared = a.store.lock().unwrap().id_space().to_global(id);

    let mut on_b = host(&b);
    on_b.process_quote(quote(99.0)).unwrap();
    let order = &snapshot(&b).orders[0];
    assert_eq!(order.account_id, shared);
    assert!(order.status.is_working(), "the mirror never fills");
    assert!(snapshot(&b).fills.is_empty());

    on_a.process_quote(quote(99.0)).unwrap();
    assert!(on_a.snapshot().orders[0].status == PaperOrderStatus::Filled);
    settle(&mut a, &mut b);
    assert_eq!(snapshot(&b).fills.len(), 1, "B sees A's fill");
    assert_eq!(snapshot(&b).orders[0].status, PaperOrderStatus::Filled);
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

#[test]
fn placing_an_order_on_the_mirror_takes_the_account_over() {
    let (_cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let id = open(&mut on_a, "Main");
    on_a.process_quote(quote(110.0)).unwrap();
    on_a.submit(limit_buy(id, 100.0)).unwrap();
    settle(&mut a, &mut b);
    let shared = a.store.lock().unwrap().id_space().to_global(id);

    let mut on_b = host(&b);
    assert!(on_b.owned_elsewhere(shared).is_some());
    on_b.process_quote(quote(110.0)).unwrap();
    on_b.submit(limit_buy(shared, 90.0))
        .expect("order on the mirror");
    assert!(on_b.owned_elsewhere(shared).is_none(), "B runs it now");
    settle(&mut a, &mut b);

    // A now mirrors it, with both orders, and does not fill them.
    let mut on_a = host(&a);
    assert!(on_a.owned_elsewhere(id).is_some());
    assert_eq!(on_a.snapshot().orders.len(), 2);
    on_a.process_quote(quote(95.0)).unwrap();
    on_a.process_quote(quote(80.0)).unwrap();
    assert!(on_a.snapshot().orders.iter().all(|o| o.status.is_working()));
    // The orders have different ids even though both Macs minted "order 2" or so on their own.
    let ids: std::collections::HashSet<_> = snapshot(&a).orders.iter().map(|o| o.id).collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(
        a.store
            .lock()
            .unwrap()
            .paper_owner(&id.to_string())
            .unwrap(),
        Some(device(&b))
    );
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

#[test]
fn account_changes_uploaded_before_either_mac_fetches_keep_both_versions() {
    let (_cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let id = open(&mut on_a, "Main");
    settle(&mut a, &mut b);
    let shared = a.store.lock().unwrap().id_space().to_global(id);

    on_a.process_quote(quote(110.0)).unwrap();
    on_a.submit(limit_buy(id, 100.0)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    let mut on_b = host(&b);
    on_b.process_quote(quote(110.0)).unwrap();
    on_b.submit(limit_buy(shared, 90.0)).unwrap();
    // The account's upload is debounced; let it come due, then both upload before either fetches.
    a.advance(10_000);
    b.advance(10_000);
    a.run();
    b.run();
    settle(&mut a, &mut b);

    for mac in [&a, &b] {
        let accounts = snapshot(mac).accounts;
        assert_eq!(accounts.len(), 2, "the account and a copy of the loser");
        assert_eq!(snapshot(mac).orders.len(), 2, "each Mac's order survives");
    }
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

#[test]
fn concurrent_changes_to_one_account_keep_both_versions() {
    let (_cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let id = open(&mut on_a, "Main");
    settle(&mut a, &mut b);
    let shared = a.store.lock().unwrap().id_space().to_global(id);

    // Neither has seen the other's change.
    on_a.process_quote(quote(110.0)).unwrap();
    on_a.submit(limit_buy(id, 100.0)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    let mut on_b = host(&b);
    on_b.process_quote(quote(110.0)).unwrap();
    on_b.submit(limit_buy(shared, 90.0)).unwrap();
    settle(&mut a, &mut b);

    for mac in [&a, &b] {
        let accounts = snapshot(mac).accounts;
        assert_eq!(accounts.len(), 2, "the account and a copy of the loser");
        let copies: Vec<_> = accounts
            .iter()
            .filter(|account| account.name.contains("conflict from"))
            .collect();
        assert_eq!(copies.len(), 1);
        assert!(copies[0].id >= crate::sync::ids::LEGACY_LIMIT, "a fresh id");
        // Each version's order survives, one in the account and one in the copy.
        let orders = snapshot(mac).orders;
        assert_eq!(orders.len(), 2);
        assert!(orders.iter().any(|o| o.account_id == copies[0].id));
        let copy_orders: Vec<_> = orders
            .iter()
            .filter(|o| o.account_id == copies[0].id)
            .collect();
        assert!(
            copy_orders
                .iter()
                .all(|o| o.id >= crate::sync::ids::LEGACY_LIMIT)
        );
        assert_eq!(
            mac.store
                .lock()
                .unwrap()
                .paper_owner(&copies[0].id.to_string())
                .unwrap(),
            Some("paused".to_string()),
            "nothing runs the copy"
        );
    }
    let noticed: usize = [&a, &b]
        .iter()
        .map(|mac| mac.store.lock().unwrap().conflicts().unwrap().len())
        .sum();
    assert!(noticed >= 1, "the Mac that merged tells the user");
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

#[test]
fn deleting_an_account_removes_it_on_the_other_mac() {
    let (_cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let keep = open(&mut on_a, "Keep");
    let drop = open(&mut on_a, "Drop");
    settle(&mut a, &mut b);
    assert_eq!(snapshot(&b).accounts.len(), 2);
    on_a.delete_account(drop).unwrap();
    settle(&mut a, &mut b);
    let on_b = snapshot(&b);
    assert_eq!(on_b.accounts.len(), 1);
    assert_eq!(on_b.accounts[0].name, "Keep");
    assert_eq!(
        on_b.accounts[0].id,
        a.store.lock().unwrap().id_space().to_global(keep)
    );
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

/// A journal of `entries` lines of `width` characters. `varied` makes every line different (and so
/// hard to deflate); otherwise they are all alike, like a long history of similar trades.
fn grow_journal(mac: &Mac, account: u64, entries: u64, width: usize, varied: bool) {
    let store = mac.store.lock().unwrap();
    let mut snapshot = store.paper_trading().unwrap();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for entry in 0..entries {
        let message = if varied {
            let mut line = String::new();
            while line.len() < width {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                line.push_str(&format!("{state:016x}"));
            }
            line
        } else {
            "x".repeat(width)
        };
        snapshot
            .journal
            .push(crate::model::paper::PaperJournalEntry {
                id: 100 + entry,
                account_id: account,
                timestamp: entry as i64,
                message,
            });
    }
    store.save_paper_trading(&snapshot).unwrap();
}

#[test]
fn an_account_that_does_not_fit_one_record_even_deflated_is_blocked_and_kept_locally() {
    let (cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let id = open(&mut on_a, "Huge");
    grow_journal(&a, id, 2_000, 1_500, true);
    settle(&mut a, &mut b);
    let rows = a.outbox();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, OutboxState::Blocked);
    assert!(cloud.names().is_empty());
    assert!(
        snapshot(&a).journal.len() >= 2_000,
        "nothing is lost locally"
    );
    assert!(snapshot(&b).accounts.is_empty());
}

#[test]
fn a_long_history_over_one_record_as_written_is_deflated_and_reaches_the_other_mac() {
    let (cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let id = open(&mut on_a, "Long history");
    // About 1.3 MB as written: over one record, far under it once deflated.
    grow_journal(&a, id, 2_000, 600, false);
    settle(&mut a, &mut b);
    assert!(
        a.outbox().is_empty(),
        "uploaded, not blocked: {:?}",
        a.outbox()
    );
    assert_eq!(cloud.names().len(), 1);
    let sent = cloud.record(&cloud.names()[0]).unwrap().payload.len();
    assert!(
        sent < super::record::MAX_PAYLOAD_BYTES,
        "{sent} bytes on the wire"
    );
    let on_b = snapshot(&b);
    assert_eq!(on_b.accounts.len(), 1);
    assert_eq!(on_b.accounts[0].name, "Long history");
    assert_eq!(on_b.journal.len(), snapshot(&a).journal.len());
    assert_eq!(a.outbox().len() + b.outbox().len(), 0, "no echo");
    let shared = a.store.lock().unwrap().id_space().to_global(id);
    assert!(host(&b).owned_elsewhere(shared).is_some(), "shown as A's");
}

#[test]
fn the_ids_of_a_mirror_never_move_the_local_counter() {
    let (_cloud, mut a, mut b) = pair();
    let mut on_a = host(&a);
    let id = open(&mut on_a, "Main");
    on_a.process_quote(quote(110.0)).unwrap();
    on_a.submit(limit_buy(id, 100.0)).unwrap();
    settle(&mut a, &mut b);
    // B makes an account of its own after the mirror arrived: a small id, not a huge one.
    let mut on_b = host(&b);
    let own = open(&mut on_b, "Mine");
    assert!(own < crate::sync::ids::LEGACY_LIMIT, "minted {own}");
}
