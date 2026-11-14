//! Pine scripts, held as files through the script sink rather than in a table. A script being
//! edited waits in the inbox until it is saved; a conflict keeps the losing version as a copy.

use super::{
    ConflictRecord, EntityKind, EntityLink, InboxRecord, OutboxOp, Outcome, RecordName,
    RemoteRecord, Side, Stamp, Work, canonical_json, conflict_copy_name, merge, sha256_hex,
};
use crate::sync::engine::SyncError;
use crate::sync::record::project::{ScriptSnapshot, project_script};
use serde_json::Value;

impl Work<'_> {
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

    pub(super) fn apply_script(&mut self, record: &RemoteRecord) -> Result<(), SyncError> {
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
                                // This Mac's version stays and still has to go up: it is not the
                                // cloud's, so the link below must not record it as synced.
                                intended = local_value;
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

        // The loser is saved first. If it cannot be, the batch fails and nothing is overwritten:
        // it is applied again later, and finds the copy it already made.
        let mut conflict = None;
        if let Some((loser, from_local)) = copy {
            conflict = Some(self.make_script_copy(record, &id, loser, from_local)?);
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
        if let Some(conflict) = conflict {
            self.store.record_conflict(&conflict, self.ctx.now_ms)?;
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

    /// Saves the losing version of a script as a new script, so neither edit is ever lost, and
    /// returns the note about it. An error when the copy cannot be written: the winner is not
    /// written either, and the whole batch is tried again.
    ///
    /// The copy's id is derived from what it holds, so applying the same batch again (after a
    /// failure later in it) finds the copy it already made instead of making another.
    fn make_script_copy(
        &mut self,
        record: &RemoteRecord,
        original: &str,
        loser: ScriptSnapshot,
        from_local: bool,
    ) -> Result<ConflictRecord, SyncError> {
        // `from_local` here means the local script won, so the loser is the cloud's version.
        let loser_is_local = !from_local;
        let device = if loser_is_local {
            self.ctx.device_name.clone()
        } else {
            self.store
                .device_name(&record.record.origin_device)?
                .unwrap_or_default()
        };
        let copy_name = conflict_copy_name(&loser.name, &device, &self.ctx.date);
        let copy_id = sha256_hex(
            format!(
                "{original}\0{copy_name}\0{}\0{}",
                loser.source, loser.is_favorite
            )
            .as_bytes(),
        )[..32]
            .to_string();
        let exists = matches!(self.scripts.get(&copy_id), Ok(Some(_)));
        if !exists
            && let Err(message) =
                self.scripts
                    .adopt(&copy_id, &copy_name, &loser.source, loser.is_favorite)
        {
            return Err(SyncError::ConflictCopy(message.to_string()));
        }
        Ok(ConflictRecord {
            entity: EntityKind::Script,
            sync_id: original.to_string(),
            kind: "script".into(),
            local_payload: loser_is_local.then(|| loser.source.clone().into_bytes()),
            remote_payload: Some(record.record.payload.clone()),
            remote_device: Some(record.record.origin_device.clone()),
            copy_entity: Some(EntityKind::Script),
            copy_sync_id: Some(copy_id),
        })
    }

    pub(super) fn delete_script(&mut self, id: &str) -> Result<(), SyncError> {
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
    pub(super) fn retry_busy_scripts(&mut self) -> Result<(), SyncError> {
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
            self.apply_script(&record)?;
        }
        Ok(())
    }
}
