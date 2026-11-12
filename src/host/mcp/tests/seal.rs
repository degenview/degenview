use super::alert::seeded;
use super::pine::empty_workspace;
use super::*;

use crate::service::keychain::api_keys::{KeyBackend, MemoryBackend};
use crate::service::keychain::webhook_secrets::Scope;
use crate::service::webhook::delivery::SecretStoreError;
use std::sync::atomic::{AtomicBool, Ordering};

/// A Keychain two host loads can share, standing in for the real one across a restart.
#[derive(Default)]
struct SharedKeychain {
    items: MemoryBackend,
    refuse_writes: AtomicBool,
    /// Reads find nothing while the item still exists: a locked or timed-out keyring.
    refuse_reads: AtomicBool,
}

impl KeyBackend for Arc<SharedKeychain> {
    fn read(&self, scope: Scope, account: &str) -> Option<String> {
        if self.refuse_reads.load(Ordering::Relaxed) {
            return None;
        }
        self.items.read(scope, account)
    }
    fn exists(&self, scope: Scope, account: &str) -> bool {
        self.items.exists(scope, account)
    }
    fn write(&self, scope: Scope, account: &str, value: &str) -> Result<(), SecretStoreError> {
        if self.refuse_writes.load(Ordering::Relaxed) {
            return Err(SecretStoreError("no keyring".into()));
        }
        self.items.write(scope, account, value)
    }
    fn delete(&self, scope: Scope, account: &str) -> Result<(), SecretStoreError> {
        self.items.delete(scope, account)
    }
}

fn sealed_host(store: &SharedStore, keychain: &Arc<SharedKeychain>) -> McpHost {
    McpHost::load_with_keys(
        store.clone(),
        Arc::new(|| {}),
        Some(SealKeyStore::with_backend(Box::new(keychain.clone()))),
    )
}

fn create_client(host: &mut McpHost, name: &str) -> String {
    host.apply(McpAction::CreateClient(name.into()));
    host.fresh_token()
        .expect("a credential was made")
        .token
        .clone()
}

/// What an attacker with write access to the file (and no Keychain) can do: add a client
/// whose verifier is the hash of a token they chose.
fn plant_client(store: &SharedStore, token: &str) {
    let store = store.lock().unwrap();
    let mut clients = store.mcp_clients().unwrap();
    clients.push(McpClient {
        id: "planted".into(),
        name: "planted".into(),
        verifier: verifier_hex(token),
        permissions: PermissionSet::of(Permission::ALL),
        created_at_ms: 1,
        last_used_ms: None,
        remote_allowed: false,
        allowed_cidrs: Vec::new(),
    });
    store.replace_mcp_clients(&clients).unwrap();
    store
        .set_mcp_settings(&McpSettings {
            enabled: true,
            port: 49_999,
            ..McpSettings::default()
        })
        .unwrap();
}

#[test]
fn a_first_load_creates_the_key_and_seals_the_empty_state() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let host = sealed_host(&store, &keychain);
    assert!(host.load_error().is_none());
    assert!(host.sealed_by.is_some());
    assert!(matches!(
        SealKeyStore::with_backend(Box::new(keychain.clone())).load(),
        KeyState::Ready(_)
    ));
    assert!(store.lock().unwrap().mcp_seal().unwrap().is_some());
}

#[test]
fn a_credential_planted_in_the_database_is_refused_after_a_restart() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let mut first = sealed_host(&store, &keychain);
    let real = create_client(&mut first, "Claude");
    drop(first);

    plant_client(&store, "attacker-token");
    let second = sealed_host(&store, &keychain);

    assert!(second.load_error().is_some_and(|e| e.contains("seal")));
    assert!(second.can_reset());
    assert!(
        !second.settings().enabled,
        "the planted settings are not used"
    );
    assert!(second.clients().is_empty());
    assert!(second.shared().authenticate("attacker-token").is_none());
    assert!(
        second.shared().authenticate(&real).is_none(),
        "nothing authenticates while the state is untrusted"
    );
}

#[test]
fn each_kind_of_edit_is_caught() {
    type Edit = Box<dyn Fn(&Store)>;
    let edits: [(&str, Edit); 5] = [
        (
            "enabling the server",
            Box::new(|s| {
                s.set_mcp_settings(&McpSettings {
                    enabled: true,
                    port: 8765,
                    ..McpSettings::default()
                })
                .unwrap()
            }),
        ),
        (
            "widening the global cap",
            Box::new(|s| {
                s.set_mcp_global_permissions(&PermissionSet::of(Permission::ALL))
                    .unwrap()
            }),
        ),
        (
            "widening a client's grants",
            Box::new(|s| {
                let mut clients = s.mcp_clients().unwrap();
                clients[0].permissions = PermissionSet::of(Permission::ALL);
                s.replace_mcp_clients(&clients).unwrap();
            }),
        ),
        (
            "swapping a verifier",
            Box::new(|s| {
                let mut clients = s.mcp_clients().unwrap();
                clients[0].verifier = verifier_hex("attacker-token");
                s.replace_mcp_clients(&clients).unwrap();
            }),
        ),
        (
            "deleting the seal",
            Box::new(|s| s.remove_setting(crate::store::mcp::SEAL_KEY).unwrap()),
        ),
    ];
    for (what, edit) in edits {
        let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
        let mut host = sealed_host(&store, &keychain);
        create_client(&mut host, "Claude");
        drop(host);
        edit(&store.lock().unwrap());
        let host = sealed_host(&store, &keychain);
        assert!(host.load_error().is_some(), "{what} went unnoticed");
        assert!(host.clients().is_empty(), "{what}");
    }
}

#[test]
fn a_deleted_or_unreadable_key_refuses_the_saved_state() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let mut host = sealed_host(&store, &keychain);
    create_client(&mut host, "Claude");
    drop(host);

    let key = SealKeyStore::with_backend(Box::new(keychain.clone()));
    key.delete();
    let lost = sealed_host(&store, &keychain);
    assert!(lost.load_error().is_some_and(|e| e.contains("missing")));
    assert!(lost.clients().is_empty());
    drop(lost);

    keychain
        .items
        .write(
            Scope {
                service: "es.nico.degenview.mcp",
                shared_group: false,
            },
            "seal-key-v1",
            "garbage",
        )
        .unwrap();
    let unreadable = sealed_host(&store, &keychain);
    assert!(unreadable.load_error().is_some_and(|e| e.contains("read")));
    assert!(unreadable.clients().is_empty());
}

#[test]
fn every_change_the_app_makes_survives_a_restart() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let mut host = sealed_host(&store, &keychain);
    let token = create_client(&mut host, "Claude");
    let second = create_client(&mut host, "Automation");
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
    host.apply(McpAction::SetPort(50_123));
    host.apply(McpAction::SetEnabled(true));
    host.apply(McpAction::SetEnabled(false));
    host.apply(McpAction::RevokeClient(host.clients()[1].id.clone()));
    // A use recorded and saved must not disturb the seal.
    assert!(host.shared().authenticate(&token).is_some());
    host.last_usage_save = Instant::now() - USAGE_SAVE_INTERVAL - Duration::from_secs(1);
    host.tick();
    let expected = (
        host.clients(),
        host.global_permissions(),
        host.settings().clone(),
    );
    drop(host);

    let reloaded = sealed_host(&store, &keychain);
    assert!(
        reloaded.load_error().is_none(),
        "{:?}",
        reloaded.load_error()
    );
    assert_eq!(
        (
            reloaded.clients(),
            reloaded.global_permissions(),
            reloaded.settings().clone()
        ),
        expected
    );
    assert!(reloaded.shared().authenticate(&token).is_some());
    assert!(reloaded.shared().authenticate(&second).is_none());
}

#[test]
fn credentials_made_before_sealing_existed_are_adopted_once() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    // An older build: no sealing.
    let mut old = McpHost::load_with_keys(store.clone(), Arc::new(|| {}), None);
    let token = create_client(&mut old, "Claude");
    drop(old);
    assert_eq!(store.lock().unwrap().mcp_seal().unwrap(), None);

    let upgraded = sealed_host(&store, &keychain);
    assert!(upgraded.load_error().is_none());
    assert!(upgraded.shared().authenticate(&token).is_some());
    assert!(store.lock().unwrap().mcp_seal().unwrap().is_some());
    drop(upgraded);

    // From then on a planted credential is caught.
    plant_client(&store, "attacker-token");
    assert!(sealed_host(&store, &keychain).load_error().is_some());
}

#[test]
fn a_state_that_was_never_configured_needs_no_seal_to_be_trusted() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    drop(sealed_host(&store, &keychain));
    store
        .lock()
        .unwrap()
        .remove_setting(crate::store::mcp::SEAL_KEY)
        .unwrap();
    let host = sealed_host(&store, &keychain);
    assert!(host.load_error().is_none());
    assert!(store.lock().unwrap().mcp_seal().unwrap().is_some());
}

#[test]
fn resetting_access_recovers_and_revokes_everything() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let mut host = sealed_host(&store, &keychain);
    create_client(&mut host, "Claude");
    drop(host);
    plant_client(&store, "attacker-token");

    let mut host = sealed_host(&store, &keychain);
    assert!(host.load_error().is_some());
    host.apply(McpAction::ResetAccess);

    assert!(host.load_error().is_none());
    assert!(!host.can_reset());
    assert!(host.clients().is_empty());
    assert!(!host.settings().enabled);
    assert!(host.shared().authenticate("attacker-token").is_none());
    assert!(host.notice().is_some_and(|n| n.contains("reset")));
    drop(host);

    // It starts sealed again: the next restart trusts it, and a new edit is caught.
    assert!(sealed_host(&store, &keychain).load_error().is_none());
    plant_client(&store, "attacker-token");
    assert!(sealed_host(&store, &keychain).load_error().is_some());
}

#[test]
fn reset_is_ignored_unless_the_state_failed_its_check() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let mut host = sealed_host(&store, &keychain);
    create_client(&mut host, "Claude");
    host.apply(McpAction::ResetAccess);
    assert_eq!(host.clients().len(), 1);
}

#[test]
fn with_no_keyring_it_runs_unsealed_and_says_so() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    keychain.refuse_writes.store(true, Ordering::Relaxed);
    let mut host = sealed_host(&store, &keychain);
    assert!(host.load_error().is_none());
    assert!(host.notice().is_some_and(|n| n.contains("protected")));
    assert!(host.sealed_by.is_none());
    let token = create_client(&mut host, "Claude");
    drop(host);
    assert_eq!(store.lock().unwrap().mcp_seal().unwrap(), None);
    let again = sealed_host(&store, &keychain);
    assert!(again.shared().authenticate(&token).is_some());
}

#[test]
fn a_distributed_build_ignores_the_variable_that_turns_the_seal_off() {
    // (test, debug_build, dev_feature, env_set)
    assert!(
        sealing_disabled(true, false, false, false),
        "unit tests run unsealed"
    );
    assert!(
        sealing_disabled(false, true, false, false),
        "debug builds never prompt for the seal key"
    );
    assert!(
        sealing_disabled(false, false, true, true),
        "a dev-unsealed-mcp release build honours the variable"
    );
    assert!(
        !sealing_disabled(false, false, true, false),
        "but only when it is set"
    );
    assert!(
        !sealing_disabled(false, false, false, true),
        "a release build seals even with DEGENVIEW_NO_KEYCHAIN set"
    );
    assert!(!sealing_disabled(false, false, false, false));
}

#[test]
fn a_reset_the_keychain_refuses_clears_nothing() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let mut host = sealed_host(&store, &keychain);
    create_client(&mut host, "Claude");
    drop(host);
    // The key becomes unreadable: the case that offers the reset.
    let scope = Scope {
        service: "es.nico.degenview.mcp",
        shared_group: false,
    };
    keychain
        .items
        .write(scope, "seal-key-v1", "garbage")
        .unwrap();
    keychain.refuse_writes.store(true, Ordering::Relaxed);

    let mut host = sealed_host(&store, &keychain);
    assert!(host.can_reset());
    host.apply(McpAction::ResetAccess);

    assert!(
        host.notice()
            .is_some_and(|n| n.contains("nothing was changed"))
    );
    assert!(host.can_reset(), "the user can try again");
    {
        let store = store.lock().unwrap();
        assert_eq!(store.mcp_clients().unwrap().len(), 1, "credentials kept");
        assert!(store.mcp_seal().unwrap().is_some());
    }

    // Once the Keychain cooperates, the same state recovers by restarting it.
    keychain.refuse_writes.store(false, Ordering::Relaxed);
    keychain.items.delete(scope, "seal-key-v1").unwrap();
    drop(host);
    let mut host = sealed_host(&store, &keychain);
    host.apply(McpAction::ResetAccess);
    assert!(host.load_error().is_none());
    assert!(host.clients().is_empty());
}

#[test]
fn resetting_from_an_unreadable_key_replaces_it() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let mut host = sealed_host(&store, &keychain);
    create_client(&mut host, "Claude");
    drop(host);
    let scope = Scope {
        service: "es.nico.degenview.mcp",
        shared_group: false,
    };
    keychain
        .items
        .write(scope, "seal-key-v1", "garbage")
        .unwrap();

    let mut host = sealed_host(&store, &keychain);
    assert!(host.load_error().is_some_and(|e| e.contains("read")));
    host.apply(McpAction::ResetAccess);

    assert!(host.load_error().is_none(), "{:?}", host.load_error());
    assert!(host.clients().is_empty());
    assert_ne!(
        keychain.items.read(scope, "seal-key-v1").as_deref(),
        Some("garbage"),
        "the old item is gone"
    );
    drop(host);
    assert!(sealed_host(&store, &keychain).load_error().is_none());
}

#[test]
fn a_read_that_fails_after_a_good_write_does_not_stop_the_reset() {
    let (store, keychain) = (store(), Arc::new(SharedKeychain::default()));
    let mut host = sealed_host(&store, &keychain);
    create_client(&mut host, "Claude");
    drop(host);
    let scope = Scope {
        service: "es.nico.degenview.mcp",
        shared_group: false,
    };
    let old_key = keychain.items.read(scope, "seal-key-v1");

    // The keyring locks: the item exists but cannot be read, so the saved state is refused.
    keychain.refuse_reads.store(true, Ordering::Relaxed);
    let mut host = sealed_host(&store, &keychain);
    assert!(host.can_reset());
    host.apply(McpAction::ResetAccess);

    // The write went through, so the key changed and the reset ran to the end. It must not
    // claim nothing happened, and the old credentials are gone, not stranded.
    assert_ne!(keychain.items.read(scope, "seal-key-v1"), old_key);
    assert!(
        host.notice()
            .is_none_or(|n| !n.contains("nothing was changed"))
    );
    assert!(store.lock().unwrap().mcp_clients().unwrap().is_empty());
    assert!(
        host.can_reset(),
        "still unreadable, so Reset stays available"
    );

    // Once the keyring answers again, a restart loads the empty state sealed and clean.
    keychain.refuse_reads.store(false, Ordering::Relaxed);
    drop(host);
    let host = sealed_host(&store, &keychain);
    assert!(host.load_error().is_none(), "{:?}", host.load_error());
    assert!(host.clients().is_empty());
}

#[test]
fn confirmation_summaries_name_the_alert_the_chart_the_drawings_and_the_script() {
    use crate::host::mcp::confirmation as describe;
    const SOURCE: &str = "//@version=6\nindicator(\"My script\")\nplot(close)\n";

    let (_store, host, id) = seeded();
    let alert = describe::delete_alert(&host, id).unwrap();
    assert_eq!(alert.action, "Delete a price alert");
    assert!(
        alert.target.contains("crosses above") && alert.target.contains("70000"),
        "{}",
        alert.target
    );
    assert_eq!(alert.note.as_deref(), Some("Its trigger history is kept."));
    assert!(matches!(
        describe::delete_alert(&host, id + 1),
        Err(McpError::NotFound(_))
    ));

    let ctx = eframe::egui::Context::default();
    let mut workspace = empty_workspace(3);
    let chart_id = workspace
        .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
        .chart_id;
    let chart = describe::remove_chart(&workspace, chart_id).unwrap();
    assert_eq!(chart.action, "Remove a chart");
    assert!(chart.target.starts_with("BTCUSDT in “"), "{}", chart.target);
    assert!(chart.note.unwrap().contains("drawings go with it"));
    assert!(matches!(
        describe::remove_chart(&workspace, chart_id + 99),
        Err(McpError::NotFound(_))
    ));
    // A drawing that is not there is refused unasked, as the removal itself would refuse it.
    assert!(matches!(
        describe::remove_drawings(&workspace, chart_id, &[42]),
        Err(McpError::NotFound(_))
    ));

    let script = ScriptToApply {
        script_id: "s1".into(),
        name: "My script".into(),
        source: SOURCE.into(),
        inputs: Default::default(),
    };
    let added = add_pine_instance(&mut workspace, chart_id, &script, true).unwrap();
    let instance: u64 = added.instance_id.parse().unwrap();
    let removed = describe::remove_pine_instance(&workspace, chart_id, instance, &[]).unwrap();
    assert_eq!(removed.action, "Remove a script from a chart");
    assert!(
        removed.target.starts_with("My script on BTCUSDT"),
        "{}",
        removed.target
    );
    assert!(matches!(
        describe::remove_pine_instance(&workspace, chart_id, instance + 1, &[]),
        Err(McpError::NotFound(_))
    ));
}
