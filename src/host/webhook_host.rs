//! Host for the app-level Settings window: owns the window state, the Keychain secret store and
//! the HTTP transport, and applies the webhook and storage actions the pure
//! `settings_window`/`webhook_settings`/`settings_storage` views return. Actions that reach outside the webhook
//! store (notifications, login items, alert reference bookkeeping) are handed back to the caller.

use crate::model::alert::{AlertNotificationSettings, AlertRuntimeHealth};
use crate::model::webhook::{EndpointId, WebhookDeliveryResult, WebhookEndpoint};
use crate::service::cache_control;
use crate::service::disk_cache::{CacheStats, ClearOutcome};
use crate::service::keychain_webhook_secrets::KeychainWebhookSecretStore;
use crate::service::llm::prefs::AiPrefs;
use crate::service::webhook_delivery::{self, Mode, UreqTransport, WebhookSecretStore};
use crate::store::Store;
use crate::ui::settings_api_keys::StatusMessage;
use crate::ui::settings_snapshots::SnapshotsAction;
use crate::ui::settings_storage::StorageAction;
use crate::ui::settings_window::{self, SettingsAction, SettingsInput, SettingsState};
use crate::ui::webhook_settings::{WebhookSettingsAction, WebhookSettingsInput};
use eframe::egui;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

/// Something the settings window asked for that the webhook store cannot do on its own.
pub enum WebhookHostAction {
    UpdateNotifications(AlertNotificationSettings),
    RetryRegistration,
    OpenLoginItems,
    OpenNotificationSettings,
    /// An endpoint was deleted: remove its id from every alert that references it.
    EndpointDeleted(EndpointId),
    /// The MCP Server page asked for a change; the app queues it on the MCP host.
    Mcp(crate::model::mcp::McpAction),
    /// The About page installed an update; quit so the helper can reopen the app.
    QuitForUpdate,
}

/// What the Storage page's background work reports back, on the UI thread's next frame.
enum StorageEvent {
    Stats(CacheStats),
    Cleared(ClearOutcome),
}

struct StorageEvents {
    sender: mpsc::Sender<StorageEvent>,
    receiver: Receiver<StorageEvent>,
}

impl Default for StorageEvents {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self { sender, receiver }
    }
}

#[derive(Default)]
pub struct WebhookHost {
    /// The snapshot choices the Snapshots page edits.
    snapshots: crate::host::snapshot_host::SnapshotHost,
    /// The saved assistant settings, read from the store when Settings first needs them.
    ai_prefs: Option<AiPrefs>,
    state: SettingsState,
    storage_events: StorageEvents,
    /// Shared with the threads that run Test, so a slow endpoint never blocks the UI.
    secrets: Arc<KeychainWebhookSecretStore>,
    transport: Arc<UreqTransport>,
    /// Tests still in flight, by endpoint; each sends its one result when it finishes.
    tests: Vec<(EndpointId, Receiver<WebhookDeliveryResult>)>,
    open: bool,
}

impl WebhookHost {
    pub fn new(snapshots: crate::host::snapshot_host::SnapshotHost) -> Self {
        Self {
            snapshots,
            ..Self::default()
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// Opens Settings on the Alpaca page (the Script Manager's "Open Settings").
    pub fn open_alpaca(&mut self) {
        self.state.section = crate::ui::settings_window::SettingsSection::Alpaca;
        self.open = true;
    }

    /// Opens Settings on the Webhooks page (an alert editor's "Add in Settings").
    pub fn open_webhooks(&mut self) {
        self.state.section = crate::ui::settings_window::SettingsSection::Webhooks;
        self.open = true;
    }

    /// Renders the Settings window and applies webhook actions in place. `reference_count`
    /// answers "how many alerts use this endpoint" for the delete confirmation; the alert host
    /// supplies it (returns 0 until it exists). Returns the out-of-store actions for the caller.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        store: &Store,
        notifications: &AlertNotificationSettings,
        health: &AlertRuntimeHealth,
        mcp: crate::ui::settings_mcp::McpSettingsInput,
        updates: &crate::host::update_host::UpdateHost,
        sync: &crate::host::sync_host::SyncHost,
        now_ms: i64,
        reference_count: &dyn Fn(EndpointId) -> usize,
    ) -> Vec<WebhookHostAction> {
        if !self.open {
            return Vec::new();
        }
        self.collect_test_results(ui.ctx());
        self.collect_storage_events();
        let mut out = Vec::new();
        // The page fills the native Settings window itself: no egui window inside it, which
        // drew a second title bar and let the page be dragged around within its own window.
        let endpoints = store.webhook_endpoints();
        let snapshot_prefs = self.snapshots.prefs();
        let ai_prefs = self
            .ai_prefs
            .get_or_insert_with(|| AiPrefs::load(store))
            .clone();
        let input = SettingsInput {
            notifications,
            health,
            webhooks: WebhookSettingsInput {
                endpoints: &endpoints,
                has_secret: &|id| self.secrets.has_secret(id),
                secret_for_edit: &|id| self.secrets.secret(id),
                reference_count,
                load_failed: false,
            },
            mcp,
            snapshots: crate::ui::settings_snapshots::SnapshotsInput {
                prefs: &snapshot_prefs,
                token_saved: crate::service::keychain_snapshot_token::SnapshotTokenStore::shared()
                    .is_configured(),
            },
            ai: crate::ui::settings_ai::AiSettingsInput { prefs: &ai_prefs },
            updates,
            sync,
            now_ms,
        };
        for action in settings_window::show(ui, &mut self.state, &input) {
            match action {
                SettingsAction::Webhook(action) => {
                    self.apply_webhook(action, store, &endpoints, now_ms, &mut out);
                }
                SettingsAction::Storage(action) => self.apply_storage(action, ui.ctx()),
                SettingsAction::Snapshots(action) => self.apply_snapshots(action, store),
                SettingsAction::Ai(crate::ui::settings_ai::AiAction::SavePrefs(prefs)) => {
                    if let Err(error) = prefs.save(store) {
                        self.state.ai.status = Some(StatusMessage::error(format!(
                            "Could not save the settings: {error}"
                        )));
                    }
                    self.ai_prefs = Some(prefs);
                }
                SettingsAction::SetTheme(theme) => {
                    let _ = store.set_setting(crate::ui::theme::THEME_KEY, &theme);
                    crate::ui::theme::apply(ui.ctx(), theme);
                }
                SettingsAction::UpdateNotifications(settings) => {
                    out.push(WebhookHostAction::UpdateNotifications(settings));
                }
                SettingsAction::RetryRegistration => {
                    out.push(WebhookHostAction::RetryRegistration);
                }
                SettingsAction::OpenLoginItems => {
                    out.push(WebhookHostAction::OpenLoginItems);
                }
                SettingsAction::OpenNotificationSettings => {
                    out.push(WebhookHostAction::OpenNotificationSettings);
                }
                SettingsAction::Mcp(action) => out.push(WebhookHostAction::Mcp(action)),
                SettingsAction::QuitForUpdate => out.push(WebhookHostAction::QuitForUpdate),
                SettingsAction::SetUpdateChannel(channel) => {
                    updates.set_channel(store, ui.ctx(), channel);
                }
            }
        }
        out
    }

    /// Records the results of finished tests, and keeps the window repainting while any are
    /// still running so their rows update without the pointer moving.
    fn collect_test_results(&mut self, ctx: &egui::Context) {
        let webhooks = &mut self.state.webhooks;
        self.tests
            .retain(|(id, receiver)| match receiver.try_recv() {
                Ok(result) => {
                    webhooks.set_test_result(&result);
                    false
                }
                Err(TryRecvError::Empty) => true,
                // The thread ended without an answer: the row stops saying "Testing…".
                Err(TryRecvError::Disconnected) => {
                    webhooks.forget(*id);
                    false
                }
            });
        if !self.tests.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Applies what the Storage page finished: fresh sizes, or a clear that has completed.
    fn collect_storage_events(&mut self) {
        while let Ok(event) = self.storage_events.receiver.try_recv() {
            match event {
                StorageEvent::Stats(stats) => self.state.storage.stats = Some(stats),
                StorageEvent::Cleared(outcome) => self.state.storage.finish_clear(outcome),
            }
        }
    }

    /// Applies what the Snapshots page changed. A failure is said on the page.
    fn apply_snapshots(&mut self, action: SnapshotsAction, store: &Store) {
        use crate::service::keychain_snapshot_token::SnapshotTokenStore;
        let result = match action {
            SnapshotsAction::Save { host, token } => self
                .snapshots
                .set_host(Some(store), &host)
                .and_then(|()| match token {
                    Some(token) => SnapshotTokenStore::shared()
                        .save(&token)
                        .map_err(|error| error.to_string()),
                    None => Ok(()),
                })
                .map(|()| {
                    self.state.snapshots.saved_token();
                    "Saved".to_owned()
                }),
            SnapshotsAction::RemoveToken => {
                SnapshotTokenStore::shared().remove();
                Ok("Token removed".to_owned())
            }
            SnapshotsAction::IncludeTrading(include) => self
                .snapshots
                .set_include_trading(Some(store), include)
                .map(|()| String::new()),
            SnapshotsAction::Scale(scale) => self
                .snapshots
                .set_scale(Some(store), scale)
                .map(|()| String::new()),
            SnapshotsAction::ForgetConsent => self
                .snapshots
                .forget_consent(Some(store))
                .map(|()| "Approved hosts forgotten".to_owned()),
        };
        self.state.snapshots.status = match result {
            Ok(text) if text.is_empty() => None,
            Ok(text) => Some(StatusMessage::ok(text)),
            Err(error) => Some(StatusMessage::error(error)),
        };
        if self
            .state
            .snapshots
            .status
            .as_ref()
            .is_some_and(|status| !status.is_error)
        {
            self.state.snapshots.saved_token();
        }
    }

    /// Walking and deleting the cache folder are disk work: both run off the UI thread and
    /// report back through `storage_events`.
    fn apply_storage(&mut self, action: StorageAction, ctx: &egui::Context) {
        let sender = self.storage_events.sender.clone();
        let repaint = ctx.clone();
        match action {
            StorageAction::Refresh => cache_control::stats_in_background(move |stats| {
                let _ = sender.send(StorageEvent::Stats(stats));
                repaint.request_repaint();
            }),
            StorageAction::Clear => cache_control::clear_in_background(ctx, move |outcome| {
                let _ = sender.send(StorageEvent::Cleared(outcome));
                let _ = sender.send(StorageEvent::Stats(cache_control::stats()));
                repaint.request_repaint();
            }),
        }
    }

    fn apply_webhook(
        &mut self,
        action: WebhookSettingsAction,
        store: &Store,
        endpoints: &[WebhookEndpoint],
        now_ms: i64,
        out: &mut Vec<WebhookHostAction>,
    ) {
        match action {
            WebhookSettingsAction::Test { endpoint, message } => {
                // The request can take its full 3 s timeout, so it runs on its own thread; the
                // row shows "Testing…" until `collect_test_results` hands the answer back.
                let id = endpoint.id;
                let (sender, receiver) = mpsc::channel();
                let transport = Arc::clone(&self.transport);
                let secrets = Arc::clone(&self.secrets);
                std::thread::spawn(move || {
                    let result = webhook_delivery::deliver(
                        &*transport,
                        &*secrets,
                        &message,
                        &endpoint,
                        Mode::Test,
                    );
                    let _ = sender.send(result);
                });
                self.tests.push((id, receiver));
            }
            WebhookSettingsAction::Toggle(id) => {
                let mut next = endpoints.to_vec();
                for endpoint in &mut next {
                    if endpoint.id == id {
                        endpoint.is_enabled = !endpoint.is_enabled;
                    }
                }
                if let Err(error) = store.replace_webhook_endpoints(&next) {
                    self.state.webhooks.save_failed(error.to_string());
                } else {
                    self.state.webhooks.save_succeeded();
                }
            }
            WebhookSettingsAction::Delete(id) => {
                let mut next = endpoints.to_vec();
                next.retain(|endpoint| endpoint.id != id);
                if let Err(error) = store.replace_webhook_endpoints(&next) {
                    self.state.webhooks.save_failed(error.to_string());
                } else {
                    self.secrets.remove(id);
                    out.push(WebhookHostAction::EndpointDeleted(id));
                }
            }
            WebhookSettingsAction::Save(result) => self.save(result, store, endpoints, now_ms),
        }
    }

    /// Creates (`id == None`) or updates an endpoint, then writes the secret. On a row-write
    /// failure the sheet stays open; on a secret-write failure the row write is rolled back
    /// (Swift parity).
    fn save(
        &mut self,
        result: crate::ui::webhook_editor::WebhookEditorResult,
        store: &Store,
        endpoints: &[WebhookEndpoint],
        now_ms: i64,
    ) {
        let original = endpoints.to_vec();
        let mut next = endpoints.to_vec();
        let target_id = match result.id {
            Some(id) => {
                for endpoint in &mut next {
                    if endpoint.id == id {
                        endpoint.name = result.name.clone();
                        endpoint.description = result.description.clone();
                        endpoint.url = result.url.clone();
                        endpoint.method = result.method;
                        endpoint.headers = result.headers.clone();
                        endpoint.is_enabled = result.is_enabled;
                        endpoint.updated_at_ms = now_ms;
                    }
                }
                id
            }
            None => {
                let mut endpoint =
                    WebhookEndpoint::new(result.name.clone(), result.url.clone(), now_ms);
                endpoint.description = result.description.clone();
                endpoint.method = result.method;
                endpoint.headers = result.headers.clone();
                endpoint.is_enabled = result.is_enabled;
                let id = endpoint.id;
                next.push(endpoint);
                id
            }
        };

        if let Err(error) = store.replace_webhook_endpoints(&next) {
            self.state.webhooks.save_failed(error.to_string());
            return;
        }
        let secret_written = if result.secret.is_empty() {
            self.secrets.remove(target_id);
            Ok(())
        } else {
            self.secrets.set_secret(target_id, &result.secret)
        };
        match secret_written {
            Ok(()) => self.state.webhooks.save_succeeded(),
            Err(error) => {
                let _ = store.replace_webhook_endpoints(&original);
                self.state.webhooks.save_failed(error.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::webhook::WebhookHttpMethod;
    use crate::ui::webhook_settings::WebhookTestState;
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
        let mut host = WebhookHost::default();
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
        let mut host = WebhookHost::default();
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

    /// The Settings window draws with the store locked, and every change on the Snapshots page
    /// is saved from there: saving must use that lock, not wait for it (which never ends).
    #[test]
    fn a_snapshot_setting_changed_in_settings_is_saved_without_locking_the_store_twice() {
        use crate::model::snapshot::{SnapshotPrefs, SnapshotScale};
        let shared: crate::store::SharedStore = Arc::new(std::sync::Mutex::new(
            Store::open_in_memory().expect("a store"),
        ));
        let snapshots = crate::host::snapshot_host::SnapshotHost::load(shared.clone());
        let mut host = WebhookHost::new(snapshots.clone());
        let (done, finished) = mpsc::channel();
        let worker_store = shared.clone();
        std::thread::spawn(move || {
            let store = worker_store.lock().expect("the store");
            for action in [
                SnapshotsAction::IncludeTrading(true),
                SnapshotsAction::Scale(SnapshotScale::X1),
                SnapshotsAction::Save {
                    host: "https://snap.example.com/up".into(),
                    token: None,
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
        assert_eq!(prefs.host.as_deref(), Some("https://snap.example.com/up"));
        // It reached the database too, not only the host's memory.
        let saved: SnapshotPrefs = shared
            .lock()
            .unwrap()
            .setting(SnapshotPrefs::KEY)
            .expect("a saved setting");
        assert_eq!(saved, prefs);
    }
}
