//! State shared between the MCP server threads and the app: the client list, the global
//! permission cap, the server status and a generation counter.
//!
//! Every request reads this fresh, so revoking a credential or changing a permission takes
//! effect on the very next request. Nothing here knows about HTTP or egui.

use crate::mcp::audit::AuditQueue;
use crate::mcp::transport::cancel::CallRegistry;
use crate::mcp::transport::limits::{RateLimiter, RateLimits};
use crate::model::DataSource;
use crate::model::InstrumentId;
use crate::model::mcp::{McpClient, Peer, Permission, PermissionSet};
use crate::model::watchlist::now_ms;
use crate::service::SearchResult;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::scripts::ScriptStore;
#[cfg(test)]
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

/// Opens the script library; replaceable so tests never read the user's scripts.
pub type ScriptLocator = Box<dyn Fn() -> Option<ScriptStore> + Send + Sync>;

/// Searches a provider for symbols; replaceable so tests never reach the network.
pub type MarketSearch =
    Arc<dyn Fn(DataSource, &str) -> Result<Vec<SearchResult>, String> + Send + Sync>;

/// Fetches candles from a provider: `(source, symbol, interval token, limit)`. Replaceable so
/// tests never reach the network.
pub type CandleFetch = Arc<
    dyn Fn(DataSource, &str, &str, u32) -> Result<Vec<crate::model::Kline>, String> + Send + Sync,
>;

/// Script reads and compiles running at once.
const PINE_JOBS: usize = 2;
/// Provider searches running at once. Each is a blocking network call.
const SEARCH_JOBS: usize = 4;
/// Provider candle fetches running at once for the market candles tool.
const CANDLE_JOBS: usize = 3;
/// Quote requests running at once. Each holds a demand on the quote coordinator while it waits.
const QUOTE_JOBS: usize = 4;
/// Where the demand ids of MCP quote requests start: clear of every window, alert and card owner.
const QUOTE_OWNER_BASE: u64 = 1 << 59;

/// The live quote feed the app already runs for its watchlists. Cheap to clone and safe to use
/// from any thread: asking for a market's quote is a short lock on the coordinator, never a
/// workspace or the store.
#[derive(Clone)]
pub struct QuoteAccess {
    pub book: WatchlistQuoteBook,
    pub coordinator: WatchlistQuoteCoordinator,
}

/// Who a request was authenticated as. Carried in the HTTP request extensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIdentity {
    pub id: String,
    pub name: String,
    /// The address the request came from (the socket's, never a header's). Always known here:
    /// an unknown peer is refused before an identity exists.
    pub peer: Peer,
    /// Asked about as a remote credential: the request came from another machine, or the
    /// credential may be used from one, as of the moment it was authenticated. Fixed then, so a
    /// request already queued keeps its origin when remote access is switched off before it runs.
    pub remote: bool,
}

impl ClientIdentity {
    /// Where the request came from, for the bridge to carry to the moment it runs.
    pub fn origin(&self) -> Origin {
        Origin {
            peer: self.peer,
            remote: self.remote,
        }
    }
}

/// What a queued request remembers of how it arrived (see [`ClientIdentity::remote`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub peer: Peer,
    pub remote: bool,
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
    /// The most a client with remote access may do (`McpSettings::remote_ceiling`).
    remote_ceiling: PermissionSet,
}

/// A request's demand on the quote feed; released when dropped, even if the request is cancelled.
pub struct QuoteLease {
    access: QuoteAccess,
    owner: u64,
}

impl QuoteLease {
    pub fn book(&self) -> &WatchlistQuoteBook {
        &self.access.book
    }
}

impl Drop for QuoteLease {
    fn drop(&mut self) {
        self.access.coordinator.release(self.owner);
    }
}

pub struct McpShared {
    config: RwLock<Config>,
    status: Mutex<McpStatus>,
    generation: AtomicU64,
    usage_dirty: AtomicBool,
    require_confirmation: AtomicBool,
    limiter: Mutex<RateLimiter>,
    calls: Arc<CallRegistry>,
    audit: AuditQueue,
    pine_jobs: Arc<tokio::sync::Semaphore>,
    script_locator: RwLock<ScriptLocator>,
    search_jobs: Arc<tokio::sync::Semaphore>,
    quote_jobs: Arc<tokio::sync::Semaphore>,
    candle_jobs: Arc<tokio::sync::Semaphore>,
    candle_fetch: RwLock<CandleFetch>,
    market_search: RwLock<MarketSearch>,
    quotes: RwLock<Option<QuoteAccess>>,
    next_quote_owner: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl McpShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            config: RwLock::new(Config {
                remote_ceiling: crate::model::mcp::default_remote_ceiling(),
                ..Config::default()
            }),
            status: Mutex::new(McpStatus::Disabled),
            generation: AtomicU64::new(1),
            usage_dirty: AtomicBool::new(false),
            require_confirmation: AtomicBool::new(true),
            limiter: Mutex::new(RateLimiter::new(RateLimits::default())),
            calls: Arc::new(CallRegistry::default()),
            audit: AuditQueue::default(),
            pine_jobs: Arc::new(tokio::sync::Semaphore::new(PINE_JOBS)),
            script_locator: RwLock::new(Box::new(ScriptStore::default_location)),
            search_jobs: Arc::new(tokio::sync::Semaphore::new(SEARCH_JOBS)),
            quote_jobs: Arc::new(tokio::sync::Semaphore::new(QUOTE_JOBS)),
            candle_jobs: Arc::new(tokio::sync::Semaphore::new(CANDLE_JOBS)),
            candle_fetch: RwLock::new(Arc::new(crate::service::fetch_klines)),
            market_search: RwLock::new(Arc::new(|source, query| {
                crate::service::search(source, query)
            })),
            quotes: RwLock::new(None),
            next_quote_owner: AtomicU64::new(QUOTE_OWNER_BASE),
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

    pub fn set_remote_ceiling(&self, ceiling: PermissionSet) {
        self.write().remote_ceiling = ceiling;
    }

    pub fn replace(&self, clients: Vec<McpClient>, global: PermissionSet) {
        lock(&self.limiter).retain_clients(clients.iter().map(|client| client.id.as_str()));
        let mut config = self.write();
        config.clients = clients;
        config.global = global;
    }

    /// Whether high-impact calls wait for the user's approval. Starts on, so a server that has
    /// not loaded its settings yet asks rather than acts.
    pub fn require_confirmation(&self) -> bool {
        self.require_confirmation.load(Ordering::Acquire)
    }

    pub fn set_require_confirmation(&self, required: bool) {
        self.require_confirmation.store(required, Ordering::Release);
    }

    /// Rows for the activity log, waiting for the app thread to write them.
    pub fn audit(&self) -> &AuditQueue {
        &self.audit
    }

    /// The tool calls in flight, so a `notifications/cancelled` can find the one it names.
    pub fn calls(&self) -> &Arc<CallRegistry> {
        &self.calls
    }

    /// Counts one failed authentication against the shared bucket (and, from another machine,
    /// against that address's own first). `Err` is how long the caller should wait. Only bad credentials ever pass through here, so valid ones are never slowed
    /// by a flood of bad ones.
    pub fn note_failed_auth(&self, peer: Option<Peer>) -> Result<(), Duration> {
        lock(&self.limiter).failed_auth(peer, Instant::now())
    }

    /// Counts one call from `client_id` against its own bucket.
    pub fn note_client_call(&self, client_id: &str) -> Result<(), Duration> {
        lock(&self.limiter).client_call(client_id, Instant::now())
    }

    /// Replaces the rate limits and refills every bucket. Tests use it to make limits easy to
    /// reach (or impossible to reach).
    #[cfg(test)]
    pub fn set_rate_limits(&self, limits: RateLimits) {
        lock(&self.limiter).set_limits(limits);
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
    ///
    /// `peer` is the address the request came from. A valid token from an address its client may
    /// not be used from is refused exactly like an unknown token, so the answer never says that
    /// the token was right. `None` (unknown) counts as remote.
    pub fn authenticate_from(&self, token: &str, peer: Option<Peer>) -> Option<ClientIdentity> {
        self.authenticate_with(token, peer, || {})
    }

    /// [`Self::authenticate_from`] a client on this machine. Tests only: the server always knows
    /// the peer.
    #[cfg(test)]
    pub fn authenticate(&self, token: &str) -> Option<ClientIdentity> {
        self.authenticate_from(
            token,
            Some(Peer::from_ip(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))),
        )
    }

    /// [`Self::authenticate`] with a hook that runs after the match, while no lock is held.
    /// Tests use it to replace the client list at the one moment that used to matter.
    ///
    /// The match is carried across the unlocked gap as the client's id, never as a position in
    /// the list (a revocation shifts positions), and the verifier is checked again against
    /// whichever client now has that id. So a token only ever yields the client whose verifier
    /// matched it, or nothing once that client is gone or its credential was replaced.
    fn authenticate_with(
        &self,
        token: &str,
        peer: Option<Peer>,
        between: impl FnOnce(),
    ) -> Option<ClientIdentity> {
        let presented = crate::mcp::transport::auth::verifier_hex(token);
        let mut matched: Option<String> = None;
        {
            let config = self.read();
            // Every verifier is compared, with no early exit, so timing does not reveal
            // which one matched.
            for client in &config.clients {
                if crate::mcp::transport::auth::verifiers_match(&client.verifier, &presented) {
                    matched = Some(client.id.clone());
                }
            }
        }
        let id = matched?;
        between();
        let mut config = self.write();
        let client = config.clients.iter_mut().find(|client| {
            client.id == id
                && crate::mcp::transport::auth::verifiers_match(&client.verifier, &presented)
        })?;
        // After the verifier, so the order of checks is not an oracle for which tokens exist;
        // before any use is recorded, so a refused address leaves no trace on the client.
        let peer = peer?;
        if !client.accepts_peer(Some(peer)) {
            return None;
        }
        client.last_used_ms = Some(now_ms());
        self.usage_dirty.store(true, Ordering::Release);
        Some(ClientIdentity {
            id: client.id.clone(),
            name: client.name.clone(),
            peer,
            remote: !peer.local || client.remote_allowed,
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
            .map(|client| client.effective(&config.global, &config.remote_ceiling))
            .unwrap_or_default()
    }

    /// Whether `client_id` may be used from other machines. A revoked (unknown) client counts as
    /// remote: a question about it is never skipped on the strength of a client that is gone.
    pub fn is_remote_client(&self, client_id: &str) -> bool {
        self.read()
            .clients
            .iter()
            .find(|client| client.id == client_id)
            .is_none_or(|client| client.remote_allowed)
    }

    /// Whether the credential, as it is now, still admits a request that arrived as `origin`. A
    /// request that waited in the queue is judged again just before it runs, as its permission is:
    /// remote access switched off, or its address taken out of the allowed ranges, in between
    /// stops it.
    pub fn accepts_origin(&self, client_id: &str, origin: &Origin) -> bool {
        self.read()
            .clients
            .iter()
            .find(|client| client.id == client_id)
            .is_some_and(|client| client.accepts_peer(Some(origin.peer)))
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

    /// Marks the "last used" times as not yet saved again, after a failed save.
    pub fn mark_usage_dirty(&self) {
        self.usage_dirty.store(true, Ordering::Release);
    }

    /// A permit for one script read/compile, or `None` when too many are already running.
    pub fn try_pine_job(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.pine_jobs.clone().try_acquire_owned().ok()
    }

    /// A permit for one provider search, or `None` when too many are already running.
    pub fn try_search_job(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.search_jobs.clone().try_acquire_owned().ok()
    }

    /// A permit for one quote request, or `None` when too many are already waiting.
    pub fn try_quote_job(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.quote_jobs.clone().try_acquire_owned().ok()
    }

    /// A permit for one candle fetch, or `None` when too many are already running.
    pub fn try_candle_job(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.candle_jobs.clone().try_acquire_owned().ok()
    }

    pub fn candle_fetch(&self) -> CandleFetch {
        self.candle_fetch
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Points candle fetches somewhere else (tests serve canned bars).
    #[cfg(test)]
    pub fn set_candle_fetch(&self, fetch: CandleFetch) {
        *self
            .candle_fetch
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = fetch;
    }

    pub fn market_search(&self) -> MarketSearch {
        self.market_search
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Points symbol search somewhere else (tests serve canned results).
    #[cfg(test)]
    pub fn set_market_search(&self, search: MarketSearch) {
        *self
            .market_search
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = search;
    }

    /// Hands the server the app's quote feed. Until then (and in tests that never do) quotes
    /// are unavailable.
    pub fn attach_quotes(&self, access: QuoteAccess) {
        *self
            .quotes
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(access);
    }

    pub fn quotes(&self) -> Option<QuoteAccess> {
        self.quotes
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Asks for `instruments` to be quoted until the returned lease is dropped. A lease has its
    /// own owner id, so concurrent requests never replace each other's demand.
    pub fn lease_quotes(&self, access: QuoteAccess, instruments: Vec<InstrumentId>) -> QuoteLease {
        let owner = self.next_quote_owner.fetch_add(1, Ordering::AcqRel);
        access.coordinator.set_demand(owner, instruments, true);
        QuoteLease { access, owner }
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
    use crate::mcp::transport::auth::verifier_hex;

    fn client(id: &str, token: &str, permissions: &[Permission]) -> McpClient {
        McpClient {
            id: id.into(),
            name: format!("client {id}"),
            verifier: verifier_hex(token),
            permissions: PermissionSet::of(permissions.iter().copied()),
            created_at_ms: 0,
            last_used_ms: None,
            remote_allowed: false,
            allowed_cidrs: Vec::new(),
        }
    }

    fn peer(text: &str) -> Option<Peer> {
        Some(Peer::from_ip(text.parse().unwrap()))
    }

    fn remote_client(id: &str, token: &str, cidrs: &[&str]) -> McpClient {
        McpClient {
            remote_allowed: true,
            allowed_cidrs: cidrs
                .iter()
                .map(|c| crate::model::mcp::Cidr::parse(c).unwrap())
                .collect(),
            ..client(
                id,
                token,
                &[Permission::ChartsRead, Permission::ChartsCreate],
            )
        }
    }

    #[test]
    fn a_valid_token_from_an_address_it_may_not_use_is_refused_like_an_unknown_one() {
        let shared = McpShared::new();
        shared.replace(
            vec![
                client("local", "token-local", &[Permission::ChartsRead]),
                remote_client("lan", "token-lan", &["192.168.1.0/24"]),
                remote_client("open", "token-open", &[]),
            ],
            PermissionSet::everything(),
        );
        let from = |token, at| shared.authenticate_from(token, at).map(|i| i.id);
        // A credential without remote access works from this machine only.
        assert_eq!(
            from("token-local", peer("127.0.0.1")).as_deref(),
            Some("local")
        );
        assert_eq!(from("token-local", peer("::1")).as_deref(), Some("local"));
        assert_eq!(from("token-local", peer("192.168.1.9")), None);
        // With remote access: inside its ranges, from anywhere when it has none.
        assert_eq!(
            from("token-lan", peer("192.168.1.9")).as_deref(),
            Some("lan")
        );
        assert_eq!(from("token-lan", peer("203.0.113.9")), None);
        assert_eq!(from("token-lan", peer("127.0.0.1")).as_deref(), Some("lan"));
        assert_eq!(
            from("token-open", peer("203.0.113.9")).as_deref(),
            Some("open")
        );
        // An unknown peer is a remote one.
        assert_eq!(from("token-open", None), None);
        assert_eq!(from("token-local", None), None);
        // A wrong token from an allowed address is no better off.
        assert_eq!(from("nope", peer("192.168.1.9")), None);
    }

    #[test]
    fn a_client_is_remote_when_it_may_be_used_remotely_or_is_gone() {
        let shared = McpShared::new();
        shared.replace(
            vec![
                client("local", "t1", &[]),
                remote_client("remote", "t2", &[]),
            ],
            PermissionSet::none(),
        );
        assert!(!shared.is_remote_client("local"));
        assert!(shared.is_remote_client("remote"));
        assert!(shared.is_remote_client("revoked"));
    }

    #[test]
    fn a_credential_that_may_not_leave_this_machine_works_from_this_machines_own_address() {
        // The server listens on the Mac's LAN address and a client on that Mac connects to it, so
        // the peer is the LAN address itself (`local`, see `Peer::of_connection`). Another host on
        // the same network is not.
        let own: IpAddr = "192.168.1.20".parse().unwrap();
        let shared = McpShared::new();
        shared.replace(
            vec![
                client("local", "token-local", &[Permission::ChartsRead]),
                remote_client("lan", "token-lan", &["10.9.9.0/24"]),
            ],
            PermissionSet::everything(),
        );
        let here = Some(Peer::of_connection(own, own));
        let there = Some(Peer::of_connection("192.168.1.21".parse().unwrap(), own));
        let id = |token, peer| shared.authenticate_from(token, peer).map(|i| i.id);
        assert_eq!(id("token-local", here).as_deref(), Some("local"));
        assert_eq!(id("token-local", there), None);
        // A range limits where a remote credential may come from, not this machine.
        assert_eq!(id("token-lan", here).as_deref(), Some("lan"));
        assert_eq!(id("token-lan", there), None);
    }

    #[test]
    fn a_refused_address_leaves_no_trace_on_the_client() {
        let shared = McpShared::new();
        shared.replace(
            vec![client("a", "token-a", &[Permission::ChartsRead])],
            PermissionSet::everything(),
        );
        assert!(
            shared
                .authenticate_from("token-a", peer("192.168.1.9"))
                .is_none()
        );
        assert!(
            shared.take_usage_update().is_none(),
            "no use may be recorded"
        );
        assert!(shared.clients()[0].last_used_ms.is_none());
    }

    #[test]
    fn the_ceiling_caps_a_remote_client_and_follows_the_setting() {
        let shared = McpShared::new();
        shared.replace(
            vec![
                remote_client("remote", "token-r", &[]),
                client(
                    "local",
                    "token-l",
                    &[Permission::ChartsRead, Permission::ChartsCreate],
                ),
            ],
            PermissionSet::everything(),
        );
        // The default ceiling is reads only.
        assert!(shared.allows("remote", Permission::ChartsRead));
        assert!(!shared.allows("remote", Permission::ChartsCreate));
        assert!(shared.allows("local", Permission::ChartsCreate));

        shared.set_remote_ceiling(PermissionSet::everything());
        assert!(shared.allows("remote", Permission::ChartsCreate));
        shared.set_remote_ceiling(PermissionSet::none());
        assert!(shared.effective("remote").is_empty());
        assert!(shared.allows("local", Permission::ChartsCreate));
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
    fn a_token_never_resolves_to_another_client_when_the_list_shifts_mid_check() {
        let shared = McpShared::new();
        shared.replace(
            vec![
                client("a", "token-a", &[]),
                client("b", "token-b", &[]),
                client("c", "token-c", &[]),
            ],
            PermissionSet::none(),
        );
        // Token B matches client B at position 1; then client A is revoked, so position 1
        // is now client C. The old code returned C here.
        let identity = shared.authenticate_with(
            "token-b",
            Some(Peer::from_ip(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))),
            || {
                shared.replace(
                    vec![client("b", "token-b", &[]), client("c", "token-c", &[])],
                    PermissionSet::none(),
                );
            },
        );
        assert_eq!(identity.unwrap().id, "b");
        let clients = shared.clients();
        assert!(
            clients
                .iter()
                .find(|c| c.id == "b")
                .unwrap()
                .last_used_ms
                .is_some()
        );
        assert!(
            clients
                .iter()
                .find(|c| c.id == "c")
                .unwrap()
                .last_used_ms
                .is_none()
        );
    }

    #[test]
    fn a_token_whose_client_is_revoked_mid_check_gets_no_identity() {
        let shared = McpShared::new();
        shared.replace(
            vec![client("a", "token-a", &[]), client("b", "token-b", &[])],
            PermissionSet::none(),
        );
        let identity = shared.authenticate_with(
            "token-b",
            Some(Peer::from_ip(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))),
            || shared.replace(Vec::new(), PermissionSet::none()),
        );
        assert!(identity.is_none());
        assert!(shared.take_usage_update().is_none());

        // A credential replaced under the same id no longer matches the old token.
        shared.replace(vec![client("b", "token-b", &[])], PermissionSet::none());
        let identity = shared.authenticate_with(
            "token-b",
            Some(Peer::from_ip(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))),
            || {
                shared.replace(vec![client("b", "token-b2", &[])], PermissionSet::none());
            },
        );
        assert!(identity.is_none());
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
