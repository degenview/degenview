//! State shared between the MCP server threads and the app: the client list, the global
//! permission cap, the server status and a generation counter.
//!
//! Every request reads this fresh, so revoking a credential or changing a permission takes
//! effect on the very next request. Nothing here knows about HTTP or egui.

use crate::model::mcp::{McpClient, Permission, PermissionSet};
use crate::model::watchlist::now_ms;
use crate::pine_studio::store::ScriptStore;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Opens the script library; replaceable so tests never read the user's scripts.
pub type ScriptLocator = Box<dyn Fn() -> Option<ScriptStore> + Send + Sync>;

/// Script reads and compiles running at once.
const PINE_JOBS: usize = 2;

/// Who a request was authenticated as. Carried in the HTTP request extensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIdentity {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpStatus {
    Disabled,
    Starting,
    Running { port: u16 },
    Stopping,
    Failed(String),
}

impl McpStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Starting => "Starting",
            Self::Running { .. } => "Running",
            Self::Stopping => "Stopping",
            Self::Failed(_) => "Failed",
        }
    }
}

#[derive(Default)]
struct Config {
    clients: Vec<McpClient>,
    global: PermissionSet,
}

pub struct McpShared {
    config: RwLock<Config>,
    status: Mutex<McpStatus>,
    generation: AtomicU64,
    usage_dirty: AtomicBool,
    pine_jobs: Arc<tokio::sync::Semaphore>,
    script_locator: RwLock<ScriptLocator>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl McpShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            config: RwLock::new(Config::default()),
            status: Mutex::new(McpStatus::Disabled),
            generation: AtomicU64::new(1),
            usage_dirty: AtomicBool::new(false),
            pine_jobs: Arc::new(tokio::sync::Semaphore::new(PINE_JOBS)),
            script_locator: RwLock::new(Box::new(ScriptStore::default_location)),
        })
    }

    fn read(&self) -> RwLockReadGuard<'_, Config> {
        self.config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Config> {
        self.config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn replace(&self, clients: Vec<McpClient>, global: PermissionSet) {
        let mut config = self.write();
        config.clients = clients;
        config.global = global;
    }

    pub fn clients(&self) -> Vec<McpClient> {
        self.read().clients.clone()
    }

    pub fn global(&self) -> PermissionSet {
        self.read().global.clone()
    }

    /// Resolves a bearer token to its client. Compares against every stored verifier in
    /// constant time, so the response time does not reveal how much of a token matched.
    /// Records the use for the "last used" column.
    pub fn authenticate(&self, token: &str) -> Option<ClientIdentity> {
        let presented = crate::mcp::auth::verifier_hex(token);
        let mut matched = None;
        {
            let config = self.read();
            for (index, client) in config.clients.iter().enumerate() {
                if crate::mcp::auth::verifiers_match(&client.verifier, &presented) {
                    matched = Some(index);
                }
            }
        }
        let index = matched?;
        let mut config = self.write();
        let client = config.clients.get_mut(index)?;
        client.last_used_ms = Some(now_ms());
        self.usage_dirty.store(true, Ordering::Release);
        Some(ClientIdentity {
            id: client.id.clone(),
            name: client.name.clone(),
        })
    }

    /// What `client_id` may do right now: its grants capped by the global permissions. A
    /// revoked (unknown) client has nothing.
    pub fn effective(&self, client_id: &str) -> PermissionSet {
        let config = self.read();
        config
            .clients
            .iter()
            .find(|client| client.id == client_id)
            .map(|client| client.effective(&config.global))
            .unwrap_or_default()
    }

    pub fn allows(&self, client_id: &str, permission: Permission) -> bool {
        self.effective(client_id).contains(permission)
    }

    /// The clients, when a use was recorded since the last call; the host persists them.
    pub fn take_usage_update(&self) -> Option<Vec<McpClient>> {
        self.usage_dirty
            .swap(false, Ordering::AcqRel)
            .then(|| self.clients())
    }

    /// A permit for one script read/compile, or `None` when too many are already running.
    pub fn try_pine_job(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.pine_jobs.clone().try_acquire_owned().ok()
    }

    pub fn script_store(&self) -> Option<ScriptStore> {
        (self
            .script_locator
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner()))()
    }

    /// Points script reads somewhere else (tests serve a throwaway library).
    #[cfg(test)]
    pub fn set_script_locator(&self, locator: ScriptLocator) {
        *self
            .script_locator
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = locator;
    }

    pub fn status(&self) -> McpStatus {
        lock(&self.status).clone()
    }

    pub fn set_status(&self, status: McpStatus) {
        *lock(&self.status) = status;
    }

    /// Bumped whenever the server stops: requests queued under an older generation never run.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::auth::verifier_hex;

    fn client(id: &str, token: &str, permissions: &[Permission]) -> McpClient {
        McpClient {
            id: id.into(),
            name: format!("client {id}"),
            verifier: verifier_hex(token),
            permissions: PermissionSet::of(permissions.iter().copied()),
            created_at_ms: 0,
            last_used_ms: None,
        }
    }

    #[test]
    fn authenticates_only_known_tokens_and_records_use() {
        let shared = McpShared::new();
        shared.replace(
            vec![client("a", "token-a", &[]), client("b", "token-b", &[])],
            PermissionSet::none(),
        );
        assert_eq!(shared.authenticate("token-b").unwrap().id, "b");
        assert!(shared.authenticate("token-c").is_none());
        assert!(shared.authenticate("").is_none());
        let update = shared.take_usage_update().expect("use recorded");
        assert!(
            update
                .iter()
                .find(|c| c.id == "b")
                .unwrap()
                .last_used_ms
                .is_some()
        );
        assert!(
            update
                .iter()
                .find(|c| c.id == "a")
                .unwrap()
                .last_used_ms
                .is_none()
        );
        assert!(shared.take_usage_update().is_none());
    }

    #[test]
    fn revocation_and_permission_changes_apply_immediately() {
        let shared = McpShared::new();
        let both = [Permission::ChartsRead, Permission::ChartsCreate];
        shared.replace(vec![client("a", "token-a", &both)], PermissionSet::of(both));
        assert!(shared.allows("a", Permission::ChartsCreate));

        // Global cap lowered: the client's own grant no longer counts.
        shared.replace(
            vec![client("a", "token-a", &both)],
            PermissionSet::of([Permission::ChartsRead]),
        );
        assert!(shared.allows("a", Permission::ChartsRead));
        assert!(!shared.allows("a", Permission::ChartsCreate));

        // Revoked: no token, no permissions.
        shared.replace(Vec::new(), PermissionSet::of(both));
        assert!(shared.authenticate("token-a").is_none());
        assert!(shared.effective("a").is_empty());
    }

    #[test]
    fn generation_only_moves_forward() {
        let shared = McpShared::new();
        let before = shared.generation();
        assert_eq!(shared.bump_generation(), before + 1);
        assert_eq!(shared.generation(), before + 1);
    }
}
