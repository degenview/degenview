use super::*;
use crate::sync::policy::EntityKind;
use std::sync::Mutex;

#[test]
fn base64_round_trips_every_length_and_byte() {
    for len in 0..40 {
        let data: Vec<u8> = (0..len).map(|n| (n * 37 + 11) as u8).collect();
        assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data, "{len}");
    }
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(base64_decode(&base64_encode(&all)).unwrap(), all);
}

#[test]
fn base64_matches_the_standard_encoding() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
    assert!(base64_decode("not base64!").is_none());
}

#[test]
fn commands_serialize_with_the_fields_the_bridge_reads() {
    let start: Value = serde_json::from_str(&command_json(&TransportCommand::Start {
        zone: "DegenViewZone".into(),
        engine_state: Some(vec![1, 2, 3]),
    }))
    .unwrap();
    assert_eq!(start["cmd"], "start");
    assert_eq!(start["zone"], "DegenViewZone");
    assert_eq!(start["container"], crate::sync::account::CONTAINER);
    assert_eq!(start["engine_state"], "AQID");
    let fresh: Value = serde_json::from_str(&command_json(&TransportCommand::Start {
        zone: "z".into(),
        engine_state: None,
    }))
    .unwrap();
    assert!(fresh["engine_state"].is_null());
    for (command, name) in [
        (TransportCommand::AccountCheck, "account_check"),
        (TransportCommand::EnsureZone, "ensure_zone"),
        (TransportCommand::Send, "send"),
        (TransportCommand::Fetch, "fetch"),
        (TransportCommand::DeleteZone, "delete_zone"),
        (TransportCommand::Stop, "stop"),
    ] {
        let value: Value = serde_json::from_str(&command_json(&command)).unwrap();
        assert_eq!(value["cmd"], name);
    }
}

#[test]
fn a_stage_command_carries_the_account_the_data_is_bound_to() {
    let record = sample_record("watchlist.7", b"{\"id\":7}");
    let stage = TransportCommand::Stage {
        account_hash: "acct".into(),
        changes: vec![
            StagedChange {
                outbox_seq: 4,
                name: record.name.clone(),
                op: StagedOp::Save {
                    record: record.clone(),
                    system_fields: Some(vec![9]),
                },
            },
            StagedChange {
                outbox_seq: 5,
                name: RecordName::new(EntityKind::Watchlist, "8").unwrap(),
                op: StagedOp::Delete,
            },
        ],
    };
    let value: Value = serde_json::from_str(&command_json(&stage)).unwrap();
    assert_eq!(value["account_hash"], "acct");
    let changes = value["changes"].as_array().unwrap();
    assert_eq!(changes[0]["op"], "save");
    assert_eq!(changes[0]["seq"], 4);
    assert_eq!(changes[0]["record_name"], "watchlist.7");
    assert_eq!(changes[0]["entity_type"], "watchlist");
    assert_eq!(
        base64_decode(changes[0]["payload"].as_str().unwrap()).unwrap(),
        b"{\"id\":7}"
    );
    assert_eq!(changes[0]["system_fields"], "CQ==");
    assert_eq!(changes[1]["op"], "delete");
    assert!(changes[1].get("payload").is_none());
}

#[test]
fn every_event_kind_decodes() {
    let b = |s: &str| base64_encode(s.as_bytes());
    let cases = [
        (r#"{"event":"ready"}"#, TransportEvent::Ready),
        (
            r#"{"event":"account","status":"available","user_hash":"h"}"#,
            TransportEvent::Account(AccountStatus::Available {
                user_hash: "h".into(),
            }),
        ),
        (
            r#"{"event":"account","status":"no_account"}"#,
            TransportEvent::Account(AccountStatus::NoAccount),
        ),
        (
            r#"{"event":"account","status":"restricted"}"#,
            TransportEvent::Account(AccountStatus::Restricted),
        ),
        (
            r#"{"event":"account_change","type":"switch","user_hash":"n"}"#,
            TransportEvent::AccountChange(AccountChange::Switch {
                user_hash: "n".into(),
            }),
        ),
        (
            r#"{"event":"account_change","type":"sign_out"}"#,
            TransportEvent::AccountChange(AccountChange::SignOut),
        ),
        (
            r#"{"event":"zone","kind":"encrypted_data_reset"}"#,
            TransportEvent::Zone(ZoneEvent::EncryptedDataReset),
        ),
        (
            r#"{"event":"zone","kind":"deleted"}"#,
            TransportEvent::Zone(ZoneEvent::Deleted),
        ),
        (
            r#"{"event":"network","available":false}"#,
            TransportEvent::Network { available: false },
        ),
        (
            r#"{"event":"error","kind":"quota","message":"full"}"#,
            TransportEvent::Error {
                kind: ErrorKind::Quota,
                message: "full".into(),
            },
        ),
    ];
    for (json, expected) in cases {
        assert_eq!(event_from_json(json).unwrap(), expected, "{json}");
    }
    let state = format!(r#"{{"event":"state","data":"{}"}}"#, b("checkpoint"));
    assert_eq!(
        event_from_json(&state).unwrap(),
        TransportEvent::State(b"checkpoint".to_vec())
    );
}

#[test]
fn fetched_and_sent_events_carry_records_and_failures() {
    let b = |s: &str| base64_encode(s.as_bytes());
    let fetched = format!(
        r#"{{"event":"fetched","records":[{{"name":"watchlist.1","schema_version":1,"revision":3,
        "origin_device":"d","modified_ms":9,"payload":"{}","system_fields":"{}"}}],"deletions":["watchlist.2"]}}"#,
        b("{}"),
        b("tag")
    );
    let TransportEvent::Fetched { records, deletions } = event_from_json(&fetched).unwrap() else {
        panic!("expected fetched");
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].revision, 3);
    assert_eq!(records[0].payload, b"{}");
    assert_eq!(records[0].system_fields, b"tag");
    assert_eq!(deletions, ["watchlist.2"]);

    let sent = format!(
        r#"{{"event":"sent","saved":[{{"seq":1,"name":"watchlist.1","system_fields":"{}"}}],
        "deleted":[{{"seq":2,"name":"watchlist.2"}}],
        "failed":[{{"seq":3,"name":"watchlist.3","kind":"server_record_changed","message":"x",
        "retry_after_s":null,"server_record":{{"name":"watchlist.3","schema_version":1,"revision":2,
        "origin_device":"d","modified_ms":1,"payload":"{}","system_fields":"{}"}}}},
        {{"seq":4,"name":"watchlist.4","kind":"rate_limited","retry_after_s":30,"message":"slow down"}}]}}"#,
        b("t1"),
        b("{}"),
        b("t2")
    );
    let TransportEvent::Sent {
        saved,
        deleted,
        failed,
    } = event_from_json(&sent).unwrap()
    else {
        panic!("expected sent");
    };
    assert_eq!(saved[0].outbox_seq, 1);
    assert_eq!(deleted[0].name, "watchlist.2");
    assert_eq!(failed[0].kind, ErrorKind::ServerRecordChanged);
    assert_eq!(
        failed[0].server_record.as_ref().unwrap().system_fields,
        b"t2"
    );
    assert_eq!(failed[1].kind, ErrorKind::RateLimited);
    assert_eq!(failed[1].retry_after_s, Some(30));
    assert!(failed[1].server_record.is_none());
}

#[test]
fn malformed_events_are_errors_not_panics() {
    for json in [
        "",
        "not json",
        "{}",
        r#"{"event":"mystery"}"#,
        r#"{"event":"state"}"#,
        r#"{"event":"state","data":"***"}"#,
        r#"{"event":"account","status":"available"}"#,
        r#"{"event":"sent","saved":[{"name":"x"}]}"#,
        r#"{"event":"fetched","records":[{"name":"a"}]}"#,
    ] {
        assert!(event_from_json(json).is_err(), "{json}");
    }
}

#[test]
fn unknown_error_names_are_classified_as_unknown() {
    assert_eq!(error_kind("quota"), ErrorKind::Quota);
    assert_eq!(error_kind("zone_not_found"), ErrorKind::ZoneNotFound);
    assert_eq!(error_kind("unknown_item"), ErrorKind::UnknownItem);
    assert_eq!(error_kind("something new"), ErrorKind::Unknown);
}

// MARK: A fake Swift side

struct FakeBridge {
    commands: Mutex<Vec<String>>,
    callback: Mutex<Option<(EventCallback, usize)>>,
    destroyed: Mutex<bool>,
    version: u32,
    refuse_create: bool,
}

static FAKE: Mutex<Option<std::sync::Arc<FakeBridge>>> = Mutex::new(None);
/// The fake bridge is process-wide (C functions have no state), so tests using it take turns.
static SERIAL: Mutex<()> = Mutex::new(());

fn fake() -> std::sync::Arc<FakeBridge> {
    FAKE.lock()
        .unwrap()
        .clone()
        .expect("a fake bridge is installed")
}

unsafe extern "C" fn fake_version() -> u32 {
    fake().version
}

unsafe extern "C" fn fake_create(
    _config: *const u8,
    _len: usize,
    callback: EventCallback,
    user: *mut c_void,
) -> *mut c_void {
    let bridge = fake();
    if bridge.refuse_create {
        return std::ptr::null_mut();
    }
    *bridge.callback.lock().unwrap() = Some((callback, user as usize));
    std::ptr::dangling_mut::<c_void>()
}

unsafe extern "C" fn fake_command(_handle: *mut c_void, json: *const u8, len: usize) {
    let text =
        String::from_utf8(unsafe { std::slice::from_raw_parts(json, len) }.to_vec()).unwrap();
    fake().commands.lock().unwrap().push(text);
}

unsafe extern "C" fn fake_destroy(_handle: *mut c_void) {
    *fake().destroyed.lock().unwrap() = true;
}

fn install(version: u32, refuse_create: bool) -> (NativeApi, std::sync::Arc<FakeBridge>) {
    let bridge = std::sync::Arc::new(FakeBridge {
        commands: Mutex::new(Vec::new()),
        callback: Mutex::new(None),
        destroyed: Mutex::new(false),
        version,
        refuse_create,
    });
    *FAKE.lock().unwrap() = Some(bridge.clone());
    (
        NativeApi {
            abi_version: fake_version,
            create: fake_create,
            command: fake_command,
            destroy: fake_destroy,
        },
        bridge,
    )
}

fn emit(bridge: &FakeBridge, json: &str) {
    let (callback, user) = bridge.callback.lock().unwrap().expect("created");
    unsafe { callback(user as *mut c_void, json.as_ptr(), json.len()) };
}

fn collecting_sink() -> (EventSink, Arc<Mutex<Vec<TransportEvent>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let seen = seen.clone();
        Arc::new(move |event| seen.lock().unwrap().push(event))
    };
    (sink, seen)
}

#[test]
fn commands_reach_the_bridge_and_its_events_reach_the_sink() {
    let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (api, bridge) = install(ABI_VERSION, false);
    let (sink, seen) = collecting_sink();
    let mut transport = NativeTransport::new(api, sink).unwrap();
    transport.send(TransportCommand::Fetch);
    transport.send(TransportCommand::Send);
    assert_eq!(bridge.commands.lock().unwrap().len(), 2);
    assert!(bridge.commands.lock().unwrap()[0].contains("\"fetch\""));
    emit(&bridge, r#"{"event":"ready"}"#);
    emit(&bridge, r#"{"event":"network","available":true}"#);
    assert_eq!(
        *seen.lock().unwrap(),
        [
            TransportEvent::Ready,
            TransportEvent::Network { available: true }
        ]
    );
    drop(transport);
    assert!(
        *bridge.destroyed.lock().unwrap(),
        "the bridge is told to stop on drop"
    );
}

#[test]
fn a_garbled_event_becomes_an_error_event_and_never_crashes() {
    let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (api, bridge) = install(ABI_VERSION, false);
    let (sink, seen) = collecting_sink();
    let _transport = NativeTransport::new(api, sink).unwrap();
    emit(&bridge, "{{{{ not json");
    let events = seen.lock().unwrap();
    assert!(
        matches!(&events[0], TransportEvent::Error { message, .. } if message.contains("unreadable"))
    );
}

#[test]
fn a_panicking_sink_does_not_unwind_into_swift() {
    let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (api, bridge) = install(ABI_VERSION, false);
    let sink: EventSink = Arc::new(|_| panic!("the app side blew up"));
    let _transport = NativeTransport::new(api, sink).unwrap();
    emit(&bridge, r#"{"event":"ready"}"#);
}

#[test]
fn a_bridge_of_the_wrong_version_is_refused() {
    let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (api, _bridge) = install(ABI_VERSION + 1, false);
    let (sink, _) = collecting_sink();
    assert!(NativeTransport::new(api, sink).is_err());
}

#[test]
fn a_bridge_that_cannot_start_is_reported_and_leaves_an_inert_transport() {
    let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (api, _bridge) = install(ABI_VERSION, true);
    let (sink, seen) = collecting_sink();
    let factory = CloudKitFactory::new(api);
    let mut transport = factory.create(sink);
    transport.send(TransportCommand::Fetch);
    assert!(matches!(
        seen.lock().unwrap()[0],
        TransportEvent::Error { .. }
    ));
}

#[test]
fn the_factory_refuses_to_run_when_this_copy_is_not_signed_for_icloud() {
    let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (api, bridge) = install(ABI_VERSION, false);
    let factory = CloudKitFactory::new(api);
    assert!(
        factory
            .environment()
            .unwrap_err()
            .contains("not signed for iCloud")
    );
    assert!(
        bridge.callback.lock().unwrap().is_none(),
        "CloudKit was never touched"
    );
}
