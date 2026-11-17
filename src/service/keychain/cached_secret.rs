//! One item in the secure storage, with what this process already learned of it. A store keeps
//! one per secret so a page asking every frame whether the secret is saved, or a request reading
//! it, reaches the Keychain (and its prompt) once, not every time.
//!
//! Nothing here calls the storage while holding the lock a page takes. A Keychain write or an
//! unlock prompt lasts as long as the user takes, and `is_saved` runs on every frame: it reads
//! `state` for an instant and never waits for `io`, which only serialises the calls that talk to
//! the storage (a read that missed, a fresh read, a save, a removal), so two workers do not
//! prompt at once and writes land in order.

use crate::lock::lock;
use crate::service::keychain::api_keys::KeyBackend;
use crate::service::keychain::webhook_secrets::Scope;
use crate::service::webhook::delivery::SecretStoreError;
use std::sync::Mutex;

#[derive(Default)]
struct State {
    /// The value read or saved this session; `Some(None)` is a confirmed absence.
    value: Option<Option<String>>,
    /// Whether one is saved, asked without reading it. Ignored once `value` is known.
    exists: Option<bool>,
}

pub(crate) struct CachedSecret {
    scope: Scope,
    account: String,
    /// Held only for a moment, never across a call to the storage.
    state: Mutex<State>,
    /// Held across the calls to the storage that read or change the item; a page never takes it.
    io: Mutex<()>,
}

impl CachedSecret {
    pub(crate) fn new(scope: Scope, account: impl Into<String>) -> Self {
        Self {
            scope,
            account: account.into(),
            state: Mutex::new(State::default()),
            io: Mutex::new(()),
        }
    }

    fn known(&self) -> Option<Option<String>> {
        lock(&self.state).value.clone()
    }

    fn read_backend(&self, backend: &dyn KeyBackend) -> Option<String> {
        backend
            .read(self.scope, &self.account)
            .filter(|value| !value.is_empty())
    }

    /// The saved value. The first read may prompt for Keychain access.
    pub(crate) fn read(&self, backend: &dyn KeyBackend) -> Option<String> {
        if let Some(value) = self.known() {
            return value;
        }
        let _io = lock(&self.io);
        // Another worker may have read it while this one waited for its turn.
        if let Some(value) = self.known() {
            return value;
        }
        let found = self.read_backend(backend);
        lock(&self.state).value = Some(found.clone());
        found
    }

    /// The saved value, asked of the storage again: another process may have changed it since
    /// this one read it.
    pub(crate) fn read_fresh(&self, backend: &dyn KeyBackend) -> Option<String> {
        let _io = lock(&self.io);
        let found = self.read_backend(backend);
        lock(&self.state).value = Some(found.clone());
        found
    }

    /// Whether one is saved. Asked of the storage once, without reading the value (so without a
    /// prompt), and without waiting for a read or a save that is under way.
    pub(crate) fn is_saved(&self, backend: &dyn KeyBackend) -> bool {
        {
            let state = lock(&self.state);
            if let Some(value) = &state.value {
                return value.is_some();
            }
            if let Some(exists) = state.exists {
                return exists;
            }
        }
        let found = backend.exists(self.scope, &self.account);
        let mut state = lock(&self.state);
        // A read or a save that finished meanwhile knows better than this answer does.
        match &state.value {
            Some(value) => value.is_some(),
            None => *state.exists.get_or_insert(found),
        }
    }

    /// Saves `value` as it is. If the storage refuses, the value is still kept for this session
    /// and the error says it was not saved for good.
    pub(crate) fn save(
        &self,
        backend: &dyn KeyBackend,
        value: &str,
    ) -> Result<(), SecretStoreError> {
        let _io = lock(&self.io);
        let written = backend.write(self.scope, &self.account, value);
        *lock(&self.state) = State {
            value: Some(Some(value.to_owned())),
            exists: None,
        };
        written
    }

    /// Removes the item; this session treats it as gone whatever the storage answered.
    pub(crate) fn remove(&self, backend: &dyn KeyBackend) -> Result<(), SecretStoreError> {
        let _io = lock(&self.io);
        let removed = backend.delete(self.scope, &self.account);
        *lock(&self.state) = State {
            value: Some(None),
            exists: None,
        };
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain::api_keys::MemoryBackend;
    use crate::service::keychain::gated_backend::gated;
    use std::sync::atomic::Ordering;

    const SCOPE: Scope = Scope {
        service: "es.nico.degenview.test",
        shared_group: false,
    };

    /// Two workers reading an item nobody has read yet prompt once, not twice.
    #[test]
    fn two_first_reads_ask_the_storage_once() {
        let (backend, gate) = gated(true, false);
        backend_with(&backend, "secret");
        let secret = CachedSecret::new(SCOPE, "item");
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| secret.read(&backend));
            gate.wait_until_held();
            let second = scope.spawn(|| secret.read(&backend));
            // The second has had every chance to reach the storage.
            std::thread::sleep(std::time::Duration::from_millis(100));
            gate.release();
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_eq!(
            (first.as_deref(), second.as_deref()),
            (Some("secret"), Some("secret"))
        );
        assert_eq!(gate.reads.load(Ordering::SeqCst), 1);
    }

    /// A save made while a slow read is under way is not overwritten by what that read finds.
    #[test]
    fn a_save_after_a_slow_read_stays_cached() {
        let (backend, gate) = gated(true, false);
        let secret = CachedSecret::new(SCOPE, "item");
        std::thread::scope(|scope| {
            let slow = scope.spawn(|| secret.read(&backend));
            gate.wait_until_held();
            let saving = scope.spawn(|| secret.save(&backend, "new"));
            std::thread::sleep(std::time::Duration::from_millis(100));
            gate.release();
            assert_eq!(slow.join().unwrap(), None, "the read began before the save");
            saving.join().unwrap().unwrap();
        });
        assert_eq!(secret.read(&backend).as_deref(), Some("new"));
        assert!(secret.is_saved(&backend));
    }

    /// An answer to "is one saved?" that arrives after a save does not undo it.
    #[test]
    fn a_stale_exists_answer_does_not_hide_a_save() {
        let backend = MemoryBackend::default();
        let secret = CachedSecret::new(SCOPE, "item");
        assert!(!secret.is_saved(&backend));
        secret.save(&backend, "v").unwrap();
        assert!(secret.is_saved(&backend));
        secret.remove(&backend).unwrap();
        assert!(!secret.is_saved(&backend));
    }

    fn backend_with(backend: &crate::service::keychain::gated_backend::GatedBackend, value: &str) {
        backend.write(SCOPE, "item", value).unwrap();
    }
}
