//! The sync tables and their upgrades: created beside the user's data when the store opens,
//! and ignored by older builds.

use super::new_id_tag;
use crate::store::Store;
use rusqlite::params;

impl Store {
    pub(in crate::store) fn create_sync_schema(&self) -> rusqlite::Result<()> {
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
                -- Reserved: never assigned a value (only cleared). Kept so an older build's row reads.
                dataset_id TEXT,
                zone_name TEXT NOT NULL DEFAULT 'DegenViewZone',
                categories TEXT NOT NULL DEFAULT '{}',
                initial_sync_done INTEGER NOT NULL DEFAULT 0,
                engine_state BLOB,
                engine_state_account_hash TEXT,
                last_fetch_ms INTEGER,
                last_send_ms INTEGER,
                -- Reserved: runtime errors live in memory (`SyncCore`) and are shown from there; this
                -- is not written while the app runs, so it says nothing about the last failure.
                last_error TEXT,
                id_tag INTEGER NOT NULL DEFAULT 0
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
                local_hash TEXT,
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
            -- Reserved: nothing writes it yet (see Known limitations in docs/icloud-sync.md).
            CREATE TABLE IF NOT EXISTS sync_device (
                device_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                app_version TEXT,
                schema_max INTEGER,
                last_seen_ms INTEGER NOT NULL,
                is_self INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS paper_account_local (
                account_id TEXT PRIMARY KEY,
                owner_device TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS price_alert_local (
                alert_id TEXT PRIMARY KEY,
                evaluate_here INTEGER NOT NULL,
                origin TEXT NOT NULL DEFAULT 'local'
            );",
        )?;
        // Databases created by an earlier development build lack these columns.
        self.add_column("sync_state", "id_tag", "INTEGER NOT NULL DEFAULT 0")?;
        self.add_column("sync_entity", "local_hash", "TEXT")?;
        self.assign_missing_id_tag()?;
        Ok(())
    }

    /// A database that enabled sync before the tag existed has a sync identity with tag 0, and
    /// every such Mac would then give its small ids the same shared id. `ensure_sync_state` only
    /// runs when sync is switched on, so the tag is assigned here. Whatever was tracked under the
    /// zero tag is forgotten: the local rows stay and are tracked again under the new tag.
    fn assign_missing_id_tag(&self) -> rusqlite::Result<()> {
        let untagged = self
            .connection
            .prepare("SELECT 1 FROM sync_state WHERE id = 1 AND id_tag = 0")?
            .exists([])?;
        if !untagged {
            return Ok(());
        }
        let (low, high) = (
            crate::model::ids::GLOBAL_BASE as i64,
            (crate::model::ids::GLOBAL_BASE | (crate::model::ids::LEGACY_LIMIT - 1)) as i64,
        );
        for table in ["sync_entity", "sync_outbox", "sync_inbox"] {
            let key = if table == "sync_inbox" {
                "substr(record_name, instr(record_name, '.') + 1)"
            } else {
                "sync_id"
            };
            self.connection.execute(
                &format!(
                    "DELETE FROM {table}
                     WHERE entity_type IN ('portfolio', 'portfoliotx', 'paperaccount')
                       AND CAST({key} AS INTEGER) BETWEEN ?1 AND ?2"
                ),
                params![low, high],
            )?;
        }
        self.connection.execute(
            "UPDATE sync_state SET id_tag = ?1 WHERE id = 1 AND id_tag = 0",
            params![new_id_tag()],
        )?;
        Ok(())
    }

    fn add_column(&self, table: &str, column: &str, declaration: &str) -> rusqlite::Result<()> {
        let present = self
            .connection
            .prepare(&format!(
                "SELECT 1 FROM pragma_table_info('{table}') WHERE name = '{column}'"
            ))?
            .exists([])?;
        if !present {
            self.connection.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"),
                [],
            )?;
        }
        Ok(())
    }
}
