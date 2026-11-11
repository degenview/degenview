//! The stateful bridge between the MCP server, SQLite, the Settings page and the workspaces.
//!
//! The host owns the listener's lifecycle (it exists only while the user has the server
//! enabled), persists settings and credentials through `store::mcp`, keeps the server's shared
//! permission snapshot equal to what is on disk, and hands queued tool requests to the app.
//! Persist first, then publish: a write that fails changes nothing the server can see.

use crate::host::alert_host::AlertHost;
use crate::host::paper_host::{PaperHost, PaperHostError, owner_text};
use crate::host::portfolio_host::PortfolioHost;
use crate::mcp::auth::{generate_token, verifier_hex};
use crate::mcp::bridge::{
    self, AlertCommandOutcome, AlertUpdate, BridgeClient, BridgeInbox, McpError, NewAlert, Request,
};
use crate::mcp::chart_control::{ChartChangeResult, TimeframeResult};
use crate::mcp::chart_data;
use crate::mcp::drawings::{DrawingEdit, NewDrawing};
use crate::mcp::indicator_values::{
    IndicatorValuesResult, ReadingMeta, ReadingRequest, indicator_values,
};
use crate::mcp::markets::{self, WatchlistChangeResult, WatchlistPage, WatchlistsResult};
use crate::mcp::paper::{
    self, PaperAccountDetail, PaperAccountInfo, PaperAccountsResult, PaperChangeResult,
    PaperFillsResult, PaperOrderResult, PaperTradesResult,
};
use crate::mcp::pine::{InputArgs, ScriptToApply};
use crate::mcp::portfolio::{
    self, HoldingBase, HoldingsBase, PortfolioInfo, PortfoliosResult, TransactionsResult,
};
use crate::mcp::replay_tools::{self, ReplayResult};
use crate::mcp::seal;
use crate::mcp::snapshot_tools::{SnapshotProgress, SnapshotStarted};
use crate::mcp::state::{McpShared, McpStatus};
use crate::mcp::tls::{self, TlsIdentity};
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, AlertInfo, AlertsResult, AppliedPineDetail,
    AppliedPineInfo, AppliedPineList, CandlesResult, ChartInfo, ChartsResult, DrawingsChangeResult,
    DrawingsResult, IndicatorsInfo, PineAlertInfo, PineAlertsResult, PineInstanceChangeResult,
    PineInstanceInfo, PineOpenTradeInfo, PineReportContext, PineReportMetrics, PineReportResult,
    PineStrategySettingsInfo, PineStyleOverridesInfo, PineStyleRowInfo, PineStylesResult,
    PineTradeInfo, WorkspaceInfo,
};
use crate::mcp::{Listen, McpServer};
use crate::model::alert::{
    AlertCondition, AlertCurrency, AlertFrequency, AlertId, AlertState, PriceAlert,
};
use crate::model::mcp::{
    AUDIT_LOG_LIMIT, BindMode, Cidr, MAX_CIDRS, MAX_CLIENTS, McpAction, McpAuditEntry,
    McpAuditKind, McpClient, McpSettings, Permission, PermissionSet, clean_client_name,
    is_valid_port,
};
use crate::model::pine_alert::PineAlertSubscription;
use crate::model::watchlist::{Watchlist, WatchlistError, WatchlistInstrument, new_id, now_ms};
use crate::model::{
    BuiltIn, ChartType, DataSource, Drawing, DrawingCommand, IndicatorConfig, InstrumentId,
    Timeframe,
};
use crate::service::keychain_mcp_seal::{KeyState, SealKey, SealKeyStore};
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::store::SharedStore;
use crate::store::watchlist::{ListChanges, ListEdit, Place, WatchlistStore};
use crate::ui::alert_editor::{Direction, PriceAlertEditorState};
use crate::workspace::{
    ChartSummary, ChartTypeError, ChartWorkspace, IndicatorError, IndicatorOutcome,
    PineInstanceView, RemoveOutcome, ReplayCommand, ReplayError,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Whether the credential seal is off. Tests and debug builds run without it: every rebuild
/// has a new code identity, so the Keychain would ask for the seal key's password on each launch.
/// A `--release` build with `dev-unsealed-mcp` (a developer's scratch HOME) also needs
/// `DEGENVIEW_NO_KEYCHAIN`. A distributed build ignores the variable, because any process that
/// can write the database could otherwise launch the app with it set, plant a credential, and
/// skip the seal.
fn sealing_disabled(test: bool, debug_build: bool, dev_feature: bool, env_set: bool) -> bool {
    test || debug_build || (dev_feature && env_set)
}

/// Recorded last-use times are written at most this often.
const USAGE_SAVE_INTERVAL: Duration = Duration::from_secs(30);
/// Activity rows waiting in memory are written at most this often, in one batch.
const AUDIT_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// A bearer token just issued. Held only until the user dismisses it; never persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshToken {
    pub client_id: String,
    pub client_name: String,
    pub token: String,
}

pub struct McpHost {
    store: SharedStore,
    shared: Arc<McpShared>,
    bridge: BridgeClient,
    inbox: BridgeInbox,
    server: Option<McpServer>,
    settings: McpSettings,
    /// Set when persisted MCP state could not be read: the server stays off and nothing is
    /// written, so unreadable credentials or permissions are never replaced by defaults.
    load_error: Option<String>,
    /// The saved state failed the seal check (or its key is gone), so the Settings page offers
    /// "Reset MCP Access". Never set for a plain read failure.
    can_reset: bool,
    /// Where the seal key lives; `None` when sealing is off (debug builds, tests).
    keys: Option<SealKeyStore>,
    /// The key every write is sealed with, once the saved state has been checked against it.
    sealed_by: Option<SealKey>,
    fresh_token: Option<FreshToken>,
    notice: Option<String>,
    last_usage_save: Instant,
    last_audit_flush: Instant,
    /// The whole activity log, newest first, for the Settings page and its log sheet, which must
    /// not read the store themselves. Shared, so handing it to a frame copies a pointer.
    activity: Arc<Vec<McpAuditEntry>>,
    /// Rows the log holds that this version cannot decode, so the sheet can say it is hiding some.
    activity_unreadable: usize,
    /// Actions from the Settings page, applied by [`McpHost::tick`]. The page runs while the
    /// store is locked, so it can only queue.
    pending: Vec<McpAction>,
    /// Where the TLS certificate for a non-loopback bind is kept: the app's data directory.
    tls_dir: Option<std::path::PathBuf>,
    /// Fingerprint of that certificate, for the Settings page.
    tls_fingerprint: Option<String>,
}

impl McpHost {
    /// Loads persisted state and, only if the user previously enabled the server, starts it.
    /// `wake` runs a UI frame when a request is queued.
    pub fn load(store: SharedStore, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let env_set = std::env::var_os("DEGENVIEW_NO_KEYCHAIN").is_some();
        let keys = (!sealing_disabled(
            cfg!(test),
            cfg!(debug_assertions),
            cfg!(feature = "dev-unsealed-mcp"),
            env_set,
        ))
        .then(SealKeyStore::system);
        Self::load_with_keys(store, wake, keys)
    }

    /// [`McpHost::load`] with the seal key store chosen by the caller (`None` turns sealing off).
    pub fn load_with_keys(
        store: SharedStore,
        wake: Arc<dyn Fn() + Send + Sync>,
        keys: Option<SealKeyStore>,
    ) -> Self {
        let shared = McpShared::new();
        let (bridge, inbox) = bridge::channel(shared.clone(), wake);
        let mut host = Self {
            store,
            shared,
            bridge,
            inbox,
            server: None,
            settings: McpSettings::default(),
            load_error: None,
            can_reset: false,
            keys,
            sealed_by: None,
            fresh_token: None,
            notice: None,
            last_usage_save: Instant::now(),
            last_audit_flush: Instant::now(),
            activity: Arc::new(Vec::new()),
            activity_unreadable: 0,
            pending: Vec::new(),
            // Tests never write into the real data directory; one that needs a folder sets it.
            tls_dir: if cfg!(test) {
                None
            } else {
                crate::store::data_dir()
            },
            tls_fingerprint: None,
        };
        host.reload();
        if host.load_error.is_none() {
            host.refresh_fingerprint();
        }
        host.refresh_activity();
        if host.load_error.is_none() && host.settings.enabled {
            host.start_server();
        }
        host
    }

    fn reload(&mut self) {
        self.can_reset = false;
        self.sealed_by = None;
        let loaded = self
            .store
            .lock()
            .map_err(|_| "store unavailable".to_owned())
            .and_then(|store| {
                let settings = store.mcp_settings().map_err(|error| error.to_string())?;
                let clients = store.mcp_clients().map_err(|error| error.to_string())?;
                let global = store
                    .mcp_global_permissions()
                    .map_err(|error| error.to_string())?;
                let stored = store.mcp_seal().map_err(|error| error.to_string())?;
                Ok((settings, clients, global, stored))
            });
        match loaded {
            Ok((settings, clients, global, stored)) => {
                match self.check_seal(&settings, &global, &clients, stored.as_deref()) {
                    Ok(()) => {
                        self.shared
                            .set_require_confirmation(settings.require_confirmation);
                        self.shared
                            .set_remote_ceiling(settings.remote_ceiling.clone());
                        self.settings = settings;
                        self.shared.replace(clients, global);
                        self.load_error = None;
                    }
                    Err(error) => self.fail_closed(error, true),
                }
            }
            Err(error) => self.fail_closed(
                format!("Saved MCP settings could not be read, so the server stays off ({error})"),
                false,
            ),
        }
    }

    /// Fail closed: no credentials are loaded, so nothing can authenticate.
    fn fail_closed(&mut self, message: String, can_reset: bool) {
        self.shared.replace(Vec::new(), PermissionSet::none());
        self.settings.enabled = false;
        self.load_error = Some(message);
        self.can_reset = can_reset;
    }

    /// Checks what was loaded against the seal in the Keychain, and sets `sealed_by` so every
    /// later write is sealed. An error means the saved state is not what this app wrote.
    ///
    /// | key in Keychain | seal in database | outcome |
    /// | --- | --- | --- |
    /// | yes | matches | trusted |
    /// | yes | differs | refused: changed outside the app |
    /// | yes | none | trusted only if nothing was ever configured, else refused |
    /// | none | none | first run of a sealing build: adopt what is saved, once, and seal it |
    /// | none | yes | refused: the key was deleted |
    /// | unreadable | any | refused |
    ///
    /// The "adopt" row cannot tell credentials made before sealing existed from a planted one;
    /// it trusts them once. Everything written after it is protected.
    fn check_seal(
        &mut self,
        settings: &McpSettings,
        global: &PermissionSet,
        clients: &[McpClient],
        stored: Option<&str>,
    ) -> Result<(), String> {
        const CHANGED: &str = "The saved MCP credentials do not match the seal kept in the Keychain, so they were changed outside DegenView. The server stays off.";
        const KEY_LOST: &str = "The Keychain key that protects the saved MCP credentials is missing, so they cannot be trusted. The server stays off.";
        const KEY_UNREADABLE: &str = "The Keychain key that protects the saved MCP credentials could not be read, so the server stays off. Restart DegenView to try again.";

        let Some(keys) = &self.keys else {
            return Ok(());
        };
        let (key, write_seal) = match (keys.load(), stored) {
            (KeyState::Ready(key), Some(stored)) => {
                if !seal::verify(&key.0, settings, global, clients, stored) {
                    return Err(CHANGED.into());
                }
                (key, false)
            }
            (KeyState::Ready(key), None) => {
                if !seal::is_pristine(settings, global, clients) {
                    return Err(CHANGED.into());
                }
                (key, true)
            }
            (KeyState::Missing, None) => match keys.create() {
                Ok(key) => (key, true),
                Err(error) => {
                    self.notice = Some(format!(
                        "MCP credentials cannot be protected against edits to the database on this Mac ({})",
                        error.0
                    ));
                    return Ok(());
                }
            },
            (KeyState::Missing, Some(_)) => return Err(KEY_LOST.into()),
            (KeyState::Unreadable, _) => return Err(KEY_UNREADABLE.into()),
        };
        if write_seal {
            let sealed = seal::seal(&key.0, settings, global, clients);
            let saved = self
                .store
                .lock()
                .map(|store| store.save_mcp_state(settings, global, clients, Some(&sealed)))
                .unwrap_or_else(|_| Err(rusqlite::Error::InvalidQuery));
            if let Err(error) = saved {
                return Err(format!(
                    "The MCP credentials could not be sealed, so the server stays off ({error})"
                ));
            }
        }
        self.sealed_by = Some(key);
        Ok(())
    }

    /// Writes settings, cap, clients and their seal in one transaction. Callers publish to the
    /// server only after this succeeds.
    fn persist(
        &self,
        settings: &McpSettings,
        global: &PermissionSet,
        clients: &[McpClient],
    ) -> rusqlite::Result<()> {
        let sealed = self
            .sealed_by
            .as_ref()
            .map(|key| seal::seal(&key.0, settings, global, clients));
        self.store
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .save_mcp_state(settings, global, clients, sealed.as_deref())
    }

    /// The user's "Reset MCP Access": replaces the key, forgets every credential and setting (the
    /// saved state cannot be trusted), and starts again sealed and off.
    fn reset_access(&mut self) {
        if !self.can_reset {
            return;
        }
        // Replace the key first. If the Keychain refuses the write (the item is locked, the very
        // case that offers this reset), nothing has changed, so the saved credentials are not lost
        // for nothing and the user can try again. Once the write succeeds the old key is gone, so
        // from here the reset runs to the end: stopping would leave credentials that no key opens
        // and a notice that wrongly says nothing happened.
        if let Some(keys) = &self.keys
            && let Err(error) = keys.rotate()
        {
            self.notice = Some(format!(
                "Could not replace the Keychain key, so nothing was changed ({}). Delete the “{}” item in Keychain Access or your keyring, then restart DegenView.",
                error.0,
                SealKeyStore::SERVICE
            ));
            return;
        }
        self.stop_server();
        let cleared = self
            .store
            .lock()
            .map(|store| store.clear_mcp_state())
            .unwrap_or_else(|_| Err(rusqlite::Error::InvalidQuery));
        if cleared.is_err() {
            self.notice = Some(
                "The Keychain key was replaced but the saved MCP state could not be cleared. Choose Reset MCP Access again."
                    .into(),
            );
            return;
        }
        self.fresh_token = None;
        self.settings = McpSettings::default();
        self.reload();
        if self.load_error.is_none() && self.notice.is_none() {
            self.notice = Some(
                "MCP access was reset: every credential is revoked and the server is off".into(),
            );
        }
    }

    pub fn shared(&self) -> Arc<McpShared> {
        self.shared.clone()
    }

    pub fn status(&self) -> McpStatus {
        self.shared.status()
    }

    pub fn settings(&self) -> &McpSettings {
        &self.settings
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Whether Settings should offer "Reset MCP Access".
    pub fn can_reset(&self) -> bool {
        self.can_reset
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// The activity log, newest first: every readable row, at most
    /// [`AUDIT_LOG_LIMIT`] of them.
    pub fn activity(&self) -> Arc<Vec<McpAuditEntry>> {
        Arc::clone(&self.activity)
    }

    /// How many rows the log holds that could not be read and so are not in [`Self::activity`].
    pub fn activity_unreadable(&self) -> usize {
        self.activity_unreadable
    }

    pub fn fresh_token(&self) -> Option<&FreshToken> {
        self.fresh_token.as_ref()
    }

    pub fn clients(&self) -> Vec<McpClient> {
        self.shared.clients()
    }

    pub fn global_permissions(&self) -> PermissionSet {
        self.shared.global()
    }

    /// Up to `max` queued tool requests for the app thread to run.
    pub fn take_requests(&self, max: usize) -> Vec<Request> {
        self.inbox.take(max)
    }

    pub fn queue(&mut self, action: McpAction) {
        self.pending.push(action);
    }

    /// Applies queued Settings actions, then persists "last used" times (at most every 30
    /// seconds). Called once per frame by the app, with no other lock held.
    pub fn tick(&mut self) {
        for action in std::mem::take(&mut self.pending) {
            self.apply(action);
        }
        if self.last_audit_flush.elapsed() >= AUDIT_FLUSH_INTERVAL {
            self.flush_activity(false);
        }
        if self.load_error.is_some() || self.last_usage_save.elapsed() < USAGE_SAVE_INTERVAL {
            return;
        }
        self.last_usage_save = Instant::now();
        if let Some(clients) = self.shared.take_usage_update() {
            let saved = self
                .store
                .lock()
                .is_ok_and(|store| store.replace_mcp_clients(&clients).is_ok());
            if !saved {
                // The times were taken before they were written; put them back so the next
                // tick tries again instead of losing them until the client's next request.
                self.shared.mark_usage_dirty();
            }
        }
    }

    /// Writes the activity rows waiting in memory to the store in one batch. `all` also writes
    /// the rate-limit counts still being added up (when quitting). A failed write puts the rows
    /// back, so the next tick tries again.
    fn flush_activity(&mut self, all: bool) {
        self.last_audit_flush = Instant::now();
        let rows = self.shared.audit().take(now_ms(), all);
        if rows.is_empty() {
            return;
        }
        let written = self
            .store
            .lock()
            .map(|store| store.append_mcp_audit(&rows))
            .unwrap_or_else(|_| Err(rusqlite::Error::InvalidQuery));
        match written {
            Ok(()) => self.refresh_activity(),
            Err(_) => self.shared.audit().requeue(rows),
        }
    }

    /// Reloads the log the Settings page shows from the store.
    fn refresh_activity(&mut self) {
        if let Ok(store) = self.store.lock() {
            let rows = store.recent_mcp_audit(AUDIT_LOG_LIMIT);
            self.activity_unreadable = store.mcp_audit_count().saturating_sub(rows.len());
            self.activity = Arc::new(rows);
        }
    }

    /// Empties the log. Rows still waiting in memory are written first, so they are cleared
    /// too and none appears after the marker that says everything before it is gone.
    fn clear_activity(&mut self) {
        self.flush_activity(true);
        let marker = McpAuditEntry {
            id: new_id(),
            at_ms: now_ms(),
            kind: McpAuditKind::Cleared,
            count: 1,
            ..McpAuditEntry::default()
        };
        let cleared = self
            .store
            .lock()
            .map(|store| store.clear_mcp_audit(&marker))
            .unwrap_or_else(|_| Err(rusqlite::Error::InvalidQuery));
        match cleared {
            Ok(()) => self.refresh_activity(),
            Err(_) => self.notice = Some("Could not clear the activity log".into()),
        }
    }

    pub fn apply(&mut self, action: McpAction) {
        self.notice = None;
        if matches!(action, McpAction::ResetAccess) {
            self.reset_access();
            return;
        }
        if self.load_error.is_some() {
            self.notice = Some("MCP settings are unreadable; nothing can be changed".into());
            return;
        }
        match action {
            McpAction::SetEnabled(enabled) => self.set_enabled(enabled),
            McpAction::SetPort(port) => self.set_port(port),
            McpAction::SetBind(bind) => self.set_bind(bind),
            McpAction::CreateClient(name) => self.create_client(&name),
            McpAction::RevokeClient(id) => self.revoke_client(&id),
            McpAction::SetClientPermission {
                client_id,
                permission,
                allowed,
            } => {
                self.edit_clients(|clients| {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.permissions.set(permission, allowed);
                    }
                });
            }
            McpAction::SetGlobalPermission {
                permission,
                allowed,
            } => self.set_global_permission(permission, allowed),
            McpAction::SetClientPermissions {
                client_id,
                permissions,
            } => {
                self.edit_clients(|clients| {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.permissions = permissions;
                    }
                });
            }
            McpAction::SetGlobalPermissions(permissions) => self.replace_global(permissions),
            McpAction::SetClientRemote { client_id, allowed } => {
                self.edit_clients(|clients| {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.remote_allowed = allowed;
                    }
                });
            }
            McpAction::SetClientCidrs { client_id, cidrs } => {
                // No duplicates, and no more than a Settings list can sensibly show.
                let mut kept: Vec<Cidr> = Vec::new();
                for cidr in cidrs {
                    if !kept.contains(&cidr) && kept.len() < MAX_CIDRS {
                        kept.push(cidr);
                    }
                }
                self.edit_clients(|clients| {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.allowed_cidrs = kept;
                    }
                });
            }
            McpAction::SetRemoteCeiling(ceiling) => {
                let settings = McpSettings {
                    remote_ceiling: ceiling,
                    ..self.settings.clone()
                };
                self.save_settings(settings);
            }
            McpAction::DismissToken => self.fresh_token = None,
            McpAction::ResetAccess => {}
            McpAction::ClearActivity => self.clear_activity(),
            McpAction::SetRequireConfirmation(required) => {
                let settings = McpSettings {
                    require_confirmation: required,
                    ..self.settings.clone()
                };
                self.save_settings(settings);
            }
        }
    }

    fn save_settings(&mut self, settings: McpSettings) -> bool {
        let saved = self.persist(&settings, &self.shared.global(), &self.shared.clients());
        match saved {
            Ok(()) => {
                self.shared
                    .set_require_confirmation(settings.require_confirmation);
                self.shared
                    .set_remote_ceiling(settings.remote_ceiling.clone());
                self.settings = settings;
                true
            }
            Err(_) => {
                self.notice = Some("Could not save the MCP settings".into());
                false
            }
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        let settings = McpSettings {
            enabled,
            ..self.settings.clone()
        };
        if !self.save_settings(settings) {
            return;
        }
        self.stop_server();
        if enabled {
            self.start_server();
        }
    }

    /// Moves the listener to this machine only, or to one named interface. The certificate for it
    /// is made now (not when the server next starts), so the Settings page can show the
    /// fingerprint to pin before anything connects. A server that is on is restarted on the new
    /// address; one that is off stays off.
    fn set_bind(&mut self, bind: BindMode) {
        if bind == self.settings.bind {
            return;
        }
        let settings = McpSettings {
            bind,
            ..self.settings.clone()
        };
        if !self.save_settings(settings) {
            return;
        }
        self.refresh_fingerprint();
        if self.settings.enabled {
            self.stop_server();
            self.start_server();
        }
    }

    /// Reads (or makes) the certificate for the saved bind, for the fingerprint the page shows.
    fn refresh_fingerprint(&mut self) {
        if let Err(message) = self.tls_identity() {
            self.notice = Some(message);
        }
    }

    /// The fingerprint of the certificate a remote client should pin, when the server listens
    /// beyond this machine.
    pub fn tls_fingerprint(&self) -> Option<&str> {
        self.tls_fingerprint.as_deref()
    }

    fn set_port(&mut self, port: u16) {
        if !is_valid_port(port) {
            self.notice = Some("Choose a port between 1024 and 65535".into());
            return;
        }
        if port == self.settings.port {
            return;
        }
        let settings = McpSettings {
            port,
            ..self.settings.clone()
        };
        if !self.save_settings(settings) {
            return;
        }
        if self.settings.enabled {
            self.stop_server();
            self.start_server();
        }
    }

    fn start_server(&mut self) {
        let tls = match self.tls_identity() {
            Ok(tls) => tls,
            Err(message) => {
                self.shared.set_status(McpStatus::Failed(message.clone()));
                self.notice = Some(message);
                return;
            }
        };
        let listen = Listen {
            bind: self.settings.bind,
            port: self.settings.port,
            tls,
        };
        match McpServer::start_listening(self.shared.clone(), self.bridge.clone(), listen) {
            Ok(server) => self.server = Some(server),
            // The status already says Failed with the reason.
            Err(message) => self.notice = Some(message),
        }
    }

    /// The certificate for the interface the server listens on: none on loopback, otherwise the
    /// one saved for that address, made on first use.
    ///
    /// The only way the host loads one, and it records the fingerprint of what it returns (nothing
    /// when there is none or it could not be loaded), so the page can never show a fingerprint
    /// other than that of the identity last handed to the listener: a file deleted or replaced
    /// while the app runs is picked up here, at the next start, with its new fingerprint.
    fn tls_identity(&mut self) -> Result<Option<TlsIdentity>, String> {
        let identity = self.load_tls_identity();
        self.tls_fingerprint = match &identity {
            Ok(Some(identity)) => Some(identity.fingerprint()),
            _ => None,
        };
        identity
    }

    fn load_tls_identity(&self) -> Result<Option<TlsIdentity>, String> {
        let BindMode::Interface(address) = self.settings.bind else {
            return Ok(None);
        };
        let dir = self
            .tls_dir
            .as_deref()
            .ok_or("There is no folder to keep the TLS certificate in")?;
        tls::load_or_create(dir, address).map(Some)
    }

    fn stop_server(&mut self) {
        if let Some(mut server) = self.server.take() {
            server.stop();
        } else {
            self.shared.bump_generation();
            self.shared.set_status(McpStatus::Disabled);
        }
    }

    /// Applies `edit` to the clients, saves the result, and only then publishes it.
    fn edit_clients(&mut self, edit: impl FnOnce(&mut Vec<McpClient>)) -> bool {
        let mut clients = self.shared.clients();
        edit(&mut clients);
        let saved = self.persist(&self.settings, &self.shared.global(), &clients);
        match saved {
            Ok(()) => {
                self.shared.replace(clients, self.shared.global());
                true
            }
            Err(_) => {
                self.notice = Some("Could not save the MCP clients".into());
                false
            }
        }
    }

    fn create_client(&mut self, name: &str) {
        let Some(name) = clean_client_name(name) else {
            self.notice = Some("Give the client a short name".into());
            return;
        };
        if self.shared.clients().len() >= MAX_CLIENTS {
            self.notice = Some(format!("At most {MAX_CLIENTS} clients are supported"));
            return;
        }
        let token = match generate_token() {
            Ok(token) => token,
            Err(error) => {
                self.notice = Some(error);
                return;
            }
        };
        let id = new_id().to_string();
        let client = McpClient {
            id: id.clone(),
            name: name.clone(),
            verifier: verifier_hex(&token),
            // A new credential can read; anything that changes the app is opt-in.
            permissions: PermissionSet::of([Permission::ChartsRead]),
            created_at_ms: now_ms(),
            last_used_ms: None,
            remote_allowed: false,
            allowed_cidrs: Vec::new(),
        };
        if self.edit_clients(|clients| clients.push(client)) {
            self.fresh_token = Some(FreshToken {
                client_id: id,
                client_name: name,
                token,
            });
        }
    }

    fn revoke_client(&mut self, id: &str) {
        if self.edit_clients(|clients| clients.retain(|client| client.id != id))
            && self
                .fresh_token
                .as_ref()
                .is_some_and(|fresh| fresh.client_id == id)
        {
            self.fresh_token = None;
        }
    }

    fn set_global_permission(&mut self, permission: Permission, allowed: bool) {
        let mut global = self.shared.global();
        global.set(permission, allowed);
        self.replace_global(global);
    }

    fn replace_global(&mut self, global: PermissionSet) {
        let saved = self.persist(&self.settings, &global, &self.shared.clients());
        match saved {
            Ok(()) => self.shared.replace(self.shared.clients(), global),
            Err(_) => self.notice = Some("Could not save the MCP permissions".into()),
        }
    }
}

impl McpHost {
    /// The app is quitting: stop taking calls, then write every activity row still in memory,
    /// including the rate-limit counts still being added up. The app calls this from its exit
    /// hook, because a quit does not necessarily drop the host. Safe to call more than once.
    pub fn shutdown(&mut self) {
        self.stop_server();
        self.flush_activity(true);
    }
}

impl Drop for McpHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// MARK: Workspace projections (run on the app thread)

pub fn source_slug(source: DataSource) -> &'static str {
    match source {
        DataSource::Binance => "binance",
        DataSource::Coinbase => "coinbase",
        DataSource::CoinGecko => "coingecko",
        DataSource::DexScreener => "dexscreener",
        DataSource::Alpaca => "alpaca",
        DataSource::Polymarket => "polymarket",
        DataSource::Kalshi => "kalshi",
        DataSource::CoinMarketCap => "coinmarketcap",
    }
}

pub fn workspace_info(workspace: &ChartWorkspace, active: bool) -> WorkspaceInfo {
    WorkspaceInfo {
        workspace_id: workspace.id().to_string(),
        title: workspace.title(),
        chart_count: workspace.chart_count(),
        active,
        timeframe: workspace.timeframe().label().to_owned(),
    }
}

fn chart_info(workspace: &ChartWorkspace, chart: ChartSummary) -> ChartInfo {
    let indicators = chart.indicators;
    let status = if chart.error.is_some() {
        "error"
    } else if chart.loading {
        "loading"
    } else {
        "ready"
    };
    ChartInfo {
        chart_id: chart.id.to_string(),
        instrument: chart.instrument.key(),
        source: source_slug(chart.instrument.source).to_owned(),
        symbol: chart.instrument.symbol.clone(),
        display_symbol: chart.display_symbol,
        timeframe: workspace.timeframe().label().to_owned(),
        chart_type: if chart.is_line_chart {
            "line"
        } else {
            chart.chart_type.slug()
        }
        .to_owned(),
        indicators: indicators_info(indicators),
        pine_instances: chart
            .pine_instances
            .into_iter()
            .map(|(id, name)| PineInstanceInfo {
                instance_id: id.to_string(),
                script_name: name,
            })
            .collect(),
        status: status.to_owned(),
        error: chart.error,
        focused: chart.focused,
    }
}

/// What the chart draws now: applied and not hidden with its eye.
fn indicators_info(config: IndicatorConfig) -> IndicatorsInfo {
    let drawn = config.drawn();
    IndicatorsInfo {
        volume: drawn.show_volume,
        rsi: drawn.show_rsi,
        ema: drawn.show_ema,
        ema_period: config.ema_period,
        bollinger: drawn.show_bollinger,
        trend_flips: drawn.show_trend_flips,
    }
}

/// Applies a built-in indicator through the workspace's own method and describes the result.
pub fn apply_indicator(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    indicator: BuiltIn,
    ema_period: Option<usize>,
) -> Result<AddIndicatorResult, McpError> {
    let workspace_id = workspace.id();
    match workspace.apply_indicator(chart_id, indicator, ema_period) {
        Ok((outcome, config)) => Ok(AddIndicatorResult {
            workspace_id: workspace_id.to_string(),
            chart_id: chart_id.to_string(),
            indicator: indicator.id().to_owned(),
            status: match outcome {
                IndicatorOutcome::Added => "added",
                IndicatorOutcome::Updated => "updated",
                IndicatorOutcome::AlreadyApplied => "already_applied",
            }
            .to_owned(),
            indicators: indicators_info(config),
        }),
        Err(IndicatorError::NoSuchChart) => Err(McpError::NotFound(format!(
            "No chart {chart_id} in workspace {workspace_id}"
        ))),
        Err(IndicatorError::UnsupportedOnLineChart) => Err(McpError::Unsupported(format!(
            "{} is not available on line charts",
            indicator.menu_title()
        ))),
        Err(IndicatorError::NoParameters) => Err(McpError::InvalidArgument(format!(
            "{} has no parameters",
            indicator.menu_title()
        ))),
        Err(IndicatorError::InvalidEmaPeriod) => Err(McpError::InvalidArgument(format!(
            "EMA period must be one of {:?}",
            crate::model::EMA_PERIODS
        ))),
    }
}

pub fn charts_result(workspace: &ChartWorkspace) -> ChartsResult {
    ChartsResult {
        workspace_id: workspace.id().to_string(),
        charts: workspace
            .chart_summaries()
            .into_iter()
            .map(|chart| chart_info(workspace, chart))
            .collect(),
    }
}

/// Adds the chart through the workspace's own path and describes the outcome.
pub fn add_chart(
    workspace: &mut ChartWorkspace,
    instrument: &InstrumentId,
    ctx: &eframe::egui::Context,
) -> AddChartResult {
    let outcome = workspace.add_chart_for(instrument, ctx);
    let chart = workspace
        .chart_summaries()
        .into_iter()
        .find(|chart| chart.id == outcome.chart_id);
    AddChartResult {
        workspace_id: workspace.id().to_string(),
        chart_id: outcome.chart_id.to_string(),
        instrument: instrument.key(),
        source: source_slug(instrument.source).to_owned(),
        symbol: chart.as_ref().map_or_else(
            || instrument.symbol.clone(),
            |chart| chart.instrument.symbol.clone(),
        ),
        display_symbol: chart.map_or_else(
            || instrument.source.display_symbol(&instrument.symbol),
            |chart| chart.display_symbol,
        ),
        timeframe: workspace.timeframe().label().to_owned(),
        status: if outcome.created {
            "created"
        } else {
            "already_present"
        }
        .to_owned(),
    }
}

// MARK: Pine scripts applied to charts (run on the app thread)

/// Report trades returned per call at most.
pub const MAX_TRADES_PAGE: usize = 200;
/// Equity points returned at most; longer curves are thinned evenly.
const MAX_EQUITY_POINTS: usize = 200;

fn applied_info(view: &PineInstanceView) -> AppliedPineInfo {
    let instance = &view.instance;
    let (run_status, error, is_strategy) = match &view.run {
        None => ("not_run", None, None),
        Some(run) => (
            if run.error.is_some() { "error" } else { "ok" },
            run.error.as_ref().map(crate::mcp::pine::diagnostic_info),
            Some(run.is_strategy),
        ),
    };
    AppliedPineInfo {
        instance_id: instance.id.to_string(),
        script_id: instance.script_id.clone(),
        script_name: instance.name.clone(),
        visible: instance.visible,
        source_hash: crate::pine_studio::store::hash_source(&instance.source),
        run_status: run_status.to_owned(),
        error,
        is_strategy,
    }
}

fn stored_inputs(view: &PineInstanceView) -> std::collections::BTreeMap<String, serde_json::Value> {
    view.instance
        .inputs
        .iter()
        .filter_map(|(id, value)| Some((id.clone(), serde_json::to_value(value).ok()?)))
        .collect()
}

fn pine_views(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance: Option<u64>,
    with_report: bool,
) -> Result<Vec<PineInstanceView>, McpError> {
    let views = workspace
        .pine_instance_views(chart_id, instance, with_report)
        .ok_or_else(|| {
            McpError::NotFound(format!(
                "No chart {chart_id} in workspace {}",
                workspace.id()
            ))
        })?;
    if let Some(id) = instance
        && views.is_empty()
    {
        return Err(McpError::NotFound(format!(
            "No script instance {id} on chart {chart_id}"
        )));
    }
    Ok(views)
}

pub fn list_chart_pine(
    workspace: &ChartWorkspace,
    chart_id: u64,
) -> Result<AppliedPineList, McpError> {
    let views = pine_views(workspace, chart_id, None, false)?;
    let (instrument, timeframe) = views.first().map_or_else(
        || {
            workspace
                .chart_summaries()
                .into_iter()
                .find(|chart| chart.id == chart_id)
                .map(|chart| {
                    (
                        chart.instrument.key(),
                        workspace.timeframe().label().to_owned(),
                    )
                })
                .unwrap_or_default()
        },
        |view| (view.instrument.key(), view.timeframe.label().to_owned()),
    );
    Ok(AppliedPineList {
        workspace_id: workspace.id().to_string(),
        chart_id: chart_id.to_string(),
        instrument,
        timeframe,
        instances: views.iter().map(applied_info).collect(),
    })
}

/// `script_status` is filled in by the caller, who can read the Pine Studio off this thread.
pub fn chart_pine_detail(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
) -> Result<AppliedPineDetail, McpError> {
    let view = pine_views(workspace, chart_id, Some(instance_id), false)?.remove(0);
    Ok(detail_of(workspace.id(), &view))
}

fn instance_change(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    status: &str,
) -> Result<PineInstanceChangeResult, McpError> {
    Ok(PineInstanceChangeResult {
        status: status.to_owned(),
        instance_id: instance_id.to_string(),
        chart: list_chart_pine(workspace, chart_id)?,
    })
}

/// Applies a checked script to a chart, after the scripts already there.
pub fn add_pine_instance(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    script: &ScriptToApply,
    visible: bool,
) -> Result<PineInstanceChangeResult, McpError> {
    let id = workspace
        .add_pine_instance_to(
            chart_id,
            script.script_id.clone(),
            &script.name,
            &script.source,
            &script.inputs,
            visible,
        )
        .ok_or_else(|| {
            McpError::NotFound(format!(
                "No chart {chart_id} in workspace {}",
                workspace.id()
            ))
        })?;
    instance_change(workspace, chart_id, id, "added")
}

/// Refuses a change that would retarget or silence a live script alert. The alert host copies an
/// instance's inputs into the alert it arms, and silences the alert when the instance goes, so
/// changing either from a client that holds only the chart permission would alter alerts that
/// have their own (deferred) permissions. The person can still do it in the app.
pub(super) fn guard_armed_alerts(
    subscriptions: &[PineAlertSubscription],
    chart_id: u64,
    instance_id: u64,
    what: &str,
) -> Result<(), McpError> {
    let armed: Vec<String> = subscriptions
        .iter()
        .filter(|alert| {
            alert.is_active()
                && alert.chart_id == chart_id
                && alert.instance_id == Some(instance_id)
        })
        .map(|alert| alert.id.to_string())
        .collect();
    if armed.is_empty() {
        return Ok(());
    }
    Err(McpError::Conflict(format!(
        "Script alert {} is armed on this instance, and {what} would change or silence it. Change it in DegenView, or pause the alert there first",
        armed.join(", ")
    )))
}

/// Changes an applied script's inputs and/or visibility. Inputs are checked against the schema
/// of the instance's own source, which it keeps from when it was applied. A change to the inputs
/// is refused while a script alert is armed on the instance; visibility never affects alerts.
pub fn update_pine_instance(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    args: Option<&InputArgs>,
    visible: Option<bool>,
    subscriptions: &[PineAlertSubscription],
) -> Result<PineInstanceChangeResult, McpError> {
    let view = pine_views(workspace, chart_id, Some(instance_id), false)?.remove(0);
    let inputs = args
        .map(|args| {
            crate::mcp::pine::instance_inputs(
                &view.instance.source,
                view.instance.input_values(),
                args,
            )
        })
        .transpose()?;
    if inputs
        .as_ref()
        .is_some_and(|inputs| *inputs != view.instance.input_values())
    {
        guard_armed_alerts(subscriptions, chart_id, instance_id, "changing its inputs")?;
    }
    let changed = workspace
        .change_pine_instance(chart_id, instance_id, inputs.as_ref(), visible)
        .ok_or_else(|| {
            McpError::NotFound(format!(
                "No script instance {instance_id} on chart {chart_id}"
            ))
        })?;
    let status = if changed { "updated" } else { "unchanged" };
    instance_change(workspace, chart_id, instance_id, status)
}

/// Takes a script off a chart, unless a script alert is armed on it.
pub fn remove_pine_instance(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    subscriptions: &[PineAlertSubscription],
) -> Result<PineInstanceChangeResult, McpError> {
    pine_views(workspace, chart_id, Some(instance_id), false)?;
    guard_armed_alerts(subscriptions, chart_id, instance_id, "removing it")?;
    workspace.remove_pine_instance_from(chart_id, instance_id);
    instance_change(workspace, chart_id, instance_id, "removed")
}

// MARK: Candles and drawings (run on the app thread)

pub(super) fn no_chart(workspace: &ChartWorkspace, chart_id: u64) -> McpError {
    McpError::NotFound(format!(
        "No chart {chart_id} in workspace {}",
        workspace.id()
    ))
}

/// The newest `limit` real candles of a chart, or those before `before`.
pub fn chart_candles(
    workspace: &ChartWorkspace,
    chart_id: u64,
    limit: usize,
    before: Option<i64>,
) -> Result<CandlesResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let meta = chart_data::CandleMeta {
        workspace_id: workspace.id(),
        chart_id,
        older_supported: data
            .instrument
            .source
            .supports_older_history(workspace.timeframe()),
        instrument: data.instrument,
        timeframe: workspace.timeframe().label().to_owned(),
        chart_type: data.chart_type,
        replay_ms: data.replay_ms,
        now_ms: now_ms(),
    };
    Ok(chart_data::candles_result(
        &meta,
        data.candles,
        limit,
        before,
    ))
}

pub fn chart_drawings(
    workspace: &ChartWorkspace,
    chart_id: u64,
) -> Result<DrawingsResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    Ok(chart_data::drawings_result(
        workspace.id(),
        chart_id,
        data.drawings,
    ))
}

/// The result of a change: the drawings it touched as they are now.
fn drawings_changed(
    workspace: &ChartWorkspace,
    chart_id: u64,
    status: &str,
    ids: &[u64],
) -> Result<DrawingsChangeResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    Ok(DrawingsChangeResult {
        workspace_id: workspace.id().to_string(),
        chart_id: chart_id.to_string(),
        status: status.to_owned(),
        drawing_ids: ids.iter().map(u64::to_string).collect(),
        drawings: data
            .drawings
            .iter()
            .filter(|drawing| ids.contains(&drawing.id()))
            .filter_map(chart_data::drawing_info)
            .collect(),
        total_drawings: chart_data::counted(data.drawings),
    })
}

/// Adds the drawings as one undo step. All or none: a drawing that cannot be placed refuses the
/// call before anything changes.
pub fn add_drawings(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    specs: &[NewDrawing],
) -> Result<DrawingsChangeResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let first_id = chart_data::next_free_id(data.drawings, data.next_drawing_id);
    let drawings = chart_data::plan_add(data.drawings, data.candles, first_id, specs)?;
    let ids: Vec<u64> = drawings.iter().map(Drawing::id).collect();
    // `plan_add` already refused a range that does not fit.
    let next_free = first_id.saturating_add(ids.len() as u64);
    let commands = drawings.into_iter().map(DrawingCommand::Add).collect();
    workspace.apply_planned_drawings(chart_id, commands, next_free);
    drawings_changed(workspace, chart_id, "added", &ids)
}

pub fn edit_drawing(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    edit: &DrawingEdit,
) -> Result<DrawingsChangeResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let command = chart_data::plan_edit(data.drawings, data.candles, edit)?;
    let status = if command.is_some() {
        "updated"
    } else {
        "unchanged"
    };
    if let Some(command) = command {
        workspace.apply_planned_drawings(chart_id, vec![command], 0);
    }
    drawings_changed(workspace, chart_id, status, &[edit.drawing_id])
}

pub fn remove_drawings(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    ids: &[u64],
) -> Result<DrawingsChangeResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let commands = chart_data::plan_remove(data.drawings, ids)?;
    let removed: Vec<u64> = commands
        .iter()
        .filter_map(|command| match command {
            DrawingCommand::Remove(id) => Some(*id),
            _ => None,
        })
        .collect();
    workspace.apply_planned_drawings(chart_id, commands, 0);
    drawings_changed(workspace, chart_id, "removed", &removed)
}

fn detail_of(workspace_id: u64, view: &PineInstanceView) -> AppliedPineDetail {
    AppliedPineDetail {
        workspace_id: workspace_id.to_string(),
        chart_id: view.chart_id.to_string(),
        instrument: view.instrument.key(),
        timeframe: view.timeframe.label().to_owned(),
        instance: applied_info(view),
        inputs: stored_inputs(view),
        style_overrides: view
            .instance
            .style_overrides
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        bar_count: view.run.as_ref().map(|run| run.bar_count),
        script_status: String::new(),
    }
}

fn color_hex(rgba: u32) -> String {
    format!("#{rgba:08X}")
}

pub fn pine_styles(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
) -> Result<PineStylesResult, McpError> {
    let view = pine_views(workspace, chart_id, Some(instance_id), false)?.remove(0);
    Ok(styles_of(workspace.id(), &view))
}

fn styles_of(workspace_id: u64, view: &PineInstanceView) -> PineStylesResult {
    let base = PineStylesResult {
        workspace_id: workspace_id.to_string(),
        chart_id: view.chart_id.to_string(),
        instance_id: view.instance.id.to_string(),
        available: false,
        reason: None,
        rows: Vec::new(),
    };
    let Some(run) = &view.run else {
        return PineStylesResult {
            reason: Some(
                "The script has not run yet: its style options exist once its chart has drawn it"
                    .into(),
            ),
            ..base
        };
    };
    let overrides = &view.instance.style_overrides;
    let rows = run
        .style_rows
        .iter()
        .map(|row| {
            let key = |attribute: &str| format!("{}.{attribute}", row.id());
            PineStyleRowInfo {
                id: row.id(),
                kind: row.kind.raw_value().to_owned(),
                title: row.title.clone(),
                default_color: row.default_color.map(color_hex),
                default_width: row.default_width,
                overrides: PineStyleOverridesInfo {
                    visible: overrides.get(&key("visible")).map(|value| value != "false"),
                    color: overrides.get(&key("color")).cloned(),
                    width: overrides
                        .get(&key("width"))
                        .and_then(|value| value.parse().ok()),
                },
            }
        })
        .collect();
    PineStylesResult {
        available: true,
        rows,
        ..base
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn direction(is_long: bool) -> String {
    if is_long { "long" } else { "short" }.to_owned()
}

/// A strategy report exactly as the chart computed it (the same numbers the Report tab shows),
/// one page of trades at a time. `script_status` is filled in by the caller.
pub fn pine_report(
    workspace: &ChartWorkspace,
    chart_id: u64,
    instance_id: u64,
    offset: usize,
    limit: usize,
    include_equity: bool,
) -> Result<PineReportResult, McpError> {
    let view = pine_views(workspace, chart_id, Some(instance_id), true)?.remove(0);
    Ok(report_of(
        workspace.id(),
        &view,
        offset,
        limit,
        include_equity,
    ))
}

fn report_of(
    workspace_id: u64,
    view: &PineInstanceView,
    offset: usize,
    limit: usize,
    include_equity: bool,
) -> PineReportResult {
    let (chart_id, instance_id) = (view.chart_id, view.instance.id);
    let unavailable = |reason: &str| PineReportResult {
        workspace_id: workspace_id.to_string(),
        chart_id: chart_id.to_string(),
        instance_id: instance_id.to_string(),
        available: false,
        reason: Some(reason.to_owned()),
        context: None,
        metrics: None,
        settings: None,
        trades: Vec::new(),
        trades_total: 0,
        trades_offset: offset,
        open_trades: Vec::new(),
        equity_curve: Vec::new(),
        equity_points_total: 0,
    };
    let Some(run) = &view.run else {
        return unavailable(
            "No report yet: the chart has not run this script (its window has not drawn it)",
        );
    };
    let Some(report) = &run.strategy else {
        return unavailable(if run.error.is_some() {
            "The script failed to run, so there is no report"
        } else {
            "This script is not a strategy, so it has no report"
        });
    };
    let limit = limit.clamp(1, MAX_TRADES_PAGE);
    let (drawdown, drawdown_fraction) = report.max_drawdown();
    let wins = report.win_count();
    let settings = &report.settings;
    let equity: Vec<f64> = if include_equity {
        let total = report.equity.len();
        if total <= MAX_EQUITY_POINTS {
            report.equity.clone()
        } else {
            (0..MAX_EQUITY_POINTS)
                .map(|index| report.equity[index * (total - 1) / (MAX_EQUITY_POINTS - 1)])
                .collect()
        }
    } else {
        Vec::new()
    };
    PineReportResult {
        available: true,
        reason: None,
        context: Some(PineReportContext {
            kind: "chart_instance".into(),
            script_id: view.instance.script_id.clone(),
            script_name: view.instance.name.clone(),
            source_hash: crate::pine_studio::store::hash_source(&view.instance.source),
            script_status: String::new(),
            instrument: view.instrument.key(),
            timeframe: view.timeframe.label().to_owned(),
            bar_count: run.bar_count,
            inputs: stored_inputs(view),
        }),
        metrics: Some(PineReportMetrics {
            net_profit: finite(report.net_profit()),
            gross_profit: finite(report.gross_profit()),
            gross_loss: finite(report.gross_loss()),
            closed_trades: report.trades.len(),
            winning_trades: wins,
            losing_trades: report
                .trades
                .iter()
                .filter(|trade| trade.profit < 0.0)
                .count(),
            win_rate: report.win_rate().and_then(finite),
            profit_factor: report.profit_factor().and_then(finite),
            average_trade: report.average_trade().and_then(finite),
            final_equity: finite(report.final_equity()),
            return_fraction: finite(report.return_fraction()),
            max_drawdown: finite(drawdown),
            max_drawdown_fraction: finite(drawdown_fraction),
            open_profit: finite(report.open_profit),
            open_trades: report.open_trades.len(),
        }),
        settings: Some(PineStrategySettingsInfo {
            initial_capital: settings.initial_capital,
            quantity_type: match settings.quantity_type {
                crate::pine::strategy::QuantityType::Fixed => "fixed",
                crate::pine::strategy::QuantityType::Cash => "cash",
                crate::pine::strategy::QuantityType::PercentOfEquity => "percent_of_equity",
            }
            .to_owned(),
            quantity_value: settings.quantity_value,
            commission_type: match settings.commission_type {
                crate::pine::strategy::CommissionType::Percent => "percent",
                crate::pine::strategy::CommissionType::CashPerOrder => "cash_per_order",
                crate::pine::strategy::CommissionType::CashPerContract => "cash_per_contract",
            }
            .to_owned(),
            commission_value: settings.commission_value,
            slippage_ticks: settings.slippage,
            pyramiding: settings.pyramiding,
            currency: settings.currency.clone(),
            process_orders_on_close: settings.process_orders_on_close,
            calc_on_every_tick: settings.calc_on_every_tick,
        }),
        trades: report
            .trades
            .iter()
            .skip(offset)
            .take(limit)
            .map(|trade| PineTradeInfo {
                id: trade.id,
                entry_id: trade.entry_id.clone(),
                exit_id: trade.exit_id.clone(),
                direction: direction(trade.is_long),
                quantity: trade.quantity,
                entry_bar: trade.entry_bar,
                entry_time: crate::format::iso8601(trade.entry_time),
                entry_price: trade.entry_price,
                exit_bar: trade.exit_bar,
                exit_time: crate::format::iso8601(trade.exit_time),
                exit_price: trade.exit_price,
                commission: trade.commission,
                profit: trade.profit,
                entry_comment: trade.entry_comment.clone(),
                exit_comment: trade.exit_comment.clone(),
                max_runup: trade.max_runup,
                max_drawdown: trade.max_drawdown,
            })
            .collect(),
        trades_total: report.trades.len(),
        trades_offset: offset,
        open_trades: report
            .open_trades
            .iter()
            .take(MAX_TRADES_PAGE)
            .map(|trade| PineOpenTradeInfo {
                entry_id: trade.entry_id.clone(),
                entry_comment: trade.entry_comment.clone(),
                direction: direction(trade.is_long),
                quantity: trade.quantity,
                entry_bar: trade.entry_bar,
                entry_time: crate::format::iso8601(trade.entry_time),
                entry_price: trade.entry_price,
            })
            .collect(),
        equity_points_total: report.equity.len(),
        equity_curve: equity,
        ..unavailable("")
    }
}

/// The scripts' alert subscriptions as stored. DegenView keeps no log of what they fired, so no
/// history is invented: reading never runs a script or sends anything.
pub fn pine_alerts(
    subscriptions: &[crate::model::pine_alert::PineAlertSubscription],
) -> PineAlertsResult {
    use crate::model::pine_alert::PineAlertState;
    PineAlertsResult {
        total: subscriptions.len(),
        subscriptions: subscriptions
            .iter()
            .take(MAX_ALERTS)
            .map(|subscription| PineAlertInfo {
                subscription_id: subscription.id.to_string(),
                script_id: subscription.script_id.clone(),
                script_name: subscription.script_name.clone(),
                instrument: subscription.symbol_key.clone(),
                timeframe: subscription.timeframe.clone(),
                state: match subscription.state {
                    PineAlertState::Active => "active",
                    PineAlertState::Paused => "paused",
                    PineAlertState::ScriptChanged => "script_changed",
                    PineAlertState::ScriptDeleted => "script_deleted",
                }
                .to_owned(),
                note: subscription.note.clone(),
                chart_id: subscription.chart_id.to_string(),
                instance_id: subscription.instance_id.map(|id| id.to_string()),
                source_hash: subscription.source_hash.clone(),
                sends_to_webhooks: !subscription.webhook_endpoint_ids.is_empty(),
                created_at: crate::format::iso8601(subscription.created_at_ms),
            })
            .collect(),
        event_history_available: false,
        recent_events: Vec::new(),
    }
}

// MARK: Price alerts (run on the app thread, against the one `AlertHost`)

/// Alerts one `degenview_list_price_alerts` call returns.
const ALERT_PAGE: usize = 200;
/// No further alert is accepted from a client beyond this many in total.
pub const MAX_ALERTS: usize = 500;

/// A number as a plain decimal (`70000`, `0.00000278`, `2.5`): rounded to 12 significant digits,
/// which drops float noise (`110.00000000000001`) but not any digit a quote carries, and never
/// written in exponent form.
fn decimal(value: f64) -> String {
    let rounded = format!("{value:.11e}").parse::<f64>().unwrap_or(value);
    format!("{rounded}")
}

pub fn alert_info(alert: &PriceAlert, last_triggered_ms: Option<i64>) -> AlertInfo {
    let currency = alert.currency;
    let price = decimal;
    let (condition, percent, reference) = match &alert.condition {
        AlertCondition::CrossesAbove { .. } => ("crosses_above", None, None),
        AlertCondition::CrossesBelow { .. } => ("crosses_below", None, None),
        AlertCondition::RisesBy {
            percent, reference, ..
        } => ("rises_by", Some(decimal(*percent)), Some(price(*reference))),
        AlertCondition::FallsBy {
            percent, reference, ..
        } => ("falls_by", Some(decimal(*percent)), Some(price(*reference))),
        AlertCondition::Unsupported { .. } => ("unsupported", None, None),
    };
    AlertInfo {
        alert_id: alert.id.to_string(),
        instrument: alert.asset.key(),
        source: source_slug(alert.asset.source()).to_owned(),
        symbol: alert.asset.instrument.symbol.clone(),
        condition: condition.to_owned(),
        target_price: alert.condition.target().map(price),
        percent,
        reference_price: reference,
        currency: currency.code().to_owned(),
        frequency: match alert.frequency {
            AlertFrequency::Once => "once",
            AlertFrequency::EveryTime => "every_time",
        }
        .to_owned(),
        status: match alert.state {
            AlertState::Active => "active",
            AlertState::Paused => "paused",
            AlertState::Triggered => "triggered",
            AlertState::Unsupported => "unsupported",
        }
        .to_owned(),
        note: alert.note.clone(),
        sends_to_webhooks: !alert.webhook_endpoint_ids.is_empty(),
        created_at: crate::format::iso8601(alert.created_at_ms),
        updated_at: crate::format::iso8601(alert.updated_at_ms),
        last_triggered_at: last_triggered_ms.map(crate::format::iso8601),
    }
}

fn last_triggered(host: &AlertHost, id: AlertId) -> Option<i64> {
    host.snapshot()
        .history
        .iter()
        .filter(|event| event.alert_id == id)
        .map(|event| event.timestamp_ms)
        .max()
}

fn info_for(host: &AlertHost, alert: &PriceAlert) -> AlertInfo {
    alert_info(alert, last_triggered(host, alert.id))
}

/// Alerts as the runtime holds them: the page, or just the one asked for.
pub fn list_alerts(host: &AlertHost, only: Option<AlertId>) -> AlertsResult {
    let alerts = &host.snapshot().alerts;
    if let Some(id) = only {
        let found: Vec<_> = alerts
            .iter()
            .filter(|alert| alert.id == id)
            .map(|alert| info_for(host, alert))
            .collect();
        return AlertsResult {
            total: found.len(),
            truncated: false,
            alerts: found,
        };
    }
    AlertsResult {
        total: alerts.len(),
        truncated: alerts.len() > ALERT_PAGE,
        alerts: alerts
            .iter()
            .take(ALERT_PAGE)
            .map(|alert| info_for(host, alert))
            .collect(),
    }
}

pub enum CreatePlan {
    /// An identical alert exists already; hand it back instead of adding a duplicate.
    Existing(Box<AlertInfo>),
    New {
        alert: Box<PriceAlert>,
        /// The market now, in the alert's currency, when a live price is known.
        baseline: Option<f64>,
    },
}

pub(super) fn require_loaded(host: &AlertHost) -> Result<(), McpError> {
    match host.load_error() {
        None => Ok(()),
        Some(_) => Err(McpError::AppUnavailable(
            "Alert data could not be loaded, so alerts cannot be changed".into(),
        )),
    }
}

/// Builds the alert exactly as the price-alert sheet does (its own parsing and rules), without
/// saving it. The caller commits a `New` plan through the same path the sheet's Save uses.
pub fn plan_create_alert(
    host: &AlertHost,
    book: &WatchlistQuoteBook,
    rates: &std::collections::HashMap<AlertCurrency, f64>,
    request: &NewAlert,
    now_ms: i64,
) -> Result<CreatePlan, McpError> {
    require_loaded(host)?;
    if host.snapshot().alerts.len() >= MAX_ALERTS {
        return Err(McpError::ResourceLimit(format!(
            "There are already {MAX_ALERTS} price alerts; delete some first"
        )));
    }
    let asset =
        ChartWorkspace::alert_asset_for(&request.instrument.symbol, request.instrument.source);
    let mut editor = PriceAlertEditorState::new(asset, None, Some(request.currency));
    editor.direction = if request.crosses_above {
        Direction::Above
    } else {
        Direction::Below
    };
    editor.target = request.target_text.clone();
    editor.frequency = request.frequency;
    editor.note = request.note.clone();
    editor.feed_from_book(book, rates);
    let alert = editor.result(now_ms).ok_or_else(|| {
        McpError::InvalidArgument("target_price must be a positive number".into())
    })?;
    if let Some(existing) = host.find_identical(&alert) {
        return Ok(CreatePlan::Existing(Box::new(info_for(host, existing))));
    }
    Ok(CreatePlan::New {
        baseline: editor.current_price(),
        alert: Box::new(alert),
    })
}

pub enum UpdatePlan {
    /// The request matches the alert as it is: nothing to queue.
    Unchanged(Box<AlertInfo>),
    Change {
        alert: Box<PriceAlert>,
        /// The alert as the edit was planned from: the runtime applies the edit only while the
        /// alert is still like this.
        base: Box<PriceAlert>,
        baseline: Option<f64>,
    },
}

/// Edits an alert as the Alerts window's Edit sheet does (its parsing, currency and arming rules),
/// without saving it. A remote edit never resumes a paused alert, and an alert that already fired
/// only becomes active again when its condition, currency or frequency changed. Webhook settings
/// are carried over untouched.
pub fn plan_update_alert(
    host: &AlertHost,
    book: &WatchlistQuoteBook,
    rates: &std::collections::HashMap<AlertCurrency, f64>,
    request: &AlertUpdate,
    now_ms: i64,
) -> Result<UpdatePlan, McpError> {
    require_loaded(host)?;
    let existing = find_alert(host, request.alert_id)?.clone();
    let percentage = matches!(
        existing.condition,
        AlertCondition::RisesBy { .. } | AlertCondition::FallsBy { .. }
    );
    if existing.condition.target().is_none() {
        return Err(McpError::Unsupported(
            "This alert uses a condition this version of DegenView does not support".into(),
        ));
    }
    if percentage
        && (request.crosses_above.is_some()
            || request.target_text.is_some()
            || request.currency.is_some())
    {
        return Err(McpError::Unsupported(
            "A percentage alert can only have its frequency and note changed".into(),
        ));
    }
    let mut editor = PriceAlertEditorState::new(
        existing.asset.clone(),
        Some(existing.clone()),
        Some(existing.currency),
    );
    if let Some(currency) = request.currency {
        editor.set_currency(currency);
    }
    if let Some(above) = request.crosses_above {
        editor.direction = if above {
            Direction::Above
        } else {
            Direction::Below
        };
    }
    if let Some(target) = &request.target_text {
        editor.target = target.clone();
    }
    if let Some(frequency) = request.frequency {
        editor.frequency = frequency;
    }
    if let Some(note) = &request.note {
        editor.note = note.clone();
    }
    editor.feed_from_book(book, rates);
    let mut alert = editor.result(now_ms).ok_or_else(|| {
        McpError::InvalidArgument("target_price must be a positive number".into())
    })?;
    let settings_changed = alert.condition != existing.condition
        || alert.currency != existing.currency
        || alert.frequency != existing.frequency;
    alert.state = match existing.state {
        AlertState::Paused => AlertState::Paused,
        AlertState::Triggered if !settings_changed => AlertState::Triggered,
        _ => AlertState::Active,
    };
    if !settings_changed && alert.note == existing.note && alert.state == existing.state {
        return Ok(UpdatePlan::Unchanged(Box::new(info_for(host, &existing))));
    }
    if let Some(other) = host.find_identical(&alert) {
        return Err(McpError::Conflict(format!(
            "Price alert {} already has exactly these settings",
            other.id
        )));
    }
    Ok(UpdatePlan::Change {
        baseline: editor.current_price(),
        alert: Box::new(alert),
        base: Box::new(existing),
    })
}

pub enum StatePlan {
    /// Already in the requested state: nothing to queue.
    Already,
    Change {
        id: AlertId,
        state: AlertState,
    },
}

pub(super) fn find_alert(host: &AlertHost, id: AlertId) -> Result<&PriceAlert, McpError> {
    host.snapshot()
        .alerts
        .iter()
        .find(|alert| alert.id == id)
        .ok_or_else(|| McpError::NotFound(format!("No price alert {id}")))
}

/// Pause or resume, deciding as the Alerts window's toggle does: pausing an active alert, and
/// resuming anything else (a fired once-alert is re-armed).
pub fn plan_pause_alert(
    host: &AlertHost,
    id: AlertId,
    paused: bool,
) -> Result<StatePlan, McpError> {
    require_loaded(host)?;
    let alert = find_alert(host, id)?;
    if alert.state == AlertState::Unsupported {
        return Err(McpError::Unsupported(
            "This alert uses a condition this version of DegenView does not support".into(),
        ));
    }
    let already = if paused {
        alert.state == AlertState::Paused
    } else {
        alert.state == AlertState::Active
    };
    if already {
        return Ok(StatePlan::Already);
    }
    Ok(StatePlan::Change {
        id,
        state: if paused {
            AlertState::Paused
        } else {
            AlertState::Active
        },
    })
}

/// What became of the alert command `id`: applied, refused because the alerts changed under it,
/// or not applied yet.
pub fn alert_command_outcome(host: &AlertHost, id: u64) -> AlertCommandOutcome {
    if let Some(reason) = host.rejection(id) {
        AlertCommandOutcome::Rejected(reason)
    } else if host.snapshot().processed_command_ids.contains(&id) {
        AlertCommandOutcome::Applied
    } else {
        AlertCommandOutcome::Pending
    }
}

pub fn plan_delete_alert(host: &AlertHost, id: AlertId) -> Result<AlertId, McpError> {
    require_loaded(host)?;
    find_alert(host, id).map(|alert| alert.id)
}

/// Takes a built-in indicator off a chart through the workspace's own method.
pub fn remove_indicator(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    indicator: BuiltIn,
) -> Result<AddIndicatorResult, McpError> {
    let workspace_id = workspace.id();
    match workspace.remove_indicator(chart_id, indicator) {
        Ok((outcome, config)) => Ok(AddIndicatorResult {
            workspace_id: workspace_id.to_string(),
            chart_id: chart_id.to_string(),
            indicator: indicator.id().to_owned(),
            status: match outcome {
                RemoveOutcome::Removed => "removed",
                RemoveOutcome::NotApplied => "not_applied",
            }
            .to_owned(),
            indicators: indicators_info(config),
        }),
        Err(_) => Err(no_chart(workspace, chart_id)),
    }
}

/// Indicator readings over a market chart's own candles, cut at the replay position.
pub fn indicator_readings(
    workspace: &ChartWorkspace,
    chart_id: u64,
    request: &ReadingRequest,
) -> Result<IndicatorValuesResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    // A constructed chart type (Heikin Ashi, Renko, …) draws bars built from the candles, and its
    // indicators are computed over those: read the same ones.
    let constructed = workspace.constructed_bars(chart_id);
    let meta = ReadingMeta {
        workspace_id: workspace.id(),
        chart_id,
        instrument: data.instrument.key(),
        timeframe: workspace.timeframe().label().to_owned(),
        chart_type: data.chart_type.slug().to_owned(),
        constructed: constructed.is_some(),
        replay_ms: data.replay_ms,
        now_ms: now_ms(),
    };
    let bars = constructed.as_deref().unwrap_or(data.candles);
    Ok(indicator_values(&meta, bars, request))
}

/// Session statistics over a market chart's own candles, cut at the replay position. Never the
/// bars of a constructed chart type: a Renko brick or a Heikin Ashi bar is not a trading day.
pub fn session_statistics(
    workspace: &ChartWorkspace,
    chart_id: u64,
    query: &crate::mcp::session_stats::Query,
) -> Result<crate::mcp::session_stats::SessionStatsResult, McpError> {
    let data = workspace
        .chart_data(chart_id)
        .ok_or_else(|| no_chart(workspace, chart_id))?;
    let meta = crate::mcp::session_stats::Meta {
        workspace_id: workspace.id(),
        chart_id,
        instrument: data.instrument.key(),
        timeframe: workspace.timeframe().label().to_owned(),
        bar_ms: crate::model::session_stats::bar_ms(workspace.timeframe()),
        hours: crate::model::session_stats::MarketHours::for_source(data.instrument.source),
    };
    crate::mcp::session_stats::session_stats(&meta, data.candles, query)
}

/// Moves the workspace to another timeframe through the toolbar's own path.
pub fn set_timeframe(
    workspace: &mut ChartWorkspace,
    timeframe: Timeframe,
    ctx: &eframe::egui::Context,
) -> TimeframeResult {
    let previous = workspace.timeframe();
    let changed = workspace.set_timeframe_for_mcp(timeframe, ctx);
    TimeframeResult {
        workspace_id: workspace.id().to_string(),
        timeframe: workspace.timeframe().label().to_owned(),
        previous_timeframe: previous.label().to_owned(),
        status: if changed { "changed" } else { "unchanged" }.to_owned(),
        chart_count: workspace.chart_count(),
    }
}

fn chart_now(workspace: &ChartWorkspace, chart_id: u64) -> Option<ChartInfo> {
    let summary = workspace
        .chart_summaries()
        .into_iter()
        .find(|chart| chart.id == chart_id)?;
    Some(chart_info(workspace, summary))
}

/// Switches a market chart's type through the chart's own method.
pub fn set_chart_type(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    chart_type: ChartType,
) -> Result<ChartChangeResult, McpError> {
    let changed = match workspace.set_chart_type_for_mcp(chart_id, chart_type) {
        Ok(changed) => changed,
        Err(ChartTypeError::NoSuchChart) => return Err(no_chart(workspace, chart_id)),
        Err(ChartTypeError::Unsupported(reason)) => {
            return Err(McpError::Unsupported(format!(
                "{} is not available on this chart: {reason}",
                chart_type.label()
            )));
        }
    };
    Ok(ChartChangeResult {
        workspace_id: workspace.id().to_string(),
        chart_id: chart_id.to_string(),
        status: if changed { "changed" } else { "unchanged" }.to_owned(),
        chart: chart_now(workspace, chart_id),
        charts_in_workspace: workspace.chart_count(),
    })
}

/// Removes a market chart through the workspace's own path.
pub fn remove_chart(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
) -> Result<ChartChangeResult, McpError> {
    if !workspace.remove_market_chart(chart_id) {
        return Err(no_chart(workspace, chart_id));
    }
    Ok(ChartChangeResult {
        workspace_id: workspace.id().to_string(),
        chart_id: chart_id.to_string(),
        status: "removed".to_owned(),
        chart: None,
        charts_in_workspace: workspace.chart_count(),
    })
}

// MARK: Snapshots (run on the app thread)

/// A picture being made for an agent: frozen on the UI thread, drawn on the snapshot worker.
pub struct SnapshotJob {
    receiver: std::sync::mpsc::Receiver<Result<Vec<u8>, String>>,
    width: u32,
    height: u32,
    started: Instant,
}

/// The pictures being made for agents. Bounded, and a job nobody asks for is forgotten.
#[derive(Default)]
pub struct SnapshotJobs {
    next: u64,
    jobs: std::collections::HashMap<u64, SnapshotJob>,
}

impl SnapshotJobs {
    const MAX_ACTIVE: usize = 4;
    const MAX_AGE: Duration = Duration::from_secs(120);

    /// Forgets the pictures nobody collected in time (and with them the PNGs they hold). The app
    /// calls it every tick, so they do not wait for another snapshot call to be reaped.
    pub fn expire(&mut self) {
        self.jobs
            .retain(|_, job| job.started.elapsed() < Self::MAX_AGE);
    }

    pub fn begin(
        &mut self,
        workspace: &ChartWorkspace,
        chart_id: u64,
        scale: crate::model::snapshot::SnapshotScale,
        ctx: &eframe::egui::Context,
    ) -> Result<SnapshotStarted, McpError> {
        self.expire();
        if self.jobs.len() >= Self::MAX_ACTIVE {
            return Err(McpError::ResourceLimit(
                "Too many chart pictures are being made; try again shortly".into(),
            ));
        }
        let (receiver, dimensions) = workspace
            .snapshot_png_for_mcp(ctx, chart_id, scale)
            .map_err(|message| {
                if message.starts_with("There is no such") {
                    no_chart(workspace, chart_id)
                } else {
                    McpError::Conflict(message)
                }
            })?;
        self.next += 1;
        self.jobs.insert(
            self.next,
            SnapshotJob {
                receiver,
                width: dimensions.width,
                height: dimensions.height,
                started: Instant::now(),
            },
        );
        Ok(SnapshotStarted {
            job_id: self.next,
            width: dimensions.width,
            height: dimensions.height,
        })
    }

    pub fn poll(&mut self, job_id: u64) -> Result<SnapshotProgress, McpError> {
        self.expire();
        let Some(job) = self.jobs.get(&job_id) else {
            return Err(McpError::NotFound(
                "No such picture job (it finished, was collected or expired)".into(),
            ));
        };
        match job.receiver.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(SnapshotProgress::Pending),
            Ok(Ok(png)) => {
                let (width, height) = (job.width, job.height);
                self.jobs.remove(&job_id);
                if png.len() > crate::model::snapshot::MAX_MCP_PNG_BYTES {
                    return Err(McpError::ResourceLimit(format!(
                        "The picture is {} MiB, over the {} MiB an MCP reply carries; ask for \
                         scale 1x or a chart with fewer or smaller images",
                        png.len().div_ceil(1024 * 1024),
                        crate::model::snapshot::MAX_MCP_PNG_BYTES / (1024 * 1024)
                    )));
                }
                Ok(SnapshotProgress::Ready {
                    png: Arc::new(png),
                    width,
                    height,
                })
            }
            Ok(Err(message)) => {
                self.jobs.remove(&job_id);
                Err(McpError::AppUnavailable(message))
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.jobs.remove(&job_id);
                Err(McpError::AppUnavailable(
                    "The picture stopped before it finished".into(),
                ))
            }
        }
    }
}

// MARK: Replay (run on the app thread)

pub fn replay_state(workspace: &ChartWorkspace) -> ReplayResult {
    replay_tools::replay_result(
        workspace.id(),
        workspace.timeframe().label(),
        &workspace.replay_view(),
    )
}

pub fn control_replay(
    workspace: &mut ChartWorkspace,
    command: ReplayCommand,
) -> Result<ReplayResult, McpError> {
    let view =
        workspace.replay_command(command).map_err(|error| {
            match error {
        ReplayError::NoSuchChart => McpError::NotFound(format!(
            "No market chart with that id in workspace {}",
            workspace.id()
        )),
        ReplayError::NoCandles => McpError::Conflict(
            "The chart has no candles yet (it may still be loading), so there is nothing to replay"
                .into(),
        ),
        ReplayError::NotRunning => McpError::Conflict(
            "No replay is running in this workspace: start one with action `start`".into(),
        ),
    }
        })?;
    Ok(replay_tools::replay_result(
        workspace.id(),
        workspace.timeframe().label(),
        &view,
    ))
}

// MARK: Paper trading (run on the app thread, read-only)

fn paper_account(
    host: &PaperHost,
    account_id: u64,
) -> Result<&crate::model::paper::PaperAccount, McpError> {
    if host.load_failed() {
        return Err(McpError::AppUnavailable(
            "Paper-trading data could not be read, so it is unavailable".into(),
        ));
    }
    host.snapshot()
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| {
            McpError::NotFound(
                "No such paper account: use an account_id from degenview_list_paper_accounts"
                    .into(),
            )
        })
}

fn paper_account_info(
    host: &PaperHost,
    account: &crate::model::paper::PaperAccount,
) -> PaperAccountInfo {
    paper::account_info(
        account,
        &host.metrics(account.id),
        host.snapshot().selected_account_id == Some(account.id),
        host.owned_elsewhere(account.id).map(owner_text),
    )
}

pub fn list_paper_accounts(host: &PaperHost) -> Result<PaperAccountsResult, McpError> {
    if host.load_failed() {
        return Err(McpError::AppUnavailable(
            "Paper-trading data could not be read, so it is unavailable".into(),
        ));
    }
    Ok(PaperAccountsResult {
        accounts: host
            .snapshot()
            .accounts
            .iter()
            .map(|account| paper_account_info(host, account))
            .collect(),
    })
}

pub fn get_paper_account(
    host: &PaperHost,
    account_id: u64,
) -> Result<PaperAccountDetail, McpError> {
    let account = paper_account(host, account_id)?;
    let snapshot = host.snapshot();
    Ok(PaperAccountDetail {
        account: paper_account_info(host, account),
        positions: snapshot
            .positions
            .iter()
            .filter(|position| position.account_id == account_id && position.signed_quantity != 0.0)
            .map(|position| {
                paper::position_info(
                    position,
                    host.mark(position),
                    host.unrealized_pnl(position),
                    host.return_ratio(position),
                )
            })
            .collect(),
        open_orders: snapshot
            .orders
            .iter()
            .filter(|order| order.account_id == account_id && paper::is_open(order))
            .map(paper::order_info)
            .collect(),
    })
}

pub fn paper_fills(
    host: &PaperHost,
    account_id: u64,
    offset: usize,
    limit: usize,
) -> Result<PaperFillsResult, McpError> {
    paper_account(host, account_id)?;
    let fills: Vec<_> = host
        .snapshot()
        .fills
        .iter()
        .filter(|fill| fill.account_id == account_id)
        .collect();
    Ok(paper::fills_result(account_id, &fills, offset, limit))
}

pub fn paper_trades(
    host: &PaperHost,
    account_id: u64,
    offset: usize,
    limit: usize,
) -> Result<PaperTradesResult, McpError> {
    paper_account(host, account_id)?;
    let trades: Vec<_> = host
        .snapshot()
        .closed_trades
        .iter()
        .filter(|trade| trade.account_id == account_id)
        .collect();
    Ok(paper::trades_result(account_id, &trades, offset, limit))
}

// MARK: Paper trading writes (run on the app thread)

/// Placed orders by `request_id`, so a retry of a call that timed out does not place a second
/// order. Held in memory only and bounded: it is a guard against an agent's retry, not a ledger.
/// A record keeps the request it was made for, so the same id with another order is a conflict
/// rather than a silent answer with the first one.
#[derive(Default)]
pub struct OrderMemory {
    entries: std::collections::VecDeque<(String, crate::model::paper::PaperOrderRequest, u64)>,
}

impl OrderMemory {
    const CAPACITY: usize = 512;

    fn key(account_id: u64, request_id: &str) -> String {
        format!("{account_id}:{request_id}")
    }

    /// The request and order id recorded for `request_id` on `account_id`.
    pub fn recall(
        &self,
        account_id: u64,
        request_id: &str,
    ) -> Option<(&crate::model::paper::PaperOrderRequest, u64)> {
        let key = Self::key(account_id, request_id);
        self.entries
            .iter()
            .find(|(known, _, _)| *known == key)
            .map(|(_, request, order_id)| (request, *order_id))
    }

    /// Drops the record, when the order it names no longer exists.
    pub fn forget(&mut self, account_id: u64, request_id: &str) {
        let key = Self::key(account_id, request_id);
        self.entries.retain(|(known, _, _)| *known != key);
    }

    /// Records `order_id`, replacing any earlier record of the same id.
    pub fn remember(
        &mut self,
        account_id: u64,
        request_id: &str,
        request: &crate::model::paper::PaperOrderRequest,
        order_id: u64,
    ) {
        self.forget(account_id, request_id);
        if self.entries.len() >= Self::CAPACITY {
            self.entries.pop_front();
        }
        self.entries
            .push_back((Self::key(account_id, request_id), request.clone(), order_id));
    }
}

fn paper_error(error: PaperHostError) -> McpError {
    use crate::model::paper::PaperTradingError as E;
    let message = error.message();
    match error {
        PaperHostError::Engine(E::AccountNotFound | E::OrderNotFound) => {
            McpError::NotFound(message)
        }
        PaperHostError::Engine(
            E::InvalidQuantity(_) | E::InvalidPrice(_) | E::UnsupportedSymbol,
        ) => McpError::InvalidArgument(message),
        PaperHostError::Engine(_) => McpError::Conflict(message),
        _ => McpError::AppUnavailable(message),
    }
}

/// An account this Mac runs. One another Mac runs is not touched: acting on it from the GUI takes
/// it over, which an agent must not do silently.
fn tradable_account(host: &PaperHost, account_id: u64) -> Result<(), McpError> {
    paper_account(host, account_id)?;
    if let Some(elsewhere) = host.owned_elsewhere(account_id) {
        return Err(McpError::Conflict(format!(
            "{} Change it in DegenView to take it over; it is not changed over MCP.",
            owner_text(elsewhere)
        )));
    }
    Ok(())
}

/// The name of a paper account an agent may trade: it exists and this Mac runs it.
pub(super) fn tradable_paper_account(
    host: &PaperHost,
    account_id: u64,
) -> Result<String, McpError> {
    tradable_account(host, account_id)?;
    Ok(paper_account(host, account_id)?.name.clone())
}

fn account_after(host: &PaperHost, account_id: u64) -> Result<PaperAccountInfo, McpError> {
    paper_account(host, account_id).map(|account| paper_account_info(host, account))
}

fn fills_of(host: &PaperHost, order_ids: &[u64]) -> Vec<paper::PaperFillInfo> {
    host.snapshot()
        .fills
        .iter()
        .filter(|fill| order_ids.contains(&fill.order_id))
        .map(paper::fill_info)
        .collect()
}

fn orders_of(host: &PaperHost, order_ids: &[u64]) -> Vec<paper::PaperOrderInfo> {
    order_ids
        .iter()
        .filter_map(|id| host.snapshot().orders.iter().find(|order| order.id == *id))
        .map(paper::order_info)
        .collect()
}

pub fn place_paper_order(
    host: &mut PaperHost,
    memory: &mut OrderMemory,
    request: crate::model::paper::PaperOrderRequest,
    dry_run: bool,
    request_id: Option<&str>,
) -> Result<PaperOrderResult, McpError> {
    let account_id = request.account_id;
    tradable_account(host, account_id)?;
    if let Some(request_id) = request_id
        && let Some((earlier, order_id)) = memory
            .recall(account_id, request_id)
            .map(|(earlier, order_id)| (earlier.clone(), order_id))
    {
        let ids = [order_id];
        if let Some(order) = orders_of(host, &ids).into_iter().next() {
            if earlier != request {
                return Err(McpError::Conflict(
                    "That request_id was used for a different order. Use a new request_id for a \
                     different order, or repeat the same order to get the first one back"
                        .into(),
                ));
            }
            return Ok(PaperOrderResult {
                status: "already_placed".into(),
                order: Some(order),
                bracket_orders: bracket_of(host, order_id),
                fills: fills_of(host, &ids),
                account: Some(account_after(host, account_id)?),
            });
        }
        // The order is gone (the account was reset): the id is free again.
        memory.forget(account_id, request_id);
    }
    if dry_run {
        host.check_order(request).map_err(paper_error)?;
        return Ok(PaperOrderResult {
            status: "would_place".into(),
            order: None,
            bracket_orders: Vec::new(),
            fills: Vec::new(),
            account: Some(account_after(host, account_id)?),
        });
    }
    let order_id = host.submit(request.clone()).map_err(paper_error)?;
    if let Some(request_id) = request_id {
        memory.remember(account_id, request_id, &request, order_id);
    }
    Ok(PaperOrderResult {
        status: "placed".into(),
        order: orders_of(host, &[order_id]).into_iter().next(),
        bracket_orders: bracket_of(host, order_id),
        fills: fills_of(host, &[order_id]),
        account: Some(account_after(host, account_id)?),
    })
}

/// The exit orders of a bracket, which are children of its entry order.
fn bracket_of(host: &PaperHost, entry_id: u64) -> Vec<paper::PaperOrderInfo> {
    host.snapshot()
        .orders
        .iter()
        .filter(|order| order.parent_order_id == Some(entry_id))
        .map(paper::order_info)
        .collect()
}

pub fn modify_paper_order(
    host: &mut PaperHost,
    order_id: u64,
    changes: crate::model::paper::PaperOrderChanges,
) -> Result<PaperChangeResult, McpError> {
    let account_id = host
        .snapshot()
        .orders
        .iter()
        .find(|order| order.id == order_id)
        .map(|order| order.account_id)
        .ok_or_else(|| McpError::NotFound("No such paper order".into()))?;
    tradable_account(host, account_id)?;
    host.modify(order_id, changes).map_err(paper_error)?;
    Ok(PaperChangeResult {
        status: "modified".into(),
        orders: orders_of(host, &[order_id]),
        fills: fills_of(host, &[order_id]),
        failures: Vec::new(),
        account: Some(account_after(host, account_id)?),
    })
}

pub fn cancel_paper_orders(
    host: &mut PaperHost,
    account_id: u64,
    order_ids: Option<&[u64]>,
) -> Result<PaperChangeResult, McpError> {
    tradable_account(host, account_id)?;
    let working: Vec<u64> = host
        .snapshot()
        .orders
        .iter()
        .filter(|order| order.account_id == account_id && order.status.is_working())
        .map(|order| order.id)
        .collect();
    let targets: Vec<u64> = match order_ids {
        None => working.clone(),
        Some(ids) => ids.to_vec(),
    };
    let (mut done, mut failures) = (Vec::new(), Vec::new());
    for id in &targets {
        // Naming an order of another account is refused, not cancelled.
        let belongs = host
            .snapshot()
            .orders
            .iter()
            .any(|order| order.id == *id && order.account_id == account_id);
        if !belongs {
            failures.push(format!("Order {id}: not an order of this account"));
            continue;
        }
        match host.cancel(*id) {
            Ok(()) => done.push(*id),
            Err(error) => failures.push(format!("Order {id}: {}", error.message())),
        }
    }
    Ok(PaperChangeResult {
        status: if done.is_empty() && failures.is_empty() {
            "nothing_to_do"
        } else {
            "canceled"
        }
        .into(),
        orders: orders_of(host, &done),
        fills: Vec::new(),
        failures,
        account: Some(account_after(host, account_id)?),
    })
}

pub fn close_paper_positions(
    host: &mut PaperHost,
    account_id: u64,
    instrument_key: Option<&str>,
) -> Result<PaperChangeResult, McpError> {
    tradable_account(host, account_id)?;
    let keys: Vec<String> = host
        .snapshot()
        .positions
        .iter()
        .filter(|position| position.account_id == account_id && position.signed_quantity != 0.0)
        .map(|position| position.instrument.key.clone())
        .filter(|key| instrument_key.is_none_or(|wanted| wanted == key))
        .collect();
    if keys.is_empty() && instrument_key.is_some() {
        return Err(McpError::NotFound(
            "That account has no open position in that market".into(),
        ));
    }
    let (mut order_ids, mut failures) = (Vec::new(), Vec::new());
    for key in &keys {
        match host.close_position(account_id, key) {
            Ok(order_id) => order_ids.push(order_id),
            Err(error) => failures.push(format!("{key}: {}", error.message())),
        }
    }
    Ok(PaperChangeResult {
        status: if keys.is_empty() {
            "nothing_to_do"
        } else {
            "closed"
        }
        .into(),
        orders: orders_of(host, &order_ids),
        fills: fills_of(host, &order_ids),
        failures,
        account: Some(account_after(host, account_id)?),
    })
}

// MARK: Portfolio (run on the app thread, read-only)

fn readable_ledger(
    host: &PortfolioHost,
) -> Result<&crate::model::portfolio::PortfolioLedgerSnapshot, McpError> {
    if host.load_failed() {
        return Err(McpError::AppUnavailable(
            "Portfolio data could not be read, so it is unavailable".into(),
        ));
    }
    Ok(host.snapshot())
}

/// The portfolio ids a read covers: the one asked for, or every unarchived portfolio.
fn portfolio_scope(
    ledger: &crate::model::portfolio::PortfolioLedgerSnapshot,
    portfolio_id: Option<u64>,
) -> Result<(Option<PortfolioInfo>, std::collections::HashSet<u64>), McpError> {
    match portfolio_id {
        Some(id) => {
            let portfolio = ledger
                .portfolios
                .iter()
                .find(|portfolio| portfolio.id == id)
                .ok_or_else(|| {
                    McpError::NotFound(
                        "No such portfolio: use a portfolio_id from degenview_list_portfolios"
                            .into(),
                    )
                })?;
            let count = ledger
                .transactions
                .iter()
                .filter(|transaction| transaction.portfolio_id == id)
                .count();
            Ok((
                Some(portfolio::portfolio_info(
                    portfolio,
                    count,
                    ledger.selected_portfolio_id == Some(id),
                )),
                [id].into_iter().collect(),
            ))
        }
        None => Ok((
            None,
            ledger
                .portfolios
                .iter()
                .filter(|portfolio| !portfolio.is_archived)
                .map(|portfolio| portfolio.id)
                .collect(),
        )),
    }
}

pub fn list_portfolios(host: &PortfolioHost) -> Result<PortfoliosResult, McpError> {
    let ledger = readable_ledger(host)?;
    Ok(PortfoliosResult {
        portfolios: ledger
            .portfolios
            .iter()
            .map(|portfolio| {
                portfolio::portfolio_info(
                    portfolio,
                    ledger
                        .transactions
                        .iter()
                        .filter(|transaction| transaction.portfolio_id == portfolio.id)
                        .count(),
                    ledger.selected_portfolio_id == Some(portfolio.id),
                )
            })
            .collect(),
        privacy_mode: host.privacy_mode(),
    })
}

pub fn portfolio_holdings_base(
    host: &PortfolioHost,
    portfolio_id: Option<u64>,
    now_ms: i64,
) -> Result<HoldingsBase, McpError> {
    let ledger = readable_ledger(host)?;
    let (info, ids) = portfolio_scope(ledger, portfolio_id)?;
    let (holdings, closed) = crate::model::portfolio_accounting::holdings_and_closed(
        &ledger.transactions,
        &ids,
        None,
        &std::collections::HashMap::new(),
        now_ms,
    )
    .map_err(|error| {
        McpError::Conflict(format!(
            "The ledger cannot be replayed, so there are no holdings: {}",
            error.message()
        ))
    })?;
    Ok(HoldingsBase {
        portfolio: info,
        holdings: holdings
            .into_iter()
            .map(|holding| {
                let mut currencies = ledger
                    .transactions
                    .iter()
                    .filter(|transaction| {
                        ids.contains(&transaction.portfolio_id)
                            && transaction.asset.key == holding.asset.key
                            && transaction.price.is_some()
                    })
                    .map(|transaction| transaction.price_currency);
                let first = currencies.next();
                let cost_currency = first.filter(|first| currencies.all(|next| next == *first));
                HoldingBase {
                    instrument: portfolio::instrument_of(
                        holding.asset.source,
                        &holding.asset.key,
                        &holding.asset.symbol,
                    ),
                    cost_currency,
                    holding,
                }
            })
            .collect(),
        closed_realized: closed
            .into_iter()
            .map(|closed| (closed.asset.key, closed.realized_pnl))
            .collect(),
    })
}

pub fn portfolio_transactions(
    host: &PortfolioHost,
    portfolio_id: Option<u64>,
    asset: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<TransactionsResult, McpError> {
    let ledger = readable_ledger(host)?;
    let (_, ids) = portfolio_scope(ledger, portfolio_id)?;
    let wanted = asset.map(str::to_uppercase);
    let rows: Vec<_> = ledger
        .transactions
        .iter()
        .filter(|transaction| ids.contains(&transaction.portfolio_id))
        .filter(|transaction| {
            wanted
                .as_deref()
                .is_none_or(|wanted| transaction.asset.key.to_uppercase() == wanted)
        })
        .collect();
    Ok(portfolio::transactions_result(&rows, offset, limit))
}

// MARK: Watchlists (run on the app thread)

fn watchlist_error(error: WatchlistError) -> McpError {
    match error {
        WatchlistError::NotFound => McpError::NotFound(error.to_string()),
        WatchlistError::Duplicate(_) => McpError::Conflict(error.to_string()),
        WatchlistError::EmptyName
        | WatchlistError::NameTooLong
        | WatchlistError::InvalidName
        | WatchlistError::AmbiguousSection(_) => McpError::InvalidArgument(error.to_string()),
        WatchlistError::ListFull | WatchlistError::TooManyLists => {
            McpError::ResourceLimit(error.to_string())
        }
        WatchlistError::PersistenceUnavailable | WatchlistError::SaveFailed => {
            McpError::AppUnavailable(error.to_string())
        }
    }
}

fn readable_watchlists(store: &WatchlistStore) -> Result<&[Watchlist], McpError> {
    if store.load_failed() {
        return Err(McpError::AppUnavailable(
            "Watchlists could not be read, so they are unavailable".into(),
        ));
    }
    Ok(store.lists())
}

pub(super) fn watchlist_by_id(store: &WatchlistStore, id: u64) -> Result<&Watchlist, McpError> {
    readable_watchlists(store)?
        .iter()
        .find(|list| list.id == id)
        .ok_or_else(|| {
            McpError::NotFound(
                "No such watchlist: use a watchlist_id from degenview_list_watchlists".into(),
            )
        })
}

pub fn list_watchlists(store: &WatchlistStore) -> Result<WatchlistsResult, McpError> {
    Ok(WatchlistsResult {
        watchlists: readable_watchlists(store)?
            .iter()
            .map(markets::watchlist_summary)
            .collect(),
    })
}

pub fn get_watchlist(
    store: &WatchlistStore,
    id: u64,
    offset: usize,
    limit: usize,
) -> Result<WatchlistPage, McpError> {
    let list = watchlist_by_id(store, id)?;
    Ok(WatchlistPage {
        detail: markets::watchlist_detail(list, offset, limit, None),
        markets: markets::page_markets(list, offset, limit),
    })
}

fn change_result(
    status: &str,
    store: &WatchlistStore,
    id: u64,
    edit: ListEdit,
) -> WatchlistChangeResult {
    let key = |markets: Vec<InstrumentId>| markets.iter().map(InstrumentId::key).collect();
    let list = store.lists().iter().find(|list| list.id == id);
    WatchlistChangeResult {
        status: status.to_owned(),
        renamed_to: edit
            .renamed
            .then(|| list.map(|list| list.name.clone()))
            .flatten(),
        watchlist: list.map(markets::watchlist_summary),
        added: key(edit.added),
        already_present: key(edit.already_present),
        removed: key(edit.removed),
        not_present: key(edit.not_present),
        moved: key(edit.moved),
        sections_added: edit.sections_added,
        sections_renamed: edit.sections_renamed,
        sections_deleted: edit.sections_deleted,
    }
}

pub fn create_watchlist(
    store: &mut WatchlistStore,
    name: &str,
    markets: Vec<WatchlistInstrument>,
) -> Result<WatchlistChangeResult, McpError> {
    readable_watchlists(store)?;
    let id = store.create_list(name, None).map_err(watchlist_error)?;
    let edit = if markets.is_empty() {
        ListEdit::default()
    } else {
        match store.edit_list(
            id,
            ListChanges {
                add: markets.into_iter().map(|item| (item, Place::End)).collect(),
                ..ListChanges::default()
            },
        ) {
            Ok((edit, _)) => edit,
            Err(error) => {
                // The list was meant to be created with its markets or not at all.
                let _ = store.delete_list(id);
                return Err(watchlist_error(error));
            }
        }
    };
    Ok(change_result("created", store, id, edit))
}

pub fn update_watchlist(
    store: &mut WatchlistStore,
    id: u64,
    changes: ListChanges,
) -> Result<WatchlistChangeResult, McpError> {
    watchlist_by_id(store, id)?;
    let (edit, changed) = store.edit_list(id, changes).map_err(watchlist_error)?;
    Ok(change_result(
        if changed { "updated" } else { "unchanged" },
        store,
        id,
        edit,
    ))
}

pub fn delete_watchlist(
    store: &mut WatchlistStore,
    id: u64,
) -> Result<WatchlistChangeResult, McpError> {
    let summary = markets::watchlist_summary(watchlist_by_id(store, id)?);
    if summary.is_favorites {
        return Err(McpError::Conflict(
            "Favorites cannot be deleted; remove its markets or rename it instead".into(),
        ));
    }
    if !store.delete_list(id).map_err(watchlist_error)? {
        return Err(McpError::Conflict(
            "That watchlist cannot be deleted".into(),
        ));
    }
    Ok(WatchlistChangeResult {
        status: "deleted".into(),
        watchlist: None,
        added: Vec::new(),
        already_present: Vec::new(),
        removed: Vec::new(),
        not_present: Vec::new(),
        renamed_to: None,
        moved: Vec::new(),
        sections_added: Vec::new(),
        sections_renamed: Vec::new(),
        sections_deleted: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::state::McpStatus;
    use crate::store::Store;
    use std::sync::Mutex;

    fn job(
        started: Instant,
    ) -> (
        SnapshotJob,
        std::sync::mpsc::Sender<Result<Vec<u8>, String>>,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            SnapshotJob {
                receiver,
                width: 4,
                height: 2,
                started,
            },
            sender,
        )
    }

    #[test]
    fn a_picture_job_is_pending_then_collected_once() {
        let mut jobs = SnapshotJobs::default();
        let (job, sender) = job(Instant::now());
        jobs.jobs.insert(1, job);
        assert_eq!(jobs.poll(1), Ok(SnapshotProgress::Pending));
        sender.send(Ok(vec![1, 2, 3])).unwrap();
        assert_eq!(
            jobs.poll(1),
            Ok(SnapshotProgress::Ready {
                png: Arc::new(vec![1, 2, 3]),
                width: 4,
                height: 2
            })
        );
        assert!(
            matches!(jobs.poll(1), Err(McpError::NotFound(_))),
            "collected once"
        );
    }

    #[test]
    fn a_failed_or_abandoned_picture_job_reports_and_is_forgotten() {
        let mut jobs = SnapshotJobs::default();
        let (failing, sender) = job(Instant::now());
        jobs.jobs.insert(1, failing);
        sender
            .send(Err("Could not draw the image.".into()))
            .unwrap();
        assert_eq!(
            jobs.poll(1),
            Err(McpError::AppUnavailable("Could not draw the image.".into()))
        );
        let (abandoned, sender) = job(Instant::now());
        jobs.jobs.insert(2, abandoned);
        drop(sender);
        assert!(matches!(jobs.poll(2), Err(McpError::AppUnavailable(_))));
        assert!(jobs.jobs.is_empty());
    }

    #[test]
    fn a_picture_over_the_reply_budget_is_refused_and_forgotten() {
        let mut jobs = SnapshotJobs::default();
        let (big, sender) = job(Instant::now());
        jobs.jobs.insert(1, big);
        sender
            .send(Ok(vec![0; crate::model::snapshot::MAX_MCP_PNG_BYTES + 1]))
            .unwrap();
        assert!(matches!(jobs.poll(1), Err(McpError::ResourceLimit(_))));
        assert!(jobs.jobs.is_empty(), "the oversized PNG is not kept");
        let (fits, sender) = job(Instant::now());
        jobs.jobs.insert(2, fits);
        sender
            .send(Ok(vec![0; crate::model::snapshot::MAX_MCP_PNG_BYTES]))
            .unwrap();
        assert!(matches!(jobs.poll(2), Ok(SnapshotProgress::Ready { .. })));
    }

    #[test]
    fn expiring_reaps_an_uncollected_picture_without_a_poll() {
        let mut jobs = SnapshotJobs::default();
        let long_ago = Instant::now()
            .checked_sub(Duration::from_secs(300))
            .expect("the clock has run for five minutes");
        let (stale, _sender) = job(long_ago);
        let (fresh, _fresh_sender) = job(Instant::now());
        jobs.jobs.insert(1, stale);
        jobs.jobs.insert(2, fresh);
        jobs.expire();
        assert_eq!(jobs.jobs.len(), 1);
        assert!(jobs.jobs.contains_key(&2));
    }

    #[test]
    fn a_picture_nobody_collects_expires() {
        let mut jobs = SnapshotJobs::default();
        let long_ago = Instant::now()
            .checked_sub(Duration::from_secs(300))
            .expect("the clock has run for five minutes");
        let (stale, _sender) = job(long_ago);
        jobs.jobs.insert(1, stale);
        assert!(matches!(jobs.poll(1), Err(McpError::NotFound(_))));
        assert!(jobs.jobs.is_empty());
    }

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
            peer: crate::model::mcp::Peer::from_ip(std::net::IpAddr::V4(
                std::net::Ipv4Addr::LOCALHOST,
            )),
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

        std::fs::remove_file(dir.join(crate::mcp::tls::FILE_NAME)).unwrap();
        // The stale value is still what the page has until something loads an identity.
        assert_eq!(host.tls_fingerprint(), Some(before.as_str()));
        // `start_server` is what loads it for the listener (the address cannot be bound here, so
        // it fails after loading, which is exactly the part under test).
        host.settings.enabled = true;
        host.start_server();
        let shown = host.tls_fingerprint().expect("a fingerprint").to_owned();
        let served = crate::mcp::tls::load_or_create(&dir, "192.0.2.1".parse().unwrap())
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
        std::fs::write(dir.join(crate::mcp::tls::FILE_NAME), "damaged").unwrap();
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
        assert!(dir.join(crate::mcp::tls::FILE_NAME).exists());
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

    // MARK: Price alerts against the real alert host

    use crate::model::alert::{
        AlertAsset, AlertPrecondition, AlertRuntimeCommand, AlertRuntimeCommandPayload,
    };
    use crate::model::watchlist_quote::{Freshness, WatchlistQuote};
    use std::collections::HashMap;

    const NOW: i64 = 1_760_000_000_000;

    fn alert_host() -> (Store, AlertHost) {
        let store = Store::open_in_memory().unwrap();
        let host = AlertHost::load(&store, NOW);
        (store, host)
    }

    fn command(store: &Store, host: &mut AlertHost, payload: AlertRuntimeCommandPayload) {
        store
            .enqueue_alert_command(&AlertRuntimeCommand::new(payload, NOW))
            .unwrap();
        host.drain_commands(store, NOW);
    }

    fn request(crosses_above: bool, target: &str) -> NewAlert {
        NewAlert {
            instrument: InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            crosses_above,
            target_text: target.to_owned(),
            currency: AlertCurrency::Usd,
            frequency: AlertFrequency::Once,
            note: String::new(),
        }
    }

    fn book_with_price(last: f64) -> WatchlistQuoteBook {
        let book = WatchlistQuoteBook::default();
        book.apply(HashMap::from([(
            InstrumentId::new(DataSource::Binance, "BTCUSDT").key(),
            WatchlistQuote {
                last: Some(last),
                freshness: Freshness::Live,
                ..WatchlistQuote::default()
            },
        )]));
        book
    }

    /// Creates through the same two steps the app takes: plan, then queue the Save.
    fn create(
        store: &Store,
        host: &mut AlertHost,
        book: &WatchlistQuoteBook,
        request: &NewAlert,
    ) -> Result<CreatePlan, McpError> {
        let plan = plan_create_alert(host, book, &HashMap::new(), request, NOW)?;
        if let CreatePlan::New { alert, baseline } = &plan {
            command(
                store,
                host,
                AlertRuntimeCommandPayload::Save {
                    alert: alert.clone(),
                    baseline: *baseline,
                },
            );
        }
        Ok(plan)
    }

    #[test]
    fn a_created_alert_is_built_by_the_sheets_rules_and_armed_from_the_live_price() {
        let (store, mut host) = alert_host();
        // Price below the target: the next cross upward counts.
        let below = book_with_price(65_000.0);
        let plan = create(&store, &mut host, &below, &request(true, "70000")).unwrap();
        let CreatePlan::New { alert, baseline } = plan else {
            panic!("expected a new alert");
        };
        assert_eq!(baseline, Some(65_000.0));
        assert_eq!(
            alert.condition,
            AlertCondition::CrossesAbove { target: 70_000.0 }
        );
        assert_eq!(alert.state, AlertState::Active);
        assert_eq!(alert.currency, AlertCurrency::Usd);
        assert_eq!(alert.frequency, AlertFrequency::Once);
        assert_eq!(alert.asset.key(), "BINANCE:BTCUSDT");
        assert!(alert.webhook_endpoint_ids.is_empty());
        assert!(host.snapshot().alerts[0].armed);

        // Price already above the target: it must first fall back and cross again.
        let above = book_with_price(80_000.0);
        create(&store, &mut host, &above, &request(true, "75000")).unwrap();
        let second = host
            .snapshot()
            .alerts
            .iter()
            .find(|a| a.condition.target() == Some(75_000.0))
            .unwrap();
        assert!(!second.armed);

        // No live price: still created, and armed by default.
        let none = WatchlistQuoteBook::default();
        let plan = create(&store, &mut host, &none, &request(false, "50000")).unwrap();
        assert!(matches!(plan, CreatePlan::New { baseline: None, .. }));
    }

    #[test]
    fn listing_reports_the_alert_and_never_exposes_webhooks() {
        let (store, mut host) = alert_host();
        let mut alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            AlertCondition::CrossesBelow { target: 0.00000278 },
            NOW,
        );
        alert.note = "dip".into();
        alert.webhook_endpoint_ids = vec![424_242];
        let id = alert.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(alert),
                baseline: None,
            },
        );
        let listed = list_alerts(&host, None);
        assert_eq!((listed.total, listed.truncated), (1, false));
        let info = &listed.alerts[0];
        assert_eq!(info.alert_id, id.to_string());
        assert_eq!(info.condition, "crosses_below");
        assert_eq!(info.target_price.as_deref(), Some("0.00000278"));
        assert_eq!(
            (info.status.as_str(), info.frequency.as_str()),
            ("active", "once")
        );
        assert_eq!(info.note, "dip");
        assert!(info.sends_to_webhooks);
        let json = serde_json::to_string(&listed).unwrap();
        assert!(
            !json.contains("424242") && !json.contains("webhook_endpoint"),
            "{json}"
        );
        assert!(info.created_at.ends_with('Z'));

        assert_eq!(list_alerts(&host, Some(id)).alerts.len(), 1);
        assert!(list_alerts(&host, Some(id + 1)).alerts.is_empty());
    }

    #[test]
    fn an_identical_alert_is_returned_instead_of_a_duplicate() {
        let (store, mut host) = alert_host();
        let book = WatchlistQuoteBook::default();
        create(&store, &mut host, &book, &request(true, "70000")).unwrap();
        let id = host.snapshot().alerts[0].id.to_string();

        let again = create(&store, &mut host, &book, &request(true, "70000")).unwrap();
        assert!(matches!(&again, CreatePlan::Existing(info) if info.alert_id == id));
        assert_eq!(host.snapshot().alerts.len(), 1);

        // A different level, direction, currency or frequency is a different alert.
        for different in [
            request(true, "71000"),
            request(false, "70000"),
            NewAlert {
                currency: AlertCurrency::Eur,
                ..request(true, "70000")
            },
            NewAlert {
                frequency: AlertFrequency::EveryTime,
                ..request(true, "70000")
            },
        ] {
            let plan = create(&store, &mut host, &book, &different).unwrap();
            assert!(matches!(plan, CreatePlan::New { .. }));
        }
        assert_eq!(host.snapshot().alerts.len(), 5);
    }

    #[test]
    fn an_unparseable_target_is_refused() {
        let (_store, host) = alert_host();
        let book = WatchlistQuoteBook::default();
        for bad in ["", "0", "-1", "abc"] {
            let result = plan_create_alert(&host, &book, &HashMap::new(), &request(true, bad), NOW);
            assert!(
                matches!(result, Err(McpError::InvalidArgument(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_alert_count_is_capped() {
        let (store, mut host) = alert_host();
        for index in 0..MAX_ALERTS {
            let alert = PriceAlert::new(
                AlertAsset::new(
                    InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                    "BTC",
                    "Bitcoin",
                ),
                AlertCondition::CrossesAbove {
                    target: 1.0 + index as f64,
                },
                NOW,
            );
            store
                .enqueue_alert_command(&AlertRuntimeCommand::new(
                    AlertRuntimeCommandPayload::Save {
                        alert: Box::new(alert),
                        baseline: None,
                    },
                    NOW,
                ))
                .unwrap();
        }
        host.drain_commands(&store, NOW);
        assert_eq!(host.snapshot().alerts.len(), MAX_ALERTS);
        let result = plan_create_alert(
            &host,
            &WatchlistQuoteBook::default(),
            &HashMap::new(),
            &request(true, "999999"),
            NOW,
        );
        assert!(matches!(result, Err(McpError::ResourceLimit(_))));
        assert_eq!(list_alerts(&host, None).alerts.len(), ALERT_PAGE);
        assert!(list_alerts(&host, None).truncated);
    }

    #[test]
    fn pause_resume_and_delete_follow_the_alert_windows_semantics() {
        let (store, mut host) = alert_host();
        create(
            &store,
            &mut host,
            &WatchlistQuoteBook::default(),
            &request(true, "70000"),
        )
        .unwrap();
        let id = host.snapshot().alerts[0].id;

        // Active: pausing changes it, resuming is a no-op.
        assert!(matches!(
            plan_pause_alert(&host, id, false),
            Ok(StatePlan::Already)
        ));
        let Ok(StatePlan::Change {
            id: change_id,
            state,
        }) = plan_pause_alert(&host, id, true)
        else {
            panic!("expected a change");
        };
        assert_eq!((change_id, state), (id, AlertState::Paused));
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::SetState {
                id,
                state,
                baseline: None,
            },
        );
        assert_eq!(list_alerts(&host, Some(id)).alerts[0].status, "paused");
        assert!(matches!(
            plan_pause_alert(&host, id, true),
            Ok(StatePlan::Already)
        ));

        // A fired once-alert is resumed (re-armed) like the Toggle button does.
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::SetState {
                id,
                state: AlertState::Triggered,
                baseline: None,
            },
        );
        assert!(matches!(
            plan_pause_alert(&host, id, false),
            Ok(StatePlan::Change {
                state: AlertState::Active,
                ..
            })
        ));

        // Delete.
        assert_eq!(plan_delete_alert(&host, id).unwrap(), id);
        command(&store, &mut host, AlertRuntimeCommandPayload::Delete(id));
        assert!(list_alerts(&host, None).alerts.is_empty());
        assert!(matches!(
            plan_delete_alert(&host, id),
            Err(McpError::NotFound(_))
        ));
        assert!(matches!(
            plan_pause_alert(&host, id, true),
            Err(McpError::NotFound(_))
        ));
    }

    #[test]
    fn an_alert_with_a_condition_from_a_newer_version_cannot_be_toggled() {
        let (store, mut host) = alert_host();
        let alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            AlertCondition::Unsupported {
                tag: "touchesVwap".into(),
            },
            NOW,
        );
        let id = alert.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(alert),
                baseline: None,
            },
        );
        assert!(matches!(
            plan_pause_alert(&host, id, true),
            Err(McpError::Unsupported(_))
        ));
        let info = &list_alerts(&host, Some(id)).alerts[0];
        assert_eq!(
            (info.condition.as_str(), info.status.as_str()),
            ("unsupported", "unsupported")
        );
        assert_eq!(info.target_price, None);
    }

    #[test]
    fn decimals_drop_float_noise_but_keep_real_digits() {
        assert_eq!(decimal(70_000.0), "70000");
        assert_eq!(decimal(110.000_000_000_000_01), "110");
        assert_eq!(decimal(0.000_002_78), "0.00000278");
        assert_eq!(decimal(61_234.567_89), "61234.56789");
        assert_eq!(decimal(2.5), "2.5");
        assert_eq!(decimal(1e-8), "0.00000001");
    }

    #[test]
    fn percent_conditions_report_their_reference() {
        let alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            AlertCondition::rises_by(10.0, 100.0),
            NOW,
        );
        let info = alert_info(&alert, Some(NOW));
        assert_eq!(info.condition, "rises_by");
        assert_eq!(info.percent.as_deref(), Some("10"));
        assert_eq!(info.reference_price.as_deref(), Some("100"));
        assert_eq!(info.target_price.as_deref(), Some("110"));
        assert!(info.last_triggered_at.is_some());
    }

    fn update(id: AlertId) -> AlertUpdate {
        AlertUpdate {
            alert_id: id,
            crosses_above: None,
            target_text: None,
            currency: None,
            frequency: None,
            note: None,
        }
    }

    fn apply_update(
        store: &Store,
        host: &mut AlertHost,
        book: &WatchlistQuoteBook,
        request: &AlertUpdate,
    ) -> Result<UpdatePlan, McpError> {
        let plan = plan_update_alert(host, book, &HashMap::new(), request, NOW)?;
        if let UpdatePlan::Change {
            alert, baseline, ..
        } = &plan
        {
            command(
                store,
                host,
                AlertRuntimeCommandPayload::Save {
                    alert: alert.clone(),
                    baseline: *baseline,
                },
            );
        }
        Ok(plan)
    }

    fn seeded() -> (Store, AlertHost, AlertId) {
        let (store, mut host) = alert_host();
        create(
            &store,
            &mut host,
            &WatchlistQuoteBook::default(),
            &request(true, "70000"),
        )
        .unwrap();
        let id = host.snapshot().alerts[0].id;
        (store, host, id)
    }

    #[test]
    fn editing_changes_only_the_given_settings_and_keeps_the_alert() {
        let (store, mut host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        let created_at = host.snapshot().alerts[0].created_at_ms;

        let mut edit = update(id);
        edit.target_text = Some("72500.5".into());
        edit.note = Some("breakout".into());
        assert!(matches!(
            apply_update(&store, &mut host, &book, &edit),
            Ok(UpdatePlan::Change { .. })
        ));
        let alert = &host.snapshot().alerts[0];
        assert_eq!(host.snapshot().alerts.len(), 1);
        assert_eq!(alert.id, id);
        assert_eq!(alert.created_at_ms, created_at);
        assert_eq!(
            alert.condition,
            AlertCondition::CrossesAbove { target: 72_500.5 }
        );
        assert_eq!(alert.note, "breakout");
        assert_eq!(alert.frequency, AlertFrequency::Once);

        let mut edit = update(id);
        edit.crosses_above = Some(false);
        edit.currency = Some(AlertCurrency::Eur);
        edit.frequency = Some(AlertFrequency::EveryTime);
        apply_update(&store, &mut host, &book, &edit).unwrap();
        let alert = &host.snapshot().alerts[0];
        assert_eq!(
            alert.condition,
            AlertCondition::CrossesBelow { target: 72_500.5 }
        );
        assert_eq!(
            (alert.currency, alert.frequency),
            (AlertCurrency::Eur, AlertFrequency::EveryTime)
        );
        assert_eq!(alert.note, "breakout");
    }

    #[test]
    fn an_edit_that_changes_nothing_queues_nothing() {
        let (_store, host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        let mut same = update(id);
        same.target_text = Some("70000".into());
        same.crosses_above = Some(true);
        same.frequency = Some(AlertFrequency::Once);
        same.note = Some(String::new());
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &same, NOW),
            Ok(UpdatePlan::Unchanged(_))
        ));
    }

    #[test]
    fn an_edit_is_rearmed_from_the_live_price_like_the_sheet() {
        let (store, mut host, id) = seeded();
        // Move the target below a price of 80,000: it must fall back and cross up again.
        let mut edit = update(id);
        edit.target_text = Some("75000".into());
        apply_update(&store, &mut host, &book_with_price(80_000.0), &edit).unwrap();
        assert!(!host.snapshot().alerts[0].armed);
        let mut edit = update(id);
        edit.target_text = Some("90000".into());
        apply_update(&store, &mut host, &book_with_price(80_000.0), &edit).unwrap();
        assert!(host.snapshot().alerts[0].armed);
    }

    #[test]
    fn a_remote_edit_never_resumes_a_paused_alert() {
        let (store, mut host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::SetState {
                id,
                state: AlertState::Paused,
                baseline: None,
            },
        );
        let mut edit = update(id);
        edit.target_text = Some("71000".into());
        apply_update(&store, &mut host, &book, &edit).unwrap();
        assert_eq!(host.snapshot().alerts[0].state, AlertState::Paused);
        assert_eq!(host.snapshot().alerts[0].condition.target(), Some(71_000.0));
    }

    #[test]
    fn a_fired_alert_is_rearmed_only_by_a_real_change() {
        let (store, mut host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::SetState {
                id,
                state: AlertState::Triggered,
                baseline: None,
            },
        );
        // A note alone leaves it fired.
        let mut edit = update(id);
        edit.note = Some("seen".into());
        apply_update(&store, &mut host, &book, &edit).unwrap();
        assert_eq!(host.snapshot().alerts[0].state, AlertState::Triggered);
        // A new level makes it live again.
        let mut edit = update(id);
        edit.target_text = Some("99000".into());
        apply_update(&store, &mut host, &book, &edit).unwrap();
        assert_eq!(host.snapshot().alerts[0].state, AlertState::Active);
    }

    #[test]
    fn an_edit_cannot_duplicate_another_alert() {
        let (store, mut host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        create(&store, &mut host, &book, &request(true, "80000")).unwrap();
        let mut edit = update(id);
        edit.target_text = Some("80000".into());
        let result = plan_update_alert(&host, &book, &HashMap::new(), &edit, NOW);
        assert!(matches!(result, Err(McpError::Conflict(_))));
        assert_eq!(
            host.snapshot()
                .alerts
                .iter()
                .filter(|a| a.condition.target() == Some(80_000.0))
                .count(),
            1
        );
    }

    #[test]
    fn edits_keep_webhooks_and_refuse_what_cannot_be_edited() {
        let (store, mut host) = alert_host();
        let book = WatchlistQuoteBook::default();
        let asset = AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        );

        let mut hooked = PriceAlert::new(
            asset.clone(),
            AlertCondition::CrossesAbove { target: 70_000.0 },
            NOW,
        );
        hooked.webhook_endpoint_ids = vec![7];
        hooked.webhook_message = Some("{{price}}".into());
        let hooked_id = hooked.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(hooked),
                baseline: None,
            },
        );
        let mut edit = update(hooked_id);
        edit.target_text = Some("71000".into());
        apply_update(&store, &mut host, &book, &edit).unwrap();
        let alert = host
            .snapshot()
            .alerts
            .iter()
            .find(|a| a.id == hooked_id)
            .unwrap();
        assert_eq!(alert.webhook_endpoint_ids, vec![7]);
        assert_eq!(alert.webhook_message.as_deref(), Some("{{price}}"));

        // A percentage alert takes a note and frequency, not a new level.
        let percent = PriceAlert::new(asset.clone(), AlertCondition::rises_by(10.0, 100.0), NOW);
        let percent_id = percent.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(percent),
                baseline: None,
            },
        );
        let mut level = update(percent_id);
        level.target_text = Some("150".into());
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &level, NOW),
            Err(McpError::Unsupported(_))
        ));
        let mut note = update(percent_id);
        note.note = Some("watch".into());
        note.frequency = Some(AlertFrequency::EveryTime);
        apply_update(&store, &mut host, &book, &note).unwrap();
        let alert = host
            .snapshot()
            .alerts
            .iter()
            .find(|a| a.id == percent_id)
            .unwrap();
        assert!(
            matches!(alert.condition, AlertCondition::RisesBy { percent, reference, .. } if percent == 10.0 && reference == 100.0)
        );
        assert_eq!(
            (alert.note.as_str(), alert.frequency),
            ("watch", AlertFrequency::EveryTime)
        );

        // A condition from a newer version, and an alert that does not exist.
        let future = PriceAlert::new(asset, AlertCondition::Unsupported { tag: "x".into() }, NOW);
        let future_id = future.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(future),
                baseline: None,
            },
        );
        let mut edit = update(future_id);
        edit.note = Some("x".into());
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &edit, NOW),
            Err(McpError::Unsupported(_))
        ));
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &update(1), NOW),
            Err(McpError::NotFound(_))
        ));
        let mut bad = update(hooked_id);
        bad.target_text = Some("0".into());
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &bad, NOW),
            Err(McpError::InvalidArgument(_))
        ));
    }

    // MARK: Pine scripts applied to charts

    use crate::model::pine_alert::{PineAlertState, PineAlertSubscription};
    use crate::model::pine_instance::{PineInstance, StoredInput};
    use crate::pine::strategy::{OpenTrade, StrategyReport, StrategySettings, Trade};
    use crate::pine::style_overrides::{StyleKind, StyleRow};
    use crate::pine::token::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, SourceRange};
    use crate::workspace::PineRunView;

    fn empty_workspace(id: u64) -> ChartWorkspace {
        use crate::service::watchlist_quotes::WatchlistQuoteCoordinator;
        use crate::store::watchlist::WatchlistStore;
        let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let book = WatchlistQuoteBook::default();
        ChartWorkspace::empty(
            id,
            crate::model::Timeframe::Day,
            Arc::new(Mutex::new(WatchlistStore::new(store.clone()))),
            book.clone(),
            WatchlistQuoteCoordinator::new(book, Arc::new(|| {})),
            store,
        )
    }

    const STRATEGY_SOURCE: &str = "//@version=6\nstrategy(\"S\")\n";

    fn trade(id: usize, profit: f64) -> Trade {
        Trade {
            id,
            entry_id: format!("e{id}"),
            exit_id: format!("x{id}"),
            is_long: id.is_multiple_of(2),
            quantity: 1.0,
            entry_bar: id * 2,
            entry_time: NOW + id as i64 * 1000,
            entry_price: 100.0,
            exit_bar: id * 2 + 1,
            exit_time: NOW + id as i64 * 1000 + 500,
            exit_price: 100.0 + profit,
            commission: 0.0,
            profit,
            ..Trade::default()
        }
    }

    fn report(trades: Vec<Trade>, equity: Vec<f64>) -> StrategyReport {
        StrategyReport {
            settings: StrategySettings::default(),
            trades,
            open_trades: vec![OpenTrade {
                entry_id: "open".into(),
                is_long: true,
                quantity: 2.0,
                entry_bar: 9,
                entry_time: NOW,
                entry_price: 101.0,
                entry_commission: 0.0,
                ..OpenTrade::default()
            }],
            equity,
            open_profit: 3.5,
            chart: crate::model::ChartKind::Standard,
        }
    }

    fn view(run: Option<PineRunView>) -> PineInstanceView {
        let mut instance = PineInstance::new(4, Some("script-1".into()), "Strat", STRATEGY_SOURCE);
        instance.inputs.insert("len".into(), StoredInput::Int(30));
        instance
            .style_overrides
            .insert("plot.0.color".into(), "FF0000FF".into());
        instance
            .style_overrides
            .insert("plot.0.visible".into(), "false".into());
        instance
            .style_overrides
            .insert("plot.0.width".into(), "3".into());
        PineInstanceView {
            chart_id: 2,
            instrument: InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            timeframe: crate::model::Timeframe::Day,
            instance,
            run,
        }
    }

    fn run(strategy: Option<StrategyReport>, error: Option<Diagnostic>) -> PineRunView {
        PineRunView {
            error,
            bar_count: 500,
            style_rows: vec![StyleRow {
                kind: StyleKind::Plot,
                output_id: 0,
                title: "SMA".into(),
                default_color: Some(0x2962_FFFF),
                default_width: Some(2),
            }],
            is_strategy: strategy.is_some(),
            strategy,
        }
    }

    #[test]
    fn applying_changing_and_removing_a_script_report_what_happened() {
        const SOURCE: &str =
            "//@version=6\nindicator(\"I\")\nlen = input.int(20, \"L\", minval=1)\nplot(close)\n";
        let ctx = eframe::egui::Context::default();
        let mut workspace = empty_workspace(3);
        let chart_id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        let script = ScriptToApply {
            script_id: "s1".into(),
            name: "I".into(),
            source: SOURCE.into(),
            inputs: Default::default(),
        };
        let args = |json: serde_json::Value| -> InputArgs { serde_json::from_value(json).unwrap() };

        assert!(matches!(
            add_pine_instance(&mut workspace, 99, &script, true),
            Err(McpError::NotFound(_))
        ));
        let added = add_pine_instance(&mut workspace, chart_id, &script, true).unwrap();
        assert_eq!(added.status, "added");
        assert_eq!(added.chart.instances.len(), 1);
        assert_eq!(added.chart.instances[0].instance_id, added.instance_id);
        assert_eq!(added.chart.instances[0].run_status, "not_run");
        let id: u64 = added.instance_id.parse().unwrap();

        let set = args(serde_json::json!({"len": 5}));
        let changed =
            update_pine_instance(&mut workspace, chart_id, id, Some(&set), None, &[]).unwrap();
        assert_eq!(changed.status, "updated");
        let again =
            update_pine_instance(&mut workspace, chart_id, id, Some(&set), None, &[]).unwrap();
        assert_eq!(again.status, "unchanged");
        let hidden =
            update_pine_instance(&mut workspace, chart_id, id, None, Some(false), &[]).unwrap();
        assert!(!hidden.chart.instances[0].visible);
        assert!(matches!(
            update_pine_instance(
                &mut workspace,
                chart_id,
                id,
                Some(&args(serde_json::json!({"nope": 1}))),
                None,
                &[]
            ),
            Err(McpError::InvalidArgument(_))
        ));
        assert!(matches!(
            update_pine_instance(&mut workspace, chart_id, id + 1, None, Some(true), &[]),
            Err(McpError::NotFound(_))
        ));

        let removed = remove_pine_instance(&mut workspace, chart_id, id, &[]).unwrap();
        assert_eq!(removed.status, "removed");
        assert!(removed.chart.instances.is_empty());
        assert!(matches!(
            remove_pine_instance(&mut workspace, chart_id, id, &[]),
            Err(McpError::NotFound(_))
        ));
    }

    #[test]
    fn a_report_carries_the_charts_own_numbers_and_the_context_that_made_them() {
        let trades = vec![
            trade(0, 10.0),
            trade(1, -4.0),
            trade(2, 6.0),
            trade(3, -2.0),
        ];
        let strategy = report(trades, vec![1000.0, 1010.0, 1006.0, 1012.0, 1010.0]);
        let expected_net = strategy.net_profit();
        let (drawdown, fraction) = strategy.max_drawdown();
        let result = report_of(1, &view(Some(run(Some(strategy), None))), 0, 50, true);

        assert!(result.available && result.reason.is_none());
        let metrics = result.metrics.unwrap();
        assert_eq!(metrics.net_profit, Some(expected_net));
        assert_eq!(metrics.net_profit, Some(10.0));
        assert_eq!(
            (
                metrics.closed_trades,
                metrics.winning_trades,
                metrics.losing_trades
            ),
            (4, 2, 2)
        );
        assert_eq!(metrics.win_rate, Some(0.5));
        assert_eq!(metrics.max_drawdown, Some(drawdown));
        assert_eq!(metrics.max_drawdown_fraction, Some(fraction));
        assert_eq!((metrics.open_profit, metrics.open_trades), (Some(3.5), 1));

        let context = result.context.unwrap();
        assert_eq!(context.kind, "chart_instance");
        assert_eq!(context.instrument, "BINANCE:BTCUSDT");
        assert_eq!(context.timeframe, "1D");
        assert_eq!(context.bar_count, 500);
        assert_eq!(context.script_id.as_deref(), Some("script-1"));
        assert_eq!(
            context.source_hash,
            crate::pine_studio::store::hash_source(STRATEGY_SOURCE)
        );
        assert_eq!(
            context.inputs["len"],
            serde_json::json!({"kind": "int", "value": 30})
        );
        assert_eq!(
            result.equity_curve,
            vec![1000.0, 1010.0, 1006.0, 1012.0, 1010.0]
        );
        assert_eq!(
            result.settings.unwrap().initial_capital,
            StrategySettings::default().initial_capital
        );
        assert_eq!(result.open_trades[0].direction, "long");
        assert!(result.trades[0].entry_time.ends_with('Z'));
    }

    #[test]
    fn trades_are_paged_and_equity_is_thinned_but_keeps_its_ends() {
        let trades: Vec<Trade> = (0..450).map(|id| trade(id, 1.0)).collect();
        let equity: Vec<f64> = (0..1000).map(f64::from).collect();
        let view = view(Some(run(Some(report(trades, equity)), None)));

        let first = report_of(1, &view, 0, 200, false);
        assert_eq!(
            (first.trades.len(), first.trades_total, first.trades_offset),
            (200, 450, 0)
        );
        assert_eq!(first.trades[0].id, 0);
        assert!(first.equity_curve.is_empty(), "equity is opt-in");
        let last = report_of(1, &view, 400, 200, false);
        assert_eq!((last.trades.len(), last.trades[0].id), (50, 400));
        let beyond = report_of(1, &view, 10_000, 10, false);
        assert!(beyond.trades.is_empty() && beyond.trades_total == 450);
        // A limit over the cap, or zero, is clamped, never honoured.
        assert_eq!(
            report_of(1, &view, 0, 10_000, false).trades.len(),
            MAX_TRADES_PAGE
        );
        assert_eq!(report_of(1, &view, 0, 0, false).trades.len(), 1);

        let curve = report_of(1, &view, 0, 1, true);
        assert_eq!(curve.equity_curve.len(), MAX_EQUITY_POINTS);
        assert_eq!(curve.equity_points_total, 1000);
        assert_eq!(curve.equity_curve.first(), Some(&0.0));
        assert_eq!(curve.equity_curve.last(), Some(&999.0));
        assert!(curve.equity_curve.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn no_report_is_a_typed_unavailable_result_never_a_stale_one() {
        let never_run = report_of(1, &view(None), 0, 50, true);
        assert!(!never_run.available);
        assert!(never_run.reason.as_deref().unwrap().contains("not run"));
        assert!(
            never_run.context.is_none()
                && never_run.metrics.is_none()
                && never_run.trades.is_empty()
        );

        let indicator = report_of(1, &view(Some(run(None, None))), 0, 50, false);
        assert!(!indicator.available);
        assert!(
            indicator
                .reason
                .as_deref()
                .unwrap()
                .contains("not a strategy")
        );

        let failed = Diagnostic::error(
            "PINE8004",
            DiagnosticCategory::Runtime,
            "boom",
            SourceRange::ZERO,
        );
        let broken = report_of(1, &view(Some(run(None, Some(failed)))), 0, 50, false);
        assert!(!broken.available);
        assert!(broken.reason.as_deref().unwrap().contains("failed"));
    }

    #[test]
    fn non_finite_numbers_never_reach_the_json() {
        let mut strategy = report(vec![trade(0, 5.0)], vec![1.0]);
        strategy.open_profit = f64::NAN;
        let result = report_of(1, &view(Some(run(Some(strategy), None))), 0, 50, false);
        let metrics = result.metrics.unwrap();
        assert_eq!(metrics.open_profit, None);
        assert_eq!(
            metrics.profit_factor, None,
            "no losing trade: no finite factor"
        );
        assert!(serde_json::to_string(&metrics).unwrap().contains("null"));
    }

    #[test]
    fn style_rows_show_defaults_and_this_instances_overrides() {
        let styles = styles_of(1, &view(Some(run(None, None))));
        assert!(styles.available && styles.reason.is_none());
        assert_eq!(styles.rows.len(), 1);
        let row = &styles.rows[0];
        assert_eq!(
            (row.id.as_str(), row.kind.as_str(), row.title.as_str()),
            ("plot.0", "plot", "SMA")
        );
        assert_eq!(row.default_color.as_deref(), Some("#2962FFFF"));
        assert_eq!(row.default_width, Some(2));
        assert_eq!(row.overrides.visible, Some(false));
        assert_eq!(row.overrides.color.as_deref(), Some("FF0000FF"));
        assert_eq!(row.overrides.width, Some(3));

        let unrun = styles_of(1, &view(None));
        assert!(!unrun.available && unrun.rows.is_empty());
        assert!(unrun.reason.is_some());
    }

    #[test]
    fn applied_instances_report_run_status_and_the_hash_of_their_own_source() {
        let info = applied_info(&view(None));
        assert_eq!(
            (info.instance_id.as_str(), info.run_status.as_str()),
            ("4", "not_run")
        );
        assert_eq!(info.is_strategy, None);
        assert_eq!(info.script_id.as_deref(), Some("script-1"));
        assert_eq!(
            info.source_hash,
            crate::pine_studio::store::hash_source(STRATEGY_SOURCE)
        );

        assert_eq!(applied_info(&view(Some(run(None, None)))).run_status, "ok");
        let failed = Diagnostic::error(
            "PINE8004",
            DiagnosticCategory::Runtime,
            "boom",
            SourceRange::ZERO,
        );
        let broken = applied_info(&view(Some(run(None, Some(failed)))));
        assert_eq!(broken.run_status, "error");
        assert_eq!(broken.error.unwrap().code, "PINE8004");
        assert!(matches!(
            applied_info(&view(Some(run(Some(report(vec![], vec![])), None)))).is_strategy,
            Some(true)
        ));

        let detail = detail_of(1, &view(None));
        assert_eq!(
            detail.inputs["len"],
            serde_json::json!({"kind": "int", "value": 30})
        );
        assert_eq!(detail.style_overrides["plot.0.visible"], "false");
        assert_eq!(detail.bar_count, None);
        let _ = DiagnosticSeverity::Error;
    }

    #[test]
    fn a_workspace_lists_its_applied_scripts_and_says_when_they_have_not_run() {
        let ctx = eframe::egui::Context::default();
        let mut workspace = empty_workspace(11);
        let chart = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        workspace.add_pine_on_focused(Some("s1".into()), "One", STRATEGY_SOURCE);
        workspace.add_pine_on_focused(Some("s1".into()), "One", STRATEGY_SOURCE);

        let list = list_chart_pine(&workspace, chart).unwrap();
        assert_eq!(
            (
                list.workspace_id.as_str(),
                list.instrument.as_str(),
                list.timeframe.as_str()
            ),
            ("11", "BINANCE:BTCUSDT", "1D")
        );
        assert_eq!(
            list.instances.len(),
            2,
            "the same script can be applied twice"
        );
        assert_ne!(list.instances[0].instance_id, list.instances[1].instance_id);
        assert!(list.instances.iter().all(|i| i.run_status == "not_run"));

        let second: u64 = list.instances[1].instance_id.parse().unwrap();
        assert_eq!(
            chart_pine_detail(&workspace, chart, second)
                .unwrap()
                .instance
                .script_name,
            "One"
        );
        let report = pine_report(&workspace, chart, second, 0, 50, false).unwrap();
        assert!(!report.available);
        assert!(!pine_styles(&workspace, chart, second).unwrap().available);

        assert!(matches!(
            list_chart_pine(&workspace, 999),
            Err(McpError::NotFound(_))
        ));
        assert!(matches!(
            chart_pine_detail(&workspace, chart, 999),
            Err(McpError::NotFound(_))
        ));
        assert!(matches!(
            pine_report(&workspace, chart, 999, 0, 1, false),
            Err(McpError::NotFound(_))
        ));
    }

    #[test]
    fn a_script_with_an_armed_alert_keeps_its_inputs_and_stays_on_the_chart() {
        const SOURCE: &str =
            "//@version=6\nindicator(\"I\")\nlen = input.int(20, \"L\", minval=1)\nplot(close)\n";
        let ctx = eframe::egui::Context::default();
        let mut workspace = empty_workspace(3);
        let chart_id = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        let script = ScriptToApply {
            script_id: "s1".into(),
            name: "I".into(),
            source: SOURCE.into(),
            inputs: Default::default(),
        };
        let id: u64 = add_pine_instance(&mut workspace, chart_id, &script, true)
            .unwrap()
            .instance_id
            .parse()
            .unwrap();
        let alert = |state, instance_id, chart_id| PineAlertSubscription {
            id: 55,
            chart_id,
            instance_id: Some(instance_id),
            script_id: Some("s1".into()),
            script_name: "I".into(),
            source: SOURCE.into(),
            inputs: Default::default(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1d".into(),
            source_hash: "abc".into(),
            note: String::new(),
            state,
            created_at_ms: NOW,
            webhook_endpoint_ids: Vec::new(),
        };
        let armed = [alert(PineAlertState::Active, id, chart_id)];
        let set: InputArgs = serde_json::from_value(serde_json::json!({"len": 5})).unwrap();

        let refused = update_pine_instance(&mut workspace, chart_id, id, Some(&set), None, &armed);
        assert!(
            matches!(&refused, Err(McpError::Conflict(message)) if message.contains("55")),
            "{refused:?}"
        );
        assert!(matches!(
            remove_pine_instance(&mut workspace, chart_id, id, &armed),
            Err(McpError::Conflict(_))
        ));
        // Nothing changed.
        let listed = list_chart_pine(&workspace, chart_id).unwrap();
        assert_eq!(listed.instances.len(), 1);
        assert_eq!(
            chart_pine_detail(&workspace, chart_id, id)
                .unwrap()
                .inputs
                .len(),
            0
        );

        // Hiding it, or setting an input to what it already is, leaves the alert alone.
        assert!(
            update_pine_instance(&mut workspace, chart_id, id, None, Some(false), &armed).is_ok()
        );
        let same: InputArgs = serde_json::from_value(serde_json::json!({"len": 20})).unwrap();
        assert_eq!(
            update_pine_instance(&mut workspace, chart_id, id, Some(&same), None, &armed)
                .unwrap()
                .status,
            "unchanged"
        );
        // Another instance's alert, a paused or silenced one, or another chart's, does not block.
        for other in [
            alert(PineAlertState::Active, id + 1, chart_id),
            alert(PineAlertState::Paused, id, chart_id),
            alert(PineAlertState::ScriptChanged, id, chart_id),
            alert(PineAlertState::Active, id, chart_id + 1),
        ] {
            assert!(
                update_pine_instance(
                    &mut workspace,
                    chart_id,
                    id,
                    Some(&set),
                    None,
                    std::slice::from_ref(&other)
                )
                .is_ok(),
                "{other:?}"
            );
        }
        assert!(remove_pine_instance(&mut workspace, chart_id, id, &[]).is_ok());
    }

    #[test]
    fn script_alert_subscriptions_are_listed_without_webhooks_or_invented_history() {
        let subscription = PineAlertSubscription {
            id: 1_791_000_000_000_000_001,
            chart_id: 2,
            instance_id: Some(4),
            script_id: Some("script-1".into()),
            script_name: "Strat".into(),
            source: STRATEGY_SOURCE.into(),
            inputs: Default::default(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1d".into(),
            source_hash: "abc".into(),
            note: "watch".into(),
            state: PineAlertState::ScriptChanged,
            created_at_ms: NOW,
            webhook_endpoint_ids: vec![424_242],
        };
        let result = pine_alerts(&[subscription]);
        assert_eq!(result.total, 1);
        let info = &result.subscriptions[0];
        assert_eq!(info.subscription_id, "1791000000000000001");
        assert_eq!(
            (
                info.state.as_str(),
                info.instrument.as_str(),
                info.timeframe.as_str()
            ),
            ("script_changed", "binance:BTCUSDT", "1d")
        );
        assert!(info.sends_to_webhooks);
        assert!(!result.event_history_available && result.recent_events.is_empty());
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            !json.contains("424242") && !json.contains(STRATEGY_SOURCE.trim()),
            "{json}"
        );
    }

    // MARK: Several requests in one frame, applied by the runtime afterwards

    /// What the app does with a request: plan against the snapshot as it is now, queue the save
    /// with its precondition, and leave applying to the runtime (nothing is drained here).
    fn queue_create(store: &Store, host: &AlertHost, request: &NewAlert) -> Option<u64> {
        let plan = plan_create_alert(
            host,
            &WatchlistQuoteBook::default(),
            &HashMap::new(),
            request,
            NOW,
        )
        .ok()?;
        let CreatePlan::New { alert, baseline } = plan else {
            return None;
        };
        let command =
            AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Save { alert, baseline }, NOW)
                .with_precondition(AlertPrecondition::Create {
                    max_alerts: MAX_ALERTS,
                });
        store.enqueue_alert_command(&command).unwrap();
        Some(command.id)
    }

    fn queue_update(store: &Store, host: &AlertHost, request: &AlertUpdate) -> Option<u64> {
        let plan = plan_update_alert(
            host,
            &WatchlistQuoteBook::default(),
            &HashMap::new(),
            request,
            NOW,
        )
        .ok()?;
        let UpdatePlan::Change {
            alert,
            base,
            baseline,
        } = plan
        else {
            return None;
        };
        let command =
            AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Save { alert, baseline }, NOW)
                .with_precondition(AlertPrecondition::Unchanged(base));
        store.enqueue_alert_command(&command).unwrap();
        Some(command.id)
    }

    #[test]
    fn identical_creates_in_one_frame_make_one_alert_and_the_other_is_refused() {
        let (store, mut host) = alert_host();
        let first = queue_create(&store, &host, &request(true, "70000")).unwrap();
        let second = queue_create(&store, &host, &request(true, "70000")).unwrap();
        assert_eq!(
            alert_command_outcome(&host, first),
            AlertCommandOutcome::Pending
        );
        host.drain_commands(&store, NOW);
        assert_eq!(host.snapshot().alerts.len(), 1);
        assert_eq!(
            alert_command_outcome(&host, first),
            AlertCommandOutcome::Applied
        );
        assert_eq!(
            alert_command_outcome(&host, second),
            AlertCommandOutcome::Rejected(crate::model::alert::AlertRejectReason::Duplicate)
        );
    }

    #[test]
    fn a_burst_of_creates_at_the_cap_cannot_exceed_it() {
        let (store, mut host) = alert_host();
        for index in 0..MAX_ALERTS - 1 {
            let alert = PriceAlert::new(
                AlertAsset::new(
                    InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                    "BTC",
                    "Bitcoin",
                ),
                AlertCondition::CrossesAbove {
                    target: 1.0 + index as f64,
                },
                NOW,
            );
            store
                .enqueue_alert_command(&AlertRuntimeCommand::new(
                    AlertRuntimeCommandPayload::Save {
                        alert: Box::new(alert),
                        baseline: None,
                    },
                    NOW,
                ))
                .unwrap();
        }
        host.drain_commands(&store, NOW);
        assert_eq!(host.snapshot().alerts.len(), MAX_ALERTS - 1);
        // Every one of these sees 499 alerts and passes the planner's own cap check.
        let ids: Vec<_> = ["900001", "900002", "900003"]
            .iter()
            .map(|target| queue_create(&store, &host, &request(true, target)).unwrap())
            .collect();
        host.drain_commands(&store, NOW);
        assert_eq!(host.snapshot().alerts.len(), MAX_ALERTS);
        assert_eq!(
            alert_command_outcome(&host, ids[0]),
            AlertCommandOutcome::Applied
        );
        for id in &ids[1..] {
            assert_eq!(
                alert_command_outcome(&host, *id),
                AlertCommandOutcome::Rejected(crate::model::alert::AlertRejectReason::LimitReached)
            );
        }
    }

    #[test]
    fn two_edits_of_different_settings_in_one_frame_do_not_lose_the_first() {
        let (store, mut host, id) = seeded();
        let mut note = update(id);
        note.note = Some("first".into());
        let mut frequency = update(id);
        frequency.frequency = Some(AlertFrequency::EveryTime);
        let one = queue_update(&store, &host, &note).unwrap();
        let two = queue_update(&store, &host, &frequency).unwrap();
        host.drain_commands(&store, NOW);
        let stored = &host.snapshot().alerts[0];
        assert_eq!(stored.note, "first", "the first edit is kept");
        assert_eq!(
            stored.frequency,
            AlertFrequency::Once,
            "not overwritten by a stale copy"
        );
        assert_eq!(
            alert_command_outcome(&host, one),
            AlertCommandOutcome::Applied
        );
        assert_eq!(
            alert_command_outcome(&host, two),
            AlertCommandOutcome::Rejected(crate::model::alert::AlertRejectReason::Changed)
        );
    }

    #[test]
    fn an_edit_and_a_delete_in_one_frame_agree_in_either_order() {
        // Delete first, then an edit planned from the old snapshot: the alert stays deleted.
        let (store, mut host, id) = seeded();
        let mut note = update(id);
        note.note = Some("late".into());
        let delete = AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Delete(id), NOW);
        store.enqueue_alert_command(&delete).unwrap();
        let edit = queue_update(&store, &host, &note).unwrap();
        host.drain_commands(&store, NOW);
        assert!(host.snapshot().alerts.is_empty());
        assert_eq!(
            alert_command_outcome(&host, edit),
            AlertCommandOutcome::Rejected(crate::model::alert::AlertRejectReason::Missing)
        );

        // Edit first, then the delete: the delete still wins.
        let (store, mut host, id) = seeded();
        let mut note = update(id);
        note.note = Some("early".into());
        let edit = queue_update(&store, &host, &note).unwrap();
        store
            .enqueue_alert_command(&AlertRuntimeCommand::new(
                AlertRuntimeCommandPayload::Delete(id),
                NOW,
            ))
            .unwrap();
        host.drain_commands(&store, NOW);
        assert!(host.snapshot().alerts.is_empty());
        assert_eq!(
            alert_command_outcome(&host, edit),
            AlertCommandOutcome::Applied
        );
    }

    #[test]
    fn a_rejection_survives_a_reload_from_the_store() {
        // The background agent may apply commands; the app only sees them after a reload.
        let (store, mut host) = alert_host();
        let first = queue_create(&store, &host, &request(true, "70000")).unwrap();
        let second = queue_create(&store, &host, &request(true, "70000")).unwrap();
        host.drain_commands(&store, NOW);
        host.save_if_dirty(&store);
        let reloaded = AlertHost::load(&store, NOW);
        assert_eq!(
            alert_command_outcome(&reloaded, first),
            AlertCommandOutcome::Applied
        );
        assert!(matches!(
            alert_command_outcome(&reloaded, second),
            AlertCommandOutcome::Rejected(_)
        ));
    }

    // MARK: Sealed credentials

    use crate::service::keychain_api_keys::{KeyBackend, MemoryBackend};
    use crate::service::keychain_webhook_secrets::Scope;
    use crate::service::webhook_delivery::SecretStoreError;
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
        use crate::host::mcp_confirmation as describe;
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
}
