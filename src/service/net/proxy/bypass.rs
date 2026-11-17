//! Which hosts never go through the proxy. Loopback, private networks and `.local` names always
//! connect directly: a proxy cannot reach them, and sending a LAN address to a remote proxy
//! leaks it. The user's own list adds to that.
//!
//! The expressions are the ones `ureq` matches (`example.com`, `.example.com`, `*.example.com`,
//! `192.168.*`, `*`), so the HTTP agent and the live-feed tunnel share one matcher.

/// Always direct, in `ureq`'s expressions.
const ALWAYS_DIRECT: &[&str] = &[
    "localhost",
    ".localhost",
    ".local",
    "127.*",
    "10.*",
    "192.168.*",
    "169.254.*",
    "172.16.*",
    "172.17.*",
    "172.18.*",
    "172.19.*",
    "172.20.*",
    "172.21.*",
    "172.22.*",
    "172.23.*",
    "172.24.*",
    "172.25.*",
    "172.26.*",
    "172.27.*",
    "172.28.*",
    "172.29.*",
    "172.30.*",
    "172.31.*",
    "[::1]",
    "[fe80:*",
    "[fc*",
    "[fd*",
];

/// Why an entry of the user's list cannot be used.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum BypassError {
    #[error("Address ranges such as {0} are not supported; list the hosts or a prefix like 10.1.*")]
    Range(String),
}

/// Splits a list written with commas, semicolons or spaces.
pub fn split(text: &str) -> Vec<String> {
    text.split([',', ';', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && *entry != "<local>")
        .map(str::to_owned)
        .collect()
}

/// The first entry that cannot be matched.
pub fn check(entries: &[String]) -> Result<(), BypassError> {
    match entries.iter().find(|entry| entry.contains('/')) {
        Some(entry) => Err(BypassError::Range(entry.clone())),
        None => Ok(()),
    }
}

/// The built-in hosts, then the user's. A bare domain also covers its subdomains (the way
/// `curl` reads `NO_PROXY`). Entries that cannot be matched are dropped.
pub fn entries(own: &[String]) -> Vec<String> {
    let mut all: Vec<String> = ALWAYS_DIRECT
        .iter()
        .map(|entry| (*entry).to_owned())
        .collect();
    for entry in own {
        if entry.contains('/') {
            continue;
        }
        let entry = entry.to_ascii_lowercase();
        if is_domain(&entry) {
            all.push(format!(".{entry}"));
        }
        all.push(entry);
    }
    all
}

fn is_domain(entry: &str) -> bool {
    !entry.starts_with(['.', '*', '['])
        && !entry.ends_with(['*', '.'])
        && entry.contains(|c: char| c.is_ascii_alphabetic())
        && entry.contains('.')
}
