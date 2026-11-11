//! The token a snapshot host asks for, kept in the Keychain beside the data providers' keys. It
//! is read only when an upload needs it, so a user who never shares a snapshot is never
//! prompted. Under `DEGENVIEW_NO_KEYCHAIN` (and on platforms with no Keychain) it lives in memory
//! for the session; `DEGENVIEW_SNAPSHOT_TOKEN` stands in when nothing is saved.

use crate::service::keychain_api_keys::{KeyBackend, default_backend};
use crate::service::keychain_webhook_secrets::Scope;
use crate::service::webhook_delivery::SecretStoreError;
use std::sync::{Mutex, OnceLock};

const SERVICE: &str = "es.nico.degenview.snapshots";
const ACCOUNT: &str = "host-token-v1";
const SCOPE: Scope = Scope {
    service: SERVICE,
    shared_group: false,
};

/// Where a developer or a machine with no Keychain gives the token.
pub const TOKEN_ENV: &str = "DEGENVIEW_SNAPSHOT_TOKEN";

#[derive(Default)]
struct Cache {
    loaded: bool,
    token: Option<String>,
    exists: Option<bool>,
}

pub struct SnapshotTokenStore {
    backend: Box<dyn KeyBackend>,
    cache: Mutex<Cache>,
}

impl SnapshotTokenStore {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self {
            backend,
            cache: Mutex::new(Cache::default()),
        }
    }

    pub fn shared() -> &'static Self {
        static STORE: OnceLock<SnapshotTokenStore> = OnceLock::new();
        STORE.get_or_init(|| Self::with_backend(default_backend()))
    }

    /// The saved token. The first read may prompt for Keychain access.
    pub fn token(&self) -> Option<String> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if !cache.loaded {
            cache.loaded = true;
            cache.token = self
                .backend
                .read(SCOPE, ACCOUNT)
                .filter(|token| !token.is_empty());
        }
        cache.token.clone()
    }

    /// Whether a token is saved, without reading it (so without a prompt).
    pub fn is_configured(&self) -> bool {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.loaded {
            return cache.token.is_some();
        }
        if let Some(exists) = cache.exists {
            return exists;
        }
        let found = self.backend.exists(SCOPE, ACCOUNT);
        cache.exists = Some(found);
        found
    }

    /// Saves the trimmed token; an empty one removes it.
    pub fn save(&self, token: &str) -> Result<(), SecretStoreError> {
        let value = token.trim();
        if value.is_empty() {
            self.remove();
            return Ok(());
        }
        self.backend.write(SCOPE, ACCOUNT, value)?;
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Cache {
            loaded: true,
            token: Some(value.to_owned()),
            exists: None,
        };
        Ok(())
    }

    pub fn remove(&self) {
        // Best effort: the token is also forgotten here, and a new one replaces it on the next share.
        let _ = self.backend.delete(SCOPE, ACCOUNT);
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Cache {
            loaded: true,
            token: None,
            exists: None,
        };
    }
}

/// The token to send: the saved one, else the environment's.
pub fn token_with(
    store: &SnapshotTokenStore,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let clean = |text: Option<String>| {
        text.map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    if store.is_configured()
        && let Some(token) = clean(store.token())
    {
        return Some(token);
    }
    clean(env(TOKEN_ENV))
}

pub fn token() -> Option<String> {
    token_with(SnapshotTokenStore::shared(), &|name| {
        std::env::var(name).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain_api_keys::MemoryBackend;

    fn store() -> SnapshotTokenStore {
        SnapshotTokenStore::with_backend(Box::new(MemoryBackend::default()))
    }

    #[test]
    fn a_saved_token_is_trimmed_and_read_back() {
        let store = store();
        assert!(!store.is_configured());
        store.save("  abc123  ").unwrap();
        assert!(store.is_configured());
        assert_eq!(store.token().as_deref(), Some("abc123"));
    }

    #[test]
    fn saving_nothing_removes_it() {
        let store = store();
        store.save("abc").unwrap();
        store.save("   ").unwrap();
        assert!(!store.is_configured());
        assert_eq!(store.token(), None);
    }

    #[test]
    fn the_environment_stands_in_only_when_nothing_is_saved() {
        let store = store();
        let env = |name: &str| (name == TOKEN_ENV).then(|| " from-env ".to_owned());
        assert_eq!(token_with(&store, &env).as_deref(), Some("from-env"));
        store.save("saved").unwrap();
        assert_eq!(token_with(&store, &env).as_deref(), Some("saved"));
        store.remove();
        assert_eq!(token_with(&store, &env).as_deref(), Some("from-env"));
        assert_eq!(token_with(&store, &|_| None), None);
    }
}
