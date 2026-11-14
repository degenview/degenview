//! Synced tables and settings as raw JSON, for applying remote changes: read, replaced through
//! the tracked path only when every row decodes, or cleared when iCloud's copy replaces them.

use super::RawDoc;
use crate::store::Store;
use rusqlite::OptionalExtension;
use serde_json::Value;

/// Why a synced table or setting could not be read or replaced as JSON.
#[derive(Debug, thiserror::Error)]
pub enum RawDataError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    /// A stored row or setting is not JSON.
    #[error(transparent)]
    NotJson(#[from] serde_json::Error),
    /// A row, stored or about to be written, does not decode as its table's model.
    #[error("{table}: {reason}")]
    Undecodable { table: String, reason: &'static str },
}

/// Whether this version can read a row of `table`. A write must never produce, or silently
/// overwrite, a row the app's own model would refuse.
pub fn row_decodes(table: &str, payload: &str) -> bool {
    use crate::model;
    match table {
        "watchlist" => serde_json::from_str::<model::watchlist::Watchlist>(payload).is_ok(),
        "saved_view" => serde_json::from_str::<model::SavedView>(payload).is_ok(),
        "chart_template" => {
            serde_json::from_str::<model::chart::template::ChartAppearanceTemplate>(payload).is_ok()
        }
        "drawing_template" => {
            serde_json::from_str::<model::drawings::template::DrawingTemplate>(payload).is_ok()
        }
        "editor_theme" => serde_json::from_str::<model::editor_theme::EditorTheme>(payload).is_ok(),
        "price_alert" => serde_json::from_str::<model::alert::PriceAlert>(payload).is_ok(),
        "portfolio" => serde_json::from_str::<model::portfolio::Portfolio>(payload).is_ok(),
        "portfolio_transaction" => {
            serde_json::from_str::<model::portfolio::PortfolioTransaction>(payload).is_ok()
        }
        _ => false,
    }
}

impl Store {
    /// Every row of a synced table as JSON, in order. A row that is not JSON fails the read: a
    /// collection with an unreadable row must never be rewritten.
    pub fn raw_rows(&self, table: &str) -> Result<Vec<Value>, RawDataError> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT payload FROM {table} ORDER BY position"))?;
        let payloads = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        payloads
            .into_iter()
            .map(|payload| Ok(serde_json::from_str(&payload)?))
            .collect()
    }

    /// Replaces a synced table with `rows`, through the same tracked path as every other write.
    /// Refuses unless every existing row, and every new row, decodes as the table's model: this
    /// version must never write something it cannot read back, nor overwrite what a newer
    /// version wrote.
    pub fn write_raw_rows(&self, table: &str, rows: &[Value]) -> Result<(), RawDataError> {
        let unreadable = |reason: &'static str| RawDataError::Undecodable {
            table: table.to_owned(),
            reason,
        };
        for existing in self.raw_rows(table)? {
            if !row_decodes(table, &existing.to_string()) {
                return Err(unreadable("an existing row cannot be decoded"));
            }
        }
        if rows.iter().any(|row| !row_decodes(table, &row.to_string())) {
            return Err(unreadable("a new row cannot be decoded"));
        }
        if table == "portfolio_transaction" {
            return Ok(self.replace_raw_transactions(rows)?);
        }
        let docs: Vec<RawDoc> = rows.iter().cloned().map(RawDoc).collect();
        Ok(self.replace_documents(table, &docs)?)
    }

    pub fn raw_setting(&self, key: &str) -> Result<Option<Value>, RawDataError> {
        let text = self
            .connection
            .query_row("SELECT value FROM setting WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        Ok(text.map(|text| serde_json::from_str(&text)).transpose()?)
    }

    /// Writes or clears a setting through the tracked path.
    pub fn write_raw_setting(&self, key: &str, value: Option<&Value>) -> rusqlite::Result<()> {
        self.write_setting(key, value.map(Value::to_string).as_deref())
    }

    /// Empties the synced collections that the cloud's copy replaces when the user chooses to use
    /// iCloud's data. Deliberately bypasses sync tracking: the cloud must never be told these
    /// rows were deleted. Alerts and scripts are never cleared; they merge instead.
    pub fn clear_replaceable_collections(&self) -> rusqlite::Result<()> {
        // All five or none: a failure part way must not leave a mixture of cleared and kept.
        self.in_savepoint("clear_replaceable", || {
            for table in [
                "watchlist",
                "saved_view",
                "chart_template",
                "drawing_template",
                "editor_theme",
            ] {
                self.connection
                    .execute(&format!("DELETE FROM {table}"), [])?;
            }
            Ok(())
        })
    }
}
