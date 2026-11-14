//! Records kept in the inbox, looked at again on every batch: those an update may now read,
//! those whose table can now be rewritten, and those whose category was switched back on.

use super::{
    DELETE_PENDING, EntityKind, InboxRecord, LOCAL_UNREADABLE, NEWER_SCHEMA, RecordName,
    RemoteRecord, UNKNOWN_TYPE, Work, category_of, remote_from_inbox, table_of,
};
use crate::sync::engine::SyncError;
use crate::sync::record::validate::{Verdict, validate};

impl Work<'_> {
    /// Records this version could not read when they arrived (a kind it did not know, a schema
    /// newer than it) are looked at again: an update may have taught it to read them. Whatever it
    /// still cannot read stays where it is, untouched.
    pub(super) fn retry_set_aside(&mut self) -> Result<(), SyncError> {
        let waiting: Vec<InboxRecord> = self
            .store
            .inbox()?
            .into_iter()
            .filter(|entry| matches!(entry.reason.as_str(), UNKNOWN_TYPE | NEWER_SCHEMA))
            .collect();
        for entry in waiting {
            let Some(name) = RecordName::parse(&entry.record_name) else {
                continue;
            };
            let Some(payload) = crate::sync::record::from_wire(entry.payload.clone()) else {
                continue;
            };
            let record = crate::sync::record::SyncRecord {
                name,
                schema_version: entry.schema_version,
                revision: entry.revision,
                origin_device: entry.origin_device.clone(),
                modified_ms: entry.modified_ms,
                payload,
            };
            if let Verdict::Accept(value) = validate(&record) {
                self.store.remove_inbox(&entry.record_name)?;
                self.apply_record(&RemoteRecord {
                    record,
                    value,
                    system_fields: entry.system_fields.clone().unwrap_or_default(),
                })?;
            }
        }
        Ok(())
    }

    /// Deletions kept because their table could not be rewritten are applied once it can be.
    pub(super) fn retry_pending_deletions(&mut self) -> Result<(), SyncError> {
        let waiting: Vec<InboxRecord> = self
            .store
            .inbox()?
            .into_iter()
            .filter(|entry| entry.reason == DELETE_PENDING)
            .collect();
        for entry in waiting {
            let Some(name) = RecordName::parse(&entry.record_name) else {
                continue;
            };
            let writable = match table_of(name.kind) {
                Some(table) => self.table_writable(table)?,
                None => true,
            };
            if writable {
                // Removes the inbox row first, then deletes as a fresh delete would.
                self.apply_deletion(&name)?;
            }
        }
        Ok(())
    }

    /// Records kept because their table held a row this version could not read are applied once
    /// every row can be.
    pub(super) fn retry_local_unreadable(&mut self) -> Result<(), SyncError> {
        let waiting: Vec<InboxRecord> = self
            .store
            .inbox()?
            .into_iter()
            .filter(|entry| entry.reason == LOCAL_UNREADABLE)
            .collect();
        for entry in waiting {
            let Some(record) = remote_from_inbox(&entry) else {
                continue;
            };
            let writable = match table_of(record.record.name.kind) {
                Some(table) => self.table_writable(table)?,
                None => true,
            };
            if writable {
                self.store.remove_inbox(&entry.record_name)?;
                self.apply_record(&record)?;
            }
        }
        Ok(())
    }

    /// Records held back because their category was off are applied once it is switched on.
    pub(super) fn retry_category_off(&mut self) -> Result<(), SyncError> {
        let waiting: Vec<InboxRecord> = self
            .store
            .inbox()?
            .into_iter()
            .filter(|r| r.reason == "category_off")
            .filter(|r| {
                let id = RecordName::parse(&r.record_name).map(|name| name.id);
                EntityKind::from_prefix(&r.entity_type).is_some_and(|kind| {
                    !self
                        .disabled
                        .contains(&category_of(kind, id.as_deref().unwrap_or_default()))
                })
            })
            .collect();
        for entry in waiting {
            if let Some(record) = remote_from_inbox(&entry) {
                self.store.remove_inbox(&entry.record_name)?;
                self.apply_record(&record)?;
            }
        }
        Ok(())
    }
}
