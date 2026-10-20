//! Paper-trading storage. `PaperTradingEngine` persists whole snapshots, so each save replaces
//! the paper tables in one transaction. Ported from `AppDatabase+PaperTrading.swift`.
#![allow(dead_code)] // transient until the paper trading host wires these

use crate::model::paper::{PaperAccount, PaperTradingSnapshot};
use crate::store::{Document, Store};
use rusqlite::params;
use serde::Serialize;

const KEY_SELECTED_ACCOUNT: &str = "paper.selectedAccountID";
const KEY_QUOTES: &str = "paper.quotes";

impl Document for PaperAccount {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    /// A row that no longer decodes fails the read: dropping a fill or order would silently
    /// change the account's cash and positions.
    pub fn paper_trading(&self) -> rusqlite::Result<PaperTradingSnapshot> {
        Ok(PaperTradingSnapshot {
            accounts: self.documents_strict("paper_account")?,
            orders: self.payloads_strict("SELECT payload FROM paper_order ORDER BY rowid")?,
            fills: self.payloads_strict("SELECT payload FROM paper_fill ORDER BY rowid")?,
            positions: self.payloads_strict("SELECT payload FROM paper_position ORDER BY rowid")?,
            order_events: self
                .payloads_strict("SELECT payload FROM paper_order_event ORDER BY rowid")?,
            closed_trades: self
                .payloads_strict("SELECT payload FROM paper_closed_trade ORDER BY rowid")?,
            journal: self.payloads_strict("SELECT payload FROM paper_journal ORDER BY rowid")?,
            selected_account_id: self.setting_strict(KEY_SELECTED_ACCOUNT)?,
            quotes: self.setting_strict(KEY_QUOTES)?.unwrap_or_default(),
        })
    }

    pub fn save_paper_trading(&self, snapshot: &PaperTradingSnapshot) -> rusqlite::Result<()> {
        self.in_transaction(|| {
            self.replace_documents("paper_account", &snapshot.accounts)?;
            self.replace_paper_rows("paper_order", &snapshot.orders, |order| order.account_id)?;
            self.replace_paper_rows("paper_fill", &snapshot.fills, |fill| fill.account_id)?;
            self.replace_paper_rows("paper_position", &snapshot.positions, |position| {
                position.account_id
            })?;
            self.replace_paper_rows("paper_order_event", &snapshot.order_events, |event| {
                event.account_id
            })?;
            self.replace_paper_rows("paper_closed_trade", &snapshot.closed_trades, |trade| {
                trade.account_id
            })?;
            self.replace_paper_rows("paper_journal", &snapshot.journal, |entry| entry.account_id)?;

            match snapshot.selected_account_id {
                Some(id) => self.set_setting(KEY_SELECTED_ACCOUNT, &id)?,
                None => self.remove_setting(KEY_SELECTED_ACCOUNT)?,
            }
            if snapshot.quotes.is_empty() {
                self.remove_setting(KEY_QUOTES)?;
            } else {
                self.set_setting(KEY_QUOTES, &snapshot.quotes)?;
            }
            self.track_paper(snapshot)?;
            Ok(())
        })
    }

    /// Replaces one account-scoped collection, kept in engine order via the rowid.
    fn replace_paper_rows<T: Serialize>(
        &self,
        table: &str,
        items: &[T],
        account_id: impl Fn(&T) -> u64,
    ) -> rusqlite::Result<()> {
        self.connection
            .execute(&format!("DELETE FROM {table}"), [])?;
        let mut statement = self.connection.prepare(&format!(
            "INSERT INTO {table} (account_id, payload) VALUES (?1, ?2)"
        ))?;
        for item in items {
            statement.execute(params![account_id(item).to_string(), json(item)?])?;
        }
        Ok(())
    }
}

fn json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::paper::PaperQuote;
    use std::collections::HashMap;

    #[test]
    fn paper_snapshot_round_trips() {
        let store = Store::open_in_memory().expect("open");
        let snapshot = PaperTradingSnapshot {
            accounts: vec![PaperAccount::new(3, "Main", 1_000)],
            selected_account_id: Some(3),
            quotes: HashMap::from([(
                "binance:BTCUSDT".to_owned(),
                PaperQuote {
                    instrument_key: "binance:BTCUSDT".to_owned(),
                    bid: Some(49_999.0),
                    ask: Some(50_001.0),
                    last: Some(50_000.0),
                    timestamp: 2_000,
                    is_market_open: true,
                },
            )]),
            ..Default::default()
        };
        store.save_paper_trading(&snapshot).expect("save");

        let loaded = store.paper_trading().expect("load");
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].id, 3);
        assert_eq!(loaded.selected_account_id, Some(3));
        assert_eq!(loaded.quotes.len(), 1);
        assert_eq!(loaded.quotes["binance:BTCUSDT"].last, Some(50_000.0));
    }

    // MARK: Sync tracking

    use crate::model::paper::{
        PaperCurrency, PaperInstrument, PaperOrderRequest, PaperOrderSide, PaperOrderType,
        PaperTimeInForce,
    };
    use crate::model::paper_engine::PaperTradingEngine;
    use crate::store::sync::{Load, OutboxOp, OutboxState};
    use crate::sync::policy::EntityKind;

    fn syncing_store() -> Store {
        let store = Store::open_in_memory().expect("open");
        let mut state = store.ensure_sync_state("Mac", "hw").expect("state");
        state.enabled = true;
        store.save_sync_state(&state).expect("enable");
        store
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64)
    }

    fn quote(last: f64, timestamp: i64) -> PaperQuote {
        PaperQuote {
            instrument_key: "Binance:BTCUSDT".to_owned(),
            bid: Some(last - 0.5),
            ask: Some(last + 0.5),
            last: Some(last),
            timestamp,
            is_market_open: true,
        }
    }

    fn market_buy(account_id: u64) -> PaperOrderRequest {
        PaperOrderRequest {
            account_id,
            instrument: PaperInstrument::chart(
                "BTCUSDT",
                "BTCUSDT",
                crate::model::DataSource::Binance,
            ),
            side: PaperOrderSide::Buy,
            kind: PaperOrderType::Market,
            quantity: 1.0,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        }
    }

    /// A snapshot with `names` as accounts, the first one holding a fill.
    fn traded(names: &[&str]) -> (PaperTradingSnapshot, Vec<u64>) {
        let mut engine = PaperTradingEngine::new(PaperTradingSnapshot::default());
        let accounts: Vec<u64> = names
            .iter()
            .map(|name| {
                engine
                    .create_account(name, PaperCurrency::Usd, 100_000.0, Default::default())
                    .unwrap()
            })
            .collect();
        engine.process(quote(100.0, now())).unwrap();
        engine.submit(market_buy(accounts[0])).unwrap();
        (engine.snapshot().clone(), accounts)
    }

    fn outbox_ids(store: &Store) -> Vec<(String, OutboxOp, OutboxState)> {
        store
            .outbox()
            .unwrap()
            .into_iter()
            .filter(|row| row.kind == EntityKind::PaperAccount)
            .map(|row| (row.sync_id, row.op, row.state))
            .collect()
    }

    fn bundle(store: &Store, local: u64) -> serde_json::Value {
        let sync_id = store.id_space().to_global(local).to_string();
        let Load::Found(entity) = store
            .load_projected(EntityKind::PaperAccount, &sync_id)
            .unwrap()
        else {
            panic!("account {local} has no bundle");
        };
        serde_json::from_slice(&entity.payload).unwrap()
    }

    #[test]
    fn an_account_syncs_as_one_bundle_under_shared_ids() {
        let store = syncing_store();
        let (snapshot, accounts) = traded(&["Paper"]);
        store.save_paper_trading(&snapshot).unwrap();
        let ids = store.id_space();
        let shared = ids.to_global(accounts[0]).to_string();
        assert_eq!(
            outbox_ids(&store),
            [(shared, OutboxOp::Upsert, OutboxState::Pending)]
        );
        let bundle = bundle(&store, accounts[0]);
        let device = store.sync_state().unwrap().unwrap().device_id;
        assert_eq!(bundle["owner"], device.as_str());
        assert_eq!(bundle["account"]["id"], ids.to_global(accounts[0]));
        assert_eq!(bundle["orders"].as_array().unwrap().len(), 1);
        assert_eq!(
            bundle["orders"][0]["account_id"],
            ids.to_global(accounts[0])
        );
        assert_eq!(bundle["fills"][0]["order_id"], bundle["orders"][0]["id"]);
        assert!(bundle["orders"][0]["id"].as_u64().unwrap() >= crate::sync::ids::GLOBAL_BASE);
        // The tables keep the small ids.
        assert_eq!(store.paper_trading().unwrap().accounts[0].id, accounts[0]);
    }

    #[test]
    fn a_save_that_changes_nothing_about_an_account_queues_nothing() {
        let store = syncing_store();
        let (snapshot, _) = traded(&["Paper"]);
        store.save_paper_trading(&snapshot).unwrap();
        let before = store.outbox().unwrap();
        // A quote moving is not a change to the account.
        let mut moved = snapshot.clone();
        moved
            .quotes
            .insert("Binance:BTCUSDT".into(), quote(150.0, 5));
        store.save_paper_trading(&moved).unwrap();
        assert_eq!(store.outbox().unwrap(), before);
    }

    #[test]
    fn a_burst_of_fills_uploads_once_after_the_first_deadline() {
        let store = syncing_store();
        let (snapshot, _) = traded(&["Paper"]);
        store.save_paper_trading(&snapshot).unwrap();
        let first = store.outbox().unwrap()[0]
            .next_attempt_ms
            .expect("debounced");
        let mut changed = snapshot.clone();
        changed.accounts[0].cash_balance -= 1.0;
        store.save_paper_trading(&changed).unwrap();
        let rows = store.outbox().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].next_attempt_ms, Some(first), "the deadline holds");
        assert!(first >= now());
    }

    #[test]
    fn deleting_an_account_queues_its_delete_and_keeps_the_others() {
        let store = syncing_store();
        let (snapshot, accounts) = traded(&["One", "Two"]);
        store.save_paper_trading(&snapshot).unwrap();
        for row in store.outbox().unwrap() {
            store.remove_outbox(row.seq).unwrap();
            store
                .ack_saved(row.seq, row.kind, &row.sync_id, "h", &[1], 0)
                .unwrap();
        }
        let mut after = snapshot.clone();
        after.accounts.retain(|a| a.id != accounts[1]);
        store.save_paper_trading(&after).unwrap();
        let ids = store.id_space();
        let deleted: Vec<_> = outbox_ids(&store)
            .into_iter()
            .filter(|row| row.1 == OutboxOp::Delete)
            .collect();
        assert_eq!(
            deleted,
            [(
                ids.to_global(accounts[1]).to_string(),
                OutboxOp::Delete,
                OutboxState::Pending
            )]
        );
    }

    #[test]
    fn the_owner_is_part_of_the_bundle_and_absent_means_this_mac() {
        let store = syncing_store();
        let (snapshot, accounts) = traded(&["Paper"]);
        store.save_paper_trading(&snapshot).unwrap();
        store
            .set_paper_owner(&accounts[0].to_string(), Some("other-mac"))
            .unwrap();
        assert_eq!(bundle(&store, accounts[0])["owner"], "other-mac");
        // Taking it back: absent means this Mac.
        store
            .set_paper_owner(&accounts[0].to_string(), None)
            .unwrap();
        let device = store.sync_state().unwrap().unwrap().device_id;
        assert_eq!(bundle(&store, accounts[0])["owner"], device.as_str());
    }

    #[test]
    fn reconcile_finds_accounts_written_before_sync_was_on() {
        let store = Store::open_in_memory().unwrap();
        let (snapshot, accounts) = traded(&["Paper"]);
        store.save_paper_trading(&snapshot).unwrap();
        let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
        state.enabled = true;
        store.save_sync_state(&state).unwrap();
        assert!(outbox_ids(&store).is_empty());
        let done = store.reconcile_stored().unwrap();
        assert_eq!(done.upserts, 1);
        assert_eq!(
            outbox_ids(&store)[0].0,
            store.id_space().to_global(accounts[0]).to_string()
        );
    }

    #[test]
    fn the_paper_trading_switch_stops_tracking() {
        let store = syncing_store();
        store
            .set_category_enabled(crate::sync::policy::Category::PaperTrading, false)
            .unwrap();
        let (snapshot, _) = traded(&["Paper"]);
        store.save_paper_trading(&snapshot).unwrap();
        assert!(outbox_ids(&store).is_empty());
    }

    #[test]
    fn an_empty_snapshot_is_clean() {
        let store = Store::open_in_memory().expect("open");
        store
            .save_paper_trading(&PaperTradingSnapshot::default())
            .expect("save empty");
        let loaded = store.paper_trading().expect("load");
        assert!(loaded.accounts.is_empty());
        assert_eq!(loaded.selected_account_id, None);
        assert!(loaded.quotes.is_empty());
    }

    #[test]
    fn unreadable_paper_setting_fails_the_load() {
        let store = Store::open_in_memory().expect("open");
        store
            .connection
            .execute(
                "INSERT INTO setting (key, value) VALUES ('paper.selectedAccountID', 'bad')",
                [],
            )
            .expect("seed");
        assert!(store.paper_trading().is_err());
    }
}
