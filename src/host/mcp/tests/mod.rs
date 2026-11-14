use super::alert::*;
use super::pine::*;
use super::*;
use crate::host::alert::AlertHost;
use crate::mcp::alert::{AlertCommandOutcome, AlertUpdate, NewAlert};
use crate::mcp::bridge::McpError;
use crate::mcp::pine::{InputArgs, ScriptToApply};
use crate::mcp::state::McpStatus;
use crate::mcp::transport::auth::verifier_hex;
use crate::model::alert::{
    AlertCondition, AlertCurrency, AlertFrequency, AlertId, AlertState, PriceAlert,
};
use crate::model::mcp::{
    AUDIT_LOG_LIMIT, BindMode, Cidr, MAX_CIDRS, McpAction, McpAuditEntry, McpAuditKind, McpClient,
    McpSettings, Permission, PermissionSet,
};
use crate::model::watchlist::{new_id, now_ms};
use crate::model::{DataSource, InstrumentId};
use crate::service::keychain::mcp_seal::{KeyState, SealKeyStore};
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::store::SharedStore;
use crate::store::Store;
use crate::workspace::{ChartWorkspace, PineInstanceView};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod alert;
mod frame;
mod pine;
mod seal;

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
}

fn host(store: &SharedStore) -> McpHost {
    McpHost::load(store.clone(), Arc::new(|| {}))
}

#[test]
fn last_used_times_that_could_not_be_saved_are_kept_for_the_next_try() {
    let store = store();
    let mut host = host(&store);
    host.shared.mark_usage_dirty();
    // The store cannot be locked (a panic while it was held).
    let poisoner = store.clone();
    let _ = std::thread::spawn(move || {
        let _held = poisoner.lock().unwrap();
        panic!("poison the store lock");
    })
    .join();
    host.last_usage_save = Instant::now() - USAGE_SAVE_INTERVAL - Duration::from_secs(1);
    host.tick();
    assert!(
        host.shared.take_usage_update().is_some(),
        "the failed save must not clear the pending update"
    );
}

fn audit_row(n: u64) -> McpAuditEntry {
    McpAuditEntry {
        id: n,
        at_ms: 1_000 + n as i64,
        client_id: "c1".into(),
        client_name: "Claude".into(),
        tool: "degenview_list_charts".into(),
        count: 1,
        ..McpAuditEntry::default()
    }
}

fn ids(rows: &[McpAuditEntry]) -> Vec<u64> {
    rows.iter().map(|row| row.id).collect()
}

fn due_for_flush(host: &mut McpHost) {
    host.last_audit_flush = Instant::now() - AUDIT_FLUSH_INTERVAL - Duration::from_secs(1);
}

#[test]
fn queued_activity_is_written_in_one_batch_and_shown_by_the_page() {
    let store = store();
    let mut host = host(&store);
    assert!(host.activity().is_empty() && host.activity_unreadable() == 0);

    host.shared.audit().push(audit_row(1));
    host.shared.audit().push(audit_row(2));
    // Not due yet: the tick leaves the rows in memory and the store alone.
    host.tick();
    assert!(host.activity().is_empty());
    assert!(store.lock().unwrap().recent_mcp_audit(10).is_empty());

    due_for_flush(&mut host);
    host.tick();
    assert_eq!(ids(&host.activity()), [2, 1], "newest first");
    assert_eq!(ids(&store.lock().unwrap().recent_mcp_audit(10)), [2, 1]);
    assert_eq!(host.shared.audit().len(), 0);
}

#[test]
fn the_page_holds_the_whole_log_and_no_more_than_the_log_keeps() {
    let store = store();
    let mut host = host(&store);
    for n in 1..=(AUDIT_LOG_LIMIT as u64 + 20) {
        host.shared.audit().push(audit_row(n));
    }
    due_for_flush(&mut host);
    host.tick();
    assert_eq!(host.activity().len(), AUDIT_LOG_LIMIT);
    assert_eq!(host.activity()[0].id, AUDIT_LOG_LIMIT as u64 + 20);
    assert_eq!(
        host.activity()[AUDIT_LOG_LIMIT - 1].id,
        21,
        "the oldest 20 were pruned"
    );
    assert_eq!(host.activity_unreadable(), 0);
}

#[test]
fn activity_that_could_not_be_written_is_kept_for_the_next_try() {
    let store = store();
    let mut host = host(&store);
    host.shared.audit().push(audit_row(1));
    let poisoner = store.clone();
    let _ = std::thread::spawn(move || {
        let _held = poisoner.lock().unwrap();
        panic!("poison the store lock");
    })
    .join();
    due_for_flush(&mut host);
    host.tick();
    assert_eq!(
        host.shared.audit().len(),
        1,
        "the row went back in the queue"
    );
}

#[test]
fn quitting_writes_the_activity_still_waiting() {
    let store = store();
    {
        let host = host(&store);
        host.shared.audit().push(audit_row(1));
        host.shared.audit().rate_limited(&claude(), now_ms());
    }
    let rows = store.lock().unwrap().recent_mcp_audit(10);
    assert_eq!(
        rows.len(),
        2,
        "the call, and the rate-limit count still open"
    );
    assert!(rows.iter().any(|row| row.kind == McpAuditKind::RateLimited));
}

#[test]
fn shutting_down_writes_what_is_left_once_and_dropping_afterwards_adds_nothing() {
    let store = store();
    let mut host = host(&store);
    host.shared.audit().push(audit_row(1));
    host.shared.audit().rate_limited(&claude(), now_ms());
    host.shutdown();
    let rows = store.lock().unwrap().recent_mcp_audit(10);
    assert_eq!(rows.len(), 2, "the call and the open rate-limit count");
    assert_eq!(host.activity().len(), 2);
    // Again, and then the drop that follows in a real quit: nothing is written twice.
    host.shutdown();
    drop(host);
    assert_eq!(store.lock().unwrap().mcp_audit_count(), 2);
}

#[test]
fn clearing_deletes_everything_even_what_was_still_in_memory() {
    let store = store();
    let mut host = host(&store);
    host.shared.audit().push(audit_row(1));
    due_for_flush(&mut host);
    host.tick();
    // A row that is only in memory when the user clears.
    host.shared.audit().push(audit_row(2));

    host.apply(McpAction::ClearActivity);
    assert_eq!(host.activity().len(), 1);
    assert_eq!(host.activity()[0].kind, McpAuditKind::Cleared);
    assert_eq!(host.shared.audit().len(), 0);
    let rows = store.lock().unwrap().recent_mcp_audit(10);
    assert_eq!(rows.len(), 1, "nothing before the marker survives");

    // Activity after that follows the marker.
    host.shared.audit().push(audit_row(3));
    due_for_flush(&mut host);
    host.tick();
    assert_eq!(host.activity()[0].id, 3);
    assert_eq!(host.activity()[1].kind, McpAuditKind::Cleared);
}

#[test]
fn an_unreadable_log_row_does_not_stop_the_host_or_the_page() {
    let store = store();
    store.lock().unwrap().insert_raw_mcp_audit("{ not json");
    store
        .lock()
        .unwrap()
        .append_mcp_audit(&[audit_row(1)])
        .unwrap();
    let host = host(&store);
    assert!(host.load_error().is_none());
    assert_eq!(ids(&host.activity()), [1]);
    assert_eq!(
        host.activity_unreadable(),
        1,
        "the sheet can say it hides one"
    );
}

fn claude() -> crate::mcp::state::ClientIdentity {
    crate::mcp::state::ClientIdentity {
        id: "c1".into(),
        name: "Claude".into(),
        remote: false,
        peer: crate::model::mcp::Peer::from_ip(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
    }
}

fn enable_on_free_port(host: &mut McpHost, store: &SharedStore) {
    // Tests bind port 0 (any free one), which the Settings page itself never offers.
    store
        .lock()
        .unwrap()
        .set_mcp_settings(&McpSettings {
            enabled: false,
            port: 0,
            ..McpSettings::default()
        })
        .unwrap();
    host.settings.port = 0;
    host.apply(McpAction::SetEnabled(true));
}

fn running_port(host: &McpHost) -> u16 {
    match host.status() {
        McpStatus::Running { port } => port,
        other => panic!("not running: {other:?}"),
    }
}

#[test]
fn a_fresh_install_is_off_and_listens_nowhere() {
    let host = host(&store());
    assert_eq!(host.status(), McpStatus::Disabled);
    assert!(!host.settings().enabled);
    assert!(host.clients().is_empty());
    assert_eq!(
        host.global_permissions(),
        crate::model::mcp::default_global_permissions()
    );
    assert!(host.load_error().is_none());
}

#[test]
fn enabling_and_disabling_controls_the_real_listener() {
    let store = store();
    let mut host = host(&store);
    enable_on_free_port(&mut host, &store);
    let port = running_port(&host);
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    assert!(store.lock().unwrap().mcp_settings().unwrap().enabled);

    host.apply(McpAction::SetEnabled(false));
    assert_eq!(host.status(), McpStatus::Disabled);
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    assert!(!store.lock().unwrap().mcp_settings().unwrap().enabled);
}

fn remote_settings() -> McpSettings {
    McpSettings {
        enabled: true,
        port: 0,
        bind: BindMode::interface("192.0.2.1".parse().unwrap()).unwrap(),
        ..McpSettings::default()
    }
}

fn lan() -> BindMode {
    BindMode::interface("192.0.2.1".parse().unwrap()).unwrap()
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("degenview-mcp-host-{name}-{}", new_id()))
}

#[test]
fn choosing_an_interface_saves_it_and_shows_the_fingerprint_to_pin() {
    let store = store();
    let mut host = host(&store);
    let dir = scratch_dir("bind");
    host.tls_dir = Some(dir.clone());
    assert!(host.tls_fingerprint().is_none());

    host.apply(McpAction::SetBind(lan()));
    assert_eq!(store.lock().unwrap().mcp_settings().unwrap().bind, lan());
    assert_eq!(host.settings().bind, lan());
    // The server is off, so nothing listens, but the certificate is there to compare.
    assert_eq!(host.status(), McpStatus::Disabled);
    let first = host.tls_fingerprint().expect("a fingerprint").to_owned();
    assert_eq!(first.len(), 32 * 3 - 1);

    // The same address keeps the same certificate; back on this Mac clears the fingerprint.
    host.apply(McpAction::SetBind(BindMode::Loopback));
    assert!(host.tls_fingerprint().is_none());
    host.apply(McpAction::SetBind(lan()));
    assert_eq!(host.tls_fingerprint(), Some(first.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_server_that_is_on_follows_the_bind_and_comes_back_on_this_mac() {
    let store = store();
    let mut host = host(&store);
    let dir = scratch_dir("follow");
    host.tls_dir = Some(dir.clone());
    enable_on_free_port(&mut host, &store);
    assert!(matches!(host.status(), McpStatus::Running { .. }));

    // 192.0.2.1 is a documentation address no machine has, so the listener cannot bind it:
    // the failure is reported, and nothing is left listening on the old address either.
    host.apply(McpAction::SetBind(lan()));
    assert!(
        matches!(host.status(), McpStatus::Failed(_)),
        "{:?}",
        host.status()
    );
    assert!(host.notice().is_some());

    host.apply(McpAction::SetBind(BindMode::Loopback));
    assert!(matches!(host.status(), McpStatus::Running { .. }));
    assert!(host.tls_fingerprint().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unchanged_bind_does_nothing_and_unreadable_settings_are_not_overwritten() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::SetBind(BindMode::Loopback));
    assert!(host.notice().is_none());

    // With nowhere to keep a certificate the choice is still saved, and the reason shown.
    host.apply(McpAction::SetBind(lan()));
    assert_eq!(host.settings().bind, lan());
    assert!(host.tls_fingerprint().is_none());
    assert!(host.notice().is_some_and(|n| n.contains("certificate")));

    store.lock().unwrap().insert_raw_mcp_client("x", "{broken");
    let mut broken = McpHost::load(store.clone(), Arc::new(|| {}));
    broken.apply(McpAction::SetBind(lan()));
    assert_eq!(broken.settings().bind, BindMode::Loopback);
}

#[test]
fn the_fingerprint_shown_is_that_of_the_identity_the_listener_was_given() {
    // The certificate is deleted while the app runs. The next start makes a new one, and the
    // page must show that one, not the one it showed before.
    let store = store();
    let mut host = host(&store);
    let dir = scratch_dir("fingerprint");
    host.tls_dir = Some(dir.clone());
    host.apply(McpAction::SetBind(lan()));
    let before = host.tls_fingerprint().expect("a fingerprint").to_owned();

    std::fs::remove_file(dir.join(crate::mcp::transport::tls::FILE_NAME)).unwrap();
    // The stale value is still what the page has until something loads an identity.
    assert_eq!(host.tls_fingerprint(), Some(before.as_str()));
    // `start_server` is what loads it for the listener (the address cannot be bound here, so
    // it fails after loading, which is exactly the part under test).
    host.settings.enabled = true;
    host.start_server();
    let shown = host.tls_fingerprint().expect("a fingerprint").to_owned();
    let served = crate::mcp::transport::tls::load_or_create(&dir, "192.0.2.1".parse().unwrap())
        .unwrap()
        .fingerprint();
    assert_ne!(shown, before, "a new certificate was made");
    assert_eq!(shown, served, "and the page shows the one that was served");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_certificate_that_cannot_be_loaded_leaves_no_fingerprint_behind() {
    let store = store();
    let mut host = host(&store);
    let dir = scratch_dir("unloadable");
    host.tls_dir = Some(dir.clone());
    host.apply(McpAction::SetBind(lan()));
    assert!(host.tls_fingerprint().is_some());
    std::fs::write(dir.join(crate::mcp::transport::tls::FILE_NAME), "damaged").unwrap();
    assert!(host.tls_identity().is_err());
    assert_eq!(host.tls_fingerprint(), None, "nothing stale is shown");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_remote_bind_with_nowhere_to_keep_a_certificate_stays_off() {
    let store = store();
    store
        .lock()
        .unwrap()
        .set_mcp_settings(&remote_settings())
        .unwrap();
    // `host` has no certificate folder (see `tls_dir`), so it cannot serve TLS.
    let host = host(&store);
    assert!(
        matches!(host.status(), McpStatus::Failed(message) if message.contains("certificate")),
        "{:?}",
        host.status()
    );
    assert!(host.notice().is_some());
}

#[test]
fn a_remote_bind_gets_a_saved_certificate_for_its_address_and_loopback_gets_none() {
    let store = store();
    let mut host = host(&store);
    assert!(host.tls_identity().unwrap().is_none());

    let dir = std::env::temp_dir().join(format!("degenview-mcp-host-tls-{}", new_id()));
    host.tls_dir = Some(dir.clone());
    host.settings = remote_settings();
    let first = host.tls_identity().unwrap().expect("a certificate");
    assert_eq!(
        first.address(),
        "192.0.2.1".parse::<std::net::IpAddr>().unwrap()
    );
    let again = host.tls_identity().unwrap().expect("the same certificate");
    assert_eq!(first.fingerprint(), again.fingerprint());
    assert!(dir.join(crate::mcp::transport::tls::FILE_NAME).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_a_previously_enabled_server_starts_on_launch() {
    let store = store();
    store
        .lock()
        .unwrap()
        .set_mcp_settings(&McpSettings {
            enabled: true,
            port: 0,
            ..McpSettings::default()
        })
        .unwrap();
    let host = host(&store);
    assert!(matches!(host.status(), McpStatus::Running { .. }));
    drop(host);
    // Dropping the host (app quit) closes the listener.
}

#[test]
fn credentials_are_shown_once_and_only_their_hash_is_kept() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::CreateClient("  Claude Desktop ".into()));
    let fresh = host.fresh_token().cloned().expect("token shown once");
    assert_eq!(fresh.client_name, "Claude Desktop");
    assert!(fresh.token.starts_with("dvmcp_"));

    let stored = store.lock().unwrap().mcp_clients().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].verifier, verifier_hex(&fresh.token));
    assert!(
        !serde_json::to_string(&stored[0])
            .unwrap()
            .contains(&fresh.token)
    );
    // A new credential can read and nothing more.
    assert_eq!(
        stored[0].permissions,
        PermissionSet::of([Permission::ChartsRead])
    );
    assert_eq!(
        host.shared().authenticate(&fresh.token).unwrap().id,
        stored[0].id
    );

    host.apply(McpAction::DismissToken);
    assert!(host.fresh_token().is_none());
}

#[test]
fn revoking_is_immediate_and_persisted() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::CreateClient("a".into()));
    let token = host.fresh_token().unwrap().token.clone();
    let id = host.clients()[0].id.clone();
    assert!(host.shared().authenticate(&token).is_some());

    host.apply(McpAction::RevokeClient(id));
    assert!(host.shared().authenticate(&token).is_none());
    assert!(host.fresh_token().is_none());
    assert!(store.lock().unwrap().mcp_clients().unwrap().is_empty());
}

#[test]
fn permission_changes_publish_only_after_they_are_saved() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::CreateClient("a".into()));
    let id = host.clients()[0].id.clone();
    host.apply(McpAction::SetClientPermission {
        client_id: id.clone(),
        permission: Permission::ChartsCreate,
        allowed: true,
    });
    // Granted by the client but capped by the global read-only default.
    assert!(!host.shared().allows(&id, Permission::ChartsCreate));
    host.apply(McpAction::SetGlobalPermission {
        permission: Permission::ChartsCreate,
        allowed: true,
    });
    assert!(host.shared().allows(&id, Permission::ChartsCreate));
    assert!(
        store
            .lock()
            .unwrap()
            .mcp_global_permissions()
            .unwrap()
            .contains(Permission::ChartsCreate)
    );
    host.apply(McpAction::SetClientPermission {
        client_id: id.clone(),
        permission: Permission::ChartsCreate,
        allowed: false,
    });
    assert!(!host.shared().allows(&id, Permission::ChartsCreate));
}

#[test]
fn a_new_credential_is_loopback_only_until_remote_access_is_given() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::CreateClient("a".into()));
    let token = host.fresh_token().unwrap().token.clone();
    let id = host.clients()[0].id.clone();
    assert!(!host.clients()[0].remote_allowed);
    let lan = Some("192.168.1.9".parse().unwrap());
    assert!(host.shared().authenticate_from(&token, lan).is_none());

    host.apply(McpAction::SetClientRemote {
        client_id: id.clone(),
        allowed: true,
    });
    assert!(store.lock().unwrap().mcp_clients().unwrap()[0].remote_allowed);
    assert!(host.shared().authenticate_from(&token, lan).is_some());

    host.apply(McpAction::SetClientRemote {
        client_id: id,
        allowed: false,
    });
    assert!(host.shared().authenticate_from(&token, lan).is_none());
}

#[test]
fn ranges_are_saved_without_duplicates_and_limit_where_a_client_may_call_from() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::CreateClient("a".into()));
    let token = host.fresh_token().unwrap().token.clone();
    let id = host.clients()[0].id.clone();
    host.apply(McpAction::SetClientRemote {
        client_id: id.clone(),
        allowed: true,
    });
    let range = |text: &str| Cidr::parse(text).unwrap();
    host.apply(McpAction::SetClientCidrs {
        client_id: id.clone(),
        cidrs: vec![
            range("192.168.1.0/24"),
            range("192.168.1.77/24"),
            range("fd00::/8"),
        ],
    });
    let saved = store.lock().unwrap().mcp_clients().unwrap();
    assert_eq!(
        saved[0].allowed_cidrs,
        [range("192.168.1.0/24"), range("fd00::/8")]
    );
    let from = |text: &str| Some(text.parse().unwrap());
    assert!(
        host.shared()
            .authenticate_from(&token, from("192.168.1.9"))
            .is_some()
    );
    assert!(
        host.shared()
            .authenticate_from(&token, from("203.0.113.9"))
            .is_none()
    );

    // More than a Settings list can show are cut, not saved.
    let many: Vec<Cidr> = (0..40).map(|n| range(&format!("10.{n}.0.0/16"))).collect();
    host.apply(McpAction::SetClientCidrs {
        client_id: id,
        cidrs: many,
    });
    assert_eq!(host.clients()[0].allowed_cidrs.len(), MAX_CIDRS);
}

#[test]
fn the_remote_ceiling_is_saved_published_and_caps_remote_clients() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::CreateClient("a".into()));
    let id = host.clients()[0].id.clone();
    host.apply(McpAction::SetClientPermission {
        client_id: id.clone(),
        permission: Permission::ChartsCreate,
        allowed: true,
    });
    host.apply(McpAction::SetGlobalPermission {
        permission: Permission::ChartsCreate,
        allowed: true,
    });
    assert!(host.shared().allows(&id, Permission::ChartsCreate));

    // Remote access turns the default ceiling (reads only) on for this client.
    host.apply(McpAction::SetClientRemote {
        client_id: id.clone(),
        allowed: true,
    });
    assert!(!host.shared().allows(&id, Permission::ChartsCreate));

    let wider = PermissionSet::everything();
    host.apply(McpAction::SetRemoteCeiling(wider.clone()));
    assert_eq!(
        store.lock().unwrap().mcp_settings().unwrap().remote_ceiling,
        wider
    );
    assert_eq!(host.settings().remote_ceiling, wider);
    assert!(host.shared().allows(&id, Permission::ChartsCreate));
}

#[test]
fn a_saved_ceiling_and_remote_client_are_back_after_a_restart() {
    let store = store();
    {
        let mut host = host(&store);
        host.apply(McpAction::CreateClient("a".into()));
        let id = host.clients()[0].id.clone();
        host.apply(McpAction::SetClientRemote {
            client_id: id,
            allowed: true,
        });
        host.apply(McpAction::SetRemoteCeiling(PermissionSet::none()));
    }
    let host = host(&store);
    assert!(host.load_error().is_none(), "{:?}", host.load_error());
    assert!(host.clients()[0].remote_allowed);
    assert!(host.shared().effective(&host.clients()[0].id).is_empty());
}

#[test]
fn unreadable_saved_state_fails_closed_and_is_never_overwritten() {
    let store = store();
    {
        let guard = store.lock().unwrap();
        guard
            .set_mcp_settings(&McpSettings {
                enabled: true,
                port: 0,
                ..McpSettings::default()
            })
            .unwrap();
        guard.insert_raw_mcp_client("x", "{broken");
    }
    let mut host = host(&store);
    assert!(host.load_error().is_some());
    assert_eq!(
        host.status(),
        McpStatus::Disabled,
        "must not start with lost credentials"
    );
    assert!(host.clients().is_empty());

    host.apply(McpAction::CreateClient("a".into()));
    host.apply(McpAction::SetEnabled(true));
    assert_eq!(host.status(), McpStatus::Disabled);
    assert!(host.fresh_token().is_none());
    // The broken row is still there to be recovered, not replaced.
    assert!(store.lock().unwrap().mcp_clients().is_err());
}

#[test]
fn a_preset_replaces_a_whole_grant_or_the_cap_in_one_write() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::CreateClient("a".into()));
    let id = host.clients()[0].id.clone();

    host.apply(McpAction::SetGlobalPermissions(PermissionSet::everything()));
    assert_eq!(host.global_permissions(), PermissionSet::everything());
    assert_eq!(
        store.lock().unwrap().mcp_global_permissions().unwrap(),
        PermissionSet::everything()
    );

    host.apply(McpAction::SetClientPermissions {
        client_id: id.clone(),
        permissions: PermissionSet::everything(),
    });
    assert!(host.shared().allows(&id, Permission::PineScriptsSave));
    assert_eq!(
        store.lock().unwrap().mcp_clients().unwrap()[0].permissions,
        PermissionSet::everything()
    );

    // Lowering the cap holds the grant back at once without touching the grant itself.
    host.apply(McpAction::SetGlobalPermissions(PermissionSet::read_only()));
    assert!(!host.shared().allows(&id, Permission::PineScriptsSave));
    assert!(host.shared().allows(&id, Permission::PineReportsRead));
    assert_eq!(host.clients()[0].permissions, PermissionSet::everything());

    host.apply(McpAction::SetClientPermissions {
        client_id: id.clone(),
        permissions: PermissionSet::none(),
    });
    assert!(host.shared().effective(&id).is_empty());
    // An unknown client changes nothing.
    host.apply(McpAction::SetClientPermissions {
        client_id: "nobody".into(),
        permissions: PermissionSet::everything(),
    });
    assert_eq!(host.clients().len(), 1);
}

#[test]
fn bad_input_is_refused_with_a_notice() {
    let store = store();
    let mut host = host(&store);
    host.apply(McpAction::CreateClient("   ".into()));
    assert!(host.notice().is_some());
    assert!(host.clients().is_empty());
    host.apply(McpAction::SetPort(80));
    assert!(host.notice().is_some());
    assert_eq!(host.settings().port, crate::model::mcp::DEFAULT_PORT);
}

#[test]
fn settings_actions_are_applied_on_the_next_tick() {
    let store = store();
    let mut host = host(&store);
    host.queue(McpAction::CreateClient("queued".into()));
    assert!(host.clients().is_empty());
    host.tick();
    assert_eq!(host.clients().len(), 1);
}
