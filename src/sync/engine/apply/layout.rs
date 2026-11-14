//! Layouts. A layout carries its charts' structure; its drawings arrive as their own records and
//! are kept from this Mac's row. A conflict keeps the losing layout as a copy.

use super::{
    ConflictRecord, EntityKind, EntityLink, OutboxOp, Outcome, RemoteRecord, Side, Stamp, Work,
    canonical_json, conflict_copy_name, merge, project_row_in, row_decodes, sha256_hex,
};
use crate::sync::engine::SyncError;
use crate::sync::record::ids::Direction;
use crate::sync::record::project::map_chart_cards;
use serde_json::Value;

impl Work<'_> {
    pub(super) fn apply_layout(&mut self, record: &RemoteRecord) -> Result<(), SyncError> {
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
        let ids = self.ids;
        // The cloud's portfolio cards name portfolios by their shared ids; this Mac stores its own.
        let compose_layout = |value: &Value, existing: Option<&Value>| {
            let mut row = compose_layout(value, existing)?;
            if let Value::Object(map) = &mut row {
                map_chart_cards(map, ids, Direction::ToLocal);
            }
            Some(row)
        };
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
    ) -> Result<(), SyncError> {
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
        Ok(self.store.record_conflict(
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
        )?)
    }

    pub(super) fn delete_layout(&mut self, id: &str) -> Result<(), SyncError> {
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
        let drawings: Vec<String> =
            project_row_in(self.ids, "saved_view", id, &local.row.to_string())
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
