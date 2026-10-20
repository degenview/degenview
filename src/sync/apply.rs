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
use super::merge::{Outcome, Side, Stamp, merge};
use super::policy::{EntityKind, Merge};
use super::project::{canonical_json, project_row, project_setting};
use super::record::{FAVORITES_ID, RecordName, sha256_hex};
use crate::store::Store;
use crate::store::sync::{EntityLink, InboxRecord, OutboxOp, row_decodes};
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

/// What changed, so live owners (the watchlist store, the template stores) can reload.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub tables: BTreeSet<String>,
    pub settings: BTreeSet<String>,
    pub applied: usize,
    pub kept_local: usize,
    pub inboxed: usize,
    pub conflicts: usize,
}

/// Chart templates below this id are the built-in presets, which live in code, not the table.
const FIRST_USER_TEMPLATE_ID: u64 = 1000;

pub fn apply_batch(
    store: &Store,
    batch: &RemoteBatch,
    ctx: &ApplyCtx,
) -> rusqlite::Result<ApplyReport> {
    store.in_savepoint("apply_remote", || {
        let mut work = Work::new(store, ctx);
        let mut records: Vec<&RemoteRecord> = batch.records.iter().collect();
        records.sort_by_key(|r| (r.record.name.kind.prefix(), r.record.name.id.clone()));
        for record in records {
            work.apply_record(record)?;
        }
        for name in &batch.deletions {
            work.apply_deletion(name)?;
        }
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
    fn new(store: &'a Store, ctx: &'a ApplyCtx) -> Self {
        Self {
            store,
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
            // Layouts, drawings, alerts and scripts have their own apply paths.
            _ => self.keep(record, "deferred"),
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

    // MARK: Deletions

    fn apply_deletion(&mut self, name: &RecordName) -> rusqlite::Result<()> {
        let (kind, id) = (name.kind, name.id.as_str());
        if !matches!(
            kind,
            EntityKind::Watchlist
                | EntityKind::ChartTemplate
                | EntityKind::DrawingTemplate
                | EntityKind::Setting
        ) {
            self.report.inboxed += 1;
            return Ok(());
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

// MARK: Composition

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
