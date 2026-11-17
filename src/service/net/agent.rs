//! An HTTP agent that follows the proxy setting. A long-lived `ureq::Agent` keeps the proxy it
//! was built with, so one held in a struct or a static would ignore a change in Settings;
//! this one is rebuilt, with its own options, the first time it is asked for after a change, and
//! hands out the agent that suits the request's host.
//!
//! Its agents never follow redirects themselves: `ureq` decides again for every hop whether to use
//! the proxy, but only by the spelling of the host, so a redirect to a private address would still
//! go to the proxy. A request that wants redirects uses [`FollowingAgent::get_following`], which
//! picks the agent again for each hop.

use super::proxy;
use crate::lock::lock;
use std::sync::Mutex;
use ureq::config::ConfigBuilder;
use ureq::http::{Response, Uri};
use ureq::typestate::AgentScope;

/// Redirects a GET follows by hand, as `ureq`'s own limit.
const MAX_REDIRECTS: usize = 5;

type Configure = Box<dyn Fn(ConfigBuilder<AgentScope>) -> ConfigBuilder<AgentScope> + Send + Sync>;

/// The two agents for one proxy generation: for hosts that go through the proxy, and for hosts
/// that skip it. `ureq` can only skip by the spelling of a host, so the choice is made here,
/// per request, with the exact rules (`proxy::route`).
struct Built {
    generation: u64,
    proxied: ureq::Agent,
    direct: ureq::Agent,
}

pub struct FollowingAgent {
    configure: Configure,
    cached: Mutex<Option<Built>>,
}

impl FollowingAgent {
    /// `configure` sets the agent's own options (timeouts, redirects); the proxy is added here.
    pub fn new(
        configure: impl Fn(ConfigBuilder<AgentScope>) -> ConfigBuilder<AgentScope>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            configure: Box::new(configure),
            cached: Mutex::new(None),
        }
    }

    /// The agent for a request to `url`, for the proxy now in force. Cloning an agent shares its
    /// connection pool. Called on a worker about to connect, so it may refresh the system's proxy
    /// first (`proxy::refresh_if_stale`).
    pub fn get_for(&self, url: &str) -> ureq::Agent {
        proxy::refresh_if_stale();
        let snapshot = proxy::snapshot();
        let direct = snapshot
            .proxy
            .as_ref()
            .is_some_and(|resolved| proxy::host_of(url).is_some_and(|host| resolved.skips(&host)));
        let mut cached = lock(&self.cached);
        if cached
            .as_ref()
            .is_none_or(|built| built.generation != snapshot.generation)
        {
            let build = |proxy: Option<ureq::Proxy>| {
                let config = (self.configure)(ureq::Agent::config_builder().proxy(proxy))
                    .max_redirects(0)
                    .max_redirects_will_error(false)
                    .build();
                ureq::Agent::new_with_config(config)
            };
            *cached = Some(Built {
                generation: snapshot.generation,
                proxied: build(snapshot.proxy.map(|resolved| resolved.proxy)),
                direct: build(None),
            });
        }
        let built = cached.as_ref().expect("built above");
        if direct {
            built.direct.clone()
        } else {
            built.proxied.clone()
        }
    }

    /// A GET that follows up to [`MAX_REDIRECTS`] redirects, choosing the agent (proxied or direct)
    /// for each hop's host. `headers` go to the host the request started at and to no other, so a
    /// key meant for one service is not handed to wherever it redirects. A redirect that cannot be
    /// followed (no `Location`, a malformed one) is the response itself.
    pub fn get_following(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<Response<ureq::Body>, ureq::Error> {
        self.follow(url, headers, false)
    }

    /// [`get_following`](Self::get_following) for a HEAD request.
    pub fn head_following(&self, url: &str) -> Result<Response<ureq::Body>, ureq::Error> {
        self.follow(url, &[], true)
    }

    fn follow(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        head: bool,
    ) -> Result<Response<ureq::Body>, ureq::Error> {
        let start = proxy::host_of(url);
        let mut current = url.to_owned();
        for _ in 0..=MAX_REDIRECTS {
            let agent = self.get_for(&current);
            let mut request = if head {
                agent.head(&current)
            } else {
                agent.get(&current)
            };
            if proxy::host_of(&current) == start {
                for &(name, value) in headers {
                    request = request.header(name, value);
                }
            }
            let response = request.call()?;
            if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            let next = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .and_then(|location| resolve_location(&current, location));
            match next {
                Some(next) => current = next,
                None => return Ok(response),
            }
        }
        Err(ureq::Error::TooManyRedirects)
    }
}

/// `location` as an absolute URL, relative to the page `base` it came from.
fn resolve_location(base: &str, location: &str) -> Option<String> {
    let base: Uri = base.parse().ok()?;
    let (scheme, authority) = (base.scheme_str()?, base.authority()?);
    let location = location.trim();
    let url = if location.starts_with("http://") || location.starts_with("https://") {
        location.to_owned()
    } else if let Some(rest) = location.strip_prefix("//") {
        format!("{scheme}://{rest}")
    } else if location.starts_with('/') {
        format!("{scheme}://{authority}{location}")
    } else if location.starts_with('?') {
        format!("{scheme}://{authority}{}{location}", base.path())
    } else if location.is_empty() {
        return None;
    } else {
        // Beside the page: its path up to the last slash.
        let path = base.path();
        let directory = &path[..path.rfind('/').map_or(0, |slash| slash + 1)];
        format!("{scheme}://{authority}{directory}{location}")
    };
    url.parse::<Uri>().is_ok().then_some(url)
}

#[cfg(test)]
mod tests {
    use super::resolve_location;

    #[test]
    fn a_location_is_read_against_the_page_it_came_from() {
        let base = "https://api.example.com/v1/items?page=2";
        for (location, expected) in [
            ("https://other.example/x", Some("https://other.example/x")),
            ("http://other.example/x", Some("http://other.example/x")),
            ("//cdn.example/x", Some("https://cdn.example/x")),
            ("/root", Some("https://api.example.com/root")),
            ("next", Some("https://api.example.com/v1/next")),
            ("?page=3", Some("https://api.example.com/v1/items?page=3")),
            ("", None),
            ("http://bad host/", None),
        ] {
            assert_eq!(
                resolve_location(base, location).as_deref(),
                expected,
                "{location:?}"
            );
        }
        assert_eq!(
            resolve_location("http://127.0.0.1:9", "next").as_deref(),
            Some("http://127.0.0.1:9/next")
        );
    }
}
