//! What the user chose about snapshots, shared by every window and the Settings page: where
//! hosted snapshots go, whether the account is drawn on them, how sharp they are and which hosts
//! they agreed to send images to. One value behind a mutex, written to the `setting` table on
//! every change.

use crate::model::snapshot::{SnapshotHost as Host, SnapshotPrefs, SnapshotScale};
use crate::store::{SharedStore, Store};

/// Whether the hosted actions can be used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Hosting {
    Ready(Host),
    /// No address is set (or the saved one is no longer valid).
    NotConfigured,
}

#[derive(Clone, Default)]
pub struct SnapshotHost {
    store: Option<SharedStore>,
    prefs: std::sync::Arc<std::sync::Mutex<SnapshotPrefs>>,
}

impl SnapshotHost {
    /// The saved choices; a value this version cannot read is ignored (and left as it is until
    /// the user changes something).
    pub fn load(store: SharedStore) -> Self {
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
            prefs: std::sync::Arc::new(std::sync::Mutex::new(prefs)),
        }
    }

    pub fn prefs(&self) -> SnapshotPrefs {
        self.prefs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn hosting(&self) -> Hosting {
        self.prefs()
            .host()
            .map_or(Hosting::NotConfigured, Hosting::Ready)
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

    pub fn set_include_trading(&self, held: Option<&Store>, include: bool) -> Result<(), String> {
        self.update(held, |prefs| prefs.include_trading = include)
    }

    pub fn set_scale(&self, held: Option<&Store>, scale: SnapshotScale) -> Result<(), String> {
        self.update(held, |prefs| prefs.scale = scale)
    }

    /// Sets the host's address; empty clears it. The address must be valid.
    pub fn set_host(&self, held: Option<&Store>, address: &str) -> Result<(), String> {
        let address = address.trim();
        if address.is_empty() {
            return self.update(held, |prefs| prefs.host = None);
        }
        let host = Host::parse(address).map_err(|error| error.message().to_owned())?;
        self.update(held, |prefs| prefs.host = Some(host.endpoint().to_owned()))
    }

    pub fn consent(&self, held: Option<&Store>, host: &Host) -> Result<(), String> {
        self.update(held, |prefs| prefs.consent(host))
    }

    pub fn forget_consent(&self, held: Option<&Store>) -> Result<(), String> {
        self.update(held, |prefs| prefs.consented.clear())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::sync::{Arc, Mutex};

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("a store")))
    }

    #[test]
    fn nothing_is_shared_until_a_host_is_set() {
        let host = SnapshotHost::load(store());
        assert_eq!(host.hosting(), Hosting::NotConfigured);
        let prefs = host.prefs();
        assert!(!prefs.include_trading);
        assert_eq!(prefs.scale, SnapshotScale::X2);
        assert!(prefs.consented.is_empty());
    }

    #[test]
    fn choices_survive_a_restart() {
        let shared = store();
        let host = SnapshotHost::load(shared.clone());
        host.set_host(None, " https://snap.example.com/up ")
            .unwrap();
        host.set_include_trading(None, true).unwrap();
        host.set_scale(None, SnapshotScale::X1).unwrap();
        let Hosting::Ready(parsed) = host.hosting() else {
            panic!("a host was set");
        };
        host.consent(None, &parsed).unwrap();

        let again = SnapshotHost::load(shared);
        let prefs = again.prefs();
        assert_eq!(prefs.host.as_deref(), Some("https://snap.example.com/up"));
        assert!(prefs.include_trading);
        assert_eq!(prefs.scale, SnapshotScale::X1);
        assert!(prefs.has_consented(&parsed));
        again.forget_consent(None).unwrap();
        assert!(again.prefs().consented.is_empty());
    }

    #[test]
    fn a_bad_address_is_refused_and_changes_nothing() {
        let host = SnapshotHost::load(store());
        host.set_host(None, "https://snap.example.com/up").unwrap();
        let error = host
            .set_host(None, "http://snap.example.com/up")
            .unwrap_err();
        assert!(error.contains("https://"), "{error}");
        assert_eq!(
            host.prefs().host.as_deref(),
            Some("https://snap.example.com/up")
        );
        host.set_host(None, "  ").unwrap();
        assert_eq!(host.hosting(), Hosting::NotConfigured);
    }

    #[test]
    fn a_value_this_version_cannot_read_falls_back_to_the_defaults() {
        let shared = store();
        shared
            .lock()
            .unwrap()
            .set_setting(SnapshotPrefs::KEY, &"not an object")
            .unwrap();
        let host = SnapshotHost::load(shared);
        assert_eq!(host.prefs(), SnapshotPrefs::default());
    }

    #[test]
    fn a_host_without_a_database_still_holds_its_choices() {
        let host = SnapshotHost::default();
        host.set_include_trading(None, true).unwrap();
        assert!(host.prefs().include_trading);
    }
}
