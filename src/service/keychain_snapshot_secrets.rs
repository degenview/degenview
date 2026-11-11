//! The secrets a snapshot provider asks for (an API key, an access token), kept in the system's
//! secure storage beside the data providers' keys. One store serves every provider: a secret is
//! addressed by the provider and the field, so a new provider needs nothing here.
//!
//! A secret is read only when an upload needs it, so a user who never shares a snapshot is never
//! prompted. Under `DEGENVIEW_NO_KEYCHAIN` (and on platforms with no secure storage) it lives in
//! memory for the session; a field's environment variable stands in when nothing is saved.

use crate::service::keychain_api_keys::{KeyBackend, default_backend};
use crate::service::keychain_webhook_secrets::Scope;
use crate::service::snapshot_providers::{FieldSpec, SecretPresence, Secrets, SnapshotProvider};
use crate::service::webhook_delivery::SecretStoreError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

const SERVICE: &str = "es.nico.degenview.snapshots";
const SCOPE: Scope = Scope {
    service: SERVICE,
    shared_group: false,
};

/// Where a secret stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretState {
    /// Saved in the secure storage.
    Saved,
    /// Not saved, but the field's environment variable gives one.
    FromEnvironment,
    Missing,
}

impl SecretState {
    pub fn is_present(self) -> bool {
        self != Self::Missing
    }
}

#[derive(Default)]
struct Slot {
    loaded: bool,
    value: Option<String>,
    exists: Option<bool>,
}

type Env = dyn Fn(&str) -> Option<String> + Send + Sync;

pub struct SnapshotSecrets {
    backend: Box<dyn KeyBackend>,
    env: Box<Env>,
    slots: Mutex<HashMap<String, Slot>>,
}

/// The account a field's secret is saved under.
fn account(provider: &str, field: &FieldSpec) -> String {
    field
        .account
        .map_or_else(|| format!("{provider}.{}-v1", field.key), str::to_owned)
}

fn clean(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

impl SnapshotSecrets {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self::with(backend, Box::new(|name| std::env::var(name).ok()))
    }

    /// For a test: the secure storage and the environment it sees.
    pub fn with(backend: Box<dyn KeyBackend>, env: Box<Env>) -> Self {
        Self {
            backend,
            env,
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// The store every window shares.
    pub fn shared() -> Arc<Self> {
        static STORE: OnceLock<Arc<SnapshotSecrets>> = OnceLock::new();
        STORE
            .get_or_init(|| Arc::new(Self::with_backend(default_backend())))
            .clone()
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn environment(&self, field: &FieldSpec) -> Option<String> {
        clean(field.env.and_then(|name| (self.env)(name)))
    }

    /// The saved secret, else the environment's. The first read may prompt for access: the lock on
    /// the cache is never held while it does, because the window asks which secrets exist on
    /// every frame and must not wait for the user to answer a prompt for an upload.
    pub fn value(&self, provider: &str, field: &FieldSpec) -> Option<String> {
        let account = account(provider, field);
        let cached = self
            .slots()
            .get(&account)
            .filter(|slot| slot.loaded)
            .map(|slot| slot.value.clone());
        let saved = match cached {
            Some(value) => value,
            None => {
                let read = clean(self.backend.read(SCOPE, &account));
                let mut slots = self.slots();
                let slot = slots.entry(account).or_default();
                if slot.loaded {
                    // Another thread (or a save) got there first: what it left stands.
                    slot.value.clone()
                } else {
                    slot.loaded = true;
                    slot.value = read.clone();
                    read
                }
            }
        };
        saved.or_else(|| self.environment(field))
    }

    /// Whether a secret is saved, without reading it (so without a prompt). Asks the storage only
    /// once per secret, and never with the cache locked.
    fn is_saved(&self, provider: &str, field: &FieldSpec) -> bool {
        let account = account(provider, field);
        {
            let slots = self.slots();
            if let Some(slot) = slots.get(&account) {
                if slot.loaded {
                    return slot.value.is_some();
                }
                if let Some(exists) = slot.exists {
                    return exists;
                }
            }
        }
        let found = self.backend.exists(SCOPE, &account);
        let mut slots = self.slots();
        let slot = slots.entry(account).or_default();
        if slot.loaded {
            return slot.value.is_some();
        }
        slot.exists = Some(found);
        found
    }

    pub fn state(&self, provider: &str, field: &FieldSpec) -> SecretState {
        if self.is_saved(provider, field) {
            SecretState::Saved
        } else if self.environment(field).is_some() {
            SecretState::FromEnvironment
        } else {
            SecretState::Missing
        }
    }

    /// Which of a provider's secrets have a value. Never prompts.
    pub fn presence(&self, provider: &dyn SnapshotProvider) -> SecretPresence {
        let descriptor = provider.descriptor();
        let mut present = SecretPresence::default();
        for field in descriptor.secret_fields() {
            if self.state(descriptor.id, field).is_present() {
                present.insert(field.key);
            }
        }
        present
    }

    /// Every secret a provider has, for one upload. Called on a worker: it may prompt.
    pub fn read_all(&self, provider: &dyn SnapshotProvider) -> Secrets {
        let descriptor = provider.descriptor();
        let mut secrets = Secrets::default();
        for field in descriptor.secret_fields() {
            if let Some(value) = self.value(descriptor.id, field) {
                secrets.insert(field.key, value);
            }
        }
        secrets
    }

    /// Saves the trimmed secret; an empty one removes it.
    pub fn save(
        &self,
        provider: &str,
        field: &FieldSpec,
        value: &str,
    ) -> Result<(), SecretStoreError> {
        let value = value.trim();
        if value.is_empty() {
            self.remove(provider, field);
            return Ok(());
        }
        let account = account(provider, field);
        self.backend.write(SCOPE, &account, value)?;
        self.slots().insert(
            account,
            Slot {
                loaded: true,
                value: Some(value.to_owned()),
                exists: None,
            },
        );
        Ok(())
    }

    pub fn remove(&self, provider: &str, field: &FieldSpec) {
        // Best effort: the secret is also forgotten here, and a new one replaces it on the next share.
        let account = account(provider, field);
        let _ = self.backend.delete(SCOPE, &account);
        self.slots().insert(
            account,
            Slot {
                loaded: true,
                value: None,
                exists: None,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::snapshot::SELF_HOSTED;
    use crate::service::keychain_api_keys::MemoryBackend;
    use crate::service::snapshot_providers::{imgbb, self_hosted};

    fn store_with_env(env: &'static [(&'static str, &'static str)]) -> SnapshotSecrets {
        SnapshotSecrets::with(
            Box::new(MemoryBackend::default()),
            Box::new(move |name| {
                env.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }),
        )
    }

    fn store() -> SnapshotSecrets {
        store_with_env(&[])
    }

    fn imgbb_key() -> &'static FieldSpec {
        imgbb::IMGBB
            .descriptor()
            .field("key")
            .expect("the key field")
    }

    fn host_token() -> &'static FieldSpec {
        self_hosted::SELF_HOSTED_PROVIDER
            .descriptor()
            .field("token")
            .expect("the token field")
    }

    #[test]
    fn a_saved_secret_is_trimmed_and_read_back() {
        let (store, key) = (store(), imgbb_key());
        assert_eq!(store.state("imgbb", key), SecretState::Missing);
        store.save("imgbb", key, "  abc123  ").unwrap();
        assert_eq!(store.state("imgbb", key), SecretState::Saved);
        assert_eq!(store.value("imgbb", key).as_deref(), Some("abc123"));
    }

    #[test]
    fn saving_nothing_removes_it() {
        let (store, key) = (store(), imgbb_key());
        store.save("imgbb", key, "abc").unwrap();
        store.save("imgbb", key, "   ").unwrap();
        assert_eq!(store.state("imgbb", key), SecretState::Missing);
        assert_eq!(store.value("imgbb", key), None);
    }

    #[test]
    fn the_environment_stands_in_only_when_nothing_is_saved() {
        let store = store_with_env(&[(imgbb::KEY_ENV, " from-env ")]);
        let key = imgbb_key();
        assert_eq!(store.state("imgbb", key), SecretState::FromEnvironment);
        assert_eq!(store.value("imgbb", key).as_deref(), Some("from-env"));
        store.save("imgbb", key, "saved").unwrap();
        assert_eq!(store.state("imgbb", key), SecretState::Saved);
        assert_eq!(store.value("imgbb", key).as_deref(), Some("saved"));
        store.remove("imgbb", key);
        assert_eq!(store.value("imgbb", key).as_deref(), Some("from-env"));
        assert_eq!(store_with_env(&[]).value("imgbb", key), None);
    }

    #[test]
    fn providers_do_not_share_secrets() {
        let (store, key, token) = (store(), imgbb_key(), host_token());
        store.save("imgbb", key, "imgbb-secret").unwrap();
        assert_eq!(store.state(SELF_HOSTED, token), SecretState::Missing);
        store.save(SELF_HOSTED, token, "host-token").unwrap();
        assert_eq!(store.value("imgbb", key).as_deref(), Some("imgbb-secret"));
        assert_eq!(
            store.value(SELF_HOSTED, token).as_deref(),
            Some("host-token")
        );
    }

    #[test]
    fn a_field_keeps_the_account_it_was_saved_under() {
        assert_eq!(
            account(SELF_HOSTED, host_token()),
            "host-token-v1",
            "a token saved before providers is still found"
        );
        assert_eq!(account("imgbb", imgbb_key()), "imgbb.key-v1");
    }

    #[test]
    fn presence_and_every_secret_are_read_per_provider() {
        let store = store();
        assert!(!store.presence(&imgbb::IMGBB).has("key"));
        store.save("imgbb", imgbb_key(), "abc").unwrap();
        assert!(store.presence(&imgbb::IMGBB).has("key"));
        assert_eq!(store.read_all(&imgbb::IMGBB).get("key"), Some("abc"));
        assert!(
            !store
                .presence(&self_hosted::SELF_HOSTED_PROVIDER)
                .has("token")
        );
    }

    /// A secure storage whose first read waits for the user (a Keychain prompt).
    struct Prompting {
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        asked: std::sync::mpsc::SyncSender<()>,
    }

    impl KeyBackend for Prompting {
        fn read(&self, _: Scope, _: &str) -> Option<String> {
            let _ = self.asked.send(());
            let _ = self.release.lock().unwrap().recv();
            Some("secret".to_owned())
        }
        fn exists(&self, _: Scope, _: &str) -> bool {
            true
        }
        fn write(&self, _: Scope, _: &str, _: &str) -> Result<(), SecretStoreError> {
            Ok(())
        }
        fn delete(&self, _: Scope, _: &str) -> Result<(), SecretStoreError> {
            Ok(())
        }
    }

    #[test]
    fn the_window_is_not_kept_waiting_while_a_prompt_is_open_on_the_upload_thread() {
        // The camera menu asks which secrets exist on every frame, on the UI thread; an upload
        // reads one on a worker, and the secure storage may be showing a prompt for it. The
        // window must not wait for the user to answer.
        let (release, released) = std::sync::mpsc::channel();
        let (asked, was_asked) = std::sync::mpsc::sync_channel(1);
        let store = Arc::new(SnapshotSecrets::with(
            Box::new(Prompting {
                release: Mutex::new(released),
                asked,
            }),
            Box::new(|_| None),
        ));
        let worker = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.value("imgbb", imgbb_key()))
        };
        was_asked
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the worker reached the prompt");
        let (done, finished) = std::sync::mpsc::channel();
        let ui = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                let state = store.state("imgbb", imgbb_key());
                let presence = store.presence(&imgbb::IMGBB).has("key");
                let _ = done.send((state, presence));
            })
        };
        let answered = finished.recv_timeout(std::time::Duration::from_secs(2));
        release.send(()).unwrap();
        assert_eq!(worker.join().unwrap().as_deref(), Some("secret"));
        ui.join().unwrap();
        assert!(
            answered.is_ok(),
            "the UI thread waited for a secure-storage prompt it has nothing to do with"
        );
    }
}
