//! Reconciling the outbox with everything stored, for changes the write hooks never saw.

use super::{OutboxOp, OutboxState, is_mass_delete_count};
use crate::store::Store;
use crate::sync::record::policy::{EntityKind, TablePolicy, category_of, table_policy};
use crate::sync::record::project::{
    Projected, project_row_in, project_setting, project_transaction,
};
use std::collections::{HashMap, HashSet};

/// What a reconcile pass queued.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reconciled {
    pub upserts: usize,
    pub deletes: usize,
}

impl Store {
    /// Brings the outbox in line with everything stored, for changes the write hooks never saw:
    /// edits made by an older build, script files changed in Finder, and data that existed
    /// before sync was turned on. Safe to run repeatedly: an entity already queued, or already
    /// matching the cloud, is left alone.
    pub fn reconcile_stored(&self) -> rusqlite::Result<Reconciled> {
        let mut by_kind: HashMap<EntityKind, HashMap<String, Projected>> = HashMap::new();
        for kind in [
            EntityKind::Layout,
            EntityKind::LayoutDrawing,
            EntityKind::Watchlist,
            EntityKind::ChartTemplate,
            EntityKind::DrawingTemplate,
            EntityKind::EditorTheme,
            EntityKind::Alert,
            EntityKind::Portfolio,
            EntityKind::PortfolioTransaction,
            EntityKind::PaperAccount,
            EntityKind::Setting,
        ] {
            by_kind.insert(kind, HashMap::new());
        }
        let ids = self.id_space();
        for entity in self.stored_paper_entities()? {
            by_kind
                .entry(entity.kind)
                .or_default()
                .insert(entity.sync_id.clone(), entity);
        }
        for table in crate::store::DOCUMENT_TABLES {
            // Pictures are large and immutable: they are looked at by id below, never read whole.
            if !matches!(table_policy(table), TablePolicy::Synced(_))
                || table == crate::store::asset_store::TABLE
            {
                continue;
            }
            let mut statement = self
                .connection
                .prepare(&format!("SELECT id, payload FROM {table}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, payload) in rows {
                for entity in project_row_in(ids, table, &id, &payload) {
                    by_kind
                        .entry(entity.kind)
                        .or_default()
                        .insert(entity.sync_id.clone(), entity);
                }
            }
        }
        {
            let mut statement = self
                .connection
                .prepare("SELECT id, payload FROM portfolio_transaction")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, payload) in rows {
                if let Some(entity) = project_transaction(ids, &id, &payload) {
                    by_kind
                        .entry(entity.kind)
                        .or_default()
                        .insert(entity.sync_id.clone(), entity);
                }
            }
        }
        let mut statement = self.connection.prepare("SELECT key, value FROM setting")?;
        let settings = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, value) in settings {
            if let Some(entity) = project_setting(&key, &value) {
                by_kind
                    .entry(EntityKind::Setting)
                    .or_default()
                    .insert(entity.sync_id.clone(), entity);
            }
        }
        let mut total = Reconciled::default();
        total.upserts += self.reconcile_assets()?;
        let mut kinds: Vec<_> = by_kind.into_iter().collect();
        kinds.sort_by_key(|(kind, _)| kind.prefix());
        for (kind, current) in kinds {
            let done = self.reconcile_kind(kind, &current)?;
            total.upserts += done.upserts;
            total.deletes += done.deletes;
        }
        Ok(total)
    }

    /// Queues the upload of every kept picture the cloud is not known to have and that is not
    /// already queued, from the ids alone: a picture's name is the hash of its bytes, so one that
    /// is known never needs its bytes looked at again. Nothing is ever queued for deletion: a
    /// picture is not deleted.
    fn reconcile_assets(&self) -> rusqlite::Result<usize> {
        let Some(state) = self.sync_state()? else {
            return Ok(0);
        };
        let kind = EntityKind::DrawingAsset;
        if self.disabled_categories().contains(&kind.category()) {
            return Ok(0);
        }
        let queued: HashSet<String> = self
            .outbox()?
            .into_iter()
            .filter(|row| row.kind == kind)
            .map(|row| row.sync_id)
            .collect();
        let mut queued_now = 0;
        for id in self.asset_ids()? {
            let known = self
                .entity_mark(kind, &id)?
                .is_some_and(|mark| !mark.deleted);
            if known || queued.contains(&id) {
                continue;
            }
            let entity = Projected {
                kind,
                sync_id: id.clone(),
                local_id: id,
                payload: Vec::new(),
                hash: String::new(),
            };
            self.queue_upsert(&entity, &state.device_id)?;
            queued_now += 1;
        }
        Ok(queued_now)
    }

    /// Reconciles one kind against the complete set of entities that currently exist for it.
    pub fn reconcile_kind(
        &self,
        kind: EntityKind,
        current: &HashMap<String, Projected>,
    ) -> rusqlite::Result<Reconciled> {
        let Some(state) = self.sync_state()? else {
            return Ok(Reconciled::default());
        };
        // Settings sit in different categories by key, so they are judged one by one below.
        let off = self.disabled_categories();
        if kind != EntityKind::Setting && off.contains(&kind.category()) {
            return Ok(Reconciled::default());
        }
        let enabled = |sync_id: &str| !off.contains(&category_of(kind, sync_id));
        let device = state.device_id;
        let mut done = Reconciled::default();

        let queued_upserts: HashSet<String> = self
            .outbox()?
            .into_iter()
            .filter(|row| row.kind == kind && row.op == OutboxOp::Upsert)
            .map(|row| row.sync_id)
            .collect();
        // Alerts arrive through the command queue and only reach the table when the alert owner
        // next runs. While commands are waiting, an absent alert may just not have landed yet.
        let alerts_in_flight = kind == EntityKind::Alert
            && self
                .connection
                .query_row("SELECT COUNT(*) FROM alert_command", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(1)
                > 0;
        let mut ids: Vec<_> = current.keys().filter(|id| enabled(id)).collect();
        ids.sort();
        for id in ids {
            let entity = &current[id];
            let mark = self.entity_mark(kind, id)?;
            let matches_cloud = mark.as_ref().is_some_and(|m| {
                !m.deleted && m.synced_hash.as_deref() == Some(entity.hash.as_str())
            });
            let already_pending =
                queued_upserts.contains(id) && mark.as_ref().is_some_and(|m| !m.deleted);
            // A deletion that arrived from another Mac is applied to the table by the alert owner
            // through the command queue; until it runs, the row still there is not a new edit.
            let delete_in_flight = alerts_in_flight && mark.as_ref().is_some_and(|m| m.deleted);
            if !matches_cloud && !already_pending && !delete_in_flight {
                self.queue_upsert(entity, &device)?;
                done.upserts += 1;
            }
        }

        let mut statement = self.connection.prepare(
            "SELECT sync_id, local_id FROM sync_entity WHERE entity_type = ?1 AND deleted = 0",
        )?;
        let known = statement
            .query_map([kind.prefix()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut missing: Vec<_> = known
            .iter()
            .filter(|(sync_id, _)| {
                !alerts_in_flight && enabled(sync_id) && !current.contains_key(sync_id)
            })
            .collect();
        missing.sort();
        // A layout's drawings follow the layout; only parents are judged for a mass delete.
        let held =
            kind != EntityKind::LayoutDrawing && is_mass_delete_count(missing.len(), known.len());
        for (sync_id, local_id) in missing {
            let gone = Projected {
                kind,
                sync_id: sync_id.clone(),
                local_id: local_id.clone(),
                payload: Vec::new(),
                hash: String::new(),
            };
            let state = if held {
                OutboxState::Held
            } else {
                OutboxState::Pending
            };
            self.queue_delete(&gone, &device, state)?;
            done.deletes += 1;
        }
        Ok(done)
    }
}
