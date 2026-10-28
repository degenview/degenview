//! Secret storage on Linux, behind the same `platform::{read, exists, write, delete}` calls the
//! macOS Keychain answers. Items live in the desktop's Secret Service (GNOME Keyring, KWallet), or,
//! inside a Flatpak, in an app-private keyring opened with a key the Secret portal hands out; the
//! `oo7` crate picks between the two.
//!
//! `oo7` is async, and this crate keeps async runtimes inside `src/mcp/`, so the calls run on one
//! worker thread that owns a private current-thread runtime. Callers send a request and wait for
//! the answer with a timeout, as they would for any other blocking call.
//!
//! Only the `oo7` half is Linux-specific. The worker, its timeouts and its caches are plain Rust
//! and are tested on every platform with a fake keyring.
//!
//! Three things keep a missing or locked keyring from stalling the interface:
//! - the keyring is opened on the first request, and a failure to open it is remembered for
//!   [`RETRY_OPEN_AFTER`] instead of retried on every call;
//! - `exists` is answered from a cache the app's own writes keep current, because the webhook list
//!   asks once per row every frame;
//! - `exists` and `delete` give up after [`QUICK`], while `read` and `write` wait [`PATIENT`] for
//!   the user to answer an unlock prompt.
//!
//! The Keychain's access groups do not exist here, so `Scope::shared_group` is ignored: an item is
//! identified by its service and account alone. Under `DEGENVIEW_NO_KEYCHAIN` the callers never
//! reach this module (`keychain_webhook_secrets::is_disabled`).

// Off Linux nothing but the tests reaches the worker.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::service::keychain_webhook_secrets::Scope;
use crate::service::webhook_delivery::SecretStoreError;
use std::collections::HashMap;
use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// Attribute that marks every item as this app's.
#[cfg(target_os = "linux")]
const APPLICATION: &str = "es.nico.degenview";
/// How long `exists` and `delete` wait: neither may prompt.
const QUICK: Duration = Duration::from_secs(10);
/// How long `read` and `write` wait: the keyring may be locked and ask the user to unlock it.
const PATIENT: Duration = Duration::from_secs(120);
/// How long a failure to open the keyring is believed.
const RETRY_OPEN_AFTER: Duration = Duration::from_secs(30);

/// One stored secret: the service and account the callers use, nothing else.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ItemId {
    service: &'static str,
    account: String,
}

impl ItemId {
    fn new(scope: Scope, account: &str) -> Self {
        Self {
            service: scope.service,
            account: account.to_owned(),
        }
    }
}

/// The keyring as the worker needs it. `oo7` implements it; the tests use a fake.
trait Vault {
    async fn read(&mut self, id: &ItemId) -> Result<Option<String>, String>;
    async fn exists(&mut self, id: &ItemId) -> Result<bool, String>;
    async fn write(&mut self, id: &ItemId, value: &str) -> Result<(), String>;
    async fn delete(&mut self, id: &ItemId) -> Result<(), String>;
}

enum Request {
    Read(ItemId, mpsc::Sender<Result<Option<String>, String>>),
    Exists(ItemId, mpsc::Sender<Result<bool, String>>),
    Write(ItemId, String, mpsc::Sender<Result<(), String>>),
    Delete(ItemId, mpsc::Sender<Result<(), String>>),
}

/// Handle to the worker thread.
struct Client {
    requests: UnboundedSender<Request>,
    exists: Mutex<HashMap<ItemId, Remembered>>,
}

/// An answer to "does it exist", and until when it is believed.
#[derive(Clone, Copy)]
struct Remembered {
    found: bool,
    until: Option<Instant>,
}

impl Client {
    /// Starts the worker. `open` runs on it, when the first request arrives and again after a
    /// failure has been believed for [`RETRY_OPEN_AFTER`].
    fn spawn<V, F, Fut>(open: F) -> Self
    where
        V: Vault + 'static,
        F: Fn() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<V, String>> + 'static,
    {
        let (requests, mut inbox) = unbounded_channel::<Request>();
        let spawned = std::thread::Builder::new()
            .name("secret-service".to_owned())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(async move {
                    let mut vault: Option<V> = None;
                    let mut failed: Option<(Instant, String)> = None;
                    while let Some(request) = inbox.recv().await {
                        if vault.is_none() {
                            match &failed {
                                Some((at, _)) if at.elapsed() < RETRY_OPEN_AFTER => {}
                                _ => match open().await {
                                    Ok(opened) => {
                                        vault = Some(opened);
                                        failed = None;
                                    }
                                    Err(reason) => failed = Some((Instant::now(), reason)),
                                },
                            }
                        }
                        let unavailable = || {
                            failed
                                .as_ref()
                                .map(|(_, reason)| reason.clone())
                                .unwrap_or_else(|| "the keyring is unavailable".to_owned())
                        };
                        // A dropped reply channel is a caller that timed out; the work is done anyway.
                        match request {
                            Request::Read(id, reply) => {
                                let _ = reply.send(match vault.as_mut() {
                                    Some(vault) => vault.read(&id).await,
                                    None => Err(unavailable()),
                                });
                            }
                            Request::Exists(id, reply) => {
                                let _ = reply.send(match vault.as_mut() {
                                    Some(vault) => vault.exists(&id).await,
                                    None => Err(unavailable()),
                                });
                            }
                            Request::Write(id, value, reply) => {
                                let _ = reply.send(match vault.as_mut() {
                                    Some(vault) => vault.write(&id, &value).await,
                                    None => Err(unavailable()),
                                });
                            }
                            Request::Delete(id, reply) => {
                                let _ = reply.send(match vault.as_mut() {
                                    Some(vault) => vault.delete(&id).await,
                                    None => Err(unavailable()),
                                });
                            }
                        }
                    }
                });
            });
        // Without a thread there is no keyring: every request finds the channel closed.
        drop(spawned);
        Self {
            requests,
            exists: Mutex::new(HashMap::new()),
        }
    }

    fn ask<T>(
        &self,
        timeout: Duration,
        build: impl FnOnce(mpsc::Sender<Result<T, String>>) -> Request,
    ) -> Result<T, String> {
        let (reply, answer) = mpsc::channel();
        self.requests
            .send(build(reply))
            .map_err(|_| "the keyring is unavailable".to_owned())?;
        match answer.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err("the keyring did not answer".to_owned()),
            Err(RecvTimeoutError::Disconnected) => Err("the keyring is unavailable".to_owned()),
        }
    }

    fn forget(&self, id: &ItemId) {
        if let Ok(mut known) = self.exists.lock() {
            known.remove(id);
        }
    }

    fn remember(&self, id: &ItemId, found: bool) {
        if let Ok(mut known) = self.exists.lock() {
            known.insert(id.clone(), Remembered { found, until: None });
        }
    }

    fn read(&self, id: ItemId) -> Option<String> {
        let value = self
            .ask(PATIENT, |reply| Request::Read(id.clone(), reply))
            .ok()
            .flatten();
        if value.is_some() {
            self.remember(&id, true);
        }
        value
    }

    fn exists(&self, id: ItemId) -> bool {
        let now = Instant::now();
        if let Ok(known) = self.exists.lock()
            && let Some(known) = known.get(&id)
            && known.until.is_none_or(|until| now < until)
        {
            return known.found;
        }
        let answer = self.ask(QUICK, |reply| Request::Exists(id.clone(), reply));
        let remembered = match answer {
            Ok(found) => Remembered { found, until: None },
            // Believe a failure briefly, so a machine with no keyring is asked once, not every frame.
            Err(_) => Remembered {
                found: false,
                until: Some(now + RETRY_OPEN_AFTER),
            },
        };
        if let Ok(mut known) = self.exists.lock() {
            known.insert(id, remembered);
        }
        remembered.found
    }

    fn write(&self, id: ItemId, value: &str) -> Result<(), SecretStoreError> {
        self.forget(&id);
        self.ask(PATIENT, |reply| {
            Request::Write(id.clone(), value.to_owned(), reply)
        })
        .map_err(|reason| SecretStoreError(format!("Could not save to the keyring: {reason}")))?;
        self.remember(&id, true);
        Ok(())
    }

    fn delete(&self, id: ItemId) {
        self.forget(&id);
        if self
            .ask(QUICK, |reply| Request::Delete(id.clone(), reply))
            .is_ok()
        {
            self.remember(&id, false);
        }
    }
}

// MARK: oo7

#[cfg(target_os = "linux")]
struct Oo7Vault {
    keyring: oo7::Keyring,
}

#[cfg(target_os = "linux")]
impl Oo7Vault {
    async fn open() -> Result<Self, String> {
        let keyring = oo7::Keyring::new()
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self { keyring })
    }

    fn attributes(id: &ItemId) -> HashMap<String, String> {
        HashMap::from([
            ("application".to_owned(), APPLICATION.to_owned()),
            ("service".to_owned(), id.service.to_owned()),
            ("account".to_owned(), id.account.clone()),
        ])
    }

    async fn find(&self, id: &ItemId) -> Result<Option<oo7::Item>, String> {
        let found = self
            .keyring
            .search_items(&Self::attributes(id))
            .await
            .map_err(|error| error.to_string())?;
        Ok(found.into_iter().next())
    }

    /// Asks the desktop to unlock a locked keyring (it may prompt). Not needed to find an item.
    async fn unlock(&self) -> Result<(), String> {
        self.keyring
            .unlock()
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "linux")]
impl Vault for Oo7Vault {
    async fn read(&mut self, id: &ItemId) -> Result<Option<String>, String> {
        let Some(item) = self.find(id).await? else {
            return Ok(None);
        };
        self.unlock().await?;
        let secret = item.secret().await.map_err(|error| error.to_string())?;
        Ok(String::from_utf8(secret.as_bytes().to_vec()).ok())
    }

    async fn exists(&mut self, id: &ItemId) -> Result<bool, String> {
        Ok(self.find(id).await?.is_some())
    }

    async fn write(&mut self, id: &ItemId, value: &str) -> Result<(), String> {
        self.unlock().await?;
        self.keyring
            .create_item(
                "DegenView",
                &Self::attributes(id),
                oo7::Secret::text(value),
                true,
            )
            .await
            .map_err(|error| error.to_string())
    }

    async fn delete(&mut self, id: &ItemId) -> Result<(), String> {
        self.unlock().await?;
        self.keyring
            .delete(&Self::attributes(id))
            .await
            .map_err(|error| error.to_string())
    }
}

// MARK: Platform

#[cfg(target_os = "linux")]
fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| Client::spawn(Oo7Vault::open))
}

#[cfg(target_os = "linux")]
pub fn read(scope: Scope, account: &str) -> Option<String> {
    client().read(ItemId::new(scope, account))
}

#[cfg(target_os = "linux")]
pub fn exists(scope: Scope, account: &str) -> bool {
    client().exists(ItemId::new(scope, account))
}

#[cfg(target_os = "linux")]
pub fn write(scope: Scope, account: &str, secret: &str) -> Result<(), SecretStoreError> {
    client().write(ItemId::new(scope, account), secret)
}

#[cfg(target_os = "linux")]
pub fn delete(scope: Scope, account: &str) {
    client().delete(ItemId::new(scope, account))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Shelf {
        items: HashMap<ItemId, String>,
        calls: AtomicUsize,
    }

    struct Fake(Arc<Mutex<Shelf>>);

    impl Vault for Fake {
        async fn read(&mut self, id: &ItemId) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().items.get(id).cloned())
        }
        async fn exists(&mut self, id: &ItemId) -> Result<bool, String> {
            let shelf = self.0.lock().unwrap();
            shelf.calls.fetch_add(1, Ordering::SeqCst);
            Ok(shelf.items.contains_key(id))
        }
        async fn write(&mut self, id: &ItemId, value: &str) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .items
                .insert(id.clone(), value.to_owned());
            Ok(())
        }
        async fn delete(&mut self, id: &ItemId) -> Result<(), String> {
            self.0.lock().unwrap().items.remove(id);
            Ok(())
        }
    }

    fn id(account: &str) -> ItemId {
        ItemId {
            service: "es.nico.degenview.test",
            account: account.to_owned(),
        }
    }

    fn working() -> (Client, Arc<Mutex<Shelf>>) {
        let shelf = Arc::new(Mutex::new(Shelf::default()));
        let handle = shelf.clone();
        let client = Client::spawn(move || {
            let shelf = handle.clone();
            async move { Ok(Fake(shelf)) }
        });
        (client, shelf)
    }

    #[test]
    fn a_secret_round_trips_and_is_deleted() {
        let (client, _) = working();
        assert_eq!(client.read(id("a")), None);
        client.write(id("a"), "s3cret ünï/😀").unwrap();
        assert_eq!(client.read(id("a")).as_deref(), Some("s3cret ünï/😀"));
        client.write(id("a"), "replaced").unwrap();
        assert_eq!(client.read(id("a")).as_deref(), Some("replaced"));
        client.delete(id("a"));
        assert_eq!(client.read(id("a")), None);
    }

    #[test]
    fn exists_is_asked_once_until_something_changes() {
        let (client, shelf) = working();
        assert!(!client.exists(id("a")));
        assert!(!client.exists(id("a")));
        assert!(!client.exists(id("a")));
        assert_eq!(shelf.lock().unwrap().calls.load(Ordering::SeqCst), 1);
        client.write(id("a"), "x").unwrap();
        assert!(client.exists(id("a")));
        client.delete(id("a"));
        assert!(!client.exists(id("a")));
        // The writes and the delete updated the answer; none of them cost another question.
        assert_eq!(shelf.lock().unwrap().calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn items_are_told_apart_by_account() {
        let (client, _) = working();
        client.write(id("a"), "one").unwrap();
        client.write(id("b"), "two").unwrap();
        assert_eq!(client.read(id("a")).as_deref(), Some("one"));
        assert_eq!(client.read(id("b")).as_deref(), Some("two"));
    }

    struct Never;

    impl Vault for Never {
        async fn read(&mut self, _: &ItemId) -> Result<Option<String>, String> {
            unreachable!()
        }
        async fn exists(&mut self, _: &ItemId) -> Result<bool, String> {
            unreachable!()
        }
        async fn write(&mut self, _: &ItemId, _: &str) -> Result<(), String> {
            unreachable!()
        }
        async fn delete(&mut self, _: &ItemId) -> Result<(), String> {
            unreachable!()
        }
    }

    #[test]
    fn no_keyring_fails_every_call_without_hanging() {
        let opens = Arc::new(AtomicUsize::new(0));
        let counted = opens.clone();
        let client = Client::spawn::<Never, _, _>(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            async { Err("no Secret Service".to_owned()) }
        });
        assert_eq!(client.read(id("a")), None);
        assert!(!client.exists(id("a")));
        let error = client.write(id("a"), "x").unwrap_err();
        assert!(error.0.contains("no Secret Service"), "{}", error.0);
        client.delete(id("a"));
        // The failure was believed, not retried for each of the four calls.
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }
}
