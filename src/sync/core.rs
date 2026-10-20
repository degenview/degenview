//! The sync state machine. It owns no threads and talks to no network: the host feeds it
//! `TransportEvent`s and calls `tick`, and it answers with `TransportCommand`s on the transport
//! and `Effect`s for the host. That keeps every rule here testable against `MockCloud`.
//!
//! Invariants it maintains:
//! * Nothing uploads unless sync is on, the iCloud account matches the one this database is
//!   bound to, and the first-time merge has finished.
//! * An outbox entry is removed only when the cloud accepted that exact entry (`seq`).
//! * A fetched change is applied locally before the engine checkpoint that covers it is saved.
//! * The status is never `UpToDate` while anything is waiting to upload.

use super::outbox::{Disposition, classify, is_transient};
use super::policy::EntityKind;
use super::record::{MAX_PAYLOAD_BYTES, RecordName, SCHEMA_VERSION, SyncRecord, sha256_hex};
use super::transport::{
    AccountChange, AccountStatus, ErrorKind, RawRecord, SendFailure, StagedChange, StagedOp,
    SyncTransport, TransportCommand, TransportEvent, ZoneEvent,
};
use super::validate::{Verdict, validate};
use crate::store::SharedStore;
use crate::store::sync::{InboxRecord, Load, OutboxOp, OutboxRow, OutboxState, SyncState};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::PoisonError;

/// A Mac that has not fetched for this long starts from scratch: its checkpoint is not trusted.
const STALE_AFTER_MS: i64 = 60 * 24 * 60 * 60 * 1_000;
/// Deletions found by catching up are applied unless they would remove this many items and over a
/// quarter of what is synced: that looks like a problem, not a clean-up, so nothing is removed.
const STALE_DELETE_LIMIT: usize = 20;

/// Entries handed to the engine per stage command.
const STAGE_BATCH: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// Another Apple Account is signed in; nothing moves until the user chooses.
    AccountChanged,
    /// The cloud data was deleted elsewhere.
    RemoteDataDeleted,
    /// A newer DegenView wrote data this version must not touch.
    NeedsUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Off,
    CheckingAccount,
    /// An iCloud account is signed in but this database is not bound to it yet.
    NeedsSetup,
    /// Bound, and iCloud's data has been read; waiting for the user to choose how to combine it
    /// with this Mac's before anything is applied or uploaded.
    AwaitingJoin,
    Ready,
    Syncing,
    UpToDate,
    WaitingForNetwork,
    WaitingForAccount,
    Paused(PauseReason),
    /// Held, blocked or conflicted items need the user.
    NeedsAttention,
    Error(ErrorKind),
}

/// A fetched record that passed validation, ready for the apply layer.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteRecord {
    pub record: SyncRecord,
    pub value: Value,
    pub system_fields: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteBatch {
    pub id: u64,
    pub records: Vec<RemoteRecord>,
    pub deletions: Vec<RecordName>,
}

/// What the host must do on the core's behalf.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Apply on the app thread, then call `remote_applied(batch.id)`.
    RemoteChanges(RemoteBatch),
    /// An account is signed in but this database has none bound: ask the user.
    NeedsBinding { user_hash: String },
    /// A different account than the bound one: ask the user.
    AccountMismatch { user_hash: String },
}

/// What the UI shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub phase: Phase,
    pub pending: usize,
    pub held: usize,
    pub blocked: usize,
    pub conflicts: usize,
    pub last_send_ms: Option<i64>,
    pub last_fetch_ms: Option<i64>,
    pub last_error: Option<String>,
    /// iCloud confirmed that this Mac's request to delete all DegenView data there is done.
    pub cloud_deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Account {
    Unknown,
    Unavailable,
    Available(String),
}

#[derive(Debug, Clone)]
struct Inflight {
    kind: EntityKind,
    sync_id: String,
    hash: Option<String>,
}

pub struct SyncCore<T: SyncTransport> {
    store: SharedStore,
    transport: T,
    clock: Box<dyn Fn() -> i64 + Send>,
    started: bool,
    account: Account,
    network_up: bool,
    inflight: HashMap<i64, Inflight>,
    fetch_in_flight: bool,
    initial_fetch_done: bool,
    last_error: Option<(ErrorKind, String)>,
    pause: Option<PauseReason>,
    unapplied: HashSet<u64>,
    pending_state: Option<Vec<u8>>,
    next_batch: u64,
    reconcile_needed: bool,
    effects: Vec<Effect>,
    scripts: Option<super::scripts::ScriptCache>,
    /// A requested deletion of all data in iCloud: asked for, then confirmed.
    cloud_delete: CloudDelete,
    /// While catching up from scratch (an expired checkpoint, or a Mac that has been away for
    /// months): the names iCloud still holds. iCloud does not replay deletions to a device that
    /// old, so whatever it no longer lists was deleted while this Mac was away.
    stale_seen: Option<HashSet<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudDelete {
    Idle,
    Requested,
    Confirmed,
}

impl<T: SyncTransport> SyncCore<T> {
    pub fn new(store: SharedStore, transport: T, clock: Box<dyn Fn() -> i64 + Send>) -> Self {
        Self {
            store,
            transport,
            clock,
            started: false,
            account: Account::Unknown,
            network_up: true,
            inflight: HashMap::new(),
            fetch_in_flight: false,
            initial_fetch_done: false,
            last_error: None,
            pause: None,
            unapplied: HashSet::new(),
            pending_state: None,
            next_batch: 1,
            reconcile_needed: false,
            effects: Vec::new(),
            scripts: None,
            cloud_delete: CloudDelete::Idle,
            stale_seen: None,
        }
    }

    /// Where uploads of script files read their content from; see `scripts::scan_scripts`.
    pub fn set_script_cache(&mut self, cache: super::scripts::ScriptCache) {
        self.scripts = Some(cache);
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn state(&self) -> Option<SyncState> {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sync_state()
            .ok()
            .flatten()
    }

    fn enabled(&self) -> bool {
        self.state().is_some_and(|state| state.enabled)
    }

    // MARK: Lifecycle

    /// Starts the engine if sync is on. Safe to call repeatedly.
    pub fn start(&mut self) {
        let Some(state) = self.state() else { return };
        if !state.enabled || self.started {
            return;
        }
        self.started = true;
        // The checkpoint is only valid for the account it was saved under, and only trusted if
        // this Mac has fetched recently.
        let away = state
            .last_fetch_ms
            .is_some_and(|last| self.now() - last > STALE_AFTER_MS);
        let engine_state = match (&state.engine_state, &state.engine_state_account_hash) {
            (Some(bytes), Some(owner)) if state.account_hash.as_ref() == Some(owner) && !away => {
                Some(bytes.clone())
            }
            _ => None,
        };
        if away && state.initial_sync_done {
            self.stale_seen = Some(HashSet::new());
        }
        self.transport.send(TransportCommand::Start {
            zone: state.zone_name,
            engine_state,
        });
        self.transport.send(TransportCommand::AccountCheck);
    }

    pub fn stop(&mut self) {
        if self.started {
            self.transport.send(TransportCommand::Stop);
        }
        self.started = false;
        self.account = Account::Unknown;
        self.inflight.clear();
        self.fetch_in_flight = false;
    }

    /// Resumes after the user resolved a pause.
    pub fn clear_pause(&mut self) {
        self.pause = None;
        self.reconcile_needed = true;
    }

    /// Asks iCloud to delete every DegenView record for this account. Nothing local is touched
    /// here; `Status::cloud_deleted` turns true once iCloud confirms, and only then should the
    /// caller forget the binding.
    pub fn delete_cloud_data(&mut self) {
        // Only ever the account this database is bound to: never whichever one happens to be
        // signed in.
        if !self.started || !self.bound_account() {
            return;
        }
        self.cloud_delete = CloudDelete::Requested;
        self.transport.send(TransportCommand::DeleteZone);
    }

    /// "Sync Now": fetch and upload, without discarding or resetting anything.
    pub fn request_sync(&mut self) {
        self.request_fetch();
        self.pump();
    }

    /// Periodic housekeeping: retry due entries and pick up local changes.
    pub fn tick(&mut self) {
        if self.reconcile_needed && self.can_sync() {
            self.reconcile_needed = false;
            let _ = self
                .store
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .reconcile_stored();
        }
        self.pump();
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    // MARK: Gates

    fn bound_account(&self) -> bool {
        let Account::Available(hash) = &self.account else {
            return false;
        };
        self.state()
            .is_some_and(|state| state.account_hash.as_deref() == Some(hash.as_str()))
    }

    fn can_sync(&self) -> bool {
        self.started && self.pause.is_none() && self.network_up && self.bound_account()
    }

    fn can_upload(&self) -> bool {
        self.can_sync()
            && self
                .state()
                .is_some_and(|s| s.enabled && s.initial_sync_done)
    }

    // MARK: Upload

    fn request_fetch(&mut self) {
        if self.can_sync() && !self.fetch_in_flight {
            self.fetch_in_flight = true;
            self.transport.send(TransportCommand::Fetch);
        }
    }

    /// Stages every due outbox entry that is not already on its way, and sends.
    pub fn pump(&mut self) {
        if !self.can_upload() {
            return;
        }
        let now = self.now();
        let device = self.state().map(|s| s.device_id).unwrap_or_default();
        let busy: HashSet<(EntityKind, String)> = self
            .inflight
            .values()
            .map(|entry| (entry.kind, entry.sync_id.clone()))
            .collect();
        let due = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .outbox_due(now)
            .unwrap_or_default();

        let mut staged = Vec::new();
        for row in due {
            if self.inflight.contains_key(&row.seq)
                || busy.contains(&(row.kind, row.sync_id.clone()))
            {
                continue;
            }
            // A delete is final in the cloud, so never send one before this session has caught up
            // with what other Macs changed: an edit made elsewhere should win.
            if row.op == OutboxOp::Delete && !self.initial_fetch_done {
                continue;
            }
            if staged.len() >= STAGE_BATCH {
                break;
            }
            if let Some((change, hash)) = self.build(&row, &device) {
                self.inflight.insert(
                    row.seq,
                    Inflight {
                        kind: row.kind,
                        sync_id: row.sync_id.clone(),
                        hash,
                    },
                );
                staged.push(change);
            }
        }
        if !staged.is_empty() {
            // The account the data is bound to travels with it, so the bridge can refuse to stage
            // for whichever account happens to be signed in by the time the command arrives.
            let Account::Available(account_hash) = &self.account else {
                return;
            };
            self.transport.send(TransportCommand::Stage {
                account_hash: account_hash.clone(),
                changes: staged,
            });
            self.transport.send(TransportCommand::Send);
        }
    }

    /// Projects the live entity for an outbox row. `None` when nothing should be sent now.
    fn build(&mut self, row: &OutboxRow, device: &str) -> Option<(StagedChange, Option<String>)> {
        let Some(name) = RecordName::new(row.kind, row.sync_id.clone()) else {
            self.block(row.seq, "illegal record name");
            return None;
        };
        let (meta, load) = {
            let store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
            (
                store.entity_meta(row.kind, &row.sync_id).ok().flatten(),
                store.load_projected(row.kind, &row.sync_id).ok(),
            )
        };
        let system_fields = meta.as_ref().and_then(|m| m.system_fields.clone());
        match row.op {
            OutboxOp::Delete => Some((
                StagedChange {
                    outbox_seq: row.seq,
                    name,
                    op: StagedOp::Delete,
                },
                None,
            )),
            OutboxOp::Upsert => {
                let entity = match load? {
                    Load::Found(entity) => entity,
                    Load::Gone => {
                        // The row vanished without a delete being queued: an out-of-band change.
                        let _ = self
                            .store
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove_outbox(row.seq);
                        self.reconcile_needed = true;
                        return None;
                    }
                    // Scripts are files: the app thread keeps their projections in a shared cache.
                    Load::NotStored => self
                        .scripts
                        .as_ref()
                        .and_then(|cache| cache.lock().ok()?.get(&row.sync_id).cloned())?,
                };
                if entity.payload.len() > MAX_PAYLOAD_BYTES {
                    self.block(row.seq, "too large to sync");
                    return None;
                }
                let revision = meta.as_ref().map_or(1, |m| m.revision.max(1)) as u64;
                let hash = entity.hash.clone();
                Some((
                    StagedChange {
                        outbox_seq: row.seq,
                        name: name.clone(),
                        op: StagedOp::Save {
                            record: SyncRecord {
                                name,
                                schema_version: SCHEMA_VERSION,
                                revision,
                                origin_device: device.to_string(),
                                modified_ms: row.queued_ms,
                                payload: entity.payload,
                            },
                            system_fields,
                        },
                    },
                    Some(hash),
                ))
            }
        }
    }

    fn block(&mut self, seq: i64, reason: &str) {
        let _ = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_outbox_state(seq, OutboxState::Blocked, Some(reason));
    }

    // MARK: Events

    pub fn handle(&mut self, event: TransportEvent) -> Vec<Effect> {
        match event {
            TransportEvent::Ready => {}
            TransportEvent::Account(status) => self.on_account(status),
            TransportEvent::AccountChange(change) => self.on_account_change(change),
            TransportEvent::Fetched { records, deletions } => self.on_fetched(records, deletions),
            TransportEvent::State(bytes) => self.on_state(bytes),
            TransportEvent::Sent {
                saved,
                deleted,
                failed,
            } => self.on_sent(saved, deleted, failed),
            TransportEvent::Zone(event) => self.on_zone(event),
            TransportEvent::FetchComplete => self.on_fetch_complete(),
            TransportEvent::ZoneDeleted => {
                if self.cloud_delete == CloudDelete::Requested {
                    self.cloud_delete = CloudDelete::Confirmed;
                }
            }
            TransportEvent::Network { available } => {
                self.network_up = available;
                if available {
                    self.last_error = self
                        .last_error
                        .take()
                        .filter(|(kind, _)| *kind != ErrorKind::Network);
                    self.request_fetch();
                }
            }
            TransportEvent::Error { kind, message } => self.on_error(kind, message),
        }
        self.pump();
        self.take_effects()
    }

    fn on_account(&mut self, status: AccountStatus) {
        match status {
            AccountStatus::Available { user_hash } => self.on_available(user_hash),
            AccountStatus::NoAccount
            | AccountStatus::Restricted
            | AccountStatus::CouldNotDetermine
            | AccountStatus::TemporarilyUnavailable => self.account = Account::Unavailable,
        }
    }

    fn on_account_change(&mut self, change: AccountChange) {
        // The engine forgets what it had staged on any account change; the outbox is the record.
        self.inflight.clear();
        self.fetch_in_flight = false;
        match change {
            AccountChange::SignOut => self.account = Account::Unavailable,
            AccountChange::SignIn { user_hash } | AccountChange::Switch { user_hash } => {
                self.on_available(user_hash)
            }
        }
    }

    fn on_available(&mut self, user_hash: String) {
        self.account = Account::Available(user_hash.clone());
        match self.state().and_then(|s| s.account_hash) {
            None => self.effects.push(Effect::NeedsBinding { user_hash }),
            Some(bound) if bound == user_hash => {
                self.transport.send(TransportCommand::EnsureZone);
                self.request_fetch();
            }
            Some(_) => {
                self.pause = Some(PauseReason::AccountChanged);
                self.effects.push(Effect::AccountMismatch { user_hash });
            }
        }
    }

    fn on_zone(&mut self, event: ZoneEvent) {
        match event {
            // Our own requested deletion surfacing as a zone change is not another Mac's doing.
            ZoneEvent::Deleted | ZoneEvent::Purged if self.cloud_delete != CloudDelete::Idle => {
                self.cloud_delete = CloudDelete::Confirmed;
            }
            ZoneEvent::Deleted | ZoneEvent::Purged => {
                self.pause = Some(PauseReason::RemoteDataDeleted);
            }
            ZoneEvent::EncryptedDataReset => {
                let store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
                let _ = store.forget_cloud_state();
                let _ = store.reconcile_stored();
                drop(store);
                self.inflight.clear();
            }
        }
    }

    fn on_error(&mut self, kind: ErrorKind, message: String) {
        match classify(kind, 0, 0, None) {
            Disposition::WaitForAccount => self.account = Account::Unavailable,
            Disposition::StaleToken => self.restart_from_scratch(),
            Disposition::EnsureZone => self.transport.send(TransportCommand::EnsureZone),
            _ => self.last_error = Some((kind, message)),
        }
        self.fetch_in_flight = false;
    }

    /// Drops the checkpoint and fetches everything again.
    fn restart_from_scratch(&mut self) {
        let _ = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .save_engine_state(None, None, self.now());
        self.pending_state = None;
        self.initial_fetch_done = false;
        self.fetch_in_flight = false;
        if self.state().is_some_and(|s| s.initial_sync_done) {
            self.stale_seen = Some(HashSet::new());
        }
        if let Some(state) = self.state() {
            self.transport.send(TransportCommand::Start {
                zone: state.zone_name,
                engine_state: None,
            });
            self.request_fetch();
        }
    }

    // MARK: Fetch

    fn on_fetched(&mut self, raw: Vec<RawRecord>, deletions: Vec<String>) {
        self.fetch_in_flight = false;
        if let Some(seen) = self.stale_seen.as_mut() {
            seen.extend(raw.iter().map(|record| record.name.clone()));
        }
        let now = self.now();
        let mut records = Vec::new();
        for raw in raw {
            if let Some(accepted) = self.admit(raw, now) {
                records.push(accepted);
            }
        }
        let deletions: Vec<RecordName> = deletions
            .iter()
            .filter_map(|name| RecordName::parse(name))
            .collect();
        if records.is_empty() && deletions.is_empty() {
            return;
        }
        // Before the first-time merge nothing is applied: the records wait in the inbox, durably,
        // until the user chooses how to combine iCloud's data with this Mac's.
        if !self.state().is_some_and(|s| s.initial_sync_done) {
            let store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
            for record in &records {
                let _ = store.put_inbox(
                    &InboxRecord {
                        record_name: record.record.name.render(),
                        entity_type: record.record.name.kind.prefix().to_string(),
                        schema_version: record.record.schema_version,
                        reason: "initial_pending".to_string(),
                        payload: record.record.payload.clone(),
                        system_fields: Some(record.system_fields.clone()),
                        revision: record.record.revision,
                        origin_device: record.record.origin_device.clone(),
                        modified_ms: record.record.modified_ms,
                    },
                    now,
                );
            }
            return;
        }
        let id = self.next_batch;
        self.next_batch += 1;
        self.unapplied.insert(id);
        self.effects.push(Effect::RemoteChanges(RemoteBatch {
            id,
            records,
            deletions,
        }));
    }

    /// Validates one fetched record. Anything not applied is kept in the inbox, never dropped.
    fn admit(&mut self, raw: RawRecord, now: i64) -> Option<RemoteRecord> {
        let keep = |reason: &str, raw: &RawRecord, this: &Self| {
            let entity_type = raw.name.split('.').next().unwrap_or_default().to_string();
            let _ = this
                .store
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .put_inbox(
                    &InboxRecord {
                        record_name: raw.name.clone(),
                        entity_type,
                        schema_version: raw.schema_version,
                        reason: reason.to_string(),
                        payload: raw.payload.clone(),
                        system_fields: Some(raw.system_fields.clone()),
                        revision: raw.revision,
                        origin_device: raw.origin_device.clone(),
                        modified_ms: raw.modified_ms,
                    },
                    now,
                );
        };
        let (record, system_fields) = match raw.clone().into_record() {
            Ok(parsed) => parsed,
            Err(unknown) => {
                keep("unknown_type", &unknown, self);
                return None;
            }
        };
        match validate(&record) {
            Verdict::NewerSchema => {
                keep("newer_schema", &raw, self);
                None
            }
            Verdict::Reject(_) => {
                keep("invalid", &raw, self);
                None
            }
            Verdict::Accept(value) => {
                // Our own upload coming back, or a duplicate delivery: already what we hold.
                let known = self
                    .store
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .entity_meta(record.name.kind, &record.name.id)
                    .ok()
                    .flatten();
                if known.is_some_and(|meta| {
                    !meta.deleted
                        && meta.synced_hash.as_deref() == Some(sha256_hex(&record.payload).as_str())
                }) {
                    return None;
                }
                Some(RemoteRecord {
                    record,
                    value,
                    system_fields,
                })
            }
        }
    }

    /// A catch-up from scratch ran to the end. What iCloud no longer lists was deleted while this
    /// Mac was away; remove it here too, unless it was edited since (then the edit wins and it is
    /// uploaded again), or unless so much is missing that something is probably wrong.
    fn on_fetch_complete(&mut self) {
        let Some(seen) = self.stale_seen.take() else {
            return;
        };
        let known = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .synced_entities()
            .unwrap_or_default();
        let missing: Vec<RecordName> = known
            .iter()
            .filter_map(|(kind, id)| RecordName::new(*kind, id.clone()))
            .filter(|name| !seen.contains(&name.render()))
            .collect();
        if missing.is_empty() {
            return;
        }
        if missing.len() > STALE_DELETE_LIMIT && missing.len() * 4 > known.len() {
            self.last_error = Some((
                ErrorKind::Unknown,
                format!(
                    "iCloud no longer has {} items that are still on this Mac. They were kept.",
                    missing.len()
                ),
            ));
            return;
        }
        let id = self.next_batch;
        self.next_batch += 1;
        self.unapplied.insert(id);
        self.effects.push(Effect::RemoteChanges(RemoteBatch {
            id,
            records: Vec::new(),
            deletions: missing,
        }));
    }

    fn on_state(&mut self, bytes: Vec<u8>) {
        self.fetch_in_flight = false;
        self.pending_state = Some(bytes);
        self.flush_state();
    }

    /// The host applied a remote batch locally.
    pub fn remote_applied(&mut self, batch_id: u64) {
        self.unapplied.remove(&batch_id);
        self.flush_state();
        self.pump();
    }

    /// Saves the engine checkpoint, but only once everything it covers has been applied.
    fn flush_state(&mut self) {
        if !self.unapplied.is_empty() {
            return;
        }
        let Some(bytes) = self.pending_state.take() else {
            return;
        };
        let account = match &self.account {
            Account::Available(hash) => Some(hash.clone()),
            _ => None,
        };
        let _ = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .save_engine_state(Some(&bytes), account.as_deref(), self.now());
        self.initial_fetch_done = true;
    }

    // MARK: Upload results

    fn on_sent(
        &mut self,
        saved: Vec<super::transport::SentSaved>,
        deleted: Vec<super::transport::SentDeleted>,
        failed: Vec<SendFailure>,
    ) {
        let now = self.now();
        for done in saved {
            let entry = self.inflight.remove(&done.outbox_seq);
            let Some(name) = RecordName::parse(&done.name) else {
                continue;
            };
            let store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
            match entry.and_then(|entry| entry.hash) {
                Some(hash) => {
                    let _ = store.ack_saved(
                        done.outbox_seq,
                        name.kind,
                        &name.id,
                        &hash,
                        &done.system_fields,
                        now,
                    );
                }
                // Unknown what was sent (the entry was forgotten across an account change): keep
                // the new change tag and let the entity upload again; that is harmless.
                None => {
                    let _ = store.set_system_fields(name.kind, &name.id, &done.system_fields);
                }
            }
            let _ = store.set_last_send(now);
            drop(store);
            self.last_error = None;
        }
        for done in deleted {
            self.inflight.remove(&done.outbox_seq);
            let Some(name) = RecordName::parse(&done.name) else {
                continue;
            };
            let store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = store.ack_deleted(done.outbox_seq, name.kind, &name.id);
            let _ = store.set_last_send(now);
            drop(store);
            self.last_error = None;
        }
        for failure in failed {
            self.on_failure(failure, now);
        }
    }

    fn on_failure(&mut self, failure: SendFailure, now: i64) {
        let entry = self.inflight.remove(&failure.outbox_seq);
        let row = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .outbox()
            .ok()
            .and_then(|rows| rows.into_iter().find(|r| r.seq == failure.outbox_seq));
        // The entry was superseded or already cleared; the next pass handles the new one.
        let Some(row) = row else { return };
        let attempts = row.attempts;
        match classify(failure.kind, attempts, row.seq, failure.retry_after_s) {
            Disposition::RetryAfter(delay) => {
                self.retry_later(row.seq, now + delay, &failure);
            }
            Disposition::Persistent(delay) => {
                self.retry_later(row.seq, now + delay, &failure);
            }
            Disposition::EnsureZone => {
                self.transport.send(TransportCommand::EnsureZone);
                let delay = super::outbox::backoff_ms(attempts, row.seq, None);
                self.retry_later(row.seq, now + delay, &failure);
            }
            Disposition::Recreate => {
                let store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
                let _ = store.clear_system_fields(row.kind, &row.sync_id);
                let _ = store.retry_now(row.seq);
            }
            Disposition::Block => self.block(row.seq, &failure.message),
            Disposition::WaitForAccount => self.account = Account::Unavailable,
            Disposition::StaleToken => self.restart_from_scratch(),
            Disposition::Conflict => self.on_conflict(&row, entry, failure),
        }
    }

    fn retry_later(&mut self, seq: i64, at_ms: i64, failure: &SendFailure) {
        let _ = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fail_outbox(seq, at_ms, &failure.message);
        self.last_error = Some((failure.kind, failure.message.clone()));
    }

    /// The cloud changed this record since we last saw it. Stage 5 resolves last-writer-wins;
    /// per-entity merges replace `resolve` later.
    fn on_conflict(&mut self, row: &OutboxRow, entry: Option<Inflight>, failure: SendFailure) {
        let now = self.now();
        let Some(server) = failure.server_record.clone() else {
            self.retry_later(row.seq, now + 1_000, &failure);
            return;
        };
        let (record, fields) = match server.clone().into_record() {
            Ok(parsed) => parsed,
            Err(_) => {
                self.retry_later(row.seq, now + 60_000, &failure);
                return;
            }
        };
        // Our upload had in fact landed (a crash before the acknowledgement): treat as accepted.
        if let Some(hash) = entry.and_then(|entry| entry.hash)
            && sha256_hex(&record.payload) == hash
        {
            let _ = self
                .store
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ack_saved(row.seq, row.kind, &row.sync_id, &hash, &fields, now);
            return;
        }
        let meta = self
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entity_meta(row.kind, &row.sync_id)
            .ok()
            .flatten();
        let device = self.state().map(|s| s.device_id).unwrap_or_default();
        let local_wins = row.op == OutboxOp::Upsert
            && meta.as_ref().is_some_and(|meta| {
                (meta.revision.max(1) as u64, row.queued_ms, device.as_str())
                    > (
                        record.revision,
                        record.modified_ms,
                        record.origin_device.as_str(),
                    )
            });
        let store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        if local_wins {
            // Save over the cloud's version: take its change tag and go again.
            let _ = store.set_system_fields(row.kind, &row.sync_id, &fields);
            let _ = store.retry_now(row.seq);
            return;
        }
        // The cloud's version wins (and an edit always beats a local delete).
        let _ = store.remove_outbox(row.seq);
        drop(store);
        if let Verdict::Accept(value) = validate(&record) {
            let id = self.next_batch;
            self.next_batch += 1;
            self.unapplied.insert(id);
            self.effects.push(Effect::RemoteChanges(RemoteBatch {
                id,
                records: vec![RemoteRecord {
                    record,
                    value,
                    system_fields: fields,
                }],
                deletions: Vec::new(),
            }));
        }
    }

    // MARK: Status

    pub fn status(&self) -> Status {
        let store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        let state = store.sync_state().ok().flatten();
        let outbox = store.outbox().unwrap_or_default();
        let conflicts = store.open_conflict_count().unwrap_or(0);
        drop(store);
        let count = |wanted: OutboxState| outbox.iter().filter(|r| r.state == wanted).count();
        let pending = count(OutboxState::Pending);
        let held = count(OutboxState::Held);
        let blocked = count(OutboxState::Blocked);
        let phase = self.phase_with(state.as_ref(), pending, held + blocked + conflicts);
        Status {
            phase,
            pending,
            held,
            blocked,
            conflicts,
            last_send_ms: state.as_ref().and_then(|s| s.last_send_ms),
            last_fetch_ms: state.as_ref().and_then(|s| s.last_fetch_ms),
            last_error: self.last_error.as_ref().map(|(_, message)| message.clone()),
            cloud_deleted: self.cloud_delete == CloudDelete::Confirmed,
        }
    }

    pub fn phase(&self) -> Phase {
        self.status().phase
    }

    fn phase_with(&self, state: Option<&SyncState>, pending: usize, attention: usize) -> Phase {
        let Some(state) = state.filter(|s| s.enabled) else {
            return Phase::Off;
        };
        if let Some(reason) = self.pause {
            return Phase::Paused(reason);
        }
        match &self.account {
            Account::Unknown => return Phase::CheckingAccount,
            Account::Unavailable => return Phase::WaitingForAccount,
            Account::Available(hash) => {
                if state.account_hash.is_none() {
                    return Phase::NeedsSetup;
                }
                if state.account_hash.as_deref() != Some(hash.as_str()) {
                    return Phase::Paused(PauseReason::AccountChanged);
                }
            }
        }
        if !self.network_up || matches!(self.last_error, Some((ErrorKind::Network, _))) {
            return Phase::WaitingForNetwork;
        }
        if let Some((kind, _)) = &self.last_error
            && !is_transient(*kind)
        {
            return Phase::Error(*kind);
        }
        if self.cloud_delete == CloudDelete::Requested {
            return Phase::Syncing;
        }
        if !state.initial_sync_done {
            return if self.fetch_in_flight {
                Phase::Syncing
            } else if self.initial_fetch_done {
                Phase::AwaitingJoin
            } else {
                Phase::Ready
            };
        }
        if pending > 0
            || !self.inflight.is_empty()
            || self.fetch_in_flight
            || !self.unapplied.is_empty()
        {
            return Phase::Syncing;
        }
        if attention > 0 {
            return Phase::NeedsAttention;
        }
        if self.initial_fetch_done {
            Phase::UpToDate
        } else {
            Phase::Ready
        }
    }
}
