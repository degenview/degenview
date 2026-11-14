use super::*;
use crate::model::webhook::WebhookHttpMethod;
use crate::ui::webhook::settings::WebhookTestState;
use std::time::Instant;

#[test]
fn a_test_runs_off_the_ui_thread_and_its_result_lands_later() {
    let store = Store::open_in_memory().unwrap();
    let endpoint = WebhookEndpoint {
        id: 7,
        name: "Broken".into(),
        description: String::new(),
        url: "not a url".into(),
        method: WebhookHttpMethod::Post,
        headers: Vec::new(),
        is_enabled: true,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    let mut host = SettingsHost::default();
    let mut out = Vec::new();
    host.apply_webhook(
        WebhookSettingsAction::Test {
            endpoint: Box::new(endpoint.clone()),
            message: "{}".into(),
        },
        &store,
        std::slice::from_ref(&endpoint),
        0,
        &mut out,
    );
    assert_eq!(host.tests.len(), 1, "the request is in flight, not awaited");

    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !host.tests.is_empty() && Instant::now() < deadline {
        host.collect_test_results(&ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(host.tests.is_empty());
    assert!(matches!(
        host.state.webhooks.test_state(7),
        WebhookTestState::Failed { .. }
    ));
}

#[test]
fn failed_delete_keeps_the_secret_and_reference_action() {
    let path = std::env::temp_dir().join(format!(
        "degenview-webhook-delete-{}.sqlite",
        crate::model::watchlist::new_id()
    ));
    let store = Store::open(&path).expect("open");
    let endpoint = WebhookEndpoint::new("Test", "https://example.com/hook", 1_000);
    store
        .replace_webhook_endpoints(std::slice::from_ref(&endpoint))
        .expect("seed");
    rusqlite::Connection::open(&path)
        .expect("second connection")
        .execute_batch(
            "CREATE TRIGGER reject_webhook_delete BEFORE DELETE ON webhook_endpoint
         BEGIN SELECT RAISE(ABORT, 'delete failed'); END;",
        )
        .expect("trigger");
    let mut host = SettingsHost::default();
    host.secrets
        .set_secret(endpoint.id, "secret")
        .expect("secret");
    let mut actions = Vec::new();
    host.apply_webhook(
        WebhookSettingsAction::Delete(endpoint.id),
        &store,
        std::slice::from_ref(&endpoint),
        1_000,
        &mut actions,
    );
    assert!(actions.is_empty());
    assert!(host.secrets.has_secret(endpoint.id));
    assert_eq!(store.webhook_endpoints(), vec![endpoint]);
    drop(store);
    let _ = std::fs::remove_file(path);
}

/// `SettingsHost::new` fills the rest of its fields from `Self::default()`, which builds a throwaway
/// `EditorThemeHost`. That must not change what the editors draw: at launch it replaced the saved
/// theme with Automatic before the first frame.
#[test]
fn building_the_host_leaves_the_editors_theme_alone() {
    use crate::host::editor_theme::EditorThemeHost;
    use crate::model::editor_theme::{BuiltInTheme, EditorPrefs, ThemeSelection};
    use crate::service::editor_theme::fonts::FontCatalog;
    use crate::ui::editor_appearance::{RestorePublished, current, of_built_in};
    let _published = RestorePublished;
    let store = Store::open_in_memory().expect("a store");
    store
        .set_editor_prefs(&EditorPrefs {
            selected: ThemeSelection::BuiltIn(BuiltInTheme::Dracula),
            ..EditorPrefs::default()
        })
        .unwrap();
    let themes = EditorThemeHost::new(
        store.load_editor_themes(),
        FontCatalog::with_entries(Vec::new()),
    );
    assert!(Arc::ptr_eq(&current(), &of_built_in(BuiltInTheme::Dracula)));
    let _host = SettingsHost::new(Default::default(), themes, Default::default());
    assert!(
        Arc::ptr_eq(&current(), &of_built_in(BuiltInTheme::Dracula)),
        "SettingsHost::new replaced the saved editor theme"
    );
}

/// The Settings window draws with the store locked, and every change on the Snapshots page
/// is saved from there: saving must use that lock, not wait for it (which never ends).
#[test]
fn a_snapshot_setting_changed_in_settings_is_saved_without_locking_the_store_twice() {
    use crate::model::chart::snapshot::{SELF_HOSTED, SnapshotPrefs, SnapshotScale};
    let shared: crate::store::SharedStore = Arc::new(std::sync::Mutex::new(
        Store::open_in_memory().expect("a store"),
    ));
    let snapshots = crate::host::snapshot::SnapshotHost::load(shared.clone());
    let mut host = SettingsHost::new(snapshots.clone(), Default::default(), Default::default());
    let (done, finished) = mpsc::channel();
    let worker_store = shared.clone();
    std::thread::spawn(move || {
        let store = worker_store.lock().expect("the store");
        for action in [
            SnapshotsAction::IncludeTrading(true),
            SnapshotsAction::Scale(SnapshotScale::X1),
            SnapshotsAction::SetProvider(Some(SELF_HOSTED)),
            SnapshotsAction::Save {
                provider: SELF_HOSTED,
                fields: std::collections::BTreeMap::from([(
                    "host".to_owned(),
                    "https://snap.example.com/up".to_owned(),
                )]),
                secrets: Default::default(),
            },
            SnapshotsAction::ForgetConsent,
        ] {
            host.apply_snapshots(action, &store);
        }
        let _ = done.send(host.state.snapshots.status.take());
    });
    let status = finished
        .recv_timeout(Duration::from_secs(5))
        .expect("the settings did not wait on a lock they already hold");
    assert!(status.is_none_or(|status| !status.is_error));
    let prefs = snapshots.prefs();
    assert!(prefs.include_trading);
    assert_eq!(prefs.scale, SnapshotScale::X1);
    assert_eq!(prefs.provider.as_deref(), Some(SELF_HOSTED));
    assert_eq!(
        prefs
            .settings_of(SELF_HOSTED)
            .get("host")
            .map(String::as_str),
        Some("https://snap.example.com/up")
    );
    // It reached the database too, not only the host's memory.
    let saved: SnapshotPrefs = shared
        .lock()
        .unwrap()
        .setting(SnapshotPrefs::KEY)
        .expect("a saved setting");
    assert_eq!(saved, prefs);
}
