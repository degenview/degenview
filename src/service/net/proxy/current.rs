//! The proxy in force for the whole process. The agents that make requests read it when they are
//! built, so a change in Settings reaches the next request without a restart. A system proxy can
//! change under the app (a VPN client switching it on), so in System mode it is read again
//! when it is older than [`REFRESH`], on whichever worker asks next.

use super::{ProxyConfig, ProxyMode, ProxyPasswordStore, Resolved, Source, os, resolve};
use crate::lock::lock;
use crate::store::Store;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(30);

struct State {
    config: ProxyConfig,
    password: Option<String>,
    resolved: Option<Resolved>,
    checked: Instant,
    /// Counts the changes of `resolved`, so a cached agent knows it is out of date.
    generation: u64,
    /// Nothing is resolved before [`apply`]: tests and tools that never call it connect directly.
    applied: bool,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State {
        config: ProxyConfig::default(),
        password: None,
        resolved: None,
        checked: Instant::now(),
        generation: 0,
        applied: false,
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
    resolve(config, password, &environment, &os::read)
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
    set(&mut state, resolved);
    status_of(&state)
}

/// Applies the saved setting; the password is read from the secure storage only when a user
/// name makes one necessary.
pub fn init(store: &Store) -> Status {
    let config = ProxyConfig::load(store);
    let password = (config.mode == ProxyMode::Manual && !config.username.trim().is_empty())
        .then(|| ProxyPasswordStore::shared().read())
        .flatten();
    apply(config, password)
}

/// The proxy now in force. Cheap: a clone, except once per [`REFRESH`] in System mode.
pub fn snapshot() -> Snapshot {
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
    let state = lock(&STATE);
    Snapshot {
        generation: state.generation,
        proxy: state.resolved.clone(),
    }
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
    }
}

/// A `ureq` config with the proxy in force. Every agent is built from this, never from
/// `ureq::Agent::config_builder()`, so no request ignores the setting.
pub fn config_builder() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder().proxy(snapshot().proxy.map(|resolved| resolved.proxy))
}
