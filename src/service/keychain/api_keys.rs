//! API keys for the data providers that need one: Alpaca (key id + secret), Oanda (account id +
//! access token) and CoinMarketCap (one key). Ports of `AlpacaCredentialsStore` and
//! `CoinMarketCapCredentialStore`, on the same Security-framework layer as the webhook secrets.
//!
//! Reading a key can raise a Keychain prompt, so `is_configured` asks only for the item's
//! attributes and the secret is read lazily on the first request that needs it. Under
//! `keychain::webhook_secrets::is_disabled` (tests, `DEGENVIEW_NO_KEYCHAIN=1`) nothing touches
//! the Keychain: keys live in memory for the session.

use crate::lock::lock;
use crate::service::keychain::webhook_secrets::{Scope, is_disabled, platform};
use crate::service::webhook::delivery::SecretStoreError;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

const ALPACA_SERVICE: &str = "es.nico.degenview.alpaca";
const ALPACA_ACCOUNT: &str = "credentials-v1";
const OANDA_SERVICE: &str = "es.nico.degenview.oanda";
const OANDA_ACCOUNT: &str = "credentials-v1";
const CMC_SERVICE: &str = "es.nico.degenview.coinmarketcap";
const CMC_ACCOUNT: &str = "api-key-v1";

// MARK: Backend

/// Where the items actually live. The Keychain in the app; memory in tests.
pub trait KeyBackend: Send + Sync {
    fn read(&self, scope: Scope, account: &str) -> Option<String>;
    fn exists(&self, scope: Scope, account: &str) -> bool;
    fn write(&self, scope: Scope, account: &str, value: &str) -> Result<(), SecretStoreError>;
    /// Removes the item. An item that is not there counts as removed; any other failure is an error:
    /// a secret that could not be removed is still stored.
    fn delete(&self, scope: Scope, account: &str) -> Result<(), SecretStoreError>;
}

pub struct KeychainBackend;

impl KeyBackend for KeychainBackend {
    fn read(&self, scope: Scope, account: &str) -> Option<String> {
        platform::read(scope, account)
    }
    fn exists(&self, scope: Scope, account: &str) -> bool {
        platform::exists(scope, account)
    }
    fn write(&self, scope: Scope, account: &str, value: &str) -> Result<(), SecretStoreError> {
        platform::write(scope, account, value)
    }
    fn delete(&self, scope: Scope, account: &str) -> Result<(), SecretStoreError> {
        platform::delete(scope, account)
    }
}

#[derive(Default)]
pub struct MemoryBackend {
    items: Mutex<std::collections::HashMap<(String, bool, String), String>>,
}

impl MemoryBackend {
    fn key(scope: Scope, account: &str) -> (String, bool, String) {
        (
            scope.service.to_owned(),
            scope.shared_group,
            account.to_owned(),
        )
    }
}

impl KeyBackend for MemoryBackend {
    fn read(&self, scope: Scope, account: &str) -> Option<String> {
        self.items
            .lock()
            .ok()?
            .get(&Self::key(scope, account))
            .cloned()
    }
    fn exists(&self, scope: Scope, account: &str) -> bool {
        self.read(scope, account).is_some()
    }
    fn write(&self, scope: Scope, account: &str, value: &str) -> Result<(), SecretStoreError> {
        if let Ok(mut items) = self.items.lock() {
            items.insert(Self::key(scope, account), value.to_owned());
        }
        Ok(())
    }
    fn delete(&self, scope: Scope, account: &str) -> Result<(), SecretStoreError> {
        if let Ok(mut items) = self.items.lock() {
            items.remove(&Self::key(scope, account));
        }
        Ok(())
    }
}

pub(crate) fn default_backend() -> Box<dyn KeyBackend> {
    if is_disabled() {
        Box::new(MemoryBackend::default())
    } else {
        Box::new(KeychainBackend)
    }
}

// MARK: Alpaca

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AlpacaCredentials {
    #[serde(rename = "keyID")]
    pub key_id: String,
    #[serde(rename = "secretKey")]
    pub secret_key: String,
}

impl CredentialPair for AlpacaCredentials {
    fn fields(&self) -> (&str, &str) {
        (&self.key_id, &self.secret_key)
    }

    fn trimmed(&self) -> Self {
        Self {
            key_id: self.key_id.trim().to_owned(),
            secret_key: self.secret_key.trim().to_owned(),
        }
    }
}

const ALPACA_SHARED: Scope = Scope {
    service: ALPACA_SERVICE,
    shared_group: true,
};
const ALPACA_OWN: Scope = Scope {
    service: ALPACA_SERVICE,
    shared_group: false,
};

pub type AlpacaCredentialsStore = CredentialsStore<AlpacaCredentials>;

impl AlpacaCredentialsStore {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self::new(backend, ALPACA_SHARED, ALPACA_ACCOUNT, Some(ALPACA_OWN))
    }

    pub fn shared() -> &'static Self {
        static STORE: OnceLock<AlpacaCredentialsStore> = OnceLock::new();
        STORE.get_or_init(|| Self::with_backend(default_backend()))
    }
}

// MARK: Oanda

/// The Oanda account id and personal access token, as one item.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OandaCredentials {
    #[serde(rename = "accountId")]
    pub account_id: String,
    #[serde(rename = "accessToken")]
    pub access_token: String,
}

impl CredentialPair for OandaCredentials {
    fn fields(&self) -> (&str, &str) {
        (&self.account_id, &self.access_token)
    }

    fn trimmed(&self) -> Self {
        Self {
            account_id: self.account_id.trim().to_owned(),
            access_token: self.access_token.trim().to_owned(),
        }
    }
}

const OANDA_SHARED: Scope = Scope {
    service: OANDA_SERVICE,
    shared_group: true,
};

pub type OandaCredentialsStore = CredentialsStore<OandaCredentials>;

impl OandaCredentialsStore {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self::new(backend, OANDA_SHARED, OANDA_ACCOUNT, None)
    }

    pub fn shared() -> &'static Self {
        static STORE: OnceLock<OandaCredentialsStore> = OnceLock::new();
        STORE.get_or_init(|| Self::with_backend(default_backend()))
    }
}

/// Two fields saved together as one Keychain item.
pub trait CredentialPair: Clone + Default + Serialize + serde::de::DeserializeOwned {
    fn fields(&self) -> (&str, &str);
    fn trimmed(&self) -> Self;

    fn is_configured(&self) -> bool {
        let (first, second) = self.fields();
        !first.trim().is_empty() && !second.trim().is_empty()
    }
}

#[derive(Default)]
struct CredentialsCache<C> {
    credentials: Option<C>,
    exists: Option<bool>,
}

pub struct CredentialsStore<C> {
    backend: Box<dyn KeyBackend>,
    scope: Scope,
    account: &'static str,
    legacy: Option<Scope>,
    cache: Mutex<CredentialsCache<C>>,
}

impl<C: CredentialPair> CredentialsStore<C> {
    fn new(
        backend: Box<dyn KeyBackend>,
        scope: Scope,
        account: &'static str,
        legacy: Option<Scope>,
    ) -> Self {
        Self {
            backend,
            scope,
            account,
            legacy,
            cache: Mutex::default(),
        }
    }

    /// Reads the saved credentials once. The first read may prompt for Keychain access.
    pub fn credentials(&self) -> C {
        let mut cache = lock(&self.cache);
        if let Some(credentials) = &cache.credentials {
            return credentials.clone();
        }
        let credentials = self
            .backend
            .read(self.scope, self.account)
            .map(|text| serde_json::from_str::<C>(&text).ok())
            .unwrap_or_else(|| self.read_legacy())
            .unwrap_or_default();
        cache.credentials = Some(credentials.clone());
        credentials
    }

    fn read_legacy(&self) -> Option<C> {
        let text = self.backend.read(self.legacy?, self.account)?;
        let credentials = serde_json::from_str(&text).ok()?;
        let _ = self.backend.write(self.scope, self.account, &text);
        Some(credentials)
    }

    /// Checks the item's attributes without prompting for Keychain access.
    pub fn is_configured(&self) -> bool {
        let mut cache = lock(&self.cache);
        if let Some(credentials) = &cache.credentials {
            return credentials.is_configured();
        }
        *cache.exists.get_or_insert_with(|| {
            self.backend.exists(self.scope, self.account)
                || self
                    .legacy
                    .is_some_and(|scope| self.backend.exists(scope, self.account))
        })
    }

    /// Saves trimmed credentials. Incomplete credentials remove the saved item.
    pub fn save(&self, credentials: &C) -> Result<(), SecretStoreError> {
        let normalized = credentials.trimmed();
        let mut cache = lock(&self.cache);
        if normalized.is_configured() {
            let text = serde_json::to_string(&normalized)
                .map_err(|error| SecretStoreError(error.to_string()))?;
            self.backend.write(self.scope, self.account, &text)?;
        } else {
            let current = self.backend.delete(self.scope, self.account);
            let legacy = self
                .legacy
                .map_or(Ok(()), |scope| self.backend.delete(scope, self.account));
            current.and(legacy)?;
        }
        cache.credentials = Some(normalized);
        cache.exists = None;
        Ok(())
    }
}

// MARK: CoinMarketCap

const CMC: Scope = Scope {
    service: CMC_SERVICE,
    shared_group: false,
};

#[derive(Default)]
struct CmcCache {
    loaded: bool,
    key: Option<String>,
    exists: Option<bool>,
}

pub struct CoinMarketCapKeyStore {
    backend: Box<dyn KeyBackend>,
    cache: Mutex<CmcCache>,
}

impl CoinMarketCapKeyStore {
    pub fn with_backend(backend: Box<dyn KeyBackend>) -> Self {
        Self {
            backend,
            cache: Mutex::new(CmcCache::default()),
        }
    }

    pub fn shared() -> &'static Self {
        static STORE: OnceLock<CoinMarketCapKeyStore> = OnceLock::new();
        STORE.get_or_init(|| Self::with_backend(default_backend()))
    }

    /// The saved key. The first read may prompt for Keychain access.
    pub fn api_key(&self) -> Option<String> {
        let mut cache = lock(&self.cache);
        if !cache.loaded {
            cache.loaded = true;
            cache.key = self
                .backend
                .read(CMC, CMC_ACCOUNT)
                .filter(|key| !key.is_empty());
        }
        cache.key.clone()
    }

    /// Whether a key is saved, without reading it (so without a prompt).
    pub fn is_configured(&self) -> bool {
        let mut cache = lock(&self.cache);
        if cache.loaded {
            return cache.key.is_some();
        }
        if let Some(exists) = cache.exists {
            return exists;
        }
        let found = self.backend.exists(CMC, CMC_ACCOUNT);
        cache.exists = Some(found);
        found
    }

    /// Saves the trimmed key; an empty one removes it.
    pub fn save(&self, key: &str) -> Result<(), SecretStoreError> {
        let value = key.trim();
        if value.is_empty() {
            return self.remove();
        }
        self.backend.write(CMC, CMC_ACCOUNT, value)?;
        let mut cache = lock(&self.cache);
        *cache = CmcCache {
            loaded: true,
            key: Some(value.to_owned()),
            exists: None,
        };
        drop(cache);
        Ok(())
    }

    pub fn remove(&self) -> Result<(), SecretStoreError> {
        self.backend.delete(CMC, CMC_ACCOUNT)?;
        let mut cache = lock(&self.cache);
        *cache = CmcCache {
            loaded: true,
            key: None,
            exists: None,
        };
        drop(cache);
        Ok(())
    }
}

// MARK: Lookup for the providers

/// Environment names, kept as a developer convenience when nothing is saved in the app.
pub const ALPACA_KEY_ID_ENV: &str = "DEGENVIEW_ALPACA_KEY_ID";
pub const ALPACA_SECRET_KEY_ENV: &str = "DEGENVIEW_ALPACA_SECRET_KEY";
pub const OANDA_ACCOUNT_ID_ENV: &str = "DEGENVIEW_OANDA_ACCOUNT_ID";
pub const OANDA_ACCESS_TOKEN_ENV: &str = "DEGENVIEW_OANDA_ACCESS_TOKEN";
pub const CMC_API_KEY_ENV: &str = "DEGENVIEW_CMC_API_KEY";

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// Whether Alpaca credentials are saved, for anything that only needs to know (a chart picker,
/// the MCP server). The first answer reads the Keychain item's attributes, never its secret, so
/// it never prompts; the store keeps it, so asking again costs nothing.
pub fn alpaca_configured() -> bool {
    AlpacaCredentialsStore::shared().is_configured()
}

/// The Alpaca key id and secret to send: what the user saved in Settings, else the environment.
/// Reads the saved secret only when one exists, so a user who never configured it is not prompted.
pub fn alpaca_credentials_with(
    store: &AlpacaCredentialsStore,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<(String, String)> {
    if store.is_configured() {
        let saved = store.credentials();
        if saved.is_configured() {
            return Some((saved.key_id, saved.secret_key));
        }
    }
    Some((
        non_empty(env(ALPACA_KEY_ID_ENV))?,
        non_empty(env(ALPACA_SECRET_KEY_ENV))?,
    ))
}

pub fn alpaca_credentials() -> Option<(String, String)> {
    alpaca_credentials_with(AlpacaCredentialsStore::shared(), &|name| {
        std::env::var(name).ok()
    })
}

/// Whether Oanda credentials are saved, for anything that only needs to know (a chart picker,
/// the MCP server). Like [`alpaca_configured`]: attributes only, never a prompt.
pub fn oanda_configured() -> bool {
    OandaCredentialsStore::shared().is_configured()
}

/// The Oanda account id and access token to send: what the user saved in Settings, else the
/// environment.
pub fn oanda_credentials_with(
    store: &OandaCredentialsStore,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<(String, String)> {
    if store.is_configured() {
        let saved = store.credentials();
        if saved.is_configured() {
            return Some((saved.account_id, saved.access_token));
        }
    }
    Some((
        non_empty(env(OANDA_ACCOUNT_ID_ENV))?,
        non_empty(env(OANDA_ACCESS_TOKEN_ENV))?,
    ))
}

pub fn oanda_credentials() -> Option<(String, String)> {
    oanda_credentials_with(OandaCredentialsStore::shared(), &|name| {
        std::env::var(name).ok()
    })
}

/// The CoinMarketCap key to send: the saved one, else the environment.
pub fn cmc_api_key_with(
    store: &CoinMarketCapKeyStore,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if store.is_configured()
        && let Some(key) = non_empty(store.api_key())
    {
        return Some(key);
    }
    non_empty(env(CMC_API_KEY_ENV))
}

pub fn cmc_api_key() -> Option<String> {
    cmc_api_key_with(CoinMarketCapKeyStore::shared(), &|name| {
        std::env::var(name).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Shares one `MemoryBackend` between a store and the test, and counts reads.
    struct Shared(Arc<MemoryBackend>, Arc<AtomicU64>);

    impl KeyBackend for Shared {
        fn read(&self, scope: Scope, account: &str) -> Option<String> {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.read(scope, account)
        }
        fn exists(&self, scope: Scope, account: &str) -> bool {
            self.0.exists(scope, account)
        }
        fn write(&self, scope: Scope, account: &str, value: &str) -> Result<(), SecretStoreError> {
            self.0.write(scope, account, value)
        }
        fn delete(&self, scope: Scope, account: &str) -> Result<(), SecretStoreError> {
            self.0.delete(scope, account)
        }
    }

    fn alpaca() -> (AlpacaCredentialsStore, Arc<MemoryBackend>, Arc<AtomicU64>) {
        let memory = Arc::new(MemoryBackend::default());
        let reads = Arc::new(AtomicU64::new(0));
        let store =
            AlpacaCredentialsStore::with_backend(Box::new(Shared(memory.clone(), reads.clone())));
        (store, memory, reads)
    }

    fn credentials(id: &str, secret: &str) -> AlpacaCredentials {
        AlpacaCredentials {
            key_id: id.into(),
            secret_key: secret.into(),
        }
    }

    #[test]
    fn alpaca_needs_both_halves() {
        assert!(!credentials("", "").is_configured());
        assert!(!credentials("key", "  ").is_configured());
        assert!(credentials(" key ", "secret").is_configured());
    }

    #[test]
    fn saved_alpaca_credentials_are_trimmed_and_read_back() {
        let (store, memory, _) = alpaca();
        assert!(!store.is_configured());
        assert_eq!(store.credentials(), AlpacaCredentials::default());
        store.save(&credentials("  PK123 ", " s3cret\n")).unwrap();
        assert_eq!(store.credentials(), credentials("PK123", "s3cret"));
        assert!(store.is_configured());
        // The Swift app wrote `{"keyID":…,"secretKey":…}`; the format must stay readable by it.
        let stored = memory.read(ALPACA_SHARED, ALPACA_ACCOUNT).unwrap();
        assert_eq!(stored, r#"{"keyID":"PK123","secretKey":"s3cret"}"#);
    }

    #[test]
    fn a_fresh_store_finds_what_an_earlier_session_saved() {
        let (first, memory, _) = alpaca();
        first.save(&credentials("PK1", "S1")).unwrap();
        let reads = Arc::new(AtomicU64::new(0));
        let second = AlpacaCredentialsStore::with_backend(Box::new(Shared(memory, reads.clone())));
        assert!(second.is_configured(), "attributes only");
        assert_eq!(
            reads.load(Ordering::SeqCst),
            0,
            "no secret was read to answer that"
        );
        assert_eq!(second.credentials(), credentials("PK1", "S1"));
        assert_eq!(second.credentials(), credentials("PK1", "S1"));
        assert_eq!(reads.load(Ordering::SeqCst), 1, "read once, then cached");
    }

    #[test]
    fn a_key_found_outside_the_shared_group_moves_into_it() {
        let (store, memory, _) = alpaca();
        memory
            .write(
                ALPACA_OWN,
                ALPACA_ACCOUNT,
                r#"{"keyID":"old","secretKey":"key"}"#,
            )
            .unwrap();
        assert!(store.is_configured());
        assert_eq!(store.credentials(), credentials("old", "key"));
        assert!(
            memory.exists(ALPACA_SHARED, ALPACA_ACCOUNT),
            "copied into the shared group"
        );
    }

    #[test]
    fn unreadable_or_incomplete_alpaca_data_means_not_configured() {
        let (store, memory, _) = alpaca();
        memory
            .write(ALPACA_SHARED, ALPACA_ACCOUNT, "not json")
            .unwrap();
        assert_eq!(store.credentials(), AlpacaCredentials::default());
        store.save(&credentials("PK", "S")).unwrap();
        store.save(&credentials("PK", "")).unwrap();
        assert!(!store.is_configured());
        assert!(
            !memory.exists(ALPACA_SHARED, ALPACA_ACCOUNT),
            "incomplete credentials delete the item"
        );
        assert!(!memory.exists(ALPACA_OWN, ALPACA_ACCOUNT));
    }

    #[test]
    fn the_coinmarketcap_key_saves_trims_and_removes() {
        let memory = Arc::new(MemoryBackend::default());
        let store = CoinMarketCapKeyStore::with_backend(Box::new(Shared(
            memory.clone(),
            Arc::new(AtomicU64::new(0)),
        )));
        assert!(!store.is_configured());
        assert_eq!(store.api_key(), None);
        store.save("  abc-123 \n").unwrap();
        assert_eq!(store.api_key().as_deref(), Some("abc-123"));
        assert!(store.is_configured());
        assert_eq!(memory.read(CMC, CMC_ACCOUNT).as_deref(), Some("abc-123"));
        store.save("   ").unwrap();
        assert_eq!(store.api_key(), None, "an empty key removes it");
        assert!(!store.is_configured());
        assert!(!memory.exists(CMC, CMC_ACCOUNT));
    }

    #[test]
    fn a_new_coinmarketcap_store_answers_without_reading_the_key() {
        let memory = Arc::new(MemoryBackend::default());
        memory.write(CMC, CMC_ACCOUNT, "stored").unwrap();
        let reads = Arc::new(AtomicU64::new(0));
        let store = CoinMarketCapKeyStore::with_backend(Box::new(Shared(memory, reads.clone())));
        assert!(store.is_configured());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(store.api_key().as_deref(), Some("stored"));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    fn oanda() -> (OandaCredentialsStore, Arc<MemoryBackend>) {
        let memory = Arc::new(MemoryBackend::default());
        let store = OandaCredentialsStore::with_backend(Box::new(Shared(
            memory.clone(),
            Arc::new(AtomicU64::new(0)),
        )));
        (store, memory)
    }

    fn oanda_credentials(id: &str, token: &str) -> OandaCredentials {
        OandaCredentials {
            account_id: id.into(),
            access_token: token.into(),
        }
    }

    #[test]
    fn oanda_needs_both_halves_and_saves_them_trimmed() {
        let (store, memory) = oanda();
        assert!(!oanda_credentials("", "").is_configured());
        assert!(!oanda_credentials("001-001", "  ").is_configured());
        assert!(oanda_credentials(" 001-001 ", "tok").is_configured());
        assert!(!store.is_configured());
        store
            .save(&oanda_credentials(" 001-001 ", " tok\n"))
            .unwrap();
        assert_eq!(
            store.credentials(),
            oanda_credentials("001-001", "tok"),
            "saved trimmed"
        );
        assert!(store.is_configured());
        assert_eq!(
            memory.read(OANDA_SHARED, OANDA_ACCOUNT).as_deref(),
            Some(r#"{"accountId":"001-001","accessToken":"tok"}"#)
        );
        store.save(&oanda_credentials("001-001", "")).unwrap();
        assert!(
            !store.is_configured(),
            "incomplete credentials delete the item"
        );
        assert!(!memory.exists(OANDA_SHARED, OANDA_ACCOUNT));
    }

    #[test]
    fn oanda_prefers_the_saved_credentials_and_falls_back_to_the_environment() {
        let (store, _) = oanda();
        let env = |name: &str| match name {
            OANDA_ACCOUNT_ID_ENV => Some(" env-account ".to_owned()),
            OANDA_ACCESS_TOKEN_ENV => Some("env-token".to_owned()),
            _ => None,
        };
        assert_eq!(
            oanda_credentials_with(&store, &env),
            Some(("env-account".to_owned(), "env-token".to_owned())),
            "nothing saved: the environment"
        );
        store
            .save(&oanda_credentials("saved-account", "saved-token"))
            .unwrap();
        assert_eq!(
            oanda_credentials_with(&store, &env),
            Some(("saved-account".to_owned(), "saved-token".to_owned()))
        );
        assert_eq!(oanda_credentials_with(&oanda().0, &|_| None), None);
        let half = |name: &str| (name == OANDA_ACCOUNT_ID_ENV).then(|| "only-id".to_owned());
        assert_eq!(oanda_credentials_with(&oanda().0, &half), None);
    }

    #[test]
    fn under_test_the_shared_stores_never_touch_the_keychain() {
        assert!(is_disabled());
        let store = CoinMarketCapKeyStore::shared();
        store.save("session-only").unwrap();
        assert_eq!(store.api_key().as_deref(), Some("session-only"));
        store.remove().unwrap();
        let alpaca = AlpacaCredentialsStore::shared();
        alpaca.save(&credentials("PK", "S")).unwrap();
        assert!(alpaca.is_configured());
        alpaca.save(&AlpacaCredentials::default()).unwrap();
        let oanda = OandaCredentialsStore::shared();
        oanda.save(&oanda_credentials("A", "T")).unwrap();
        assert!(oanda.is_configured());
        oanda.save(&OandaCredentials::default()).unwrap();
        assert!(!oanda.is_configured());
    }

    #[test]
    fn providers_prefer_the_saved_key_and_fall_back_to_the_environment() {
        let (store, _, _) = alpaca();
        let env = |name: &str| match name {
            ALPACA_KEY_ID_ENV => Some(" env-id ".to_owned()),
            ALPACA_SECRET_KEY_ENV => Some("env-secret".to_owned()),
            CMC_API_KEY_ENV => Some("env-cmc".to_owned()),
            _ => None,
        };
        assert_eq!(
            alpaca_credentials_with(&store, &env),
            Some(("env-id".to_owned(), "env-secret".to_owned())),
            "nothing saved: the environment"
        );
        store
            .save(&credentials("saved-id", "saved-secret"))
            .unwrap();
        assert_eq!(
            alpaca_credentials_with(&store, &env),
            Some(("saved-id".to_owned(), "saved-secret".to_owned()))
        );
        assert_eq!(alpaca_credentials_with(&alpaca().0, &|_| None), None);
        let half = |name: &str| (name == ALPACA_KEY_ID_ENV).then(|| "only-id".to_owned());
        assert_eq!(
            alpaca_credentials_with(&alpaca().0, &half),
            None,
            "both halves are needed"
        );

        let cmc = CoinMarketCapKeyStore::with_backend(Box::new(MemoryBackend::default()));
        assert_eq!(cmc_api_key_with(&cmc, &env).as_deref(), Some("env-cmc"));
        cmc.save("saved-cmc").unwrap();
        assert_eq!(cmc_api_key_with(&cmc, &env).as_deref(), Some("saved-cmc"));
        cmc.remove().unwrap();
        assert_eq!(cmc_api_key_with(&cmc, &|_| None), None);
        assert_eq!(cmc_api_key_with(&cmc, &|_| Some("  ".to_owned())), None);
    }
}
