//! A keyed seal over everything that decides who may use the MCP server: the server settings,
//! the global permission cap and each client's identity, verifier and grants.
//!
//! A client's verifier is a plain SHA-256 of its token, so anyone who can write the SQLite file
//! can invent a token, store its hash, and be let in. The seal closes that: it is an
//! HMAC-SHA-256 under a key that lives only in the Keychain (`keychain::mcp_seal`), checked when
//! the host loads. A row or setting the app did not write no longer matches, and the server stays
//! off. The key is never in the database, so editing the file alone cannot produce a valid seal.
//!
//! What the seal does not cover: `last_used_ms` (bookkeeping the app rewrites every half minute),
//! and a replay of an older database copy together with its own seal. Pure values only.

use crate::mcp::transport::auth::verifiers_match;
use crate::model::mcp::{
    BindMode, Cidr, McpClient, McpSettings, PermissionSet, default_global_permissions,
    default_remote_ceiling,
};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;

/// Bytes of the Keychain key.
pub const KEY_BYTES: usize = 32;

/// Separates this MAC from any other use of the same key, and versions the encoding below.
const DOMAIN: &[u8] = b"degenview.mcp.seal.v1\n";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SealedClient<'a> {
    id: &'a str,
    name: &'a str,
    verifier: &'a str,
    permissions: &'a PermissionSet,
    created_at_ms: i64,
    /// Absent while false/empty, so a seal made before remote access existed still verifies.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    remote_allowed: bool,
    #[serde(skip_serializing_if = "<[Cidr]>::is_empty")]
    allowed_cidrs: &'a [Cidr],
}

#[derive(Serialize)]
struct Sealed<'a> {
    enabled: bool,
    port: u16,
    /// Absent while loopback, so a seal made before remote access existed still verifies.
    #[serde(skip_serializing_if = "BindMode::is_loopback")]
    bind: BindMode,
    /// Absent while it is the default, for the same reason.
    #[serde(skip_serializing_if = "is_default_ceiling")]
    remote_ceiling: &'a PermissionSet,
    require_confirmation: bool,
    global: &'a PermissionSet,
    clients: Vec<SealedClient<'a>>,
}

fn is_default_ceiling(ceiling: &&PermissionSet) -> bool {
    **ceiling == default_remote_ceiling()
}

/// Lowercase hex HMAC-SHA-256 of the trust state, in the order it is stored.
pub fn seal(
    key: &[u8; KEY_BYTES],
    settings: &McpSettings,
    global: &PermissionSet,
    clients: &[McpClient],
) -> String {
    let sealed = Sealed {
        enabled: settings.enabled,
        port: settings.port,
        bind: settings.bind,
        remote_ceiling: &settings.remote_ceiling,
        require_confirmation: settings.require_confirmation,
        global,
        clients: clients
            .iter()
            .map(|client| SealedClient {
                id: &client.id,
                name: &client.name,
                verifier: &client.verifier,
                permissions: &client.permissions,
                created_at_ms: client.created_at_ms,
                remote_allowed: client.remote_allowed,
                allowed_cidrs: &client.allowed_cidrs,
            })
            .collect(),
    };
    // Plain structs of strings, numbers and sets: serialising them cannot fail.
    let encoded = serde_json::to_vec(&sealed).unwrap_or_default();
    // HMAC accepts a key of any length.
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(DOMAIN);
    mac.update(&encoded);
    hex(&mac.finalize().into_bytes())
}

/// Whether `stored` is the seal of this state under `key`, compared in constant time.
pub fn verify(
    key: &[u8; KEY_BYTES],
    settings: &McpSettings,
    global: &PermissionSet,
    clients: &[McpClient],
    stored: &str,
) -> bool {
    verifiers_match(&seal(key, settings, global, clients), stored)
}

/// Nothing has ever been configured: no credentials, server off at its default port, default
/// cap. Such a state grants nobody anything, so it needs no seal to be trusted.
pub fn is_pristine(settings: &McpSettings, global: &PermissionSet, clients: &[McpClient]) -> bool {
    clients.is_empty()
        && *settings == McpSettings::default()
        && *global == default_global_permissions()
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// The inverse of [`hex`] for a key; `None` unless it is exactly [`KEY_BYTES`] bytes.
pub fn key_from_hex(text: &str) -> Option<[u8; KEY_BYTES]> {
    let text = text.trim();
    if text.len() != KEY_BYTES * 2 || !text.is_ascii() {
        return None;
    }
    let mut key = [0u8; KEY_BYTES];
    for (byte, pair) in key.iter_mut().zip(text.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::transport::auth::verifier_hex;
    use crate::model::mcp::Permission;

    const KEY: [u8; KEY_BYTES] = [7; KEY_BYTES];

    fn client(id: &str, token: &str) -> McpClient {
        McpClient {
            id: id.into(),
            name: format!("client {id}"),
            verifier: verifier_hex(token),
            permissions: PermissionSet::of([Permission::ChartsRead]),
            created_at_ms: 5,
            last_used_ms: None,
            remote_allowed: false,
            allowed_cidrs: Vec::new(),
        }
    }

    fn state() -> (McpSettings, PermissionSet, Vec<McpClient>) {
        (
            McpSettings {
                enabled: true,
                port: 9001,
                ..McpSettings::default()
            },
            default_global_permissions(),
            vec![client("a", "token-a"), client("b", "token-b")],
        )
    }

    #[test]
    fn a_state_verifies_under_its_own_key_only() {
        let (settings, global, clients) = state();
        let stored = seal(&KEY, &settings, &global, &clients);
        assert_eq!(stored.len(), 64);
        assert!(verify(&KEY, &settings, &global, &clients, &stored));
        assert!(!verify(
            &[8; KEY_BYTES],
            &settings,
            &global,
            &clients,
            &stored
        ));
        assert!(!verify(&KEY, &settings, &global, &clients, ""));
    }

    #[test]
    fn loopback_leaves_no_trace_in_the_sealed_bytes() {
        // A seal written before `bind` existed must keep verifying: the loopback encoding has to
        // be byte-for-byte the old one, so a field is added only when it is not the default.
        let sealed = |bind: BindMode| {
            serde_json::to_string(&Sealed {
                enabled: true,
                port: 9001,
                bind,
                remote_ceiling: &default_remote_ceiling(),
                require_confirmation: true,
                global: &default_global_permissions(),
                clients: Vec::new(),
            })
            .unwrap()
        };
        assert!(!sealed(BindMode::Loopback).contains("bind"));
        assert!(!sealed(BindMode::Loopback).contains("ceiling"));
        // Nor does a client that has no remote access.
        let plain = serde_json::to_string(&SealedClient {
            id: "a",
            name: "a",
            verifier: "v",
            permissions: &PermissionSet::none(),
            created_at_ms: 1,
            remote_allowed: false,
            allowed_cidrs: &[],
        })
        .unwrap();
        assert!(
            !plain.contains("remote") && !plain.contains("llowed"),
            "{plain}"
        );
        let lan = BindMode::interface("10.0.0.7".parse().unwrap()).unwrap();
        assert!(sealed(lan).contains(r#""bind":"10.0.0.7""#));
    }

    #[test]
    fn a_forged_client_row_does_not_verify() {
        // What an attacker who can write the file but has no key can do: pick a token, store
        // its SHA-256 as a verifier with every permission.
        let (settings, global, mut clients) = state();
        let stored = seal(&KEY, &settings, &global, &clients);
        let mut forged = client("evil", "attacker-token");
        forged.permissions = PermissionSet::everything();
        clients.push(forged);
        assert!(!verify(&KEY, &settings, &global, &clients, &stored));
    }

    #[test]
    fn every_trust_field_is_covered() {
        let (settings, global, clients) = state();
        let stored = seal(&KEY, &settings, &global, &clients);
        let changed = |settings: &McpSettings, global: &PermissionSet, clients: &[McpClient]| {
            !verify(&KEY, settings, global, clients, &stored)
        };

        assert!(changed(
            &McpSettings {
                enabled: false,
                ..settings.clone()
            },
            &global,
            &clients
        ));
        assert!(changed(
            &McpSettings {
                port: 9002,
                ..settings.clone()
            },
            &global,
            &clients
        ));
        assert!(changed(
            &McpSettings {
                require_confirmation: !settings.require_confirmation,
                ..settings.clone()
            },
            &global,
            &clients
        ));
        assert!(changed(
            &McpSettings {
                bind: BindMode::interface("192.168.1.20".parse().unwrap()).unwrap(),
                ..settings.clone()
            },
            &global,
            &clients
        ));
        assert!(changed(
            &McpSettings {
                remote_ceiling: PermissionSet::everything(),
                ..settings.clone()
            },
            &global,
            &clients
        ));
        assert!(changed(&settings, &PermissionSet::everything(), &clients));

        let edit = |edit: &dyn Fn(&mut McpClient)| {
            let mut clients = clients.clone();
            edit(&mut clients[0]);
            clients
        };
        assert!(changed(&settings, &global, &edit(&|c| c.id = "z".into())));
        assert!(changed(&settings, &global, &edit(&|c| c.name = "z".into())));
        assert!(changed(
            &settings,
            &global,
            &edit(&|c| c.verifier = verifier_hex("other"))
        ));
        assert!(changed(
            &settings,
            &global,
            &edit(&|c| c.permissions = PermissionSet::everything())
        ));
        assert!(changed(&settings, &global, &edit(&|c| c.created_at_ms = 6)));
        assert!(changed(
            &settings,
            &global,
            &edit(&|c| c.remote_allowed = true)
        ));
        assert!(changed(
            &settings,
            &global,
            &edit(&|c| c.allowed_cidrs = vec![Cidr::parse("10.0.0.0/8").unwrap()])
        ));

        // Order and removal count too.
        let mut reversed = clients.clone();
        reversed.reverse();
        assert!(changed(&settings, &global, &reversed));
        assert!(changed(&settings, &global, &clients[..1]));
    }

    #[test]
    fn last_use_is_bookkeeping_and_does_not_break_the_seal() {
        let (settings, global, mut clients) = state();
        let stored = seal(&KEY, &settings, &global, &clients);
        clients[1].last_used_ms = Some(123_456);
        assert!(verify(&KEY, &settings, &global, &clients, &stored));
    }

    #[test]
    fn pristine_means_nothing_configured() {
        let (settings, global, clients) = state();
        assert!(is_pristine(
            &McpSettings::default(),
            &default_global_permissions(),
            &[]
        ));
        assert!(!is_pristine(&settings, &default_global_permissions(), &[]));
        assert!(!is_pristine(
            &McpSettings::default(),
            &PermissionSet::everything(),
            &[]
        ));
        assert!(!is_pristine(&McpSettings::default(), &global, &clients));
    }

    #[test]
    fn keys_round_trip_through_hex() {
        let key = [0xAB; KEY_BYTES];
        assert_eq!(key_from_hex(&hex(&key)), Some(key));
        assert_eq!(key_from_hex(&format!("  {}\n", hex(&key))), Some(key));
        assert_eq!(key_from_hex(""), None);
        assert_eq!(key_from_hex(&"zz".repeat(KEY_BYTES)), None);
        assert_eq!(key_from_hex(&"ab".repeat(KEY_BYTES - 1)), None);
        assert_eq!(key_from_hex(&"é".repeat(KEY_BYTES)), None);
    }
}
