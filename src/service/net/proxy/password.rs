//! The manual proxy's password, kept in the system's secure storage and never in the database.
//! It is read only when the proxy is applied and a user name is set, so a user who has no
//! proxy password is never prompted. Under `DEGENVIEW_NO_KEYCHAIN` it lives in memory for the
//! session.

use crate::lock::lock;
use crate::service::keychain::api_keys::{KeyBackend, default_backend};
use crate::service::keychain::webhook_secrets::Scope;
use crate::service::webhook::delivery::SecretStoreError;
use std::sync::{Mutex, OnceLock};

const SCOPE: Scope = Scope {
    service: "es.nico.degenview.network",
    shared_group: false,
};
const ACCOUNT: &str = "proxy-password-v1";

#[derive(Default)]
struct Cache {
    /// The password read or saved this session; `Some(None)` is a confirmed absence.
    value: Option<Option<String>>,
    /// Whether one is saved, asked without reading it.
    exists: Option<bool>,
}

pub struct ProxyPasswordStore {
    backend: Box<dyn KeyBackend>,
    cache: Mutex<Cache>,
}

impl ProxyPasswordStore {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self {
            backend,
            cache: Mutex::new(Cache::default()),
        }
    }

    pub fn shared() -> &'static Self {
        static STORE: OnceLock<ProxyPasswordStore> = OnceLock::new();
        STORE.get_or_init(|| Self::with_backend(default_backend()))
    }

    /// The saved password. The first read may prompt for Keychain access.
    pub fn read(&self) -> Option<String> {
        lock(&self.cache)
            .value
            .get_or_insert_with(|| {
                self.backend
                    .read(SCOPE, ACCOUNT)
                    .filter(|password| !password.is_empty())
            })
            .clone()
    }

    /// The saved password, asked of the storage again: another process (the app, for the alert
    /// helper) may have changed it since this one read it.
    pub fn read_fresh(&self) -> Option<String> {
        let found = self
            .backend
            .read(SCOPE, ACCOUNT)
            .filter(|password| !password.is_empty());
        lock(&self.cache).value = Some(found.clone());
        found
    }

    /// Saves the password; an empty one removes it. If the storage refuses, the password is
    /// still kept for this session and the error says it was not saved for good.
    pub fn save(&self, password: &str) -> Result<(), SecretStoreError> {
        if password.is_empty() {
            return self.remove();
        }
        let written = self.backend.write(SCOPE, ACCOUNT, password);
        *lock(&self.cache) = Cache {
            value: Some(Some(password.to_owned())),
            exists: None,
        };
        written
    }

    pub fn remove(&self) -> Result<(), SecretStoreError> {
        let removed = self.backend.delete(SCOPE, ACCOUNT);
        *lock(&self.cache) = Cache {
            value: Some(None),
            exists: None,
        };
        removed
    }

    /// Whether one is saved. Asked of the storage once, without reading the password (so
    /// without a prompt); a page may call it every frame.
    pub fn is_saved(&self) -> bool {
        let mut cache = lock(&self.cache);
        if let Some(value) = &cache.value {
            return value.is_some();
        }
        *cache
            .exists
            .get_or_insert_with(|| self.backend.exists(SCOPE, ACCOUNT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain::api_keys::MemoryBackend;

    fn store() -> ProxyPasswordStore {
        ProxyPasswordStore::with_backend(Box::new(MemoryBackend::default()))
    }

    #[test]
    fn a_fresh_read_sees_what_another_process_wrote() {
        let store = store();
        assert_eq!(store.read(), None, "now cached as absent");
        store.backend.write(SCOPE, ACCOUNT, "from the app").unwrap();
        assert_eq!(store.read(), None, "the cache still says absent");
        assert_eq!(store.read_fresh().as_deref(), Some("from the app"));
        assert_eq!(store.read().as_deref(), Some("from the app"));
    }

    #[test]
    fn a_saved_password_is_read_back_and_an_empty_one_removes_it() {
        let store = store();
        assert!(!store.is_saved());
        store.save("hunter2").unwrap();
        assert!(store.is_saved());
        assert_eq!(store.read().as_deref(), Some("hunter2"));
        store.save("").unwrap();
        assert!(!store.is_saved());
        assert_eq!(store.read(), None);
    }
}
