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
//!
//! This file holds the batch, the working copy of the tables, the dispatch by kind and the delete
//! of a plain row. Each kind's rules are in its own file (`simple`, `asset`, `script`, `alert`,
//! `layout`, `drawing`, `paper`, `portfolio`); `retry` looks again at what the inbox kept.

use super::core::{RemoteBatch, RemoteRecord};
use super::merge::{Outcome, Side, Stamp, conflict_copy_name, merge};
use crate::store::Store;
use crate::store::sync::{
    ConflictRecord, EntityLink, EntityMeta, InboxRecord, OutboxOp, row_decodes,
};
use crate::sync::engine::SyncError;
use crate::sync::record::policy::{EntityKind, Merge, category_of, merge_of, setting_is_portable};
use crate::sync::record::project::{
    canonical_json, project_row_in, project_setting, project_transaction,
};
use crate::sync::record::scripts::ScriptSink;
use crate::sync::record::{FAVORITES_ID, RecordName, sha256_hex};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

mod alert;
mod asset;
mod drawing;
mod layout;
mod paper;
mod portfolio;
mod retry;
mod script;
mod simple;

/// Why a fetched record was kept in the inbox, for the two reasons an update can fix.
pub const UNKNOWN_TYPE: &str = "unknown_type";
pub const NEWER_SCHEMA: &str = "newer_schema";
/// A record kept because a table holds a row this version cannot read, so it cannot be rewritten.
pub const LOCAL_UNREADABLE: &str = "local_unreadable";
/// A deletion kept for the same reason. Its payload is empty; only the name matters.
pub const DELETE_PENDING: &str = "delete_pending";

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
        record: crate::sync::record::SyncRecord {
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

pub fn apply_batch(
    store: &Store,
    scripts: &mut dyn ScriptSink,
    batch: &RemoteBatch,
    ctx: &ApplyCtx,
) -> Result<ApplyReport, SyncError> {
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
        // Portfolios too: a portfolio that is kept keeps its transactions.
        deletions.sort_by_key(|name| {
            (
                name.kind != EntityKind::Layout,
                name.kind != EntityKind::Portfolio,
            )
        });
        for name in deletions {
            work.apply_deletion(name)?;
        }
        work.retry_pending_deletions()?;
        work.retry_local_unreadable()?;
        work.retry_set_aside()?;
        work.retry_category_off()?;
        work.retry_orphans()?;
        work.retry_orphan_transactions()?;
        work.retry_held_ledgers()?;
        work.retry_busy_scripts()?;
        work.finish_ledgers()?;
        work.flush()?;
        work.flush_ledger_history()?;
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
    disabled: std::collections::HashSet<crate::sync::record::policy::Category>,
    report: ApplyReport,
    ids: crate::sync::record::ids::IdSpace,
    ledger: portfolio::Ledger,
}

pub fn table_of(kind: EntityKind) -> Option<&'static str> {
    match kind {
        EntityKind::Watchlist => Some("watchlist"),
        EntityKind::ChartTemplate => Some("chart_template"),
        EntityKind::EditorTheme => Some("editor_theme"),
        EntityKind::DrawingTemplate => Some("drawing_template"),
        EntityKind::Layout | EntityKind::LayoutDrawing => Some("saved_view"),
        EntityKind::DrawingAsset => Some("drawing_asset"),
        EntityKind::Alert => Some("price_alert"),
        EntityKind::Portfolio => Some("portfolio"),
        EntityKind::PortfolioTransaction => Some(portfolio::TRANSACTIONS),
        EntityKind::Setting | EntityKind::Script | EntityKind::PaperAccount => None,
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
            disabled: store.disabled_categories(),
            report: ApplyReport::default(),
            ids: store.id_space(),
            ledger: portfolio::Ledger::default(),
        }
    }

    // MARK: Table access

    /// Whether every existing row of the table is one this version can read and rewrite.
    fn table_writable(&mut self, table: &'static str) -> Result<bool, SyncError> {
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

    fn rows(&mut self, table: &'static str) -> Result<&mut Vec<Value>, SyncError> {
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

    fn find_local(&mut self, kind: EntityKind, sync_id: &str) -> Result<Option<Local>, SyncError> {
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
        let ids = self.ids;
        let rows = self.rows(table)?;
        let favorites = kind == EntityKind::Watchlist && sync_id == FAVORITES_ID;
        for (index, row) in rows.iter().enumerate() {
            let id = Self::row_id(row);
            // An entity's sync id is its row id (apart from the Favorites list, found by its
            // flag), so only the matching row needs projecting. Without this, applying n records
            // would project n rows n times.
            if favorites {
                if row.get("is_favorites").and_then(Value::as_bool) != Some(true) {
                    continue;
                }
            } else if kind != EntityKind::LayoutDrawing {
                // Only portfolios have counter ids that change on the way out.
                let row_sync_id = if matches!(
                    kind,
                    EntityKind::Portfolio | EntityKind::PortfolioTransaction
                ) {
                    ids.to_global_string(&id)
                } else {
                    id.clone()
                };
                if row_sync_id != sync_id {
                    continue;
                }
            }
            let entities = if kind == EntityKind::PortfolioTransaction {
                project_transaction(ids, &id, &row.to_string())
                    .into_iter()
                    .collect()
            } else {
                project_row_in(ids, table, &id, &row.to_string())
            };
            for entity in entities {
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
    ) -> Result<(), SyncError> {
        let rows = self.rows(table)?;
        match index {
            Some(index) => rows[index] = row,
            None => rows.push(row),
        }
        self.dirty.insert(table);
        self.report.tables.insert(table.to_string());
        Ok(())
    }

    fn remove_row(&mut self, table: &'static str, index: usize) -> Result<(), SyncError> {
        self.rows(table)?.remove(index);
        self.dirty.insert(table);
        self.report.tables.insert(table.to_string());
        Ok(())
    }

    fn flush(&mut self) -> Result<(), SyncError> {
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

    fn keep(&mut self, record: &RemoteRecord, reason: &str) -> Result<(), SyncError> {
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

    fn apply_record(&mut self, record: &RemoteRecord) -> Result<(), SyncError> {
        // A category the user switched off waits in the inbox, to be applied if it is switched on.
        let name = &record.record.name;
        if self.disabled.contains(&category_of(name.kind, &name.id)) {
            return self.keep(record, "category_off");
        }
        match record.record.name.kind {
            EntityKind::Watchlist
            | EntityKind::ChartTemplate
            | EntityKind::DrawingTemplate
            | EntityKind::EditorTheme
            | EntityKind::Portfolio
            | EntityKind::Setting => self.apply_simple(record),
            EntityKind::PortfolioTransaction => self.apply_transaction(record),
            EntityKind::Layout => self.apply_layout(record),
            EntityKind::LayoutDrawing => self.apply_drawing(record),
            EntityKind::DrawingAsset => self.apply_asset(record),
            EntityKind::Alert => self.apply_alert(record),
            EntityKind::Script => self.apply_script(record),
            EntityKind::PaperAccount => self.apply_paper(record),
        }
    }

    /// What an entity should become when the cloud's version meets this Mac's, in the shared
    /// (not the local) form. `None` when this Mac already holds something newer.
    fn merged_value(
        &mut self,
        record: &RemoteRecord,
        kind: EntityKind,
        id: &str,
        meta: Option<&EntityMeta>,
        local: Option<&Local>,
    ) -> Result<Option<Value>, SyncError> {
        let remote_hash = sha256_hex(&canonical_json(&record.value));
        let value = match local {
            None => record.value.clone(),
            Some(local) if local.hash == remote_hash => record.value.clone(),
            Some(local) => {
                let unchanged = meta.is_some_and(|m| {
                    !m.deleted && m.synced_hash.as_deref() == Some(local.hash.as_str())
                });
                if unchanged {
                    // We already hold something newer than this record: ignore the stale one.
                    if meta.is_some_and(|m| m.revision as u64 > record.record.revision) {
                        self.report.kept_local += 1;
                        return Ok(None);
                    }
                    record.value.clone()
                } else {
                    let base: Option<Value> = (merge_of(kind, id) == Merge::ThreeWay)
                        .then(|| meta.and_then(|m| m.base_payload.as_deref()))
                        .flatten()
                        .and_then(|bytes| serde_json::from_slice(bytes).ok());
                    let modified = self
                        .store
                        .outbox_entry(kind, id)?
                        .map_or(self.ctx.now_ms, |row| row.queued_ms);
                    let local_stamp = Stamp::new(
                        meta.map_or(1, |m| m.revision.max(1)) as u64,
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
        Ok(Some(value))
    }

    fn apply_deletion(&mut self, name: &RecordName) -> Result<(), SyncError> {
        let (kind, id) = (name.kind, name.id.as_str());
        // A copy set aside is for a record that still exists: it must not come back later.
        self.store.remove_inbox(&name.render())?;
        if kind == EntityKind::Setting && !setting_is_portable(id) {
            // A setting this version keeps on the Mac is not deleted, or tracked, from iCloud.
            return Ok(());
        }
        if self.disabled.contains(&category_of(kind, id)) {
            // Not applied; if the category returns, a reconcile re-uploads anything still here.
            return Ok(());
        }
        match kind {
            EntityKind::Layout => return self.delete_layout(id),
            EntityKind::LayoutDrawing => return self.delete_drawing(name),
            EntityKind::Alert => return self.delete_alert(id),
            EntityKind::Script => return self.delete_script(id),
            EntityKind::PortfolioTransaction => return self.delete_transaction(name),
            EntityKind::PaperAccount => return self.delete_paper(name),
            EntityKind::DrawingAsset => return self.keep_asset(id),
            _ => {}
        }
        if let Some(table) = table_of(kind)
            && !self.table_writable(table)?
        {
            // The delete cannot be applied to a table this version cannot rewrite. Keep it, by
            // name, and apply it when the table can be read: a checkpoint that moves on must not
            // leave the item here to be uploaded again.
            self.store.put_inbox(
                &InboxRecord {
                    record_name: name.render(),
                    entity_type: kind.prefix().to_string(),
                    schema_version: crate::sync::record::SCHEMA_VERSION,
                    reason: DELETE_PENDING.to_string(),
                    payload: Vec::new(),
                    system_fields: None,
                    revision: 0,
                    origin_device: String::new(),
                    modified_ms: self.ctx.now_ms,
                },
                self.ctx.now_ms,
            )?;
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
        // A transaction added here, which the Mac that deleted the portfolio cannot have seen, is an edit.
        let new_transactions = kind == EntityKind::Portfolio
            && match local.row.get("id").and_then(Value::as_u64) {
                Some(portfolio) => self.has_unseen_transactions(portfolio)?,
                None => false,
            };
        if edited_since || new_transactions || is_favorites {
            // An edit beats a delete: the record comes back from this Mac.
            if kind == EntityKind::Portfolio {
                self.keep_portfolio(&local);
            }
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
        if kind == EntityKind::Portfolio {
            self.cascade_portfolio_delete(&local)?;
        }
        self.report.applied += 1;
        Ok(())
    }
}
