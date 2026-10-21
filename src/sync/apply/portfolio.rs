//! Portfolios and their transactions.
//!
//! A portfolio is an ordinary synced row. A transaction is applied in two steps because the
//! ledger as a whole must stay possible: a sell needs the buy before it. Changes are staged while
//! the batch is applied, then each affected portfolio is checked once with the same accounting
//! the portfolio screen uses. Whatever would make it impossible is held back in the inbox (or, for
//! a delete, not applied) and tried again whenever more arrives, so a portfolio is never left
//! with a ledger the screen cannot show.

use super::{
    ConflictRecord, EntityKind, EntityLink, InboxRecord, Local, RecordName, RemoteRecord, Work,
    remote_from_inbox, row_decodes, sha256_hex,
};
use crate::model::portfolio::PortfolioTransaction;
use crate::model::portfolio_accounting::holdings;
use crate::store::sync::OutboxOp;
use crate::sync::ids::{Direction, map_fields};
use crate::sync::project::canonical_json;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub(super) const TRANSACTIONS: &str = "portfolio_transaction";
const HELD: &str = "ledger_invalid";
/// A transaction this Mac uploaded this recently was added at about the time another Mac deleted
/// its portfolio: that delete was fetched within one poll, and a person cannot have seen the
/// transaction, decided to delete the portfolio and uploaded it in that time.
const RECENT_UPLOAD_MS: i64 = 30_000;

#[derive(Default)]
pub(super) struct Ledger {
    staged: Vec<Staged>,
    /// Portfolios whose delete lost to a local edit; their transactions' deletes lose too.
    kept: HashSet<u64>,
    /// The earliest moment each portfolio's valuation history is no longer true.
    history: BTreeMap<u64, i64>,
    /// Portfolios deleted here by a remote delete; their valuation history goes with them.
    removed: BTreeSet<u64>,
}

enum Change {
    Upsert(Box<RemoteRecord>),
    Delete,
}

struct Staged {
    sync_id: String,
    local_id: String,
    portfolio: u64,
    change: Change,
    /// The row before this batch touched it.
    previous: Option<Value>,
}

fn row_id(row: &Value) -> String {
    match row.get("id") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn portfolio_of(row: &Value) -> Option<u64> {
    row.get("portfolio_id").and_then(Value::as_u64)
}

fn timestamp_of(row: &Value) -> i64 {
    row.get("timestamp").and_then(Value::as_i64).unwrap_or(0)
}

/// Whether the ledger of one portfolio can be valued: no sale of more than is held.
fn ledger_valid(rows: &[Value], portfolio: u64) -> bool {
    let transactions: Vec<PortfolioTransaction> = rows
        .iter()
        .filter(|row| portfolio_of(row) == Some(portfolio))
        .filter_map(|row| serde_json::from_value(row.clone()).ok())
        .collect();
    holdings(
        &transactions,
        &HashSet::from([portfolio]),
        None,
        &HashMap::new(),
        0,
    )
    .is_ok()
}

fn set_row(rows: &mut Vec<Value>, id: &str, row: Option<&Value>) {
    let position = rows.iter().position(|r| row_id(r) == id);
    match (position, row) {
        (Some(index), Some(row)) => rows[index] = row.clone(),
        (Some(index), None) => {
            rows.remove(index);
        }
        (None, Some(row)) => rows.push(row.clone()),
        (None, None) => {}
    }
}

impl Work<'_> {
    fn portfolio_exists(&mut self, local_id: u64) -> rusqlite::Result<bool> {
        let id = local_id.to_string();
        Ok(self.rows("portfolio")?.iter().any(|row| row_id(row) == id))
    }

    pub(super) fn apply_transaction(&mut self, record: &RemoteRecord) -> rusqlite::Result<()> {
        let kind = EntityKind::PortfolioTransaction;
        let sync_id = record.record.name.id.clone();
        if !self.table_writable(TRANSACTIONS)? {
            return self.keep(record, "local_unreadable");
        }
        let meta = self.store.entity_meta(kind, &sync_id)?;
        let local = self.find_local(kind, &sync_id)?;
        let Some(merged) =
            self.merged_value(record, kind, &sync_id, meta.as_ref(), local.as_ref())?
        else {
            return Ok(());
        };
        let mut row = merged;
        map_fields(
            &mut row,
            &["id", "portfolio_id"],
            self.ids,
            Direction::ToLocal,
        );
        let Some(portfolio) = portfolio_of(&row) else {
            return self.keep(record, "invalid");
        };
        if !row_decodes(TRANSACTIONS, &row.to_string()) {
            return self.keep(record, "invalid");
        }
        if !self.portfolio_exists(portfolio)? {
            return self.keep(record, "orphan");
        }
        let local_id = row_id(&row);
        self.ledger.staged.push(Staged {
            sync_id,
            local_id,
            portfolio,
            change: Change::Upsert(Box::new(record.clone())),
            previous: local.as_ref().map(|l| l.row.clone()),
        });
        self.put_row(TRANSACTIONS, local.as_ref().map(|l| l.index), row)
    }

    pub(super) fn delete_transaction(&mut self, name: &RecordName) -> rusqlite::Result<()> {
        let kind = EntityKind::PortfolioTransaction;
        let sync_id = name.id.as_str();
        let local_id = self.ids.to_local_string(sync_id);
        if !self.table_writable(TRANSACTIONS)? {
            self.report.inboxed += 1;
            return Ok(());
        }
        let Some(local) = self.find_local(kind, sync_id)? else {
            self.store
                .tombstone_entity(kind, sync_id, &local_id, self.ctx.now_ms)?;
            return Ok(());
        };
        let meta = self.store.entity_meta(kind, sync_id)?;
        let portfolio = portfolio_of(&local.row).unwrap_or_default();
        let edited_since = meta
            .as_ref()
            .is_none_or(|m| m.synced_hash.as_deref() != Some(local.hash.as_str()))
            || self
                .store
                .outbox_entry(kind, sync_id)?
                .is_some_and(|row| row.op == super::OutboxOp::Upsert);
        if edited_since || self.ledger.kept.contains(&portfolio) {
            // An edit beats a delete, and a kept portfolio keeps what is in it.
            self.store.forget_synced(kind, sync_id)?;
            self.store
                .queue_entity_upsert(kind, sync_id, &local.local_id)?;
            self.report.kept_local += 1;
            return Ok(());
        }
        self.ledger.staged.push(Staged {
            sync_id: sync_id.to_string(),
            local_id,
            portfolio,
            change: Change::Delete,
            previous: Some(local.row.clone()),
        });
        self.remove_row(TRANSACTIONS, local.index)
    }

    /// A remote delete of a portfolio took it; its transactions and valuation history go too.
    pub(super) fn cascade_portfolio_delete(&mut self, local: &Local) -> rusqlite::Result<()> {
        let Some(portfolio) = local.row.get("id").and_then(Value::as_u64) else {
            return Ok(());
        };
        let ids = self.ids;
        let now = self.ctx.now_ms;
        let rows = self.rows(TRANSACTIONS)?;
        let gone: Vec<String> = rows
            .iter()
            .filter(|row| portfolio_of(row) == Some(portfolio))
            .map(row_id)
            .collect();
        if gone.is_empty() {
            self.ledger.removed.insert(portfolio);
            return Ok(());
        }
        rows.retain(|row| portfolio_of(row) != Some(portfolio));
        for id in &gone {
            let sync_id = ids.to_global_string(id);
            let kind = EntityKind::PortfolioTransaction;
            self.store.tombstone_entity(kind, &sync_id, id, now)?;
            self.store.cancel_outbox(kind, &sync_id)?;
        }
        self.dirty.insert(TRANSACTIONS);
        self.report.tables.insert(TRANSACTIONS.to_string());
        self.ledger.removed.insert(portfolio);
        Ok(())
    }

    /// Whether this Mac holds a transaction of the portfolio that the Mac which deleted it cannot
    /// have seen: added or changed here and still waiting to upload, or uploaded from here a moment
    /// ago. Adding to a portfolio counts as editing it, so the delete must not take it.
    pub(super) fn has_unseen_transactions(&mut self, portfolio: u64) -> rusqlite::Result<bool> {
        let ids = self.ids;
        let kind = EntityKind::PortfolioTransaction;
        let now = self.ctx.now_ms;
        let this_mac = self.ctx.device_id.clone();
        let local: Vec<String> = self
            .rows(TRANSACTIONS)?
            .iter()
            .filter(|row| portfolio_of(row) == Some(portfolio))
            .map(row_id)
            .collect();
        for id in local {
            let sync_id = ids.to_global_string(&id);
            let meta = self.store.entity_meta(kind, &sync_id)?;
            let confirmed = meta
                .as_ref()
                .is_some_and(|meta| !meta.deleted && meta.synced_hash.is_some());
            let queued = self
                .store
                .outbox_entry(kind, &sync_id)?
                .is_some_and(|row| row.op == OutboxOp::Upsert);
            let just_uploaded = meta.as_ref().is_some_and(|meta| {
                meta.origin_device == this_mac
                    && meta
                        .cloud_modified_ms
                        .is_some_and(|at| now - at <= RECENT_UPLOAD_MS)
            });
            if !confirmed || queued || just_uploaded {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// A portfolio whose delete lost to a local edit keeps its transactions too.
    pub(super) fn keep_portfolio(&mut self, local: &Local) {
        if let Some(portfolio) = local.row.get("id").and_then(Value::as_u64) {
            self.ledger.kept.insert(portfolio);
        }
    }

    /// Records held back for the ledger, applied again now that more may have arrived.
    pub(super) fn retry_held_ledgers(&mut self) -> rusqlite::Result<()> {
        if self.ledger.staged.is_empty() {
            return Ok(());
        }
        let waiting: Vec<InboxRecord> = self
            .store
            .inbox()?
            .into_iter()
            .filter(|r| r.reason == HELD)
            .collect();
        for entry in waiting {
            if let Some(record) = remote_from_inbox(&entry) {
                self.store.remove_inbox(&entry.record_name)?;
                self.apply_record(&record)?;
            }
        }
        Ok(())
    }

    /// Transactions that arrived before their portfolio are applied once it exists.
    pub(super) fn retry_orphan_transactions(&mut self) -> rusqlite::Result<()> {
        let waiting: Vec<InboxRecord> = self
            .store
            .inbox()?
            .into_iter()
            .filter(|r| {
                r.reason == "orphan" && r.entity_type == EntityKind::PortfolioTransaction.prefix()
            })
            .collect();
        for entry in waiting {
            let Some(record) = remote_from_inbox(&entry) else {
                continue;
            };
            let mut value = record.value.clone();
            map_fields(&mut value, &["portfolio_id"], self.ids, Direction::ToLocal);
            let Some(portfolio) = portfolio_of(&value) else {
                continue;
            };
            if self.portfolio_exists(portfolio)? {
                self.store.remove_inbox(&entry.record_name)?;
                self.apply_transaction(&record)?;
            }
        }
        Ok(())
    }

    /// Checks every portfolio that changed this batch, links what is sound and holds the rest.
    pub(super) fn finish_ledgers(&mut self) -> rusqlite::Result<()> {
        let staged = std::mem::take(&mut self.ledger.staged);
        if staged.is_empty() {
            return Ok(());
        }
        let mut by_portfolio: BTreeMap<u64, Vec<Staged>> = BTreeMap::new();
        for change in staged {
            by_portfolio
                .entry(change.portfolio)
                .or_default()
                .push(change);
        }
        for (portfolio, group) in by_portfolio {
            let accepted = if ledger_valid(self.rows(TRANSACTIONS)?, portfolio) {
                vec![true; group.len()]
            } else {
                self.sort_out(portfolio, &group)?
            };
            for (change, accepted) in group.into_iter().zip(accepted) {
                if accepted {
                    self.accept(change)?;
                } else {
                    self.reject(change, portfolio)?;
                }
            }
        }
        Ok(())
    }

    /// Puts the portfolio back as it was, then lets each staged change back in while the ledger
    /// stays possible. A sale staged before its purchase gets another chance once the purchase is
    /// in, so the order the batch arrived in does not matter.
    fn sort_out(&mut self, portfolio: u64, group: &[Staged]) -> rusqlite::Result<Vec<bool>> {
        let ids = self.ids;
        let rows = self.rows(TRANSACTIONS)?;
        let incoming = |change: &Staged| match &change.change {
            Change::Upsert(record) => {
                let mut row = record.value.clone();
                map_fields(&mut row, &["id", "portfolio_id"], ids, Direction::ToLocal);
                Some(row)
            }
            Change::Delete => None,
        };
        let rows = &mut *rows;
        for change in group {
            set_row(rows, &change.local_id, change.previous.as_ref());
        }
        let mut accepted = vec![false; group.len()];
        let mut order: Vec<usize> = (0..group.len()).collect();
        order.sort_by_key(|index| {
            let change = &group[*index];
            let when = incoming(change)
                .or_else(|| change.previous.clone())
                .map_or(0, |row| timestamp_of(&row));
            (when, change.local_id.clone())
        });
        loop {
            let mut progress = false;
            for &index in &order {
                if accepted[index] {
                    continue;
                }
                let change = &group[index];
                let before = rows.iter().find(|r| row_id(r) == change.local_id).cloned();
                set_row(rows, &change.local_id, incoming(change).as_ref());
                if ledger_valid(rows, portfolio) {
                    accepted[index] = true;
                    progress = true;
                } else {
                    set_row(rows, &change.local_id, before.as_ref());
                }
            }
            if !progress {
                break;
            }
        }
        Ok(accepted)
    }

    fn accept(&mut self, change: Staged) -> rusqlite::Result<()> {
        let kind = EntityKind::PortfolioTransaction;
        let current = self
            .rows(TRANSACTIONS)?
            .iter()
            .find(|row| row_id(row) == change.local_id)
            .map(timestamp_of);
        let earliest = [change.previous.as_ref().map(timestamp_of), current]
            .into_iter()
            .flatten()
            .min();
        if let Some(from) = earliest {
            self.ledger
                .history
                .entry(change.portfolio)
                .and_modify(|current| *current = (*current).min(from))
                .or_insert(from);
        }
        match change.change {
            Change::Upsert(record) => {
                self.report.applied += 1;
                self.store.link_entity(&EntityLink {
                    kind,
                    sync_id: change.sync_id,
                    local_id: change.local_id,
                    synced_hash: sha256_hex(&canonical_json(&record.value)),
                    system_fields: record.system_fields.clone(),
                    base_payload: None,
                    revision: record.record.revision as i64,
                    origin_device: record.record.origin_device.clone(),
                    now_ms: self.ctx.now_ms,
                })
            }
            Change::Delete => {
                self.report.applied += 1;
                self.store.tombstone_entity(
                    kind,
                    &change.sync_id,
                    &change.local_id,
                    self.ctx.now_ms,
                )?;
                self.store.cancel_outbox(kind, &change.sync_id)
            }
        }
    }

    fn reject(&mut self, change: Staged, portfolio: u64) -> rusqlite::Result<()> {
        let kind = EntityKind::PortfolioTransaction;
        let name = self.portfolio_name(portfolio)?;
        match change.change {
            Change::Upsert(record) => {
                self.keep(&record, HELD)?;
                self.note_held(&change.sync_id, &name, Some(record.record.payload.clone()))
            }
            Change::Delete => {
                // The ledger needs the transaction another Mac deleted: it comes back from here.
                self.store.forget_synced(kind, &change.sync_id)?;
                self.store
                    .queue_entity_upsert(kind, &change.sync_id, &change.local_id)?;
                self.report.kept_local += 1;
                self.note_held(&change.sync_id, &name, None)
            }
        }
    }

    fn portfolio_name(&mut self, local_id: u64) -> rusqlite::Result<String> {
        let id = local_id.to_string();
        Ok(self
            .rows("portfolio")?
            .iter()
            .find(|row| row_id(row) == id)
            .and_then(|row| row.get("name").and_then(Value::as_str))
            .unwrap_or("a portfolio")
            .to_string())
    }

    /// One notice per held transaction, however many times it is retried.
    fn note_held(
        &mut self,
        sync_id: &str,
        portfolio_name: &str,
        remote: Option<Vec<u8>>,
    ) -> rusqlite::Result<()> {
        let kind = EntityKind::PortfolioTransaction;
        let open =
            self.store.conflicts()?.iter().any(|(entity, id, what)| {
                entity == kind.prefix() && id == sync_id && what == "ledger"
            });
        if open {
            return Ok(());
        }
        self.report.conflicts += 1;
        self.store.record_conflict(
            &ConflictRecord {
                entity: kind,
                sync_id: sync_id.to_string(),
                kind: "ledger".into(),
                local_payload: Some(portfolio_name.as_bytes().to_vec()),
                remote_payload: remote,
                remote_device: None,
                copy_entity: None,
                copy_sync_id: None,
            },
            self.ctx.now_ms,
        )
    }

    /// Valuation history that no longer matches the ledger, dropped and marked stale.
    pub(super) fn flush_ledger_history(&mut self) -> rusqlite::Result<()> {
        let history = std::mem::take(&mut self.ledger.history);
        let removed = std::mem::take(&mut self.ledger.removed);
        if !history.is_empty() {
            self.store.invalidate_portfolio_history(&history)?;
        }
        if !removed.is_empty() {
            self.store.forget_portfolio_history(&removed)?;
        }
        Ok(())
    }
}
