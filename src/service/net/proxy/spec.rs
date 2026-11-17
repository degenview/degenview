//! One proxy server as every source describes it (the settings, the environment, the operating
//! system), before it becomes the `ureq` proxy the HTTP agent uses.

use ureq::ProxyProtocol;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Spec {
    pub protocol: ProxyProtocol,
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
        let host = proxy.host();
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
            .host(&self.host)
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
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let address = format!("{host}:{}", self.port);
        match label(self.protocol) {
            "HTTP" => address,
            protocol => format!("{address} ({protocol})"),
        }
    }
}

fn label(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::Http => "HTTP",
        ProxyProtocol::Https => "HTTPS",
        ProxyProtocol::Socks4 | ProxyProtocol::Socks4A => "SOCKS4",
        _ => "SOCKS5",
    }
}
