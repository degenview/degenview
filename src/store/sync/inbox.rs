//! `sync_inbox`: remote records kept instead of applied (a newer schema, an unknown type, an
//! orphan), to be reconsidered later.

use crate::store::Store;
use rusqlite::params;

/// A remote record kept in `sync_inbox` instead of being applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxRecord {
    pub record_name: String,
    pub entity_type: String,
    pub schema_version: u32,
    /// `newer_schema`, `invalid`, `unknown_type`, `orphan` or `initial_pending`.
    pub reason: String,
    pub payload: Vec<u8>,
    pub system_fields: Option<Vec<u8>>,
    pub revision: u64,
    pub origin_device: String,
    pub modified_ms: i64,
}

impl Store {
    /// Keeps a remote record that is not applied, to be reconsidered later (an update may teach
    /// this version to read it).
    pub fn put_inbox(&self, record: &InboxRecord, now_ms: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO sync_inbox
                (record_name, entity_type, schema_version, reason, payload, system_fields, revision,
                 origin_device, modified_ms, received_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.record_name,
                record.entity_type,
                record.schema_version,
                record.reason,
                record.payload,
                record.system_fields,
                record.revision as i64,
                record.origin_device,
                record.modified_ms,
                now_ms
            ],
        )?;
        Ok(())
    }

    pub fn inbox(&self) -> rusqlite::Result<Vec<InboxRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT record_name, entity_type, schema_version, reason, payload, system_fields,
                revision, origin_device, modified_ms
             FROM sync_inbox ORDER BY received_ms, record_name",
        )?;
        statement
            .query_map([], |row| {
                Ok(InboxRecord {
                    record_name: row.get(0)?,
                    entity_type: row.get(1)?,
                    schema_version: row.get(2)?,
                    reason: row.get(3)?,
                    payload: row.get(4)?,
                    system_fields: row.get(5)?,
                    revision: row.get::<_, i64>(6)? as u64,
                    origin_device: row.get(7)?,
                    modified_ms: row.get(8)?,
                })
            })?
            .collect()
    }

    /// How many kept records are waiting for one of `reasons`, without reading their payloads.
    pub fn inbox_count(&self, reasons: &[&str]) -> rusqlite::Result<usize> {
        let mut total = 0;
        for reason in reasons {
            total += self.connection.query_row(
                "SELECT COUNT(*) FROM sync_inbox WHERE reason = ?1",
                [reason],
                |row| row.get::<_, i64>(0).map(|count| count as usize),
            )?;
        }
        Ok(total)
    }

    pub fn remove_inbox(&self, record_name: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "DELETE FROM sync_inbox WHERE record_name = ?1",
            [record_name],
        )?;
        Ok(())
    }

    /// Removes a set-aside record only if it is waiting for one of `reasons`. Used when the cloud
    /// deletes a record whose name this version cannot parse: the copy it kept must not come back.
    pub fn remove_inbox_if_reason(
        &self,
        record_name: &str,
        reasons: &[&str],
    ) -> rusqlite::Result<()> {
        for reason in reasons {
            self.connection.execute(
                "DELETE FROM sync_inbox WHERE record_name = ?1 AND reason = ?2",
                params![record_name, reason],
            )?;
        }
        Ok(())
    }
}
