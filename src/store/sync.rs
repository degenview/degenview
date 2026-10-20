//! Durable iCloud Sync metadata: the binding to one iCloud account, the engine checkpoint, and
//! (in later stages) the outbox and tombstones. These tables live beside the user's data so a
//! local write and the intent to upload it commit in one transaction. Older builds ignore them.

// Consumed by the sync core in later stages.
#![allow(dead_code)]

use super::Store;
use rusqlite::{OptionalExtension, params};

/// The singleton row describing this Mac's sync identity and checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    /// Tracking and uploading only happen while this is set.
    pub enabled: bool,
    pub device_id: String,
    pub device_name: String,
    /// Hash of the hardware identity; a mismatch means this database was cloned to another Mac.
    pub hardware_hash: String,
    pub container_id: Option<String>,
    /// `Production` or `Development`, as the build's entitlement reported.
    pub environment: Option<String>,
    /// Hash of the CloudKit user record id this database is bound to.
    pub account_hash: Option<String>,
    pub account_bound_ms: Option<i64>,
    pub dataset_id: Option<String>,
    pub zone_name: String,
    /// JSON map of per-category switches; a missing category is on.
    pub categories: String,
    pub initial_sync_done: bool,
    /// Opaque `CKSyncEngine` state serialization.
    pub engine_state: Option<Vec<u8>>,
    pub engine_state_account_hash: Option<String>,
    pub last_fetch_ms: Option<i64>,
    pub last_send_ms: Option<i64>,
    pub last_error: Option<String>,
}

pub const DEFAULT_ZONE: &str = "DegenViewZone";

/// 128 random bits as hex. A device id must not collide across Macs.
pub fn new_device_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // No OS randomness: fall back to the clock and a stack address rather than fail startup.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64);
        bytes[..8].copy_from_slice(&nanos.to_le_bytes());
        let address = &nanos as *const u64 as usize as u64;
        bytes[8..].copy_from_slice(&address.to_le_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Store {
    pub(super) fn create_sync_schema(&self) -> rusqlite::Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS sync_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                schema_version INTEGER NOT NULL DEFAULT 1,
                enabled INTEGER NOT NULL DEFAULT 0,
                device_id TEXT NOT NULL,
                device_name TEXT NOT NULL,
                hardware_hash TEXT NOT NULL,
                container_id TEXT,
                environment TEXT,
                account_hash TEXT,
                account_bound_ms INTEGER,
                dataset_id TEXT,
                zone_name TEXT NOT NULL DEFAULT 'DegenViewZone',
                categories TEXT NOT NULL DEFAULT '{}',
                initial_sync_done INTEGER NOT NULL DEFAULT 0,
                engine_state BLOB,
                engine_state_account_hash TEXT,
                last_fetch_ms INTEGER,
                last_send_ms INTEGER,
                last_error TEXT
            );
            CREATE TABLE IF NOT EXISTS sync_entity (
                entity_type TEXT NOT NULL,
                sync_id TEXT NOT NULL,
                local_id TEXT NOT NULL,
                logical_revision INTEGER NOT NULL DEFAULT 0,
                origin_device TEXT NOT NULL DEFAULT '',
                synced_hash TEXT,
                base_payload BLOB,
                system_fields BLOB,
                cloud_modified_ms INTEGER,
                deleted INTEGER NOT NULL DEFAULT 0,
                deleted_ms INTEGER,
                PRIMARY KEY (entity_type, sync_id)
            );
            CREATE INDEX IF NOT EXISTS sync_entity_on_local ON sync_entity (entity_type, local_id);
            CREATE TABLE IF NOT EXISTS sync_outbox (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                entity_type TEXT NOT NULL,
                sync_id TEXT NOT NULL,
                op TEXT NOT NULL CHECK (op IN ('upsert', 'delete')),
                state TEXT NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'held', 'blocked')),
                queued_ms INTEGER NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                next_attempt_ms INTEGER,
                last_error TEXT,
                UNIQUE (entity_type, sync_id)
            );
            CREATE TABLE IF NOT EXISTS sync_inbox (
                record_name TEXT PRIMARY KEY,
                entity_type TEXT NOT NULL,
                schema_version INTEGER NOT NULL,
                reason TEXT NOT NULL,
                payload BLOB NOT NULL,
                system_fields BLOB,
                received_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sync_conflict (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                entity_type TEXT NOT NULL,
                sync_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                local_payload BLOB,
                remote_payload BLOB,
                remote_device TEXT,
                copy_entity_type TEXT,
                copy_sync_id TEXT,
                created_ms INTEGER NOT NULL,
                resolved_ms INTEGER,
                resolution TEXT
            );
            CREATE TABLE IF NOT EXISTS sync_device (
                device_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                app_version TEXT,
                schema_max INTEGER,
                last_seen_ms INTEGER NOT NULL,
                is_self INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS price_alert_local (
                alert_id TEXT PRIMARY KEY,
                evaluate_here INTEGER NOT NULL,
                origin TEXT NOT NULL DEFAULT 'local'
            );",
        )
    }

    /// Whether sync tracking is active. One indexed lookup; the write hooks call it on every save.
    pub fn sync_enabled(&self) -> bool {
        self.connection
            .query_row("SELECT enabled FROM sync_state WHERE id = 1", [], |row| {
                row.get::<_, bool>(0)
            })
            .optional()
            .ok()
            .flatten()
            .unwrap_or(false)
    }

    pub fn sync_state(&self) -> rusqlite::Result<Option<SyncState>> {
        self.connection
            .query_row(
                "SELECT enabled, device_id, device_name, hardware_hash, container_id, environment,
                    account_hash, account_bound_ms, dataset_id, zone_name, categories,
                    initial_sync_done, engine_state, engine_state_account_hash, last_fetch_ms,
                    last_send_ms, last_error
                 FROM sync_state WHERE id = 1",
                [],
                |row| {
                    Ok(SyncState {
                        enabled: row.get(0)?,
                        device_id: row.get(1)?,
                        device_name: row.get(2)?,
                        hardware_hash: row.get(3)?,
                        container_id: row.get(4)?,
                        environment: row.get(5)?,
                        account_hash: row.get(6)?,
                        account_bound_ms: row.get(7)?,
                        dataset_id: row.get(8)?,
                        zone_name: row.get(9)?,
                        categories: row.get(10)?,
                        initial_sync_done: row.get(11)?,
                        engine_state: row.get(12)?,
                        engine_state_account_hash: row.get(13)?,
                        last_fetch_ms: row.get(14)?,
                        last_send_ms: row.get(15)?,
                        last_error: row.get(16)?,
                    })
                },
            )
            .optional()
    }

    /// The sync identity, created disabled on first use. Existing rows are never overwritten,
    /// so the device id stays stable for the life of the database.
    pub fn ensure_sync_state(
        &self,
        device_name: &str,
        hardware_hash: &str,
    ) -> rusqlite::Result<SyncState> {
        self.connection.execute(
            "INSERT OR IGNORE INTO sync_state (id, device_id, device_name, hardware_hash)
             VALUES (1, ?1, ?2, ?3)",
            params![new_device_id(), device_name, hardware_hash],
        )?;
        self.sync_state()?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// Writes every mutable field of the singleton. Callers read-modify-write under the store
    /// lock; the device id and hardware hash change only through `reset_sync_device`.
    pub fn save_sync_state(&self, state: &SyncState) -> rusqlite::Result<()> {
        let updated = self.connection.execute(
            "UPDATE sync_state SET enabled = ?1, device_name = ?2, container_id = ?3,
                environment = ?4, account_hash = ?5, account_bound_ms = ?6, dataset_id = ?7,
                zone_name = ?8, categories = ?9, initial_sync_done = ?10, engine_state = ?11,
                engine_state_account_hash = ?12, last_fetch_ms = ?13, last_send_ms = ?14,
                last_error = ?15
             WHERE id = 1",
            params![
                state.enabled,
                state.device_name,
                state.container_id,
                state.environment,
                state.account_hash,
                state.account_bound_ms,
                state.dataset_id,
                state.zone_name,
                state.categories,
                state.initial_sync_done,
                state.engine_state,
                state.engine_state_account_hash,
                state.last_fetch_ms,
                state.last_send_ms,
                state.last_error,
            ],
        )?;
        if updated == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }

    /// Gives a cloned database a new identity and drops the per-device engine checkpoint.
    pub fn reset_sync_device(&self, hardware_hash: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_state SET device_id = ?1, hardware_hash = ?2, engine_state = NULL,
                engine_state_account_hash = NULL, initial_sync_done = 0 WHERE id = 1",
            params![new_device_id(), hardware_hash],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn table_exists(store: &Store, name: &str) -> bool {
        store
            .connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [name],
                |_| Ok(()),
            )
            .is_ok()
    }

    #[test]
    fn schema_creates_every_sync_table() {
        let store = Store::open_in_memory().unwrap();
        for table in [
            "sync_state",
            "sync_entity",
            "sync_outbox",
            "sync_inbox",
            "sync_conflict",
            "sync_device",
            "price_alert_local",
        ] {
            assert!(table_exists(&store, table), "{table}");
        }
    }

    #[test]
    fn schema_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        store.ensure_sync_state("Mac", "hw").unwrap();
        store.create_schema().unwrap();
        store.create_schema().unwrap();
        assert!(store.sync_state().unwrap().is_some());
    }

    #[test]
    fn a_database_from_before_sync_opens_and_keeps_its_data() {
        let path = std::env::temp_dir().join(format!("dv-sync-old-{}.sqlite", new_device_id()));
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO setting VALUES ('appTheme', '\"dark\"');",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert!(table_exists(&store, "sync_outbox"));
        assert_eq!(store.setting::<String>("appTheme").as_deref(), Some("dark"));
        assert!(!store.sync_enabled());
        drop(store);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sync_is_off_until_enabled() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.sync_enabled());
        assert!(store.sync_state().unwrap().is_none());
        let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
        assert!(!state.enabled);
        state.enabled = true;
        store.save_sync_state(&state).unwrap();
        assert!(store.sync_enabled());
    }

    #[test]
    fn device_identity_is_stable_and_never_overwritten() {
        let store = Store::open_in_memory().unwrap();
        let first = store.ensure_sync_state("Mac A", "hw-a").unwrap();
        let again = store.ensure_sync_state("Mac B", "hw-b").unwrap();
        assert_eq!(first.device_id, again.device_id);
        assert_eq!(again.device_name, "Mac A");
        assert_eq!(first.device_id.len(), 32);
        assert_eq!(first.zone_name, DEFAULT_ZONE);
    }

    #[test]
    fn state_round_trips() {
        let store = Store::open_in_memory().unwrap();
        let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
        state.enabled = true;
        state.account_hash = Some("acct".into());
        state.engine_state = Some(vec![1, 2, 3]);
        state.engine_state_account_hash = Some("acct".into());
        state.last_error = Some("offline".into());
        state.last_fetch_ms = Some(42);
        store.save_sync_state(&state).unwrap();
        assert_eq!(store.sync_state().unwrap().unwrap(), state);
    }

    #[test]
    fn saving_without_a_row_is_an_error() {
        let store = Store::open_in_memory().unwrap();
        let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
        store
            .connection
            .execute("DELETE FROM sync_state", [])
            .unwrap();
        state.enabled = true;
        assert!(store.save_sync_state(&state).is_err());
    }

    #[test]
    fn a_cloned_database_gets_a_new_identity_and_loses_its_checkpoint() {
        let store = Store::open_in_memory().unwrap();
        let mut state = store.ensure_sync_state("Mac", "hw-a").unwrap();
        state.engine_state = Some(vec![9]);
        state.initial_sync_done = true;
        store.save_sync_state(&state).unwrap();
        store.reset_sync_device("hw-b").unwrap();
        let reset = store.sync_state().unwrap().unwrap();
        assert_ne!(reset.device_id, state.device_id);
        assert_eq!(reset.hardware_hash, "hw-b");
        assert!(reset.engine_state.is_none());
        assert!(!reset.initial_sync_done);
    }

    #[test]
    fn outbox_rejects_an_unknown_operation() {
        let store = Store::open_in_memory().unwrap();
        let bad = store.connection.execute(
            "INSERT INTO sync_outbox (entity_type, sync_id, op, queued_ms) VALUES ('layout', '1', 'poke', 0)",
            [],
        );
        assert!(bad.is_err());
    }
}
