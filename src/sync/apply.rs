//! Applies fetched changes to the local stores, as one transaction.
//!
//! The rules this layer enforces:
//! * Every remote row is checked against this version's own model before it is written, and a
//!   table holding a row this version cannot read is never rewritten. What cannot be applied is
//!   kept in the inbox, not dropped.
//! * Before writing, the entity is recorded as "the cloud holds exactly this", so the write is
//!   not mistaken for a local edit and echoed back. Only what a merge added on top is uploaded.
//! * An edit beats a delete, and a Mac's Favorites list is never deleted by another Mac.
//! * Nothing is acknowledged to the core until the whole batch committed.

use super::core::{RemoteBatch, RemoteRecord};
use super::merge::{
    Outcome, Side, Stamp, conflict_copy_name, merge, next_drawing_id, renumber_drawing,
};
use super::policy::{EntityKind, Merge};
use super::project::{
    ScriptSnapshot, canonical_json, drawing_id, project_row, project_script, project_setting,
};
use super::record::{FAVORITES_ID, RecordName, sha256_hex};
use super::scripts::ScriptSink;
use crate::store::Store;
use crate::store::sync::{ConflictRecord, EntityLink, InboxRecord, OutboxOp, row_decodes};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Who is applying, and when. Passed in so applying is deterministic under test.
#[derive(Debug, Clone)]
pub struct ApplyCtx {
    pub device_id: String,
    pub device_name: String,
    pub now_ms: i64,
    /// `YYYY-MM-DD`, for naming conflict copies.
    pub date: String,
}

/// A record kept in the inbox, ready to be applied again.
pub fn remote_from_inbox(entry: &InboxRecord) -> Option<RemoteRecord> {
    let name = RecordName::parse(&entry.record_name)?;
    let value = serde_json::from_slice::<Value>(&entry.payload).ok()?;
    Some(RemoteRecord {
        record: super::record::SyncRecord {
            name,
            schema_version: entry.schema_version,
            revision: entry.revision,
            origin_device: entry.origin_device.clone(),
            modified_ms: entry.modified_ms,
            payload: entry.payload.clone(),
        },
        value,
        system_fields: entry.system_fields.clone().unwrap_or_default(),
    })
}

/// What changed, so live owners (the watchlist store, the template stores) can reload.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub tables: BTreeSet<String>,
    pub settings: BTreeSet<String>,
    pub applied: usize,
    pub kept_local: usize,
    pub inboxed: usize,
    pub conflicts: usize,
    /// Alert changes queued for whichever process owns alert evaluation.
    pub alert_commands: usize,
}

/// Chart templates below this id are the built-in presets, which live in code, not the table.
const FIRST_USER_TEMPLATE_ID: u64 = 1000;

pub fn apply_batch(
    store: &Store,
    scripts: &mut dyn ScriptSink,
    batch: &RemoteBatch,
    ctx: &ApplyCtx,
) -> rusqlite::Result<ApplyReport> {
    store.in_savepoint("apply_remote", || {
        let mut work = Work::new(store, scripts, ctx);
        let mut records: Vec<&RemoteRecord> = batch.records.iter().collect();
        // Drawings in id order, so a fresh Mac stacks them in the order they were drawn.
        records.sort_by_key(|r| {
            (
                r.record.name.kind.prefix(),
                r.record.name.layout_drawing_parts().unwrap_or_default(),
                r.record.name.id.clone(),
            )
        });
        for record in records {
            work.apply_record(record)?;
        }
        // Layouts go first: a layout's drawings follow it, and a kept layout keeps its drawings.
        let mut deletions: Vec<&RecordName> = batch.deletions.iter().collect();
        deletions.sort_by_key(|name| name.kind != EntityKind::Layout);
        for name in deletions {
            work.apply_deletion(name)?;
        }
        work.retry_orphans()?;
        work.retry_busy_scripts()?;
        work.flush()?;
        Ok(work.report)
    })
}

/// The local side of one entity.
struct Local {
    /// Index into the table's rows; unused for settings.
    index: usize,
    row: Value,
    projected_value: Value,
    hash: String,
    local_id: String,
}

struct Work<'a> {
    store: &'a Store,
    scripts: &'a mut dyn ScriptSink,
    ctx: &'a ApplyCtx,
    tables: HashMap<&'static str, Vec<Value>>,
    writable: HashMap<&'static str, bool>,
    dirty: BTreeSet<&'static str>,
    settings: BTreeMap<String, Option<Value>>,
    report: ApplyReport,
}

pub fn table_of(kind: EntityKind) -> Option<&'static str> {
    match kind {
        EntityKind::Watchlist => Some("watchlist"),
        EntityKind::ChartTemplate => Some("chart_template"),
        EntityKind::DrawingTemplate => Some("drawing_template"),
        EntityKind::Layout | EntityKind::LayoutDrawing => Some("saved_view"),
        EntityKind::Alert => Some("price_alert"),
        EntityKind::Setting | EntityKind::Script => None,
    }
}

impl<'a> Work<'a> {
    fn new(store: &'a Store, scripts: &'a mut dyn ScriptSink, ctx: &'a ApplyCtx) -> Self {
        Self {
            store,
            scripts,
            ctx,
            tables: HashMap::new(),
            writable: HashMap::new(),
            dirty: BTreeSet::new(),
            settings: BTreeMap::new(),
            report: ApplyReport::default(),
        }
    }

    // MARK: Table access

    /// Whether every existing row of the table is one this version can read and rewrite.
    fn table_writable(&mut self, table: &'static str) -> rusqlite::Result<bool> {
        if let Some(known) = self.writable.get(table) {
            return Ok(*known);
        }
        let ok = match self.store.raw_rows(table) {
            Ok(rows) => rows.iter().all(|row| row_decodes(table, &row.to_string())),
            Err(_) => false,
        };
        self.writable.insert(table, ok);
        Ok(ok)
    }

    fn rows(&mut self, table: &'static str) -> rusqlite::Result<&mut Vec<Value>> {
        if !self.tables.contains_key(table) {
            let rows = self.store.raw_rows(table)?;
            self.tables.insert(table, rows);
        }
        Ok(self.tables.get_mut(table).expect("just inserted"))
    }

    fn row_id(row: &Value) -> String {
        match row.get("id") {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        }
    }

    fn find_local(&mut self, kind: EntityKind, sync_id: &str) -> rusqlite::Result<Option<Local>> {
        if kind == EntityKind::Setting {
            let current = match self.settings.get(sync_id) {
                Some(pending) => pending.clone(),
                None => self.store.raw_setting(sync_id)?,
            };
            let Some(row) = current else { return Ok(None) };
            let Some(projected) = project_setting(sync_id, &row.to_string()) else {
                return Ok(None);
            };
            return Ok(Some(Local {
                index: 0,
                projected_value: serde_json::from_slice(&projected.payload).unwrap_or(Value::Null),
                hash: projected.hash,
                local_id: sync_id.to_string(),
                row,
            }));
        }
        let Some(table) = table_of(kind) else {
            return Ok(None);
        };
        let rows = self.rows(table)?;
        for (index, row) in rows.iter().enumerate() {
            let id = Self::row_id(row);
            // A layout's id is its row id, so only that row needs projecting.
            if kind == EntityKind::Layout && id != sync_id {
                continue;
            }
            for entity in project_row(table, &id, &row.to_string()) {
                if entity.kind == kind && entity.sync_id == sync_id {
                    return Ok(Some(Local {
                        index,
                        projected_value: serde_json::from_slice(&entity.payload)
                            .unwrap_or(Value::Null),
                        hash: entity.hash,
                        local_id: id,
                        row: row.clone(),
                    }));
                }
            }
        }
        Ok(None)
    }

    fn put_row(
        &mut self,
        table: &'static str,
        index: Option<usize>,
        row: Value,
    ) -> rusqlite::Result<()> {
        let rows = self.rows(table)?;
        match index {
            Some(index) => rows[index] = row,
            None => rows.push(row),
        }
        self.dirty.insert(table);
        self.report.tables.insert(table.to_string());
        Ok(())
    }

    fn remove_row(&mut self, table: &'static str, index: usize) -> rusqlite::Result<()> {
        self.rows(table)?.remove(index);
        self.dirty.insert(table);
        self.report.tables.insert(table.to_string());
        Ok(())
    }

    fn flush(&mut self) -> rusqlite::Result<()> {
        let dirty: Vec<_> = self.dirty.iter().copied().collect();
        for table in dirty {
            let rows = self.tables.get(table).cloned().unwrap_or_default();
            self.store.write_raw_rows(table, &rows)?;
        }
        for (key, value) in std::mem::take(&mut self.settings) {
            self.store.write_raw_setting(&key, value.as_ref())?;
            self.report.settings.insert(key);
        }
        Ok(())
    }

    // MARK: Inbox

    fn keep(&mut self, record: &RemoteRecord, reason: &str) -> rusqlite::Result<()> {
        self.store.put_inbox(
            &InboxRecord {
                record_name: record.record.name.render(),
                entity_type: record.record.name.kind.prefix().to_string(),
                schema_version: record.record.schema_version,
                reason: reason.to_string(),
                payload: record.record.payload.clone(),
                system_fields: Some(record.system_fields.clone()),
                revision: record.record.revision,
                origin_device: record.record.origin_device.clone(),
                modified_ms: record.record.modified_ms,
            },
            self.ctx.now_ms,
        )?;
        self.report.inboxed += 1;
        Ok(())
    }

    // MARK: Records

    fn apply_record(&mut self, record: &RemoteRecord) -> rusqlite::Result<()> {
        match record.record.name.kind {
            EntityKind::Watchlist
            | EntityKind::ChartTemplate
            | EntityKind::DrawingTemplate
            | EntityKind::Setting => self.apply_simple(record),
            EntityKind::Layout => self.apply_layout(record),
            EntityKind::LayoutDrawing => self.apply_drawing(record),
            EntityKind::Alert => self.apply_alert(record),
            EntityKind::Script => self.apply_script(record),
        }
    }

    fn apply_simple(&mut self, record: &RemoteRecord) -> rusqlite::Result<()> {
        let RecordName { kind, id } = &record.record.name;
        let (kind, id) = (*kind, id.as_str());
        if let Some(table) = table_of(kind)
            && !self.table_writable(table)?
        {
            return self.keep(record, "local_unreadable");
        }
        if kind == EntityKind::ChartTemplate
            && id.parse::<u64>().is_ok_and(|n| n < FIRST_USER_TEMPLATE_ID)
        {
            return self.keep(record, "invalid");
        }

        let remote_payload = canonical_json(&record.value);
        let remote_hash = sha256_hex(&remote_payload);
        let meta = self.store.entity_meta(kind, id)?;
        let local = self.find_local(kind, id)?;

        let new_value = match &local {
            None => record.value.clone(),
            Some(local) if local.hash == remote_hash => record.value.clone(),
            Some(local) => {
                let unchanged = meta.as_ref().is_some_and(|m| {
                    !m.deleted && m.synced_hash.as_deref() == Some(local.hash.as_str())
                });
                if unchanged {
                    // We already hold something newer than this record: ignore the stale one.
                    if meta
                        .as_ref()
                        .is_some_and(|m| m.revision as u64 > record.record.revision)
                    {
                        self.report.kept_local += 1;
                        return Ok(());
                    }
                    record.value.clone()
                } else {
                    let base: Option<Value> = (kind.merge() == Merge::ThreeWay)
                        .then(|| meta.as_ref().and_then(|m| m.base_payload.as_deref()))
                        .flatten()
                        .and_then(|bytes| serde_json::from_slice(bytes).ok());
                    let modified = self
                        .store
                        .outbox_entry(kind, id)?
                        .map_or(self.ctx.now_ms, |row| row.queued_ms);
                    let local_stamp = Stamp::new(
                        meta.as_ref().map_or(1, |m| m.revision.max(1)) as u64,
                        modified,
                        self.ctx.device_id.clone(),
                    );
                    let remote_stamp = Stamp::new(
                        record.record.revision,
                        record.record.modified_ms,
                        record.record.origin_device.clone(),
                    );
                    match merge(
                        kind,
                        base.as_ref(),
                        Side {
                            value: &local.projected_value,
                            stamp: local_stamp,
                        },
                        Side {
                            value: &record.value,
                            stamp: remote_stamp,
                        },
                    ) {
                        Outcome::Resolved(value) | Outcome::Conflict { winner: value, .. } => value,
                    }
                }
            }
        };

        // Compose the row this version stores, and refuse anything it could not read back.
        let local_id;
        if kind == EntityKind::Setting {
            let value = compose_setting(id, &new_value, local.as_ref().map(|l| &l.row));
            local_id = id.to_string();
            self.settings.insert(id.to_string(), Some(value));
            self.report.applied += 1;
        } else {
            let table = table_of(kind).expect("table kinds have a table");
            let existing = local.as_ref().map(|l| &l.row);
            let Some(row) = compose_row(kind, id, &new_value, existing) else {
                return self.keep(record, "invalid");
            };
            if !row_decodes(table, &row.to_string()) {
                return self.keep(record, "invalid");
            }
            local_id = Self::row_id(&row);
            self.put_row(table, local.as_ref().map(|l| l.index), row)?;
            self.report.applied += 1;
        }

        self.store.link_entity(&EntityLink {
            kind,
            sync_id: id.to_string(),
            local_id,
            synced_hash: remote_hash,
            system_fields: record.system_fields.clone(),
            base_payload: (kind.merge() == Merge::ThreeWay).then_some(remote_payload),
            revision: record.record.revision as i64,
            origin_device: record.record.origin_device.clone(),
            now_ms: self.ctx.now_ms,
        })?;
        Ok(())
    }

    // MARK: Scripts

    fn script_fields(value: &Value) -> Option<(String, String, bool)> {
        Some((
            value.get("name")?.as_str()?.to_string(),
            value.get("source")?.as_str()?.to_string(),
            value
                .get("is_favorite")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ))
    }

    fn apply_script(&mut self, record: &RemoteRecord) -> rusqlite::Result<()> {
        let id = record.record.name.id.clone();
        let Some((name, source, favorite)) = Self::script_fields(&record.value) else {
            return self.keep(record, "invalid");
        };
        let remote_hash = sha256_hex(&canonical_json(&record.value));
        let meta = self.store.entity_meta(EntityKind::Script, &id)?;
        let Ok(local) = self.scripts.get(&id) else {
            return self.keep(record, "busy");
        };
        let local_projected = local.as_ref().map(project_script);

        // What this Mac's script should become, and whether anything must be written at all.
        let mut intended = record.value.clone();
        let mut write: Option<(String, String, bool)> =
            Some((name.clone(), source.clone(), favorite));
        let mut copy: Option<(ScriptSnapshot, bool)> = None;

        match (&local, &local_projected) {
            (Some(_), Some(projected)) if projected.hash == remote_hash => write = None,
            (Some(_), Some(projected)) => {
                if self.scripts.has_unsaved_work(&id) {
                    // Never replace text the user is typing; try again once it is saved or closed.
                    return self.keep(record, "busy");
                }
                let unchanged = meta.as_ref().is_some_and(|m| {
                    !m.deleted && m.synced_hash.as_deref() == Some(projected.hash.as_str())
                });
                if unchanged {
                    if meta
                        .as_ref()
                        .is_some_and(|m| m.revision as u64 > record.record.revision)
                    {
                        self.report.kept_local += 1;
                        return Ok(());
                    }
                } else {
                    let local_value: Value =
                        serde_json::from_slice(&projected.payload).unwrap_or(Value::Null);
                    let modified = self
                        .store
                        .outbox_entry(EntityKind::Script, &id)?
                        .map_or(self.ctx.now_ms, |row| row.queued_ms);
                    let outcome = merge(
                        EntityKind::Script,
                        None,
                        Side {
                            value: &local_value,
                            stamp: Stamp::new(
                                meta.as_ref().map_or(1, |m| m.revision.max(1)) as u64,
                                modified,
                                self.ctx.device_id.clone(),
                            ),
                        },
                        Side {
                            value: &record.value,
                            stamp: Stamp::new(
                                record.record.revision,
                                record.record.modified_ms,
                                record.record.origin_device.clone(),
                            ),
                        },
                    );
                    match outcome {
                        Outcome::Resolved(value) => {
                            let Some(fields) = Self::script_fields(&value) else {
                                return self.keep(record, "invalid");
                            };
                            intended = value;
                            write = Some(fields);
                        }
                        Outcome::Conflict { winner, loser } => {
                            let local_wins = winner == local_value;
                            self.report.conflicts += 1;
                            let Some(win) = Self::script_fields(&winner) else {
                                return self.keep(record, "invalid");
                            };
                            let Some(lose) = Self::script_fields(&loser) else {
                                return self.keep(record, "invalid");
                            };
                            copy = Some((
                                ScriptSnapshot {
                                    id: String::new(),
                                    name: lose.0,
                                    source: lose.1,
                                    is_favorite: lose.2,
                                },
                                local_wins,
                            ));
                            if local_wins {
                                write = None;
                            } else {
                                intended = winner;
                                write = Some(win);
                            }
                        }
                    }
                }
            }
            _ => {
                // New to this Mac. If it was deleted here and not yet uploaded, the edit made
                // elsewhere wins over that delete.
                self.store.cancel_outbox(EntityKind::Script, &id)?;
                if let Ok(Some(taken)) = self.scripts.find_by_name(&name)
                    && taken.id != id
                    && taken.pristine_demo
                {
                    // An untouched bundled example gives way to the user's own script of that name.
                    let _ = self.scripts.delete(&taken.id);
                }
            }
        }

        if let Some((name, source, favorite)) = write {
            let result = if local.is_some() {
                self.scripts.update(&id, &name, &source, favorite)
            } else {
                self.scripts.adopt(&id, &name, &source, favorite)
            };
            if result.is_err() {
                return self.keep(record, "busy");
            }
            self.report.applied += 1;
        }
        if let Some((loser, from_local)) = copy {
            self.save_script_copy(record, &id, loser, from_local)?;
        }

        // A name this Mac had to change (another script already has it) is not an edit to upload.
        let mut synced_hash = remote_hash;
        if intended == record.value
            && let Ok(Some(after)) = self.scripts.get(&id)
        {
            let actual = project_script(&after).hash;
            if actual != synced_hash {
                synced_hash = actual;
            }
        }
        self.store.link_entity(&EntityLink {
            kind: EntityKind::Script,
            sync_id: id.clone(),
            local_id: id,
            synced_hash,
            system_fields: record.system_fields.clone(),
            base_payload: None,
            revision: record.record.revision as i64,
            origin_device: record.record.origin_device.clone(),
            now_ms: self.ctx.now_ms,
        })?;
        Ok(())
    }

    /// Saves the losing version of a script as a new script, so neither edit is ever lost.
    fn save_script_copy(
        &mut self,
        record: &RemoteRecord,
        original: &str,
        loser: ScriptSnapshot,
        from_local: bool,
    ) -> rusqlite::Result<()> {
        // `from_local` here means the local script won, so the loser is the cloud's version.
        let loser_is_local = !from_local;
        let device = if loser_is_local {
            self.ctx.device_name.clone()
        } else {
            self.store
                .device_name(&record.record.origin_device)?
                .unwrap_or_default()
        };
        let copy_id = self.scripts.fresh_id();
        let copy_name = conflict_copy_name(&loser.name, &device, &self.ctx.date);
        if self
            .scripts
            .adopt(&copy_id, &copy_name, &loser.source, loser.is_favorite)
            .is_err()
        {
            return Ok(());
        }
        self.store.record_conflict(
            &ConflictRecord {
                entity: EntityKind::Script,
                sync_id: original.to_string(),
                kind: "script".into(),
                local_payload: loser_is_local.then(|| loser.source.clone().into_bytes()),
                remote_payload: Some(record.record.payload.clone()),
                remote_device: Some(record.record.origin_device.clone()),
                copy_entity: Some(EntityKind::Script),
                copy_sync_id: Some(copy_id),
            },
            self.ctx.now_ms,
        )
    }

    fn delete_script(&mut self, id: &str) -> rusqlite::Result<()> {
        let kind = EntityKind::Script;
        let meta = self.store.entity_meta(kind, id)?;
        let Ok(Some(local)) = self.scripts.get(id) else {
            self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
            return Ok(());
        };
        let hash = project_script(&local).hash;
        let edited_since = meta
            .as_ref()
            .is_none_or(|m| m.synced_hash.as_deref() != Some(hash.as_str()))
            || self
                .store
                .outbox_entry(kind, id)?
                .is_some_and(|row| row.op == OutboxOp::Upsert)
            || self.scripts.has_unsaved_work(id);
        if edited_since {
            // An edit beats a delete: the script comes back from this Mac.
            self.store.forget_synced(kind, id)?;
            self.store.queue_entity_upsert(kind, id, id)?;
            self.report.kept_local += 1;
            return Ok(());
        }
        if self.scripts.delete(id).is_err() {
            self.report.kept_local += 1;
            return Ok(());
        }
        self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
        self.store.cancel_outbox(kind, id)?;
        self.report.applied += 1;
        Ok(())
    }

    /// Scripts that were being edited when their update arrived are applied once they are saved.
    fn retry_busy_scripts(&mut self) -> rusqlite::Result<()> {
        let waiting: Vec<InboxRecord> = self
            .store
            .inbox()?
            .into_iter()
            .filter(|r| r.reason == "busy" && r.entity_type == EntityKind::Script.prefix())
            .collect();
        for entry in waiting {
            let Some(name) = RecordName::parse(&entry.record_name) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<Value>(&entry.payload) else {
                continue;
            };
            self.store.remove_inbox(&entry.record_name)?;
            let record = RemoteRecord {
                record: super::record::SyncRecord {
                    name,
                    schema_version: entry.schema_version,
                    revision: entry.revision,
                    origin_device: entry.origin_device.clone(),
                    modified_ms: entry.modified_ms,
                    payload: entry.payload.clone(),
                },
                value,
                system_fields: entry.system_fields.clone().unwrap_or_default(),
            };
            self.apply_script(&record)?;
        }
        Ok(())
    }

    // MARK: Alerts

    /// Alerts are owned by whichever process runs evaluation (the app or the background agent),
    /// which holds the live list in memory and rewrites it. A table write from here would be
    /// overwritten, so remote changes go through the same command queue as the UI.
    fn apply_alert(&mut self, record: &RemoteRecord) -> rusqlite::Result<()> {
        let id = record.record.name.id.clone();
        let remote_payload = canonical_json(&record.value);
        let remote_hash = sha256_hex(&remote_payload);
        let meta = self.store.entity_meta(EntityKind::Alert, &id)?;
        let local = self.find_local(EntityKind::Alert, &id)?;

        let target = match &local {
            None => record.value.clone(),
            Some(local) if local.hash == remote_hash => {
                // Already identical; just remember what the cloud holds.
                return self.link_alert(record, &id, remote_hash);
            }
            Some(local) => {
                let unchanged = meta.as_ref().is_some_and(|m| {
                    !m.deleted && m.synced_hash.as_deref() == Some(local.hash.as_str())
                });
                if unchanged {
                    if meta
                        .as_ref()
                        .is_some_and(|m| m.revision as u64 > record.record.revision)
                    {
                        self.report.kept_local += 1;
                        return Ok(());
                    }
                    record.value.clone()
                } else {
                    let modified = self
                        .store
                        .outbox_entry(EntityKind::Alert, &id)?
                        .map_or(self.ctx.now_ms, |row| row.queued_ms);
                    let (Outcome::Resolved(value) | Outcome::Conflict { winner: value, .. }) =
                        merge(
                            EntityKind::Alert,
                            None,
                            Side {
                                value: &local.projected_value,
                                stamp: Stamp::new(
                                    meta.as_ref().map_or(1, |m| m.revision.max(1)) as u64,
                                    modified,
                                    self.ctx.device_id.clone(),
                                ),
                            },
                            Side {
                                value: &record.value,
                                stamp: Stamp::new(
                                    record.record.revision,
                                    record.record.modified_ms,
                                    record.record.origin_device.clone(),
                                ),
                            },
                        );
                    value
                }
            }
        };

        let Some(alert) = compose_alert(&target, local.as_ref().map(|l| &l.row), self.ctx.now_ms)
        else {
            return self.keep(record, "invalid");
        };
        let Ok(alert) = serde_json::from_value::<crate::model::alert::PriceAlert>(alert) else {
            return self.keep(record, "invalid");
        };
        let command = crate::model::alert::AlertRuntimeCommand::new(
            crate::model::alert::AlertRuntimeCommandPayload::Save {
                alert: Box::new(alert),
                baseline: None,
            },
            self.ctx.now_ms,
        );
        self.store.enqueue_alert_command(&command)?;
        if local.is_none() {
            // An alert that arrives from another Mac is defined here but switched off: it would
            // otherwise notify from every Mac at once.
            self.store.set_alert_local(&id, false, "remote")?;
        }
        self.report.alert_commands += 1;
        self.report.applied += 1;
        self.link_alert(record, &id, remote_hash)
    }

    fn link_alert(
        &mut self,
        record: &RemoteRecord,
        id: &str,
        remote_hash: String,
    ) -> rusqlite::Result<()> {
        self.store.link_entity(&EntityLink {
            kind: EntityKind::Alert,
            sync_id: id.to_string(),
            local_id: id.to_string(),
            synced_hash: remote_hash,
            system_fields: record.system_fields.clone(),
            base_payload: None,
            revision: record.record.revision as i64,
            origin_device: record.record.origin_device.clone(),
            now_ms: self.ctx.now_ms,
        })
    }

    fn delete_alert(&mut self, id: &str) -> rusqlite::Result<()> {
        let kind = EntityKind::Alert;
        let meta = self.store.entity_meta(kind, id)?;
        let Some(local) = self.find_local(kind, id)? else {
            self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
            self.store.remove_alert_local(id)?;
            return Ok(());
        };
        let edited_since = meta
            .as_ref()
            .is_none_or(|m| m.synced_hash.as_deref() != Some(local.hash.as_str()))
            || self
                .store
                .outbox_entry(kind, id)?
                .is_some_and(|row| row.op == OutboxOp::Upsert);
        if edited_since {
            self.store.forget_synced(kind, id)?;
            self.store.queue_entity_upsert(kind, id, id)?;
            self.report.kept_local += 1;
            return Ok(());
        }
        self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
        self.store.cancel_outbox(kind, id)?;
        self.store.remove_alert_local(id)?;
        if let Ok(alert_id) = id.parse() {
            let command = crate::model::alert::AlertRuntimeCommand::new(
                crate::model::alert::AlertRuntimeCommandPayload::Delete(alert_id),
                self.ctx.now_ms,
            );
            self.store.enqueue_alert_command(&command)?;
            self.report.alert_commands += 1;
        }
        self.report.applied += 1;
        Ok(())
    }

    // MARK: Layouts

    fn apply_layout(&mut self, record: &RemoteRecord) -> rusqlite::Result<()> {
        let id = record.record.name.id.clone();
        if !self.table_writable("saved_view")? {
            return self.keep(record, "local_unreadable");
        }
        let remote_payload = canonical_json(&record.value);
        let remote_hash = sha256_hex(&remote_payload);
        let meta = self.store.entity_meta(EntityKind::Layout, &id)?;
        let local = self.find_local(EntityKind::Layout, &id)?;

        let outcome = match &local {
            None => Outcome::Resolved(record.value.clone()),
            Some(local) if local.hash == remote_hash => Outcome::Resolved(record.value.clone()),
            Some(local) => {
                let unchanged = meta.as_ref().is_some_and(|m| {
                    !m.deleted && m.synced_hash.as_deref() == Some(local.hash.as_str())
                });
                if unchanged {
                    if meta
                        .as_ref()
                        .is_some_and(|m| m.revision as u64 > record.record.revision)
                    {
                        self.report.kept_local += 1;
                        return Ok(());
                    }
                    Outcome::Resolved(record.value.clone())
                } else {
                    let modified = self
                        .store
                        .outbox_entry(EntityKind::Layout, &id)?
                        .map_or(self.ctx.now_ms, |row| row.queued_ms);
                    merge(
                        EntityKind::Layout,
                        None,
                        Side {
                            value: &local.projected_value,
                            stamp: Stamp::new(
                                meta.as_ref().map_or(1, |m| m.revision.max(1)) as u64,
                                modified,
                                self.ctx.device_id.clone(),
                            ),
                        },
                        Side {
                            value: &record.value,
                            stamp: Stamp::new(
                                record.record.revision,
                                record.record.modified_ms,
                                record.record.origin_device.clone(),
                            ),
                        },
                    )
                }
            }
        };

        let existing = local.as_ref().map(|l| &l.row);
        let mut copy: Option<(Value, bool)> = None;
        let new_row = match outcome {
            Outcome::Resolved(value) => compose_layout(&value, existing),
            Outcome::Conflict { winner, loser } => {
                let local_wins = local.as_ref().is_some_and(|l| l.projected_value == winner);
                self.report.conflicts += 1;
                if local_wins {
                    // Keep this Mac's version; the cloud's becomes a copy.
                    copy = compose_layout(&loser, None).map(|row| (row, false));
                    existing.cloned()
                } else {
                    // The cloud's version wins; this Mac's old version becomes a copy.
                    copy = existing.cloned().map(|row| (row, true));
                    compose_layout(&winner, existing)
                }
            }
        };
        let Some(new_row) = new_row else {
            return self.keep(record, "invalid");
        };
        if !row_decodes("saved_view", &new_row.to_string()) {
            return self.keep(record, "invalid");
        }
        let index = local.as_ref().map(|l| l.index);
        self.put_row("saved_view", index, new_row)?;

        if let Some((row, from_local)) = copy {
            self.save_layout_copy(record, row, from_local)?;
        }
        self.report.applied += 1;
        self.store.link_entity(&EntityLink {
            kind: EntityKind::Layout,
            sync_id: id.clone(),
            local_id: id,
            synced_hash: remote_hash,
            system_fields: record.system_fields.clone(),
            base_payload: None,
            revision: record.record.revision as i64,
            origin_device: record.record.origin_device.clone(),
            now_ms: self.ctx.now_ms,
        })?;
        Ok(())
    }

    /// Saves the losing version of a layout as a new, separately named layout.
    fn save_layout_copy(
        &mut self,
        record: &RemoteRecord,
        mut row: Value,
        from_local: bool,
    ) -> rusqlite::Result<()> {
        let Some(object) = row.as_object_mut() else {
            return Ok(());
        };
        let new_id = crate::model::watchlist::new_id();
        let device = if from_local {
            self.ctx.device_name.clone()
        } else {
            self.store
                .device_name(&record.record.origin_device)?
                .unwrap_or_default()
        };
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Layout");
        let copy_name = conflict_copy_name(name, &device, &self.ctx.date);
        object.insert("id".into(), new_id.into());
        object.insert("name".into(), copy_name.into());
        object.remove("last_opened_at_ms");
        object.remove("autosave");
        if !row_decodes("saved_view", &row.to_string()) {
            return Ok(());
        }
        let local_payload = from_local.then(|| row.to_string().into_bytes());
        self.put_row("saved_view", None, row)?;
        self.store.record_conflict(
            &ConflictRecord {
                entity: EntityKind::Layout,
                sync_id: record.record.name.id.clone(),
                kind: "layout".into(),
                local_payload,
                remote_payload: Some(record.record.payload.clone()),
                remote_device: Some(record.record.origin_device.clone()),
                copy_entity: Some(EntityKind::Layout),
                copy_sync_id: Some(new_id.to_string()),
            },
            self.ctx.now_ms,
        )
    }

    fn delete_layout(&mut self, id: &str) -> rusqlite::Result<()> {
        let kind = EntityKind::Layout;
        if !self.table_writable("saved_view")? {
            self.report.inboxed += 1;
            return Ok(());
        }
        let meta = self.store.entity_meta(kind, id)?;
        let Some(local) = self.find_local(kind, id)? else {
            self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
            return Ok(());
        };
        let drawings: Vec<String> = project_row("saved_view", id, &local.row.to_string())
            .into_iter()
            .filter(|e| e.kind == EntityKind::LayoutDrawing)
            .map(|e| e.sync_id)
            .collect();
        let edited_since = meta
            .as_ref()
            .is_none_or(|m| m.synced_hash.as_deref() != Some(local.hash.as_str()))
            || self
                .store
                .outbox_entry(kind, id)?
                .is_some_and(|row| row.op == OutboxOp::Upsert);
        if edited_since {
            // An edit beats a delete: the layout and its drawings come back from this Mac.
            self.store.forget_synced(kind, id)?;
            self.store.queue_entity_upsert(kind, id, id)?;
            for drawing in &drawings {
                self.store
                    .forget_synced(EntityKind::LayoutDrawing, drawing)?;
                self.store
                    .queue_entity_upsert(EntityKind::LayoutDrawing, drawing, drawing)?;
            }
            self.report.kept_local += 1;
            return Ok(());
        }
        for drawing in &drawings {
            self.store.tombstone_entity(
                EntityKind::LayoutDrawing,
                drawing,
                drawing,
                self.ctx.now_ms,
            )?;
            self.store
                .cancel_outbox(EntityKind::LayoutDrawing, drawing)?;
        }
        self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
        self.store.cancel_outbox(kind, id)?;
        self.remove_row("saved_view", local.index)?;
        self.report.applied += 1;
        Ok(())
    }

    // MARK: Drawings

    /// Where a drawing sits: the layout row, the chart within it, and its place in the chart.
    fn locate_drawing(
        &mut self,
        layout: u64,
        chart: u64,
        drawing: u64,
    ) -> rusqlite::Result<Option<DrawingSpot>> {
        let id = layout.to_string();
        let rows = self.rows("saved_view")?;
        let Some(row_index) = rows.iter().position(|row| Self::row_id(row) == id) else {
            return Ok(None);
        };
        let Some(chart_index) = rows[row_index]
            .get("charts")
            .and_then(Value::as_array)
            .and_then(|charts| {
                charts
                    .iter()
                    .position(|c| c.get("id").and_then(Value::as_u64) == Some(chart))
            })
        else {
            return Ok(None);
        };
        let position = rows[row_index]["charts"][chart_index]
            .get("drawings")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().position(|d| drawing_id(d) == Some(drawing)));
        Ok(Some(DrawingSpot {
            row_index,
            chart_index,
            position,
        }))
    }

    fn apply_drawing(&mut self, record: &RemoteRecord) -> rusqlite::Result<()> {
        let name = &record.record.name;
        let Some((layout, chart, drawing_id_number)) = name.layout_drawing_parts() else {
            return self.keep(record, "invalid");
        };
        if !self.table_writable("saved_view")? {
            return self.keep(record, "local_unreadable");
        }
        let Some(remote_drawing) = record.value.get("drawing").cloned() else {
            return self.keep(record, "invalid");
        };
        if drawing_id(&remote_drawing) != Some(drawing_id_number) {
            return self.keep(record, "invalid");
        }
        let Some(spot) = self.locate_drawing(layout, chart, drawing_id_number)? else {
            // The layout or chart has not arrived yet; hold the drawing until it does.
            return self.keep(record, "orphan");
        };

        let sync_id = name.id.clone();
        let remote_payload = canonical_json(&record.value);
        let remote_hash = sha256_hex(&remote_payload);
        let meta = self
            .store
            .entity_meta(EntityKind::LayoutDrawing, &sync_id)?;

        let mut insert = Some(remote_drawing);
        // Where an updated drawing replaces the local one; `None` appends a new one on top.
        let mut replace_at = spot.position;
        match spot.position {
            None => {
                // Absent locally. If this Mac deleted it, the remote edit wins and the pending
                // delete is dropped.
                self.store
                    .cancel_outbox(EntityKind::LayoutDrawing, &sync_id)?;
            }
            Some(position) => {
                let local_value = self.rows("saved_view")?[spot.row_index]["charts"]
                    [spot.chart_index]["drawings"][position]
                    .clone();
                let projected = serde_json::json!({ "drawing": local_value });
                let local_hash = sha256_hex(&canonical_json(&projected));
                if local_hash == remote_hash {
                    insert = None;
                    replace_at = None;
                } else {
                    let unchanged = meta.as_ref().is_some_and(|m| {
                        !m.deleted && m.synced_hash.as_deref() == Some(local_hash.as_str())
                    });
                    let never_synced = meta
                        .as_ref()
                        .is_none_or(|m| m.synced_hash.is_none() && !m.deleted);
                    if unchanged {
                        if meta
                            .as_ref()
                            .is_some_and(|m| m.revision as u64 > record.record.revision)
                        {
                            self.report.kept_local += 1;
                            return Ok(());
                        }
                    } else if never_synced {
                        // Two Macs gave different drawings the same per-chart id. Keep both: the
                        // local one moves to a fresh id and uploads as a new drawing.
                        let row = self.rows("saved_view")?[spot.row_index].clone();
                        let fresh = next_drawing_id(&row, chart, &[drawing_id_number]);
                        let rows = self.rows("saved_view")?;
                        renumber_drawing(
                            &mut rows[spot.row_index],
                            chart,
                            drawing_id_number,
                            fresh,
                        );
                        self.dirty.insert("saved_view");
                        replace_at = None;
                        self.report.conflicts += 1;
                    } else {
                        let modified = self
                            .store
                            .outbox_entry(EntityKind::LayoutDrawing, &sync_id)?
                            .map_or(self.ctx.now_ms, |row| row.queued_ms);
                        let outcome = merge(
                            EntityKind::LayoutDrawing,
                            None,
                            Side {
                                value: &projected,
                                stamp: Stamp::new(
                                    meta.as_ref().map_or(1, |m| m.revision.max(1)) as u64,
                                    modified,
                                    self.ctx.device_id.clone(),
                                ),
                            },
                            Side {
                                value: &record.value,
                                stamp: Stamp::new(
                                    record.record.revision,
                                    record.record.modified_ms,
                                    record.record.origin_device.clone(),
                                ),
                            },
                        );
                        let (Outcome::Resolved(value) | Outcome::Conflict { winner: value, .. }) =
                            outcome;
                        insert = value.get("drawing").cloned();
                    }
                }
            }
        }

        if let Some(drawing) = insert {
            let rows = self.rows("saved_view")?;
            let list = rows[spot.row_index]["charts"][spot.chart_index]
                .as_object_mut()
                .map(|chart| {
                    chart
                        .entry("drawings")
                        .or_insert_with(|| Value::Array(Vec::new()))
                });
            if let Some(Value::Array(list)) = list {
                match replace_at {
                    Some(position) if position < list.len() => list[position] = drawing,
                    _ => list.push(drawing),
                }
            }
            self.dirty.insert("saved_view");
            self.report.tables.insert("saved_view".into());
            self.report.applied += 1;
        }
        self.store.link_entity(&EntityLink {
            kind: EntityKind::LayoutDrawing,
            sync_id: sync_id.clone(),
            local_id: sync_id,
            synced_hash: remote_hash,
            system_fields: record.system_fields.clone(),
            base_payload: None,
            revision: record.record.revision as i64,
            origin_device: record.record.origin_device.clone(),
            now_ms: self.ctx.now_ms,
        })?;
        Ok(())
    }

    fn delete_drawing(&mut self, name: &RecordName) -> rusqlite::Result<()> {
        let kind = EntityKind::LayoutDrawing;
        let id = name.id.as_str();
        let Some((layout, chart, drawing)) = name.layout_drawing_parts() else {
            return Ok(());
        };
        if !self.table_writable("saved_view")? {
            self.report.inboxed += 1;
            return Ok(());
        }
        let spot = self.locate_drawing(layout, chart, drawing)?;
        let Some((spot, position)) = spot.and_then(|s| s.position.map(|p| (s, p))) else {
            self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
            return Ok(());
        };
        let meta = self.store.entity_meta(kind, id)?;
        let local_value =
            self.rows("saved_view")?[spot.row_index]["charts"][spot.chart_index]["drawings"]
                [position]
                .clone();
        let local_hash = sha256_hex(&canonical_json(
            &serde_json::json!({ "drawing": local_value }),
        ));
        let edited_since = meta
            .as_ref()
            .is_none_or(|m| m.synced_hash.as_deref() != Some(local_hash.as_str()))
            || self
                .store
                .outbox_entry(kind, id)?
                .is_some_and(|row| row.op == OutboxOp::Upsert);
        if edited_since {
            self.store.forget_synced(kind, id)?;
            self.store.queue_entity_upsert(kind, id, id)?;
            self.report.kept_local += 1;
            return Ok(());
        }
        self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
        self.store.cancel_outbox(kind, id)?;
        let rows = self.rows("saved_view")?;
        if let Some(list) =
            rows[spot.row_index]["charts"][spot.chart_index]["drawings"].as_array_mut()
        {
            list.remove(position);
        }
        self.dirty.insert("saved_view");
        self.report.tables.insert("saved_view".into());
        self.report.applied += 1;
        Ok(())
    }

    /// Drawings that arrived before their layout are applied once it exists.
    fn retry_orphans(&mut self) -> rusqlite::Result<()> {
        let waiting: Vec<InboxRecord> = self
            .store
            .inbox()?
            .into_iter()
            .filter(|r| r.reason == "orphan" && r.entity_type == EntityKind::LayoutDrawing.prefix())
            .collect();
        for entry in waiting {
            let Some(name) = RecordName::parse(&entry.record_name) else {
                continue;
            };
            let Some((layout, chart, drawing)) = name.layout_drawing_parts() else {
                continue;
            };
            if self.locate_drawing(layout, chart, drawing)?.is_none() {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<Value>(&entry.payload) else {
                continue;
            };
            self.store.remove_inbox(&entry.record_name)?;
            let record = RemoteRecord {
                record: super::record::SyncRecord {
                    name,
                    schema_version: entry.schema_version,
                    revision: entry.revision,
                    origin_device: entry.origin_device.clone(),
                    modified_ms: entry.modified_ms,
                    payload: entry.payload.clone(),
                },
                value,
                system_fields: entry.system_fields.clone().unwrap_or_default(),
            };
            self.apply_drawing(&record)?;
        }
        Ok(())
    }

    // MARK: Deletions

    fn apply_deletion(&mut self, name: &RecordName) -> rusqlite::Result<()> {
        let (kind, id) = (name.kind, name.id.as_str());
        match kind {
            EntityKind::Layout => return self.delete_layout(id),
            EntityKind::LayoutDrawing => return self.delete_drawing(name),
            EntityKind::Alert => return self.delete_alert(id),
            EntityKind::Script => return self.delete_script(id),
            _ => {}
        }
        if let Some(table) = table_of(kind)
            && !self.table_writable(table)?
        {
            self.report.inboxed += 1;
            return Ok(());
        }
        let meta = self.store.entity_meta(kind, id)?;
        let Some(local) = self.find_local(kind, id)? else {
            self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
            return Ok(());
        };
        let edited_since = meta
            .as_ref()
            .is_none_or(|m| m.synced_hash.as_deref() != Some(local.hash.as_str()))
            || self
                .store
                .outbox_entry(kind, id)?
                .is_some_and(|row| row.op == OutboxOp::Upsert);
        let is_favorites = id == FAVORITES_ID;
        if edited_since || is_favorites {
            // An edit beats a delete: the record comes back from this Mac.
            self.store.forget_synced(kind, id)?;
            self.store.queue_entity_upsert(kind, id, &local.local_id)?;
            self.report.kept_local += 1;
            return Ok(());
        }
        self.store
            .tombstone_entity(kind, id, &local.local_id, self.ctx.now_ms)?;
        self.store.cancel_outbox(kind, id)?;
        match table_of(kind) {
            Some(table) => self.remove_row(table, local.index)?,
            None => {
                self.settings.insert(id.to_string(), None);
            }
        }
        self.report.applied += 1;
        Ok(())
    }
}

/// A drawing's place inside the working copy of the layouts table.
struct DrawingSpot {
    row_index: usize,
    chart_index: usize,
    position: Option<usize>,
}

// MARK: Composition

/// The alert definition fields that sync; everything else on a stored alert is this Mac's
/// runtime progress.
const ALERT_DEFINITION: [&str; 9] = [
    "id",
    "asset",
    "condition",
    "currency",
    "frequency",
    "note",
    "created_at_ms",
    "webhook_endpoint_ids",
    "webhook_message",
];

/// A full alert for the runtime to save: the synced definition laid over this Mac's own runtime
/// fields. A paused alert stays paused; a spent one-shot alert stays spent unless it was edited.
fn compose_alert(target: &Value, existing: Option<&Value>, now_ms: i64) -> Option<Value> {
    let definition = target.as_object()?;
    let mut alert: Map<String, Value> = match existing.and_then(Value::as_object) {
        Some(existing) => existing.clone(),
        None => {
            let mut fresh = Map::new();
            fresh.insert("armed".into(), true.into());
            fresh.insert("previous_value".into(), Value::Null);
            fresh.insert("last_fingerprint".into(), Value::Null);
            fresh.insert("updated_at_ms".into(), now_ms.into());
            fresh
        }
    };
    let definition_unchanged = existing.is_some_and(|existing| {
        ALERT_DEFINITION
            .iter()
            .all(|field| existing.get(*field) == definition.get(*field))
    });
    for field in ALERT_DEFINITION {
        match definition.get(field) {
            Some(value) => {
                alert.insert(field.into(), value.clone());
            }
            None => {
                alert.remove(field);
            }
        }
    }
    let local_state = alert
        .get("state")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let state = match definition.get("intent").and_then(Value::as_str) {
        Some("paused") => "Paused".to_string(),
        // Triggered is this Mac's runtime progress, not an intent to re-arm.
        _ if definition_unchanged && local_state.as_deref() == Some("Triggered") => {
            "Triggered".to_string()
        }
        _ => "Active".to_string(),
    };
    alert.insert("state".into(), state.into());
    Some(Value::Object(alert))
}

/// The row this Mac stores for a synced layout: the cloud's structure, with the drawings and
/// per-Mac fields this Mac already has. Drawings arrive as their own records.
fn compose_layout(value: &Value, existing: Option<&Value>) -> Option<Value> {
    let mut row = value.as_object()?.clone();
    let local_charts = existing
        .and_then(|e| e.get("charts"))
        .and_then(Value::as_array);
    if let Some(Value::Array(charts)) = row.get_mut("charts") {
        for chart in charts {
            let id = chart.get("id").and_then(Value::as_u64);
            let drawings = local_charts
                .and_then(|list| {
                    list.iter()
                        .find(|c| c.get("id").and_then(Value::as_u64) == id)
                })
                .and_then(|c| c.get("drawings"))
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            if let Some(chart) = chart.as_object_mut() {
                chart.insert("drawings".into(), drawings);
            }
        }
    }
    for field in ["last_opened_at_ms", "autosave"] {
        match existing.and_then(|e| e.get(field)) {
            Some(value) => {
                row.insert(field.into(), value.clone());
            }
            None => {
                row.remove(field);
            }
        }
    }
    Some(Value::Object(row))
}

/// The row this Mac stores for a synced value. `None` when the value cannot become a valid row.
fn compose_row(
    kind: EntityKind,
    sync_id: &str,
    value: &Value,
    existing: Option<&Value>,
) -> Option<Value> {
    let mut row: Map<String, Value> = value.as_object()?.clone();
    match kind {
        EntityKind::Watchlist => {
            if sync_id == FAVORITES_ID {
                row.insert("is_favorites".into(), true.into());
                // Each Mac keeps its own Favorites row id.
                let id = existing
                    .and_then(|e| e.get("id").cloned())
                    .unwrap_or_else(|| crate::model::watchlist::new_id().into());
                row.insert("id".into(), id);
            } else {
                row.insert("is_favorites".into(), false.into());
                row.insert("id".into(), sync_id.parse::<u64>().ok()?.into());
            }
        }
        EntityKind::ChartTemplate | EntityKind::DrawingTemplate => {}
        _ => return None,
    }
    Some(Value::Object(row))
}

/// The value stored under a setting key. Some settings carry per-Mac choices that must survive
/// another Mac's change.
fn compose_setting(key: &str, synced: &Value, existing: Option<&Value>) -> Value {
    if key != "snapshot.prefs" {
        return synced.clone();
    }
    // Take where to upload and how; keep the hosts this Mac has agreed to send images to.
    let mut prefs = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(remote) = synced.as_object() {
        for field in ["host", "include_trading", "scale"] {
            if let Some(value) = remote.get(field) {
                prefs.insert(field.into(), value.clone());
            }
        }
    }
    prefs
        .entry("consented")
        .or_insert_with(|| Value::Array(Vec::new()));
    Value::Object(prefs)
}
