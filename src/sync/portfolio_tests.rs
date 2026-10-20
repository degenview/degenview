//! Portfolios and their transactions across two Macs, through the real core and apply layer.

use super::core::{RemoteBatch, RemoteRecord};
use super::core_tests::{Mac, mac};
use super::mock::MockCloud;
use super::policy::EntityKind;
use super::record::{RecordName, SCHEMA_VERSION, SyncRecord};
use crate::model::DataSource;
use crate::model::portfolio::{
    Portfolio, PortfolioAsset, PortfolioCurrency, PortfolioLedgerSnapshot, PortfolioSource,
    PortfolioTransaction, PortfolioTransactionType,
};
use crate::store::Store;
use crate::store::sync::Load;
use std::collections::HashMap;

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

fn settle(a: &mut Mac, b: &mut Mac) {
    for _ in 0..6 {
        a.core.request_sync();
        a.run();
        b.core.request_sync();
        b.run();
    }
}

fn tx(
    id: u64,
    portfolio: u64,
    kind: PortfolioTransactionType,
    quantity: f64,
    timestamp: i64,
) -> PortfolioTransaction {
    PortfolioTransaction {
        id,
        portfolio_id: portfolio,
        asset: PortfolioAsset {
            key: "binance:BTCUSDT".into(),
            symbol: "BTC".into(),
            name: "Bitcoin".into(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        },
        kind,
        quantity,
        price: Some(100.0),
        price_currency: PortfolioCurrency::Usd,
        fee: 0.0,
        fee_currency: PortfolioCurrency::Usd,
        timestamp,
        notes: String::new(),
        source: PortfolioSource::Manual,
        external_transaction_id: None,
        created_at: timestamp,
        updated_at: timestamp,
    }
}

const BUY: PortfolioTransactionType = PortfolioTransactionType::Buy;
const SELL: PortfolioTransactionType = PortfolioTransactionType::Sell;

fn save(mac: &Mac, portfolios: &[(u64, &str)], transactions: Vec<PortfolioTransaction>) {
    let snapshot = PortfolioLedgerSnapshot {
        portfolios: portfolios
            .iter()
            .map(|(id, name)| Portfolio::new(*id, *name, 1_000))
            .collect(),
        transactions,
        ..Default::default()
    };
    mac.store
        .lock()
        .unwrap()
        .save_portfolio_ledger(&snapshot)
        .unwrap();
}

fn ledger(mac: &Mac) -> PortfolioLedgerSnapshot {
    mac.store.lock().unwrap().portfolio_ledger().unwrap()
}

fn names(snapshot: &PortfolioLedgerSnapshot) -> Vec<String> {
    let mut names: Vec<_> = snapshot.portfolios.iter().map(|p| p.name.clone()).collect();
    names.sort();
    names
}

fn local_id(mac: &Mac, name: &str) -> u64 {
    ledger(mac)
        .portfolios
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no portfolio {name}"))
        .id
}

fn ids(mac: &Mac) -> super::ids::IdSpace {
    mac.store.lock().unwrap().id_space()
}

const NANO: u64 = 1_700_000_000_000_000_000;

#[test]
fn a_portfolio_and_its_ledger_appear_on_the_other_mac_without_echoing_back() {
    let (_cloud, mut a, mut b) = pair();
    save(
        &a,
        &[(1, "Main")],
        vec![
            tx(NANO, 1, BUY, 2.0, 1_000),
            tx(NANO + 1, 1, SELL, 1.0, 2_000),
        ],
    );
    settle(&mut a, &mut b);
    let on_b = ledger(&b);
    assert_eq!(names(&on_b), ["Main"]);
    let shared = ids(&a).to_global(1);
    assert_eq!(on_b.portfolios[0].id, shared, "B holds A's shared id");
    assert_eq!(on_b.transactions.len(), 2);
    assert!(on_b.transactions.iter().all(|t| t.portfolio_id == shared));
    assert_eq!(a.outbox().len() + b.outbox().len(), 0, "no echo");
    // A's own data is untouched.
    assert_eq!(ledger(&a).portfolios[0].id, 1);
}

#[test]
fn two_macs_that_both_have_a_portfolio_one_keep_two_portfolios() {
    let (_cloud, mut a, mut b) = pair();
    save(&a, &[(1, "From A")], vec![tx(NANO, 1, BUY, 1.0, 1_000)]);
    save(&b, &[(1, "From B")], vec![tx(NANO + 5, 1, BUY, 3.0, 1_000)]);
    settle(&mut a, &mut b);
    for mac in [&a, &b] {
        let snapshot = ledger(mac);
        assert_eq!(names(&snapshot), ["From A", "From B"]);
        assert_eq!(snapshot.transactions.len(), 2);
        // Each transaction belongs to the right portfolio on both Macs.
        for transaction in &snapshot.transactions {
            let portfolio = snapshot
                .portfolios
                .iter()
                .find(|p| p.id == transaction.portfolio_id)
                .expect("transaction has a portfolio");
            let expected = if transaction.quantity == 1.0 {
                "From A"
            } else {
                "From B"
            };
            assert_eq!(portfolio.name, expected);
        }
    }
    assert_eq!(local_id(&a, "From A"), 1);
    assert_eq!(local_id(&b, "From B"), 1);
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

#[test]
fn an_edit_on_the_other_mac_returns_to_the_original_small_id() {
    let (_cloud, mut a, mut b) = pair();
    save(&a, &[(1, "Main")], vec![tx(NANO, 1, BUY, 1.0, 1_000)]);
    settle(&mut a, &mut b);
    let mut on_b = ledger(&b);
    on_b.portfolios[0].name = "Renamed".into();
    on_b.transactions[0].quantity = 4.0;
    b.store
        .lock()
        .unwrap()
        .save_portfolio_ledger(&on_b)
        .unwrap();
    settle(&mut a, &mut b);
    let on_a = ledger(&a);
    assert_eq!(names(&on_a), ["Renamed"]);
    assert_eq!(on_a.portfolios[0].id, 1, "A still knows it as portfolio 1");
    assert_eq!(on_a.transactions[0].portfolio_id, 1);
    assert_eq!(on_a.transactions[0].quantity, 4.0);
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

#[test]
fn transactions_added_on_both_macs_are_both_kept() {
    let (_cloud, mut a, mut b) = pair();
    save(&a, &[(1, "Main")], vec![tx(NANO, 1, BUY, 5.0, 1_000)]);
    settle(&mut a, &mut b);
    let shared = ids(&a).to_global(1);
    let mut on_a = ledger(&a);
    on_a.transactions.push(tx(NANO + 10, 1, SELL, 1.0, 2_000));
    a.store
        .lock()
        .unwrap()
        .save_portfolio_ledger(&on_a)
        .unwrap();
    let mut on_b = ledger(&b);
    on_b.transactions
        .push(tx(NANO + 20, shared, SELL, 2.0, 3_000));
    b.store
        .lock()
        .unwrap()
        .save_portfolio_ledger(&on_b)
        .unwrap();
    settle(&mut a, &mut b);
    assert_eq!(ledger(&a).transactions.len(), 3);
    assert_eq!(ledger(&b).transactions.len(), 3);
}

#[test]
fn deleting_a_portfolio_removes_it_and_its_transactions_on_the_other_mac() {
    let (_cloud, mut a, mut b) = pair();
    save(
        &a,
        &[(1, "Keep"), (2, "Drop")],
        vec![
            tx(NANO, 1, BUY, 1.0, 1_000),
            tx(NANO + 1, 2, BUY, 1.0, 1_000),
            tx(NANO + 2, 2, BUY, 2.0, 1_500),
        ],
    );
    settle(&mut a, &mut b);
    assert_eq!(ledger(&b).transactions.len(), 3);
    let mut on_a = ledger(&a);
    on_a.portfolios.retain(|p| p.id != 2);
    on_a.transactions.retain(|t| t.portfolio_id != 2);
    a.store
        .lock()
        .unwrap()
        .save_portfolio_ledger(&on_a)
        .unwrap();
    settle(&mut a, &mut b);
    let on_b = ledger(&b);
    assert_eq!(names(&on_b), ["Keep"]);
    assert_eq!(on_b.transactions.len(), 1);
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

#[test]
fn applying_a_change_drops_the_valuation_history_after_it() {
    let (_cloud, mut a, mut b) = pair();
    save(&a, &[(1, "Main")], vec![tx(NANO, 1, BUY, 1.0, 1_000)]);
    settle(&mut a, &mut b);
    let shared = ids(&a).to_global(1);
    // B has valued the portfolio.
    let mut on_b = ledger(&b);
    // Valuing the portfolio clears what the first apply marked stale.
    on_b.invalidated_after.clear();
    on_b.historical_snapshots = vec![
        crate::model::portfolio::PortfolioSnapshot {
            portfolio_id: shared,
            timestamp: 1_500,
            value: 1.0,
            net_contributions: 1.0,
            realized_pnl: 0.0,
            unrealized_pnl: 0.0,
            is_complete: true,
        },
        crate::model::portfolio::PortfolioSnapshot {
            portfolio_id: shared,
            timestamp: 5_000,
            value: 2.0,
            net_contributions: 1.0,
            realized_pnl: 0.0,
            unrealized_pnl: 0.0,
            is_complete: true,
        },
    ];
    b.store
        .lock()
        .unwrap()
        .save_portfolio_ledger(&on_b)
        .unwrap();
    // A adds a transaction at time 4_000.
    let mut on_a = ledger(&a);
    on_a.transactions.push(tx(NANO + 1, 1, BUY, 1.0, 4_000));
    a.store
        .lock()
        .unwrap()
        .save_portfolio_ledger(&on_a)
        .unwrap();
    settle(&mut a, &mut b);
    let on_b = ledger(&b);
    assert_eq!(on_b.transactions.len(), 2);
    let kept: Vec<_> = on_b
        .historical_snapshots
        .iter()
        .map(|s| s.timestamp)
        .collect();
    assert_eq!(kept, [1_500], "the snapshot after the change is gone");
    assert_eq!(on_b.invalidated_after.get(&shared), Some(&4_000));
}

// MARK: Held ledgers

fn cloud_record(store: &Store, kind: EntityKind, sync_id: &str, revision: u64) -> RemoteRecord {
    let Load::Found(entity) = store.load_projected(kind, sync_id).unwrap() else {
        panic!("{sync_id} is not stored");
    };
    RemoteRecord {
        record: SyncRecord {
            name: RecordName::new(kind, sync_id).unwrap(),
            schema_version: SCHEMA_VERSION,
            revision,
            origin_device: "other-mac".into(),
            modified_ms: 5,
            payload: entity.payload.clone(),
        },
        value: serde_json::from_slice(&entity.payload).unwrap(),
        system_fields: vec![1],
    }
}

fn apply(store: &Store, records: Vec<RemoteRecord>, deletions: Vec<RecordName>) {
    let state = store.sync_state().unwrap().unwrap();
    let ctx = super::apply::ApplyCtx {
        device_id: state.device_id,
        device_name: state.device_name,
        now_ms: 100,
        date: "2026-10-20".into(),
    };
    let batch = RemoteBatch {
        id: 1,
        records,
        deletions,
    };
    super::apply::apply_batch(store, &mut super::scripts::NoScripts, &batch, &ctx).unwrap();
}

fn syncing_store() -> Store {
    let store = Store::open_in_memory().unwrap();
    let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
    state.enabled = true;
    store.save_sync_state(&state).unwrap();
    store
}

/// A source Mac's store with a portfolio, a purchase and a sale of it.
fn source() -> (Store, String, String, String) {
    let source = syncing_store();
    let snapshot = PortfolioLedgerSnapshot {
        portfolios: vec![Portfolio::new(1, "Main", 1_000)],
        transactions: vec![
            tx(NANO, 1, BUY, 2.0, 1_000),
            tx(NANO + 1, 1, SELL, 2.0, 2_000),
        ],
        ..Default::default()
    };
    source.save_portfolio_ledger(&snapshot).unwrap();
    let ids = source.id_space();
    (
        source,
        ids.to_global(1).to_string(),
        NANO.to_string(),
        (NANO + 1).to_string(),
    )
}

fn held_reasons(store: &Store) -> Vec<(String, String)> {
    store
        .inbox()
        .unwrap()
        .into_iter()
        .map(|e| (e.record_name, e.reason))
        .collect()
}

#[test]
fn a_sale_that_arrives_before_its_purchase_is_held_then_applied() {
    let (source, portfolio, buy, sell) = source();
    let receiver = syncing_store();
    let portfolio_record = cloud_record(&source, EntityKind::Portfolio, &portfolio, 1);
    let sell_record = cloud_record(&source, EntityKind::PortfolioTransaction, &sell, 1);
    let buy_record = cloud_record(&source, EntityKind::PortfolioTransaction, &buy, 1);

    apply(&receiver, vec![portfolio_record, sell_record], vec![]);
    let held = held_reasons(&receiver);
    assert_eq!(
        held,
        [(format!("portfoliotx.{sell}"), "ledger_invalid".to_string())]
    );
    assert_eq!(receiver.portfolio_ledger().unwrap().transactions.len(), 0);
    assert_eq!(receiver.conflicts().unwrap().len(), 1);
    // Holding a record must not upload anything.
    assert!(receiver.outbox().unwrap().is_empty());

    apply(&receiver, vec![buy_record], vec![]);
    assert!(held_reasons(&receiver).is_empty(), "the sale was released");
    assert_eq!(receiver.portfolio_ledger().unwrap().transactions.len(), 2);
    assert!(receiver.outbox().unwrap().is_empty());
}

#[test]
fn the_order_inside_one_batch_does_not_matter() {
    let (source, portfolio, buy, sell) = source();
    let receiver = syncing_store();
    let records = vec![
        cloud_record(&source, EntityKind::PortfolioTransaction, &sell, 1),
        cloud_record(&source, EntityKind::Portfolio, &portfolio, 1),
        cloud_record(&source, EntityKind::PortfolioTransaction, &buy, 1),
    ];
    apply(&receiver, records, vec![]);
    assert!(held_reasons(&receiver).is_empty());
    assert_eq!(receiver.portfolio_ledger().unwrap().transactions.len(), 2);
}

#[test]
fn a_transaction_that_arrives_before_its_portfolio_waits_for_it() {
    let (source, portfolio, buy, _) = source();
    let receiver = syncing_store();
    apply(
        &receiver,
        vec![cloud_record(
            &source,
            EntityKind::PortfolioTransaction,
            &buy,
            1,
        )],
        vec![],
    );
    assert_eq!(
        held_reasons(&receiver),
        [(format!("portfoliotx.{buy}"), "orphan".to_string())]
    );
    apply(
        &receiver,
        vec![cloud_record(&source, EntityKind::Portfolio, &portfolio, 1)],
        vec![],
    );
    assert!(held_reasons(&receiver).is_empty());
    assert_eq!(receiver.portfolio_ledger().unwrap().transactions.len(), 1);
}

#[test]
fn a_remote_delete_of_a_purchase_that_a_sale_needs_is_not_applied() {
    let (source, portfolio, buy, sell) = source();
    let receiver = syncing_store();
    apply(
        &receiver,
        vec![
            cloud_record(&source, EntityKind::Portfolio, &portfolio, 1),
            cloud_record(&source, EntityKind::PortfolioTransaction, &buy, 1),
            cloud_record(&source, EntityKind::PortfolioTransaction, &sell, 1),
        ],
        vec![],
    );
    assert_eq!(receiver.portfolio_ledger().unwrap().transactions.len(), 2);
    apply(
        &receiver,
        vec![],
        vec![RecordName::new(EntityKind::PortfolioTransaction, buy.clone()).unwrap()],
    );
    let after = receiver.portfolio_ledger().unwrap();
    assert_eq!(after.transactions.len(), 2, "the ledger stays possible");
    // The purchase goes back to iCloud instead of silently diverging.
    let queued: Vec<_> = receiver
        .outbox()
        .unwrap()
        .into_iter()
        .map(|row| row.sync_id)
        .collect();
    assert_eq!(queued, [buy]);
}

#[test]
fn an_edit_beats_a_remote_portfolio_delete_and_keeps_its_transactions() {
    let (source, portfolio, buy, sell) = source();
    let receiver = syncing_store();
    apply(
        &receiver,
        vec![
            cloud_record(&source, EntityKind::Portfolio, &portfolio, 1),
            cloud_record(&source, EntityKind::PortfolioTransaction, &buy, 1),
            cloud_record(&source, EntityKind::PortfolioTransaction, &sell, 1),
        ],
        vec![],
    );
    // The receiver renames the portfolio, then another Mac's delete arrives.
    let mut ledger = receiver.portfolio_ledger().unwrap();
    ledger.portfolios[0].name = "Renamed here".into();
    receiver.save_portfolio_ledger(&ledger).unwrap();
    apply(
        &receiver,
        vec![],
        vec![
            RecordName::new(EntityKind::Portfolio, portfolio.clone()).unwrap(),
            RecordName::new(EntityKind::PortfolioTransaction, buy.clone()).unwrap(),
            RecordName::new(EntityKind::PortfolioTransaction, sell.clone()).unwrap(),
        ],
    );
    let after = receiver.portfolio_ledger().unwrap();
    assert_eq!(after.portfolios.len(), 1);
    assert_eq!(after.transactions.len(), 2, "its ledger is kept with it");
}

#[test]
fn the_portfolios_switch_holds_records_until_it_is_back_on() {
    let (source, portfolio, buy, _) = source();
    let receiver = syncing_store();
    receiver
        .set_category_enabled(super::policy::Category::Portfolios, false)
        .unwrap();
    apply(
        &receiver,
        vec![
            cloud_record(&source, EntityKind::Portfolio, &portfolio, 1),
            cloud_record(&source, EntityKind::PortfolioTransaction, &buy, 1),
        ],
        vec![],
    );
    assert!(receiver.portfolio_ledger().unwrap().portfolios.is_empty());
    assert_eq!(held_reasons(&receiver).len(), 2);
    receiver
        .set_category_enabled(super::policy::Category::Portfolios, true)
        .unwrap();
    apply(&receiver, vec![], vec![]);
    assert_eq!(receiver.portfolio_ledger().unwrap().portfolios.len(), 1);
    assert_eq!(receiver.portfolio_ledger().unwrap().transactions.len(), 1);
}

// MARK: Cards follow the portfolio

#[test]
fn a_portfolio_card_in_a_watchlist_follows_the_portfolio_to_the_other_mac() {
    use crate::model::InstrumentId;
    use crate::model::portfolio_card::{CardKind, PortfolioCard};
    use crate::model::watchlist::{Watchlist, WatchlistEntry, WatchlistInstrument};

    let (_cloud, mut a, mut b) = pair();
    // B already has its own portfolio 1, which a naive id would collide with.
    save(&b, &[(1, "B's own")], vec![]);
    save(&a, &[(1, "A's main")], vec![]);
    let card = PortfolioCard::new(CardKind::Allocation, Some(1));
    let list = Watchlist {
        id: 10,
        name: "Cards".into(),
        entries: vec![WatchlistEntry::Instrument(WatchlistInstrument {
            id: 1,
            instrument: InstrumentId::new(DataSource::CoinMarketCap, "portfolio:allocation:1"),
            name: "Allocation".into(),
            label: "Allocation".into(),
            display_name: None,
            image_url: None,
            card: Some(card),
        })],
        display: Default::default(),
        is_favorites: false,
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    a.store
        .lock()
        .unwrap()
        .replace_documents(
            "watchlist",
            &[super::core_tests::Raw(serde_json::to_value(list).unwrap())],
        )
        .unwrap();
    settle(&mut a, &mut b);
    let shared = ids(&a).to_global(1);
    let on_b = b.store.lock().unwrap().raw_rows("watchlist").unwrap();
    let entry = &on_b[0]["entries"][0]["Instrument"];
    assert_eq!(entry["card"]["portfolio_id"], shared);
    assert_eq!(
        entry["instrument"]["symbol"],
        format!("portfolio:allocation:{shared}")
    );
    // The card points at A's portfolio on B, not at B's own portfolio 1.
    assert_eq!(local_id(&b, "A's main"), shared);
    assert_eq!(local_id(&b, "B's own"), 1);
    // And back on A it is the small id again.
    let on_a = a.store.lock().unwrap().raw_rows("watchlist").unwrap();
    assert_eq!(
        on_a[0]["entries"][0]["Instrument"]["card"]["portfolio_id"],
        1
    );
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}

#[test]
fn a_portfolio_card_in_a_layout_follows_the_portfolio_to_the_other_mac() {
    use serde_json::json;
    let (_cloud, mut a, mut b) = pair();
    save(&b, &[(1, "B's own")], vec![]);
    save(&a, &[(1, "A's main")], vec![]);
    let indicators = serde_json::to_value(crate::model::IndicatorConfig::default()).unwrap();
    let source = serde_json::to_value(DataSource::CoinMarketCap).unwrap();
    let layout = json!({
        "id": 30, "name": "Dashboard", "timeframe": "Day", "created_at_ms": 1,
        "candle_count": 0,
        "charts": [{
            "id": 1, "symbol": "portfolio:allocation:1", "source": source, "log_scale": false,
            "y_zoom": 1.0, "indicators": indicators, "drawings": [],
            "card": {"kind": "allocation", "portfolio_id": 1, "range": "OneMonth"},
        }],
    });
    assert!(crate::store::sync::row_decodes(
        "saved_view",
        &layout.to_string()
    ));
    a.store
        .lock()
        .unwrap()
        .replace_documents("saved_view", &[super::core_tests::Raw(layout)])
        .unwrap();
    settle(&mut a, &mut b);
    let shared = ids(&a).to_global(1);
    let on_b = b.store.lock().unwrap().raw_rows("saved_view").unwrap();
    assert_eq!(on_b[0]["charts"][0]["card"]["portfolio_id"], shared);
    assert_eq!(
        on_b[0]["charts"][0]["symbol"],
        format!("portfolio:allocation:{shared}")
    );
    let on_a = a.store.lock().unwrap().raw_rows("saved_view").unwrap();
    assert_eq!(on_a[0]["charts"][0]["card"]["portfolio_id"], 1);
    assert_eq!(on_a[0]["charts"][0]["symbol"], "portfolio:allocation:1");
    assert_eq!(a.outbox().len() + b.outbox().len(), 0);
}
