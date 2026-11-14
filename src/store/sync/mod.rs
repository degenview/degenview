//! Durable iCloud Sync metadata: the binding to one iCloud account, the engine checkpoint, and
//! (in later stages) the outbox and tombstones. These tables live beside the user's data so a
//! local write and the intent to upload it commit in one transaction. Older builds ignore them.
//!
//! `schema` creates the tables and `state` keeps the identity singleton. The write hooks (`track`)
//! and `reconcile` queue through the primitives here; `outbox`, `entity`, `inbox`, `conflicts`,
//! `alert` and `paper` each read and write their own tables, and `raw` serves remote applies.

use super::Store;

mod alert;
mod conflicts;
mod entity;
#[cfg(test)]
mod hook_tests;
mod inbox;
mod outbox;
mod paper;
mod raw;
mod reconcile;
mod schema;
mod state;
#[cfg(test)]
mod tests;
mod track;

use crate::sync::record::ids::IdSpace;
use crate::sync::record::policy::{EntityKind, SettingPolicy, setting_policy};
use crate::sync::record::project::Projected;
pub use conflicts::{ConflictRecord, ConflictSummary};
pub use entity::{EntityLink, EntityMeta};
pub use inbox::InboxRecord;
pub use outbox::{Load, OUTBOX_MERGING, OutboxOp, OutboxRow, OutboxState};
pub use paper::Elsewhere;
pub use raw::{RawDataError, row_decodes};
pub use reconcile::Reconciled;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use state::SyncState;
use std::collections::HashMap;
use std::collections::HashSet;

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

/// 31 random bits, never zero (zero marks an unassigned tag).
pub fn new_id_tag() -> u32 {
    let mut bytes = [0u8; 4];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |elapsed| elapsed.as_nanos() as u64);
        bytes = (nanos as u32).to_le_bytes();
    }
    (u32::from_le_bytes(bytes) & 0x7fff_ffff).max(1)
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

    #[cfg(test)]
    pub fn execute_raw(&self, sql: &str) -> rusqlite::Result<usize> {
        self.connection.execute(sql, [])
    }

    pub(crate) fn in_savepoint<R, E: From<rusqlite::Error>>(
        &self,
        name: &str,
        f: impl FnOnce() -> Result<R, E>,
    ) -> Result<R, E> {
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

fn is_mass_delete_count(gone: usize, before: usize) -> bool {
    gone >= MASS_DELETE_MIN && gone * 2 > before
}
