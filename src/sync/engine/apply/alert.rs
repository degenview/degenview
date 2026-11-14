//! Price alerts. The process that evaluates alerts owns their table, so remote changes reach it
//! through the alert command queue; only the synced definition is laid over this Mac's runtime
//! fields.

use super::{
    EntityKind, EntityLink, OutboxOp, Outcome, RemoteRecord, Side, Stamp, Work, canonical_json,
    merge, sha256_hex,
};
use crate::sync::engine::SyncError;
use serde_json::{Map, Value};

impl Work<'_> {
    /// Alerts are owned by whichever process runs evaluation (the app or the background agent),
    /// which holds the live list in memory and rewrites it. A table write from here would be
    /// overwritten, so remote changes go through the same command queue as the UI.
    pub(super) fn apply_alert(&mut self, record: &RemoteRecord) -> Result<(), SyncError> {
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
    ) -> Result<(), SyncError> {
        Ok(self.store.link_entity(&EntityLink {
            kind: EntityKind::Alert,
            sync_id: id.to_string(),
            local_id: id.to_string(),
            synced_hash: remote_hash,
            system_fields: record.system_fields.clone(),
            base_payload: None,
            revision: record.record.revision as i64,
            origin_device: record.record.origin_device.clone(),
            now_ms: self.ctx.now_ms,
        })?)
    }

    pub(super) fn delete_alert(&mut self, id: &str) -> Result<(), SyncError> {
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
}

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
