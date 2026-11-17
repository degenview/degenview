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

/// `location` as an absolute URL, resolved against the page `base` it came from as RFC 3986
/// section 5.2 says, which is what a browser or `curl` sends: dot segments are removed and a
/// fragment is dropped. `None` for an empty or unparsable `Location`.
fn resolve_location(base: &str, location: &str) -> Option<String> {
    let base: Uri = base.parse().ok()?;
    let (base_scheme, base_authority) = (base.scheme_str()?, base.authority()?);
    let location = location.trim();
    if location.is_empty() {
        return None;
    }
    let reference = location.split('#').next().unwrap_or_default();
    let with_authority = if reference.starts_with("http://") || reference.starts_with("https://") {
        Some(reference.to_owned())
    } else if reference.starts_with("//") {
        Some(format!("{base_scheme}:{reference}"))
    } else {
        None
    };
    let (scheme, authority, path, query) = match with_authority {
        Some(text) => {
            let uri: Uri = text.parse().ok()?;
            (
                uri.scheme_str()?.to_owned(),
                uri.authority()?.to_string(),
                remove_dot_segments(uri.path()),
                uri.query().map(str::to_owned),
            )
        }
        None => {
            let (path, query) = match reference.split_once('?') {
                Some((path, query)) => (path, Some(query.to_owned())),
                None => (reference, None),
            };
            let (path, query) = if path.is_empty() {
                // Only a query (or a fragment): the same page, with the query it names.
                (
                    base.path().to_owned(),
                    query.or(base.query().map(str::to_owned)),
                )
            } else if path.starts_with('/') {
                (remove_dot_segments(path), query)
            } else {
                // Beside the page: its path up to the last slash, then the reference.
                let directory = &base.path()[..base.path().rfind('/').map_or(0, |slash| slash + 1)];
                (remove_dot_segments(&format!("{directory}{path}")), query)
            };
            (
                base_scheme.to_owned(),
                base_authority.to_string(),
                path,
                query,
            )
        }
    };
    let url = match query {
        Some(query) => format!("{scheme}://{authority}{path}?{query}"),
        None => format!("{scheme}://{authority}{path}"),
    };
    url.parse::<Uri>().is_ok().then_some(url)
}

/// RFC 3986 section 5.2.4 for a path that starts with `/`: `.` goes, `..` takes the segment
/// before it (never going above the root), and a path that ends in either ends in a slash.
fn remove_dot_segments(path: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut segments = path.strip_prefix('/').unwrap_or(path).split('/').peekable();
    while let Some(segment) = segments.next() {
        let last = segments.peek().is_none();
        match segment {
            "." => {}
            ".." => {
                kept.pop();
            }
            _ => kept.push(segment),
        }
        if last && matches!(segment, "." | "..") {
            kept.push("");
        }
    }
    format!("/{}", kept.join("/"))
}

#[cfg(test)]
mod tests {
    use super::resolve_location;

    /// RFC 3986 section 5.4: the examples that resolve against one base, for the schemes this
    /// client speaks. `//g` and `` are the two places it differs: an authority-only URL gets its
    /// `/`, and an empty `Location` is no redirect at all.
    #[test]
    fn a_location_resolves_as_rfc_3986_says() {
        let base = "http://a/b/c/d;p?q";
        for (location, expected) in [
            ("http://g", Some("http://g/")),
            ("g", Some("http://a/b/c/g")),
            ("./g", Some("http://a/b/c/g")),
            ("g/", Some("http://a/b/c/g/")),
            ("/g", Some("http://a/g")),
            ("//g", Some("http://g/")),
            ("?y", Some("http://a/b/c/d;p?y")),
            ("g?y", Some("http://a/b/c/g?y")),
            ("#s", Some("http://a/b/c/d;p?q")),
            ("g#s", Some("http://a/b/c/g")),
            ("g?y#s", Some("http://a/b/c/g?y")),
            (";x", Some("http://a/b/c/;x")),
            ("g;x", Some("http://a/b/c/g;x")),
            ("g;x?y#s", Some("http://a/b/c/g;x?y")),
            ("", None),
            (".", Some("http://a/b/c/")),
            ("./", Some("http://a/b/c/")),
            ("..", Some("http://a/b/")),
            ("../", Some("http://a/b/")),
            ("../g", Some("http://a/b/g")),
            ("../..", Some("http://a/")),
            ("../../", Some("http://a/")),
            ("../../g", Some("http://a/g")),
            ("../../../g", Some("http://a/g")),
            ("../../../../g", Some("http://a/g")),
            ("/./g", Some("http://a/g")),
            ("/../g", Some("http://a/g")),
            ("g.", Some("http://a/b/c/g.")),
            (".g", Some("http://a/b/c/.g")),
            ("g..", Some("http://a/b/c/g..")),
            ("..g", Some("http://a/b/c/..g")),
            ("./../g", Some("http://a/b/g")),
            ("./g/.", Some("http://a/b/c/g/")),
            ("g/./h", Some("http://a/b/c/g/h")),
            ("g/../h", Some("http://a/b/c/h")),
            ("g;x=1/./y", Some("http://a/b/c/g;x=1/y")),
            ("g;x=1/../y", Some("http://a/b/c/y")),
            ("g?y/./x", Some("http://a/b/c/g?y/./x")),
            ("g?y/../x", Some("http://a/b/c/g?y/../x")),
            ("g#s/./x", Some("http://a/b/c/g")),
            ("g#s/../x", Some("http://a/b/c/g")),
            ("http://bad host/", None),
        ] {
            assert_eq!(
                resolve_location(base, location).as_deref(),
                expected,
                "{location:?}"
            );
        }
    }

    #[test]
    fn a_parent_reference_does_not_leave_dot_segments_in_the_request() {
        let resolved = resolve_location("http://127.0.0.1:9/a/start", "../final");
        assert_eq!(resolved.as_deref(), Some("http://127.0.0.1:9/final"));
        // The scheme, the port and the page's own query are kept where the location leaves them.
        let base = "https://api.example.com:8443/v1/items?page=2";
        assert_eq!(
            resolve_location(base, "next").as_deref(),
            Some("https://api.example.com:8443/v1/next")
        );
        assert_eq!(
            resolve_location(base, "?page=3").as_deref(),
            Some("https://api.example.com:8443/v1/items?page=3")
        );
        assert_eq!(
            resolve_location(base, "//cdn.example/x/../y").as_deref(),
            Some("https://cdn.example/y")
        );
        assert_eq!(
            resolve_location("http://a", "g").as_deref(),
            Some("http://a/g")
        );
    }
}
