#![allow(dead_code)] // transient until the journal tab reads these
//! Trading-journal storage: accounts, the append-only execution log and its ingest cursors.
//!
//! Executions are normalised records copied out of a trading source and never edited except by an
//! explicit source correction. Trades, statistics and charts are derived from them and are not
//! stored here. Nothing in this file reads or writes a `paper_*` table; the paper source hands
//! over an immutable snapshot.
//!
//! Capture. [`Store::save_paper_trading`] calls [`Store::journal_ingest_paper`] inside its own
//! transaction, so a fill and its journal row commit together. The ingest is idempotent
//! (`INSERT OR IGNORE` on the execution's identity), which is also what lets
//! [`Store::journal_reconcile_paper`] repair a crash, a mirrored account that arrived through
//! iCloud, or history that predates the journal by running the same pass over everything.
//!
//! A journal failure never fails a paper save: the journal part is rolled back by itself, the
//! reason is kept (`journal_meta.last_error`) and the next reconcile retries.

use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};

use crate::model::journal::daily::DailyEntry;
use crate::model::journal::model::{Execution, ExecutionStatus, JournalAccount};
use crate::model::journal::normalize;
use crate::model::journal::playbook::Playbook;
use crate::model::journal::reviews::Review;
use crate::model::journal::source::{Cursor, RejectedExecution, TradingActivitySource};
use crate::model::journal::sources::paper::PaperSource;
use crate::model::paper::PaperTradingSnapshot;
use crate::store::Store;

const KEY_REVISION: &str = "revision";
const KEY_LAST_ERROR: &str = "last_error";

/// What one ingest pass did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IngestReport {
    pub inserted: usize,
    /// Executions already in the log (the normal case on a repeat pass).
    pub already_present: usize,
    pub rejected: Vec<RejectedExecution>,
    pub accounts_changed: usize,
}

impl IngestReport {
    fn changed(&self) -> bool {
        self.inserted > 0 || self.accounts_changed > 0
    }
}

impl Store {
    pub(super) fn create_journal_schema(&self) -> rusqlite::Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS journal_account (
                uid TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0,
                payload TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS journal_execution (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                uid TEXT NOT NULL UNIQUE,
                account_uid TEXT NOT NULL,
                source_exec_id TEXT NOT NULL,
                instrument_key TEXT NOT NULL,
                market_key TEXT NOT NULL,
                ts_ms INTEGER NOT NULL,
                status TEXT NOT NULL,
                payload TEXT NOT NULL,
                UNIQUE (account_uid, source_exec_id)
            );
            CREATE INDEX IF NOT EXISTS journal_execution_on_account_time
                ON journal_execution (account_uid, ts_ms);
            CREATE INDEX IF NOT EXISTS journal_execution_on_instrument_time
                ON journal_execution (instrument_key, ts_ms);
            CREATE INDEX IF NOT EXISTS journal_execution_on_market_time
                ON journal_execution (market_key, ts_ms);
            CREATE TABLE IF NOT EXISTS journal_cursor (
                account_uid TEXT PRIMARY KEY,
                cursor TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS journal_review (
                trade_key TEXT PRIMARY KEY,
                account_uid TEXT NOT NULL,
                playbook TEXT NOT NULL,
                status TEXT NOT NULL,
                updated_ms INTEGER NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS journal_review_on_account ON journal_review (account_uid);
            CREATE INDEX IF NOT EXISTS journal_review_on_playbook ON journal_review (playbook);
            CREATE INDEX IF NOT EXISTS journal_review_on_status ON journal_review (status);
            CREATE INDEX IF NOT EXISTS journal_review_on_updated ON journal_review (updated_ms);
            CREATE TABLE IF NOT EXISTS journal_playbook (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                updated_ms INTEGER NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS journal_day (
                date TEXT PRIMARY KEY,
                updated_ms INTEGER NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS journal_trade_stats (
                trade_key TEXT PRIMARY KEY,
                account_uid TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS journal_trade_stats_on_account
                ON journal_trade_stats (account_uid);
            CREATE TABLE IF NOT EXISTS journal_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )
    }

    // MARK: Reads

    pub fn journal_accounts(&self) -> rusqlite::Result<Vec<JournalAccount>> {
        let mut accounts: Vec<JournalAccount> =
            self.payloads_strict("SELECT payload FROM journal_account ORDER BY rowid")?;
        let archived: HashSet<String> = {
            let mut statement = self
                .connection
                .prepare("SELECT uid FROM journal_account WHERE archived = 1")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<_, _>>()?
        };
        for account in &mut accounts {
            account.archived = archived.contains(&account.uid);
        }
        Ok(accounts)
    }

    /// Every execution in arrival order. A row that no longer decodes fails the read: dropping
    /// one would change a trade's result.
    pub fn journal_executions(&self) -> rusqlite::Result<Vec<Execution>> {
        self.payloads_strict("SELECT payload FROM journal_execution ORDER BY seq")
    }

    /// The stored execution payloads, undecoded: for a reader that decodes them after letting go
    /// of the store (a rebuild on a worker thread).
    pub fn journal_execution_payloads(&self) -> rusqlite::Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM journal_execution ORDER BY seq")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect()
    }

    /// A cached figure for a trade, only when it was computed from the same fills and candle
    /// interval (`fingerprint`). The cache is rebuildable: nothing depends on it being there.
    pub fn journal_stat(
        &self,
        trade_key: &str,
        fingerprint: &str,
    ) -> rusqlite::Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT payload FROM journal_trade_stats WHERE trade_key = ?1 AND fingerprint = ?2",
                params![trade_key, fingerprint],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn journal_save_stat(
        &self,
        trade_key: &str,
        account_uid: &str,
        fingerprint: &str,
        payload: &str,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO journal_trade_stats (trade_key, account_uid, fingerprint, payload)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(trade_key) DO UPDATE SET account_uid = excluded.account_uid,
                fingerprint = excluded.fingerprint, payload = excluded.payload",
            params![trade_key, account_uid, fingerprint, payload],
        )?;
        Ok(())
    }

    pub fn journal_reviews(&self) -> rusqlite::Result<Vec<Review>> {
        self.payloads_strict("SELECT payload FROM journal_review ORDER BY updated_ms, trade_key")
    }

    /// Saves one review, stamping its times. A review with nothing in it is removed instead of
    /// stored. Returns what was stored (`None` when it was removed).
    pub fn journal_save_review(
        &self,
        review: &Review,
        now_ms: i64,
    ) -> rusqlite::Result<Option<Review>> {
        let mut review = review.clone();
        review.tidy();
        if review.is_empty() {
            self.connection.execute(
                "DELETE FROM journal_review WHERE trade_key = ?1",
                params![review.trade_key],
            )?;
            return Ok(None);
        }
        let created: Option<i64> = self
            .connection
            .query_row(
                "SELECT payload FROM journal_review WHERE trade_key = ?1",
                params![review.trade_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|payload| serde_json::from_str::<Review>(&payload).ok())
            .map(|existing| existing.created_ms);
        review.created_ms = created.filter(|created| *created > 0).unwrap_or(now_ms);
        review.updated_ms = now_ms;
        self.connection.execute(
            "INSERT INTO journal_review (trade_key, account_uid, playbook, status, updated_ms, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(trade_key) DO UPDATE SET
                account_uid = excluded.account_uid, playbook = excluded.playbook,
                status = excluded.status, updated_ms = excluded.updated_ms,
                payload = excluded.payload",
            params![
                review.trade_key,
                review.account_uid,
                review.playbook.clone().unwrap_or_default(),
                review.status().label(),
                review.updated_ms,
                json(&review)?,
            ],
        )?;
        Ok(Some(review))
    }

    pub fn journal_playbooks(&self) -> rusqlite::Result<Vec<Playbook>> {
        self.payloads_strict(
            "SELECT payload FROM journal_playbook ORDER BY name COLLATE NOCASE, id",
        )
    }

    /// Saves one playbook, stamping its times. `None` when it has no name (nothing is stored).
    pub fn journal_save_playbook(
        &self,
        playbook: &Playbook,
        now_ms: i64,
    ) -> rusqlite::Result<Option<Playbook>> {
        let mut playbook = playbook.clone();
        playbook.tidy();
        if !playbook.is_valid() {
            return Ok(None);
        }
        let created: Option<i64> = self
            .connection
            .query_row(
                "SELECT payload FROM journal_playbook WHERE id = ?1",
                params![playbook.id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|payload| serde_json::from_str::<Playbook>(&payload).ok())
            .map(|existing| existing.created_ms);
        playbook.created_ms = created.filter(|created| *created > 0).unwrap_or(now_ms);
        playbook.updated_ms = now_ms;
        self.connection.execute(
            "INSERT INTO journal_playbook (id, name, updated_ms, payload) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                updated_ms = excluded.updated_ms, payload = excluded.payload",
            params![
                playbook.id,
                playbook.name,
                playbook.updated_ms,
                json(&playbook)?
            ],
        )?;
        Ok(Some(playbook))
    }

    /// Removes a playbook. Reviews that named it keep the id and show no playbook.
    pub fn journal_delete_playbook(&self, id: &str) -> rusqlite::Result<()> {
        self.connection
            .execute("DELETE FROM journal_playbook WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn journal_days(&self) -> rusqlite::Result<Vec<DailyEntry>> {
        self.payloads_strict("SELECT payload FROM journal_day ORDER BY date")
    }

    /// Saves one day's entry; an entry with nothing in it is removed instead.
    pub fn journal_save_day(
        &self,
        entry: &DailyEntry,
        now_ms: i64,
    ) -> rusqlite::Result<Option<DailyEntry>> {
        let mut entry = entry.clone();
        entry.tidy();
        if entry.is_empty() {
            self.connection.execute(
                "DELETE FROM journal_day WHERE date = ?1",
                params![entry.date],
            )?;
            return Ok(None);
        }
        let created: Option<i64> = self
            .connection
            .query_row(
                "SELECT payload FROM journal_day WHERE date = ?1",
                params![entry.date],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|payload| serde_json::from_str::<DailyEntry>(&payload).ok())
            .map(|existing| existing.created_ms);
        entry.created_ms = created.filter(|created| *created > 0).unwrap_or(now_ms);
        entry.updated_ms = now_ms;
        self.connection.execute(
            "INSERT INTO journal_day (date, updated_ms, payload) VALUES (?1, ?2, ?3)
             ON CONFLICT(date) DO UPDATE SET updated_ms = excluded.updated_ms,
                payload = excluded.payload",
            params![entry.date, entry.updated_ms, json(&entry)?],
        )?;
        Ok(Some(entry))
    }

    /// Executions on one market (`Execution::instrument.market_key`) in a time window, for chart
    /// markers.
    pub fn journal_executions_on_market(
        &self,
        market_key: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> rusqlite::Result<Vec<Execution>> {
        let mut statement = self.connection.prepare(
            "SELECT payload FROM journal_execution
             WHERE market_key = ?1 AND ts_ms >= ?2 AND ts_ms <= ?3 ORDER BY seq",
        )?;
        let payloads: Vec<String> = statement
            .query_map(params![market_key, from_ms, to_ms], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        payloads
            .iter()
            .map(|payload| {
                serde_json::from_str(payload).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .collect()
    }

    pub fn journal_execution_count(&self) -> rusqlite::Result<usize> {
        self.connection
            .query_row("SELECT COUNT(*) FROM journal_execution", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count as usize)
    }

    /// A counter that moves whenever the journal's executions or accounts change. Derived data
    /// is cached against it.
    pub fn journal_revision(&self) -> u64 {
        self.journal_meta(KEY_REVISION)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    /// Why the last ingest failed, if it did and nothing has succeeded since.
    pub fn journal_last_error(&self) -> Option<String> {
        self.journal_meta(KEY_LAST_ERROR)
    }

    fn journal_meta(&self, key: &str) -> Option<String> {
        self.connection
            .query_row(
                "SELECT value FROM journal_meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    // MARK: Ingest

    /// Copies the paper snapshot's new fills into the journal. Called from inside
    /// `save_paper_trading`'s transaction. Never fails the caller: see the module note.
    pub(crate) fn journal_ingest_paper(&self, snapshot: &PaperTradingSnapshot) {
        let source = PaperSource::new(snapshot);
        match self.in_savepoint("journal_ingest", || self.ingest_source(&source, false)) {
            Ok(_) => self.clear_journal_error(),
            Err(error) => self.record_journal_error(&error.to_string()),
        }
    }

    /// Runs the whole snapshot through the journal again, ignoring cursors. Safe to repeat. For
    /// startup, and after iCloud changed the paper tables underneath.
    pub fn journal_reconcile_paper(
        &self,
        snapshot: &PaperTradingSnapshot,
    ) -> rusqlite::Result<IngestReport> {
        let source = PaperSource::new(snapshot);
        let result = self.in_savepoint("journal_reconcile", || self.ingest_source(&source, true));
        match &result {
            Ok(_) => self.clear_journal_error(),
            Err(error) => self.record_journal_error(&error.to_string()),
        }
        result
    }

    /// Ingests any source. `full` ignores the stored cursors and offers everything again.
    pub fn journal_ingest(
        &self,
        source: &dyn TradingActivitySource,
        full: bool,
    ) -> rusqlite::Result<IngestReport> {
        self.in_savepoint("journal_ingest_source", || self.ingest_source(source, full))
    }

    fn ingest_source(
        &self,
        source: &dyn TradingActivitySource,
        full: bool,
    ) -> rusqlite::Result<IngestReport> {
        let mut report = IngestReport::default();
        let accounts = source.accounts();
        let source_id = source.source_id();

        for account in &accounts {
            if self.upsert_account(account)? {
                report.accounts_changed += 1;
            }
        }
        // An account the source no longer has (a paper account that was reset or deleted) keeps
        // its history and is marked archived. It can return (a mirrored account arriving later).
        let present: HashSet<&str> = accounts
            .iter()
            .map(|account| account.uid.as_str())
            .collect();
        let known: Vec<String> = {
            let mut statement = self
                .connection
                .prepare("SELECT uid FROM journal_account WHERE source = ?1")?;
            statement
                .query_map(params![source_id.0], |row| row.get::<_, String>(0))?
                .collect::<Result<_, _>>()?
        };
        for uid in known {
            let archived = !present.contains(uid.as_str());
            let changed = self.connection.execute(
                "UPDATE journal_account SET archived = ?2 WHERE uid = ?1 AND archived != ?2",
                params![uid, i64::from(archived)],
            )?;
            report.accounts_changed += changed;
        }

        for account in &accounts {
            let cursor = if full {
                None
            } else {
                self.read_cursor(&account.uid)?
            };
            let batch = match source.executions(&account.uid, cursor.as_ref()) {
                Ok(batch) => batch,
                Err(error) => {
                    report.rejected.push(RejectedExecution {
                        source_exec_id: account.uid.clone(),
                        reason: error.to_string(),
                    });
                    continue;
                }
            };
            report.rejected.extend(batch.rejected);
            for execution in &batch.executions {
                match normalize::validate(execution) {
                    Ok(()) => {
                        if self.insert_execution(execution)? {
                            report.inserted += 1;
                        } else {
                            report.already_present += 1;
                        }
                    }
                    Err(reason) => report.rejected.push(RejectedExecution {
                        source_exec_id: execution.source_exec_id.clone(),
                        reason,
                    }),
                }
            }
            for source_exec_id in &batch.voided {
                if self.void_execution(&account.uid, source_exec_id)? {
                    report.inserted += 1; // a change the derived data must see
                }
            }
            if let Some(next) = &batch.next {
                self.write_cursor(&account.uid, next)?;
            }
        }

        if report.changed() {
            self.bump_journal_revision()?;
        }
        Ok(report)
    }

    fn upsert_account(&self, account: &JournalAccount) -> rusqlite::Result<bool> {
        let payload = json(account)?;
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM journal_account WHERE uid = ?1",
                params![account.uid],
                |row| row.get(0),
            )
            .optional()?;
        if existing.as_deref() == Some(payload.as_str()) {
            return Ok(false);
        }
        self.connection.execute(
            "INSERT INTO journal_account (uid, source, archived, payload) VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(uid) DO UPDATE SET payload = excluded.payload",
            params![account.uid, account.source.0, payload],
        )?;
        Ok(true)
    }

    /// True when the execution was new.
    fn insert_execution(&self, execution: &Execution) -> rusqlite::Result<bool> {
        let inserted = self.connection.execute(
            "INSERT OR IGNORE INTO journal_execution
                (uid, account_uid, source_exec_id, instrument_key, market_key, ts_ms, status, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                execution.uid,
                execution.account_uid,
                execution.source_exec_id,
                execution.instrument.key,
                execution.instrument.market_key,
                execution.ts_ms,
                status_name(execution.status),
                json(execution)?,
            ],
        )?;
        Ok(inserted > 0)
    }

    /// Marks one execution cancelled by its source. True when something changed.
    fn void_execution(&self, account_uid: &str, source_exec_id: &str) -> rusqlite::Result<bool> {
        let row: Option<(i64, String)> = self
            .connection
            .query_row(
                "SELECT seq, payload FROM journal_execution
                 WHERE account_uid = ?1 AND source_exec_id = ?2 AND status = 'active'",
                params![account_uid, source_exec_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((seq, payload)) = row else {
            return Ok(false);
        };
        let mut execution: Execution = serde_json::from_str(&payload).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        execution.status = ExecutionStatus::Voided;
        self.connection.execute(
            "UPDATE journal_execution SET status = 'voided', payload = ?2 WHERE seq = ?1",
            params![seq, json(&execution)?],
        )?;
        Ok(true)
    }

    fn read_cursor(&self, account_uid: &str) -> rusqlite::Result<Option<Cursor>> {
        Ok(self
            .connection
            .query_row(
                "SELECT cursor FROM journal_cursor WHERE account_uid = ?1",
                params![account_uid],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(Cursor))
    }

    fn write_cursor(&self, account_uid: &str, cursor: &Cursor) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO journal_cursor (account_uid, cursor) VALUES (?1, ?2)
             ON CONFLICT(account_uid) DO UPDATE SET cursor = excluded.cursor",
            params![account_uid, cursor.0],
        )?;
        Ok(())
    }

    fn bump_journal_revision(&self) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO journal_meta (key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
            params![KEY_REVISION],
        )?;
        Ok(())
    }

    fn record_journal_error(&self, message: &str) {
        let _ = self.connection.execute(
            "INSERT INTO journal_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEY_LAST_ERROR, message],
        );
    }

    fn clear_journal_error(&self) {
        let _ = self.connection.execute(
            "DELETE FROM journal_meta WHERE key = ?1",
            params![KEY_LAST_ERROR],
        );
    }

    // MARK: Removal

    /// Deletes one account's journal data: its executions, cursor, reviews and the account
    /// itself. What the user chooses when they decline to keep an account's history.
    pub fn journal_purge_account(&self, account_uid: &str) -> rusqlite::Result<()> {
        self.in_savepoint("journal_purge", || {
            for table in [
                "journal_execution",
                "journal_cursor",
                "journal_review",
                "journal_trade_stats",
            ] {
                self.connection.execute(
                    &format!("DELETE FROM {table} WHERE account_uid = ?1"),
                    params![account_uid],
                )?;
            }
            self.connection.execute(
                "DELETE FROM journal_account WHERE uid = ?1",
                params![account_uid],
            )?;
            self.bump_journal_revision()
        })
    }
}

fn status_name(status: ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Active => "active",
        ExecutionStatus::Voided => "voided",
    }
}

fn json<T: serde::Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

#[cfg(test)]
mod tests {
    use crate::model::journal::reviews::Review;

    use super::*;
    use crate::model::DataSource;
    use crate::model::journal::testing::d;
    use crate::model::paper::{
        PaperAccountSettings, PaperCommissionConfiguration, PaperCurrency, PaperInstrument,
        PaperOrderRequest, PaperOrderSide, PaperOrderType, PaperQuote, PaperTimeInForce,
    };
    use crate::model::paper_engine::PaperTradingEngine;

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64)
    }

    fn instrument() -> PaperInstrument {
        PaperInstrument::chart("BTCUSDT", "BTCUSDT", DataSource::Binance)
    }

    fn quote(last: f64) -> PaperQuote {
        PaperQuote {
            instrument_key: instrument().key,
            bid: Some(last - 0.5),
            ask: Some(last + 0.5),
            last: Some(last),
            timestamp: now(),
            is_market_open: true,
        }
    }

    fn market(account_id: u64, side: PaperOrderSide, quantity: f64) -> PaperOrderRequest {
        PaperOrderRequest {
            account_id,
            instrument: instrument(),
            side,
            kind: PaperOrderType::Market,
            quantity,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        }
    }

    fn engine() -> (PaperTradingEngine, u64) {
        let mut engine = PaperTradingEngine::new(PaperTradingSnapshot::default());
        let settings = PaperAccountSettings {
            commission: PaperCommissionConfiguration::FixedPerOrder { value: 1.0 },
            ..PaperAccountSettings::default()
        };
        let id = engine
            .create_account("Main", PaperCurrency::Usd, 100_000.0, settings)
            .unwrap();
        (engine, id)
    }

    fn buy_then_sell(engine: &mut PaperTradingEngine, id: u64) {
        engine.process(quote(100.0)).unwrap();
        engine.submit(market(id, PaperOrderSide::Buy, 1.0)).unwrap();
        engine.process(quote(110.0)).unwrap();
        engine
            .submit(market(id, PaperOrderSide::Sell, 1.0))
            .unwrap();
    }

    #[test]
    fn a_paper_save_puts_its_fills_in_the_journal() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        store.save_paper_trading(engine.snapshot()).unwrap();

        let accounts = store.journal_accounts().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "Main");
        assert!(!accounts[0].archived);
        let executions = store.journal_executions().unwrap();
        assert_eq!(executions.len(), 2);
        assert_eq!(executions[0].price, d("100.5"));
        assert_eq!(executions[1].price, d("109.5"));
        assert!(store.journal_revision() > 0);
        assert_eq!(store.journal_last_error(), None);
    }

    #[test]
    fn saving_again_adds_only_the_new_fills() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        engine.process(quote(100.0)).unwrap();
        engine.submit(market(id, PaperOrderSide::Buy, 1.0)).unwrap();
        store.save_paper_trading(engine.snapshot()).unwrap();
        let first = store.journal_revision();
        store.save_paper_trading(engine.snapshot()).unwrap();
        assert_eq!(store.journal_executions().unwrap().len(), 1);
        assert_eq!(
            store.journal_revision(),
            first,
            "nothing changed, nothing moved"
        );

        engine.process(quote(110.0)).unwrap();
        engine
            .submit(market(id, PaperOrderSide::Sell, 1.0))
            .unwrap();
        store.save_paper_trading(engine.snapshot()).unwrap();
        assert_eq!(store.journal_executions().unwrap().len(), 2);
        assert!(store.journal_revision() > first);
    }

    #[test]
    fn a_full_pass_over_the_same_history_adds_nothing() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        store.save_paper_trading(engine.snapshot()).unwrap();
        let before = store.journal_executions().unwrap();

        let report = store.journal_reconcile_paper(engine.snapshot()).unwrap();
        assert_eq!(report.inserted, 0);
        assert_eq!(report.already_present, 2);
        assert!(report.rejected.is_empty());
        let report = store.journal_reconcile_paper(engine.snapshot()).unwrap();
        assert_eq!(report.inserted, 0);
        assert_eq!(store.journal_executions().unwrap(), before);
    }

    #[test]
    fn history_that_predates_the_journal_is_backfilled_once() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        // The ledger exists but the journal never saw it (the tables were written directly, as
        // an iCloud apply does).
        store
            .raw_connection()
            .execute("DELETE FROM journal_execution", [])
            .unwrap();
        store
            .raw_connection()
            .execute("DELETE FROM journal_cursor", [])
            .unwrap();

        let report = store.journal_reconcile_paper(engine.snapshot()).unwrap();
        assert_eq!(report.inserted, 2);
        assert_eq!(store.journal_execution_count().unwrap(), 2);
        let again = store.journal_reconcile_paper(engine.snapshot()).unwrap();
        assert_eq!(again.inserted, 0);
    }

    #[test]
    fn a_reset_keeps_the_old_history_under_an_archived_account() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        store.save_paper_trading(engine.snapshot()).unwrap();
        let old_uid = store.journal_accounts().unwrap()[0].uid.clone();

        // A reset gives the account a new `created_at`, so the journal sees a new account.
        std::thread::sleep(std::time::Duration::from_millis(3));
        engine
            .reset_account(
                id,
                PaperCurrency::Usd,
                50_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        engine.process(quote(100.0)).unwrap();
        engine.submit(market(id, PaperOrderSide::Buy, 1.0)).unwrap();
        store.save_paper_trading(engine.snapshot()).unwrap();

        let accounts = store.journal_accounts().unwrap();
        assert_eq!(accounts.len(), 2);
        let old = accounts.iter().find(|a| a.uid == old_uid).unwrap();
        let new = accounts.iter().find(|a| a.uid != old_uid).unwrap();
        assert!(old.archived);
        assert!(!new.archived);
        let executions = store.journal_executions().unwrap();
        assert_eq!(executions.len(), 3);
        assert_eq!(
            executions
                .iter()
                .filter(|e| e.account_uid == old_uid)
                .count(),
            2
        );
    }

    #[test]
    fn deleting_a_paper_account_archives_it_and_removing_it_purges_it() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        store.save_paper_trading(engine.snapshot()).unwrap();
        let uid = store.journal_accounts().unwrap()[0].uid.clone();

        engine.delete_account(id).unwrap();
        store.save_paper_trading(engine.snapshot()).unwrap();
        assert!(store.journal_accounts().unwrap()[0].archived);
        assert_eq!(store.journal_execution_count().unwrap(), 2);

        store.journal_purge_account(&uid).unwrap();
        assert!(store.journal_accounts().unwrap().is_empty());
        assert_eq!(store.journal_execution_count().unwrap(), 0);
    }

    #[test]
    fn an_account_that_comes_back_is_no_longer_archived() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        let with_account = engine.snapshot().clone();
        store.save_paper_trading(&with_account).unwrap();
        store
            .save_paper_trading(&PaperTradingSnapshot::default())
            .unwrap();
        assert!(store.journal_accounts().unwrap()[0].archived);
        store.save_paper_trading(&with_account).unwrap();
        assert!(!store.journal_accounts().unwrap()[0].archived);
        assert_eq!(store.journal_execution_count().unwrap(), 2);
    }

    #[test]
    fn a_renamed_account_updates_the_journal_account() {
        let store = Store::open_in_memory().unwrap();
        let (engine, _) = engine();
        let mut snapshot = engine.snapshot().clone();
        store.save_paper_trading(&snapshot).unwrap();
        snapshot.accounts[0].name = "Renamed".to_owned();
        store.save_paper_trading(&snapshot).unwrap();
        assert_eq!(store.journal_accounts().unwrap()[0].name, "Renamed");
    }

    #[test]
    fn the_journal_never_changes_the_paper_tables() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        store.save_paper_trading(engine.snapshot()).unwrap();
        let before = store.paper_trading().unwrap();
        store.journal_reconcile_paper(&before).unwrap();
        store.journal_executions().unwrap();
        store.journal_accounts().unwrap();
        assert_eq!(store.paper_trading().unwrap(), before);
    }

    #[test]
    fn an_invalid_execution_is_reported_and_not_stored() {
        struct Bad;
        impl TradingActivitySource for Bad {
            fn source_id(&self) -> crate::model::journal::model::SourceId {
                crate::model::journal::model::SourceId("bad".to_owned())
            }
            fn kind(&self) -> crate::model::journal::model::TradingSourceKind {
                crate::model::journal::model::TradingSourceKind::Imported
            }
            fn capabilities(&self) -> crate::model::journal::source::SourceCapabilities {
                Default::default()
            }
            fn accounts(&self) -> Vec<JournalAccount> {
                vec![JournalAccount {
                    uid: "bad:1".to_owned(),
                    source: self.source_id(),
                    kind: self.kind(),
                    external_id: "1".to_owned(),
                    name: "Bad".to_owned(),
                    currency: "USD".to_owned(),
                    initial_balance: None,
                    cost_basis: Default::default(),
                    archived: false,
                    created_at_ms: 1,
                }]
            }
            fn executions(
                &self,
                _: &str,
                _: Option<&Cursor>,
            ) -> Result<
                crate::model::journal::source::ExecutionBatch,
                crate::model::journal::source::SourceError,
            > {
                use crate::model::journal::model::Side;
                use crate::model::journal::testing::exec;
                let good = exec("1", Side::Buy, "1", "100", 1_000).in_account("bad:1");
                let zero = exec("2", Side::Buy, "0", "100", 1_000).in_account("bad:1");
                Ok(crate::model::journal::source::ExecutionBatch {
                    executions: vec![good, zero],
                    ..Default::default()
                })
            }
        }
        let store = Store::open_in_memory().unwrap();
        let report = store.journal_ingest(&Bad, true).unwrap();
        assert_eq!(report.inserted, 1);
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(report.rejected[0].source_exec_id, "2");
        assert!(report.rejected[0].reason.contains("quantity"));
        assert_eq!(store.journal_execution_count().unwrap(), 1);
    }

    #[test]
    fn executions_can_be_read_by_market_and_window() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        store.save_paper_trading(engine.snapshot()).unwrap();
        let all = store.journal_executions().unwrap();
        let inside = store
            .journal_executions_on_market("BINANCE:BTCUSDT", 0, i64::MAX)
            .unwrap();
        assert_eq!(inside, all);
        assert!(
            store
                .journal_executions_on_market("BINANCE:ETHUSDT", 0, i64::MAX)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .journal_executions_on_market("BINANCE:BTCUSDT", 0, 1)
                .unwrap()
                .is_empty()
        );
    }

    /// What the journal adds to a paper save with a long history. Run by hand:
    /// `cargo test --release --lib journal_ingest_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing"]
    fn journal_ingest_cost() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        engine.process(quote(100.0)).unwrap();
        for index in 0..5_000 {
            let side = if index % 2 == 0 {
                PaperOrderSide::Buy
            } else {
                PaperOrderSide::Sell
            };
            engine.submit(market(id, side, 1.0)).unwrap();
        }
        let snapshot = engine.snapshot().clone();
        let started = std::time::Instant::now();
        store.journal_reconcile_paper(&snapshot).unwrap();
        println!(
            "first pass over {} fills: {:?}",
            snapshot.fills.len(),
            started.elapsed()
        );

        engine.submit(market(id, PaperOrderSide::Buy, 1.0)).unwrap();
        let snapshot = engine.snapshot().clone();
        let started = std::time::Instant::now();
        store.journal_ingest_paper(&snapshot);
        println!("one new fill on top of 5000: {:?}", started.elapsed());
        let started = std::time::Instant::now();
        store.save_paper_trading(&snapshot).unwrap();
        println!(
            "whole paper save, journal included: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_review_is_saved_updated_and_removed_when_emptied() {
        let store = Store::open_in_memory().unwrap();
        let mut review = Review::new("trade-1", "acct");
        review.notes = "  Entered early.  ".into();
        review.tags = vec!["fomo".into(), "FOMO".into()];
        review.rating = Some(3);
        let saved = store.journal_save_review(&review, 1_000).unwrap().unwrap();
        assert_eq!(saved.notes, "  Entered early.");
        assert_eq!(saved.tags, ["fomo"]);
        assert_eq!((saved.created_ms, saved.updated_ms), (1_000, 1_000));

        review.reviewed = true;
        let again = store.journal_save_review(&review, 5_000).unwrap().unwrap();
        assert_eq!((again.created_ms, again.updated_ms), (1_000, 5_000));
        let all = store.journal_reviews().unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].reviewed);

        let empty = Review::new("trade-1", "acct");
        assert_eq!(store.journal_save_review(&empty, 9_000).unwrap(), None);
        assert!(store.journal_reviews().unwrap().is_empty());
    }

    #[test]
    fn purging_an_account_takes_its_reviews_with_it() {
        let store = Store::open_in_memory().unwrap();
        let (mut engine, id) = engine();
        buy_then_sell(&mut engine, id);
        store.save_paper_trading(engine.snapshot()).unwrap();
        let uid = store.journal_accounts().unwrap()[0].uid.clone();
        let mut mine = Review::new("t1", &uid);
        mine.notes = "x".into();
        let mut other = Review::new("t2", "someone-else");
        other.notes = "y".into();
        store.journal_save_review(&mine, 1).unwrap();
        store.journal_save_review(&other, 1).unwrap();
        store.journal_purge_account(&uid).unwrap();
        let left = store.journal_reviews().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].trade_key, "t2");
    }

    #[test]
    fn reviews_survive_a_restart_and_a_journal_reconcile() {
        let path = std::env::temp_dir().join(format!(
            "degenview-journal-review-{}-{}.sqlite",
            std::process::id(),
            now()
        ));
        {
            let store = Store::open(&path).unwrap();
            let mut review = Review::new("k", "a");
            review.notes = "kept".into();
            store.journal_save_review(&review, 7).unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            store
                .journal_reconcile_paper(&PaperTradingSnapshot::default())
                .unwrap();
            assert_eq!(store.journal_reviews().unwrap()[0].notes, "kept");
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn playbooks_save_update_delete_and_need_a_name() {
        let store = Store::open_in_memory().unwrap();
        let mut playbook = Playbook::new("1".into(), " Breakout ");
        playbook.checklist = vec!["Volume".into(), "volume".into(), "Stop".into()];
        let saved = store
            .journal_save_playbook(&playbook, 100)
            .unwrap()
            .unwrap();
        assert_eq!(saved.name, "Breakout");
        assert_eq!(saved.checklist, ["Volume", "Stop"]);
        assert_eq!((saved.created_ms, saved.updated_ms), (100, 100));
        playbook.description = "x".into();
        let again = store
            .journal_save_playbook(&playbook, 900)
            .unwrap()
            .unwrap();
        assert_eq!((again.created_ms, again.updated_ms), (100, 900));
        store
            .journal_save_playbook(&Playbook::new("2".into(), "alpha"), 5)
            .unwrap();
        let names: Vec<_> = store
            .journal_playbooks()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            names,
            ["alpha", "Breakout"],
            "ordered by name, ignoring case"
        );
        assert_eq!(
            store
                .journal_save_playbook(&Playbook::new("3".into(), "  "), 1)
                .unwrap(),
            None
        );
        store.journal_delete_playbook("1").unwrap();
        assert_eq!(store.journal_playbooks().unwrap().len(), 1);
    }

    #[test]
    fn daily_entries_save_by_date_and_vanish_when_emptied() {
        let store = Store::open_in_memory().unwrap();
        let mut entry = DailyEntry::new("2026-11-09");
        entry.plan = "Trade the open.  ".into();
        let saved = store.journal_save_day(&entry, 10).unwrap().unwrap();
        assert_eq!(saved.plan, "Trade the open.");
        entry.lessons = "Wait for the close.".into();
        store.journal_save_day(&entry, 20).unwrap();
        let days = store.journal_days().unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!((days[0].created_ms, days[0].updated_ms), (10, 20));
        assert_eq!(
            store
                .journal_save_day(&DailyEntry::new("2026-11-09"), 30)
                .unwrap(),
            None
        );
        assert!(store.journal_days().unwrap().is_empty());
    }

    #[test]
    fn journal_rows_survive_a_restart() {
        let path = std::env::temp_dir().join(format!(
            "degenview-journal-{}-{}.sqlite",
            std::process::id(),
            now()
        ));
        {
            let store = Store::open(&path).unwrap();
            let (mut engine, id) = engine();
            buy_then_sell(&mut engine, id);
            store.save_paper_trading(engine.snapshot()).unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.journal_execution_count().unwrap(), 2);
            assert_eq!(store.journal_accounts().unwrap().len(), 1);
            let snapshot = store.paper_trading().unwrap();
            let report = store.journal_reconcile_paper(&snapshot).unwrap();
            assert_eq!(report.inserted, 0);
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
