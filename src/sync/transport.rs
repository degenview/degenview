//! The boundary between the pure sync core and CloudKit. The core sends `TransportCommand`s and
//! reacts to `TransportEvent`s; nothing else about CloudKit is visible to it. The real transport
//! is the Swift `CKSyncEngine` bridge; tests use `MockCloud`.

use super::record::{RecordName, SyncRecord};

/// What went wrong with a cloud operation, as the core needs to react to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Network,
    AccountUnavailable,
    Quota,
    RateLimited,
    ZoneNotFound,
    /// The record changed in the cloud since this Mac last saw it.
    ServerRecordChanged,
    /// The record this save was based on no longer exists in the cloud.
    UnknownItem,
    /// The saved fetch position is too old; start over from scratch.
    ChangeTokenExpired,
    TooLarge,
    Permission,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountStatus {
    Available { user_hash: String },
    NoAccount,
    Restricted,
    CouldNotDetermine,
    TemporarilyUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountChange {
    SignIn { user_hash: String },
    SignOut,
    Switch { user_hash: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneEvent {
    /// The user deleted DegenView's iCloud data (or the zone) elsewhere.
    Deleted,
    Purged,
    /// iCloud lost its encryption keys; every record must be uploaded again.
    EncryptedDataReset,
}

/// A record as the cloud returned it. The name and entity type are still strings because a newer
/// app may have written kinds this version has never heard of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
    pub name: String,
    pub schema_version: u32,
    pub revision: u64,
    pub origin_device: String,
    pub modified_ms: i64,
    pub payload: Vec<u8>,
    /// CloudKit's encoded system fields (change tag), needed to save over this version.
    pub system_fields: Vec<u8>,
}

impl RawRecord {
    /// `Err` when the name is not one this version understands, or the payload is a damaged
    /// deflated one. A deflated payload is unpacked here, so nothing after this sees the difference.
    pub fn into_record(self) -> Result<(SyncRecord, Vec<u8>), RawRecord> {
        match (
            RecordName::parse(&self.name),
            super::record::from_wire(self.payload.clone()),
        ) {
            (Some(name), Some(payload)) => Ok((
                SyncRecord {
                    name,
                    schema_version: self.schema_version,
                    revision: self.revision,
                    origin_device: self.origin_device,
                    modified_ms: self.modified_ms,
                    payload,
                },
                self.system_fields,
            )),
            _ => Err(self),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedOp {
    Save {
        record: SyncRecord,
        system_fields: Option<Vec<u8>>,
    },
    Delete,
}

/// One outbox entry handed to the engine. `outbox_seq` comes back in the result so only the
/// exact entry that was sent is cleared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedChange {
    pub outbox_seq: i64,
    pub name: RecordName,
    pub op: StagedOp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportCommand {
    Start {
        zone: String,
        engine_state: Option<Vec<u8>>,
    },
    AccountCheck,
    EnsureZone,
    /// Replaces any earlier staging of the same record. `account_hash` is the account this
    /// database is bound to; the bridge drops the stage if a different account is signed in.
    Stage {
        account_hash: String,
        changes: Vec<StagedChange>,
    },
    Send,
    Fetch,
    DeleteZone,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentSaved {
    pub outbox_seq: i64,
    pub name: String,
    pub system_fields: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentDeleted {
    pub outbox_seq: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendFailure {
    pub outbox_seq: i64,
    pub name: String,
    pub kind: ErrorKind,
    pub retry_after_s: Option<u64>,
    pub message: String,
    /// The cloud's current version, for `ServerRecordChanged`.
    pub server_record: Option<RawRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEvent {
    Ready,
    Account(AccountStatus),
    AccountChange(AccountChange),
    Fetched {
        records: Vec<RawRecord>,
        /// Names of records deleted in the cloud.
        deletions: Vec<String>,
    },
    /// The engine's opaque checkpoint. Persisted only after the fetched changes it covers have
    /// been applied locally.
    State(Vec<u8>),
    Sent {
        saved: Vec<SentSaved>,
        deleted: Vec<SentDeleted>,
        failed: Vec<SendFailure>,
    },
    Zone(ZoneEvent),
    /// A fetch ran to the end: everything iCloud holds newer than the checkpoint has been delivered.
    FetchComplete,
    /// The zone this Mac asked to delete is gone (confirmation of `DeleteZone`).
    ZoneDeleted,
    Network {
        available: bool,
    },
    Error {
        kind: ErrorKind,
        message: String,
    },
}

/// Implemented by the Swift bridge and by the test cloud. Must not block.
pub trait SyncTransport {
    fn send(&mut self, command: TransportCommand);
}
