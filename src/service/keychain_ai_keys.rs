//! The API key for each AI provider, kept in the Keychain beside the data providers' keys. A key
//! is read only when a request needs it, so a user who never opens the assistant is never
//! prompted. Under `DEGENVIEW_NO_KEYCHAIN` the keys live in memory for the session. On a platform
//! with no Keychain a save reports the failure, but the key is still kept for the session so the
//! assistant works until the app quits.

// Transient: the Script Manager assistant reads keys through `key` next.
#![allow(dead_code)]

use crate::service::keychain_api_keys::{KeyBackend, default_backend};
use crate::service::keychain_webhook_secrets::Scope;
use crate::service::llm::ProviderKind;
use crate::service::webhook_delivery::SecretStoreError;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const SERVICE: &str = "es.nico.degenview.ai";
const SCOPE: Scope = Scope {
    service: SERVICE,
    shared_group: false,
};

fn account(kind: ProviderKind) -> String {
    format!("{}-key-v1", kind.key())
}

#[derive(Default)]
struct Cache {
    /// Keys read or saved this session; `None` is a confirmed absence.
    keys: HashMap<ProviderKind, Option<String>>,
    exists: HashMap<ProviderKind, bool>,
}

pub struct AiKeyStore {
    backend: Box<dyn KeyBackend>,
    cache: Mutex<Cache>,
    revision: AtomicU64,
}

impl AiKeyStore {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self {
            backend,
            cache: Mutex::new(Cache::default()),
            revision: AtomicU64::new(0),
        }
    }

    pub fn shared() -> &'static Self {
        static STORE: OnceLock<AiKeyStore> = OnceLock::new();
        STORE.get_or_init(|| Self::with_backend(default_backend()))
    }

    /// Bumped on every save or removal.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The saved key. The first read may prompt for Keychain access.
    pub fn key(&self, kind: ProviderKind) -> Option<String> {
        let mut cache = self.lock();
        cache
            .keys
            .entry(kind)
            .or_insert_with(|| {
                self.backend
                    .read(SCOPE, &account(kind))
                    .filter(|key| !key.is_empty())
            })
            .clone()
    }

    /// Whether a key is saved, without reading it (so without a prompt).
    pub fn is_configured(&self, kind: ProviderKind) -> bool {
        let mut cache = self.lock();
        if let Some(key) = cache.keys.get(&kind) {
            return key.is_some();
        }
        if let Some(exists) = cache.exists.get(&kind) {
            return *exists;
        }
        let found = self.backend.exists(SCOPE, &account(kind));
        cache.exists.insert(kind, found);
        found
    }

    /// Saves the trimmed key; an empty one removes it. If the Keychain refuses (or there is none),
    /// the key is still kept for this session and the error says it was not saved for good.
    pub fn save(&self, kind: ProviderKind, key: &str) -> Result<(), SecretStoreError> {
        let value = key.trim();
        if value.is_empty() {
            self.remove(kind);
            return Ok(());
        }
        let written = self.backend.write(SCOPE, &account(kind), value);
        let mut cache = self.lock();
        cache.keys.insert(kind, Some(value.to_owned()));
        cache.exists.remove(&kind);
        drop(cache);
        self.revision.fetch_add(1, Ordering::SeqCst);
        written
    }

    pub fn remove(&self, kind: ProviderKind) {
        self.backend.delete(SCOPE, &account(kind));
        let mut cache = self.lock();
        cache.keys.insert(kind, None);
        cache.exists.remove(&kind);
        drop(cache);
        self.revision.fetch_add(1, Ordering::SeqCst);
    }
}

/// The key to send for `kind`: the saved one, else the environment's.
pub fn key_with(
    store: &AiKeyStore,
    kind: ProviderKind,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let clean = |text: Option<String>| {
        text.map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    if store.is_configured(kind)
        && let Some(key) = clean(store.key(kind))
    {
        return Some(key);
    }
    clean(env(kind.env_var()))
}

pub fn key(kind: ProviderKind) -> Option<String> {
    key_with(AiKeyStore::shared(), kind, &|name| std::env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain_api_keys::MemoryBackend;

    fn store() -> AiKeyStore {
        AiKeyStore::with_backend(Box::new(MemoryBackend::default()))
    }

    /// A backend with no Keychain behind it, as on Linux or Windows.
    struct Refusing;

    impl KeyBackend for Refusing {
        fn read(&self, _: Scope, _: &str) -> Option<String> {
            None
        }
        fn exists(&self, _: Scope, _: &str) -> bool {
            false
        }
        fn write(&self, _: Scope, _: &str, _: &str) -> Result<(), SecretStoreError> {
            Err(SecretStoreError("no Keychain on this platform".to_owned()))
        }
        fn delete(&self, _: Scope, _: &str) {}
    }

    #[test]
    fn each_provider_has_its_own_trimmed_key() {
        let store = store();
        store.save(ProviderKind::OpenAi, "  sk-a ").unwrap();
        store.save(ProviderKind::Anthropic, "sk-b").unwrap();
        assert_eq!(store.key(ProviderKind::OpenAi).as_deref(), Some("sk-a"));
        assert_eq!(store.key(ProviderKind::Anthropic).as_deref(), Some("sk-b"));
        assert!(store.is_configured(ProviderKind::OpenAi));
        assert!(!store.is_configured(ProviderKind::Compatible));
    }

    #[test]
    fn saving_nothing_removes_the_key_and_bumps_the_revision() {
        let store = store();
        let before = store.revision();
        store.save(ProviderKind::OpenAi, "k").unwrap();
        store.save(ProviderKind::OpenAi, "  ").unwrap();
        assert!(!store.is_configured(ProviderKind::OpenAi));
        assert_eq!(store.key(ProviderKind::OpenAi), None);
        assert_eq!(store.revision(), before + 2);
    }

    #[test]
    fn a_key_the_keychain_refused_is_still_kept_for_the_session() {
        let store = AiKeyStore::with_backend(Box::new(Refusing));
        assert!(store.save(ProviderKind::Anthropic, "k").is_err());
        assert!(store.is_configured(ProviderKind::Anthropic));
        assert_eq!(store.key(ProviderKind::Anthropic).as_deref(), Some("k"));
    }

    #[test]
    fn the_environment_stands_in_only_when_nothing_is_saved() {
        let store = store();
        let kind = ProviderKind::OpenAi;
        let env = |name: &str| (name == kind.env_var()).then(|| " from-env ".to_owned());
        assert_eq!(key_with(&store, kind, &env).as_deref(), Some("from-env"));
        store.save(kind, "saved").unwrap();
        assert_eq!(key_with(&store, kind, &env).as_deref(), Some("saved"));
        store.remove(kind);
        assert_eq!(key_with(&store, kind, &env).as_deref(), Some("from-env"));
        assert_eq!(key_with(&store, kind, &|_| None), None);
        // The environment variable of one provider does not leak into another.
        assert_eq!(key_with(&store, ProviderKind::Anthropic, &env), None);
    }
}
