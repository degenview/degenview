//! Paper-trading accounts as sync bundles (see `sync::paper`).
//!
//! A bundle is rebuilt from the whole snapshot on every save, so what keeps this cheap is
//! remembering the hash of the last bundle looked at (`sync_entity.local_hash`): a save that
//! changed nothing about an account queues nothing, and a burst of fills queues one upload.

use super::*;
use crate::model::paper::PaperTradingSnapshot;
use crate::sync::paper::{COLLECTIONS, Parts, project};
use serde::Serialize;

/// How long a changed account waits before it uploads, so a burst of fills sends once.
pub const UPLOAD_DEBOUNCE_MS: i64 = 5_000;

/// Who runs an account that this Mac only shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elsewhere {
    /// The other Mac's name, empty when it is not known.
    pub name: String,
    /// A conflict copy that nobody runs yet.
    pub paused: bool,
}

impl Store {
    /// Which Mac runs this account (a local account id). Absent means this one.
    pub fn paper_owner(&self, local_id: &str) -> rusqlite::Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT owner_device FROM paper_account_local WHERE account_id = ?1",
                [local_id],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn paper_account_exists(&self, local_id: &str) -> rusqlite::Result<bool> {
        self.connection
            .prepare("SELECT 1 FROM paper_account WHERE id = ?1")?
            .exists([local_id])
    }

    /// Records which Mac runs an account; `None` returns it to this Mac.
    pub fn set_paper_owner(&self, local_id: &str, owner: Option<&str>) -> rusqlite::Result<()> {
        match owner {
            Some(owner) => self.connection.execute(
                "INSERT INTO paper_account_local (account_id, owner_device) VALUES (?1, ?2)
                 ON CONFLICT (account_id) DO UPDATE SET owner_device = excluded.owner_device",
                params![local_id, owner],
            )?,
            None => self.connection.execute(
                "DELETE FROM paper_account_local WHERE account_id = ?1",
                [local_id],
            )?,
        };
        Ok(())
    }

    /// Accounts another Mac runs, by local account id.
    pub fn paper_elsewhere(&self) -> rusqlite::Result<HashMap<u64, Elsewhere>> {
        let own = self.paper_self()?;
        let mut elsewhere = HashMap::new();
        for (account, owner) in self.paper_owners()? {
            let Ok(account) = account.parse::<u64>() else {
                continue;
            };
            if owner == own {
                continue;
            }
            let paused = owner == crate::sync::paper::PAUSED;
            let name = self.device_name(&owner)?.unwrap_or_default();
            elsewhere.insert(account, Elsewhere { name, paused });
        }
        Ok(elsewhere)
    }

    /// Stores an account and everything it owns exactly as given, replacing what was there. The
    /// caller decides the owner; nothing is queued for upload.
    pub fn write_paper_account(&self, parts: &Parts) -> rusqlite::Result<()> {
        let Some(id) = parts.account_id().map(|id| id.to_string()) else {
            return Ok(());
        };
        let payload = parts.account.to_string();
        let updated = self.connection.execute(
            "UPDATE paper_account SET payload = ?2 WHERE id = ?1",
            params![id, payload],
        )?;
        if updated == 0 {
            self.connection.execute(
                "INSERT INTO paper_account (id, position, payload)
                 VALUES (?1, (SELECT COALESCE(MAX(position), -1) + 1 FROM paper_account), ?2)",
                params![id, payload],
            )?;
        }
        for ((_, table), rows) in COLLECTIONS.iter().zip(&parts.rows) {
            self.connection
                .execute(&format!("DELETE FROM {table} WHERE account_id = ?1"), [&id])?;
            let mut insert = self.connection.prepare(&format!(
                "INSERT INTO {table} (account_id, payload) VALUES (?1, ?2)"
            ))?;
            for row in rows {
                insert.execute(params![id, row.to_string()])?;
            }
        }
        Ok(())
    }

    /// Removes an account and everything it owns. If it was the selected account the selection
    /// is cleared, for the app to choose another.
    pub fn delete_paper_account(&self, local_id: &str) -> rusqlite::Result<()> {
        self.connection
            .execute("DELETE FROM paper_account WHERE id = ?1", [local_id])?;
        for (_, table) in COLLECTIONS {
            self.connection.execute(
                &format!("DELETE FROM {table} WHERE account_id = ?1"),
                [local_id],
            )?;
        }
        self.connection.execute(
            "DELETE FROM paper_account_local WHERE account_id = ?1",
            [local_id],
        )?;
        self.connection.execute(
            "DELETE FROM setting WHERE key = 'paper.selectedAccountID' AND value = ?1",
            [local_id],
        )?;
        Ok(())
    }

    /// Drops this Mac's own default paper account when nothing was ever done in it and iCloud has
    /// accounts of its own, so joining does not leave a second, empty "Paper Trading" on every Mac.
    /// "Untouched": never uploaded, named like the default, balance unchanged, and no order, fill,
    /// position, event, closed trade or journal entry. Anything else is kept. Returns whether one
    /// was dropped. Nothing is uploaded or deleted in iCloud.
    pub fn drop_untouched_default_paper_account(&self, now_ms: i64) -> rusqlite::Result<bool> {
        let kind = EntityKind::PaperAccount;
        let ids = self.id_space();
        let accounts = self
            .connection
            .prepare("SELECT id FROM paper_account")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut candidate = None;
        let mut cloud_has_accounts = false;
        for id in accounts {
            let sync_id = ids.to_global_string(&id);
            let known_to_icloud = self
                .entity_meta(kind, &sync_id)?
                .is_some_and(|meta| !meta.deleted && meta.synced_hash.is_some());
            if known_to_icloud {
                cloud_has_accounts = true;
                continue;
            }
            if self.paper_owner(&id)?.is_some() {
                continue;
            }
            let Some(parts) = self.paper_parts(&id)? else {
                continue;
            };
            let account = &parts.account;
            let is_default = account.get("name").and_then(Value::as_str) == Some("Paper Trading");
            // Creating an account writes one journal line; that is not something the user did.
            let nothing_done = COLLECTIONS
                .iter()
                .zip(&parts.rows)
                .all(|((name, _), rows)| {
                    rows.iter().all(|row| {
                        *name == "journal"
                            && row
                                .get("message")
                                .and_then(Value::as_str)
                                .is_some_and(|text| text.starts_with("PAPER account created"))
                    })
                });
            let untouched =
                account.get("cash_balance") == account.get("initial_balance") && nothing_done;
            if is_default && untouched {
                candidate = Some(id);
            }
        }
        let (Some(id), true) = (candidate, cloud_has_accounts) else {
            return Ok(false);
        };
        let sync_id = ids.to_global_string(&id);
        self.in_savepoint("drop_default_paper", || {
            self.delete_paper_account(&id)?;
            self.cancel_outbox(kind, &sync_id)?;
            self.tombstone_entity(kind, &sync_id, &id, now_ms)
        })?;
        Ok(true)
    }

    /// The device id that stands for this Mac in a bundle's owner.
    fn paper_self(&self) -> rusqlite::Result<String> {
        Ok(self
            .sync_state()?
            .map(|state| state.device_id)
            .unwrap_or_default())
    }

    fn paper_owners(&self) -> rusqlite::Result<HashMap<String, String>> {
        let mut statement = self
            .connection
            .prepare("SELECT account_id, owner_device FROM paper_account_local")?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect()
    }

    /// Everything one stored account owns, read from the tables.
    pub fn paper_parts(&self, local_id: &str) -> rusqlite::Result<Option<Parts>> {
        let account: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM paper_account WHERE id = ?1",
                [local_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(account) = account else {
            return Ok(None);
        };
        let Ok(account) = serde_json::from_str::<Value>(&account) else {
            return Ok(None);
        };
        let owner = match self.paper_owner(local_id)? {
            Some(owner) => owner,
            None => self.paper_self()?,
        };
        let mut parts = Parts::empty(account, owner);
        for (index, (_, table)) in COLLECTIONS.iter().enumerate() {
            let mut statement = self.connection.prepare(&format!(
                "SELECT payload FROM {table} WHERE account_id = ?1 ORDER BY rowid"
            ))?;
            let payloads = statement
                .query_map([local_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for payload in payloads {
                match serde_json::from_str(&payload) {
                    Ok(value) => parts.rows[index].push(value),
                    // An unreadable row means the bundle cannot be trusted to be whole.
                    Err(_) => return Ok(None),
                }
            }
        }
        Ok(Some(parts))
    }

    /// Every stored account as the entity it syncs as.
    pub(super) fn stored_paper_entities(&self) -> rusqlite::Result<Vec<Projected>> {
        let ids = self.id_space();
        let mut statement = self.connection.prepare("SELECT id FROM paper_account")?;
        let accounts = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut entities = Vec::new();
        for id in accounts {
            if let Some(parts) = self.paper_parts(&id)?
                && let Some(entity) = project(ids, parts)
            {
                entities.push(entity);
            }
        }
        Ok(entities)
    }

    /// Queues what a save of the whole paper-trading snapshot changed. Runs in the save's
    /// transaction, after the rows were written.
    pub(crate) fn track_paper(&self, snapshot: &PaperTradingSnapshot) -> rusqlite::Result<()> {
        if !self.sync_enabled() {
            return Ok(());
        }
        let Some(state) = self.sync_state()? else {
            return Ok(());
        };
        if self
            .disabled_categories()
            .contains(&crate::sync::policy::Category::PaperTrading)
        {
            return Ok(());
        }
        let ids = IdSpace::new(state.id_tag);
        let kind = EntityKind::PaperAccount;
        let owners = self.paper_owners()?;
        let mut present: HashSet<String> = HashSet::new();

        let mut orders = group(&snapshot.orders, |row| row.account_id);
        let mut fills = group(&snapshot.fills, |row| row.account_id);
        let mut positions = group(&snapshot.positions, |row| row.account_id);
        let mut events = group(&snapshot.order_events, |row| row.account_id);
        let mut trades = group(&snapshot.closed_trades, |row| row.account_id);
        let mut journal = group(&snapshot.journal, |row| row.account_id);

        for account in &snapshot.accounts {
            let Ok(value) = serde_json::to_value(account) else {
                continue;
            };
            let owner = owners
                .get(&account.id.to_string())
                .cloned()
                .unwrap_or_else(|| state.device_id.clone());
            let mut parts = Parts::empty(value, owner);
            parts.rows[0] = orders.remove(&account.id).unwrap_or_default();
            parts.rows[1] = fills.remove(&account.id).unwrap_or_default();
            parts.rows[2] = positions.remove(&account.id).unwrap_or_default();
            parts.rows[3] = events.remove(&account.id).unwrap_or_default();
            parts.rows[4] = trades.remove(&account.id).unwrap_or_default();
            parts.rows[5] = journal.remove(&account.id).unwrap_or_default();
            let Some(entity) = project(ids, parts) else {
                continue;
            };
            present.insert(entity.sync_id.clone());
            self.track_paper_entity(&entity, &state.device_id)?;
        }

        // Accounts the cloud knows that are no longer here were deleted.
        let mut statement = self.connection.prepare(
            "SELECT sync_id, local_id FROM sync_entity WHERE entity_type = ?1 AND deleted = 0",
        )?;
        let known = statement
            .query_map([kind.prefix()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut gone: Vec<_> = known
            .iter()
            .filter(|(sync_id, _)| !present.contains(sync_id))
            .collect();
        gone.sort();
        let held = is_mass_delete_count(gone.len(), known.len());
        for (sync_id, local_id) in gone {
            let stub = Projected {
                kind,
                sync_id: sync_id.clone(),
                local_id: local_id.clone(),
                payload: Vec::new(),
                hash: String::new(),
            };
            let queue = if held {
                OutboxState::Held
            } else {
                OutboxState::Pending
            };
            self.queue_delete(&stub, &state.device_id, queue)?;
        }
        self.connection.execute(
            "DELETE FROM paper_account_local WHERE account_id NOT IN (SELECT id FROM paper_account)",
            [],
        )?;
        Ok(())
    }

    fn track_paper_entity(&self, entity: &Projected, device: &str) -> rusqlite::Result<()> {
        let mark: Option<(Option<String>, Option<String>, bool)> = self
            .connection
            .query_row(
                "SELECT synced_hash, local_hash, deleted FROM sync_entity
                 WHERE entity_type = ?1 AND sync_id = ?2",
                params![entity.kind.prefix(), entity.sync_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (synced, looked_at, deleted) = mark.unwrap_or((None, None, false));
        if !deleted && looked_at.as_deref() == Some(entity.hash.as_str()) {
            return Ok(());
        }
        let known = !deleted && synced.as_deref() == Some(entity.hash.as_str());
        if !known {
            // A burst of changes keeps the first deadline, so the account still uploads.
            let due = self
                .outbox_entry(entity.kind, &entity.sync_id)?
                .and_then(|row| row.next_attempt_ms)
                .unwrap_or_else(|| now_ms() + UPLOAD_DEBOUNCE_MS);
            self.queue_upsert(entity, device)?;
            self.connection.execute(
                "UPDATE sync_outbox SET next_attempt_ms = ?3 WHERE entity_type = ?1 AND sync_id = ?2",
                params![entity.kind.prefix(), entity.sync_id, due],
            )?;
        }
        self.connection.execute(
            "UPDATE sync_entity SET local_hash = ?3 WHERE entity_type = ?1 AND sync_id = ?2",
            params![entity.kind.prefix(), entity.sync_id, entity.hash],
        )?;
        Ok(())
    }
}

fn group<T: Serialize>(items: &[T], account: impl Fn(&T) -> u64) -> HashMap<u64, Vec<Value>> {
    let mut grouped: HashMap<u64, Vec<Value>> = HashMap::new();
    for item in items {
        if let Ok(value) = serde_json::to_value(item) {
            grouped.entry(account(item)).or_default().push(value);
        }
    }
    grouped
}
