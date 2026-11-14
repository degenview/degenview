//! `sync_conflict`: conflicts the user may want to look at, with where each version went, until
//! they are resolved or dismissed.

use crate::store::Store;
use crate::sync::record::policy::EntityKind;
use rusqlite::params;

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

impl Store {
    /// Conflicts the user has not resolved or dismissed.
    pub fn open_conflict_count(&self) -> rusqlite::Result<usize> {
        self.connection.query_row(
            "SELECT COUNT(*) FROM sync_conflict WHERE resolved_ms IS NULL",
            [],
            |row| row.get::<_, i64>(0).map(|n| n as usize),
        )
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

    /// A conflict that no longer needs anyone: what it was waiting for happened (a held record was
    /// applied). It is closed like a dismissed one, so it leaves the page.
    pub fn resolve_conflicts(
        &self,
        entity: EntityKind,
        sync_id: &str,
        kind: &str,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_conflict SET resolved_ms = ?4, resolution = 'applied'
             WHERE entity_type = ?1 AND sync_id = ?2 AND kind = ?3 AND resolved_ms IS NULL",
            params![entity.prefix(), sync_id, kind, now_ms],
        )?;
        Ok(())
    }

    /// The user has looked at conflicts (both versions were kept either way): those of one kind,
    /// or every open one when `kind` is `None`.
    pub fn dismiss_conflicts(&self, kind: Option<&str>, now_ms: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_conflict SET resolved_ms = ?2, resolution = 'dismissed'
             WHERE resolved_ms IS NULL AND (?1 IS NULL OR kind = ?1)",
            params![kind, now_ms],
        )?;
        Ok(())
    }

    pub fn conflicts(&self) -> rusqlite::Result<Vec<(String, String, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT entity_type, sync_id, kind FROM sync_conflict WHERE resolved_ms IS NULL ORDER BY id",
        )?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect()
    }
}
