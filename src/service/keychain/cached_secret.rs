//! One item in the secure storage, with what this process already learned of it. A store keeps
//! one per secret so a page asking every frame whether the secret is saved, or a request reading
//! it, reaches the Keychain (and its prompt) once, not every time.

use crate::service::keychain::api_keys::KeyBackend;
use crate::service::keychain::webhook_secrets::Scope;
use crate::service::webhook::delivery::SecretStoreError;

pub(crate) struct CachedSecret {
    scope: Scope,
    account: String,
    /// The value read or saved this session; `Some(None)` is a confirmed absence.
    value: Option<Option<String>>,
    /// Whether one is saved, asked without reading it.
    exists: Option<bool>,
}

impl CachedSecret {
    pub(crate) fn new(scope: Scope, account: impl Into<String>) -> Self {
        Self {
            scope,
            account: account.into(),
            value: None,
            exists: None,
        }
    }

    /// The saved value. The first read may prompt for Keychain access.
    pub(crate) fn read(&mut self, backend: &dyn KeyBackend) -> Option<String> {
        if self.value.is_none() {
            self.value = Some(self.read_backend(backend));
        }
        self.value.clone().flatten()
    }

    /// The saved value, asked of the storage again: another process may have changed it since
    /// this one read it.
    pub(crate) fn read_fresh(&mut self, backend: &dyn KeyBackend) -> Option<String> {
        let found = self.read_backend(backend);
        self.value = Some(found.clone());
        found
    }

    fn read_backend(&self, backend: &dyn KeyBackend) -> Option<String> {
        backend
            .read(self.scope, &self.account)
            .filter(|value| !value.is_empty())
    }

    /// Whether one is saved. Asked of the storage once, without reading the value (so without a
    /// prompt).
    pub(crate) fn is_saved(&mut self, backend: &dyn KeyBackend) -> bool {
        if let Some(value) = &self.value {
            return value.is_some();
        }
        *self
            .exists
            .get_or_insert_with(|| backend.exists(self.scope, &self.account))
    }

    /// Saves `value` as it is. If the storage refuses, the value is still kept for this session
    /// and the error says it was not saved for good.
    pub(crate) fn save(
        &mut self,
        backend: &dyn KeyBackend,
        value: &str,
    ) -> Result<(), SecretStoreError> {
        let written = backend.write(self.scope, &self.account, value);
        self.value = Some(Some(value.to_owned()));
        self.exists = None;
        written
    }

    /// Removes the item; this session treats it as gone whatever the storage answered.
    pub(crate) fn remove(&mut self, backend: &dyn KeyBackend) -> Result<(), SecretStoreError> {
        let removed = backend.delete(self.scope, &self.account);
        self.value = Some(None);
        self.exists = None;
        removed
    }
}
