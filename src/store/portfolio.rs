//! Portfolio ledger storage. `PortfolioLedger` persists whole snapshots, so each save replaces
//! the ledger tables in one transaction. Ported from `AppDatabase+Portfolio.swift`.
#![allow(dead_code)] // transient until the portfolio ledger host wires these

use crate::model::portfolio::{Portfolio, PortfolioLedgerSnapshot};
use crate::store::{Document, Store};
use rusqlite::params;
use serde::Serialize;

const KEY_SELECTED: &str = "portfolio.selectedPortfolioID";
const KEY_INVALIDATED_AFTER: &str = "portfolio.invalidatedAfter";

impl Document for Portfolio {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    /// Unlike the document list, a transaction or snapshot that no longer decodes fails the
    /// read: dropping one would silently change every balance after it.
    pub fn portfolio_ledger(&self) -> rusqlite::Result<PortfolioLedgerSnapshot> {
        Ok(PortfolioLedgerSnapshot {
            portfolios: self.documents_strict("portfolio")?,
            transactions: self
                .payloads_strict("SELECT payload FROM portfolio_transaction ORDER BY position")?,
            historical_snapshots: self
                .payloads_strict("SELECT payload FROM portfolio_snapshot ORDER BY rowid")?,
            selected_portfolio_id: self.setting_strict(KEY_SELECTED)?,
            invalidated_after: self
                .setting_strict(KEY_INVALIDATED_AFTER)?
                .unwrap_or_default(),
        })
    }

    pub fn save_portfolio_ledger(
        &self,
        snapshot: &PortfolioLedgerSnapshot,
    ) -> rusqlite::Result<()> {
        self.in_transaction(|| {
            self.replace_documents("portfolio", &snapshot.portfolios)?;

            let rows = snapshot
                .transactions
                .iter()
                .map(|value| {
                    Ok(TransactionRow {
                        id: value.id.to_string(),
                        portfolio_id: value.portfolio_id.to_string(),
                        timestamp: value.timestamp,
                        payload: json(value)?,
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            self.replace_transaction_rows(&rows)?;

            self.connection
                .execute("DELETE FROM portfolio_snapshot", [])?;
            let mut history = self.connection.prepare(
                "INSERT INTO portfolio_snapshot (portfolio_id, timestamp, payload) VALUES (?1, ?2, ?3)",
            )?;
            for value in &snapshot.historical_snapshots {
                history.execute(params![
                    value.portfolio_id.to_string(),
                    value.timestamp,
                    json(value)?,
                ])?;
            }

            match snapshot.selected_portfolio_id {
                Some(id) => self.set_setting(KEY_SELECTED, &id)?,
                None => self.remove_setting(KEY_SELECTED)?,
            }
            if snapshot.invalidated_after.is_empty() {
                self.remove_setting(KEY_INVALIDATED_AFTER)?;
            } else {
                self.set_setting(KEY_INVALIDATED_AFTER, &snapshot.invalidated_after)?;
            }
            Ok(())
        })
    }
}

/// One row of `portfolio_transaction`, ready to store.
struct TransactionRow {
    id: String,
    portfolio_id: String,
    timestamp: i64,
    payload: String,
}

impl Store {
    /// Replaces every transaction, queueing the differences for sync in the same transaction.
    fn replace_transaction_rows(&self, rows: &[TransactionRow]) -> rusqlite::Result<()> {
        let old = self.tracked_old_transactions()?;
        self.connection
            .execute("DELETE FROM portfolio_transaction", [])?;
        let mut insert = self.connection.prepare(
            "INSERT INTO portfolio_transaction (id, portfolio_id, timestamp, position, payload)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (position, row) in rows.iter().enumerate() {
            insert.execute(params![
                row.id,
                row.portfolio_id,
                row.timestamp,
                position as i64,
                row.payload,
            ])?;
        }
        drop(insert);
        if let Some(old) = old {
            let new: Vec<(String, String)> = rows
                .iter()
                .map(|row| (row.id.clone(), row.payload.clone()))
                .collect();
            self.track_transactions(&old, &new)?;
        }
        Ok(())
    }

    /// Replaces the transactions with rows that were already checked against the model, exactly
    /// as stored (a typed round trip could add fields an older Mac does not know).
    pub(crate) fn replace_raw_transactions(
        &self,
        rows: &[serde_json::Value],
    ) -> rusqlite::Result<()> {
        let rows = rows
            .iter()
            .map(|row| {
                let number = |key: &str| row.get(key).and_then(serde_json::Value::as_u64);
                Ok(TransactionRow {
                    id: number("id").map(|n| n.to_string()).ok_or_else(missing)?,
                    portfolio_id: number("portfolio_id")
                        .map(|n| n.to_string())
                        .ok_or_else(missing)?,
                    timestamp: row
                        .get("timestamp")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default(),
                    payload: row.to_string(),
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        self.in_savepoint("replace_transactions", || {
            self.replace_transaction_rows(&rows)
        })
    }

    /// Valuation history is derived from the ledger, so a change to the ledger invalidates it
    /// from the earliest affected moment: later snapshots go, and the portfolio is marked stale.
    pub(crate) fn invalidate_portfolio_history(
        &self,
        from: &std::collections::BTreeMap<u64, i64>,
    ) -> rusqlite::Result<()> {
        let mut stale: std::collections::HashMap<u64, i64> = self
            .setting_strict(KEY_INVALIDATED_AFTER)?
            .unwrap_or_default();
        for (portfolio, moment) in from {
            stale
                .entry(*portfolio)
                .and_modify(|current| *current = (*current).min(*moment))
                .or_insert(*moment);
            self.connection.execute(
                "DELETE FROM portfolio_snapshot WHERE portfolio_id = ?1 AND timestamp >= ?2",
                params![portfolio.to_string(), moment],
            )?;
        }
        self.set_setting(KEY_INVALIDATED_AFTER, &stale)
    }

    /// A deleted portfolio's valuation history, and the selection if it pointed there.
    pub(crate) fn forget_portfolio_history(
        &self,
        portfolios: &std::collections::BTreeSet<u64>,
    ) -> rusqlite::Result<()> {
        let mut stale: std::collections::HashMap<u64, i64> = self
            .setting_strict(KEY_INVALIDATED_AFTER)?
            .unwrap_or_default();
        let selected: Option<u64> = self.setting_strict(KEY_SELECTED)?;
        for portfolio in portfolios {
            stale.remove(portfolio);
            self.connection.execute(
                "DELETE FROM portfolio_snapshot WHERE portfolio_id = ?1",
                [portfolio.to_string()],
            )?;
            if selected == Some(*portfolio) {
                self.remove_setting(KEY_SELECTED)?;
            }
        }
        if stale.is_empty() {
            self.remove_setting(KEY_INVALIDATED_AFTER)
        } else {
            self.set_setting(KEY_INVALIDATED_AFTER, &stale)
        }
    }
}

fn missing() -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(
        "a transaction has no numeric id",
    )))
}

fn json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{
        PortfolioAsset, PortfolioCurrency, PortfolioSnapshot, PortfolioTransaction,
    };
    use std::collections::HashMap;

    fn portfolio(id: u64) -> Portfolio {
        Portfolio::new(id, "Main", 1_000)
    }

    fn transaction(id: u64, portfolio_id: u64) -> PortfolioTransaction {
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
            kind: crate::model::portfolio::PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(50_000.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 2_000,
            notes: String::new(),
            source: crate::model::portfolio::PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 2_000,
            updated_at: 2_000,
        }
    }

    #[test]
    fn ledger_round_trips() {
        let store = Store::open_in_memory().expect("open");
        let snapshot = PortfolioLedgerSnapshot {
            portfolios: vec![portfolio(3)],
            transactions: vec![transaction(9, 3)],
            historical_snapshots: vec![PortfolioSnapshot {
                portfolio_id: 3,
                timestamp: 2_500,
                value: 50_000.0,
                net_contributions: 50_000.0,
                realized_pnl: 0.0,
                unrealized_pnl: 0.0,
                is_complete: true,
            }],
            selected_portfolio_id: Some(3),
            invalidated_after: HashMap::from([(3, 2_500)]),
        };
        store.save_portfolio_ledger(&snapshot).expect("save");

        let loaded = store.portfolio_ledger().expect("load");
        assert_eq!(loaded.portfolios.len(), 1);
        assert_eq!(loaded.transactions.len(), 1);
        assert_eq!(loaded.historical_snapshots.len(), 1);
        assert_eq!(loaded.selected_portfolio_id, Some(3));
        assert_eq!(loaded.invalidated_after.get(&3), Some(&2_500));
    }

    // MARK: Sync tracking

    use crate::store::sync::Load;
    use crate::sync::ids::{GLOBAL_BASE, IdSpace};
    use crate::sync::policy::EntityKind;

    fn syncing_store() -> Store {
        let store = Store::open_in_memory().expect("open");
        let mut state = store.ensure_sync_state("Mac", "hw").expect("state");
        state.enabled = true;
        store.save_sync_state(&state).expect("enable");
        store
    }

    fn queued(store: &Store) -> Vec<(String, String, String)> {
        let mut rows: Vec<_> = store
            .outbox()
            .expect("outbox")
            .into_iter()
            .map(|row| {
                (
                    row.kind.prefix().to_string(),
                    row.sync_id,
                    format!("{:?}/{:?}", row.op, row.state),
                )
            })
            .collect();
        rows.sort();
        rows
    }

    /// Pretends the cloud accepted this outbox entry.
    fn complete(store: &Store, row: &crate::store::sync::OutboxRow) {
        match row.op {
            crate::store::sync::OutboxOp::Upsert => {
                let Load::Found(entity) = store.load_projected(row.kind, &row.sync_id).unwrap()
                else {
                    panic!("nothing to upload for {}", row.sync_id);
                };
                store
                    .ack_saved(row.seq, row.kind, &row.sync_id, &entity.hash, &[1], 0)
                    .unwrap();
            }
            crate::store::sync::OutboxOp::Delete => {
                store.ack_deleted(row.seq, row.kind, &row.sync_id).unwrap();
            }
        }
    }

    fn ledger(portfolios: &[u64], transactions: &[(u64, u64)]) -> PortfolioLedgerSnapshot {
        PortfolioLedgerSnapshot {
            portfolios: portfolios.iter().map(|id| portfolio(*id)).collect(),
            transactions: transactions
                .iter()
                .map(|(id, portfolio_id)| transaction(*id, *portfolio_id))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn small_ids_sync_under_this_databases_tag() {
        let store = syncing_store();
        let ids = store.id_space();
        store
            .save_portfolio_ledger(&ledger(&[1], &[(2, 1)]))
            .expect("save");
        let global_portfolio = ids.to_global(1).to_string();
        assert!(ids.to_global(1) >= GLOBAL_BASE);
        assert_eq!(
            queued(&store),
            [
                (
                    "portfolio".into(),
                    global_portfolio.clone(),
                    "Upsert/Pending".into()
                ),
                (
                    "portfoliotx".into(),
                    ids.to_global(2).to_string(),
                    "Upsert/Pending".into()
                ),
            ]
        );
        let Load::Found(found) = store
            .load_projected(EntityKind::Portfolio, &global_portfolio)
            .expect("load")
        else {
            panic!("portfolio not found");
        };
        let payload: serde_json::Value = serde_json::from_slice(&found.payload).unwrap();
        assert_eq!(payload["id"], ids.to_global(1));
        assert_eq!(found.local_id, "1");
        // The row on disk keeps its small id.
        assert_eq!(store.portfolio_ledger().unwrap().portfolios[0].id, 1);
    }

    #[test]
    fn two_databases_do_not_share_a_portfolio_one() {
        let (a, b) = (syncing_store(), syncing_store());
        assert_ne!(a.id_space(), b.id_space());
        a.save_portfolio_ledger(&ledger(&[1], &[])).unwrap();
        b.save_portfolio_ledger(&ledger(&[1], &[])).unwrap();
        assert_ne!(queued(&a), queued(&b));
    }

    #[test]
    fn nanosecond_transaction_ids_sync_unchanged() {
        let store = syncing_store();
        let id = 1_700_000_000_000_000_000;
        store
            .save_portfolio_ledger(&ledger(&[1], &[(id, 1)]))
            .unwrap();
        assert!(
            queued(&store)
                .iter()
                .any(|row| row.0 == "portfoliotx" && row.1 == id.to_string())
        );
    }

    #[test]
    fn saving_the_same_ledger_again_queues_nothing_new() {
        let store = syncing_store();
        store
            .save_portfolio_ledger(&ledger(&[1], &[(2, 1)]))
            .unwrap();
        let before = store.outbox().unwrap();
        store
            .save_portfolio_ledger(&ledger(&[1], &[(2, 1)]))
            .unwrap();
        assert_eq!(store.outbox().unwrap(), before);
    }

    #[test]
    fn editing_and_deleting_one_transaction_queues_just_that_one() {
        let store = syncing_store();
        store
            .save_portfolio_ledger(&ledger(&[1], &[(2, 1), (3, 1)]))
            .unwrap();
        let ids = store.id_space();
        // Pretend the first upload completed.
        for row in store.outbox().unwrap() {
            complete(&store, &row);
        }
        let mut edited = ledger(&[1], &[(2, 1), (3, 1)]);
        edited.transactions[0].quantity = 5.0;
        store.save_portfolio_ledger(&edited).unwrap();
        assert_eq!(
            queued(&store),
            [(
                "portfoliotx".into(),
                ids.to_global(2).to_string(),
                "Upsert/Pending".into()
            )]
        );
        for row in store.outbox().unwrap() {
            complete(&store, &row);
        }
        store
            .save_portfolio_ledger(&ledger(&[1], &[(3, 1)]))
            .unwrap();
        assert_eq!(
            queued(&store),
            [(
                "portfoliotx".into(),
                ids.to_global(2).to_string(),
                "Delete/Pending".into()
            )]
        );
    }

    #[test]
    fn deleting_a_portfolio_deletes_its_transactions_without_a_mass_delete_hold() {
        let store = syncing_store();
        let transactions: Vec<_> = (10..20).map(|id| (id, 1)).collect();
        store
            .save_portfolio_ledger(&ledger(&[1, 2], &transactions))
            .unwrap();
        for row in store.outbox().unwrap() {
            complete(&store, &row);
        }
        store.save_portfolio_ledger(&ledger(&[2], &[])).unwrap();
        let rows = queued(&store);
        assert_eq!(rows.len(), 11, "{rows:?}");
        assert!(rows.iter().all(|row| row.2 == "Delete/Pending"), "{rows:?}");
    }

    #[test]
    fn reconcile_finds_ledgers_written_before_sync_was_on() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_portfolio_ledger(&ledger(&[1], &[(2, 1)]))
            .unwrap();
        let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
        state.enabled = true;
        store.save_sync_state(&state).unwrap();
        assert!(queued(&store).is_empty());
        let done = store.reconcile_stored().unwrap();
        assert_eq!(done.upserts, 2);
        let ids: IdSpace = store.id_space();
        assert_eq!(
            queued(&store)
                .iter()
                .map(|row| row.1.clone())
                .collect::<Vec<_>>(),
            [ids.to_global(1).to_string(), ids.to_global(2).to_string()]
        );
    }

    #[test]
    fn the_portfolios_switch_stops_tracking() {
        let store = syncing_store();
        store
            .set_category_enabled(crate::sync::policy::Category::Portfolios, false)
            .unwrap();
        store
            .save_portfolio_ledger(&ledger(&[1], &[(2, 1)]))
            .unwrap();
        assert!(queued(&store).is_empty());
    }

    #[test]
    fn an_empty_ledger_is_clean() {
        let store = Store::open_in_memory().expect("open");
        store
            .save_portfolio_ledger(&PortfolioLedgerSnapshot::default())
            .expect("save empty");
        let loaded = store.portfolio_ledger().expect("load");
        assert!(loaded.portfolios.is_empty());
        assert_eq!(loaded.selected_portfolio_id, None);
        assert!(loaded.invalidated_after.is_empty());
    }

    #[test]
    fn unreadable_ledger_setting_fails_the_load() {
        let store = Store::open_in_memory().expect("open");
        store
            .connection
            .execute(
                "INSERT INTO setting (key, value) VALUES ('portfolio.invalidatedAfter', 'bad')",
                [],
            )
            .expect("seed");
        assert!(store.portfolio_ledger().is_err());
    }
}
