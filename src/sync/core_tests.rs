//! Core behavior against the in-memory cloud: upload, acknowledgement, retry, conflicts, account
//! gating and the status invariant. Each `Mac` is a real in-memory store plus a real `SyncCore`.

use super::core::{Effect, PauseReason, Phase, RemoteBatch, SyncCore};
use super::mock::{Fault, MockCloud, MockTransport};
use super::policy::EntityKind;
use super::project::project_row;
use super::record::{SCHEMA_VERSION, sha256_hex};
use super::transport::{
    ErrorKind, RawRecord, SentSaved, TransportCommand, TransportEvent, ZoneEvent,
};
use crate::store::sync::{OutboxOp, OutboxState};
use crate::store::{Document, SharedStore, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct Raw(pub(super) Value);

impl Document for Raw {
    fn document_id(&self) -> String {
        self.0["id"].to_string()
    }
}

pub(super) fn list(id: u64, name: &str) -> Raw {
    Raw(json!({"id": id, "name": name, "entries": [], "is_favorites": false}))
}

/// An in-memory script library standing in for the files `ScriptStore` manages.
#[derive(Default)]
pub(super) struct FakeLibrary {
    pub(super) scripts: std::collections::BTreeMap<String, super::project::ScriptSnapshot>,
    pub(super) unsaved: std::collections::HashSet<String>,
    pub(super) pristine_demos: std::collections::HashSet<String>,
    pub(super) fail_listing: bool,
    seed: u128,
    counter: u128,
}

impl FakeLibrary {
    fn named(&self, name: &str, except: Option<&str>) -> bool {
        self.scripts
            .values()
            .any(|s| s.name.eq_ignore_ascii_case(name) && Some(s.id.as_str()) != except)
    }

    fn unique(&self, name: &str, except: Option<&str>) -> String {
        let mut candidate = name.to_string();
        let mut n = 2;
        while self.named(&candidate, except) {
            candidate = format!("{name} {n}");
            n += 1;
        }
        candidate
    }

    pub(super) fn add(&mut self, name: &str, source: &str) -> String {
        let id = self.fresh_id_value();
        let name = self.unique(name, None);
        self.scripts.insert(
            id.clone(),
            super::project::ScriptSnapshot {
                id: id.clone(),
                name,
                source: source.into(),
                is_favorite: false,
            },
        );
        id
    }

    pub(super) fn set_source(&mut self, id: &str, source: &str) {
        self.scripts.get_mut(id).unwrap().source = source.into();
    }

    pub(super) fn source(&self, id: &str) -> String {
        self.scripts[id].source.clone()
    }

    fn fresh_id_value(&mut self) -> String {
        self.counter += 1;
        format!("{:032x}", (self.seed << 64) | self.counter)
    }

    pub(super) fn sorted(&self) -> Vec<(String, String)> {
        let mut all: Vec<_> = self
            .scripts
            .values()
            .map(|s| (s.name.clone(), s.source.clone()))
            .collect();
        all.sort();
        all
    }
}

impl super::scripts::ScriptSink for FakeLibrary {
    fn all(&mut self) -> Result<Vec<super::project::ScriptSnapshot>, String> {
        if self.fail_listing {
            return Err("disk unavailable".into());
        }
        Ok(self
            .scripts
            .values()
            .filter(|s| !self.pristine_demos.contains(&s.id))
            .cloned()
            .collect())
    }
    fn get(&mut self, id: &str) -> Result<Option<super::project::ScriptSnapshot>, String> {
        Ok(self.scripts.get(id).cloned())
    }
    fn find_by_name(&mut self, name: &str) -> Result<Option<super::scripts::NamedScript>, String> {
        Ok(self
            .scripts
            .values()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .map(|s| super::scripts::NamedScript {
                id: s.id.clone(),
                pristine_demo: self.pristine_demos.contains(&s.id),
            }))
    }
    fn has_unsaved_work(&mut self, id: &str) -> bool {
        self.unsaved.contains(id)
    }
    fn adopt(&mut self, id: &str, name: &str, source: &str, favorite: bool) -> Result<(), String> {
        let name = self.unique(name, None);
        self.scripts.insert(
            id.into(),
            super::project::ScriptSnapshot {
                id: id.into(),
                name,
                source: source.into(),
                is_favorite: favorite,
            },
        );
        Ok(())
    }
    fn update(&mut self, id: &str, name: &str, source: &str, favorite: bool) -> Result<(), String> {
        let name = self.unique(name, Some(id));
        let script = self.scripts.get_mut(id).ok_or("missing")?;
        script.name = name;
        script.source = source.into();
        script.is_favorite = favorite;
        Ok(())
    }
    fn delete(&mut self, id: &str) -> Result<(), String> {
        self.scripts.remove(id);
        Ok(())
    }
    fn fresh_id(&mut self) -> String {
        self.fresh_id_value()
    }
}

pub(super) struct Mac {
    pub(super) store: SharedStore,
    pub(super) core: SyncCore<MockTransport>,
    pub(super) now: Arc<AtomicI64>,
    pub(super) batches: Vec<RemoteBatch>,
    pub(super) effects: Vec<Effect>,
    pub(super) hold_applies: bool,
    pub(super) unapplied: Vec<u64>,
    /// Apply fetched changes to the store for real, as the app does.
    pub(super) apply: bool,
    pub(super) reports: Vec<super::apply::ApplyReport>,
    pub(super) library: FakeLibrary,
    pub(super) script_cache: super::scripts::ScriptCache,
}

pub(super) fn mac(cloud: &MockCloud, name: &str, user: &str, bound: Option<&str>) -> Mac {
    mac_with(cloud, name, user, bound, true)
}

pub(super) fn mac_with(
    cloud: &MockCloud,
    name: &str,
    user: &str,
    bound: Option<&str>,
    initial_done: bool,
) -> Mac {
    let store = Store::open_in_memory().unwrap();
    let mut state = store.ensure_sync_state(name, "hw").unwrap();
    state.enabled = true;
    state.account_hash = bound.map(String::from);
    state.initial_sync_done = initial_done;
    store.save_sync_state(&state).unwrap();
    let store: SharedStore = Arc::new(Mutex::new(store));
    // The store stamps rows with the wall clock, so the core's clock starts there too.
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(10_000_000, |elapsed| elapsed.as_millis() as i64);
    let now = Arc::new(AtomicI64::new(started));
    let clock = {
        let now = now.clone();
        Box::new(move || now.load(Ordering::Relaxed))
    };
    let mut core = SyncCore::new(store.clone(), MockTransport::new(cloud, name, user), clock);
    let script_cache = super::scripts::ScriptCache::default();
    core.set_script_cache(script_cache.clone());
    let seed = name.bytes().fold(7u128, |acc, b| {
        acc.wrapping_mul(131).wrapping_add(b as u128)
    }) & 0xffff_ffff;
    Mac {
        store,
        core,
        now,
        batches: Vec::new(),
        effects: Vec::new(),
        hold_applies: false,
        unapplied: Vec::new(),
        apply: false,
        reports: Vec::new(),
        library: FakeLibrary {
            seed,
            ..Default::default()
        },
        script_cache,
    }
}

impl Mac {
    pub(super) fn lists(&self, rows: &[(u64, &str)]) {
        let docs: Vec<_> = rows.iter().map(|(id, name)| list(*id, name)).collect();
        self.store
            .lock()
            .unwrap()
            .replace_documents("watchlist", &docs)
            .unwrap();
    }

    pub(super) fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::Relaxed);
    }

    pub(super) fn start(&mut self) {
        self.core.start();
        self.run();
    }

    pub(super) fn deliver(&mut self, event: TransportEvent) {
        let effects = self.core.handle(event);
        for effect in effects {
            if let Effect::RemoteChanges(batch) = &effect {
                self.batches.push(batch.clone());
                if self.hold_applies {
                    self.unapplied.push(batch.id);
                } else {
                    if self.apply {
                        let report = self.apply_to_store(batch);
                        self.reports.push(report);
                    }
                    self.core.remote_applied(batch.id);
                }
            }
            self.effects.push(effect);
        }
        self.check_invariant();
    }

    /// Delivers events until the transport is quiet, ticking first.
    pub(super) fn run(&mut self) {
        self.core.tick();
        for _ in 0..200 {
            let events = self.core.transport_mut().take_events();
            if events.is_empty() {
                return;
            }
            for event in events {
                self.deliver(event);
            }
        }
        panic!("sync did not settle");
    }

    pub(super) fn apply_to_store(&mut self, batch: &RemoteBatch) -> super::apply::ApplyReport {
        let state = self.store.lock().unwrap().sync_state().unwrap().unwrap();
        let ctx = super::apply::ApplyCtx {
            device_id: state.device_id,
            device_name: state.device_name,
            now_ms: self.now.load(Ordering::Relaxed),
            date: "2026-10-20".into(),
        };
        let store = self.store.lock().unwrap();
        super::apply::apply_batch(&store, &mut self.library, batch, &ctx).unwrap()
    }

    /// Re-reads the script library and queues what changed, as the app does on a timer.
    pub(super) fn scan_scripts(&mut self) {
        let store = self.store.lock().unwrap();
        super::scripts::scan_scripts(&store, &mut self.library, &self.script_cache).unwrap();
    }

    pub(super) fn apply_held(&mut self) {
        for id in std::mem::take(&mut self.unapplied) {
            self.core.remote_applied(id);
        }
        self.run();
    }

    /// The status is never "up to date" while anything waits to upload or is in flight.
    pub(super) fn check_invariant(&self) {
        let status = self.core.status();
        if status.phase == Phase::UpToDate {
            assert_eq!(status.pending, 0, "UpToDate with pending changes");
            assert!(
                self.store
                    .lock()
                    .unwrap()
                    .outbox()
                    .unwrap()
                    .iter()
                    .all(|r| r.state != OutboxState::Pending),
                "UpToDate with an outbox entry"
            );
        }
    }

    pub(super) fn outbox(&self) -> Vec<crate::store::sync::OutboxRow> {
        self.store.lock().unwrap().outbox().unwrap()
    }

    pub(super) fn phase(&self) -> Phase {
        self.core.status().phase
    }
}

pub(super) fn raw(
    name: &str,
    payload: &Value,
    revision: u64,
    modified_ms: i64,
    device: &str,
) -> RawRecord {
    RawRecord {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        revision,
        origin_device: device.to_string(),
        modified_ms,
        payload: serde_json::to_vec(payload).unwrap(),
        system_fields: Vec::new(),
    }
}

pub(super) fn canonical(id: u64, name: &str) -> Vec<u8> {
    project_row("watchlist", &id.to_string(), &list(id, name).0.to_string())[0]
        .payload
        .clone()
}

pub(super) fn payload_name(cloud: &MockCloud, record: &str) -> Option<String> {
    let raw = cloud.record(record)?;
    let value: Value = serde_json::from_slice(&raw.payload).ok()?;
    value["name"].as_str().map(String::from)
}

// MARK: Upload

#[test]
fn a_local_change_uploads_and_the_outbox_clears() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    assert_eq!(cloud.names(), ["watchlist.1"]);
    assert!(a.outbox().is_empty());
    let meta = a
        .store
        .lock()
        .unwrap()
        .entity_meta(EntityKind::Watchlist, "1")
        .unwrap()
        .unwrap();
    assert!(meta.synced_hash.is_some());
    assert!(meta.system_fields.is_some());
    assert_eq!(a.phase(), Phase::UpToDate);
    assert!(a.core.status().last_send_ms.is_some());
}

#[test]
fn the_uploaded_record_carries_identity_revision_and_payload() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    let record = cloud.record("watchlist.1").unwrap();
    assert_eq!(record.revision, 1);
    assert_eq!(record.schema_version, SCHEMA_VERSION);
    assert_eq!(record.payload, canonical(1, "Tech"));
    let device = a
        .store
        .lock()
        .unwrap()
        .sync_state()
        .unwrap()
        .unwrap()
        .device_id;
    assert_eq!(record.origin_device, device);
}

#[test]
fn nothing_is_staged_until_the_account_is_known() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.core.start();
    assert_eq!(a.phase(), Phase::CheckingAccount);
    assert!(cloud.names().is_empty());
}

#[test]
fn an_unbound_database_asks_for_setup_and_uploads_nothing() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", None);
    a.lists(&[(1, "Tech")]);
    a.start();
    assert!(
        a.effects
            .iter()
            .any(|e| matches!(e, Effect::NeedsBinding { user_hash } if user_hash == "acct"))
    );
    assert_eq!(a.phase(), Phase::NeedsSetup);
    assert!(cloud.names().is_empty());
    a.advance(60_000);
    a.run();
    assert!(cloud.names().is_empty());
}

#[test]
fn a_different_account_pauses_and_uploads_nothing() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "account-b", Some("account-a"));
    a.lists(&[(1, "Private")]);
    a.start();
    assert!(
        a.effects
            .iter()
            .any(|e| matches!(e, Effect::AccountMismatch { .. }))
    );
    assert_eq!(a.phase(), Phase::Paused(PauseReason::AccountChanged));
    a.advance(120_000);
    a.run();
    assert!(cloud.names().is_empty());
    assert_eq!(a.outbox().len(), 1);
    assert_eq!(cloud.fetches(), 0);
}

#[test]
fn nothing_uploads_before_the_first_merge_finishes() {
    let cloud = MockCloud::new();
    let mut a = mac_with(&cloud, "A", "acct", Some("acct"), false);
    a.lists(&[(1, "Tech")]);
    a.start();
    assert!(cloud.names().is_empty());
    assert!(cloud.fetches() > 0);
    assert_eq!(a.outbox().len(), 1);
}

#[test]
fn disabled_sync_does_nothing() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    {
        let store = a.store.lock().unwrap();
        let mut state = store.sync_state().unwrap().unwrap();
        state.enabled = false;
        store.save_sync_state(&state).unwrap();
    }
    a.start();
    assert_eq!(a.phase(), Phase::Off);
    assert!(a.core.transport_mut().commands.is_empty());
}

#[test]
fn an_edit_during_an_upload_survives_the_acknowledgement() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "v1")]);
    a.core.start();
    // Deliver up to and including the account event: this stages and sends v1.
    let events = a.core.transport_mut().take_events();
    for event in events {
        a.deliver(event);
    }
    assert_eq!(a.phase(), Phase::Syncing);
    // The user edits again before the acknowledgement arrives.
    a.lists(&[(1, "v2")]);
    a.run();
    assert_eq!(payload_name(&cloud, "watchlist.1").as_deref(), Some("v2"));
    assert!(a.outbox().is_empty());
    assert_eq!(cloud.save_attempts(), 2);
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn a_duplicate_acknowledgement_is_harmless() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    let fields = a
        .store
        .lock()
        .unwrap()
        .entity_meta(EntityKind::Watchlist, "1")
        .unwrap()
        .unwrap()
        .system_fields
        .unwrap();
    a.deliver(TransportEvent::Sent {
        saved: vec![SentSaved {
            outbox_seq: 1,
            name: "watchlist.1".into(),
            system_fields: fields,
        }],
        deleted: vec![],
        failed: vec![],
    });
    assert!(a.outbox().is_empty());
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn a_failure_for_an_entry_that_no_longer_exists_is_ignored() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    a.deliver(TransportEvent::Sent {
        saved: vec![],
        deleted: vec![],
        failed: vec![super::transport::SendFailure {
            outbox_seq: 999,
            name: "watchlist.1".into(),
            kind: ErrorKind::Network,
            retry_after_s: None,
            message: "late".into(),
            server_record: None,
        }],
    });
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn large_backlogs_are_staged_in_bounded_batches() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    let rows: Vec<_> = (1..=250).map(|id| (id, "x")).collect();
    a.lists(&rows);
    a.start();
    assert_eq!(cloud.names().len(), 250);
    let stages: Vec<_> = a
        .core
        .transport_mut()
        .commands
        .iter()
        .filter_map(|c| match c {
            TransportCommand::Stage { changes, .. } => Some(changes.len()),
            _ => None,
        })
        .collect();
    assert!(stages.iter().all(|n| *n <= 100), "{stages:?}");
    assert!(stages.len() >= 3);
}

// MARK: Deletes

#[test]
fn a_delete_uploads_and_leaves_a_tombstone() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech"), (2, "Other")]);
    a.start();
    a.lists(&[(2, "Other")]);
    a.run();
    assert_eq!(cloud.names(), ["watchlist.2"]);
    assert_eq!(cloud.delete_attempts(), 1);
    assert!(a.outbox().is_empty());
    let meta = a
        .store
        .lock()
        .unwrap()
        .entity_meta(EntityKind::Watchlist, "1")
        .unwrap()
        .unwrap();
    assert!(meta.deleted);
    assert!(meta.synced_hash.is_none());
}

#[test]
fn deleting_a_record_the_cloud_already_lost_still_succeeds() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech"), (2, "Other")]);
    a.start();
    cloud.delete_from_elsewhere("watchlist.1", "B");
    a.lists(&[(2, "Other")]);
    a.run();
    assert!(a.outbox().is_empty());
}

// MARK: Failures and retry

#[test]
fn a_network_failure_backs_off_and_then_succeeds() {
    let cloud = MockCloud::new();
    cloud.inject(Fault::Send(ErrorKind::Network, None));
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    assert!(cloud.names().is_empty());
    let row = &a.outbox()[0];
    assert_eq!(row.attempts, 1);
    assert!(row.next_attempt_ms.unwrap() > a.now.load(Ordering::Relaxed) - 1);
    assert_eq!(a.phase(), Phase::WaitingForNetwork);
    // No tight loop: ticking again before the retry time sends nothing.
    a.run();
    a.run();
    assert!(cloud.names().is_empty());
    a.advance(2_000);
    a.run();
    assert_eq!(cloud.names(), ["watchlist.1"]);
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn the_servers_retry_after_is_respected() {
    let cloud = MockCloud::new();
    cloud.inject(Fault::Send(ErrorKind::RateLimited, Some(120)));
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    a.advance(119_000);
    a.run();
    assert!(cloud.names().is_empty());
    a.advance(2_000);
    a.run();
    assert_eq!(cloud.names(), ["watchlist.1"]);
}

#[test]
fn repeated_failures_back_off_further() {
    let cloud = MockCloud::new();
    for _ in 0..3 {
        cloud.inject(Fault::Send(ErrorKind::Unknown, None));
    }
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    let mut delays = Vec::new();
    for _ in 0..2 {
        let due = a.outbox()[0].next_attempt_ms.unwrap();
        delays.push(due - a.now.load(Ordering::Relaxed));
        a.advance(due - a.now.load(Ordering::Relaxed) + 1);
        a.run();
    }
    assert!(delays[1] > delays[0], "{delays:?}");
}

#[test]
fn a_run_of_unexplained_failures_is_shown_and_clears_when_an_upload_lands() {
    let cloud = MockCloud::new();
    for _ in 0..3 {
        cloud.inject(Fault::Send(ErrorKind::Unknown, None));
    }
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    assert_eq!(a.phase(), Phase::Syncing, "one failure is quiet");
    for expected in [Phase::Syncing, Phase::Error(ErrorKind::Unknown)] {
        let due = a.outbox()[0].next_attempt_ms.unwrap();
        a.advance(due - a.now.load(Ordering::Relaxed) + 1);
        a.run();
        assert_eq!(a.phase(), expected);
    }
    assert!(a.core.status().last_error.is_some());
    let due = a.outbox()[0].next_attempt_ms.unwrap();
    a.advance(due - a.now.load(Ordering::Relaxed) + 1);
    a.run();
    assert_eq!(cloud.names(), ["watchlist.1"]);
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn a_missing_zone_is_created_and_the_upload_retried() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    cloud.delete_zone_contents();
    a.lists(&[(1, "Tech")]);
    a.run();
    assert!(
        a.core
            .transport_mut()
            .commands
            .contains(&TransportCommand::EnsureZone)
    );
    a.advance(5_000);
    a.run();
    assert_eq!(cloud.names(), ["watchlist.1"]);
}

#[test]
fn quota_errors_are_shown_and_retried_slowly() {
    let cloud = MockCloud::new();
    cloud.inject(Fault::Send(ErrorKind::Quota, None));
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    assert_eq!(a.phase(), Phase::Error(ErrorKind::Quota));
    let started = a.now.load(Ordering::Relaxed);
    assert!(a.outbox()[0].next_attempt_ms.unwrap() >= started + 3_600_000);
    a.advance(1_800_000);
    a.run();
    assert!(cloud.names().is_empty());
    a.advance(1_900_000);
    a.run();
    assert_eq!(cloud.names(), ["watchlist.1"]);
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn a_record_too_large_to_sync_is_blocked_and_needs_attention() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    let huge = "x".repeat(1_000_000);
    a.lists(&[(1, &huge)]);
    a.start();
    assert!(cloud.names().is_empty());
    assert_eq!(a.outbox()[0].state, OutboxState::Blocked);
    assert_eq!(a.phase(), Phase::NeedsAttention);
}

#[test]
fn a_cloud_side_too_large_error_blocks_the_entry() {
    let cloud = MockCloud::new();
    cloud.inject(Fault::Send(ErrorKind::TooLarge, None));
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    assert_eq!(a.outbox()[0].state, OutboxState::Blocked);
    assert_eq!(a.phase(), Phase::NeedsAttention);
}

#[test]
fn held_deletes_need_attention_and_are_not_sent() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    let rows: Vec<_> = (1..=6).map(|id| (id, "x")).collect();
    a.lists(&rows);
    a.start();
    a.lists(&[]);
    a.run();
    assert_eq!(cloud.names().len(), 6);
    assert!(a.outbox().iter().all(|r| r.state == OutboxState::Held));
    assert_eq!(a.phase(), Phase::NeedsAttention);
}

#[test]
fn one_failed_record_does_not_hold_back_the_others() {
    let cloud = MockCloud::new();
    cloud.inject(Fault::Partial(
        vec!["watchlist.2".into()],
        ErrorKind::Unknown,
    ));
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "a"), (2, "b"), (3, "c")]);
    a.start();
    assert_eq!(cloud.names(), ["watchlist.1", "watchlist.3"]);
    assert_eq!(a.outbox().len(), 1);
    a.advance(5_000);
    a.run();
    assert_eq!(cloud.names().len(), 3);
}

#[test]
fn an_edit_over_a_record_deleted_elsewhere_recreates_it() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "v1")]);
    a.start();
    cloud.delete_from_elsewhere("watchlist.1", "B");
    a.lists(&[(1, "v2")]);
    a.run();
    assert_eq!(payload_name(&cloud, "watchlist.1").as_deref(), Some("v2"));
    assert!(a.outbox().is_empty());
}

// MARK: Conflicts

#[test]
fn an_upload_that_already_landed_counts_as_accepted() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    // The cloud already holds exactly what this Mac is about to send (a crash before the ack).
    cloud.put_from_elsewhere(
        raw(
            "watchlist.1",
            &serde_json::from_slice(&canonical(1, "Tech")).unwrap(),
            1,
            5,
            "A-before-crash",
        ),
        "A",
    );
    a.lists(&[(1, "Tech")]);
    a.run();
    assert!(a.outbox().is_empty());
    assert!(a.batches.is_empty());
    assert_eq!(a.phase(), Phase::UpToDate);
    let meta = a
        .store
        .lock()
        .unwrap()
        .entity_meta(EntityKind::Watchlist, "1")
        .unwrap()
        .unwrap();
    assert_eq!(meta.synced_hash.unwrap(), sha256_hex(&canonical(1, "Tech")));
}

#[test]
fn a_newer_cloud_version_wins_and_goes_to_the_apply_layer() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    let theirs = json!({"id": 1, "name": "Theirs", "entries": [], "is_favorites": false});
    cloud.put_from_elsewhere(raw("watchlist.1", &theirs, 9, i64::MAX, "zzz"), "B");
    a.lists(&[(1, "Mine")]);
    a.run();
    assert!(a.outbox().is_empty());
    assert_eq!(a.batches.len(), 1);
    assert_eq!(a.batches[0].records[0].record.name.render(), "watchlist.1");
    assert_eq!(
        payload_name(&cloud, "watchlist.1").as_deref(),
        Some("Theirs")
    );
}

#[test]
fn a_newer_local_edit_wins_a_conflict_and_overwrites() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    let theirs = json!({"id": 1, "name": "Theirs", "entries": [], "is_favorites": false});
    cloud.put_from_elsewhere(raw("watchlist.1", &theirs, 1, 0, "a-device"), "B");
    a.lists(&[(1, "Mine")]);
    a.run();
    assert_eq!(payload_name(&cloud, "watchlist.1").as_deref(), Some("Mine"));
    assert!(a.outbox().is_empty());
    assert!(a.batches.is_empty());
}

#[test]
fn deletes_wait_for_a_completed_fetch() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.lists(&[]);
    assert_eq!(a.outbox()[0].op, OutboxOp::Delete);
    a.core.start();
    // Handle events one at a time: the account event requests a fetch but its result is pending.
    let events = a.core.transport_mut().take_events();
    for event in events {
        a.deliver(event);
    }
    assert_eq!(
        cloud.delete_attempts(),
        0,
        "a delete must not race an unseen remote edit"
    );
    a.run();
    assert_eq!(cloud.delete_attempts(), 1);
    assert!(a.outbox().is_empty());
}

// MARK: Fetch

#[test]
fn fetched_changes_go_to_the_apply_layer_and_state_waits_for_them() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.hold_applies = true;
    a.start();
    let checkpoint = |a: &Mac| {
        a.store
            .lock()
            .unwrap()
            .sync_state()
            .unwrap()
            .unwrap()
            .engine_state
    };
    let before = checkpoint(&a);
    assert!(before.is_some());
    let theirs = json!({"id": 7, "name": "From B", "entries": [], "is_favorites": false});
    cloud.put_from_elsewhere(raw("watchlist.7", &theirs, 1, 1, "B"), "B");
    a.core.request_sync();
    a.run();
    assert_eq!(a.batches.len(), 1);
    // The engine checkpoint must not move until the batch is applied locally.
    assert_eq!(checkpoint(&a), before);
    assert_eq!(a.phase(), Phase::Syncing);
    a.apply_held();
    let after = checkpoint(&a).unwrap();
    assert_eq!(u64::from_le_bytes(after.try_into().unwrap()), 1);
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn unreadable_remote_records_are_kept_not_applied() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    let mut newer = raw("watchlist.1", &json!({"id": 1}), 1, 1, "B");
    newer.schema_version = SCHEMA_VERSION + 3;
    cloud.put_from_elsewhere(newer, "B");
    cloud.put_from_elsewhere(
        raw(
            "watchlist.2",
            &json!({"id": 99, "name": "x", "entries": []}),
            1,
            1,
            "B",
        ),
        "B",
    );
    cloud.put_from_elsewhere(raw("hologram.3", &json!({}), 1, 1, "B"), "B");
    a.core.request_sync();
    a.run();
    assert!(a.batches.is_empty());
    let reasons: Vec<_> = a
        .store
        .lock()
        .unwrap()
        .inbox()
        .unwrap()
        .into_iter()
        .map(|r| (r.record_name, r.reason))
        .collect();
    assert!(
        reasons.contains(&("watchlist.1".into(), "newer_schema".into())),
        "{reasons:?}"
    );
    assert!(
        reasons.contains(&("watchlist.2".into(), "invalid".into())),
        "{reasons:?}"
    );
    assert!(
        reasons.contains(&("hologram.3".into(), "unknown_type".into())),
        "{reasons:?}"
    );
}

#[test]
fn a_remote_deletion_reaches_the_apply_layer() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    cloud.put_from_elsewhere(
        raw(
            "watchlist.7",
            &json!({"id": 7, "name": "x", "entries": []}),
            1,
            1,
            "B",
        ),
        "B",
    );
    cloud.delete_from_elsewhere("watchlist.7", "B");
    a.core.request_sync();
    a.run();
    assert_eq!(a.batches.len(), 1);
    assert_eq!(a.batches[0].deletions[0].render(), "watchlist.7");
}

#[test]
fn our_own_upload_echoing_back_is_not_reapplied() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.core.transport_mut().echo_own = true;
    a.lists(&[(1, "Tech")]);
    a.start();
    a.core.request_sync();
    a.run();
    assert!(a.batches.is_empty());
}

#[test]
fn sync_now_fetches_without_discarding_pending_work() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    let before = cloud.fetches();
    a.lists(&[(1, "Tech")]);
    a.core.request_sync();
    a.run();
    assert!(cloud.fetches() > before);
    assert_eq!(cloud.names(), ["watchlist.1"]);
}

#[test]
fn the_checkpoint_survives_a_restart_for_the_same_account_only() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    cloud.put_from_elsewhere(
        raw(
            "watchlist.7",
            &json!({"id": 7, "name": "x", "entries": []}),
            1,
            1,
            "B",
        ),
        "B",
    );
    a.core.request_sync();
    a.run();

    let start_state = |store: &SharedStore| {
        let mut core = SyncCore::new(
            store.clone(),
            MockTransport::new(&cloud, "A", "acct"),
            Box::new(|| 0),
        );
        core.start();
        core.transport_mut().commands.iter().find_map(|c| match c {
            TransportCommand::Start { engine_state, .. } => Some(engine_state.clone()),
            _ => None,
        })
    };
    assert!(start_state(&a.store).unwrap().is_some());
    {
        let store = a.store.lock().unwrap();
        let mut state = store.sync_state().unwrap().unwrap();
        state.engine_state_account_hash = Some("somebody-else".into());
        store.save_sync_state(&state).unwrap();
    }
    assert!(start_state(&a.store).unwrap().is_none());
}

// MARK: Account and zone events

#[test]
fn signing_out_pauses_uploads_and_keeps_the_outbox() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    a.core.transport_mut().sign_out();
    a.run();
    assert_eq!(a.phase(), Phase::WaitingForAccount);
    a.lists(&[(1, "Offline edit")]);
    a.run();
    assert!(cloud.names().is_empty());
    assert_eq!(a.outbox().len(), 1);
    a.core.transport_mut().sign_in("acct");
    a.run();
    assert_eq!(cloud.names(), ["watchlist.1"]);
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn switching_accounts_never_uploads_the_old_accounts_data() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "account-a", Some("account-a"));
    a.lists(&[(1, "Private to A")]);
    a.start();
    assert_eq!(cloud.names().len(), 1);
    let other = MockCloud::new();
    let mut b = mac(&other, "A", "account-a", Some("account-a"));
    b.lists(&[(1, "Private to A")]);
    b.start();
    b.lists(&[(1, "Edited after the switch")]);
    b.core.transport_mut().sign_in("account-b");
    b.run();
    assert_eq!(b.phase(), Phase::Paused(PauseReason::AccountChanged));
    assert!(
        b.effects.iter().any(
            |e| matches!(e, Effect::AccountMismatch { user_hash } if user_hash == "account-b")
        )
    );
    assert_eq!(other.names(), ["watchlist.1"]);
    assert_eq!(
        payload_name(&other, "watchlist.1").as_deref(),
        Some("Private to A")
    );
    assert_eq!(b.outbox().len(), 1);
    b.advance(600_000);
    b.run();
    assert_eq!(
        payload_name(&other, "watchlist.1").as_deref(),
        Some("Private to A")
    );
}

#[test]
fn a_deleted_zone_pauses_until_the_user_decides() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    a.core.transport_mut().zone_event(ZoneEvent::Deleted);
    a.run();
    assert_eq!(a.phase(), Phase::Paused(PauseReason::RemoteDataDeleted));
    a.lists(&[(1, "Tech 2")]);
    a.advance(60_000);
    a.run();
    assert_eq!(payload_name(&cloud, "watchlist.1").as_deref(), Some("Tech"));
    a.core.clear_pause();
    a.run();
    assert_eq!(
        payload_name(&cloud, "watchlist.1").as_deref(),
        Some("Tech 2")
    );
}

#[test]
fn lost_encryption_keys_trigger_a_full_reupload() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "a"), (2, "b")]);
    a.start();
    assert_eq!(cloud.save_attempts(), 2);
    cloud.delete_zone_contents();
    a.core
        .transport_mut()
        .zone_event(ZoneEvent::EncryptedDataReset);
    a.run();
    // The zone has to be recreated first; give the retry time to come due.
    for _ in 0..3 {
        a.advance(5_000);
        a.run();
    }
    assert_eq!(cloud.names(), ["watchlist.1", "watchlist.2"]);
    assert_eq!(a.phase(), Phase::UpToDate);
}

#[test]
fn staging_for_the_wrong_account_is_refused_by_the_bridge() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "account-a", Some("account-a"));
    a.start();
    // The user switches accounts; the event has not reached the core yet.
    a.core.transport_mut().user_hash = Some("account-b".into());
    a.lists(&[(1, "Private to A")]);
    a.core.tick();
    assert!(cloud.names().is_empty());
}

#[test]
fn going_offline_waits_and_coming_back_resumes() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    a.deliver(TransportEvent::Network { available: false });
    assert_eq!(a.phase(), Phase::WaitingForNetwork);
    a.lists(&[(1, "Offline")]);
    a.run();
    assert!(cloud.names().is_empty());
    a.deliver(TransportEvent::Network { available: true });
    a.run();
    assert_eq!(cloud.names(), ["watchlist.1"]);
}

#[test]
fn an_expired_change_token_restarts_from_a_fresh_fetch() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    a.deliver(TransportEvent::Error {
        kind: ErrorKind::ChangeTokenExpired,
        message: "expired".into(),
    });
    a.run();
    let state = a.store.lock().unwrap().sync_state().unwrap().unwrap();
    assert!(
        state.engine_state.is_some(),
        "a fresh checkpoint is saved after refetching"
    );
    let starts: Vec<_> = a
        .core
        .transport_mut()
        .commands
        .iter()
        .filter_map(|c| match c {
            TransportCommand::Start { engine_state, .. } => Some(engine_state.is_some()),
            _ => None,
        })
        .collect();
    assert_eq!(starts.last(), Some(&false));
}

#[test]
fn reconcile_runs_after_a_pause_is_cleared() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    a.core.transport_mut().zone_event(ZoneEvent::Deleted);
    a.run();
    // A write the hooks never saw (an older build) while paused.
    a.store
        .lock()
        .unwrap()
        .replace_documents("watchlist", &[list(1, "x")])
        .unwrap();
    a.store.lock().unwrap().remove_outbox(1).ok();
    a.core.clear_pause();
    a.run();
    assert_eq!(cloud.names(), ["watchlist.1"]);
}

#[test]
fn an_upsert_whose_row_vanished_is_dropped_and_reconciled() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    a.advance(1);
    a.lists(&[(1, "x")]);
    // Simulate an out-of-band removal that left the upsert queued.
    a.store
        .lock()
        .unwrap()
        .execute_raw("DELETE FROM watchlist")
        .unwrap();
    a.run();
    assert!(a.outbox().iter().all(|r| r.op == OutboxOp::Delete) || a.outbox().is_empty());
}

// MARK: Deleting everything in iCloud

#[test]
fn deleting_cloud_data_is_confirmed_by_icloud_before_it_counts() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    assert_eq!(cloud.names(), ["watchlist.1"]);
    assert!(!a.core.status().cloud_deleted);
    a.core.delete_cloud_data();
    assert_eq!(a.phase(), Phase::Syncing, "waiting for iCloud to confirm");
    a.run();
    assert!(cloud.names().is_empty());
    assert!(a.core.status().cloud_deleted);
    assert_eq!(a.lists_len(), 1, "nothing local was touched");
}

#[test]
fn only_the_account_this_database_is_bound_to_can_be_deleted() {
    let cloud = MockCloud::new();
    // Signed in as a different account than the one this database syncs with.
    let mut a = mac(&cloud, "A", "someone-else", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    a.core.delete_cloud_data();
    a.run();
    assert!(
        !a.core
            .transport_mut()
            .commands
            .contains(&TransportCommand::DeleteZone)
    );
    assert!(!a.core.status().cloud_deleted);
}

#[test]
fn a_mac_that_did_not_ask_for_the_deletion_pauses_instead_of_deleting_anything() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    let mut b = mac(&cloud, "B", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    b.start();
    a.core.delete_cloud_data();
    a.run();
    b.core.transport_mut().zone_event(ZoneEvent::Deleted);
    b.run();
    assert_eq!(b.phase(), Phase::Paused(PauseReason::RemoteDataDeleted));
    assert!(!b.core.status().cloud_deleted);
    assert_eq!(b.lists_len(), 0);
}

#[test]
fn a_failed_deletion_says_why_changes_nothing_and_can_be_retried() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.lists(&[(1, "Tech")]);
    a.start();
    cloud.inject(Fault::DeleteZone(ErrorKind::Network));
    a.core.delete_cloud_data();
    assert!(a.core.status().cloud_deleting);
    a.run();
    let status = a.core.status();
    assert!(!status.cloud_deleting && !status.cloud_deleted);
    assert_eq!(
        status.cloud_delete_error.as_deref(),
        Some("DegenView couldn't reach iCloud.")
    );
    assert_eq!(cloud.names(), ["watchlist.1"], "nothing was deleted");
    // Trying again clears the error and succeeds.
    a.core.delete_cloud_data();
    assert_eq!(a.core.status().cloud_delete_error, None);
    a.run();
    assert!(a.core.status().cloud_deleted);
    assert!(cloud.names().is_empty());
}

#[test]
fn stopping_forgets_a_finished_deletion_so_it_is_not_reported_twice() {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "A", "acct", Some("acct"));
    a.start();
    a.core.delete_cloud_data();
    a.run();
    assert!(a.core.status().cloud_deleted);
    a.core.stop();
    assert!(!a.core.status().cloud_deleted);
}

impl Mac {
    pub(super) fn lists_len(&self) -> usize {
        self.store
            .lock()
            .unwrap()
            .raw_rows("watchlist")
            .unwrap()
            .len()
    }
}
