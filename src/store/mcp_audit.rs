//! The MCP activity log: what agents asked DegenView to do and how each call ended.
//!
//! One append-only table, newest rows kept. It is local to this Mac on purpose (see
//! `sync::policy`): it describes what happened *here*, and no other device should inherit or
//! rewrite it. Reads skip a row this version cannot decode, because one unreadable row must not
//! hide the rest of the history (or stop the server from starting): nothing writes the log back
//! from what it read.

use crate::model::mcp::{AUDIT_LOG_LIMIT, McpAuditEntry};
use crate::store::Store;

impl Store {
    /// Appends `entries` in order and prunes the table to the newest [`AUDIT_LOG_LIMIT`] rows,
    /// all in one transaction: a failure leaves the log exactly as it was.
    pub fn append_mcp_audit(&self, entries: &[McpAuditEntry]) -> rusqlite::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let encoded: Vec<(i64, String)> = entries
            .iter()
            .map(|entry| {
                serde_json::to_string(entry)
                    .map(|payload| (entry.at_ms, payload))
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })
            .collect::<rusqlite::Result<_>>()?;
        // SAVEPOINT also works when another writer already owns a transaction.
        self.connection
            .execute_batch("SAVEPOINT append_mcp_audit")?;
        let result = (|| {
            let mut insert = self
                .connection
                .prepare("INSERT INTO mcp_audit (at_ms, payload) VALUES (?1, ?2)")?;
            for (at_ms, payload) in &encoded {
                insert.execute(rusqlite::params![at_ms, payload])?;
            }
            self.connection.execute(
                "DELETE FROM mcp_audit WHERE id <= (SELECT MAX(id) FROM mcp_audit) - ?1",
                rusqlite::params![AUDIT_LOG_LIMIT as i64],
            )?;
            Ok(())
        })();
        if result.is_err() {
            self.connection
                .execute_batch("ROLLBACK TO append_mcp_audit")?;
        }
        self.connection.execute_batch("RELEASE append_mcp_audit")?;
        result
    }

    /// The newest `limit` rows, newest first. Rows that no longer decode are skipped.
    pub fn recent_mcp_audit(&self, limit: usize) -> Vec<McpAuditEntry> {
        self.payloads(&format!(
            "SELECT payload FROM mcp_audit ORDER BY id DESC LIMIT {limit}"
        ))
    }

    /// How many rows the log holds, readable or not.
    pub fn mcp_audit_count(&self) -> usize {
        self.connection
            .query_row("SELECT COUNT(*) FROM mcp_audit", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_or(0, |count| count.max(0) as usize)
    }

    /// Empties the log and leaves `marker` as its only row, so the log itself shows that it was
    /// cleared, when, and that nothing before that point survives.
    pub fn clear_mcp_audit(&self, marker: &McpAuditEntry) -> rusqlite::Result<()> {
        let payload = serde_json::to_string(marker)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        self.in_transaction(|| {
            // Not `DELETE FROM mcp_audit`: AUTOINCREMENT keeps counting, so ids never repeat.
            self.connection.execute("DELETE FROM mcp_audit", [])?;
            self.connection.execute(
                "INSERT INTO mcp_audit (at_ms, payload) VALUES (?1, ?2)",
                rusqlite::params![marker.at_ms, payload],
            )?;
            Ok(())
        })
    }

    /// Writes a row bypassing the model, to simulate data this version cannot read.
    #[cfg(test)]
    pub(crate) fn insert_raw_mcp_audit(&self, payload: &str) {
        self.connection
            .execute(
                "INSERT INTO mcp_audit (at_ms, payload) VALUES (0, ?1)",
                [payload],
            )
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mcp::{McpAuditKind, McpAuditOutcome};

    fn entry(n: u64) -> McpAuditEntry {
        McpAuditEntry {
            id: n,
            at_ms: 1_000 + n as i64,
            client_id: "c1".into(),
            client_name: "Claude".into(),
            tool: "degenview_list_charts".into(),
            count: 1,
            ..McpAuditEntry::default()
        }
    }

    #[test]
    fn rows_come_back_newest_first_and_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.recent_mcp_audit(10).is_empty());
        let mut denied = entry(2);
        denied.outcome = McpAuditOutcome::Denied;
        denied.code = "PERMISSION_DENIED".into();
        store.append_mcp_audit(&[entry(1), denied.clone()]).unwrap();
        store.append_mcp_audit(&[entry(3)]).unwrap();
        let rows = store.recent_mcp_audit(10);
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [3, 2, 1]);
        assert_eq!(rows[1], denied);
        assert_eq!(store.recent_mcp_audit(2).len(), 2);
        assert_eq!(store.mcp_audit_count(), 3);
    }

    #[test]
    fn the_log_keeps_only_the_newest_rows() {
        let store = Store::open_in_memory().unwrap();
        let all: Vec<_> = (1..=(AUDIT_LOG_LIMIT as u64 + 250)).map(entry).collect();
        // In several batches, as the host writes them.
        for batch in all.chunks(97) {
            store.append_mcp_audit(batch).unwrap();
        }
        assert_eq!(store.mcp_audit_count(), AUDIT_LOG_LIMIT);
        let rows = store.recent_mcp_audit(AUDIT_LOG_LIMIT + 10);
        assert_eq!(rows.len(), AUDIT_LOG_LIMIT);
        assert_eq!(rows[0].id, AUDIT_LOG_LIMIT as u64 + 250);
        assert_eq!(rows[AUDIT_LOG_LIMIT - 1].id, 251);
    }

    #[test]
    fn clearing_leaves_one_marker_and_later_rows_follow_it() {
        let store = Store::open_in_memory().unwrap();
        store.append_mcp_audit(&[entry(1), entry(2)]).unwrap();
        let marker = McpAuditEntry {
            id: 9,
            at_ms: 5_000,
            kind: McpAuditKind::Cleared,
            count: 1,
            ..McpAuditEntry::default()
        };
        store.clear_mcp_audit(&marker).unwrap();
        assert_eq!(store.recent_mcp_audit(10), vec![marker.clone()]);
        store.append_mcp_audit(&[entry(10)]).unwrap();
        let rows = store.recent_mcp_audit(10);
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [10, 9]);
    }

    #[test]
    fn a_row_this_version_cannot_read_is_skipped_not_fatal() {
        let store = Store::open_in_memory().unwrap();
        store.append_mcp_audit(&[entry(1)]).unwrap();
        store.insert_raw_mcp_audit("{ not json");
        store.insert_raw_mcp_audit(r#"{"outcome":"a_future_outcome"}"#);
        store.append_mcp_audit(&[entry(2)]).unwrap();
        let ids: Vec<_> = store.recent_mcp_audit(10).iter().map(|r| r.id).collect();
        assert_eq!(ids, [2, 1]);
        // The unreadable rows are still counted, and still pruned like any other.
        assert_eq!(store.mcp_audit_count(), 4);
    }

    #[test]
    fn a_row_written_by_an_older_version_fills_in_what_it_lacked() {
        let store = Store::open_in_memory().unwrap();
        store.insert_raw_mcp_audit(r#"{"id":7,"atMs":42,"tool":"x"}"#);
        let row = &store.recent_mcp_audit(1)[0];
        assert_eq!((row.id, row.at_ms, row.tool.as_str()), (7, 42, "x"));
        assert_eq!(row.kind, McpAuditKind::Call);
        assert_eq!(row.outcome, McpAuditOutcome::Succeeded);
        assert!(row.targets.is_empty());
    }

    #[test]
    fn an_empty_batch_writes_nothing() {
        let store = Store::open_in_memory().unwrap();
        store.append_mcp_audit(&[]).unwrap();
        assert_eq!(store.mcp_audit_count(), 0);
    }
}
