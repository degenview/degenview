//! Durable iCloud Sync metadata: the binding to one iCloud account, the engine checkpoint, and
//! (in later stages) the outbox and tombstones. These tables live beside the user's data so a
//! local write and the intent to upload it commit in one transaction. Older builds ignore them.

// Consumed by the sync core in later stages.
#![allow(dead_code)]

use super::Store;
use crate::sync::policy::{EntityKind, SettingPolicy, TablePolicy, setting_policy, table_policy};
use crate::sync::project::{Projected, project_row, project_setting};
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;

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

/// Whether a `setting` key syncs at all.
pub(super) fn tracks_setting(key: &str) -> bool {
    matches!(setting_policy(key), SettingPolicy::Portable(_))
}

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

/// One pending change waiting to upload. Only identity and operation are stored: the payload is
/// projected again from the live row when it is sent, so a queue entry can never go stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub seq: i64,
    pub kind: EntityKind,
    pub sync_id: String,
    pub op: OutboxOp,
    pub state: OutboxState,
    pub attempts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxOp {
    Upsert,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    Pending,
    /// A suspiciously large delete waiting for the user to confirm.
    Held,
    /// Cannot be sent as is (too large); needs the user's attention.
    Blocked,
}

/// Deleting this share of a kind, and at least `MASS_DELETE_MIN` entities, in one write is held
/// for confirmation instead of propagating to every Mac.
const MASS_DELETE_MIN: usize = 5;

/// The sync metadata of one entity, as far as the outbox needs it.
struct EntityMark {
    synced_hash: Option<String>,
    deleted: bool,
}

type EntityKey = (EntityKind, String);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

impl Store {
    /// The current rows of `table`, only when they will need diffing: sync is on and the table
    /// syncs. Read inside the write's savepoint, before the rows are replaced.
    pub(super) fn tracked_old_rows(
        &self,
        table: &str,
    ) -> rusqlite::Result<Option<Vec<(String, String)>>> {
        if !matches!(table_policy(table), TablePolicy::Synced(_)) || !self.sync_enabled() {
            return Ok(None);
        }
        let mut statement = self
            .connection
            .prepare(&format!("SELECT id, payload FROM {table}"))?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(rows))
    }

    /// Queues the entity-level differences between a table's old and new rows. Runs in the same
    /// savepoint as the write, so a change is never committed without its upload intent.
    pub(super) fn track_documents(
        &self,
        table: &str,
        old: &[(String, String)],
        new: &[(String, i64, String)],
    ) -> rusqlite::Result<()> {
        let project = |rows: &mut dyn Iterator<Item = (&str, &str)>| {
            let mut map: HashMap<EntityKey, Projected> = HashMap::new();
            for (id, payload) in rows {
                for entity in project_row(table, id, payload) {
                    map.insert((entity.kind, entity.sync_id.clone()), entity);
                }
            }
            map
        };
        let old_entities = project(&mut old.iter().map(|(id, p)| (id.as_str(), p.as_str())));
        let new_entities = project(&mut new.iter().map(|(id, _, p)| (id.as_str(), p.as_str())));
        self.queue_differences(&old_entities, &new_entities)
    }

    /// Queues the change to a portable setting; local settings are ignored.
    pub(super) fn track_setting(
        &self,
        key: &str,
        old: Option<&str>,
        new: Option<&str>,
    ) -> rusqlite::Result<()> {
        if !matches!(setting_policy(key), SettingPolicy::Portable(_)) {
            return Ok(());
        }
        let single = |value: Option<&str>| {
            let mut map: HashMap<EntityKey, Projected> = HashMap::new();
            if let Some(entity) = value.and_then(|value| project_setting(key, value)) {
                map.insert((entity.kind, entity.sync_id.clone()), entity);
            }
            map
        };
        self.queue_differences(&single(old), &single(new))
    }

    fn queue_differences(
        &self,
        old: &HashMap<EntityKey, Projected>,
        new: &HashMap<EntityKey, Projected>,
    ) -> rusqlite::Result<()> {
        let Some(state) = self.sync_state()? else {
            return Ok(());
        };
        let device = state.device_id;

        let mut changed: Vec<&Projected> = new
            .iter()
            .filter(|(key, entity)| old.get(*key).map(|before| &before.hash) != Some(&entity.hash))
            .map(|(_, entity)| entity)
            .collect();
        changed.sort_by(|a, b| (a.kind.prefix(), &a.sync_id).cmp(&(b.kind.prefix(), &b.sync_id)));
        for entity in changed {
            let mark = self.entity_mark(entity.kind, &entity.sync_id)?;
            // Equal to what the cloud already holds: a remote apply echoing back, or an edit that
            // was undone. Nothing to upload.
            let known = mark.as_ref().is_some_and(|m| {
                !m.deleted && m.synced_hash.as_deref() == Some(entity.hash.as_str())
            });
            if !known {
                self.queue_upsert(entity, &device)?;
            }
        }

        let mut vanished: Vec<&Projected> = old
            .iter()
            .filter(|(key, _)| !new.contains_key(*key))
            .map(|(_, entity)| entity)
            .collect();
        vanished.sort_by(|a, b| (a.kind.prefix(), &a.sync_id).cmp(&(b.kind.prefix(), &b.sync_id)));
        let held_layouts = self.mass_deleted_layouts(old, &vanished);
        for entity in vanished {
            let already = self
                .entity_mark(entity.kind, &entity.sync_id)?
                .is_some_and(|mark| mark.deleted);
            if already {
                continue;
            }
            let held = match entity.kind {
                EntityKind::LayoutDrawing => entity
                    .sync_id
                    .split('.')
                    .next()
                    .is_some_and(|layout| held_layouts.contains(layout)),
                _ => is_mass_delete(entity.kind, old, &vanished_count(old, new, entity.kind)),
            };
            let state = if held {
                OutboxState::Held
            } else {
                OutboxState::Pending
            };
            self.queue_delete(entity, &device, state)?;
        }
        Ok(())
    }

    /// Layouts whose delete is held, so the deletes of their drawings are held with them.
    fn mass_deleted_layouts(
        &self,
        old: &HashMap<EntityKey, Projected>,
        vanished: &[&Projected],
    ) -> std::collections::HashSet<String> {
        let gone = vanished
            .iter()
            .filter(|entity| entity.kind == EntityKind::Layout)
            .count();
        if is_mass_delete_count(gone, count_kind(old, EntityKind::Layout)) {
            vanished
                .iter()
                .filter(|entity| entity.kind == EntityKind::Layout)
                .map(|entity| entity.sync_id.clone())
                .collect()
        } else {
            Default::default()
        }
    }

    fn entity_mark(&self, kind: EntityKind, sync_id: &str) -> rusqlite::Result<Option<EntityMark>> {
        self.connection
            .query_row(
                "SELECT synced_hash, deleted FROM sync_entity WHERE entity_type = ?1 AND sync_id = ?2",
                params![kind.prefix(), sync_id],
                |row| {
                    Ok(EntityMark {
                        synced_hash: row.get(0)?,
                        deleted: row.get(1)?,
                    })
                },
            )
            .optional()
    }

    fn queue_upsert(&self, entity: &Projected, device: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO sync_entity (entity_type, sync_id, local_id, logical_revision, origin_device)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT (entity_type, sync_id) DO UPDATE SET
                local_id = excluded.local_id,
                logical_revision = logical_revision + 1,
                origin_device = excluded.origin_device,
                deleted = 0, deleted_ms = NULL",
            params![entity.kind.prefix(), entity.sync_id, entity.local_id, device],
        )?;
        self.enqueue(
            entity.kind,
            &entity.sync_id,
            OutboxOp::Upsert,
            OutboxState::Pending,
        )
    }

    fn queue_delete(
        &self,
        entity: &Projected,
        device: &str,
        state: OutboxState,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO sync_entity
                (entity_type, sync_id, local_id, logical_revision, origin_device, deleted, deleted_ms)
             VALUES (?1, ?2, ?3, 1, ?4, 1, ?5)
             ON CONFLICT (entity_type, sync_id) DO UPDATE SET
                logical_revision = logical_revision + 1,
                origin_device = excluded.origin_device,
                deleted = 1, deleted_ms = excluded.deleted_ms",
            params![
                entity.kind.prefix(),
                entity.sync_id,
                entity.local_id,
                device,
                now_ms()
            ],
        )?;
        self.enqueue(entity.kind, &entity.sync_id, OutboxOp::Delete, state)
    }

    /// Re-queuing replaces the row, giving it a new `seq`: an acknowledgement of the older upload
    /// then cannot delete the newer intent.
    fn enqueue(
        &self,
        kind: EntityKind,
        sync_id: &str,
        op: OutboxOp,
        state: OutboxState,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "DELETE FROM sync_outbox WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id],
        )?;
        self.connection.execute(
            "INSERT INTO sync_outbox (entity_type, sync_id, op, state, queued_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                kind.prefix(),
                sync_id,
                match op {
                    OutboxOp::Upsert => "upsert",
                    OutboxOp::Delete => "delete",
                },
                match state {
                    OutboxState::Pending => "pending",
                    OutboxState::Held => "held",
                    OutboxState::Blocked => "blocked",
                },
                now_ms()
            ],
        )?;
        Ok(())
    }

    /// Everything waiting to upload, oldest first.
    pub fn outbox(&self) -> rusqlite::Result<Vec<OutboxRow>> {
        let mut statement = self.connection.prepare(
            "SELECT seq, entity_type, sync_id, op, state, attempts FROM sync_outbox ORDER BY seq",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(seq, kind, sync_id, op, state, attempts)| {
                Some(OutboxRow {
                    seq,
                    kind: EntityKind::from_prefix(&kind)?,
                    sync_id,
                    op: if op == "delete" {
                        OutboxOp::Delete
                    } else {
                        OutboxOp::Upsert
                    },
                    state: match state.as_str() {
                        "held" => OutboxState::Held,
                        "blocked" => OutboxState::Blocked,
                        _ => OutboxState::Pending,
                    },
                    attempts,
                })
            })
            .collect())
    }
}

fn count_kind(entities: &HashMap<EntityKey, Projected>, kind: EntityKind) -> usize {
    entities.keys().filter(|(k, _)| *k == kind).count()
}

fn vanished_count(
    old: &HashMap<EntityKey, Projected>,
    new: &HashMap<EntityKey, Projected>,
    kind: EntityKind,
) -> usize {
    old.keys()
        .filter(|key| key.0 == kind && !new.contains_key(*key))
        .count()
}

fn is_mass_delete(kind: EntityKind, old: &HashMap<EntityKey, Projected>, gone: &usize) -> bool {
    is_mass_delete_count(*gone, count_kind(old, kind))
}

fn is_mass_delete_count(gone: usize, before: usize) -> bool {
    gone >= MASS_DELETE_MIN && gone * 2 > before
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

#[cfg(test)]
mod hook_tests {
    use super::*;
    use crate::store::Document;
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(transparent)]
    struct Raw(Value);

    impl Document for Raw {
        fn document_id(&self) -> String {
            self.0["id"].to_string()
        }
    }

    fn store(enabled: bool) -> Store {
        let store = Store::open_in_memory().unwrap();
        let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
        state.enabled = enabled;
        store.save_sync_state(&state).unwrap();
        store
    }

    fn watchlist(id: u64, name: &str) -> Raw {
        Raw(json!({"id": id, "name": name, "entries": [], "is_favorites": false}))
    }

    fn put(store: &Store, table: &str, rows: &[Raw]) {
        store.replace_documents(table, rows).unwrap();
    }

    fn outbox(store: &Store) -> Vec<(String, String, OutboxOp, OutboxState)> {
        store
            .outbox()
            .unwrap()
            .into_iter()
            .map(|row| {
                (
                    row.kind.prefix().to_string(),
                    row.sync_id,
                    row.op,
                    row.state,
                )
            })
            .collect()
    }

    fn mark_synced(store: &Store, kind: EntityKind, sync_id: &str, hash: &str) {
        store
            .connection
            .execute(
                "UPDATE sync_entity SET synced_hash = ?3, deleted = 0 WHERE entity_type = ?1 AND sync_id = ?2",
                params![kind.prefix(), sync_id, hash],
            )
            .unwrap();
    }

    fn revision(store: &Store, kind: EntityKind, sync_id: &str) -> i64 {
        store
            .connection
            .query_row(
                "SELECT logical_revision FROM sync_entity WHERE entity_type = ?1 AND sync_id = ?2",
                params![kind.prefix(), sync_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn is_tombstone(store: &Store, kind: EntityKind, sync_id: &str) -> bool {
        store
            .connection
            .query_row(
                "SELECT deleted FROM sync_entity WHERE entity_type = ?1 AND sync_id = ?2",
                params![kind.prefix(), sync_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn nothing_is_tracked_while_sync_is_off() {
        let store = store(false);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        store.set_setting("appTheme", &"dark").unwrap();
        assert!(outbox(&store).is_empty());
        let entities: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_entity", [], |row| row.get(0))
            .unwrap();
        assert_eq!(entities, 0);
    }

    #[test]
    fn nothing_is_tracked_before_a_sync_identity_exists() {
        let store = Store::open_in_memory().unwrap();
        put(&store, "watchlist", &[watchlist(1, "A")]);
        assert!(outbox(&store).is_empty());
    }

    #[test]
    fn a_new_row_queues_an_upload() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        assert_eq!(
            outbox(&store),
            [(
                "watchlist".into(),
                "1".into(),
                OutboxOp::Upsert,
                OutboxState::Pending
            )]
        );
        assert_eq!(revision(&store, EntityKind::Watchlist, "1"), 1);
    }

    #[test]
    fn local_tables_are_never_tracked() {
        let store = store(true);
        put(&store, "tab", &[watchlist(1, "A")]);
        put(&store, "mcp_client", &[watchlist(2, "B")]);
        put(&store, "webhook_endpoint", &[watchlist(3, "C")]);
        assert!(outbox(&store).is_empty());
    }

    #[test]
    fn only_the_row_that_changed_is_queued() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A"), watchlist(2, "B")]);
        let first = store.outbox().unwrap();
        let hashes: Vec<_> = first.iter().map(|row| row.sync_id.clone()).collect();
        for row in &first {
            let projected = project_row(
                "watchlist",
                &row.sync_id,
                &watchlist(
                    row.sync_id.parse().unwrap(),
                    if row.sync_id == "1" { "A" } else { "B" },
                )
                .0
                .to_string(),
            );
            mark_synced(&store, row.kind, &row.sync_id, &projected[0].hash);
        }
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(
            &store,
            "watchlist",
            &[watchlist(1, "A"), watchlist(2, "B2")],
        );
        let queued = outbox(&store);
        assert_eq!(hashes.len(), 2);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].1, "2");
    }

    #[test]
    fn rewriting_identical_rows_queues_nothing() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "watchlist", &[watchlist(1, "A")]);
        assert!(outbox(&store).is_empty());
    }

    #[test]
    fn repeated_edits_coalesce_into_one_entry_with_a_newer_seq() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        let first = store.outbox().unwrap()[0].seq;
        put(&store, "watchlist", &[watchlist(1, "B")]);
        put(&store, "watchlist", &[watchlist(1, "C")]);
        let rows = store.outbox().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].seq > first);
        assert_eq!(revision(&store, EntityKind::Watchlist, "1"), 3);
    }

    #[test]
    fn a_remote_apply_does_not_echo_back() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        // The apply layer records what the cloud holds, then writes the row.
        let remote = watchlist(1, "From other Mac");
        let hash = project_row("watchlist", "1", &remote.0.to_string())[0]
            .hash
            .clone();
        mark_synced(&store, EntityKind::Watchlist, "1", &hash);
        put(&store, "watchlist", &[remote]);
        assert!(outbox(&store).is_empty());
    }

    #[test]
    fn undoing_an_edit_back_to_the_synced_state_queues_nothing() {
        let store = store(true);
        let original = watchlist(1, "A");
        put(&store, "watchlist", std::slice::from_ref(&original));
        let hash = project_row("watchlist", "1", &original.0.to_string())[0]
            .hash
            .clone();
        mark_synced(&store, EntityKind::Watchlist, "1", &hash);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "watchlist", &[watchlist(1, "B")]);
        assert_eq!(outbox(&store).len(), 1);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "watchlist", &[original]);
        assert!(outbox(&store).is_empty());
    }

    #[test]
    fn a_failed_outbox_write_rolls_the_document_back() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER no_outbox BEFORE INSERT ON sync_outbox
                 BEGIN SELECT RAISE(ABORT, 'outbox full'); END;",
            )
            .unwrap();
        let result = store.replace_documents("watchlist", &[watchlist(1, "B")]);
        assert!(result.is_err());
        let rows: Vec<Raw> = store.documents("watchlist");
        assert_eq!(rows[0].0["name"], "A");
    }

    #[test]
    fn removing_a_row_queues_a_delete_and_leaves_a_tombstone() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A"), watchlist(2, "B")]);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "watchlist", &[watchlist(2, "B")]);
        assert_eq!(
            outbox(&store),
            [(
                "watchlist".into(),
                "1".into(),
                OutboxOp::Delete,
                OutboxState::Pending
            )]
        );
        assert!(is_tombstone(&store, EntityKind::Watchlist, "1"));
    }

    #[test]
    fn a_deleted_row_that_returns_clears_its_tombstone() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        put(&store, "watchlist", &[]);
        assert!(is_tombstone(&store, EntityKind::Watchlist, "1"));
        put(&store, "watchlist", &[watchlist(1, "A")]);
        assert!(!is_tombstone(&store, EntityKind::Watchlist, "1"));
        assert_eq!(outbox(&store)[0].2, OutboxOp::Upsert);
    }

    #[test]
    fn a_remote_delete_does_not_echo_back() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        store
            .connection
            .execute("UPDATE sync_entity SET deleted = 1, deleted_ms = 1", [])
            .unwrap();
        put(&store, "watchlist", &[]);
        assert!(outbox(&store).is_empty());
    }

    #[test]
    fn deleting_most_of_a_collection_is_held_for_confirmation() {
        let store = store(true);
        let rows: Vec<_> = (1..=6).map(|id| watchlist(id, "x")).collect();
        put(&store, "watchlist", &rows);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "watchlist", &[]);
        let queued = outbox(&store);
        assert_eq!(queued.len(), 6);
        assert!(queued.iter().all(|row| row.3 == OutboxState::Held));
    }

    #[test]
    fn deleting_a_few_rows_is_not_held() {
        let store = store(true);
        let rows: Vec<_> = (1..=10).map(|id| watchlist(id, "x")).collect();
        put(&store, "watchlist", &rows);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "watchlist", &rows[4..]);
        let queued = outbox(&store);
        assert_eq!(queued.len(), 4);
        assert!(queued.iter().all(|row| row.3 == OutboxState::Pending));
    }

    #[test]
    fn deleting_a_small_collection_entirely_is_not_held() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "x"), watchlist(2, "y")]);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "watchlist", &[]);
        assert!(
            outbox(&store)
                .iter()
                .all(|row| row.3 == OutboxState::Pending)
        );
    }

    fn alert(state: &str, armed: bool, previous: f64, updated: i64) -> Raw {
        Raw(json!({
            "id": 5, "asset": {"symbol": "BTC"}, "condition": {"Above": 100.0},
            "currency": "USD", "frequency": "Once", "state": state, "note": "",
            "armed": armed, "previous_value": previous, "last_fingerprint": "f",
            "created_at_ms": 1, "updated_at_ms": updated,
            "webhook_endpoint_ids": [], "webhook_message": null
        }))
    }

    #[test]
    fn the_alert_engine_heartbeat_queues_nothing() {
        let store = store(true);
        put(&store, "price_alert", &[alert("Active", true, 90.0, 1)]);
        assert_eq!(outbox(&store).len(), 1);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        for tick in 0..20 {
            put(
                &store,
                "price_alert",
                &[alert(
                    "Active",
                    tick % 2 == 0,
                    90.0 + tick as f64,
                    1000 + tick,
                )],
            );
        }
        put(
            &store,
            "price_alert",
            &[alert("Triggered", false, 120.0, 5000)],
        );
        assert!(outbox(&store).is_empty());
        put(&store, "price_alert", &[alert("Paused", true, 120.0, 6000)]);
        assert_eq!(outbox(&store).len(), 1);
    }

    fn layout(drawings: Vec<Value>, opened: i64) -> Raw {
        Raw(json!({
            "id": 10, "name": "Main", "last_opened_at_ms": opened,
            "charts": [{"id": 2, "symbol": "BTC", "drawings": drawings}]
        }))
    }

    fn trend(id: u64, price: f64) -> Value {
        json!({"TrendLine": {"id": id, "a": {"time_ms": 1, "price": price}}})
    }

    #[test]
    fn a_drawing_edit_queues_only_that_drawing() {
        let store = store(true);
        put(
            &store,
            "saved_view",
            &[layout(vec![trend(1, 1.0), trend(2, 2.0)], 1)],
        );
        let kinds: Vec<_> = outbox(&store).into_iter().map(|row| row.1).collect();
        assert_eq!(kinds, ["10", "10.2.1", "10.2.2"]);
        for row in store.outbox().unwrap() {
            store
                .connection
                .execute(
                    "UPDATE sync_entity SET synced_hash = 'x' WHERE sync_id = ?1",
                    [&row.sync_id],
                )
                .unwrap();
        }
        // Re-sync the hashes exactly as an acknowledgement would.
        let stored: Vec<(String, String)> = {
            let mut statement = store
                .connection
                .prepare("SELECT id, payload FROM saved_view")
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        for (id, payload) in stored {
            for entity in project_row("saved_view", &id, &payload) {
                mark_synced(&store, entity.kind, &entity.sync_id, &entity.hash);
            }
        }
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(
            &store,
            "saved_view",
            &[layout(vec![trend(1, 1.0), trend(2, 9.0)], 50)],
        );
        let queued = outbox(&store);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].1, "10.2.2");
    }

    #[test]
    fn removing_a_drawing_queues_its_delete() {
        let store = store(true);
        put(
            &store,
            "saved_view",
            &[layout(vec![trend(1, 1.0), trend(2, 2.0)], 1)],
        );
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "saved_view", &[layout(vec![trend(1, 1.0)], 1)]);
        let queued = outbox(&store);
        assert!(
            queued.iter().any(|row| row.0 == "layoutdrawing"
                && row.1 == "10.2.2"
                && row.2 == OutboxOp::Delete)
        );
    }

    #[test]
    fn deleting_a_big_layout_holds_its_drawings_with_it() {
        let store = store(true);
        let layouts: Vec<_> = (1..=6)
            .map(|id| {
                Raw(json!({"id": id, "name": "x",
                    "charts": [{"id": 1, "drawings": [trend(1, 1.0)]}]}))
            })
            .collect();
        put(&store, "saved_view", &layouts);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "saved_view", &[]);
        let queued = outbox(&store);
        assert_eq!(queued.len(), 12);
        assert!(queued.iter().all(|row| row.3 == OutboxState::Held));
    }

    #[test]
    fn deleting_one_layout_does_not_hold_its_many_drawings() {
        let store = store(true);
        let drawings: Vec<_> = (1..=8).map(|id| trend(id, 1.0)).collect();
        let layouts = [
            Raw(json!({"id": 1, "name": "a", "charts": [{"id": 1, "drawings": drawings}]})),
            Raw(json!({"id": 2, "name": "b", "charts": []})),
            Raw(json!({"id": 3, "name": "c", "charts": []})),
        ];
        put(&store, "saved_view", &layouts);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        put(&store, "saved_view", &layouts[1..]);
        let queued = outbox(&store);
        assert_eq!(queued.len(), 9);
        assert!(queued.iter().all(|row| row.3 == OutboxState::Pending));
    }

    #[test]
    fn portable_settings_queue_and_local_settings_do_not() {
        let store = store(true);
        store.set_setting("appTheme", &"dark").unwrap();
        store
            .set_setting("alert.health", &json!({"heartbeat_ms": 1}))
            .unwrap();
        store
            .set_setting("mcp.settings", &json!({"port": 1}))
            .unwrap();
        assert_eq!(
            outbox(&store),
            [(
                "setting".into(),
                "appTheme".into(),
                OutboxOp::Upsert,
                OutboxState::Pending
            )]
        );
    }

    #[test]
    fn clearing_a_portable_setting_queues_a_delete() {
        let store = store(true);
        store.set_setting("appTheme", &"dark").unwrap();
        store.remove_setting("appTheme").unwrap();
        assert_eq!(outbox(&store)[0].2, OutboxOp::Delete);
        assert!(store.setting::<String>("appTheme").is_none());
    }

    #[test]
    fn removing_a_setting_that_never_existed_queues_nothing() {
        let store = store(true);
        store.remove_setting("appTheme").unwrap();
        assert!(outbox(&store).is_empty());
    }

    #[test]
    fn a_failed_setting_outbox_write_rolls_the_setting_back() {
        let store = store(true);
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER no_outbox BEFORE INSERT ON sync_outbox
                 BEGIN SELECT RAISE(ABORT, 'outbox full'); END;",
            )
            .unwrap();
        assert!(store.set_setting("appTheme", &"dark").is_err());
        assert!(store.setting::<String>("appTheme").is_none());
    }

    #[test]
    fn consenting_to_a_snapshot_host_does_not_sync() {
        let store = store(true);
        let prefs = |consented: Vec<&str>| json!({"host": "h.example", "include_trading": false, "scale": "X2", "consented": consented});
        store.set_setting("snapshot.prefs", &prefs(vec![])).unwrap();
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        store
            .set_setting("snapshot.prefs", &prefs(vec!["h.example"]))
            .unwrap();
        assert!(outbox(&store).is_empty());
    }

    #[test]
    fn writes_inside_an_outer_transaction_are_tracked_and_roll_back_together() {
        let store = store(true);
        let result: rusqlite::Result<()> = store.in_transaction(|| {
            store.replace_documents("watchlist", &[watchlist(1, "A")])?;
            Err(rusqlite::Error::QueryReturnedNoRows)
        });
        assert!(result.is_err());
        assert!(outbox(&store).is_empty());
        assert!(store.documents::<Raw>("watchlist").is_empty());
    }
}
