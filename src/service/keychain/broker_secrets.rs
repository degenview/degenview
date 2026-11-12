//! Broker credentials (a trading API key and its private key), kept in the operating system's
//! secret store: the Keychain, the Secret Service or Credential Manager, through the same backend
//! as the other API keys.
//!
//! These are the most sensitive secrets the app holds: they can place orders with real money. So:
//!
//! * they are stored in the **app-only** group (`shared_group: false`), never the one the
//!   background alert agent can read;
//! * they never go to SQLite, a file, iCloud Sync or a log, and nothing here prints one;
//! * a credential is read when a connection needs it, not at startup, so a user who never connects
//!   a broker is never prompted;
//! * under `DEGENVIEW_NO_KEYCHAIN` (tests, development) they live in memory for the session.
//!
//! Implements the broker crate's `SecretStore`, so the crate never decides where a secret lives.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use degenview_brokers::{BrokerId, Credentials, SecretStore, SecretStoreError};

use crate::service::keychain::api_keys::{KeyBackend, default_backend};
use crate::service::keychain::webhook_secrets::Scope;

const SERVICE: &str = "es.nico.degenview.broker";
const SCOPE: Scope = Scope {
    service: SERVICE,
    shared_group: false,
};

fn account(broker: BrokerId, profile: &str) -> String {
    format!("{}-{profile}-v1", broker.as_str())
}

#[derive(Default)]
struct Cache {
    /// What was read or saved this session; `None` is a confirmed absence.
    credentials: HashMap<String, Option<Credentials>>,
    exists: HashMap<String, bool>,
}

pub struct BrokerSecrets {
    backend: Box<dyn KeyBackend>,
    cache: Mutex<Cache>,
    revision: AtomicU64,
}

impl BrokerSecrets {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self {
            backend,
            cache: Mutex::new(Cache::default()),
            revision: AtomicU64::new(0),
        }
    }

    pub fn shared() -> &'static Self {
        static STORE: OnceLock<BrokerSecrets> = OnceLock::new();
        STORE.get_or_init(|| Self::with_backend(default_backend()))
    }

    /// Bumped on every save or removal.
    #[allow(dead_code)] // Settings polls it once a page shows saved state.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl SecretStore for BrokerSecrets {
    fn load(
        &self,
        broker: BrokerId,
        profile: &str,
    ) -> Result<Option<Credentials>, SecretStoreError> {
        let key = account(broker, profile);
        let mut cache = self.lock();
        if let Some(known) = cache.credentials.get(&key) {
            return Ok(known.clone());
        }
        let read = self
            .backend
            .read(SCOPE, &key)
            .and_then(|text| Credentials::from_json(&text))
            .filter(|credentials| !credentials.is_empty());
        cache.credentials.insert(key, read.clone());
        Ok(read)
    }

    /// Saves the credentials. If the secret store refuses (or there is none), they are still kept
    /// for this session and the error says they were not saved for good.
    fn save(
        &self,
        broker: BrokerId,
        profile: &str,
        credentials: &Credentials,
    ) -> Result<(), SecretStoreError> {
        if credentials.is_empty() {
            return self.delete(broker, profile);
        }
        let key = account(broker, profile);
        let written = self
            .backend
            .write(SCOPE, &key, &credentials.to_json())
            .map_err(|error| SecretStoreError(error.0));
        let mut cache = self.lock();
        cache
            .credentials
            .insert(key.clone(), Some(credentials.clone()));
        cache.exists.remove(&key);
        drop(cache);
        self.revision.fetch_add(1, Ordering::SeqCst);
        written
    }

    fn delete(&self, broker: BrokerId, profile: &str) -> Result<(), SecretStoreError> {
        let key = account(broker, profile);
        // Forgotten in memory only once the Keychain has let go of it: a secret that could not be
        // removed is still stored, comes back at the next start, and must not be reported as gone.
        self.backend
            .delete(SCOPE, &key)
            .map_err(|error| SecretStoreError(error.0))?;
        let mut cache = self.lock();
        cache.credentials.insert(key.clone(), None);
        cache.exists.remove(&key);
        drop(cache);
        self.revision.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Whether credentials are saved, without reading them (so without a prompt).
    fn is_configured(&self, broker: BrokerId, profile: &str) -> bool {
        let key = account(broker, profile);
        let mut cache = self.lock();
        if let Some(known) = cache.credentials.get(&key) {
            return known.is_some();
        }
        if let Some(exists) = cache.exists.get(&key) {
            return *exists;
        }
        let found = self.backend.exists(SCOPE, &key);
        cache.exists.insert(key, found);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain::api_keys::MemoryBackend;

    const ID: BrokerId = BrokerId::new("acme");

    fn store() -> BrokerSecrets {
        BrokerSecrets::with_backend(Box::new(MemoryBackend::default()))
    }

    fn credentials() -> Credentials {
        Credentials::new()
            .with("name", "organizations/o/apiKeys/k")
            .with("private_key", "-----BEGIN EC PRIVATE KEY-----\nAAAA\n")
    }

    #[test]
    fn credentials_round_trip_and_removal_is_final() {
        let store = store();
        assert!(!store.is_configured(ID, "default"));
        assert!(store.load(ID, "default").unwrap().is_none());
        store.save(ID, "default", &credentials()).unwrap();
        assert!(store.is_configured(ID, "default"));
        let loaded = store.load(ID, "default").unwrap().unwrap();
        assert_eq!(loaded.get("name"), Some("organizations/o/apiKeys/k"));
        store.delete(ID, "default").unwrap();
        assert!(!store.is_configured(ID, "default"));
        assert!(store.load(ID, "default").unwrap().is_none());
    }

    #[test]
    fn saving_blank_credentials_removes_them() {
        let store = store();
        store.save(ID, "default", &credentials()).unwrap();
        store
            .save(ID, "default", &Credentials::new().with("name", "  "))
            .unwrap();
        assert!(!store.is_configured(ID, "default"));
    }

    #[test]
    fn profiles_and_brokers_are_kept_apart() {
        let store = store();
        store.save(ID, "default", &credentials()).unwrap();
        assert!(!store.is_configured(ID, "other"));
        assert!(!store.is_configured(BrokerId::new("other"), "default"));
    }

    #[test]
    fn every_change_moves_the_revision() {
        let store = store();
        let start = store.revision();
        store.save(ID, "default", &credentials()).unwrap();
        store.delete(ID, "default").unwrap();
        assert_eq!(store.revision(), start + 2);
    }

    use crate::service::webhook::delivery::SecretStoreError as BackendError;

    /// A backend whose removals the system refuses.
    struct Stubborn(MemoryBackend);

    impl KeyBackend for Stubborn {
        fn read(&self, scope: Scope, account: &str) -> Option<String> {
            self.0.read(scope, account)
        }
        fn exists(&self, scope: Scope, account: &str) -> bool {
            self.0.exists(scope, account)
        }
        fn write(&self, scope: Scope, account: &str, value: &str) -> Result<(), BackendError> {
            self.0.write(scope, account, value)
        }
        fn delete(&self, _: Scope, _: &str) -> Result<(), BackendError> {
            Err(BackendError("Keychain error -25293".to_owned()))
        }
    }

    /// A key the system would not let go of is still stored: it is an error, and nothing in memory
    /// pretends otherwise, so a restart cannot bring back something that was reported as gone.
    #[test]
    fn a_removal_the_system_refused_is_an_error_and_forgets_nothing() {
        let store = BrokerSecrets::with_backend(Box::new(Stubborn(MemoryBackend::default())));
        store.save(ID, "default", &credentials()).unwrap();
        let start = store.revision();
        let error = store.delete(ID, "default").unwrap_err();
        assert!(error.0.contains("-25293"), "{}", error.0);
        assert!(store.is_configured(ID, "default"), "still stored");
        assert!(store.load(ID, "default").unwrap().is_some());
        assert_eq!(store.revision(), start, "nothing changed");
    }

    /// Removing what is not there is not a failure.
    #[test]
    fn removing_a_key_that_is_not_stored_succeeds() {
        let store = store();
        store.delete(ID, "default").unwrap();
        assert!(!store.is_configured(ID, "default"));
    }

    /// The items live in the app-only group, never the one the alert agent can read.
    #[test]
    fn the_scope_is_not_shared_with_the_background_agent() {
        const { assert!(!SCOPE.shared_group) };
        assert_eq!(SCOPE.service, "es.nico.degenview.broker");
    }

    /// A corrupt item reads as absent rather than as credentials.
    #[test]
    fn an_unreadable_item_is_treated_as_missing() {
        let backend = MemoryBackend::default();
        backend
            .write(SCOPE, &account(ID, "default"), "not json")
            .unwrap();
        let store = BrokerSecrets::with_backend(Box::new(backend));
        assert!(store.load(ID, "default").unwrap().is_none());
    }
}
