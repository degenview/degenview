//! The `sync_state` singleton: this Mac's sync identity, the account it is bound to, the
//! category switches and the engine checkpoint; and the resets that go with an account change.

use super::{new_device_id, new_id_tag};
use crate::store::Store;
use crate::sync::record::ids::IdSpace;
use crate::sync::record::policy::Category;
use rusqlite::{OptionalExtension, params};
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
    /// Reserved: never given a value, only cleared.
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
    /// Reserved: not written at runtime; the live error is in `SyncCore::status`.
    pub last_error: Option<String>,
    /// 31 random bits that namespace this database's counter ids when they cross into a synced
    /// record (see `sync::record::ids`). Kept by a copied database so a lineage keeps meaning the same ids.
    pub id_tag: u32,
}

impl Store {
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
                    last_send_ms, last_error, id_tag
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
                        id_tag: row.get(17)?,
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
        // Zero means unassigned: a fresh row, or one that predates the column.
        self.connection.execute(
            "UPDATE sync_state SET id_tag = ?1 WHERE id = 1 AND id_tag = 0",
            params![new_id_tag()],
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
    /// Records the hardware this database runs on without changing who it is.
    pub fn set_hardware_hash(&self, hardware_hash: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_state SET hardware_hash = ?1 WHERE id = 1",
            [hardware_hash],
        )?;
        Ok(())
    }

    pub fn reset_sync_device(&self, hardware_hash: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_state SET device_id = ?1, hardware_hash = ?2, engine_state = NULL,
                engine_state_account_hash = NULL, initial_sync_done = 0 WHERE id = 1",
            params![new_device_id(), hardware_hash],
        )?;
        Ok(())
    }

    /// This database's id space; see `sync::record::ids`.
    pub(crate) fn id_space(&self) -> IdSpace {
        let tag: Option<u32> = self
            .connection
            .query_row("SELECT id_tag FROM sync_state WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()
            .ok()
            .flatten();
        IdSpace::new(tag.unwrap_or(0))
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
}
