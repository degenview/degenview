//! The wire format and plumbing between the Rust core and the Swift CloudKit bridge.
//!
//! Commands and events are JSON with base64 for binary fields. The C functions themselves are
//! declared in the `degenview` binary and handed to `NativeApi` as function pointers, so the
//! library (and the background alert agent that shares it) never links Swift. Everything here is
//! plain Rust and is tested against a fake bridge.

use super::record::{RecordName, SyncRecord};
use super::transport::{
    AccountChange, AccountStatus, ErrorKind, RawRecord, SendFailure, SentDeleted, SentSaved,
    StagedChange, StagedOp, SyncTransport, TransportCommand, TransportEvent, ZoneEvent,
};
use crate::service::entitlements::{icloud_environment, read_self};
use crate::sync_host::{EventSink, TransportFactory};
use serde_json::{Value, json};
use std::ffi::c_void;
use std::sync::Arc;

/// Bumped when the command or event format changes incompatibly.
pub const ABI_VERSION: u32 = 1;

// MARK: Base64

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let value = ALPHABET.iter().position(|c| *c == byte)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// MARK: Commands

fn optional_b64(bytes: &Option<Vec<u8>>) -> Value {
    bytes
        .as_ref()
        .map_or(Value::Null, |b| Value::String(base64_encode(b)))
}

fn change_json(change: &StagedChange) -> Value {
    match &change.op {
        StagedOp::Delete => json!({
            "seq": change.outbox_seq,
            "record_name": change.name.render(),
            "op": "delete",
        }),
        StagedOp::Save {
            record,
            system_fields,
        } => json!({
            "seq": change.outbox_seq,
            "record_name": change.name.render(),
            "op": "save",
            "entity_type": record.name.kind.prefix(),
            "schema_version": record.schema_version,
            "revision": record.revision,
            "origin_device": record.origin_device,
            "modified_ms": record.modified_ms,
            "payload": base64_encode(&record.payload),
            "system_fields": optional_b64(system_fields),
        }),
    }
}

pub fn command_json(command: &TransportCommand) -> String {
    let value = match command {
        TransportCommand::Start { zone, engine_state } => json!({
            "cmd": "start",
            "zone": zone,
            "container": crate::sync::account::CONTAINER,
            "engine_state": optional_b64(engine_state),
        }),
        TransportCommand::AccountCheck => json!({"cmd": "account_check"}),
        TransportCommand::EnsureZone => json!({"cmd": "ensure_zone"}),
        TransportCommand::Stage {
            account_hash,
            changes,
        } => json!({
            "cmd": "stage",
            "account_hash": account_hash,
            "changes": changes.iter().map(change_json).collect::<Vec<_>>(),
        }),
        TransportCommand::Send => json!({"cmd": "send"}),
        TransportCommand::Fetch => json!({"cmd": "fetch"}),
        TransportCommand::DeleteZone => json!({"cmd": "delete_zone"}),
        TransportCommand::Stop => json!({"cmd": "stop"}),
    };
    value.to_string()
}

// MARK: Events

fn text(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing `{key}`"))
}

fn bytes(value: &Value, key: &str) -> Result<Vec<u8>, String> {
    base64_decode(&text(value, key)?).ok_or_else(|| format!("`{key}` is not base64"))
}

fn raw_record(value: &Value) -> Result<RawRecord, String> {
    Ok(RawRecord {
        name: text(value, "name")?,
        schema_version: value
            .get("schema_version")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        revision: value.get("revision").and_then(Value::as_u64).unwrap_or(0),
        origin_device: value
            .get("origin_device")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        modified_ms: value
            .get("modified_ms")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        payload: bytes(value, "payload")?,
        system_fields: bytes(value, "system_fields").unwrap_or_default(),
    })
}

pub fn error_kind(name: &str) -> ErrorKind {
    match name {
        "network" => ErrorKind::Network,
        "account_unavailable" => ErrorKind::AccountUnavailable,
        "quota" => ErrorKind::Quota,
        "rate_limited" => ErrorKind::RateLimited,
        "zone_not_found" => ErrorKind::ZoneNotFound,
        "server_record_changed" => ErrorKind::ServerRecordChanged,
        "unknown_item" => ErrorKind::UnknownItem,
        "change_token_expired" => ErrorKind::ChangeTokenExpired,
        "too_large" => ErrorKind::TooLarge,
        "permission" => ErrorKind::Permission,
        _ => ErrorKind::Unknown,
    }
}

pub fn event_from_json(text_json: &str) -> Result<TransportEvent, String> {
    let value: Value = serde_json::from_str(text_json).map_err(|e| e.to_string())?;
    let kind = text(&value, "event")?;
    Ok(match kind.as_str() {
        "ready" => TransportEvent::Ready,
        "account" => TransportEvent::Account(match text(&value, "status")?.as_str() {
            "available" => AccountStatus::Available {
                user_hash: text(&value, "user_hash")?,
            },
            "no_account" => AccountStatus::NoAccount,
            "restricted" => AccountStatus::Restricted,
            "temporarily_unavailable" => AccountStatus::TemporarilyUnavailable,
            _ => AccountStatus::CouldNotDetermine,
        }),
        "account_change" => TransportEvent::AccountChange(match text(&value, "type")?.as_str() {
            "sign_in" => AccountChange::SignIn {
                user_hash: text(&value, "user_hash")?,
            },
            "switch" => AccountChange::Switch {
                user_hash: text(&value, "user_hash")?,
            },
            _ => AccountChange::SignOut,
        }),
        "fetched" => TransportEvent::Fetched {
            records: value
                .get("records")
                .and_then(Value::as_array)
                .map(|list| list.iter().map(raw_record).collect::<Result<Vec<_>, _>>())
                .transpose()?
                .unwrap_or_default(),
            deletions: value
                .get("deletions")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        },
        "state" => TransportEvent::State(bytes(&value, "data")?),
        "sent" => {
            let list = |key: &str| {
                value
                    .get(key)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            };
            TransportEvent::Sent {
                saved: list("saved")
                    .iter()
                    .map(|item| {
                        Ok(SentSaved {
                            outbox_seq: item
                                .get("seq")
                                .and_then(Value::as_i64)
                                .ok_or("missing `seq`")?,
                            name: text(item, "name")?,
                            system_fields: bytes(item, "system_fields").unwrap_or_default(),
                        })
                    })
                    .collect::<Result<_, String>>()?,
                deleted: list("deleted")
                    .iter()
                    .map(|item| {
                        Ok(SentDeleted {
                            outbox_seq: item
                                .get("seq")
                                .and_then(Value::as_i64)
                                .ok_or("missing `seq`")?,
                            name: text(item, "name")?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
                failed: list("failed")
                    .iter()
                    .map(|item| {
                        Ok(SendFailure {
                            outbox_seq: item
                                .get("seq")
                                .and_then(Value::as_i64)
                                .ok_or("missing `seq`")?,
                            name: text(item, "name")?,
                            kind: error_kind(&text(item, "kind")?),
                            retry_after_s: item.get("retry_after_s").and_then(Value::as_u64),
                            message: item
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .into(),
                            server_record: item
                                .get("server_record")
                                .filter(|v| !v.is_null())
                                .map(raw_record)
                                .transpose()?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            }
        }
        "zone" => TransportEvent::Zone(match text(&value, "kind")?.as_str() {
            "purged" => ZoneEvent::Purged,
            "encrypted_data_reset" => ZoneEvent::EncryptedDataReset,
            _ => ZoneEvent::Deleted,
        }),
        "network" => TransportEvent::Network {
            available: value
                .get("available")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        },
        "error" => TransportEvent::Error {
            kind: error_kind(&text(&value, "kind")?),
            message: value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into(),
        },
        other => return Err(format!("unknown event `{other}`")),
    })
}

// MARK: The bridge

pub type EventCallback = unsafe extern "C" fn(user: *mut c_void, json: *const u8, len: usize);

/// The C functions the Swift library exports, as pointers handed in by the binary.
#[derive(Clone, Copy)]
pub struct NativeApi {
    pub abi_version: unsafe extern "C" fn() -> u32,
    pub create: unsafe extern "C" fn(
        config: *const u8,
        len: usize,
        callback: EventCallback,
        user: *mut c_void,
    ) -> *mut c_void,
    pub command: unsafe extern "C" fn(handle: *mut c_void, json: *const u8, len: usize),
    pub destroy: unsafe extern "C" fn(handle: *mut c_void),
}

/// Receives the bridge's events. Runs on a CloudKit thread: it only decodes and forwards, never
/// locks a store or calls back into Swift, and never lets a panic cross into Swift.
unsafe extern "C" fn on_event(user: *mut c_void, json: *const u8, len: usize) {
    if user.is_null() || json.is_null() {
        return;
    }
    let _ = std::panic::catch_unwind(|| {
        // SAFETY: Swift passes a buffer valid for `len` bytes for the duration of this call, and
        // `user` is the boxed sink installed by `NativeTransport::new`, alive until `destroy`.
        let (sink, slice) = unsafe {
            (
                &*(user as *const EventSink),
                std::slice::from_raw_parts(json, len),
            )
        };
        let event = std::str::from_utf8(slice)
            .map_err(|e| e.to_string())
            .and_then(event_from_json);
        match event {
            Ok(event) => sink(event),
            Err(message) => sink(TransportEvent::Error {
                kind: ErrorKind::Unknown,
                message: format!("unreadable event from the iCloud bridge: {message}"),
            }),
        }
    });
}

/// One connection to the Swift bridge.
pub struct NativeTransport {
    api: NativeApi,
    handle: *mut c_void,
    sink: *mut EventSink,
}

// SAFETY: the handle is an opaque pointer the Swift side synchronises internally, and the sink is
// `Send + Sync`; the transport is only ever driven from the sync worker thread.
unsafe impl Send for NativeTransport {}

impl NativeTransport {
    pub fn new(api: NativeApi, sink: EventSink) -> Result<Self, String> {
        // SAFETY: plain call returning an integer.
        let version = unsafe { (api.abi_version)() };
        if version != ABI_VERSION {
            return Err(format!(
                "iCloud bridge speaks version {version}, expected {ABI_VERSION}"
            ));
        }
        let sink = Box::into_raw(Box::new(sink));
        let config = json!({"container": crate::sync::account::CONTAINER}).to_string();
        // SAFETY: `config` is valid for the call; `sink` stays allocated until `Drop`.
        let handle = unsafe { (api.create)(config.as_ptr(), config.len(), on_event, sink.cast()) };
        if handle.is_null() {
            // SAFETY: reclaiming the box created above; nothing else holds it.
            drop(unsafe { Box::from_raw(sink) });
            return Err("the iCloud bridge could not start".into());
        }
        Ok(Self { api, handle, sink })
    }
}

impl SyncTransport for NativeTransport {
    fn send(&mut self, command: TransportCommand) {
        let json = command_json(&command);
        // SAFETY: the handle is live until `Drop`; `json` is valid for the call and Swift copies
        // what it keeps.
        unsafe { (self.api.command)(self.handle, json.as_ptr(), json.len()) };
    }
}

impl Drop for NativeTransport {
    fn drop(&mut self) {
        // SAFETY: after `destroy` returns, Swift fires no more callbacks, so the sink can be freed.
        unsafe {
            (self.api.destroy)(self.handle);
            drop(Box::from_raw(self.sink));
        }
    }
}

/// A transport that does nothing, for when the bridge cannot start; the page shows why.
struct Inert;

impl SyncTransport for Inert {
    fn send(&mut self, _: TransportCommand) {}
}

/// Connects the app to CloudKit through the Swift bridge, once the signature has been checked.
pub struct CloudKitFactory {
    api: NativeApi,
    release_build: bool,
    read_entitlement: fn(&str) -> Option<crate::service::entitlements::EntitlementValue>,
}

impl CloudKitFactory {
    pub fn new(api: NativeApi) -> Self {
        Self {
            api,
            release_build: !cfg!(debug_assertions),
            read_entitlement: read_self,
        }
    }
}

impl TransportFactory for CloudKitFactory {
    fn environment(&self) -> Result<String, String> {
        icloud_environment(&self.read_entitlement, self.release_build)
    }

    fn create(&self, events: EventSink) -> Box<dyn SyncTransport + Send> {
        match NativeTransport::new(self.api, Arc::clone(&events)) {
            Ok(transport) => Box::new(transport),
            Err(message) => {
                events(TransportEvent::Error {
                    kind: ErrorKind::Permission,
                    message,
                });
                Box::new(Inert)
            }
        }
    }
}

/// Builds a record the way it appears after a round trip, for tests.
#[allow(dead_code)]
pub fn sample_record(name: &str, payload: &[u8]) -> SyncRecord {
    SyncRecord {
        name: RecordName::parse(name).expect("a valid record name"),
        schema_version: 1,
        revision: 1,
        origin_device: "device".into(),
        modified_ms: 5,
        payload: payload.to_vec(),
    }
}

#[cfg(test)]
mod tests;
