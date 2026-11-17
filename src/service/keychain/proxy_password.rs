//! The manual proxy's password, kept in the system's secure storage and never in the database.
//! It is read only when the proxy is applied and a user name is set, so a user who has no
//! proxy password is never prompted. Under `DEGENVIEW_NO_KEYCHAIN` it lives in memory for the
//! session.

use crate::lock::lock;
use crate::service::keychain::api_keys::{KeyBackend, default_backend};
use crate::service::keychain::cached_secret::CachedSecret;
use crate::service::keychain::webhook_secrets::Scope;
use crate::service::webhook::delivery::SecretStoreError;
use std::sync::{Mutex, OnceLock};

const SCOPE: Scope = Scope {
    service: "es.nico.degenview.network",
    shared_group: false,
};
const ACCOUNT: &str = "proxy-password-v1";

pub struct ProxyPasswordStore {
    backend: Box<dyn KeyBackend>,
    secret: Mutex<CachedSecret>,
}

impl ProxyPasswordStore {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self {
            backend,
            secret: Mutex::new(CachedSecret::new(SCOPE, ACCOUNT)),
        }
    }

    pub fn shared() -> &'static Self {
        static STORE: OnceLock<ProxyPasswordStore> = OnceLock::new();
        STORE.get_or_init(|| Self::with_backend(default_backend()))
    }

    /// The saved password. The first read may prompt for Keychain access.
    pub fn read(&self) -> Option<String> {
        lock(&self.secret).read(&*self.backend)
    }

    /// The saved password, asked of the storage again: another process (the app, for the alert
    /// helper) may have changed it since this one read it.
    pub fn read_fresh(&self) -> Option<String> {
        lock(&self.secret).read_fresh(&*self.backend)
    }

    /// Saves the password as typed (spaces are part of a password); an empty one removes it. If
    /// the storage refuses, the password is still kept for this session and the error says it
    /// was not saved for good.
    pub fn save(&self, password: &str) -> Result<(), SecretStoreError> {
        if password.is_empty() {
            return self.remove();
        }
        lock(&self.secret).save(&*self.backend, password)
    }

    pub fn remove(&self) -> Result<(), SecretStoreError> {
        lock(&self.secret).remove(&*self.backend)
    }

    /// Whether one is saved, without reading it (so without a prompt); a page may ask every
    /// frame.
    pub fn is_saved(&self) -> bool {
        lock(&self.secret).is_saved(&*self.backend)
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
        store.save(" hunter2 ").unwrap();
        assert!(store.is_saved());
        assert_eq!(store.read().as_deref(), Some(" hunter2 "), "kept as typed");
        store.save("").unwrap();
        assert!(!store.is_saved());
        assert_eq!(store.read(), None);
    }
}
