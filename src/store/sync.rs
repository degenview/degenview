//! Durable iCloud Sync metadata: the binding to one iCloud account, the engine checkpoint, and
//! (in later stages) the outbox and tombstones. These tables live beside the user's data so a
//! local write and the intent to upload it commit in one transaction. Older builds ignore them.

// Consumed by the sync core in later stages.
#![allow(dead_code)]

use super::Store;
use crate::sync::policy::{
    Category, EntityKind, SettingPolicy, TablePolicy, setting_policy, table_policy,
};
use crate::sync::project::{Projected, project_row, project_setting};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;

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
                revision INTEGER NOT NULL DEFAULT 0,
                origin_device TEXT NOT NULL DEFAULT '',
                modified_ms INTEGER NOT NULL DEFAULT 0,
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

    /// The categories the user switched off. A missing entry is on.
    pub fn disabled_categories(&self) -> HashSet<Category> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT categories FROM sync_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let map: std::collections::HashMap<String, bool> = json
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Category::ALL
            .into_iter()
            .filter(|category| map.get(category.key()) == Some(&false))
            .collect()
    }

    /// Turns one category on or off. Turning one off stops its changes from being queued or
    /// applied; turning it on is followed by a reconcile and a retry of what was held back.
    pub fn set_category_enabled(&self, category: Category, enabled: bool) -> rusqlite::Result<()> {
        let Some(mut state) = self.sync_state()? else {
            return Ok(());
        };
        let mut map: std::collections::HashMap<String, bool> =
            serde_json::from_str(&state.categories).unwrap_or_default();
        map.insert(category.key().to_string(), enabled);
        state.categories = serde_json::to_string(&map).unwrap_or_else(|_| "{}".into());
        self.save_sync_state(&state)
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
    pub queued_ms: i64,
    pub next_attempt_ms: Option<i64>,
    pub last_error: Option<String>,
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
        let off = self.disabled_categories();

        let mut changed: Vec<&Projected> = new
            .iter()
            .filter(|(key, entity)| old.get(*key).map(|before| &before.hash) != Some(&entity.hash))
            .map(|(_, entity)| entity)
            .filter(|entity| !off.contains(&entity.kind.category()))
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
            .filter(|entity| !off.contains(&entity.kind.category()))
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
        self.outbox_where("1 = 1", [])
    }

    /// Pending entries whose retry time has come, oldest first.
    pub fn outbox_due(&self, now_ms: i64) -> rusqlite::Result<Vec<OutboxRow>> {
        self.outbox_where(
            "state = 'pending' AND (next_attempt_ms IS NULL OR next_attempt_ms <= ?1)",
            [now_ms],
        )
    }

    fn outbox_where<P: rusqlite::Params>(
        &self,
        condition: &str,
        params: P,
    ) -> rusqlite::Result<Vec<OutboxRow>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT seq, entity_type, sync_id, op, state, attempts, queued_ms, next_attempt_ms,
                last_error FROM sync_outbox WHERE {condition} ORDER BY seq"
        ))?;
        let rows = statement
            .query_map(params, |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(
                |(
                    seq,
                    kind,
                    sync_id,
                    op,
                    state,
                    attempts,
                    queued_ms,
                    next_attempt_ms,
                    last_error,
                )| {
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
                        queued_ms,
                        next_attempt_ms,
                        last_error,
                    })
                },
            )
            .collect())
    }

    /// The live entity for an outbox entry, projected fresh so a payload is never stale.
    pub fn load_projected(&self, kind: EntityKind, sync_id: &str) -> rusqlite::Result<Load> {
        let by_id = |table: &str, id: &str| -> rusqlite::Result<Option<String>> {
            self.connection
                .query_row(
                    &format!("SELECT payload FROM {table} WHERE id = ?1"),
                    [id],
                    |row| row.get(0),
                )
                .optional()
        };
        let pick = |table: &str, id: &str, payload: Option<String>| {
            payload
                .into_iter()
                .flat_map(|payload| project_row(table, id, &payload))
                .find(|entity| entity.kind == kind && entity.sync_id == sync_id)
        };
        let found = match kind {
            EntityKind::Script => return Ok(Load::NotStored),
            EntityKind::Layout => pick("saved_view", sync_id, by_id("saved_view", sync_id)?),
            EntityKind::LayoutDrawing => {
                let layout = sync_id.split('.').next().unwrap_or_default();
                pick("saved_view", layout, by_id("saved_view", layout)?)
            }
            EntityKind::Watchlist => {
                let mut statement = self
                    .connection
                    .prepare("SELECT id, payload FROM watchlist")?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter()
                    .flat_map(|(id, payload)| project_row("watchlist", &id, &payload))
                    .find(|entity| entity.sync_id == sync_id)
            }
            EntityKind::ChartTemplate => {
                pick("chart_template", sync_id, by_id("chart_template", sync_id)?)
            }
            EntityKind::DrawingTemplate => pick(
                "drawing_template",
                sync_id,
                by_id("drawing_template", sync_id)?,
            ),
            EntityKind::Alert => pick("price_alert", sync_id, by_id("price_alert", sync_id)?),
            EntityKind::Setting => {
                let value: Option<String> = self
                    .connection
                    .query_row(
                        "SELECT value FROM setting WHERE key = ?1",
                        [sync_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                value.and_then(|value| project_setting(sync_id, &value))
            }
        };
        Ok(found.map_or(Load::Gone, Load::Found))
    }

    /// Entities the cloud is known to hold, as `(kind, sync_id)`: confirmed uploads and applied
    /// remote records, excluding deletions.
    pub fn synced_entities(&self) -> rusqlite::Result<Vec<(EntityKind, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT entity_type, sync_id FROM sync_entity WHERE deleted = 0 AND synced_hash IS NOT NULL",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, id)| Some((EntityKind::from_prefix(&kind)?, id)))
            .collect())
    }

    /// Sync metadata for one entity.
    pub fn entity_meta(
        &self,
        kind: EntityKind,
        sync_id: &str,
    ) -> rusqlite::Result<Option<EntityMeta>> {
        self.connection
            .query_row(
                "SELECT logical_revision, origin_device, synced_hash, system_fields, deleted, deleted_ms,
                    base_payload
                 FROM sync_entity WHERE entity_type = ?1 AND sync_id = ?2",
                params![kind.prefix(), sync_id],
                |row| {
                    Ok(EntityMeta {
                        revision: row.get(0)?,
                        origin_device: row.get(1)?,
                        synced_hash: row.get(2)?,
                        system_fields: row.get(3)?,
                        deleted: row.get(4)?,
                        deleted_ms: row.get(5)?,
                        base_payload: row.get(6)?,
                    })
                },
            )
            .optional()
    }

    /// The cloud accepted an upload. Records what the cloud now holds, then removes the outbox
    /// entry only if it is still the one that was sent: a newer local edit keeps its place.
    pub fn ack_saved(
        &self,
        seq: i64,
        kind: EntityKind,
        sync_id: &str,
        sent_hash: &str,
        system_fields: &[u8],
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        self.in_savepoint("ack_saved", || {
            self.connection.execute(
                "UPDATE sync_entity SET synced_hash = ?3, system_fields = ?4, cloud_modified_ms = ?5
                 WHERE entity_type = ?1 AND sync_id = ?2",
                params![kind.prefix(), sync_id, sent_hash, system_fields, now_ms],
            )?;
            self.connection
                .execute("DELETE FROM sync_outbox WHERE seq = ?1", [seq])?;
            Ok(())
        })
    }

    /// The cloud confirmed a delete (or that the record was already gone).
    pub fn ack_deleted(&self, seq: i64, kind: EntityKind, sync_id: &str) -> rusqlite::Result<()> {
        self.in_savepoint("ack_deleted", || {
            self.connection.execute(
                "UPDATE sync_entity SET synced_hash = NULL, system_fields = NULL
                 WHERE entity_type = ?1 AND sync_id = ?2 AND deleted = 1",
                params![kind.prefix(), sync_id],
            )?;
            self.connection
                .execute("DELETE FROM sync_outbox WHERE seq = ?1", [seq])?;
            Ok(())
        })
    }

    /// Keeps the cloud's change tag current without touching the outbox.
    pub fn set_system_fields(
        &self,
        kind: EntityKind,
        sync_id: &str,
        system_fields: &[u8],
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_entity SET system_fields = ?3 WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id, system_fields],
        )?;
        Ok(())
    }

    /// An upload failed and will be retried at `next_attempt_ms`.
    pub fn fail_outbox(&self, seq: i64, next_attempt_ms: i64, error: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_outbox SET attempts = attempts + 1, next_attempt_ms = ?2, last_error = ?3
             WHERE seq = ?1",
            params![seq, next_attempt_ms, error],
        )?;
        Ok(())
    }

    pub fn set_outbox_state(
        &self,
        seq: i64,
        state: OutboxState,
        error: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_outbox SET state = ?2, last_error = ?3 WHERE seq = ?1",
            params![
                seq,
                match state {
                    OutboxState::Pending => "pending",
                    OutboxState::Held => "held",
                    OutboxState::Blocked => "blocked",
                },
                error
            ],
        )?;
        Ok(())
    }

    /// Drops an outbox entry, but only the exact one that was looked at.
    pub fn remove_outbox(&self, seq: i64) -> rusqlite::Result<()> {
        self.connection
            .execute("DELETE FROM sync_outbox WHERE seq = ?1", [seq])?;
        Ok(())
    }

    #[cfg(test)]
    pub fn execute_raw(&self, sql: &str) -> rusqlite::Result<usize> {
        self.connection.execute(sql, [])
    }

    /// Makes an entry due immediately without counting a failed attempt.
    pub fn retry_now(&self, seq: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_outbox SET next_attempt_ms = NULL WHERE seq = ?1",
            [seq],
        )?;
        Ok(())
    }

    /// Forgets the cloud's change tag for one entity so the next save creates it afresh.
    pub fn clear_system_fields(&self, kind: EntityKind, sync_id: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_entity SET system_fields = NULL WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id],
        )?;
        Ok(())
    }

    /// iCloud lost its keys or the zone was wiped: nothing is known to be in the cloud any more.
    pub fn forget_cloud_state(&self) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_entity SET system_fields = NULL, synced_hash = NULL WHERE deleted = 0",
            [],
        )?;
        Ok(())
    }

    /// Persists the engine checkpoint together with the account it belongs to.
    pub fn save_engine_state(
        &self,
        state: Option<&[u8]>,
        account_hash: Option<&str>,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_state SET engine_state = ?1, engine_state_account_hash = ?2,
                last_fetch_ms = ?3 WHERE id = 1",
            params![state, account_hash, now_ms],
        )?;
        Ok(())
    }

    pub fn set_last_send(&self, now_ms: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_state SET last_send_ms = ?1 WHERE id = 1",
            [now_ms],
        )?;
        Ok(())
    }

    /// Keeps a remote record that is not applied, to be reconsidered later (an update may teach
    /// this version to read it).
    pub fn put_inbox(&self, record: &InboxRecord, now_ms: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO sync_inbox
                (record_name, entity_type, schema_version, reason, payload, system_fields, revision,
                 origin_device, modified_ms, received_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.record_name,
                record.entity_type,
                record.schema_version,
                record.reason,
                record.payload,
                record.system_fields,
                record.revision as i64,
                record.origin_device,
                record.modified_ms,
                now_ms
            ],
        )?;
        Ok(())
    }

    pub fn inbox(&self) -> rusqlite::Result<Vec<InboxRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT record_name, entity_type, schema_version, reason, payload, system_fields,
                revision, origin_device, modified_ms
             FROM sync_inbox ORDER BY received_ms, record_name",
        )?;
        statement
            .query_map([], |row| {
                Ok(InboxRecord {
                    record_name: row.get(0)?,
                    entity_type: row.get(1)?,
                    schema_version: row.get(2)?,
                    reason: row.get(3)?,
                    payload: row.get(4)?,
                    system_fields: row.get(5)?,
                    revision: row.get::<_, i64>(6)? as u64,
                    origin_device: row.get(7)?,
                    modified_ms: row.get(8)?,
                })
            })?
            .collect()
    }

    pub fn remove_inbox(&self, record_name: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "DELETE FROM sync_inbox WHERE record_name = ?1",
            [record_name],
        )?;
        Ok(())
    }

    /// Conflicts the user has not resolved or dismissed.
    pub fn open_conflict_count(&self) -> rusqlite::Result<usize> {
        self.connection.query_row(
            "SELECT COUNT(*) FROM sync_conflict WHERE resolved_ms IS NULL",
            [],
            |row| row.get::<_, i64>(0).map(|n| n as usize),
        )
    }

    // MARK: Applying remote changes

    /// Every row of a synced table as JSON, in order. A row that is not JSON fails the read: a
    /// collection with an unreadable row must never be rewritten.
    pub fn raw_rows(&self, table: &str) -> rusqlite::Result<Vec<Value>> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT payload FROM {table} ORDER BY position"))?;
        let payloads = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        payloads
            .into_iter()
            .map(|payload| {
                serde_json::from_str(&payload)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })
            .collect()
    }

    /// Replaces a synced table with `rows`, through the same tracked path as every other write.
    /// Refuses unless every existing row, and every new row, decodes as the table's model: this
    /// version must never write something it cannot read back, nor overwrite what a newer
    /// version wrote.
    pub fn write_raw_rows(&self, table: &str, rows: &[Value]) -> rusqlite::Result<()> {
        let unreadable = |reason: &str| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(format!(
                "{table}: {reason}"
            ))))
        };
        for existing in self.raw_rows(table)? {
            if !row_decodes(table, &existing.to_string()) {
                return Err(unreadable("an existing row cannot be decoded"));
            }
        }
        if rows.iter().any(|row| !row_decodes(table, &row.to_string())) {
            return Err(unreadable("a new row cannot be decoded"));
        }
        let docs: Vec<RawDoc> = rows.iter().cloned().map(RawDoc).collect();
        self.replace_documents(table, &docs)
    }

    pub fn raw_setting(&self, key: &str) -> rusqlite::Result<Option<Value>> {
        self.connection
            .query_row("SELECT value FROM setting WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .map(|text| {
                serde_json::from_str(&text)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })
            .transpose()
    }

    /// Writes or clears a setting through the tracked path.
    pub fn write_raw_setting(&self, key: &str, value: Option<&Value>) -> rusqlite::Result<()> {
        self.write_setting(key, value.map(Value::to_string).as_deref())
    }

    /// Records that the cloud holds exactly `link.synced_hash` for this entity, so writing that
    /// content locally is not an edit.
    pub fn link_entity(&self, link: &EntityLink) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO sync_entity
                (entity_type, sync_id, local_id, logical_revision, origin_device, synced_hash,
                 base_payload, system_fields, cloud_modified_ms, deleted, deleted_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, NULL)
             ON CONFLICT (entity_type, sync_id) DO UPDATE SET
                local_id = excluded.local_id,
                logical_revision = MAX(logical_revision, excluded.logical_revision),
                synced_hash = excluded.synced_hash,
                base_payload = excluded.base_payload,
                system_fields = excluded.system_fields,
                cloud_modified_ms = excluded.cloud_modified_ms,
                deleted = 0, deleted_ms = NULL",
            params![
                link.kind.prefix(),
                link.sync_id,
                link.local_id,
                link.revision,
                link.origin_device,
                link.synced_hash,
                link.base_payload,
                link.system_fields,
                link.now_ms
            ],
        )?;
        Ok(())
    }

    /// The entity is gone locally and in the cloud.
    pub fn tombstone_entity(
        &self,
        kind: EntityKind,
        sync_id: &str,
        local_id: &str,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO sync_entity
                (entity_type, sync_id, local_id, logical_revision, origin_device, deleted, deleted_ms)
             VALUES (?1, ?2, ?3, 1, '', 1, ?4)
             ON CONFLICT (entity_type, sync_id) DO UPDATE SET
                synced_hash = NULL, system_fields = NULL, base_payload = NULL,
                deleted = 1, deleted_ms = excluded.deleted_ms",
            params![kind.prefix(), sync_id, local_id, now_ms],
        )?;
        Ok(())
    }

    /// Forgets what the cloud holds for an entity that must be uploaded again as new.
    pub fn forget_synced(&self, kind: EntityKind, sync_id: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_entity SET synced_hash = NULL, system_fields = NULL, base_payload = NULL,
                deleted = 0, deleted_ms = NULL WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id],
        )?;
        Ok(())
    }

    /// Queues an upload for an entity by identity, for callers that already changed the row.
    pub fn queue_entity_upsert(
        &self,
        kind: EntityKind,
        sync_id: &str,
        local_id: &str,
    ) -> rusqlite::Result<()> {
        let Some(state) = self.sync_state()? else {
            return Ok(());
        };
        let stub = Projected {
            kind,
            sync_id: sync_id.to_string(),
            local_id: local_id.to_string(),
            payload: Vec::new(),
            hash: String::new(),
        };
        self.queue_upsert(&stub, &state.device_id)
    }

    pub fn outbox_entry(
        &self,
        kind: EntityKind,
        sync_id: &str,
    ) -> rusqlite::Result<Option<OutboxRow>> {
        Ok(self
            .outbox()?
            .into_iter()
            .find(|row| row.kind == kind && row.sync_id == sync_id))
    }

    pub fn cancel_outbox(&self, kind: EntityKind, sync_id: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "DELETE FROM sync_outbox WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id],
        )?;
        Ok(())
    }

    /// Notes a conflict the user may want to look at; both versions stay recoverable.
    pub fn record_conflict(&self, conflict: &ConflictRecord, now_ms: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO sync_conflict
                (entity_type, sync_id, kind, local_payload, remote_payload, remote_device,
                 copy_entity_type, copy_sync_id, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                conflict.entity.prefix(),
                conflict.sync_id,
                conflict.kind,
                conflict.local_payload,
                conflict.remote_payload,
                conflict.remote_device,
                conflict.copy_entity.map(EntityKind::prefix),
                conflict.copy_sync_id,
                now_ms
            ],
        )?;
        Ok(())
    }

    /// Whether this Mac evaluates a synced alert. Absent means yes: an alert made here behaves as
    /// it always did, and only alerts that arrived from another Mac start switched off.
    pub fn set_alert_local(
        &self,
        alert_id: &str,
        evaluate_here: bool,
        origin: &str,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO price_alert_local (alert_id, evaluate_here, origin) VALUES (?1, ?2, ?3)
             ON CONFLICT (alert_id) DO UPDATE SET evaluate_here = excluded.evaluate_here",
            params![alert_id, evaluate_here, origin],
        )?;
        Ok(())
    }

    pub fn alert_local(&self, alert_id: &str) -> rusqlite::Result<Option<(bool, String)>> {
        self.connection
            .query_row(
                "SELECT evaluate_here, origin FROM price_alert_local WHERE alert_id = ?1",
                [alert_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
    }

    pub fn remove_alert_local(&self, alert_id: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "DELETE FROM price_alert_local WHERE alert_id = ?1",
            [alert_id],
        )?;
        Ok(())
    }

    /// Alerts this Mac must not evaluate. A read failure means none: alerts keep working.
    pub fn muted_alerts(&self) -> HashSet<u64> {
        let Ok(mut statement) = self
            .connection
            .prepare("SELECT alert_id FROM price_alert_local WHERE evaluate_here = 0")
        else {
            return HashSet::new();
        };
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map(|rows| {
                rows.filter_map(Result::ok)
                    .filter_map(|id| id.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Writes a consistent copy of the whole database to `path`. The copy opens as an ordinary
    /// DegenView database, so a stashed dataset is always recoverable.
    pub fn backup_to(&self, path: &std::path::Path) -> rusqlite::Result<()> {
        let target = path.to_string_lossy().replace('\'', "''");
        self.connection
            .execute_batch(&format!("VACUUM INTO '{target}'"))
    }

    /// Forgets everything learned about the cloud: what is linked, queued, kept aside, and in
    /// conflict. The user's data is untouched. Used when the iCloud account changes.
    pub fn reset_sync_data(&self) -> rusqlite::Result<()> {
        self.connection.execute_batch(
            "DELETE FROM sync_entity;
             DELETE FROM sync_outbox;
             DELETE FROM sync_inbox;
             DELETE FROM sync_conflict;
             DELETE FROM sync_device;",
        )
    }

    /// Empties the synced collections that the cloud's copy replaces when the user chooses to use
    /// iCloud's data. Deliberately bypasses sync tracking: the cloud must never be told these
    /// rows were deleted. Alerts and scripts are never cleared; they merge instead.
    pub fn clear_replaceable_collections(&self) -> rusqlite::Result<()> {
        for table in [
            "watchlist",
            "saved_view",
            "chart_template",
            "drawing_template",
        ] {
            self.connection
                .execute(&format!("DELETE FROM {table}"), [])?;
        }
        Ok(())
    }

    /// The friendly name of a Mac, if the cloud's device list has told us.
    pub fn device_name(&self, device_id: &str) -> rusqlite::Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT name FROM sync_device WHERE device_id = ?1",
                [device_id],
                |row| row.get(0),
            )
            .optional()
    }

    /// Conflicts waiting for the user, newest last, with where each version went.
    pub fn open_conflicts(&self) -> rusqlite::Result<Vec<ConflictSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT id, entity_type, sync_id, kind, copy_sync_id, created_ms FROM sync_conflict
             WHERE resolved_ms IS NULL ORDER BY id",
        )?;
        statement
            .query_map([], |row| {
                Ok(ConflictSummary {
                    id: row.get(0)?,
                    entity_type: row.get(1)?,
                    sync_id: row.get(2)?,
                    kind: row.get(3)?,
                    copy_sync_id: row.get(4)?,
                    created_ms: row.get(5)?,
                })
            })?
            .collect()
    }

    /// The user has looked at a conflict (both versions were kept either way).
    pub fn dismiss_conflict(&self, id: i64, now_ms: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_conflict SET resolved_ms = ?2, resolution = 'dismissed'
             WHERE id = ?1 AND resolved_ms IS NULL",
            params![id, now_ms],
        )?;
        Ok(())
    }

    /// The user confirmed a large delete: let it go to iCloud and the other Macs.
    pub fn release_held(&self) -> rusqlite::Result<usize> {
        self.connection.execute(
            "UPDATE sync_outbox SET state = 'pending' WHERE state = 'held'",
            [],
        )
    }

    /// The user does not want a large delete to reach iCloud: it stays on the other Macs.
    pub fn drop_held(&self) -> rusqlite::Result<usize> {
        self.connection
            .execute("DELETE FROM sync_outbox WHERE state = 'held'", [])
    }

    pub fn conflicts(&self) -> rusqlite::Result<Vec<(String, String, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT entity_type, sync_id, kind FROM sync_conflict WHERE resolved_ms IS NULL ORDER BY id",
        )?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect()
    }

    pub(crate) fn in_savepoint<R>(
        &self,
        name: &str,
        f: impl FnOnce() -> rusqlite::Result<R>,
    ) -> rusqlite::Result<R> {
        self.connection
            .execute_batch(&format!("SAVEPOINT {name}"))?;
        let result = f();
        if result.is_err() {
            self.connection
                .execute_batch(&format!("ROLLBACK TO {name}"))?;
        }
        self.connection.execute_batch(&format!("RELEASE {name}"))?;
        result
    }
}

/// A remote record kept in `sync_inbox` instead of being applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxRecord {
    pub record_name: String,
    pub entity_type: String,
    pub schema_version: u32,
    /// `newer_schema`, `invalid`, `unknown_type`, `orphan` or `initial_pending`.
    pub reason: String,
    pub payload: Vec<u8>,
    pub system_fields: Option<Vec<u8>>,
    pub revision: u64,
    pub origin_device: String,
    pub modified_ms: i64,
}

/// A conflict the user may want to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSummary {
    pub id: i64,
    pub entity_type: String,
    pub sync_id: String,
    pub kind: String,
    /// The recoverable copy that holds the version that did not win.
    pub copy_sync_id: Option<String>,
    pub created_ms: i64,
}

/// What the cloud holds for an entity after a remote record was applied locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityLink {
    pub kind: EntityKind,
    pub sync_id: String,
    pub local_id: String,
    pub synced_hash: String,
    pub system_fields: Vec<u8>,
    pub base_payload: Option<Vec<u8>>,
    pub revision: i64,
    pub origin_device: String,
    pub now_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRecord {
    pub entity: EntityKind,
    pub sync_id: String,
    pub kind: String,
    pub local_payload: Option<Vec<u8>>,
    pub remote_payload: Option<Vec<u8>>,
    pub remote_device: Option<String>,
    pub copy_entity: Option<EntityKind>,
    pub copy_sync_id: Option<String>,
}

/// A row of a synced table as raw JSON. Ids come from the row's own `id` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawDoc(pub Value);

impl super::Document for RawDoc {
    fn document_id(&self) -> String {
        match self.0.get("id") {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        }
    }
}

/// Whether this version can read a row of `table`. A write must never produce, or silently
/// overwrite, a row the app's own model would refuse.
pub fn row_decodes(table: &str, payload: &str) -> bool {
    use crate::model;
    match table {
        "watchlist" => serde_json::from_str::<model::watchlist::Watchlist>(payload).is_ok(),
        "saved_view" => serde_json::from_str::<model::SavedView>(payload).is_ok(),
        "chart_template" => {
            serde_json::from_str::<model::chart_template::ChartAppearanceTemplate>(payload).is_ok()
        }
        "drawing_template" => {
            serde_json::from_str::<model::drawing_template::DrawingTemplate>(payload).is_ok()
        }
        "price_alert" => serde_json::from_str::<model::alert::PriceAlert>(payload).is_ok(),
        _ => false,
    }
}

/// The result of loading an outbox entry's live entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    Found(Projected),
    /// The row no longer exists; a delete or a reconcile will follow.
    Gone,
    /// Not held in SQLite (scripts are files); the caller must supply it.
    NotStored,
}

/// Sync bookkeeping for one entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityMeta {
    pub revision: i64,
    pub origin_device: String,
    pub synced_hash: Option<String>,
    pub system_fields: Option<Vec<u8>>,
    pub deleted: bool,
    pub deleted_ms: Option<i64>,
    /// The last payload both sides agreed on, kept for kinds that merge three ways.
    pub base_payload: Option<Vec<u8>>,
}

/// What a reconcile pass queued.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reconciled {
    pub upserts: usize,
    pub deletes: usize,
}

impl Store {
    /// Brings the outbox in line with everything stored, for changes the write hooks never saw:
    /// edits made by an older build, script files changed in Finder, and data that existed
    /// before sync was turned on. Safe to run repeatedly: an entity already queued, or already
    /// matching the cloud, is left alone.
    pub fn reconcile_stored(&self) -> rusqlite::Result<Reconciled> {
        let mut by_kind: HashMap<EntityKind, HashMap<String, Projected>> = HashMap::new();
        for kind in [
            EntityKind::Layout,
            EntityKind::LayoutDrawing,
            EntityKind::Watchlist,
            EntityKind::ChartTemplate,
            EntityKind::DrawingTemplate,
            EntityKind::Alert,
            EntityKind::Setting,
        ] {
            by_kind.insert(kind, HashMap::new());
        }
        for table in crate::store::DOCUMENT_TABLES {
            if !matches!(table_policy(table), TablePolicy::Synced(_)) {
                continue;
            }
            let mut statement = self
                .connection
                .prepare(&format!("SELECT id, payload FROM {table}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, payload) in rows {
                for entity in project_row(table, &id, &payload) {
                    by_kind
                        .entry(entity.kind)
                        .or_default()
                        .insert(entity.sync_id.clone(), entity);
                }
            }
        }
        let mut statement = self.connection.prepare("SELECT key, value FROM setting")?;
        let settings = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, value) in settings {
            if let Some(entity) = project_setting(&key, &value) {
                by_kind
                    .entry(EntityKind::Setting)
                    .or_default()
                    .insert(entity.sync_id.clone(), entity);
            }
        }
        let mut total = Reconciled::default();
        let mut kinds: Vec<_> = by_kind.into_iter().collect();
        kinds.sort_by_key(|(kind, _)| kind.prefix());
        for (kind, current) in kinds {
            let done = self.reconcile_kind(kind, &current)?;
            total.upserts += done.upserts;
            total.deletes += done.deletes;
        }
        Ok(total)
    }

    /// Reconciles one kind against the complete set of entities that currently exist for it.
    pub fn reconcile_kind(
        &self,
        kind: EntityKind,
        current: &HashMap<String, Projected>,
    ) -> rusqlite::Result<Reconciled> {
        let Some(state) = self.sync_state()? else {
            return Ok(Reconciled::default());
        };
        if self.disabled_categories().contains(&kind.category()) {
            return Ok(Reconciled::default());
        }
        let device = state.device_id;
        let mut done = Reconciled::default();

        let queued_upserts: HashSet<String> = self
            .outbox()?
            .into_iter()
            .filter(|row| row.kind == kind && row.op == OutboxOp::Upsert)
            .map(|row| row.sync_id)
            .collect();
        let mut ids: Vec<_> = current.keys().collect();
        ids.sort();
        for id in ids {
            let entity = &current[id];
            let mark = self.entity_mark(kind, id)?;
            let matches_cloud = mark.as_ref().is_some_and(|m| {
                !m.deleted && m.synced_hash.as_deref() == Some(entity.hash.as_str())
            });
            let already_pending =
                queued_upserts.contains(id) && mark.as_ref().is_some_and(|m| !m.deleted);
            if !matches_cloud && !already_pending {
                self.queue_upsert(entity, &device)?;
                done.upserts += 1;
            }
        }

        let mut statement = self.connection.prepare(
            "SELECT sync_id, local_id FROM sync_entity WHERE entity_type = ?1 AND deleted = 0",
        )?;
        let known = statement
            .query_map([kind.prefix()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Alerts arrive through the command queue and only reach the table when the alert owner
        // next runs. While commands are waiting, an absent alert may just not have landed yet.
        let alerts_in_flight = kind == EntityKind::Alert
            && self
                .connection
                .query_row("SELECT COUNT(*) FROM alert_command", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(1)
                > 0;
        let mut missing: Vec<_> = known
            .iter()
            .filter(|(sync_id, _)| !alerts_in_flight && !current.contains_key(sync_id))
            .collect();
        missing.sort();
        // A layout's drawings follow the layout; only parents are judged for a mass delete.
        let held =
            kind != EntityKind::LayoutDrawing && is_mass_delete_count(missing.len(), known.len());
        for (sync_id, local_id) in missing {
            let gone = Projected {
                kind,
                sync_id: sync_id.clone(),
                local_id: local_id.clone(),
                payload: Vec::new(),
                hash: String::new(),
            };
            let state = if held {
                OutboxState::Held
            } else {
                OutboxState::Pending
            };
            self.queue_delete(&gone, &device, state)?;
            done.deletes += 1;
        }
        Ok(done)
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
    fn reconcile_finds_data_that_predates_sync_and_is_then_quiet() {
        let store = store(false);
        put(&store, "watchlist", &[watchlist(1, "A"), watchlist(2, "B")]);
        store.set_setting("appTheme", &"dark").unwrap();
        assert!(outbox(&store).is_empty());
        let mut state = store.sync_state().unwrap().unwrap();
        state.enabled = true;
        store.save_sync_state(&state).unwrap();

        let first = store.reconcile_stored().unwrap();
        assert_eq!(
            first,
            Reconciled {
                upserts: 3,
                deletes: 0
            }
        );
        let seqs: Vec<_> = store.outbox().unwrap().iter().map(|r| r.seq).collect();
        let second = store.reconcile_stored().unwrap();
        assert_eq!(second, Reconciled::default());
        let again: Vec<_> = store.outbox().unwrap().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, again);
    }

    #[test]
    fn reconcile_skips_entities_that_match_the_cloud() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        let hash = project_row("watchlist", "1", &watchlist(1, "A").0.to_string())[0]
            .hash
            .clone();
        mark_synced(&store, EntityKind::Watchlist, "1", &hash);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        assert_eq!(store.reconcile_stored().unwrap(), Reconciled::default());
    }

    #[test]
    fn reconcile_catches_an_edit_an_older_build_made() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A")]);
        let hash = project_row("watchlist", "1", &watchlist(1, "A").0.to_string())[0]
            .hash
            .clone();
        mark_synced(&store, EntityKind::Watchlist, "1", &hash);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        // An older build writes without the hook.
        store
            .connection
            .execute(
                "UPDATE watchlist SET payload = ?1 WHERE id = '1'",
                [watchlist(1, "Edited").0.to_string()],
            )
            .unwrap();
        assert_eq!(store.reconcile_stored().unwrap().upserts, 1);
    }

    #[test]
    fn reconcile_queues_deletes_for_rows_an_older_build_removed() {
        let store = store(true);
        put(&store, "watchlist", &[watchlist(1, "A"), watchlist(2, "B")]);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        store
            .connection
            .execute("DELETE FROM watchlist WHERE id = '1'", [])
            .unwrap();
        let done = store.reconcile_stored().unwrap();
        assert_eq!(done.deletes, 1);
        assert!(is_tombstone(&store, EntityKind::Watchlist, "1"));
        let deletes: Vec<_> = store
            .outbox()
            .unwrap()
            .into_iter()
            .filter(|r| r.op == OutboxOp::Delete)
            .collect();
        assert_eq!(deletes.len(), 1);
    }

    #[test]
    fn reconcile_holds_a_mass_delete() {
        let store = store(true);
        let rows: Vec<_> = (1..=6).map(|id| watchlist(id, "x")).collect();
        put(&store, "watchlist", &rows);
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        store
            .connection
            .execute("DELETE FROM watchlist", [])
            .unwrap();
        store.reconcile_stored().unwrap();
        let held = store
            .outbox()
            .unwrap()
            .iter()
            .filter(|r| r.state == OutboxState::Held)
            .count();
        assert_eq!(held, 6);
    }

    #[test]
    fn reconcile_never_touches_local_only_data() {
        let store = store(true);
        put(&store, "tab", &[watchlist(1, "A")]);
        store.set_setting("alert.health", &json!({})).unwrap();
        assert_eq!(store.reconcile_stored().unwrap(), Reconciled::default());
    }

    fn script(id: &str, source: &str) -> Projected {
        crate::sync::project::project_script(&crate::sync::project::ScriptSnapshot {
            id: id.into(),
            name: format!("Script {id}"),
            source: source.into(),
            is_favorite: false,
        })
    }

    #[test]
    fn script_files_reconcile_like_rows() {
        let store = store(true);
        let current: HashMap<_, _> = [
            ("a".to_string(), script("a", "x")),
            ("b".to_string(), script("b", "y")),
        ]
        .into();
        assert_eq!(
            store
                .reconcile_kind(EntityKind::Script, &current)
                .unwrap()
                .upserts,
            2
        );
        assert_eq!(
            store.reconcile_kind(EntityKind::Script, &current).unwrap(),
            Reconciled::default()
        );

        for entity in current.values() {
            mark_synced(&store, EntityKind::Script, &entity.sync_id, &entity.hash);
        }
        store
            .connection
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        let edited: HashMap<_, _> = [
            ("a".to_string(), script("a", "x2")),
            ("b".to_string(), script("b", "y")),
        ]
        .into();
        assert_eq!(
            store.reconcile_kind(EntityKind::Script, &edited).unwrap(),
            Reconciled {
                upserts: 1,
                deletes: 0
            }
        );

        let only_b: HashMap<_, _> = [("b".to_string(), script("b", "y"))].into();
        let done = store.reconcile_kind(EntityKind::Script, &only_b).unwrap();
        assert_eq!(done.deletes, 1);
        assert!(is_tombstone(&store, EntityKind::Script, "a"));
    }

    #[test]
    fn a_script_that_returns_after_deletion_is_uploaded_again() {
        let store = store(true);
        let one: HashMap<_, _> = [("a".to_string(), script("a", "x"))].into();
        store.reconcile_kind(EntityKind::Script, &one).unwrap();
        store
            .reconcile_kind(EntityKind::Script, &HashMap::new())
            .unwrap();
        assert!(is_tombstone(&store, EntityKind::Script, "a"));
        store.reconcile_kind(EntityKind::Script, &one).unwrap();
        assert!(!is_tombstone(&store, EntityKind::Script, "a"));
        let rows = store.outbox().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].op, OutboxOp::Upsert);
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
