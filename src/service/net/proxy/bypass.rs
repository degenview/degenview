//! Which hosts never go through the proxy. Loopback, private networks and `.local` names always
//! connect directly: a proxy cannot reach them, and sending a LAN address to a remote proxy
//! leaks it. The user's own list adds to that.
//!
//! The built-in rule is [`is_local`], which looks at what the host *is*: an address in a private
//! range, or a local name. It does not match on the spelling of the host, so `10.example.com` (a
//! public name) goes through the proxy like any other. The user's entries are `ureq`'s
//! expressions (`example.com`, `.example.com`, `*.example.com`, `192.168.*`, `*`), which the HTTP
//! agent and the live-feed tunnel share.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Local names in `ureq`'s expressions. Only spellings that cannot belong to a public host; they
/// back [`is_local`] up for a redirect, where the agent decides on its own.
const ALWAYS_DIRECT: &[&str] = &["localhost", ".localhost", ".local", "[::1]"];

/// Whether `host` (a name, an address, or an address in brackets) is on this machine or its
/// local network, and so is never reached through a proxy.
pub fn is_local(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(&host);
    // A zone (`fe80::1%en0`) says which interface; the address is what counts.
    let address = host.split('%').next().unwrap_or(host);
    match address.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => local_v4(address),
        Ok(IpAddr::V6(address)) => local_v6(address),
        Err(_) => host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local"),
    }
}

fn local_v4(address: Ipv4Addr) -> bool {
    // 127/8, 10/8, 172.16/12, 192.168/16, 169.254/16, 0.0.0.0.
    address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_unspecified()
}

fn local_v6(address: Ipv6Addr) -> bool {
    let first = address.segments()[0];
    address.is_loopback()
        || address.is_unspecified()
        || first & 0xffc0 == 0xfe80 // link-local, fe80::/10
        || first & 0xfe00 == 0xfc00 // unique local, fc00::/7
        || address.to_ipv4_mapped().is_some_and(local_v4)
}

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
