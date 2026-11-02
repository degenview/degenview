//! Moving a database to another Mac without two Macs sharing one id space.
//!
//! Portfolios, paper accounts and everything inside them are numbered by a per-Mac counter and sync
//! under `GLOBAL_BASE | tag << 24 | id` (`sync::ids`). A database copied to a second Mac (Migration
//! Assistant, a restored backup) would keep minting the same tagged ids as the original, and the two
//! Macs would overwrite each other's records. So when a database finds itself on different hardware it
//! gets a new tag, and everything it already holds under counter ids is rewritten *to the old tagged
//! form*. Those records keep the sync identity they have in iCloud, the new Mac and the original see
//! the same portfolio, and counters minted from now on belong to the new tag.

use super::Store;
use crate::sync::ids::{Direction, IdSpace, map_fields_with};
use crate::sync::paper::{COLLECTIONS, map_collection_row};
use crate::sync::policy::EntityKind;
use crate::sync::project::{map_chart_cards, map_entry_cards};
use rusqlite::params;
use serde_json::Value;

type Map<'a> = &'a dyn Fn(u64) -> u64;

impl Store {
    /// Takes this database to new hardware: counter ids move to the old tag's form, then the device
    /// identity, the engine checkpoint and the id tag are replaced. One savepoint: all of it or none.
    pub fn move_to_new_device(&self, hardware_hash: &str) -> rusqlite::Result<()> {
        self.in_savepoint("move_to_new_device", || {
            let state = self
                .sync_state()?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            self.tag_counter_ids(IdSpace::new(state.id_tag))?;
            self.reset_sync_device(hardware_hash)?;
            let mut tag = super::sync::new_id_tag();
            while tag == state.id_tag {
                tag = super::sync::new_id_tag();
            }
            self.connection
                .execute("UPDATE sync_state SET id_tag = ?1 WHERE id = 1", [tag])?;
            Ok(())
        })
    }

    /// Rewrites every counter id this database holds to the form `old` gave it in iCloud. Ids that
    /// are already global (nanosecond ids, other Macs' tagged ids) pass through unchanged.
    fn tag_counter_ids(&self, old: IdSpace) -> rusqlite::Result<()> {
        let map = |id: u64| old.to_global(id);
        let map: Map = &map;

        self.retag_rows("portfolio", &["id"], map, &|row| {
            map_fields_with(row, &["id"], map)
        })?;
        self.retag_rows(
            "portfolio_transaction",
            &["id", "portfolio_id"],
            map,
            &|row| map_fields_with(row, &["id", "portfolio_id"], map),
        )?;
        self.retag_rows("portfolio_snapshot", &["portfolio_id"], map, &|row| {
            map_fields_with(row, &["portfolio_id"], map)
        })?;
        self.retag_rows("paper_account", &["id"], map, &|row| {
            map_fields_with(row, &["id"], map)
        })?;
        for (key, table) in COLLECTIONS {
            self.retag_rows(table, &["account_id"], map, &|row| {
                map_collection_row(key, row, map)
            })?;
        }
        // Cards in saved layouts and in watchlists point at portfolios by id.
        self.retag_rows("saved_view", &[], map, &|row| {
            if let Value::Object(object) = row {
                map_chart_cards(object, old, Direction::ToGlobal);
            }
        })?;
        self.retag_rows("watchlist", &[], map, &|row| {
            if let Value::Object(object) = row {
                map_entry_cards(object, old, Direction::ToGlobal);
            }
        })?;

        self.retag_id_setting("portfolio.selectedPortfolioID", map)?;
        self.retag_id_setting("paper.selectedAccountID", map)?;
        self.retag_id_keys("portfolio.invalidatedAfter", map)?;

        for kind in [
            EntityKind::Portfolio,
            EntityKind::PortfolioTransaction,
            EntityKind::PaperAccount,
        ] {
            self.retag_column("sync_entity", "local_id", Some(kind.prefix()), map)?;
        }
        self.retag_column("paper_account_local", "account_id", None, map)?;
        Ok(())
    }

    /// Rewrites the stored JSON of each row of `table` and then the id columns named. A row whose
    /// JSON cannot be read is left as it is.
    fn retag_rows(
        &self,
        table: &str,
        id_columns: &[&str],
        map: Map,
        rewrite: &dyn Fn(&mut Value),
    ) -> rusqlite::Result<()> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT rowid, payload FROM {table}"))?;
        let rows: Vec<(i64, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(statement);
        for (rowid, payload) in rows {
            let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
                continue;
            };
            rewrite(&mut value);
            self.connection.execute(
                &format!("UPDATE {table} SET payload = ?1 WHERE rowid = ?2"),
                params![value.to_string(), rowid],
            )?;
        }
        for column in id_columns {
            self.retag_column(table, column, None, map)?;
        }
        Ok(())
    }

    /// Rewrites a text column holding a number, optionally only on rows of one entity type.
    fn retag_column(
        &self,
        table: &str,
        column: &str,
        entity_type: Option<&str>,
        map: Map,
    ) -> rusqlite::Result<()> {
        let filter = entity_type.map_or(String::new(), |_| " AND entity_type = ?3".to_string());
        let select = format!(
            "SELECT DISTINCT {column} FROM {table} WHERE 1 = 1{}",
            entity_type.map_or("", |_| " AND entity_type = ?1")
        );
        let mut statement = self.connection.prepare(&select)?;
        let found: Vec<String> = match entity_type {
            Some(kind) => statement
                .query_map([kind], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?,
            None => statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?,
        };
        drop(statement);
        let update = format!("UPDATE {table} SET {column} = ?1 WHERE {column} = ?2{filter}");
        for old in found {
            let Ok(number) = old.parse::<u64>() else {
                continue;
            };
            let new = map(number).to_string();
            if new == old {
                continue;
            }
            match entity_type {
                Some(kind) => self.connection.execute(&update, params![new, old, kind])?,
                None => self.connection.execute(&update, params![new, old])?,
            };
        }
        Ok(())
    }

    /// A setting that holds one id as a JSON number.
    fn retag_id_setting(&self, key: &str, map: Map) -> rusqlite::Result<()> {
        let Some(Value::Number(number)) = self.raw_setting(key)? else {
            return Ok(());
        };
        let Some(id) = number.as_u64() else {
            return Ok(());
        };
        self.connection.execute(
            "UPDATE setting SET value = ?1 WHERE key = ?2",
            params![map(id).to_string(), key],
        )?;
        Ok(())
    }

    /// A setting that is a JSON object keyed by ids.
    fn retag_id_keys(&self, key: &str, map: Map) -> rusqlite::Result<()> {
        let Some(Value::Object(object)) = self.raw_setting(key)? else {
            return Ok(());
        };
        let moved: serde_json::Map<String, Value> = object
            .into_iter()
            .map(|(name, value)| match name.parse::<u64>() {
                Ok(id) => (map(id).to_string(), value),
                Err(_) => (name, value),
            })
            .collect();
        self.connection.execute(
            "UPDATE setting SET value = ?1 WHERE key = ?2",
            params![Value::Object(moved).to_string(), key],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{
        PaperCurrency, PaperInstrument, PaperOrderRequest, PaperOrderSide, PaperOrderType,
        PaperQuote, PaperTimeInForce, PaperTradingSnapshot,
    };
    use crate::model::paper_engine::PaperTradingEngine;
    use crate::model::portfolio::{
        Portfolio, PortfolioAsset, PortfolioCurrency, PortfolioLedgerSnapshot, PortfolioSnapshot,
        PortfolioSource, PortfolioTransaction, PortfolioTransactionType,
    };
    use crate::store::sync::Load;
    use crate::sync::ids::GLOBAL_BASE;
    use std::collections::HashMap;

    fn syncing_store() -> Store {
        let store = Store::open_in_memory().expect("open");
        let mut state = store.ensure_sync_state("Mac", "hw-a").expect("state");
        state.enabled = true;
        store.save_sync_state(&state).expect("enable");
        store
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
            kind: PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(50_000.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 2_000,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 2_000,
            updated_at: 2_000,
        }
    }

    fn traded() -> (PaperTradingSnapshot, u64) {
        let mut engine = PaperTradingEngine::new(PaperTradingSnapshot::default());
        let account = engine
            .create_account("Paper", PaperCurrency::Usd, 100_000.0, Default::default())
            .unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        engine
            .process(PaperQuote {
                instrument_key: "Binance:BTCUSDT".to_owned(),
                bid: Some(99.5),
                ask: Some(100.5),
                last: Some(100.0),
                timestamp: now,
                is_market_open: true,
            })
            .unwrap();
        engine
            .submit(PaperOrderRequest {
                account_id: account,
                instrument: PaperInstrument::chart("BTCUSDT", "BTCUSDT", DataSource::Binance),
                side: PaperOrderSide::Buy,
                kind: PaperOrderType::Market,
                quantity: 1.0,
                limit_price: None,
                stop_price: None,
                time_in_force: PaperTimeInForce::GoodTilCanceled,
                take_profit: None,
                stop_loss: None,
            })
            .unwrap();
        (engine.snapshot().clone(), account)
    }

    /// The synced record as JSON. A paper account names the device that runs it, which is rightly a
    /// different one after the move, so that field is left out.
    fn payload(store: &Store, kind: EntityKind, sync_id: &str) -> Value {
        let Load::Found(entity) = store.load_projected(kind, sync_id).unwrap() else {
            panic!("{sync_id} is not there");
        };
        let mut value: Value = serde_json::from_slice(&entity.payload).unwrap();
        if let Some(object) = value.as_object_mut() {
            object.remove("owner");
        }
        value
    }

    /// Everything a copied database holds under counter ids, saved and tracked for sync.
    fn populated() -> (Store, u64, u64) {
        let store = syncing_store();
        store
            .save_portfolio_ledger(&PortfolioLedgerSnapshot {
                portfolios: vec![Portfolio::new(8, "Main", 1_000)],
                transactions: vec![transaction(21, 8)],
                historical_snapshots: vec![PortfolioSnapshot {
                    portfolio_id: 8,
                    timestamp: 5,
                    value: 1.0,
                    net_contributions: 1.0,
                    realized_pnl: 0.0,
                    unrealized_pnl: 0.0,
                    is_complete: true,
                }],
                selected_portfolio_id: Some(8),
                invalidated_after: HashMap::from([(8, 77)]),
            })
            .unwrap();
        let (mut paper, account) = traded();
        paper.selected_account_id = Some(account);
        store.save_paper_trading(&paper).unwrap();
        store
            .execute_raw(
                r#"INSERT INTO saved_view (id, position, payload) VALUES ('1', 0,
                   '{"id":1,"name":"L","charts":[{"id":1,"symbol":"portfolio:allocation:8","card":{"portfolio_id":8}}]}')"#,
            )
            .unwrap();
        store
            .execute_raw(
                r#"INSERT INTO watchlist (id, position, payload) VALUES ('2', 0,
                   '{"id":2,"name":"W","entries":[{"Instrument":{"id":3,"symbol":"portfolio:allocation:8","card":{"portfolio_id":8}}}]}')"#,
            )
            .unwrap();
        // The two rows above went in without the write hook; let sync find them, as it does at start.
        store.reconcile_stored().unwrap();
        (store, 8, account)
    }

    #[test]
    fn a_copied_database_keeps_every_synced_identity_and_gets_a_new_tag() {
        let (store, portfolio, account) = populated();
        let old = store.id_space();
        let (old_portfolio, old_account) = (old.to_global(portfolio), old.to_global(account));
        let state = store.sync_state().unwrap().unwrap();
        let bundle_before = payload(&store, EntityKind::PaperAccount, &old_account.to_string());
        let portfolio_before = payload(&store, EntityKind::Portfolio, &old_portfolio.to_string());
        let tx_before = payload(
            &store,
            EntityKind::PortfolioTransaction,
            &old.to_global(21).to_string(),
        );
        let entities_before = store.synced_entities().unwrap();

        store.move_to_new_device("hw-b").unwrap();

        // A new Mac: new device, new tag, no checkpoint, back to the first-sync question.
        let moved = store.sync_state().unwrap().unwrap();
        assert_ne!(moved.device_id, state.device_id);
        assert_ne!(moved.id_tag, state.id_tag);
        assert_eq!(moved.hardware_hash, "hw-b");
        assert!(moved.engine_state.is_none());
        assert!(!moved.initial_sync_done);

        // The rows now carry the ids iCloud already knows them by.
        let ledger = store.portfolio_ledger().unwrap();
        assert_eq!(ledger.portfolios[0].id, old_portfolio);
        assert_eq!(ledger.transactions[0].portfolio_id, old_portfolio);
        assert_eq!(ledger.transactions[0].id, old.to_global(21));
        assert_eq!(ledger.historical_snapshots[0].portfolio_id, old_portfolio);
        assert_eq!(ledger.selected_portfolio_id, Some(old_portfolio));
        assert_eq!(
            ledger.invalidated_after,
            HashMap::from([(old_portfolio, 77)])
        );
        let paper = store.paper_trading().unwrap();
        assert_eq!(paper.accounts[0].id, old_account);
        assert_eq!(paper.orders[0].account_id, old_account);
        assert_eq!(paper.fills[0].account_id, old_account);
        assert_eq!(paper.selected_account_id, Some(old_account));
        assert!(paper.accounts[0].id >= GLOBAL_BASE);
        let layout = store.raw_rows("saved_view").unwrap();
        assert_eq!(
            layout[0]["charts"][0]["card"]["portfolio_id"],
            old_portfolio
        );
        assert_eq!(
            layout[0]["charts"][0]["symbol"],
            format!("portfolio:allocation:{old_portfolio}")
        );
        let list = store.raw_rows("watchlist").unwrap();
        assert_eq!(
            list[0]["entries"][0]["Instrument"]["card"]["portfolio_id"],
            old_portfolio
        );

        // Under the new tag the same entities have the same sync ids and the same bytes.
        assert_eq!(
            payload(&store, EntityKind::PaperAccount, &old_account.to_string()),
            bundle_before
        );
        assert_eq!(
            payload(&store, EntityKind::Portfolio, &old_portfolio.to_string()),
            portfolio_before
        );
        assert_eq!(
            payload(
                &store,
                EntityKind::PortfolioTransaction,
                &old.to_global(21).to_string()
            ),
            tx_before
        );
        assert_eq!(store.synced_entities().unwrap(), entities_before);

        // Nothing is queued, deleted or re-uploaded because of the move.
        let queued = store.outbox().unwrap().len();
        store.reconcile_stored().unwrap();
        assert_eq!(store.outbox().unwrap().len(), queued);
        assert_eq!(store.synced_entities().unwrap(), entities_before);

        // New counters belong to the new tag, and nothing collides with the old ones.
        let fresh = store.id_space();
        assert_ne!(fresh, old);
        assert_ne!(fresh.to_global(8), old_portfolio);
    }

    #[test]
    fn a_failed_move_changes_nothing() {
        let (store, ..) = populated();
        let before = store.portfolio_ledger().unwrap();
        let state = store.sync_state().unwrap().unwrap();
        // The last write of the move fails, after every id has already been rewritten.
        store
            .execute_raw(
                "CREATE TRIGGER stop_move BEFORE UPDATE OF hardware_hash ON sync_state
                 BEGIN SELECT RAISE(ABORT, 'disk full'); END",
            )
            .unwrap();
        assert!(store.move_to_new_device("hw-b").is_err());
        assert_eq!(store.portfolio_ledger().unwrap(), before);
        assert_eq!(store.sync_state().unwrap().unwrap(), state);
    }

    #[test]
    fn a_database_with_nothing_to_rewrite_still_moves() {
        let store = syncing_store();
        let state = store.sync_state().unwrap().unwrap();
        store.move_to_new_device("hw-b").unwrap();
        let moved = store.sync_state().unwrap().unwrap();
        assert_ne!(moved.device_id, state.device_id);
        assert_ne!(moved.id_tag, state.id_tag);
    }
}
