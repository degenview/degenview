//! Kinds that are one synced row or setting each: watchlists, templates, editor themes,
//! portfolios and portable settings. The merged value is composed into the row this version
//! stores, which must read back.

use super::{
    EntityKind, EntityLink, FAVORITES_ID, LOCAL_UNREADABLE, Merge, RecordName, RemoteRecord, Work,
    canonical_json, merge_of, row_decodes, setting_is_portable, sha256_hex, table_of,
};
use crate::sync::engine::SyncError;
use crate::sync::record::ids::{Direction, map_fields};
use crate::sync::record::project::map_entry_cards;
use serde_json::{Map, Value};

/// Chart templates below this id are the built-in presets, which live in code, not the table.
const FIRST_USER_TEMPLATE_ID: u64 = 1000;

impl Work<'_> {
    pub(super) fn apply_simple(&mut self, record: &RemoteRecord) -> Result<(), SyncError> {
        let RecordName { kind, id } = &record.record.name;
        let (kind, id) = (*kind, id.as_str());
        if kind == EntityKind::Setting && !setting_is_portable(id) {
            // Validation already refuses these; this is the second lock on the same door.
            return self.keep(record, "invalid");
        }
        if let Some(table) = table_of(kind)
            && !self.table_writable(table)?
        {
            return self.keep(record, LOCAL_UNREADABLE);
        }
        if matches!(kind, EntityKind::ChartTemplate | EntityKind::EditorTheme)
            && id.parse::<u64>().is_ok_and(|n| n < FIRST_USER_TEMPLATE_ID)
        {
            return self.keep(record, "invalid");
        }

        let remote_payload = canonical_json(&record.value);
        let remote_hash = sha256_hex(&remote_payload);
        let meta = self.store.entity_meta(kind, id)?;
        let local = self.find_local(kind, id)?;

        let Some(mut new_value) =
            self.merged_value(record, kind, id, meta.as_ref(), local.as_ref())?
        else {
            return Ok(());
        };
        if kind == EntityKind::Portfolio {
            map_fields(&mut new_value, &["id"], self.ids, Direction::ToLocal);
        }

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
            let Some(mut row) = compose_row(kind, id, &new_value, existing) else {
                return self.keep(record, "invalid");
            };
            if kind == EntityKind::Watchlist
                && let Value::Object(map) = &mut row
            {
                map_entry_cards(map, self.ids, Direction::ToLocal);
            }
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
            base_payload: (merge_of(kind, id) == Merge::ThreeWay).then_some(remote_payload),
            revision: record.record.revision as i64,
            origin_device: record.record.origin_device.clone(),
            now_ms: self.ctx.now_ms,
        })?;
        Ok(())
    }
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
        EntityKind::ChartTemplate
        | EntityKind::DrawingTemplate
        | EntityKind::EditorTheme
        | EntityKind::Portfolio => {}
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
    // Take which provider to upload to and how it is set up; keep the destinations this Mac has
    // agreed to send images to. A Mac from before providers sends its one `host` instead: it is
    // taken too, and only read if this Mac has not chosen a provider yet.
    let mut prefs = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(remote) = synced.as_object() {
        for field in ["host", "provider", "providers", "include_trading", "scale"] {
            let Some(value) = remote.get(field) else {
                continue;
            };
            // Each provider's settings are taken by provider: a Mac that never set one up must not
            // erase what this Mac has for it by sending a map without it.
            if field == "providers"
                && let (Some(theirs), Some(Value::Object(ours))) =
                    (value.as_object(), prefs.get_mut(field))
            {
                for (id, settings) in theirs {
                    ours.insert(id.clone(), settings.clone());
                }
                continue;
            }
            prefs.insert(field.into(), value.clone());
        }
    }
    prefs
        .entry("consented")
        .or_insert_with(|| Value::Array(Vec::new()));
    Value::Object(prefs)
}
