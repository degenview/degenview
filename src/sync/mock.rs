//! An in-memory CloudKit stand-in for tests: one shared zone with change tags and a change log,
//! plus a per-device transport that behaves like `CKSyncEngine` as far as the core can tell.
//! Faults are injected explicitly so each failure path is deterministic.

use super::record::RecordName;
use super::transport::{
    AccountChange, AccountStatus, ErrorKind, RawRecord, SendFailure, SentDeleted, SentSaved,
    StagedChange, StagedOp, SyncTransport, TransportCommand, TransportEvent, ZoneEvent,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The next send fails wholesale.
    Send(ErrorKind, Option<u64>),
    /// Only these record names fail on the next send.
    Partial(Vec<String>, ErrorKind),
}

#[derive(Debug, Clone)]
struct CloudRecord {
    raw: RawRecord,
    tag: u64,
}

#[derive(Debug, Clone)]
enum Change {
    Saved(String, String),
    Deleted(String, String),
}

#[derive(Default)]
struct Inner {
    records: BTreeMap<String, CloudRecord>,
    log: Vec<Change>,
    next_tag: u64,
    zone_exists: bool,
    faults: VecDeque<Fault>,
    save_attempts: usize,
    delete_attempts: usize,
    fetches: usize,
}

/// The shared cloud. Clone it to give every device a handle.
#[derive(Clone, Default)]
pub struct MockCloud {
    inner: Arc<Mutex<Inner>>,
}

impl MockCloud {
    pub fn new() -> Self {
        let cloud = Self::default();
        cloud.inner.lock().unwrap().zone_exists = true;
        cloud
    }

    pub fn inject(&self, fault: Fault) {
        self.inner.lock().unwrap().faults.push_back(fault);
    }

    pub fn delete_zone_contents(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.records.clear();
        inner.log.clear();
        inner.zone_exists = false;
    }

    pub fn names(&self) -> Vec<String> {
        self.inner.lock().unwrap().records.keys().cloned().collect()
    }

    pub fn record(&self, name: &str) -> Option<RawRecord> {
        self.inner
            .lock()
            .unwrap()
            .records
            .get(name)
            .map(|r| r.raw.clone())
    }

    pub fn save_attempts(&self) -> usize {
        self.inner.lock().unwrap().save_attempts
    }

    pub fn delete_attempts(&self) -> usize {
        self.inner.lock().unwrap().delete_attempts
    }

    pub fn fetches(&self) -> usize {
        self.inner.lock().unwrap().fetches
    }

    /// Writes a record as if another Mac had uploaded it.
    pub fn put_from_elsewhere(&self, mut raw: RawRecord, writer: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.next_tag += 1;
        let tag = inner.next_tag;
        raw.system_fields = tag.to_le_bytes().to_vec();
        inner
            .log
            .push(Change::Saved(raw.name.clone(), writer.to_string()));
        inner
            .records
            .insert(raw.name.clone(), CloudRecord { raw, tag });
    }

    pub fn delete_from_elsewhere(&self, name: &str, writer: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.records.remove(name);
        inner
            .log
            .push(Change::Deleted(name.to_string(), writer.to_string()));
    }
}

/// One Mac's connection to the cloud.
pub struct MockTransport {
    cloud: MockCloud,
    pub device: String,
    pub user_hash: Option<String>,
    events: VecDeque<TransportEvent>,
    staged: Vec<StagedChange>,
    token: usize,
    /// Whether this Mac is told about its own uploads when it fetches.
    pub echo_own: bool,
    pub commands: Vec<TransportCommand>,
}

impl MockTransport {
    pub fn new(cloud: &MockCloud, device: &str, user_hash: &str) -> Self {
        Self {
            cloud: cloud.clone(),
            device: device.to_string(),
            user_hash: Some(user_hash.to_string()),
            events: VecDeque::new(),
            staged: Vec::new(),
            token: 0,
            echo_own: false,
            commands: Vec::new(),
        }
    }

    pub fn push(&mut self, event: TransportEvent) {
        self.events.push_back(event);
    }

    pub fn take_events(&mut self) -> Vec<TransportEvent> {
        self.events.drain(..).collect()
    }

    pub fn sign_out(&mut self) {
        self.user_hash = None;
        self.events
            .push_back(TransportEvent::AccountChange(AccountChange::SignOut));
    }

    pub fn sign_in(&mut self, user_hash: &str) {
        self.user_hash = Some(user_hash.to_string());
        self.events
            .push_back(TransportEvent::AccountChange(AccountChange::SignIn {
                user_hash: user_hash.to_string(),
            }));
        self.staged.clear();
    }

    pub fn zone_event(&mut self, event: ZoneEvent) {
        self.events.push_back(TransportEvent::Zone(event));
    }

    fn account_event(&self) -> TransportEvent {
        TransportEvent::Account(match &self.user_hash {
            Some(hash) => AccountStatus::Available {
                user_hash: hash.clone(),
            },
            None => AccountStatus::NoAccount,
        })
    }

    fn fail_all(&mut self, kind: ErrorKind, retry_after_s: Option<u64>) {
        let failed = std::mem::take(&mut self.staged)
            .into_iter()
            .map(|change| SendFailure {
                outbox_seq: change.outbox_seq,
                name: change.name.render(),
                kind,
                retry_after_s,
                message: format!("{kind:?}"),
                server_record: None,
            })
            .collect();
        self.events.push_back(TransportEvent::Sent {
            saved: Vec::new(),
            deleted: Vec::new(),
            failed,
        });
    }

    fn send_staged(&mut self) {
        let fault = self.cloud.inner.lock().unwrap().faults.pop_front();
        let mut partial: Vec<String> = Vec::new();
        let mut partial_kind = ErrorKind::Unknown;
        match fault {
            Some(Fault::Send(kind, retry)) => return self.fail_all(kind, retry),
            Some(Fault::Partial(names, kind)) => {
                partial = names;
                partial_kind = kind;
            }
            None => {}
        }
        if !self.cloud.inner.lock().unwrap().zone_exists {
            return self.fail_all(ErrorKind::ZoneNotFound, None);
        }
        let mut saved = Vec::new();
        let mut deleted = Vec::new();
        let mut failed = Vec::new();
        let mut inner = self.cloud.inner.lock().unwrap();
        for change in std::mem::take(&mut self.staged) {
            let name = change.name.render();
            let fail = |kind: ErrorKind, server: Option<RawRecord>| SendFailure {
                outbox_seq: change.outbox_seq,
                name: name.clone(),
                kind,
                retry_after_s: None,
                message: format!("{kind:?}"),
                server_record: server,
            };
            if partial.contains(&name) {
                failed.push(fail(partial_kind, None));
                continue;
            }
            match &change.op {
                StagedOp::Delete => {
                    inner.delete_attempts += 1;
                    inner.records.remove(&name);
                    inner
                        .log
                        .push(Change::Deleted(name.clone(), self.device.clone()));
                    deleted.push(SentDeleted {
                        outbox_seq: change.outbox_seq,
                        name,
                    });
                }
                StagedOp::Save {
                    record,
                    system_fields,
                } => {
                    inner.save_attempts += 1;
                    let known = inner.records.get(&name).cloned();
                    let held_tag = system_fields
                        .as_ref()
                        .and_then(|bytes| bytes.as_slice().try_into().ok())
                        .map(u64::from_le_bytes);
                    match (known, held_tag) {
                        (Some(server), held) if held != Some(server.tag) => {
                            failed.push(fail(ErrorKind::ServerRecordChanged, Some(server.raw)));
                            continue;
                        }
                        (None, Some(_)) => {
                            failed.push(fail(ErrorKind::UnknownItem, None));
                            continue;
                        }
                        _ => {}
                    }
                    inner.next_tag += 1;
                    let tag = inner.next_tag;
                    let fields = tag.to_le_bytes().to_vec();
                    let raw = RawRecord {
                        name: name.clone(),
                        schema_version: record.schema_version,
                        revision: record.revision,
                        origin_device: record.origin_device.clone(),
                        modified_ms: record.modified_ms,
                        payload: record.payload.clone(),
                        system_fields: fields.clone(),
                    };
                    inner.records.insert(name.clone(), CloudRecord { raw, tag });
                    inner
                        .log
                        .push(Change::Saved(name.clone(), self.device.clone()));
                    saved.push(SentSaved {
                        outbox_seq: change.outbox_seq,
                        name,
                        system_fields: fields,
                    });
                }
            }
        }
        drop(inner);
        self.events.push_back(TransportEvent::Sent {
            saved,
            deleted,
            failed,
        });
    }

    fn fetch(&mut self) {
        let inner = self.cloud.inner.lock().unwrap();
        let mut order: Vec<(String, bool)> = Vec::new();
        for change in &inner.log[self.token.min(inner.log.len())..] {
            let (name, writer, _deleted) = match change {
                Change::Saved(name, writer) => (name, writer, false),
                Change::Deleted(name, writer) => (name, writer, true),
            };
            if writer == &self.device && !self.echo_own {
                continue;
            }
            if !order.iter().any(|(n, _)| n == name) {
                order.push((name.clone(), false));
            }
        }
        let mut records = Vec::new();
        let mut deletions = Vec::new();
        for (name, _) in order {
            match inner.records.get(&name) {
                Some(record) => records.push(record.raw.clone()),
                None => deletions.push(name),
            }
        }
        let new_token = inner.log.len();
        drop(inner);
        self.cloud.inner.lock().unwrap().fetches += 1;
        self.token = new_token;
        if !records.is_empty() || !deletions.is_empty() {
            self.events
                .push_back(TransportEvent::Fetched { records, deletions });
        }
        self.events.push_back(TransportEvent::State(
            (new_token as u64).to_le_bytes().to_vec(),
        ));
    }
}

impl SyncTransport for MockTransport {
    fn send(&mut self, command: TransportCommand) {
        self.commands.push(command.clone());
        match command {
            TransportCommand::Start { engine_state, .. } => {
                self.token = engine_state
                    .as_deref()
                    .and_then(|bytes| bytes.try_into().ok())
                    .map_or(0, |bytes| u64::from_le_bytes(bytes) as usize);
                self.events.push_back(TransportEvent::Ready);
            }
            TransportCommand::AccountCheck => {
                let event = self.account_event();
                self.events.push_back(event);
            }
            TransportCommand::EnsureZone => self.cloud.inner.lock().unwrap().zone_exists = true,
            TransportCommand::Stage {
                account_hash,
                changes,
            } => {
                // Never stage data bound to one account for another account's database.
                if self.user_hash.as_deref() != Some(account_hash.as_str()) {
                    return;
                }
                for change in changes {
                    self.staged.retain(|existing| existing.name != change.name);
                    self.staged.push(change);
                }
            }
            TransportCommand::Send => self.send_staged(),
            TransportCommand::Fetch => self.fetch(),
            TransportCommand::DeleteZone => self.cloud.delete_zone_contents(),
            TransportCommand::Stop => {}
        }
    }
}

#[allow(dead_code)]
pub fn name_of(kind: super::policy::EntityKind, id: &str) -> String {
    RecordName::new(kind, id).unwrap().render()
}
