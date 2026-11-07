//! The key that seals the MCP trust state (`mcp::seal`), kept in the Keychain so that editing
//! the SQLite file is not enough to add a credential. It is created on first use from the OS
//! random source and never leaves this process.
//!
//! Under `DEGENVIEW_NO_KEYCHAIN` (and in tests) the host does not seal at all, because a key that
//! lived in memory would be new on every launch and lock the user out of their own credentials.
//! On Linux without a Secret Service the key cannot be created, and the host runs unsealed.

use crate::mcp::seal::{KEY_BYTES, hex, key_from_hex};
use crate::service::keychain_api_keys::{KeyBackend, default_backend};
use crate::service::keychain_webhook_secrets::Scope;
use crate::service::webhook_delivery::SecretStoreError;

const SCOPE: Scope = Scope {
    service: "es.nico.degenview.mcp",
    shared_group: false,
};
const ACCOUNT: &str = "seal-key-v1";

/// The seal key. Not `Debug`/`Clone`-printable: it must not reach a log.
#[derive(Clone, PartialEq, Eq)]
pub struct SealKey(pub [u8; KEY_BYTES]);

impl std::fmt::Debug for SealKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealKey(…)")
    }
}

/// What the Keychain holds for the seal.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyState {
    Ready(SealKey),
    /// No item: the seal has never been set up on this Mac (or its key was deleted).
    Missing,
    /// An item exists but could not be read or decoded (access denied, malformed value).
    Unreadable,
}

pub struct SealKeyStore {
    backend: Box<dyn KeyBackend>,
}

impl SealKeyStore {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self { backend }
    }

    /// The store on the platform's real secret storage.
    pub fn system() -> Self {
        Self::with_backend(default_backend())
    }

    /// Reads the key. The first read after a launch may raise a Keychain prompt.
    pub fn load(&self) -> KeyState {
        match self.backend.read(SCOPE, ACCOUNT) {
            Some(text) => key_from_hex(&text)
                .map_or(KeyState::Unreadable, |key| KeyState::Ready(SealKey(key))),
            None if self.backend.exists(SCOPE, ACCOUNT) => KeyState::Unreadable,
            None => KeyState::Missing,
        }
    }

    /// Makes a new random key and saves it. Never replaces an item that is already there.
    pub fn create(&self) -> Result<SealKey, SecretStoreError> {
        let mut key = [0u8; KEY_BYTES];
        getrandom::fill(&mut key).map_err(|error| SecretStoreError(error.to_string()))?;
        if self.backend.exists(SCOPE, ACCOUNT) {
            return Err(SecretStoreError(
                "the MCP seal key already exists".to_owned(),
            ));
        }
        self.backend.write(SCOPE, ACCOUNT, &hex(&key))?;
        Ok(SealKey(key))
    }

    /// Removes the key (the user chose to reset MCP access).
    pub fn delete(&self) {
        self.backend.delete(SCOPE, ACCOUNT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain_api_keys::MemoryBackend;

    fn store() -> SealKeyStore {
        SealKeyStore::with_backend(Box::new(MemoryBackend::default()))
    }

    #[test]
    fn a_key_is_created_once_and_read_back() {
        let store = store();
        assert_eq!(store.load(), KeyState::Missing);
        let key = store.create().unwrap();
        assert_eq!(store.load(), KeyState::Ready(key.clone()));
        assert!(store.create().is_err(), "an existing key is never replaced");
        assert_eq!(store.load(), KeyState::Ready(key));
    }

    #[test]
    fn keys_are_random() {
        let (a, b) = (store(), store());
        assert_ne!(a.create().unwrap(), b.create().unwrap());
    }

    #[test]
    fn a_malformed_item_is_unreadable_not_missing() {
        let backend = MemoryBackend::default();
        backend.write(SCOPE, ACCOUNT, "not a key").unwrap();
        let store = SealKeyStore::with_backend(Box::new(backend));
        assert_eq!(store.load(), KeyState::Unreadable);
    }

    #[test]
    fn delete_forgets_the_key() {
        let store = store();
        store.create().unwrap();
        store.delete();
        assert_eq!(store.load(), KeyState::Missing);
    }

    #[test]
    fn debug_output_hides_the_key() {
        assert_eq!(format!("{:?}", SealKey([1; KEY_BYTES])), "SealKey(…)");
    }
}
