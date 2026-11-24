//! Controller for the app-level Settings window: owns the window state, the Keychain secret store
//! and the HTTP transport, builds what the pure `ui::settings::window` view shows, and applies the
//! actions it returns. It draws nothing itself: the caller hands it the view to run. Actions that
//! reach outside the settings it owns (notifications, login items, alert reference bookkeeping) are
//! handed back to the caller.

mod paint;

pub use paint::SettingsPaintData;

use crate::graphics::prefs::GraphicsPrefs;
use crate::host::alert::AlertHost;
use crate::host::mcp::McpHost;
use crate::i18n::tr;
use crate::model::alert::{AlertNotificationSettings, AlertRuntimeHealth};
use crate::model::webhook::{EndpointId, WebhookDeliveryResult, WebhookEndpoint};
use crate::service::cache::control;
use crate::service::cache::disk::{CacheStats, ClearOutcome};
use crate::service::keychain::webhook_secrets::KeychainWebhookSecretStore;
use crate::service::llm::prefs::AiPrefs;
use crate::service::net::proxy::ProxyConfig;
use crate::service::net::proxy::{self, ApplyReport};
use crate::service::webhook::delivery::{self, Mode, UreqTransport, WebhookSecretStore};
use crate::service::work_queue::{self, Pending};
use crate::store::Store;
use crate::store::frame_cache::ChangeCached;
use crate::ui::settings::card::StatusMessage;
use crate::ui::settings::graphics::GraphicsAction;
use crate::ui::settings::network::NetworkAction;
use crate::ui::settings::snapshots::SnapshotsAction;
use crate::ui::settings::storage::StorageAction;
use crate::ui::settings::window::{SettingsAction, SettingsInput, SettingsSection, SettingsState};
use crate::ui::webhook::settings::{WebhookSettingsAction, WebhookSettingsInput};
use eframe::egui;
use paint::McpPaintData;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Something the Settings window asked for that only the caller can do.
pub enum SettingsHostAction {
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
    /// Graphics prefs were saved; the app should apply the surface config on the next frame.
    ApplyGraphics(GraphicsPrefs),
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
pub struct SettingsHost {
    /// The snapshot choices the Snapshots page edits.
    snapshots: crate::host::snapshot::SnapshotHost,
    /// The editor's themes and typography, which the Pine Studio page edits.
    editor_themes: crate::host::editor_theme::EditorThemeHost,
    /// How chart tabs name themselves, which the Appearance page edits.
    tab_titles: crate::host::tab::title::TabTitleHost,
    /// The saved assistant settings, read from the store when Settings first needs them.
    ai_prefs: Option<AiPrefs>,
    ai_prefs_read_at: Option<std::time::Instant>,
    /// The `net.proxy` setting the Network page shows, read again only after a write.
    network_config: ChangeCached<ProxyConfig>,
    /// The `graphics.prefs` setting the Graphics page shows.
    graphics_prefs: ChangeCached<GraphicsPrefs>,
    /// Adapter / backend lines for the Graphics page (filled by the app at start).
    graphics_adapter: String,
    graphics_active_backend: String,
    /// When the snapshot choices were last read again, for what iCloud Sync applied meanwhile.
    snapshots_read_at: Option<std::time::Instant>,
    state: SettingsState,
    storage_events: StorageEvents,
    /// Shared with the jobs that run Test, so a slow endpoint never blocks the UI.
    secrets: Arc<KeychainWebhookSecretStore>,
    transport: Arc<UreqTransport>,
    /// Tests still in flight, by endpoint; each sends its one result when it finishes.
    tests: Vec<(EndpointId, Pending<WebhookDeliveryResult>)>,
    open: bool,
    /// Host snapshots reused across pointer-move frames (see `paint`).
    paint: Option<SettingsPaintData>,
}

impl SettingsHost {
    pub fn new(
        snapshots: crate::host::snapshot::SnapshotHost,
        editor_themes: crate::host::editor_theme::EditorThemeHost,
        tab_titles: crate::host::tab::title::TabTitleHost,
    ) -> Self {
        Self {
            snapshots,
            editor_themes,
            tab_titles,
            ..Self::default()
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// Opens Settings on the Alpaca page (the Pine Studio's "Open Settings").
    pub fn open_alpaca(&mut self) {
        self.state.section = crate::ui::settings::window::SettingsSection::Alpaca;
        self.open = true;
    }

    /// Opens Settings on the Brokers page (the live trading panel's "Open Settings").
    pub fn open_brokers(&mut self) {
        self.state.section = crate::ui::settings::window::SettingsSection::Brokers;
        self.open = true;
    }

    /// Opens Settings on the AI Assistant page (the assistant panel's "Open Settings").
    pub fn open_ai(&mut self) {
        self.state.section = crate::ui::settings::window::SettingsSection::Ai;
        self.open = true;
    }

    /// Opens Settings on the Snapshots page (the chart camera menu's "Configure Snapshots…").
    pub fn open_snapshots(&mut self) {
        self.state.section = crate::ui::settings::window::SettingsSection::Snapshots;
        self.open = true;
    }

    /// Opens Settings on the Pine Studio page, where the editor's themes are (the Pine Studio's
    /// Theme menu, "Configure Themes…").
    pub fn open_editor_themes(&mut self) {
        self.state.section = crate::ui::settings::window::SettingsSection::PineStudio;
        self.open = true;
    }

    /// Opens Settings on the Webhooks page (an alert editor's "Add in Settings").
    pub fn open_webhooks(&mut self) {
        self.state.section = crate::ui::settings::window::SettingsSection::Webhooks;
        self.open = true;
    }

    /// Fills the Graphics page's "This computer" lines from the adapter chosen at start.
    pub fn set_graphics_info(
        &mut self,
        adapter: impl Into<String>,
        active_backend: impl Into<String>,
    ) {
        self.graphics_adapter = adapter.into();
        self.graphics_active_backend = active_backend.into();
    }

    /// The page currently showing (for section-gated host snapshots).
    pub fn section(&self) -> SettingsSection {
        self.state.section
    }

    /// Refreshes the paint snapshot when the section changed or the cache went stale. Heavy MCP
    /// and webhook clones only run for those pages.
    pub fn prepare_paint(&mut self, store: &Store, alerts: &AlertHost, mcp: &McpHost) {
        let section = self.state.section;
        if self
            .paint
            .as_ref()
            .is_some_and(|paint| paint.serves(section))
        {
            return;
        }

        let need_notes = SettingsPaintData::needs_notifications(section);
        let need_hooks = SettingsPaintData::needs_webhook_refs(section);
        let need_mcp = SettingsPaintData::needs_mcp(section);

        let (notifications, health, alert_list) = if need_notes || need_hooks {
            (
                alerts.settings().clone(),
                alerts.health().clone(),
                if need_hooks {
                    alerts.snapshot().alerts.clone()
                } else {
                    Vec::new()
                },
            )
        } else {
            (
                AlertNotificationSettings::default(),
                AlertRuntimeHealth::default(),
                Vec::new(),
            )
        };

        let subscriptions = if need_hooks {
            Arc::from(store.pine_subscriptions_cached().as_slice())
        } else {
            Arc::from([])
        };

        let mcp = if need_mcp {
            McpPaintData {
                status: mcp.status(),
                settings: mcp.settings().clone(),
                clients: Arc::from(mcp.clients()),
                global: mcp.global_permissions(),
                fresh_token: mcp
                    .fresh_token()
                    .map(|fresh| (fresh.client_name.clone(), fresh.token.clone())),
                load_error: mcp.load_error().map(str::to_owned),
                notice: mcp.notice().map(str::to_owned),
                can_reset: mcp.can_reset(),
                activity: Arc::from(mcp.activity().as_slice()),
                activity_unreadable: mcp.activity_unreadable(),
                tls_fingerprint: mcp.tls_fingerprint().map(str::to_owned),
            }
        } else {
            McpPaintData::default()
        };

        self.paint = Some(SettingsPaintData {
            at: Instant::now(),
            section,
            notifications,
            health,
            alerts: Arc::from(alert_list),
            subscriptions,
            mcp,
        });
    }

    /// Latest paint snapshot; call [`prepare_paint`] first.
    pub fn paint_data(&self) -> Option<&SettingsPaintData> {
        self.paint.as_ref()
    }

    /// Drop the paint cache so the next frame reloads (after a write that other pages must see).
    pub fn invalidate_paint(&mut self) {
        self.paint = None;
    }

    /// Runs one frame of the Settings window: builds its input, hands it with the window state to
    /// `view` (which draws the page and returns what the user did), and applies those actions in
    /// place. `reference_count` answers "how many alerts use this endpoint" for the delete
    /// confirmation; the alert host supplies it. Returns the actions only the caller can apply.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        view: impl FnOnce(&mut SettingsState, &SettingsInput<'_>) -> Vec<SettingsAction>,
        store: &Store,
        notifications: &AlertNotificationSettings,
        health: &AlertRuntimeHealth,
        mcp: crate::ui::settings::mcp::McpSettingsInput,
        updates: &crate::host::update::UpdateHost,
        sync: &crate::host::sync::SyncHost,
        trading_hub: &std::sync::Arc<crate::host::trading::TradingHub>,
        now_ms: i64,
        reference_count: &dyn Fn(EndpointId) -> usize,
    ) -> Vec<SettingsHostAction> {
        if !self.open {
            return Vec::new();
        }
        let section = self.state.section;
        // Only the open page's background work runs; other pages stay idle so pointer moves stay cheap.
        if matches!(section, SettingsSection::Webhooks) || !self.tests.is_empty() {
            self.collect_test_results(ctx);
        }
        if matches!(section, SettingsSection::Storage) {
            self.collect_storage_events();
        }
        if matches!(section, SettingsSection::Network) {
            self.collect_network();
        }
        let mut out = Vec::new();
        // The page fills the native Settings window itself: no egui window inside it, which
        // drew a second title bar and let the page be dragged around within its own window.
        let endpoints = if matches!(section, SettingsSection::Webhooks) {
            store.webhook_endpoints_cached()
        } else {
            Arc::new(Vec::new())
        };
        // Read again every second, like the assistant's: a change another Mac made reaches the page.
        let snapshot_prefs = if matches!(section, SettingsSection::Snapshots) {
            if self
                .snapshots_read_at
                .is_none_or(|at| at.elapsed() > Duration::from_secs(1))
            {
                self.snapshots.refresh(Some(store));
                self.snapshots_read_at = Some(std::time::Instant::now());
            }
            self.snapshots.prefs()
        } else {
            crate::model::chart::snapshot::SnapshotPrefs::default()
        };
        // Read again every second: the assistant panel changes these too, in another window.
        let ai_fallback = AiPrefs::default();
        if matches!(section, SettingsSection::Ai)
            && (self.ai_prefs.is_none()
                || self
                    .ai_prefs_read_at
                    .is_none_or(|at| at.elapsed() > Duration::from_secs(1)))
        {
            self.ai_prefs = Some(AiPrefs::load(store));
            self.ai_prefs_read_at = Some(std::time::Instant::now());
        }
        let ai_prefs = if matches!(section, SettingsSection::Ai) {
            self.ai_prefs.as_ref().unwrap_or(&ai_fallback)
        } else {
            &ai_fallback
        };
        let network_config = if matches!(section, SettingsSection::Network) {
            self.network_config
                .get(store.change_stamp(), || ProxyConfig::load(store))
        } else {
            Arc::new(ProxyConfig::default())
        };
        let graphics_prefs = if matches!(section, SettingsSection::Graphics) {
            *self
                .graphics_prefs
                .get(store.change_stamp(), || GraphicsPrefs::load(store))
        } else {
            GraphicsPrefs::default()
        };
        let proxy_status = if matches!(section, SettingsSection::Network) {
            proxy::status()
        } else {
            proxy::Status {
                mode: network_config.mode,
                active: None,
                problem: None,
            }
        };
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
            snapshots: crate::ui::settings::snapshots::SnapshotsInput {
                prefs: &snapshot_prefs,
                secrets: self.snapshots.secrets(),
            },
            ai: crate::ui::settings::ai::AiSettingsInput { prefs: ai_prefs },
            network: crate::ui::settings::network::NetworkInput {
                config: network_config.as_ref(),
                status: &proxy_status,
            },
            graphics: crate::ui::settings::graphics::GraphicsInput {
                prefs: graphics_prefs,
                adapter: &self.graphics_adapter,
                active_backend: &self.graphics_active_backend,
            },
            updates,
            sync,
            trading_hub,
            editor_themes: &self.editor_themes,
            tab_title_style: if matches!(section, SettingsSection::Appearance) {
                self.tab_titles.style()
            } else {
                Default::default()
            },
            now_ms,
        };
        for action in view(&mut self.state, &input) {
            match action {
                SettingsAction::Webhook(action) => {
                    self.apply_webhook(action, store, &endpoints, now_ms, &mut out);
                }
                SettingsAction::Storage(action) => self.apply_storage(action, ctx),
                SettingsAction::Snapshots(action) => self.apply_snapshots(action, store),
                SettingsAction::Network(action) => self.apply_network(action, store, ctx),
                SettingsAction::Graphics(action) => self.apply_graphics(action, store, &mut out),
                SettingsAction::Ai(crate::ui::settings::ai::AiAction::SavePrefs(prefs)) => {
                    if let Err(error) = prefs.save(store) {
                        self.state.ai.status = Some(StatusMessage::error(tr!(
                            "Could not save the settings: {error}",
                            error = error
                        )));
                    }
                    self.ai_prefs = Some(prefs);
                    self.ai_prefs_read_at = Some(std::time::Instant::now());
                }
                SettingsAction::Ai(crate::ui::settings::ai::AiAction::ClearHistory) => {
                    use crate::pine_studio::ai::history::History;
                    let mut history = History::load(store);
                    history.clear();
                    self.state.ai.status = Some(match history.save(store) {
                        Ok(()) => StatusMessage::ok(tr!("Saved conversations cleared.")),
                        Err(error) => StatusMessage::error(tr!(
                            "Could not clear them: {error}",
                            error = error
                        )),
                    });
                }
                SettingsAction::SetTheme(theme) => {
                    let _ = store.set_setting(crate::ui::theme::THEME_KEY, &theme);
                    crate::ui::theme::apply(ctx, theme);
                }
                SettingsAction::SetTabTitleStyle(style) => {
                    // The page draws under the store's lock, so it is written through it.
                    let _ = self.tab_titles.set(Some(store), style);
                }
                SettingsAction::SetLanguage(preference) => {
                    // Applied in memory even when the save fails, like the theme.
                    let _ = crate::host::language::choose(store, preference);
                }
                SettingsAction::UpdateNotifications(settings) => {
                    out.push(SettingsHostAction::UpdateNotifications(settings));
                }
                SettingsAction::SetPaperPrefs(prefs) => {
                    // Applied in memory even when the save fails, like the theme.
                    let _ = trading_hub.set_paper_prefs(store, prefs);
                }
                SettingsAction::RetryRegistration => {
                    out.push(SettingsHostAction::RetryRegistration);
                }
                SettingsAction::OpenLoginItems => {
                    out.push(SettingsHostAction::OpenLoginItems);
                }
                SettingsAction::OpenNotificationSettings => {
                    out.push(SettingsHostAction::OpenNotificationSettings);
                }
                SettingsAction::Mcp(action) => out.push(SettingsHostAction::Mcp(action)),
                SettingsAction::QuitForUpdate => out.push(SettingsHostAction::QuitForUpdate),
                SettingsAction::SetUpdateChannel(channel) => {
                    updates.set_channel(store, ctx, channel);
                }
            }
        }
        out
    }

    /// Saves graphics prefs and asks the app to apply the surface on the next frame.
    fn apply_graphics(
        &mut self,
        action: GraphicsAction,
        store: &Store,
        out: &mut Vec<SettingsHostAction>,
    ) {
        let GraphicsAction::Save(prefs) = action;
        if let Err(error) = prefs.save(store) {
            self.state.graphics.status = Some(StatusMessage::error(tr!(
                "Could not save the settings: {error}",
                error = error
            )));
            return;
        }
        self.state.graphics.status = None;
        self.graphics_prefs = ChangeCached::default();
        out.push(SettingsHostAction::ApplyGraphics(prefs));
    }

    /// Saves the proxy setting and hands it to the proxy worker, which applies it whether or not
    /// this window stays open. A failure to save is said on the page.
    fn apply_network(&mut self, action: NetworkAction, store: &Store, ctx: &egui::Context) {
        let NetworkAction::Save { config, password } = action;
        if let Err(error) = config.save(store) {
            self.state.network.status = Some(StatusMessage::error(tr!(
                "Could not save the settings: {error}",
                error = error
            )));
            return;
        }
        self.state.network.status = None;
        let repaint = ctx.clone();
        proxy::submit(
            config,
            password,
            Arc::new(move || repaint.request_repaint()),
        );
    }

    /// Shows what the proxy worker has to say, which waits here while the window is closed.
    fn collect_network(&mut self) {
        let Some(report) = proxy::take_report() else {
            return;
        };
        self.state.network.status = Some(match report {
            ApplyReport::PasswordSaved => StatusMessage::saved_securely(),
            ApplyReport::PasswordNotKept(error) => StatusMessage::error(tr!(
                "The password could not be saved for later ({error}). It works until you quit DegenView.",
                error = error.0
            )),
        });
    }

    /// Records the results of finished tests, and keeps the window repainting while any are
    /// still running so their rows update without the pointer moving.
    fn collect_test_results(&mut self, ctx: &egui::Context) {
        let webhooks = &mut self.state.webhooks;
        self.tests.retain_mut(|(id, test)| match test.poll() {
            Some(result) => {
                webhooks.set_test_result(&result);
                false
            }
            None if test.is_pending() => true,
            // The job ended without an answer: the row stops saying "Testing…".
            None => {
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

    /// Applies what the Snapshots page changed. A failure is said on the page, on the row it is
    /// about.
    fn apply_snapshots(&mut self, action: SnapshotsAction, store: &Store) {
        let held = Some(store);
        // The row a result is about, if it is about one, and what to say.
        let (row, result): (Option<&'static str>, Result<Option<StatusMessage>, String>) =
            match action {
                SnapshotsAction::SetProvider(provider) => (
                    None,
                    self.snapshots.set_provider(held, provider).map(|()| None),
                ),
                SnapshotsAction::Save {
                    provider,
                    fields,
                    secrets,
                } => {
                    let result =
                        self.snapshots
                            .set_fields(held, provider, &fields)
                            .and_then(|()| {
                                secrets.iter().try_for_each(|(key, value)| {
                                    self.snapshots.save_secret(provider, key, value)
                                })
                            });
                    if result.is_ok() {
                        self.state.snapshots.saved(provider);
                    }
                    (
                        Some(provider),
                        result.map(|()| {
                            Some(if secrets.is_empty() {
                                StatusMessage::ok("Saved")
                            } else {
                                StatusMessage::saved_securely()
                            })
                        }),
                    )
                }
                SnapshotsAction::RemoveSecret { provider, field } => (
                    Some(provider),
                    self.snapshots
                        .remove_secret(provider, field)
                        .map(|()| Some(StatusMessage::ok("Removed"))),
                ),
                SnapshotsAction::IncludeTrading(include) => (
                    None,
                    self.snapshots
                        .set_include_trading(held, include)
                        .map(|()| None),
                ),
                SnapshotsAction::Scale(scale) => {
                    (None, self.snapshots.set_scale(held, scale).map(|()| None))
                }
                SnapshotsAction::ForgetConsent => {
                    (None, self.snapshots.forget_consent(held).map(|()| None))
                }
            };
        let status = match result {
            Ok(status) => status,
            Err(error) => Some(StatusMessage::error(error)),
        };
        match row {
            Some(provider) => {
                self.state.snapshots.say_for(provider, status);
                self.state.snapshots.status = None;
            }
            None => self.state.snapshots.status = status,
        }
    }

    /// Walking and deleting the cache folder are disk work: both run off the UI thread and
    /// report back through `storage_events`.
    fn apply_storage(&mut self, action: StorageAction, ctx: &egui::Context) {
        let sender = self.storage_events.sender.clone();
        let repaint = ctx.clone();
        match action {
            StorageAction::Refresh => control::stats_in_background(move |stats| {
                let _ = sender.send(StorageEvent::Stats(stats));
                repaint.request_repaint();
            }),
            StorageAction::Clear => control::clear_in_background(ctx, move |outcome| {
                let _ = sender.send(StorageEvent::Cleared(outcome));
                let _ = sender.send(StorageEvent::Stats(control::stats()));
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
        out: &mut Vec<SettingsHostAction>,
    ) {
        match action {
            WebhookSettingsAction::Test { endpoint, message } => {
                // The request can take its full 3 s timeout and reading the secret may prompt, so
                // it runs on the workers that wait on the user; the row shows "Testing…" until
                // `collect_test_results` (which repaints while any is in flight) hands it back.
                let id = endpoint.id;
                let transport = Arc::clone(&self.transport);
                let secrets = Arc::clone(&self.secrets);
                let test = work_queue::SNAPSHOTS.start(None, move || {
                    delivery::deliver(&*transport, &*secrets, &message, &endpoint, Mode::Test)
                });
                self.tests.push((id, test));
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
                    out.push(SettingsHostAction::EndpointDeleted(id));
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
        result: crate::ui::webhook::editor::WebhookEditorResult,
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
mod tests;
