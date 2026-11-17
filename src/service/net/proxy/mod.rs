//! Network proxy support. One setting, [`ProxyConfig`], says whether DegenView follows the
//! system's proxy (the default), uses one the user typed, or connects directly. [`resolve`]
//! turns it into the proxy in force and where it came from, [`current`] keeps that for the whole
//! process and rebuilds the HTTP agents when it changes, and [`tunnel`] carries the WebSocket
//! live feeds through the same proxy. See `docs/network-proxy.md`.

mod bypass;
mod config;
mod current;
mod os;
mod password;
mod spec;
mod tunnel;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::AppliedProxy;

pub use config::{Protocol, ProxyConfig, ProxyMode};
pub use current::{Active, Status, apply, config_builder_for, follow, init, status};
pub(in crate::service) use current::{generation, host_of, route, snapshot};
pub use password::ProxyPasswordStore;
pub(in crate::service) use tunnel::dial;

use os::SystemProxy;
use spec::Spec;

/// Where the proxy in force was found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    Manual,
    Environment,
    System,
}

/// The proxy to use, as a server and as the `ureq` proxy that already knows what to skip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resolved {
    pub spec: Spec,
    pub proxy: ureq::Proxy,
    pub source: Source,
}

impl Resolved {
    fn new(spec: Spec, bypass: &[String], source: Source) -> Option<Self> {
        let proxy = spec.build(&bypass::entries(bypass)).ok()?;
        Some(Self {
            spec,
            proxy,
            source,
        })
    }

    /// Whether a connection to `host` skips the proxy: it is local (`bypass::is_local`), or the
    /// no-proxy lists name it.
    pub fn skips(&self, host: &str) -> bool {
        if bypass::is_local(host) {
            return true;
        }
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        format!("https://{host}/")
            .parse::<ureq::http::Uri>()
            .is_ok_and(|uri| self.proxy.is_no_proxy(&uri))
    }
}

/// Which proxy `config` selects: the manual one, or, in System mode, the environment's, else the
/// system's. `env` reads an environment variable; `system` reads the operating system's setting
/// (only asked when the environment names none). The user's no-proxy list adds to every source's.
pub fn resolve(
    config: &ProxyConfig,
    password: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
    system: &dyn Fn() -> Option<SystemProxy>,
) -> Option<Resolved> {
    let (spec, mut skipped, source) = match config.mode {
        ProxyMode::Off => return None,
        ProxyMode::Manual => (config.spec(password)?, Vec::new(), Source::Manual),
        ProxyMode::System => match from_environment(env) {
            Some((spec, skipped)) => (spec, skipped, Source::Environment),
            None => {
                let found = system()?;
                (found.spec, found.bypass, Source::System)
            }
        },
    };
    skipped.extend(bypass::split(&config.no_proxy));
    Resolved::new(spec, &skipped, source)
}

/// The proxy URL variables, and `NO_PROXY` beside them.
fn from_environment(env: &dyn Fn(&str) -> Option<String>) -> Option<(Spec, Vec<String>)> {
    const URLS: [&str; 6] = [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ];
    let spec = URLS
        .iter()
        .find_map(|name| env(name).as_deref().and_then(Spec::parse))?;
    let skipped = ["NO_PROXY", "no_proxy"]
        .iter()
        .find_map(|name| env(name))
        .map(|value| bypass::split(&value))
        .unwrap_or_default();
    Some((spec, skipped))
}
