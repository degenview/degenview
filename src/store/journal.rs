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

use crate::store::asset_store::StoredAsset;
use degenview_journal::backup::{Backup, BackupAsset, RestoreReport};
use rusqlite::{OptionalExtension, params};

use crate::model::journal_paper_source::PaperSource;
use crate::model::paper::PaperTradingSnapshot;
use crate::store::Store;
use degenview_journal::daily::DailyEntry;
use degenview_journal::model::{Execution, ExecutionStatus, JournalAccount, TradingSourceKind};
use degenview_journal::normalize;
use degenview_journal::playbook::Playbook;
use degenview_journal::reviews::Review;
use degenview_journal::source::{Cursor, RejectedExecution, TradingActivitySource};

/// An import (or any pass) disagrees with an account that already exists.
#[derive(Debug)]
struct AccountConflict(String);

impl std::fmt::Display for AccountConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AccountConflict {}

const KEY_REVISION: &str = "revision";
const KEY_LAST_ERROR: &str = "last_error";

pub const REVIEWS: &str = "journal_review";
pub const PLAYBOOKS: &str = "journal_playbook";
pub const DAYS: &str = "journal_day";

/// The name a review has in the document table and in iCloud: a trade key holds `:` and `|`, which a
/// record name may not, so the row is named by the hash of the key (the key itself is in the row).
pub fn review_row_id(trade_key: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(trade_key.as_bytes())
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

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

    /// The latest creation time of any paper account the journal has ever seen, archived ones
    /// included: a new paper account must be created after it.
    pub fn journal_max_paper_account_time(&self) -> rusqlite::Result<Option<i64>> {
        Ok(self
            .journal_accounts()?
            .iter()
            .filter(|account| account.kind == TradingSourceKind::Paper)
            .map(|account| account.created_at_ms)
            .max())
    }

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

    /// Reads the journal's user-written rows from a document table.
    fn journal_documents<T: serde::de::DeserializeOwned>(
        &self,
        table: &str,
    ) -> rusqlite::Result<Vec<T>> {
        self.payloads_strict(&format!("SELECT payload FROM {table} ORDER BY id"))
    }

    /// The rows of a document table that decode, and the `table id` of each that does not: one
    /// damaged row must not hide the rest. Writes keep using the strict read.
    fn journal_documents_report<T: serde::de::DeserializeOwned>(
        &self,
        table: &str,
    ) -> rusqlite::Result<(Vec<T>, Vec<String>)> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT id, payload FROM {table} ORDER BY id"))?;
        let rows: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut good = Vec::with_capacity(rows.len());
        let mut bad = Vec::new();
        for (id, payload) in rows {
            match serde_json::from_str::<T>(&payload) {
                Ok(value) => good.push(value),
                Err(_) => bad.push(format!("{table} {id}")),
            }
        }
        Ok((good, bad))
    }

    pub fn journal_reviews_report(&self) -> rusqlite::Result<(Vec<Review>, Vec<String>)> {
        let (mut reviews, bad) = self.journal_documents_report::<Review>(REVIEWS)?;
        reviews.sort_by(|a, b| (a.updated_ms, &a.trade_key).cmp(&(b.updated_ms, &b.trade_key)));
        Ok((reviews, bad))
    }

    pub fn journal_playbooks_report(&self) -> rusqlite::Result<(Vec<Playbook>, Vec<String>)> {
        let (mut playbooks, bad) = self.journal_documents_report::<Playbook>(PLAYBOOKS)?;
        playbooks.sort_by_key(|playbook| (playbook.name.to_lowercase(), playbook.id.clone()));
        Ok((playbooks, bad))
    }

    pub fn journal_days_report(&self) -> rusqlite::Result<(Vec<DailyEntry>, Vec<String>)> {
        self.journal_documents_report::<DailyEntry>(DAYS)
    }

    fn journal_document_text(&self, table: &str, id: &str) -> rusqlite::Result<Option<String>> {
        self.connection
            .query_row(
                &format!("SELECT payload FROM {table} WHERE id = ?1"),
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
    }

    /// Writes one row of a journal document table as it is and queues its upload, in one
    /// savepoint. The stored text carries the row's `id`, which is how rows are named in the
    /// table and by sync; reading ignores it.
    fn put_journal_document<T: serde::Serialize>(
        &self,
        table: &str,
        id: &str,
        value: &T,
    ) -> rusqlite::Result<()> {
        let mut value = serde_json::to_value(value)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        if let Some(object) = value.as_object_mut() {
            object.insert("id".to_owned(), serde_json::Value::String(id.to_owned()));
        }
        let text = value.to_string();
        self.in_savepoint("journal_document_put", || {
            let old = self.journal_document_text(table, id)?;
            if old.as_deref() == Some(text.as_str()) {
                return Ok(());
            }
            self.connection.execute(
                &format!(
                    "INSERT INTO {table} (id, position, payload) VALUES (?1, 0, ?2)
                     ON CONFLICT(id) DO UPDATE SET payload = excluded.payload"
                ),
                params![id, text],
            )?;
            if self.sync_enabled() {
                let old: Vec<(String, String)> =
                    old.into_iter().map(|text| (id.to_owned(), text)).collect();
                self.track_documents(table, &old, &[(id.to_owned(), 0, text)])?;
            }
            Ok(())
        })
    }

    fn delete_journal_document(&self, table: &str, id: &str) -> rusqlite::Result<()> {
        self.in_savepoint("journal_document_delete", || {
            let Some(old) = self.journal_document_text(table, id)? else {
                return Ok(());
            };
            self.connection
                .execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])?;
            if self.sync_enabled() {
                self.track_documents(table, &[(id.to_owned(), old)], &[])?;
            }
            Ok(())
        })
    }

    pub fn journal_reviews(&self) -> rusqlite::Result<Vec<Review>> {
        let mut reviews: Vec<Review> = self.journal_documents(REVIEWS)?;
        reviews.sort_by(|a, b| (a.updated_ms, &a.trade_key).cmp(&(b.updated_ms, &b.trade_key)));
        Ok(reviews)
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
            self.delete_journal_document(REVIEWS, &review_row_id(&review.trade_key))?;
            return Ok(None);
        }
        let created = self
            .journal_document_text(REVIEWS, &review_row_id(&review.trade_key))?
            .and_then(|payload| serde_json::from_str::<Review>(&payload).ok())
            .map(|existing| existing.created_ms);
        review.created_ms = created.filter(|created| *created > 0).unwrap_or(now_ms);
        review.updated_ms = now_ms;
        self.put_journal_document(REVIEWS, &review_row_id(&review.trade_key), &review)?;
        Ok(Some(review))
    }

    pub fn journal_playbooks(&self) -> rusqlite::Result<Vec<Playbook>> {
        let mut playbooks: Vec<Playbook> = self.journal_documents(PLAYBOOKS)?;
        playbooks.sort_by_key(|playbook| (playbook.name.to_lowercase(), playbook.id.clone()));
        Ok(playbooks)
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
        let created = self
            .journal_document_text(PLAYBOOKS, &playbook.id)?
            .and_then(|payload| serde_json::from_str::<Playbook>(&payload).ok())
            .map(|existing| existing.created_ms);
        playbook.created_ms = created.filter(|created| *created > 0).unwrap_or(now_ms);
        playbook.updated_ms = now_ms;
        self.put_journal_document(PLAYBOOKS, &playbook.id, &playbook)?;
        Ok(Some(playbook))
    }

    /// Removes a playbook. Reviews that named it keep the id and show no playbook.
    pub fn journal_delete_playbook(&self, id: &str) -> rusqlite::Result<()> {
        self.delete_journal_document(PLAYBOOKS, id)
    }

    pub fn journal_days(&self) -> rusqlite::Result<Vec<DailyEntry>> {
        self.journal_documents(DAYS)
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
            self.delete_journal_document(DAYS, &entry.date)?;
            return Ok(None);
        }
        let created = self
            .journal_document_text(DAYS, &entry.date)?
            .and_then(|payload| serde_json::from_str::<DailyEntry>(&payload).ok())
            .map(|existing| existing.created_ms);
        entry.created_ms = created.filter(|created| *created > 0).unwrap_or(now_ms);
        entry.updated_ms = now_ms;
        self.put_journal_document(DAYS, &entry.date, &entry)?;
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
        if let Some(existing) = existing.as_deref() {
            // An account is its currency, its cost basis and where it came from for good: a later
            // pass (an import under a name already used) must not reinterpret the executions the
            // account already holds. Imported accounts are not rewritten at all.
            let stored: JournalAccount = serde_json::from_str(existing).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            if stored.currency != account.currency
                || stored.cost_basis != account.cost_basis
                || stored.kind != account.kind
                || stored.source != account.source
            {
                return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
                    AccountConflict(format!(
                        "The account \"{}\" already exists ({}, {:?} cost basis); this pass has it as {}, {:?}. Use another account name.",
                        stored.name,
                        stored.currency,
                        stored.cost_basis,
                        account.currency,
                        account.cost_basis,
                    )),
                )));
            }
            if stored.kind == TradingSourceKind::Imported {
                return Ok(false);
            }
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
            for table in ["journal_execution", "journal_cursor", "journal_trade_stats"] {
                self.connection.execute(
                    &format!("DELETE FROM {table} WHERE account_uid = ?1"),
                    params![account_uid],
                )?;
            }
            let reviews: Vec<Review> = self.journal_documents(REVIEWS)?;
            for review in reviews
                .iter()
                .filter(|review| review.account_uid == account_uid)
            {
                self.delete_journal_document(REVIEWS, &review_row_id(&review.trade_key))?;
            }
            self.connection.execute(
                "DELETE FROM journal_account WHERE uid = ?1",
                params![account_uid],
            )?;
            self.bump_journal_revision()
        })
    }
}

// MARK: Backup and restore

impl Store {
    /// Everything the journal keeps, for a portable file. Screenshots the reviews use are
    /// included when they are in the store.
    pub fn journal_backup(
        &self,
        exported_at: String,
        app_version: &str,
    ) -> rusqlite::Result<Backup> {
        let mut backup = Backup::empty(exported_at, app_version);
        backup.accounts = self.journal_accounts()?;
        backup.executions = self.journal_executions()?;
        backup.reviews = self.journal_reviews()?;
        backup.playbooks = self.journal_playbooks()?;
        backup.days = self.journal_days()?;
        let mut wanted: Vec<&String> = backup
            .reviews
            .iter()
            .flat_map(|review| review.images.iter())
            .collect();
        wanted.sort();
        wanted.dedup();
        let mut assets = Vec::new();
        for hash in wanted {
            if let Some(asset) = self.asset(hash)? {
                assets.push(BackupAsset {
                    id: asset.id,
                    mime: asset.mime,
                    width: asset.width,
                    height: asset.height,
                    data: asset.data,
                });
            }
        }
        backup.assets = assets;
        Ok(backup)
    }

    /// Merges a backup in. Adds what is missing; a review, playbook or day the journal edited
    /// after the backup was kept as it is. All or nothing: a failure rolls the whole restore back.
    pub fn journal_restore(&self, backup: &Backup) -> rusqlite::Result<RestoreReport> {
        self.in_savepoint("journal_restore", || {
            let mut report = RestoreReport::default();
            for account in &backup.accounts {
                let known: Option<i64> = self
                    .connection
                    .query_row(
                        "SELECT 1 FROM journal_account WHERE uid = ?1",
                        params![account.uid],
                        |row| row.get(0),
                    )
                    .optional()?;
                if known.is_none() {
                    self.upsert_account(account)?;
                    self.connection.execute(
                        "UPDATE journal_account SET archived = ?2 WHERE uid = ?1",
                        params![account.uid, i64::from(account.archived)],
                    )?;
                    report.accounts_added += 1;
                }
            }
            for execution in &backup.executions {
                match normalize::validate(execution) {
                    Ok(()) => {
                        let account_known: Option<i64> = self
                            .connection
                            .query_row(
                                "SELECT 1 FROM journal_account WHERE uid = ?1",
                                params![execution.account_uid],
                                |row| row.get(0),
                            )
                            .optional()?;
                        if account_known.is_none() {
                            report.rejected.push(format!(
                                "execution {} belongs to an account the backup does not contain",
                                execution.source_exec_id
                            ));
                        } else if self.insert_execution(execution)? {
                            report.executions_added += 1;
                        } else {
                            report.executions_present += 1;
                        }
                    }
                    Err(reason) => report
                        .rejected
                        .push(format!("execution {}: {reason}", execution.source_exec_id)),
                }
            }
            for review in &backup.reviews {
                let mut review = review.clone();
                review.tidy();
                if review.is_empty() || review.trade_key.is_empty() {
                    continue;
                }
                let local =
                    self.journal_document_text(REVIEWS, &review_row_id(&review.trade_key))?;
                let local_updated = local
                    .and_then(|payload| serde_json::from_str::<Review>(&payload).ok())
                    .map(|existing| existing.updated_ms);
                if local_updated.is_some_and(|local| local >= review.updated_ms) {
                    report.kept_newer += 1;
                    continue;
                }
                self.put_journal_document(REVIEWS, &review_row_id(&review.trade_key), &review)?;
                report.reviews_restored += 1;
            }
            for playbook in &backup.playbooks {
                let mut playbook = playbook.clone();
                playbook.tidy();
                if !playbook.is_valid() {
                    report
                        .rejected
                        .push(format!("playbook {:?} has no name", playbook.id));
                    continue;
                }
                let local = self.journal_document_text(PLAYBOOKS, &playbook.id)?;
                let local_updated = local
                    .and_then(|payload| serde_json::from_str::<Playbook>(&payload).ok())
                    .map(|existing| existing.updated_ms);
                if local_updated.is_some_and(|local| local >= playbook.updated_ms) {
                    report.kept_newer += 1;
                    continue;
                }
                self.put_journal_document(PLAYBOOKS, &playbook.id, &playbook)?;
                report.playbooks_restored += 1;
            }
            for day in &backup.days {
                let mut day = day.clone();
                day.tidy();
                if day.is_empty() {
                    continue;
                }
                let local = self.journal_document_text(DAYS, &day.date)?;
                let local_updated = local
                    .and_then(|payload| serde_json::from_str::<DailyEntry>(&payload).ok())
                    .map(|existing| existing.updated_ms);
                if local_updated.is_some_and(|local| local >= day.updated_ms) {
                    report.kept_newer += 1;
                    continue;
                }
                self.put_journal_document(DAYS, &day.date, &day)?;
                report.days_restored += 1;
            }
            for asset in &backup.assets {
                let stored = StoredAsset {
                    id: asset.id.clone(),
                    mime: asset.mime.clone(),
                    width: asset.width,
                    height: asset.height,
                    data: asset.data.clone(),
                };
                if stored.verified_bytes().is_none() {
                    report
                        .rejected
                        .push(format!("screenshot {} is not a valid picture", asset.id));
                } else if self.put_asset(&stored)? {
                    report.assets_added += 1;
                }
            }
            if report.changed() {
                self.bump_journal_revision()?;
            }
            Ok(report)
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
    #[test]
    fn a_review_row_id_is_a_legal_record_name_and_depends_only_on_the_key() {
        let id = super::review_row_id("import:main|csv-0a1b");
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(id, super::review_row_id("import:main|csv-0a1b"));
        assert_ne!(id, super::review_row_id("paper:1:2:0"));
    }

    use degenview_journal::reviews::Review;

    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{
        PaperAccountSettings, PaperCommissionConfiguration, PaperCurrency, PaperInstrument,
        PaperOrderRequest, PaperOrderSide, PaperOrderType, PaperQuote, PaperTimeInForce,
    };
    use crate::model::paper_engine::PaperTradingEngine;
    use degenview_journal::testing::d;

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
            fn source_id(&self) -> degenview_journal::model::SourceId {
                degenview_journal::model::SourceId("bad".to_owned())
            }
            fn kind(&self) -> degenview_journal::model::TradingSourceKind {
                degenview_journal::model::TradingSourceKind::Imported
            }
            fn capabilities(&self) -> degenview_journal::source::SourceCapabilities {
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
                degenview_journal::source::ExecutionBatch,
                degenview_journal::source::SourceError,
            > {
                use degenview_journal::model::Side;
                use degenview_journal::testing::exec;
                let good = exec("1", Side::Buy, "1", "100", 1_000).in_account("bad:1");
                let zero = exec("2", Side::Buy, "0", "100", 1_000).in_account("bad:1");
                Ok(degenview_journal::source::ExecutionBatch {
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

    // MARK: Import, backup and restore

    use degenview_journal::csv::{
        ColumnMap, ImportOptions, ImportSource, interpret, plan, read_table,
    };
    use degenview_journal::model::CostBasis;
    use degenview_journal::playbook::Playbook;
    use degenview_journal::reconstruct::reconstruct;

    const FILE: &str = "time,symbol,side,qty,price,fee,account\n\
        2026-11-09T10:00:00Z,BTC,buy,1,100,0.5,Main\n\
        2026-11-09T11:00:00Z,BTC,sell,1,110,0.5,Main\n\
        2026-11-10T10:00:00Z,ETH,sell,2,50,,Side\n\
        2026-11-10T12:00:00Z,ETH,buy,2,45,,Side\n";

    fn import(store: &Store, text: &str, options: &ImportOptions) -> IngestReport {
        let table = read_table(text).unwrap();
        let outcomes = interpret(
            &table,
            &ColumnMap::detect(&table.headers),
            options,
            &degenview_journal::csv::NoMarkets,
            &|_| 0,
        );
        let mut total = IngestReport::default();
        for source in ImportSource::split(plan(
            &outcomes,
            options,
            &degenview_journal::csv::NoMarkets,
            1,
        )) {
            let report = store.journal_ingest(&source, true).unwrap();
            total.inserted += report.inserted;
            total.already_present += report.already_present;
            total.rejected.extend(report.rejected);
        }
        total
    }

    fn png(seed: u8) -> StoredAsset {
        let image =
            image::RgbaImage::from_fn(16, 10, |x, y| image::Rgba([seed, x as u8, y as u8, 255]));
        let mut png = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut png),
            image.as_raw(),
            16,
            10,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        StoredAsset::from_ingested(&crate::service::image_ingest::ingest(&png).unwrap())
    }

    fn trades_of(store: &Store) -> Vec<(String, String)> {
        let executions = store.journal_executions().unwrap();
        let mut trades: Vec<_> = reconstruct(&executions, |_: &str| CostBasis::AverageCost)
            .into_iter()
            .map(|trade| (trade.key.clone(), trade.net_pnl().to_string()))
            .collect();
        trades.sort();
        trades
    }

    #[test]
    fn an_import_adds_accounts_and_executions_and_a_second_import_adds_nothing() {
        let store = Store::open_in_memory().unwrap();
        let first = import(&store, FILE, &ImportOptions::default());
        assert_eq!((first.inserted, first.already_present), (4, 0));
        assert_eq!(store.journal_accounts().unwrap().len(), 2);
        let second = import(&store, FILE, &ImportOptions::default());
        assert_eq!((second.inserted, second.already_present), (0, 4));
        assert_eq!(store.journal_execution_count().unwrap(), 4);
        // Another import never archives an earlier one's account.
        assert!(
            store
                .journal_accounts()
                .unwrap()
                .iter()
                .all(|a| !a.archived)
        );
        // The trades: Main made 9 net, Side (a short) made 10 with no known fee.
        let trades = trades_of(&store);
        assert_eq!(trades.len(), 2);
    }

    #[test]
    fn a_backup_restores_everything_into_an_empty_journal_and_again_changes_nothing() {
        let a = Store::open_in_memory().unwrap();
        import(&a, FILE, &ImportOptions::default());
        let trade_key = a.journal_executions().unwrap()[0].uid.clone();
        let picture = png(7);
        a.put_asset(&picture).unwrap();
        let mut review = Review::new(&trade_key, "import:main");
        review.notes = "Good patience.".into();
        review.images = vec![picture.id.clone()];
        review.updated_ms = 0;
        a.journal_save_review(&review, 5_000).unwrap();
        let mut playbook = Playbook::new("pb1".into(), "Breakout");
        playbook.checklist = vec!["Volume".into()];
        a.journal_save_playbook(&playbook, 5_000).unwrap();
        let mut day = DailyEntry::new("2026-11-09");
        day.plan = "Trade the open".into();
        a.journal_save_day(&day, 5_000).unwrap();

        let backup = a
            .journal_backup("2026-11-10T00:00:00.000Z".into(), "test")
            .unwrap();
        let text = backup.to_json().unwrap();
        let parsed = Backup::parse(&text).unwrap();
        assert_eq!(parsed.assets.len(), 1);

        let b = Store::open_in_memory().unwrap();
        let report = b.journal_restore(&parsed).unwrap();
        assert_eq!(report.executions_added, 4);
        assert_eq!(report.reviews_restored, 1);
        assert_eq!(report.playbooks_restored, 1);
        assert_eq!(report.days_restored, 1);
        assert_eq!(report.assets_added, 1);
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
        assert_eq!(trades_of(&a), trades_of(&b));
        assert_eq!(b.journal_reviews().unwrap(), a.journal_reviews().unwrap());
        assert_eq!(
            b.journal_playbooks().unwrap(),
            a.journal_playbooks().unwrap()
        );
        assert_eq!(b.journal_days().unwrap(), a.journal_days().unwrap());
        assert!(b.asset(&picture.id).unwrap().is_some());
        assert_eq!(b.journal_accounts().unwrap(), a.journal_accounts().unwrap());

        // Again: nothing to add, nothing lost.
        let again = b.journal_restore(&parsed).unwrap();
        assert!(!again.changed(), "{again:?}");
        assert_eq!(again.executions_present, 4);
        assert_eq!(again.kept_newer, 3);
    }

    #[test]
    fn a_backup_never_overwrites_an_edit_made_after_it() {
        let a = Store::open_in_memory().unwrap();
        import(&a, FILE, &ImportOptions::default());
        let key = a.journal_executions().unwrap()[0].uid.clone();
        let mut review = Review::new(&key, "import:main");
        review.notes = "old".into();
        a.journal_save_review(&review, 1_000).unwrap();
        let backup = a.journal_backup("x".into(), "t").unwrap();
        review.notes = "newer".into();
        a.journal_save_review(&review, 9_000).unwrap();
        let report = a.journal_restore(&backup).unwrap();
        assert_eq!(report.reviews_restored, 0);
        assert_eq!(a.journal_reviews().unwrap()[0].notes, "newer");
        // An older local copy is replaced by the newer backup.
        let b = Store::open_in_memory().unwrap();
        b.journal_restore(&a.journal_backup("x".into(), "t").unwrap())
            .unwrap();
        let mut older_backup = backup.clone();
        older_backup.reviews[0].updated_ms = 500;
        assert_eq!(
            b.journal_restore(&older_backup).unwrap().reviews_restored,
            0
        );
        let mut newest = backup;
        newest.reviews[0].notes = "from the backup".into();
        newest.reviews[0].updated_ms = 99_000;
        assert_eq!(b.journal_restore(&newest).unwrap().reviews_restored, 1);
        assert_eq!(b.journal_reviews().unwrap()[0].notes, "from the backup");
    }

    #[test]
    fn unusable_parts_of_a_backup_are_reported_and_the_rest_restored() {
        let a = Store::open_in_memory().unwrap();
        import(&a, FILE, &ImportOptions::default());
        let mut backup = a.journal_backup("x".into(), "t").unwrap();
        backup.executions[0].quantity = d("-1");
        backup.executions[1].account_uid = "import:ghost".into();
        backup.assets.push(degenview_journal::backup::BackupAsset {
            id: "0".repeat(64),
            mime: "image/png".into(),
            width: 1,
            height: 1,
            data: "AAAA".into(),
        });
        let b = Store::open_in_memory().unwrap();
        let report = b.journal_restore(&backup).unwrap();
        assert_eq!(report.executions_added, 2);
        assert_eq!(report.rejected.len(), 3, "{:?}", report.rejected);
        assert!(report.rejected.iter().any(|r| r.contains("not positive")));
        assert!(
            report
                .rejected
                .iter()
                .any(|r| r.contains("does not contain"))
        );
        assert!(
            report
                .rejected
                .iter()
                .any(|r| r.contains("not a valid picture"))
        );
    }
    fn try_import(
        store: &Store,
        text: &str,
        options: &ImportOptions,
    ) -> rusqlite::Result<IngestReport> {
        let table = read_table(text).unwrap();
        let outcomes = interpret(
            &table,
            &ColumnMap::detect(&table.headers),
            options,
            &degenview_journal::csv::NoMarkets,
            &|_| 0,
        );
        let mut total = IngestReport::default();
        for source in ImportSource::split(plan(
            &outcomes,
            options,
            &degenview_journal::csv::NoMarkets,
            1,
        )) {
            let report = store.journal_ingest(&source, true)?;
            total.inserted += report.inserted;
            total.already_present += report.already_present;
        }
        Ok(total)
    }

    #[test]
    fn a_second_import_cannot_change_an_accounts_currency_or_cost_basis() {
        // Finding 3: the second import used to overwrite the account's row and keep its executions.
        let store = Store::open_in_memory().unwrap();
        let usd = ImportOptions {
            account_name: "Main".into(),
            ..ImportOptions::default()
        };
        try_import(&store, FILE, &usd).unwrap();
        let eur = ImportOptions {
            currency: "EUR".into(),
            ..usd.clone()
        };
        let error = try_import(&store, FILE, &eur).unwrap_err().to_string();
        assert!(error.contains("Main") && error.contains("USD"), "{error}");
        let fifo = ImportOptions {
            cost_basis: CostBasis::Fifo,
            ..usd.clone()
        };
        assert!(try_import(&store, FILE, &fifo).is_err());
        let accounts = store.journal_accounts().unwrap();
        assert!(accounts.iter().all(|account| account.currency == "USD"));
        assert!(
            accounts
                .iter()
                .all(|account| account.cost_basis == CostBasis::AverageCost)
        );
        // The same options again are the idempotent re-import it always was.
        assert_eq!(try_import(&store, FILE, &usd).unwrap().inserted, 0);
    }

    #[test]
    fn an_imported_account_is_never_overwritten_by_a_later_import() {
        let store = Store::open_in_memory().unwrap();
        let first = ImportOptions {
            initial_balance: Some(rust_decimal::Decimal::from(5_000)),
            ..ImportOptions::default()
        };
        try_import(&store, FILE, &first).unwrap();
        let second = ImportOptions {
            initial_balance: Some(rust_decimal::Decimal::from(9_999)),
            ..ImportOptions::default()
        };
        try_import(&store, FILE, &second).unwrap();
        for account in store.journal_accounts().unwrap() {
            assert_eq!(
                account.initial_balance,
                Some(rust_decimal::Decimal::from(5_000)),
                "{}",
                account.name
            );
        }
    }
}
