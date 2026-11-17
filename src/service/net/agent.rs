//! An HTTP agent that follows the proxy setting. A long-lived `ureq::Agent` keeps the proxy it
//! was built with, so one held in a struct or a static would ignore a change in Settings;
//! this one is rebuilt, with its own options, the first time it is asked for after a change.

use super::proxy;
use crate::lock::lock;
use std::sync::Mutex;
use ureq::config::ConfigBuilder;
use ureq::typestate::AgentScope;

type Configure = Box<dyn Fn(ConfigBuilder<AgentScope>) -> ConfigBuilder<AgentScope> + Send + Sync>;

pub struct FollowingAgent {
    configure: Configure,
    /// The agent and the proxy generation it was built for.
    cached: Mutex<Option<(u64, ureq::Agent)>>,
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

    /// The agent for the proxy now in force. Cloning an agent shares its connection pool.
    pub fn get(&self) -> ureq::Agent {
        let generation = proxy::snapshot().generation;
        let mut cached = lock(&self.cached);
        match &*cached {
            Some((built_for, agent)) if *built_for == generation => agent.clone(),
            _ => {
                let config = (self.configure)(proxy::config_builder()).build();
                let agent = ureq::Agent::new_with_config(config);
                *cached = Some((generation, agent.clone()));
                agent
            }
        }
    }
}
