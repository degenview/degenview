//! The proxy the operating system is set to use (the "system proxy" other apps follow): the
//! Internet Settings registry key on Windows, `scutil --proxy` on macOS. Linux has no single
//! system setting, so only the environment and the manual setting apply there.
//!
//! Reading it blocks (a process spawn on macOS), so it never runs on a frame: the agent reads
//! it when it is built or refreshed on a worker.

#[cfg(windows)]
mod registry;
#[cfg(any(test, target_os = "macos"))]
mod scutil;
#[cfg(any(test, windows))]
mod server_list;

use super::spec::Spec;

/// The system's proxy and its own bypass list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemProxy {
    pub spec: Spec,
    pub bypass: Vec<String>,
}

/// What the system says right now; `None` when it uses no proxy or this platform has no setting.
pub fn read() -> Option<SystemProxy> {
    #[cfg(windows)]
    {
        registry::read()
    }
    #[cfg(target_os = "macos")]
    {
        scutil::read()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}
