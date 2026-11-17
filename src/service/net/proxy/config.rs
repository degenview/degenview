//! The proxy setting. Saved as the local `net.proxy` setting: it names this Mac's own network,
//! so it never syncs. The password is not here; it lives in the secure storage
//! (`ProxyPasswordStore`).

use super::bypass::{self, BypassError};
use super::spec::Spec;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use ureq::ProxyProtocol;

/// Where the proxy comes from.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    /// The `HTTP(S)_PROXY` environment variables, else the operating system's proxy.
    #[default]
    System,
    /// The server set here.
    Manual,
    /// No proxy, whatever the system says.
    Off,
}

/// What the manual proxy speaks.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Protocol {
    /// An HTTP proxy that tunnels with `CONNECT`.
    #[default]
    Http,
    Socks5,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    pub protocol: Protocol,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    /// Hosts that skip the proxy, on top of the always-direct local ones.
    pub no_proxy: String,
}

/// Why a manual proxy cannot be used yet. The messages are shown as they are.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProxyError {
    #[error("Enter the proxy's host name or address.")]
    MissingHost,
    #[error("\"{0}\" is not a host name or address. Leave out http:// and the path.")]
    BadHost(String),
    #[error("Enter the proxy's port.")]
    MissingPort,
    #[error(transparent)]
    NoProxy(#[from] BypassError),
}

impl ProxyConfig {
    pub const KEY: &'static str = "net.proxy";

    pub fn load(store: &Store) -> Self {
        store.setting(Self::KEY).unwrap_or_default()
    }

    pub fn save(&self, store: &Store) -> Result<(), String> {
        store
            .set_setting(Self::KEY, self)
            .map_err(|error| error.to_string())
    }

    /// Whether this can be saved and used. System and Off need nothing; only the no-proxy list
    /// is checked for them, since it applies on top of the system's proxy too.
    pub fn validate(&self) -> Result<(), ProxyError> {
        bypass::check(&bypass::split(&self.no_proxy))?;
        if self.mode != ProxyMode::Manual {
            return Ok(());
        }
        let host = self.host.trim();
        if host.is_empty() {
            return Err(ProxyError::MissingHost);
        }
        if !host
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'))
        {
            return Err(ProxyError::BadHost(host.to_owned()));
        }
        if self.port.is_none() {
            return Err(ProxyError::MissingPort);
        }
        Ok(())
    }

    /// The manual server, or `None` while it is incomplete.
    pub(super) fn spec(&self, password: Option<&str>) -> Option<Spec> {
        self.validate().ok()?;
        let protocol = match self.protocol {
            Protocol::Http => ProxyProtocol::Http,
            // The proxy resolves host names, so a name blocked locally still works.
            Protocol::Socks5 => ProxyProtocol::Socks5h,
        };
        let spec = Spec {
            protocol,
            host: self.host.trim().to_owned(),
            port: self.port?,
            username: None,
            password: None,
        };
        Some(spec.with_credentials(&self.username, password))
    }
}
