//! The proxy in force for the whole process. The agents that make requests read it when they are
//! built, so a change in Settings reaches the next request without a restart. A system proxy can
//! change under the app (a VPN client switching it on), so in System mode a worker about to
//! connect calls [`refresh_if_stale`], which reads it again once it is older than [`REFRESH`].
//! Every other read here is a clone under a lock.

use super::{ProxyConfig, ProxyMode, Resolved, Source, os, resolve};
use crate::lock::lock;
use crate::service::keychain::proxy_password::ProxyPasswordStore;
use crate::store::Store;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(30);
/// How often a process that does not own the setting (the alert helper) looks at the saved one.
const FOLLOW_EVERY: Duration = Duration::from_secs(2);
/// How often it reads the proxy password again, which another process may have changed.
const PASSWORD_EVERY: Duration = Duration::from_secs(30);

struct State {
    config: ProxyConfig,
    password: Option<String>,
    resolved: Option<Resolved>,
    checked: Instant,
    /// Counts the changes of `resolved`, so a cached agent knows it is out of date.
    generation: u64,
    /// Nothing is resolved before [`apply`]: tests and tools that never call it connect directly.
    applied: bool,
    /// When [`follow`] last looked at the saved setting.
    followed: Instant,
    /// When `password` was last read from the secure storage.
    password_read: Instant,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State {
        config: ProxyConfig::default(),
        password: None,
        resolved: None,
        checked: Instant::now(),
        generation: 0,
        applied: false,
        followed: Instant::now(),
        password_read: Instant::now(),
    })
});

/// What the agents build from.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub generation: u64,
    pub proxy: Option<Resolved>,
}

/// The proxy now in force, for a status line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub mode: ProxyMode,
    pub active: Option<Active>,
    /// Set when the setting is not doing what it says: shown on the page instead of leaving the
    /// user to wonder why the proxy is not in use.
    pub problem: Option<ProxyProblem>,
}

/// Why the proxy in force is not the one the setting asks for. The messages are shown as they are.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProxyProblem {
    /// The manual proxy is complete yet could not be built, so connections go direct.
    #[error("This proxy server cannot be used, so connections go directly.")]
    Unusable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Active {
    pub source: Source,
    /// `host:port`, with the protocol when it is not plain HTTP.
    pub server: String,
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn resolve_now(config: &ProxyConfig, password: Option<&str>) -> Option<Resolved> {
    resolve(config, password, &environment, &|| {
        #[cfg(test)]
        SYSTEM_READS.with(|reads| reads.set(reads.get() + 1));
        os::read()
    })
}

#[cfg(test)]
thread_local! {
    /// How many times this thread asked the operating system for its proxy, which blocks: a test
    /// that says "never on this path" counts it.
    static SYSTEM_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The system-proxy reads this thread made so far.
#[cfg(test)]
pub(crate) fn system_reads() -> usize {
    SYSTEM_READS.with(std::cell::Cell::get)
}

/// Makes the last system-proxy read older than [`REFRESH`], so the next worker about to connect
/// reads it again.
#[cfg(test)]
pub(crate) fn age_system_read() {
    lock(&STATE).checked = Instant::now()
        .checked_sub(REFRESH * 2)
        .unwrap_or_else(Instant::now);
}

fn set(state: &mut State, resolved: Option<Resolved>) {
    if state.resolved != resolved {
        state.resolved = resolved;
        state.generation += 1;
    }
    state.checked = Instant::now();
}

/// Makes `config` the proxy setting, with the manual proxy's `password`. Reads the system's
/// proxy, so it runs on a worker or at startup, never on a frame.
pub fn apply(config: ProxyConfig, password: Option<String>) -> Status {
    let resolved = resolve_now(&config, password.as_deref());
    let mut state = lock(&STATE);
    state.config = config;
    state.password = password;
    state.applied = true;
    state.password_read = Instant::now();
    set(&mut state, resolved);
    status_of(&state)
}

/// Applies the saved setting; the password is read from the secure storage only when a user
/// name makes one necessary.
pub fn init(store: &Store) -> Status {
    let config = ProxyConfig::load(store);
    let password = config
        .needs_password()
        .then(|| ProxyPasswordStore::shared().read())
        .flatten();
    apply(config, password)
}

/// For a process that does not own the setting, the alert helper: looks at the saved setting
/// every couple of seconds and applies it when it changed, so a change made in the app reaches
/// this process too. A changed password is found by reading it again every 30 seconds, only
/// for a manual proxy with a user name. `true` when something was applied.
pub fn follow(store: &Store) -> bool {
    follow_with(store, ProxyPasswordStore::shared(), Instant::now())
}

pub(super) fn follow_with(store: &Store, passwords: &ProxyPasswordStore, now: Instant) -> bool {
    let (applied, password_due) = {
        let mut state = lock(&STATE);
        if now.saturating_duration_since(state.followed) < FOLLOW_EVERY {
            return false;
        }
        state.followed = now;
        (
            state.applied.then(|| state.config.clone()),
            now.saturating_duration_since(state.password_read) >= PASSWORD_EVERY,
        )
    };
    let saved = ProxyConfig::load(store);
    if applied.as_ref() != Some(&saved) {
        let password = saved
            .needs_password()
            .then(|| passwords.read_fresh())
            .flatten();
        apply(saved, password);
        return true;
    }
    if !(password_due && saved.needs_password()) {
        return false;
    }
    let fresh = passwords.read_fresh();
    let current = lock(&STATE).password.clone();
    if fresh == current {
        lock(&STATE).password_read = Instant::now();
        return false;
    }
    apply(saved, fresh);
    true
}

/// In System mode, reads the system's proxy again when the last read is older than [`REFRESH`].
/// That read blocks (a process on macOS), so this runs only on a worker about to connect, never
/// on a frame. Another worker asking meanwhile goes on with the current answer.
pub fn refresh_if_stale() {
    let stale = {
        let mut state = lock(&STATE);
        let stale = state.applied
            && state.config.mode == ProxyMode::System
            && state.checked.elapsed() >= REFRESH;
        if stale {
            // Claimed: another worker asking meanwhile uses the current answer.
            state.checked = Instant::now();
        }
        stale.then(|| (state.config.clone(), state.password.clone()))
    };
    if let Some((config, password)) = stale {
        let resolved = resolve_now(&config, password.as_deref());
        let mut state = lock(&STATE);
        if state.config == config {
            set(&mut state, resolved);
        }
    }
}

/// The proxy now in force, as last resolved. Cheap: a clone.
pub fn snapshot() -> Snapshot {
    let state = lock(&STATE);
    Snapshot {
        generation: state.generation,
        proxy: state.resolved.clone(),
    }
}

/// Counts the changes of the proxy in force; a connection kept open is on the old route once this
/// moves.
pub fn generation() -> u64 {
    lock(&STATE).generation
}

/// The proxy a connection to `host` goes through; `None` when it connects directly.
pub fn route(host: &str) -> Option<Resolved> {
    snapshot().proxy.filter(|proxy| !proxy.skips(host))
}

pub fn status() -> Status {
    status_of(&lock(&STATE))
}

fn status_of(state: &State) -> Status {
    Status {
        mode: state.config.mode,
        active: state.resolved.as_ref().map(|resolved| Active {
            source: resolved.source,
            server: resolved.spec.describe(),
        }),
        problem: problem(&state.config, state.resolved.as_ref()),
    }
}

/// A manual proxy that passed validation but did not resolve: it could not be built, and every
/// connection would go direct without a word. Nothing to say while it is unfinished.
pub(super) fn problem(config: &ProxyConfig, resolved: Option<&Resolved>) -> Option<ProxyProblem> {
    (config.mode == ProxyMode::Manual && resolved.is_none() && config.validate().is_ok())
        .then_some(ProxyProblem::Unusable)
}

/// The host of `url`, as `Uri` spells it (an IPv6 address keeps its brackets).
pub(in crate::service) fn host_of(url: &str) -> Option<String> {
    let uri = url.parse::<ureq::http::Uri>().ok()?;
    uri.host().map(str::to_owned)
}

/// The proxy a request to `url` goes through; `None` when it goes direct. A URL that has no host
/// (one `ureq` will reject itself) gets the proxy in force. Called on a worker about to connect,
/// so it may refresh the system's proxy first.
pub(in crate::service) fn route_for_url(url: &str) -> Option<Resolved> {
    refresh_if_stale();
    match host_of(url) {
        Some(host) => route(&host),
        None => snapshot().proxy,
    }
}

/// A `ureq` config for requests to `url`: through the proxy in force, unless it skips the host.
/// Every agent is built from this or from `FollowingAgent::get_for`, never from
/// `ureq::Agent::config_builder()`, so no request ignores the setting. Called on a worker about
/// to connect, so it may refresh the system's proxy first.
pub fn config_builder_for(url: &str) -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder().proxy(route_for_url(url).map(|resolved| resolved.proxy))
}
