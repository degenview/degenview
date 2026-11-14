//! Reading and settling the outbox: what is due, the live entity an entry uploads, the cloud's
//! acknowledgements, retries, and the held large deletes the user confirms or drops.

use crate::store::Store;
use crate::sync::record::policy::EntityKind;
use crate::sync::record::project::{
    Projected, project_row_in, project_setting, project_transaction,
};
use rusqlite::{OptionalExtension, params};

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

/// The outbox note on an edit that is waiting to be merged with the cloud's newer version.
pub const OUTBOX_MERGING: &str = "merging with the cloud";

/// The result of loading an outbox entry's live entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Load {
    Found(Projected),
    /// The row no longer exists; a delete or a reconcile will follow.
    Gone,
    /// Not held in SQLite (scripts are files); the caller must supply it.
    NotStored,
}

impl Store {
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
        let ids = self.id_space();
        let pick = |table: &str, id: &str, payload: Option<String>| {
            payload
                .into_iter()
                .flat_map(|payload| project_row_in(ids, table, id, &payload))
                .find(|entity| entity.kind == kind && entity.sync_id == sync_id)
        };
        let found = match kind {
            EntityKind::Script => return Ok(Load::NotStored),
            EntityKind::PaperAccount => {
                let local = ids.to_local_string(sync_id);
                self.paper_parts(&local)?
                    .and_then(|parts| crate::sync::record::paper::project(ids, parts))
            }
            EntityKind::Portfolio => {
                let local = ids.to_local_string(sync_id);
                pick("portfolio", &local, by_id("portfolio", &local)?)
            }
            EntityKind::PortfolioTransaction => {
                let local = ids.to_local_string(sync_id);
                by_id("portfolio_transaction", &local)?
                    .and_then(|payload| project_transaction(ids, &local, &payload))
            }
            EntityKind::DrawingAsset => {
                pick("drawing_asset", sync_id, by_id("drawing_asset", sync_id)?)
            }
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
                    .flat_map(|(id, payload)| project_row_in(ids, "watchlist", &id, &payload))
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
            EntityKind::EditorTheme => {
                pick("editor_theme", sync_id, by_id("editor_theme", sync_id)?)
            }
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

    /// Makes an entry due immediately without counting a failed attempt.
    /// Edits that were refused because the cloud had a newer version wait for it to be merged
    /// in; once it has been, they go out at once.
    pub fn release_merged_edits(&self) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_outbox SET next_attempt_ms = NULL, last_error = NULL
             WHERE last_error = ?1",
            [OUTBOX_MERGING],
        )?;
        Ok(())
    }

    pub fn retry_now(&self, seq: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE sync_outbox SET next_attempt_ms = NULL WHERE seq = ?1",
            [seq],
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
}
