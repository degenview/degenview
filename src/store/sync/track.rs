//! The write hooks: each tracked write diffs a table's old and new rows (or a setting, or the
//! portfolio ledger) and queues the entity-level changes in the write's own savepoint, holding a
//! suspiciously large delete for confirmation.

use super::{EntityKey, OutboxState, is_mass_delete_count};
use crate::store::Store;
use crate::sync::record::policy::{
    EntityKind, SettingPolicy, TablePolicy, category_of, setting_policy, table_policy,
};
use crate::sync::record::project::{
    Projected, project_row_in, project_setting, project_transaction,
};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::collections::HashMap;

impl Store {
    /// The current rows of `table`, only when they will need diffing: sync is on and the table
    /// syncs. Read inside the write's savepoint, before the rows are replaced.
    pub(in crate::store) fn tracked_old_rows(
        &self,
        table: &str,
    ) -> rusqlite::Result<Option<Vec<(String, String)>>> {
        if !matches!(table_policy(table), TablePolicy::Synced(_)) || !self.sync_enabled() {
            return Ok(None);
        }
        let mut statement = self
            .connection
            .prepare(&format!("SELECT id, payload FROM {table}"))?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(rows))
    }

    /// Queues the entity-level differences between a table's old and new rows. Runs in the same
    /// savepoint as the write, so a change is never committed without its upload intent.
    pub(in crate::store) fn track_documents(
        &self,
        table: &str,
        old: &[(String, String)],
        new: &[(String, i64, String)],
    ) -> rusqlite::Result<()> {
        let ids = self.id_space();
        let project = |rows: &mut dyn Iterator<Item = (&str, &str)>| {
            let mut map: HashMap<EntityKey, Projected> = HashMap::new();
            for (id, payload) in rows {
                for entity in project_row_in(ids, table, id, payload) {
                    map.insert((entity.kind, entity.sync_id.clone()), entity);
                }
            }
            map
        };
        // Most rows are rewritten byte for byte. They cannot have changed, so only rows whose text
        // differs, appeared or vanished are projected: this keeps a save of thirty layouts, or the
        // alert engine's rewrite of its list, cheap.
        let old_text: HashMap<&str, &str> = old
            .iter()
            .map(|(id, p)| (id.as_str(), p.as_str()))
            .collect();
        let new_text: HashMap<&str, &str> = new
            .iter()
            .map(|(id, _, p)| (id.as_str(), p.as_str()))
            .collect();
        let old_entities = project(
            &mut old
                .iter()
                .filter(|(id, p)| new_text.get(id.as_str()) != Some(&p.as_str()))
                .map(|(id, p)| (id.as_str(), p.as_str())),
        );
        let new_entities = project(
            &mut new
                .iter()
                .filter(|(id, _, p)| old_text.get(id.as_str()) != Some(&p.as_str()))
                .map(|(id, _, p)| (id.as_str(), p.as_str())),
        );
        // How many of the table's entities existed before, for the mass-delete check. Layout
        // drawings follow their layout and are never counted on their own.
        let mut totals = HashMap::new();
        if let TablePolicy::Synced(kind) = table_policy(table) {
            totals.insert(kind, old.len());
        }
        self.queue_differences(&old_entities, &new_entities, &totals)
    }

    /// Queues the change to a portable setting; local settings are ignored.
    pub(in crate::store) fn track_setting(
        &self,
        key: &str,
        old: Option<&str>,
        new: Option<&str>,
    ) -> rusqlite::Result<()> {
        if !matches!(setting_policy(key), SettingPolicy::Portable(_)) {
            return Ok(());
        }
        let single = |value: Option<&str>| {
            let mut map: HashMap<EntityKey, Projected> = HashMap::new();
            if let Some(entity) = value.and_then(|value| project_setting(key, value)) {
                map.insert((entity.kind, entity.sync_id.clone()), entity);
            }
            map
        };
        let totals = HashMap::from([(EntityKind::Setting, usize::from(old.is_some()))]);
        self.queue_differences(&single(old), &single(new), &totals)
    }

    fn queue_differences(
        &self,
        old: &HashMap<EntityKey, Projected>,
        new: &HashMap<EntityKey, Projected>,
        totals: &HashMap<EntityKind, usize>,
    ) -> rusqlite::Result<()> {
        let Some(state) = self.sync_state()? else {
            return Ok(());
        };
        let device = state.device_id;
        let off = self.disabled_categories();

        let mut changed: Vec<&Projected> = new
            .iter()
            .filter(|(key, entity)| old.get(*key).map(|before| &before.hash) != Some(&entity.hash))
            .map(|(_, entity)| entity)
            .filter(|entity| !off.contains(&category_of(entity.kind, &entity.sync_id)))
            .collect();
        changed.sort_by(|a, b| (a.kind.prefix(), &a.sync_id).cmp(&(b.kind.prefix(), &b.sync_id)));
        for entity in changed {
            let mark = self.entity_mark(entity.kind, &entity.sync_id)?;
            // Equal to what the cloud already holds: a remote apply echoing back, or an edit that
            // was undone. Nothing to upload.
            let known = mark.as_ref().is_some_and(|m| {
                !m.deleted && m.synced_hash.as_deref() == Some(entity.hash.as_str())
            });
            if !known {
                self.queue_upsert(entity, &device)?;
            }
        }

        let mut vanished: Vec<&Projected> = old
            .iter()
            .filter(|(key, _)| !new.contains_key(*key))
            .map(|(_, entity)| entity)
            .filter(|entity| !off.contains(&category_of(entity.kind, &entity.sync_id)))
            .collect();
        vanished.sort_by(|a, b| (a.kind.prefix(), &a.sync_id).cmp(&(b.kind.prefix(), &b.sync_id)));
        let held_layouts = self.mass_deleted_layouts(totals, &vanished);
        // A transaction that goes with its portfolio follows that delete, held or not, and does
        // not count towards a mass delete of its own.
        let mut with_portfolio: HashMap<String, OutboxState> = HashMap::new();
        for entity in &vanished {
            if entity.kind == EntityKind::PortfolioTransaction
                && let Some(state) = self.portfolio_delete_state(entity)?
            {
                with_portfolio.insert(entity.sync_id.clone(), state);
            }
        }
        let gone_by_kind: HashMap<EntityKind, usize> = vanished
            .iter()
            .filter(|entity| !with_portfolio.contains_key(&entity.sync_id))
            .fold(HashMap::new(), |mut counts, entity| {
                *counts.entry(entity.kind).or_default() += 1;
                counts
            });
        for entity in vanished {
            let already = self
                .entity_mark(entity.kind, &entity.sync_id)?
                .is_some_and(|mark| mark.deleted);
            if already {
                continue;
            }
            if let Some(state) = with_portfolio.get(&entity.sync_id) {
                self.queue_delete(entity, &device, *state)?;
                continue;
            }
            let held = match entity.kind {
                EntityKind::LayoutDrawing => entity
                    .sync_id
                    .split('.')
                    .next()
                    .is_some_and(|layout| held_layouts.contains(layout)),
                _ => is_mass_delete_count(
                    gone_by_kind.get(&entity.kind).copied().unwrap_or(0),
                    totals.get(&entity.kind).copied().unwrap_or(0),
                ),
            };
            let state = if held {
                OutboxState::Held
            } else {
                OutboxState::Pending
            };
            self.queue_delete(entity, &device, state)?;
        }
        Ok(())
    }

    /// The state of the delete queued for the portfolio a vanished transaction belonged to.
    fn portfolio_delete_state(
        &self,
        transaction: &Projected,
    ) -> rusqlite::Result<Option<OutboxState>> {
        let Ok(Value::Object(payload)) = serde_json::from_slice::<Value>(&transaction.payload)
        else {
            return Ok(None);
        };
        let Some(portfolio) = payload.get("portfolio_id").and_then(Value::as_u64) else {
            return Ok(None);
        };
        let state: Option<String> = self
            .connection
            .query_row(
                "SELECT state FROM sync_outbox
                 WHERE entity_type = ?1 AND sync_id = ?2 AND op = 'delete'",
                params![EntityKind::Portfolio.prefix(), portfolio.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(state.map(|state| {
            if state == "held" {
                OutboxState::Held
            } else {
                OutboxState::Pending
            }
        }))
    }

    /// The current transaction rows, only when they will need diffing.
    pub(in crate::store) fn tracked_old_transactions(
        &self,
    ) -> rusqlite::Result<Option<Vec<(String, String)>>> {
        if !self.sync_enabled() {
            return Ok(None);
        }
        let mut statement = self
            .connection
            .prepare("SELECT id, payload FROM portfolio_transaction")?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(rows))
    }

    /// Queues the differences between the old and new transaction rows, in the savepoint of the
    /// ledger save. Runs after the portfolios were tracked, so a deleted portfolio's delete is
    /// already queued when its transactions are looked at.
    pub(in crate::store) fn track_transactions(
        &self,
        old: &[(String, String)],
        new: &[(String, String)],
    ) -> rusqlite::Result<()> {
        let ids = self.id_space();
        let old_text: HashMap<&str, &str> = old
            .iter()
            .map(|(id, p)| (id.as_str(), p.as_str()))
            .collect();
        let new_text: HashMap<&str, &str> = new
            .iter()
            .map(|(id, p)| (id.as_str(), p.as_str()))
            .collect();
        let project = |rows: &mut dyn Iterator<Item = &(String, String)>| {
            rows.filter_map(|(id, payload)| project_transaction(ids, id, payload))
                .map(|entity| ((entity.kind, entity.sync_id.clone()), entity))
                .collect::<HashMap<EntityKey, Projected>>()
        };
        let old_entities = project(
            &mut old
                .iter()
                .filter(|(id, p)| new_text.get(id.as_str()) != Some(&p.as_str())),
        );
        let new_entities = project(
            &mut new
                .iter()
                .filter(|(id, p)| old_text.get(id.as_str()) != Some(&p.as_str())),
        );
        let totals = HashMap::from([(EntityKind::PortfolioTransaction, old.len())]);
        self.queue_differences(&old_entities, &new_entities, &totals)
    }

    /// Layouts whose delete is held, so the deletes of their drawings are held with them.
    fn mass_deleted_layouts(
        &self,
        totals: &HashMap<EntityKind, usize>,
        vanished: &[&Projected],
    ) -> std::collections::HashSet<String> {
        let gone = vanished
            .iter()
            .filter(|entity| entity.kind == EntityKind::Layout)
            .count();
        if is_mass_delete_count(gone, totals.get(&EntityKind::Layout).copied().unwrap_or(0)) {
            vanished
                .iter()
                .filter(|entity| entity.kind == EntityKind::Layout)
                .map(|entity| entity.sync_id.clone())
                .collect()
        } else {
            Default::default()
        }
    }
}
