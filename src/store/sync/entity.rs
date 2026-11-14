//! `sync_entity`: what the cloud is known to hold for each entity (its hash, change tag,
//! revision and agreed base payload), linked after a remote apply and tombstoned after a delete.

use crate::store::Store;
use crate::sync::record::policy::EntityKind;
use rusqlite::{OptionalExtension, params};

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
    /// When iCloud last accepted or sent this record, as this Mac saw it.
    pub cloud_modified_ms: Option<i64>,
}

impl Store {
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
                    base_payload, cloud_modified_ms
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
                        cloud_modified_ms: row.get(7)?,
                    })
                },
            )
            .optional()
    }

    /// Records the payload both sides last agreed on, for kinds that merge three ways.
    pub fn set_base_payload(
        &self,
        kind: EntityKind,
        sync_id: &str,
        payload: &[u8],
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_entity SET base_payload = ?3 WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id, payload],
        )?;
        Ok(())
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
                local_hash = excluded.synced_hash,
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
                synced_hash = NULL, system_fields = NULL, base_payload = NULL, local_hash = NULL,
                deleted = 1, deleted_ms = excluded.deleted_ms",
            params![kind.prefix(), sync_id, local_id, now_ms],
        )?;
        Ok(())
    }

    /// Forgets what the cloud holds for an entity that must be uploaded again as new.
    pub fn forget_synced(&self, kind: EntityKind, sync_id: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_entity SET synced_hash = NULL, system_fields = NULL, base_payload = NULL,
                local_hash = NULL, deleted = 0, deleted_ms = NULL
             WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id],
        )?;
        Ok(())
    }
}
