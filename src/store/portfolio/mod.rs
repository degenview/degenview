//! Portfolio ledger storage. `PortfolioLedger` persists whole snapshots, so each save replaces
//! the ledger tables in one transaction. Ported from `AppDatabase+Portfolio.swift`.

use crate::model::portfolio::PortfolioSnapshot;
use crate::model::portfolio::{Portfolio, PortfolioLedgerSnapshot};
use crate::store::{Document, Store};
use rusqlite::params;
use serde::Serialize;
use std::collections::HashMap;

const KEY_SELECTED: &str = "portfolio.selectedPortfolioID";
const KEY_INVALIDATED_AFTER: &str = "portfolio.invalidatedAfter";

impl Document for Portfolio {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    /// Unlike the document list, a transaction or snapshot that no longer decodes fails the
    /// read: dropping one would silently change every balance after it.
    pub fn portfolio_ledger(&self) -> rusqlite::Result<PortfolioLedgerSnapshot> {
        Ok(PortfolioLedgerSnapshot {
            portfolios: self.documents_strict("portfolio")?,
            transactions: self
                .payloads_strict("SELECT payload FROM portfolio_transaction ORDER BY position")?,
            historical_snapshots: self
                .payloads_strict("SELECT payload FROM portfolio_snapshot ORDER BY rowid")?,
            selected_portfolio_id: self.setting_strict(KEY_SELECTED)?,
            invalidated_after: self
                .setting_strict(KEY_INVALIDATED_AFTER)?
                .unwrap_or_default(),
        })
    }

    /// Remembers which portfolio is selected, without rewriting the ledger.
    pub fn set_selected_portfolio(&self, id: Option<u64>) -> rusqlite::Result<()> {
        match id {
            Some(id) => self.set_setting(KEY_SELECTED, &id),
            None => self.remove_setting(KEY_SELECTED),
        }
    }

    /// Replaces one portfolio's value history from `from` on with `snapshots`, and stores the
    /// updated stale marks, touching nothing else: a rebuild must not rewrite every transaction.
    pub fn replace_portfolio_history(
        &self,
        portfolio_id: u64,
        from: i64,
        snapshots: &[PortfolioSnapshot],
        invalidated_after: &HashMap<u64, i64>,
    ) -> rusqlite::Result<()> {
        self.in_transaction(|| {
            self.connection.execute(
                "DELETE FROM portfolio_snapshot WHERE portfolio_id = ?1 AND timestamp >= ?2",
                params![portfolio_id.to_string(), from],
            )?;
            let mut insert = self.connection.prepare(
                "INSERT INTO portfolio_snapshot (portfolio_id, timestamp, payload) VALUES (?1, ?2, ?3)",
            )?;
            for value in snapshots {
                insert.execute(params![
                    value.portfolio_id.to_string(),
                    value.timestamp,
                    json(value)?,
                ])?;
            }
            drop(insert);
            if invalidated_after.is_empty() {
                self.remove_setting(KEY_INVALIDATED_AFTER)
            } else {
                self.set_setting(KEY_INVALIDATED_AFTER, invalidated_after)
            }
        })
    }

    pub fn save_portfolio_ledger(
        &self,
        snapshot: &PortfolioLedgerSnapshot,
    ) -> rusqlite::Result<()> {
        self.in_transaction(|| {
            self.replace_documents("portfolio", &snapshot.portfolios)?;

            let rows = snapshot
                .transactions
                .iter()
                .map(|value| {
                    Ok(TransactionRow {
                        id: value.id.to_string(),
                        portfolio_id: value.portfolio_id.to_string(),
                        timestamp: value.timestamp,
                        payload: json(value)?,
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            self.replace_transaction_rows(&rows)?;

            self.connection
                .execute("DELETE FROM portfolio_snapshot", [])?;
            let mut history = self.connection.prepare(
                "INSERT INTO portfolio_snapshot (portfolio_id, timestamp, payload) VALUES (?1, ?2, ?3)",
            )?;
            for value in &snapshot.historical_snapshots {
                history.execute(params![
                    value.portfolio_id.to_string(),
                    value.timestamp,
                    json(value)?,
                ])?;
            }

            match snapshot.selected_portfolio_id {
                Some(id) => self.set_setting(KEY_SELECTED, &id)?,
                None => self.remove_setting(KEY_SELECTED)?,
            }
            if snapshot.invalidated_after.is_empty() {
                self.remove_setting(KEY_INVALIDATED_AFTER)?;
            } else {
                self.set_setting(KEY_INVALIDATED_AFTER, &snapshot.invalidated_after)?;
            }
            Ok(())
        })
    }
}

/// One row of `portfolio_transaction`, ready to store.
struct TransactionRow {
    id: String,
    portfolio_id: String,
    timestamp: i64,
    payload: String,
}

impl Store {
    /// Replaces every transaction, queueing the differences for sync in the same transaction.
    fn replace_transaction_rows(&self, rows: &[TransactionRow]) -> rusqlite::Result<()> {
        let old = self.tracked_old_transactions()?;
        self.connection
            .execute("DELETE FROM portfolio_transaction", [])?;
        let mut insert = self.connection.prepare(
            "INSERT INTO portfolio_transaction (id, portfolio_id, timestamp, position, payload)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (position, row) in rows.iter().enumerate() {
            insert.execute(params![
                row.id,
                row.portfolio_id,
                row.timestamp,
                position as i64,
                row.payload,
            ])?;
        }
        drop(insert);
        if let Some(old) = old {
            let new: Vec<(String, String)> = rows
                .iter()
                .map(|row| (row.id.clone(), row.payload.clone()))
                .collect();
            self.track_transactions(&old, &new)?;
        }
        Ok(())
    }

    /// Replaces the transactions with rows that were already checked against the model, exactly
    /// as stored (a typed round trip could add fields an older Mac does not know).
    pub(crate) fn replace_raw_transactions(
        &self,
        rows: &[serde_json::Value],
    ) -> rusqlite::Result<()> {
        let rows = rows
            .iter()
            .map(|row| {
                let number = |key: &str| row.get(key).and_then(serde_json::Value::as_u64);
                Ok(TransactionRow {
                    id: number("id").map(|n| n.to_string()).ok_or_else(missing)?,
                    portfolio_id: number("portfolio_id")
                        .map(|n| n.to_string())
                        .ok_or_else(missing)?,
                    timestamp: row
                        .get("timestamp")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or_default(),
                    payload: row.to_string(),
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        self.in_savepoint("replace_transactions", || {
            self.replace_transaction_rows(&rows)
        })
    }

    /// Valuation history is derived from the ledger, so a change to the ledger invalidates it
    /// from the earliest affected moment: later snapshots go, and the portfolio is marked stale.
    pub(crate) fn invalidate_portfolio_history(
        &self,
        from: &std::collections::BTreeMap<u64, i64>,
    ) -> rusqlite::Result<()> {
        let mut stale: std::collections::HashMap<u64, i64> = self
            .setting_strict(KEY_INVALIDATED_AFTER)?
            .unwrap_or_default();
        for (portfolio, moment) in from {
            stale
                .entry(*portfolio)
                .and_modify(|current| *current = (*current).min(*moment))
                .or_insert(*moment);
            self.connection.execute(
                "DELETE FROM portfolio_snapshot WHERE portfolio_id = ?1 AND timestamp >= ?2",
                params![portfolio.to_string(), moment],
            )?;
        }
        self.set_setting(KEY_INVALIDATED_AFTER, &stale)
    }

    /// A deleted portfolio's valuation history, and the selection if it pointed there.
    pub(crate) fn forget_portfolio_history(
        &self,
        portfolios: &std::collections::BTreeSet<u64>,
    ) -> rusqlite::Result<()> {
        let mut stale: std::collections::HashMap<u64, i64> = self
            .setting_strict(KEY_INVALIDATED_AFTER)?
            .unwrap_or_default();
        let selected: Option<u64> = self.setting_strict(KEY_SELECTED)?;
        for portfolio in portfolios {
            stale.remove(portfolio);
            self.connection.execute(
                "DELETE FROM portfolio_snapshot WHERE portfolio_id = ?1",
                [portfolio.to_string()],
            )?;
            if selected == Some(*portfolio) {
                self.remove_setting(KEY_SELECTED)?;
            }
        }
        if stale.is_empty() {
            self.remove_setting(KEY_INVALIDATED_AFTER)
        } else {
            self.set_setting(KEY_INVALIDATED_AFTER, &stale)
        }
    }
}

fn missing() -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(
        "a transaction has no numeric id",
    )))
}

fn json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

#[cfg(test)]
mod tests;
