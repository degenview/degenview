use super::*;
use crate::model::DataSource;
use crate::model::portfolio::{PortfolioAsset, PortfolioSource, PortfolioTransactionType};
use crate::store::Store;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("open store")))
}

fn transaction(id: u64, portfolio_id: u64, quantity: f64) -> PortfolioTransaction {
    PortfolioTransaction {
        id,
        portfolio_id,
        asset: PortfolioAsset {
            key: "binance:BTCUSDT".into(),
            symbol: "BTC".into(),
            name: "Bitcoin".into(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        },
        kind: PortfolioTransactionType::Buy,
        quantity,
        price: Some(50_000.0),
        price_currency: PortfolioCurrency::Usd,
        fee: 0.0,
        fee_currency: PortfolioCurrency::Usd,
        timestamp: 1_000,
        notes: String::new(),
        source: PortfolioSource::Manual,
        external_transaction_id: None,
        created_at: 1_000,
        updated_at: 1_000,
    }
}

#[test]
fn reload_picks_up_what_sync_wrote_and_repairs_a_vanished_selection() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    assert!(host.snapshot().portfolios.is_empty());
    {
        let guard = store.lock().unwrap();
        let snapshot = PortfolioLedgerSnapshot {
            portfolios: vec![Portfolio::new(7, "From iCloud", 1)],
            transactions: vec![transaction(9, 7, 1.0)],
            selected_portfolio_id: Some(99),
            ..Default::default()
        };
        guard.save_portfolio_ledger(&snapshot).unwrap();
    }
    host.reload();
    assert_eq!(host.snapshot().portfolios.len(), 1);
    assert_eq!(host.snapshot().transactions.len(), 1);
    assert_eq!(host.snapshot().selected_portfolio_id, Some(7));
}

#[test]
fn mutation_persists_before_publishing() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    let id = host
        .create_portfolio("Long-term", PortfolioCurrency::Eur)
        .expect("create");
    host.add_transaction(transaction(40, id, 1.0))
        .expect("add transaction");

    assert_eq!(
        host.snapshot().portfolios[0].base_currency,
        PortfolioCurrency::Eur
    );
    let persisted = store.lock().unwrap().portfolio_ledger().expect("reload");
    assert_eq!(persisted, *host.snapshot());
}

#[test]
fn rejected_mutation_changes_neither_memory_nor_store() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    let id = host
        .create_portfolio("Main", PortfolioCurrency::Usd)
        .expect("create");
    let baseline = host.snapshot().clone();

    let error = host
        .add_transaction(transaction(1, id, 0.0))
        .expect_err("zero quantity");
    assert!(matches!(
        error,
        PortfolioHostError::Mutation(PortfolioError::InvalidQuantity)
    ));
    assert_eq!(host.snapshot(), &baseline);
    assert_eq!(store.lock().unwrap().portfolio_ledger().unwrap(), baseline);
}

#[test]
fn selection_and_delete_are_durable() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    let first = host
        .create_portfolio("First", PortfolioCurrency::Usd)
        .unwrap();
    let second = host
        .create_portfolio("Second", PortfolioCurrency::Usd)
        .unwrap();
    host.select(Some(first)).unwrap();
    host.delete_portfolio(first).unwrap();

    assert_eq!(host.snapshot().selected_portfolio_id, Some(second));
    assert_eq!(
        store
            .lock()
            .unwrap()
            .portfolio_ledger()
            .unwrap()
            .selected_portfolio_id,
        Some(second)
    );
}

#[test]
fn remapping_an_asset_moves_its_transactions_durably_and_only_in_scope() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    let first = host
        .create_portfolio("First", PortfolioCurrency::Usd)
        .unwrap();
    let second = host
        .create_portfolio("Second", PortfolioCurrency::Usd)
        .unwrap();
    host.add_transaction(transaction(10, first, 1.0)).unwrap();
    host.add_transaction(transaction(11, second, 2.0)).unwrap();
    let mut destination = transaction(0, first, 0.0).asset;
    destination.key = "coinbase:BTC-USD".into();
    destination.source = DataSource::Coinbase;

    let scope: HashSet<u64> = [first].into_iter().collect();
    host.remap_asset("binance:BTCUSDT", &destination, &scope)
        .unwrap();
    let key_of = |id: u64| {
        host.snapshot()
            .transactions
            .iter()
            .find(|transaction| transaction.id == id)
            .map(|transaction| transaction.asset.key.clone())
    };
    assert_eq!(key_of(10).as_deref(), Some("coinbase:BTC-USD"));
    assert_eq!(
        key_of(11).as_deref(),
        Some("binance:BTCUSDT"),
        "other portfolios are untouched"
    );
    let saved = store.lock().unwrap().portfolio_ledger().unwrap();
    assert!(
        saved
            .transactions
            .iter()
            .any(|t| t.id == 10 && t.asset.key == "coinbase:BTC-USD"),
        "persisted before it was published"
    );
    // Remapping to the same market, or an asset nobody holds, changes nothing and is not an error.
    host.remap_asset("coinbase:BTC-USD", &destination, &scope)
        .unwrap();
    host.remap_asset("none", &destination, &scope).unwrap();
    assert!(host.last_error().is_none());
}

fn rebuilt(portfolio_id: u64, basis: u64) -> HistoryResult {
    HistoryResult {
        portfolio_id,
        from: 0,
        snapshots: Some(vec![PortfolioSnapshot {
            portfolio_id,
            timestamp: 0,
            value: 1.0,
            net_contributions: 1.0,
            realized_pnl: 0.0,
            unrealized_pnl: 0.0,
            is_complete: true,
        }]),
        hopeless: false,
        incomplete: false,
        basis,
    }
}

#[test]
fn a_rebuild_is_stored_only_for_the_transactions_it_was_computed_from() {
    let mut host = PortfolioHost::load(store());
    let portfolio = host
        .create_portfolio("Main", PortfolioCurrency::Usd)
        .unwrap();
    host.add_transaction(transaction(1, portfolio, 1.0))
        .unwrap();
    let own = |host: &PortfolioHost| transactions_fingerprint(&host.ledger.snapshot().transactions);
    let before = own(&host);

    // An edit lands while the rebuild runs, in the same day: its result is stale.
    host.add_transaction(transaction(2, portfolio, 2.0))
        .unwrap();
    host.accept_rebuilt(rebuilt(portfolio, before), 10);
    assert!(host.ledger.snapshot().historical_snapshots.is_empty());
    assert!(
        host.ledger
            .snapshot()
            .invalidated_after
            .contains_key(&portfolio)
    );

    // A result for what the portfolio holds now is stored and settles the marker.
    host.accept_rebuilt(rebuilt(portfolio, own(&host)), 10);
    assert_eq!(host.ledger.snapshot().historical_snapshots.len(), 1);
    assert!(
        !host
            .ledger
            .snapshot()
            .invalidated_after
            .contains_key(&portfolio)
    );
}

fn point(portfolio_id: u64, timestamp: i64, value: f64) -> PortfolioSnapshot {
    PortfolioSnapshot {
        portfolio_id,
        timestamp,
        value,
        net_contributions: value,
        realized_pnl: 0.0,
        unrealized_pnl: 0.0,
        is_complete: true,
    }
}

/// The row ids of the two big tables: a delete-and-reinsert gives every row a new one.
fn row_ids(store: &SharedStore) -> (String, String) {
    let guard = store.lock().unwrap();
    let ids = |table: &str| -> String {
        guard
            .raw_connection()
            .query_row(
                &format!("SELECT COALESCE(group_concat(rowid), '') FROM {table}"),
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    (ids("portfolio_transaction"), ids("portfolio_snapshot"))
}

#[test]
fn selecting_a_portfolio_does_not_rewrite_the_ledger() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    let first = host.create_portfolio("A", PortfolioCurrency::Usd).unwrap();
    let second = host.create_portfolio("B", PortfolioCurrency::Usd).unwrap();
    host.add_transaction(transaction(1, first, 1.0)).unwrap();
    host.store_snapshots(vec![point(first, 10, 1.0)], first, 0)
        .unwrap();
    let before = row_ids(&store);

    host.select(Some(second)).unwrap();
    host.select(Some(first)).unwrap();
    assert_eq!(
        row_ids(&store),
        before,
        "no transaction or snapshot row was rewritten"
    );
    assert!(matches!(
        host.select(Some(999)),
        Err(PortfolioHostError::Mutation(
            PortfolioError::PortfolioNotFound
        ))
    ));
    host.select(Some(second)).unwrap();
    assert_eq!(
        PortfolioHost::load(store).snapshot().selected_portfolio_id,
        Some(second)
    );
}

#[test]
fn a_rebuild_replaces_only_its_portfolios_later_history() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    let first = host.create_portfolio("A", PortfolioCurrency::Usd).unwrap();
    let second = host.create_portfolio("B", PortfolioCurrency::Usd).unwrap();
    host.add_transaction(transaction(1, first, 1.0)).unwrap();
    host.store_snapshots(
        vec![
            point(first, 0, 1.0),
            point(first, 10, 2.0),
            point(first, 20, 3.0),
        ],
        first,
        0,
    )
    .unwrap();
    host.store_snapshots(vec![point(second, 5, 7.0)], second, 0)
        .unwrap();
    let other_before: String = store
        .lock()
        .unwrap()
        .raw_connection()
        .query_row(
            "SELECT group_concat(rowid) FROM portfolio_snapshot WHERE portfolio_id = ?1",
            [second.to_string()],
            |row| row.get(0),
        )
        .unwrap();

    host.store_snapshots(vec![point(first, 10, 9.0)], first, 10)
        .unwrap();

    let other_after: String = store
        .lock()
        .unwrap()
        .raw_connection()
        .query_row(
            "SELECT group_concat(rowid) FROM portfolio_snapshot WHERE portfolio_id = ?1",
            [second.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        other_before, other_after,
        "the other portfolio was not touched"
    );
    // What is in memory is what a restart reads.
    let stored = store.lock().unwrap().portfolio_ledger().unwrap();
    assert_eq!(
        stored.historical_snapshots,
        host.snapshot().historical_snapshots
    );
    let mine: Vec<(i64, f64)> = stored
        .historical_snapshots
        .iter()
        .filter(|p| p.portfolio_id == first)
        .map(|p| (p.timestamp, p.value))
        .collect();
    assert_eq!(mine, [(0, 1.0), (10, 9.0)], "the stale tail is gone");
    assert!(!stored.invalidated_after.contains_key(&first));
}

#[test]
fn a_preference_that_cannot_be_saved_is_reported() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    store
        .lock()
        .unwrap()
        .raw_connection()
        .execute_batch("DROP TABLE setting")
        .unwrap();
    assert!(host.last_error().is_none());
    host.set_privacy_mode(true);
    assert!(matches!(
        host.last_error(),
        Some(PortfolioHostError::Persistence(_))
    ));
    host.clear_error();
    host.set_reporting_currency(None, PortfolioCurrency::Chf);
    assert!(host.last_error().is_some());
}

#[test]
fn privacy_and_reporting_currency_are_remembered_per_selection() {
    let store = store();
    let mut host = PortfolioHost::load(store.clone());
    let euro = host
        .create_portfolio("Euros", PortfolioCurrency::Eur)
        .unwrap();
    assert!(!host.privacy_mode());
    host.set_privacy_mode(true);
    assert!(host.privacy_mode());

    assert_eq!(
        host.reporting_currency_for(Some(euro)),
        PortfolioCurrency::Eur,
        "its own currency until chosen"
    );
    assert_eq!(host.reporting_currency_for(None), PortfolioCurrency::Usd);
    host.set_reporting_currency(Some(euro), PortfolioCurrency::Gbp);
    host.set_reporting_currency(None, PortfolioCurrency::Chf);
    assert_eq!(
        host.reporting_currency_for(Some(euro)),
        PortfolioCurrency::Gbp
    );
    assert_eq!(host.reporting_currency_for(None), PortfolioCurrency::Chf);

    // A new window reads the same answers from the store.
    let again = PortfolioHost::load(store);
    assert!(again.privacy_mode());
    assert_eq!(
        again.reporting_currency_for(Some(euro)),
        PortfolioCurrency::Gbp
    );
    assert_eq!(
        again.reporting_currency_for(Some(999)),
        PortfolioCurrency::Usd,
        "unknown selection"
    );
}
