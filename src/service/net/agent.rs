//! An HTTP agent that follows the proxy setting. A long-lived `ureq::Agent` keeps the proxy it
//! was built with, so one held in a struct or a static would ignore a change in Settings;
//! this one is rebuilt, with its own options, the first time it is asked for after a change, and
//! hands out the agent that suits the request's host.

use super::proxy;
use crate::lock::lock;
use std::sync::Mutex;
use ureq::config::ConfigBuilder;
use ureq::typestate::AgentScope;

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
    /// connection pool.
    pub fn get_for(&self, url: &str) -> ureq::Agent {
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
                let config = (self.configure)(ureq::Agent::config_builder().proxy(proxy)).build();
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
}
