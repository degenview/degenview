//! The stateful bridge between the MCP server, SQLite, the Settings page and the workspaces.
//!
//! The host owns the listener's lifecycle (it exists only while the user has the server
//! enabled), persists settings and credentials through `store::mcp`, keeps the server's shared
//! permission snapshot equal to what is on disk, and hands queued tool requests to the app.
//! Persist first, then publish: a write that fails changes nothing the server can see.

pub mod alert;
pub mod chart;
pub mod confirmation;
pub mod paper;
pub mod pine;
pub mod portfolio;
pub mod replay;
pub mod snapshot;
pub mod watchlist;

use crate::mcp::bridge::{self, BridgeClient, BridgeInbox, Request};
use crate::mcp::state::{McpShared, McpStatus};
use crate::mcp::transport::auth::{generate_token, verifier_hex};
use crate::mcp::transport::seal;
use crate::mcp::transport::tls::{self, TlsIdentity};
use crate::mcp::{Listen, McpServer};
use crate::model::mcp::{
    AUDIT_LOG_LIMIT, BindMode, Cidr, MAX_CIDRS, MAX_CLIENTS, McpAction, McpAuditEntry,
    McpAuditKind, McpClient, McpSettings, Permission, PermissionSet, clean_client_name,
    is_valid_port,
};
use crate::model::watchlist::{new_id, now_ms};
use crate::service::keychain::mcp_seal::{KeyState, SealKey, SealKeyStore};
use crate::store::SharedStore;
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

#[cfg(test)]
mod tests;
