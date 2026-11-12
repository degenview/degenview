//! What the user chose about snapshots, shared by every window and the Settings page: which
//! provider hosted snapshots go to (if any), what each was given, whether the account is drawn on
//! them, how sharp they are and which destinations they agreed to send images to. One value behind
//! a mutex, written to the `setting` table on every change. The secrets a provider asks for are
//! not here: they are in the secure storage (`keychain::snapshot_secrets`).

use crate::model::chart::snapshot::{SnapshotPrefs, SnapshotScale};
use crate::service::keychain::snapshot_secrets::SnapshotSecrets;
use crate::service::snapshot::providers::{self, Destination, FieldKind, Readiness};
use crate::store::{SharedStore, Store};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Said while no provider is turned on.
pub const TURN_ON_A_HOST: &str = "Turn on a snapshot host in Settings ▸ Snapshots to share.";

/// Whether the hosted actions can be used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Hosting {
    Ready(Destination),
    /// They cannot yet, and this is what the user has to do (the menu shows it).
    Unavailable(String),
}

#[derive(Clone)]
pub struct SnapshotHost {
    store: Option<SharedStore>,
    prefs: Arc<Mutex<SnapshotPrefs>>,
    secrets: Arc<SnapshotSecrets>,
}

impl Default for SnapshotHost {
    fn default() -> Self {
        Self {
            store: None,
            prefs: Arc::default(),
            secrets: SnapshotSecrets::shared(),
        }
    }
}

impl SnapshotHost {
    /// The saved choices; a value this version cannot read is ignored (and left as it is until
    /// the user changes something).
    pub fn load(store: SharedStore) -> Self {
        Self::load_with(store, SnapshotSecrets::shared())
    }

    pub fn load_with(store: SharedStore, secrets: Arc<SnapshotSecrets>) -> Self {
        let prefs = store
            .lock()
            .ok()
            .and_then(|store| {
                store
                    .setting_strict::<SnapshotPrefs>(SnapshotPrefs::KEY)
                    .ok()
            })
            .flatten()
            .unwrap_or_default();
        Self {
            store: Some(store),
            prefs: Arc::new(Mutex::new(prefs)),
            secrets,
        }
    }

    pub fn prefs(&self) -> SnapshotPrefs {
        self.prefs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn secrets(&self) -> &Arc<SnapshotSecrets> {
        &self.secrets
    }

    /// Where a hosted snapshot would go now, or what is missing.
    pub fn hosting(&self) -> Hosting {
        let prefs = self.prefs();
        let Some(id) = prefs.provider.as_deref() else {
            return Hosting::Unavailable(TURN_ON_A_HOST.to_owned());
        };
        let Some(provider) = providers::provider(id) else {
            return Hosting::Unavailable(
                "This snapshot host is not in this version of DegenView. Choose another in \
                 Settings ▸ Snapshots."
                    .to_owned(),
            );
        };
        let settings = prefs.settings_of(id);
        let present = self.secrets.presence(provider);
        if present.is_pending() {
            // The secure storage is being asked, off this thread: "no key" would be a guess.
            return Hosting::Unavailable(
                "Checking your saved key. Try again in a moment.".to_owned(),
            );
        }
        match provider.readiness(&settings, &present) {
            Readiness::Ready => {
                let note = provider.consent_note(&settings, &present);
                Hosting::Ready(Destination::new(provider, settings).with_note(note))
            }
            Readiness::Needs(what) => Hosting::Unavailable(what),
        }
    }

    /// Applies `change` and saves the result. `held` is the store when the caller already has it
    /// locked (the Settings window draws under that lock): locking it again would wait for itself
    /// forever, so it is written through directly. `None` locks the host's own store.
    pub fn update(
        &self,
        held: Option<&Store>,
        change: impl FnOnce(&mut SnapshotPrefs),
    ) -> Result<(), String> {
        let mut prefs = self.prefs.lock().unwrap_or_else(|e| e.into_inner());
        // What is stored is the truth, not what was read at launch: iCloud Sync may have applied
        // another Mac's change since, and writing the old copy back would undo it there.
        if let Some(stored) = self.stored(held) {
            *prefs = stored;
        }
        change(&mut prefs);
        let write = |store: &Store| {
            store
                .set_setting(SnapshotPrefs::KEY, &*prefs)
                .map_err(|error| format!("Could not save the snapshot settings: {error}"))
        };
        if let Some(store) = held {
            return write(store);
        }
        let Some(shared) = &self.store else {
            return Ok(());
        };
        let store = shared
            .lock()
            .map_err(|_| "The settings database is unavailable.".to_owned())?;
        write(&store)
    }

    /// The saved choices, read again (`None` when there is nothing readable to read).
    fn stored(&self, held: Option<&Store>) -> Option<SnapshotPrefs> {
        let read = |store: &Store| {
            store
                .setting_strict::<SnapshotPrefs>(SnapshotPrefs::KEY)
                .ok()
                .flatten()
        };
        match held {
            Some(store) => read(store),
            None => self
                .store
                .as_ref()
                .and_then(|shared| shared.lock().ok().and_then(|store| read(&store))),
        }
    }

    /// Takes what is saved now, for a window that shows it: a change synced from another Mac
    /// reaches the page and the camera menu this way. Not for every frame: it reads the database.
    pub fn refresh(&self, held: Option<&Store>) {
        if let Some(stored) = self.stored(held) {
            *self.prefs.lock().unwrap_or_else(|e| e.into_inner()) = stored;
        }
    }

    pub fn set_include_trading(&self, held: Option<&Store>, include: bool) -> Result<(), String> {
        self.update(held, |prefs| prefs.include_trading = include)
    }

    pub fn set_scale(&self, held: Option<&Store>, scale: SnapshotScale) -> Result<(), String> {
        self.update(held, |prefs| prefs.scale = scale)
    }

    /// Turns `provider` on, which turns the one that was on off; `None` turns sharing off.
    pub fn set_provider(&self, held: Option<&Store>, provider: Option<&str>) -> Result<(), String> {
        if let Some(id) = provider
            && providers::provider(id).is_none()
        {
            return Err("That snapshot host is not available.".to_owned());
        }
        self.update(held, |prefs| prefs.activate(provider))
    }

    /// Saves some of a provider's (non-secret) fields: all of them or, if one is not right,
    /// none, with the reason. An empty value clears a field.
    pub fn set_fields(
        &self,
        held: Option<&Store>,
        provider: &str,
        fields: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        let descriptor = providers::provider(provider)
            .ok_or_else(|| "That snapshot host is not available.".to_owned())?
            .descriptor();
        for (key, value) in fields {
            let field = descriptor
                .field(key)
                .ok_or_else(|| format!("{} has no setting called {key}.", descriptor.name))?;
            let value = value.trim();
            match field.kind {
                FieldKind::Secret => {
                    return Err(format!("{} is kept in the secure storage.", field.title));
                }
                FieldKind::Text => {
                    if !value.is_empty()
                        && let Some(problem) = (field.validate)(value)
                    {
                        return Err(problem.to_owned());
                    }
                }
                FieldKind::Choice(choices) => {
                    if !value.is_empty() && !choices.iter().any(|choice| choice.value == value) {
                        return Err(format!("{value} is not a choice for {}.", field.title));
                    }
                }
            }
        }
        self.update(held, |prefs| {
            for (key, value) in fields {
                prefs.set_field(provider, key, value);
            }
        })
    }

    /// Saves a secret in the secure storage; an empty one removes it.
    pub fn save_secret(&self, provider: &str, key: &str, value: &str) -> Result<(), String> {
        let (id, field) = self.secret_field(provider, key)?;
        let value = value.trim();
        if !value.is_empty()
            && let Some(problem) = (field.validate)(value)
        {
            return Err(problem.to_owned());
        }
        self.secrets
            .save(id, field, value)
            .map_err(|error| format!("Could not save it: {error}"))
    }

    pub fn remove_secret(&self, provider: &str, key: &str) -> Result<(), String> {
        let (id, field) = self.secret_field(provider, key)?;
        self.secrets.remove(id, field);
        Ok(())
    }

    fn secret_field(
        &self,
        provider: &str,
        key: &str,
    ) -> Result<(&'static str, &'static providers::FieldSpec), String> {
        let descriptor = providers::provider(provider)
            .ok_or_else(|| "That snapshot host is not available.".to_owned())?
            .descriptor();
        descriptor
            .field(key)
            .filter(|field| field.is_secret())
            .map(|field| (descriptor.id, field))
            .ok_or_else(|| format!("{} has no secret called {key}.", descriptor.name))
    }

    pub fn consent(&self, held: Option<&Store>, destination: &str) -> Result<(), String> {
        self.update(held, |prefs| prefs.consent(destination))
    }

    pub fn forget_consent(&self, held: Option<&Store>) -> Result<(), String> {
        self.update(held, |prefs| prefs.consented.clear())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chart::snapshot::SELF_HOSTED;
    use crate::service::keychain::api_keys::MemoryBackend;
    use crate::store::Store;

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("a store")))
    }

    /// A host with secure storage of its own, so tests do not share a key.
    pub(crate) fn host_with(store: SharedStore) -> SnapshotHost {
        SnapshotHost::load_with(
            store,
            Arc::new(SnapshotSecrets::with(
                Box::new(MemoryBackend::default()),
                Box::new(|_| None),
            )),
        )
    }

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn nothing_is_shared_until_a_provider_is_turned_on() {
        let host = host_with(store());
        assert_eq!(
            host.hosting(),
            Hosting::Unavailable(TURN_ON_A_HOST.to_owned())
        );
        let prefs = host.prefs();
        assert_eq!(prefs.provider, None);
        assert!(!prefs.include_trading);
        assert_eq!(prefs.scale, SnapshotScale::X2);
        assert!(prefs.consented.is_empty());
    }

    #[test]
    fn a_provider_that_is_on_but_not_set_up_says_what_is_missing() {
        let host = host_with(store());
        host.set_provider(None, Some("imgbb")).unwrap();
        assert_eq!(
            host.hosting(),
            Hosting::Unavailable("Add your ImgBB API key in Settings ▸ Snapshots to share.".into())
        );
        host.save_secret("imgbb", "key", "abc123").unwrap();
        let Hosting::Ready(destination) = host.hosting() else {
            panic!("a key was saved");
        };
        assert_eq!(destination.id(), "imgbb");
        assert_eq!(destination.consent_key(), "api.imgbb.com");
        // Remove the key and it is not ready again.
        host.remove_secret("imgbb", "key").unwrap();
        assert!(matches!(host.hosting(), Hosting::Unavailable(_)));
    }

    #[test]
    fn a_host_of_your_own_is_ready_with_a_valid_address_alone() {
        let host = host_with(store());
        host.set_provider(None, Some(SELF_HOSTED)).unwrap();
        assert!(matches!(host.hosting(), Hosting::Unavailable(_)));
        host.set_fields(
            None,
            SELF_HOSTED,
            &fields(&[("host", " https://snap.example.com/up ")]),
        )
        .unwrap();
        let Hosting::Ready(destination) = host.hosting() else {
            panic!("an address was set");
        };
        assert_eq!(destination.label(), "snap.example.com");
    }

    #[test]
    fn choices_survive_a_restart() {
        let shared = store();
        let host = host_with(shared.clone());
        host.set_fields(
            None,
            SELF_HOSTED,
            &fields(&[("host", "https://snap.example.com/up")]),
        )
        .unwrap();
        host.set_fields(None, "imgbb", &fields(&[("expiry", "7d")]))
            .unwrap();
        host.set_provider(None, Some(SELF_HOSTED)).unwrap();
        host.set_include_trading(None, true).unwrap();
        host.set_scale(None, SnapshotScale::X1).unwrap();
        host.consent(None, "snap.example.com").unwrap();

        let again = host_with(shared);
        let prefs = again.prefs();
        assert_eq!(prefs.provider.as_deref(), Some(SELF_HOSTED));
        assert_eq!(
            prefs
                .settings_of(SELF_HOSTED)
                .get("host")
                .map(String::as_str),
            Some("https://snap.example.com/up")
        );
        assert_eq!(
            prefs.settings_of("imgbb").get("expiry").map(String::as_str),
            Some("7d"),
            "the provider that is off keeps what it was given"
        );
        assert!(prefs.include_trading);
        assert_eq!(prefs.scale, SnapshotScale::X1);
        assert!(prefs.has_consented("snap.example.com"));
        again.forget_consent(None).unwrap();
        assert!(again.prefs().consented.is_empty());
    }

    #[test]
    fn turning_one_on_turns_the_other_off() {
        let host = host_with(store());
        host.set_provider(None, Some("imgbb")).unwrap();
        host.set_provider(None, Some(SELF_HOSTED)).unwrap();
        assert_eq!(host.prefs().provider.as_deref(), Some(SELF_HOSTED));
        host.set_provider(None, None).unwrap();
        assert_eq!(host.prefs().provider, None);
        assert!(host.set_provider(None, Some("nope")).is_err());
    }

    #[test]
    fn a_setting_that_is_not_right_is_refused_whole_and_changes_nothing() {
        let host = host_with(store());
        host.set_fields(
            None,
            SELF_HOSTED,
            &fields(&[("host", "https://snap.example.com/up")]),
        )
        .unwrap();
        let error = host
            .set_fields(
                None,
                SELF_HOSTED,
                &fields(&[("host", "http://snap.example.com/up")]),
            )
            .unwrap_err();
        assert!(error.contains("https://"), "{error}");
        assert!(
            host.set_fields(None, "imgbb", &fields(&[("expiry", "forever")]))
                .is_err()
        );
        assert!(
            host.set_fields(None, "imgbb", &fields(&[("nope", "x")]))
                .is_err()
        );
        assert!(
            host.set_fields(None, "imgbb", &fields(&[("key", "x")]))
                .is_err(),
            "a secret is not a setting"
        );
        assert!(host.set_fields(None, "nope", &fields(&[])).is_err());
        assert_eq!(
            host.prefs()
                .settings_of(SELF_HOSTED)
                .get("host")
                .map(String::as_str),
            Some("https://snap.example.com/up")
        );
        host.set_fields(None, SELF_HOSTED, &fields(&[("host", "  ")]))
            .unwrap();
        assert!(host.prefs().settings_of(SELF_HOSTED).is_empty());
    }

    #[test]
    fn a_secret_is_checked_and_never_reaches_the_settings() {
        let shared = store();
        let host = host_with(shared.clone());
        assert!(host.save_secret("imgbb", "key", "has space").is_err());
        assert!(
            host.save_secret("imgbb", "expiry", "x").is_err(),
            "not a secret field"
        );
        host.save_secret("imgbb", "key", "topsecret123").unwrap();
        let stored = shared
            .lock()
            .unwrap()
            .raw_setting(SnapshotPrefs::KEY)
            .unwrap()
            .unwrap_or_default()
            .to_string();
        assert!(!stored.contains("topsecret123"), "{stored}");
    }

    #[test]
    fn a_value_this_version_cannot_read_falls_back_to_the_defaults() {
        let shared = store();
        shared
            .lock()
            .unwrap()
            .set_setting(SnapshotPrefs::KEY, &"not an object")
            .unwrap();
        let host = host_with(shared);
        assert_eq!(host.prefs(), SnapshotPrefs::default());
    }

    #[test]
    fn a_save_from_before_providers_keeps_sharing_to_its_host() {
        let shared = store();
        shared
            .lock()
            .unwrap()
            .set_setting(
                SnapshotPrefs::KEY,
                &serde_json::json!({"host": "https://snap.example.com/up", "scale": "X1"}),
            )
            .unwrap();
        let host = host_with(shared.clone());
        let Hosting::Ready(destination) = host.hosting() else {
            panic!("the old host is still the host");
        };
        assert_eq!(destination.id(), SELF_HOSTED);
        assert_eq!(destination.label(), "snap.example.com");
        // The next change writes the new shape, with no `host` left in it.
        host.set_scale(None, SnapshotScale::X2).unwrap();
        let stored = shared
            .lock()
            .unwrap()
            .raw_setting(SnapshotPrefs::KEY)
            .unwrap()
            .unwrap();
        assert!(stored.get("host").is_none(), "{stored}");
        assert_eq!(stored["provider"], "self_hosted");
        assert_eq!(
            stored["providers"]["self_hosted"]["host"],
            "https://snap.example.com/up"
        );
    }

    #[test]
    fn a_change_synced_in_since_launch_is_not_undone_by_the_next_setting_changed_here() {
        let shared = store();
        let host = host_with(shared.clone());
        assert_eq!(host.prefs().provider, None);
        // Another Mac turned ImgBB on and iCloud Sync applied it to the database, which this
        // running window was never told about.
        shared
            .lock()
            .unwrap()
            .set_setting(
                SnapshotPrefs::KEY,
                &serde_json::json!({
                    "provider": "imgbb", "providers": {"imgbb": {"expiry": "7d"}},
                    "include_trading": false, "scale": "X2", "consented": []
                }),
            )
            .unwrap();
        host.set_include_trading(None, true).unwrap();
        let prefs = host.prefs();
        assert_eq!(
            prefs.provider.as_deref(),
            Some("imgbb"),
            "the synced choice stands"
        );
        assert_eq!(
            prefs.settings_of("imgbb").get("expiry").map(String::as_str),
            Some("7d")
        );
        assert!(prefs.include_trading, "and the change made here is made");
        let saved = host_with(shared).prefs();
        assert_eq!(saved.provider.as_deref(), Some("imgbb"));
    }

    #[test]
    fn a_window_can_take_what_sync_applied() {
        let shared = store();
        let host = host_with(shared.clone());
        shared
            .lock()
            .unwrap()
            .set_setting(
                SnapshotPrefs::KEY,
                &serde_json::json!({"provider": null, "providers": {}, "scale": "X1"}),
            )
            .unwrap();
        assert_eq!(host.prefs().scale, SnapshotScale::X2, "not yet");
        {
            let store = shared.lock().unwrap();
            host.refresh(Some(&store));
        }
        assert_eq!(host.prefs().scale, SnapshotScale::X1);
    }

    #[test]
    fn while_the_secure_storage_has_not_answered_the_menu_says_so_instead_of_guessing() {
        use crate::service::keychain::api_keys::KeyBackend;
        use crate::service::keychain::webhook_secrets::Scope;
        use crate::service::webhook::delivery::SecretStoreError;
        struct Slow(Mutex<std::sync::mpsc::Receiver<()>>);
        impl KeyBackend for Slow {
            fn read(&self, _: Scope, _: &str) -> Option<String> {
                Some("k".to_owned())
            }
            fn exists(&self, _: Scope, _: &str) -> bool {
                let _ = self.0.lock().unwrap().recv();
                true
            }
            fn write(&self, _: Scope, _: &str, _: &str) -> Result<(), SecretStoreError> {
                Ok(())
            }
            fn delete(&self, _: Scope, _: &str) -> Result<(), SecretStoreError> {
                Ok(())
            }
        }
        let (release, released) = std::sync::mpsc::channel();
        let (ready, answered) = std::sync::mpsc::channel();
        let secrets = Arc::new(SnapshotSecrets::asynchronous(
            Box::new(Slow(Mutex::new(released))),
            Box::new(|_| None),
        ));
        secrets.set_on_ready(Arc::new(move || {
            let _ = ready.send(());
        }));
        let host = SnapshotHost::load_with(store(), secrets);
        host.set_provider(None, Some("imgbb")).unwrap();
        let Hosting::Unavailable(waiting) = host.hosting() else {
            panic!("nothing is known yet");
        };
        assert!(waiting.contains("Checking"), "{waiting}");
        assert!(
            !waiting.contains("Add your"),
            "not a guess that the key is missing"
        );
        release.send(()).unwrap();
        answered
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the answer arrives");
        assert!(matches!(host.hosting(), Hosting::Ready(_)));
    }

    #[test]
    fn a_host_with_a_saved_token_says_so_before_the_first_upload() {
        let host = host_with(store());
        host.set_provider(None, Some(SELF_HOSTED)).unwrap();
        host.set_fields(
            None,
            SELF_HOSTED,
            &fields(&[("host", "https://snap.example.com/up")]),
        )
        .unwrap();
        let Hosting::Ready(without) = host.hosting() else {
            panic!("an address was set");
        };
        assert_eq!(without.note(), None);
        host.save_secret(SELF_HOSTED, "token", "tok123").unwrap();
        let Hosting::Ready(with) = host.hosting() else {
            panic!("ready");
        };
        assert!(
            with.note()
                .is_some_and(|note| note.contains("access token"))
        );
    }

    #[test]
    fn a_host_without_a_database_still_holds_its_choices() {
        let host = SnapshotHost::default();
        host.set_include_trading(None, true).unwrap();
        assert!(host.prefs().include_trading);
    }
}
