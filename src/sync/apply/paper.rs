//! Paper-trading accounts.
//!
//! An account arrives whole. When only one side changed since they last agreed, that side wins.
//! When both did, the later edit keeps the account and the other version is kept as a separate,
//! paused account with ids of its own, so nothing either Mac did is lost and nothing is run twice.

use super::{
    ConflictRecord, EntityKind, EntityLink, Outcome, RecordName, RemoteRecord, Side, Stamp, Work,
    canonical_json, conflict_copy_name, merge, sha256_hex,
};
use crate::model::paper::{
    PaperAccount, PaperClosedTrade, PaperFill, PaperJournalEntry, PaperOrder, PaperOrderEvent,
    PaperPosition,
};
use crate::sync::ids::Direction;
use crate::sync::paper::{PAUSED, Parts, map_parts, project, renumber, split_bundle};
use serde_json::Value;

/// Whether every row of the account is one this version can read back.
fn decodes(parts: &Parts) -> bool {
    fn all<T: serde::de::DeserializeOwned>(rows: &[Value]) -> bool {
        rows.iter()
            .all(|row| serde_json::from_value::<T>(row.clone()).is_ok())
    }
    serde_json::from_value::<PaperAccount>(parts.account.clone()).is_ok()
        && all::<PaperOrder>(&parts.rows[0])
        && all::<PaperFill>(&parts.rows[1])
        && all::<PaperPosition>(&parts.rows[2])
        && all::<PaperOrderEvent>(&parts.rows[3])
        && all::<PaperClosedTrade>(&parts.rows[4])
        && all::<PaperJournalEntry>(&parts.rows[5])
}

impl Work<'_> {
    pub(super) fn apply_paper(&mut self, record: &RemoteRecord) -> rusqlite::Result<()> {
        let kind = EntityKind::PaperAccount;
        let sync_id = record.record.name.id.clone();
        let ids = self.ids;
        let Some(mut remote) = split_bundle(&record.value) else {
            return self.keep(record, "invalid");
        };
        map_parts(&mut remote, ids, Direction::ToLocal);
        let local_id = ids.to_local_string(&sync_id);
        if !decodes(&remote) || remote.account_id().map(|n| n.to_string()) != Some(local_id.clone())
        {
            return self.keep(record, "invalid");
        }

        // Never overwrite an account this version cannot read back.
        let local = self.store.paper_parts(&local_id)?;
        let present = self.store.paper_account_exists(&local_id)?;
        if present && local.as_ref().is_none_or(|parts| !decodes(parts)) {
            return self.keep(record, "local_unreadable");
        }

        let remote_hash = sha256_hex(&canonical_json(&record.value));
        let local_entity = local.clone().and_then(|parts| project(ids, parts));
        let meta = self.store.entity_meta(kind, &sync_id)?;

        let mut copy_of_local = false;
        let mut copy_of_remote = false;
        match &local_entity {
            None => {}
            Some(entity) if entity.hash == remote_hash => {}
            Some(entity) => {
                let unchanged = meta
                    .as_ref()
                    .is_some_and(|m| !m.deleted && m.synced_hash.as_deref() == Some(&entity.hash));
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
                        serde_json::from_slice(&entity.payload).unwrap_or(Value::Null);
                    let modified = self
                        .store
                        .outbox_entry(kind, &sync_id)?
                        .map_or(self.ctx.now_ms, |row| row.queued_ms);
                    let outcome = merge(
                        kind,
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
                    if let Outcome::Conflict { winner, .. } = outcome {
                        self.report.conflicts += 1;
                        if winner == local_value {
                            copy_of_remote = true;
                        } else {
                            copy_of_local = true;
                        }
                    }
                }
            }
        }

        let remote_wins = !copy_of_remote;
        if copy_of_local && let Some(parts) = local.clone() {
            self.save_paper_copy(record, parts, true)?;
        }
        if copy_of_remote {
            self.save_paper_copy(record, remote.clone(), false)?;
        }

        if remote_wins {
            if local_entity.as_ref().is_none_or(|e| e.hash != remote_hash) {
                self.store.write_paper_account(&remote)?;
                self.report.applied += 1;
            }
            let owner = (remote.owner != self.ctx.device_id).then_some(remote.owner.as_str());
            self.store.set_paper_owner(&local_id, owner)?;
            self.report.tables.insert("paper_account".to_string());
            self.store.link_entity(&EntityLink {
                kind,
                sync_id,
                local_id,
                synced_hash: remote_hash,
                system_fields: record.system_fields.clone(),
                base_payload: None,
                revision: record.record.revision as i64,
                origin_device: record.record.origin_device.clone(),
                now_ms: self.ctx.now_ms,
            })?;
        } else {
            // This Mac's version stays and goes back up over the cloud's.
            self.store.link_entity(&EntityLink {
                kind,
                sync_id: sync_id.clone(),
                local_id: local_id.clone(),
                synced_hash: remote_hash,
                system_fields: record.system_fields.clone(),
                base_payload: None,
                revision: record.record.revision as i64,
                origin_device: record.record.origin_device.clone(),
                now_ms: self.ctx.now_ms,
            })?;
            self.store.queue_entity_upsert(kind, &sync_id, &local_id)?;
        }
        Ok(())
    }

    /// Keeps the losing version as a separate account that nothing runs until the user takes it.
    fn save_paper_copy(
        &mut self,
        record: &RemoteRecord,
        mut parts: Parts,
        from_local: bool,
    ) -> rusqlite::Result<()> {
        let device = if from_local {
            self.ctx.device_name.clone()
        } else {
            self.store
                .device_name(&record.record.origin_device)?
                .unwrap_or_default()
        };
        renumber(&mut parts, crate::model::watchlist::new_id);
        let name = parts
            .account
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Paper Trading")
            .to_string();
        parts.account["name"] = conflict_copy_name(&name, &device, &self.ctx.date).into();
        parts.owner = PAUSED.to_string();
        let Some(copy_id) = parts.account_id() else {
            return Ok(());
        };
        if !decodes(&parts) {
            return Ok(());
        }
        let local_payload = from_local.then(|| {
            crate::sync::paper::bundle_value(&parts)
                .to_string()
                .into_bytes()
        });
        self.store.write_paper_account(&parts)?;
        self.store
            .set_paper_owner(&copy_id.to_string(), Some(PAUSED))?;
        let copy_sync_id = self.ids.to_global(copy_id).to_string();
        self.store.queue_entity_upsert(
            EntityKind::PaperAccount,
            &copy_sync_id,
            &copy_id.to_string(),
        )?;
        self.report.tables.insert("paper_account".to_string());
        self.store.record_conflict(
            &ConflictRecord {
                entity: EntityKind::PaperAccount,
                sync_id: record.record.name.id.clone(),
                kind: "paper".into(),
                local_payload,
                remote_payload: Some(record.record.payload.clone()),
                remote_device: Some(record.record.origin_device.clone()),
                copy_entity: Some(EntityKind::PaperAccount),
                copy_sync_id: Some(copy_sync_id),
            },
            self.ctx.now_ms,
        )
    }

    pub(super) fn delete_paper(&mut self, name: &RecordName) -> rusqlite::Result<()> {
        let kind = EntityKind::PaperAccount;
        let sync_id = name.id.as_str();
        let ids = self.ids;
        let local_id = ids.to_local_string(sync_id);
        let Some(parts) = self.store.paper_parts(&local_id)? else {
            if self.store.paper_account_exists(&local_id)? {
                // Present but unreadable: leave it alone.
                self.report.inboxed += 1;
            } else {
                self.store
                    .tombstone_entity(kind, sync_id, &local_id, self.ctx.now_ms)?;
            }
            return Ok(());
        };
        let meta = self.store.entity_meta(kind, sync_id)?;
        let hash = project(ids, parts).map(|entity| entity.hash);
        let edited_since = meta
            .as_ref()
            .is_none_or(|m| m.synced_hash.is_none() || m.synced_hash != hash)
            || self
                .store
                .outbox_entry(kind, sync_id)?
                .is_some_and(|row| row.op == super::OutboxOp::Upsert);
        if edited_since {
            // An edit beats a delete: the account comes back from this Mac.
            self.store.forget_synced(kind, sync_id)?;
            self.store.queue_entity_upsert(kind, sync_id, &local_id)?;
            self.report.kept_local += 1;
            return Ok(());
        }
        self.store
            .tombstone_entity(kind, sync_id, &local_id, self.ctx.now_ms)?;
        self.store.cancel_outbox(kind, sync_id)?;
        self.store.delete_paper_account(&local_id)?;
        self.report.tables.insert("paper_account".to_string());
        self.report.applied += 1;
        Ok(())
    }
}
