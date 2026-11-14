//! Pictures drawings show. A picture is stored on its own, only when it is what its name says,
//! and never deleted by another Mac.

use super::{EntityKind, EntityLink, RemoteRecord, Work, canonical_json, sha256_hex};
use crate::store::asset_store::StoredAsset;
use crate::sync::engine::SyncError;

impl Work<'_> {
    /// A picture from the cloud. It is stored on its own (never through the table's working copy,
    /// which would read every picture), and only when it is what its name says.
    pub(super) fn apply_asset(&mut self, record: &RemoteRecord) -> Result<(), SyncError> {
        let id = record.record.name.id.clone();
        let Ok(asset) = serde_json::from_value::<StoredAsset>(record.value.clone()) else {
            return self.keep(record, "invalid");
        };
        if asset.id != id || asset.verified_bytes().is_none() {
            return self.keep(record, "invalid");
        }
        if self.store.put_asset_from_cloud(&asset)? {
            self.report.tables.insert("drawing_asset".to_string());
            self.report.applied += 1;
        }
        self.store.link_entity(&EntityLink {
            kind: EntityKind::DrawingAsset,
            sync_id: id.clone(),
            local_id: id,
            synced_hash: sha256_hex(&canonical_json(&record.value)),
            system_fields: record.system_fields.clone(),
            base_payload: None,
            revision: record.record.revision as i64,
            origin_device: record.record.origin_device.clone(),
            now_ms: self.ctx.now_ms,
        })?;
        Ok(())
    }

    /// Another Mac deleted a picture. Nothing here ever deletes one: a drawing on some Mac, in a
    /// layout not synced yet or on an undo stack may still show it. A picture this Mac holds goes
    /// back up (an edit beats a delete); one it does not hold is only noted as gone.
    pub(super) fn keep_asset(&mut self, id: &str) -> Result<(), SyncError> {
        let kind = EntityKind::DrawingAsset;
        if self.store.has_asset(id)? {
            self.store.forget_synced(kind, id)?;
            self.store.queue_entity_upsert(kind, id, id)?;
            self.report.kept_local += 1;
        } else {
            self.store.tombstone_entity(kind, id, id, self.ctx.now_ms)?;
        }
        Ok(())
    }
}
