//! Layout drawings, each its own record placed into its layout's chart. A drawing that arrives
//! before its layout waits in the inbox until the layout exists.

use super::{
    EntityKind, EntityLink, InboxRecord, OutboxOp, Outcome, RecordName, RemoteRecord, Side, Stamp,
    Work, canonical_json, merge, sha256_hex,
};
use crate::sync::engine::SyncError;
use crate::sync::engine::merge::{next_drawing_id, renumber_drawing};
use crate::sync::record::project::drawing_id;
use serde_json::Value;

impl Work<'_> {
    /// Where a drawing sits: the layout row, the chart within it, and its place in the chart.
    fn locate_drawing(
        &mut self,
        layout: u64,
        chart: u64,
        drawing: u64,
    ) -> Result<Option<DrawingSpot>, SyncError> {
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

    pub(super) fn apply_drawing(&mut self, record: &RemoteRecord) -> Result<(), SyncError> {
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

    pub(super) fn delete_drawing(&mut self, name: &RecordName) -> Result<(), SyncError> {
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
    pub(super) fn retry_orphans(&mut self) -> Result<(), SyncError> {
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
            };
            self.apply_drawing(&record)?;
        }
        Ok(())
    }
}

/// A drawing's place inside the working copy of the layouts table.
struct DrawingSpot {
    row_index: usize,
    chart_index: usize,
    position: Option<usize>,
}
