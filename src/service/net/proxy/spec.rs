//! One proxy server as every source describes it (the settings, the environment, the operating
//! system), before it becomes the `ureq` proxy the HTTP agent uses.

use std::borrow::Cow;
use std::fmt;
use ureq::ProxyProtocol;

/// `Debug` is written out below so a password never reaches a log.
#[derive(Clone, Eq, PartialEq)]
pub struct Spec {
    pub protocol: ProxyProtocol,
    /// The bare name or address: an IPv6 address has no brackets here (they belong to URLs and to
    /// `host:port` text, see `bare_host` and `describe`), so it can be dialled as it is.
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Spec {
    /// Reads `[scheme://][user:password@]host[:port]`. No scheme means an HTTP proxy, as in
    /// the environment variables and the Windows registry.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let proxy = ureq::Proxy::new(text).ok()?;
        let host = bare_host(proxy.host());
        if host.is_empty() {
            return None;
        }
        Some(Self {
            protocol: proxy.protocol(),
            host: host.to_owned(),
            port: proxy.port(),
            username: proxy.username().map(str::to_owned),
            password: proxy.password().map(str::to_owned),
        })
    }

    pub fn with_credentials(mut self, username: &str, password: Option<&str>) -> Self {
        let username = username.trim();
        if !username.is_empty() {
            self.username = Some(username.to_owned());
            self.password = password.map(str::to_owned);
        }
        self
    }

    /// The `ureq` proxy for this server. Traffic to the hosts `bypass` names (see
    /// `bypass::entries`) skips it.
    pub fn build(&self, bypass: &[String]) -> Result<ureq::Proxy, ureq::Error> {
        let mut builder = ureq::Proxy::builder(self.protocol)
            .host(&bracketed(&self.host))
            .port(self.port);
        if let Some(username) = &self.username {
            builder = builder.username(username);
            if let Some(password) = &self.password {
                builder = builder.password(password);
            }
        }
        for entry in bypass {
            builder = builder.no_proxy(entry);
        }
        builder.build()
    }

    /// `host:port`, with the protocol when it is not plain HTTP, for a status line.
    pub fn describe(&self) -> String {
        let address = format!("{}:{}", bracketed(&self.host), self.port);
        match label(self.protocol) {
            "HTTP" => address,
            protocol => format!("{address} ({protocol})"),
        }
    }
}

impl fmt::Debug for Spec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Spec")
            .field("protocol", &self.protocol)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// `host` as a URL or `host:port` text needs it: an IPv6 address in brackets.
pub(super) fn bracketed(host: &str) -> Cow<'_, str> {
    if host.contains(':') && !host.starts_with('[') {
        Cow::Owned(format!("[{host}]"))
    } else {
        Cow::Borrowed(host)
    }
}

/// `host` without the brackets an IPv6 address has in a URL.
pub(super) fn bare_host(host: &str) -> &str {
    let host = host.trim();
    host.strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host)
}

fn label(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::Http => "HTTP",
        ProxyProtocol::Https => "HTTPS",
        ProxyProtocol::Socks4 | ProxyProtocol::Socks4A => "SOCKS4",
        _ => "SOCKS5",
    }
}
