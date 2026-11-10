//! Live trading, as the application sees it: the brokers it can connect to, which are connected,
//! and a handle to each connection's account.
//!
//! The work is in the `degenview-brokers` crate. This module is only the seam:
//!
//! * credentials live in the operating system's secret store ([`BrokerSecrets`]), never anywhere
//!   else, and are read when a connection is made;
//! * an order sent and not yet confirmed is remembered in a **local-only** setting
//!   (`broker.pending`), so a restart cannot lose track of an order that may be live;
//! * each connection's runtime runs on its own thread, owns its network I/O and its account
//!   state, and publishes `Arc` snapshots; this host is shared by every window and is **not** behind
//!   the paper-trading lock, so reading it never waits on a request;
//! * a change anywhere asks for a redraw through [`BrokerHost::revision`], which the app compares
//!   once a frame and uses to repaint every window.
//!
//! Nothing here names a particular broker (a guard test checks); the list comes from the crate's
//! registry. Live trading has no path to Pine scripts, alerts, webhooks or MCP.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock};

use degenview_brokers::model::pending::{
    JournalError, PendingJournal, PendingSubmission, SubmissionOutcome,
};
use degenview_brokers::model::reducer::Notice;
use degenview_brokers::{
    Broker, BrokerError, BrokerId, BrokerRuntime, BrokerSnapshot, ClientOrderId, Credentials, Env,
    NoticeSink, RuntimeConfig, SecretStore, Session, Waker,
};

use crate::service::keychain_broker_secrets::BrokerSecrets;
use crate::store::SharedStore;

/// The setting that holds submissions sent and not yet confirmed.
pub const PENDING_KEY: &str = "broker.pending";
/// The one credential profile in use. A broker that supports several keys at once would add more.
pub const PROFILE: &str = "default";

/// Orders sent and not yet settled, kept in the local database so a restart cannot lose them.
pub struct StoreJournal {
    store: SharedStore,
    /// One writer at a time: read-modify-write of one setting.
    gate: Mutex<()>,
}

impl StoreJournal {
    pub fn new(store: SharedStore) -> Self {
        Self {
            store,
            gate: Mutex::new(()),
        }
    }

    fn read(&self) -> Result<Vec<PendingSubmission>, JournalError> {
        let store = self
            .store
            .lock()
            .map_err(|_| JournalError("the store is unavailable".to_owned()))?;
        // An undecodable record is an error, never "nothing pending": that would forget an order.
        store
            .setting_strict::<Vec<PendingSubmission>>(PENDING_KEY)
            .map(Option::unwrap_or_default)
            .map_err(|error| JournalError(format!("pending orders could not be read: {error}")))
    }

    fn write(&self, entries: &[PendingSubmission]) -> Result<(), JournalError> {
        let store = self
            .store
            .lock()
            .map_err(|_| JournalError("the store is unavailable".to_owned()))?;
        store
            .set_setting(PENDING_KEY, &entries)
            .map_err(|error| JournalError(format!("pending orders could not be saved: {error}")))
    }
}

impl PendingJournal for StoreJournal {
    fn record(&self, entry: &PendingSubmission) -> Result<(), JournalError> {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.read()?;
        entries.retain(|e| !(e.broker == entry.broker && e.client_id == entry.client_id));
        entries.push(entry.clone());
        self.write(&entries)
    }

    fn resolve(
        &self,
        broker: BrokerId,
        client_id: &ClientOrderId,
        _outcome: &SubmissionOutcome,
    ) -> Result<(), JournalError> {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.read()?;
        let before = entries.len();
        entries.retain(|e| !(e.broker == broker && &e.client_id == client_id));
        if entries.len() == before {
            return Ok(());
        }
        self.write(&entries)
    }

    fn unresolved(&self, broker: BrokerId) -> Result<Vec<PendingSubmission>, JournalError> {
        Ok(self
            .read()?
            .into_iter()
            .filter(|entry| entry.broker == broker)
            .collect())
    }
}

/// Asks the interface for a frame whenever a runtime changes something.
#[derive(Default)]
struct AppWaker {
    context: OnceLock<eframe::egui::Context>,
    wakes: AtomicU64,
}

impl Waker for AppWaker {
    fn wake(&self) {
        self.wakes.fetch_add(1, Ordering::AcqRel);
        if let Some(context) = self.context.get() {
            context.request_repaint();
        }
    }
}

/// Notices (an order filled, one cancelled...) waiting for the interface to show them.
#[derive(Default)]
struct NoticeQueue(Mutex<VecDeque<(BrokerId, Notice)>>);

struct TaggedNotices {
    broker: BrokerId,
    queue: Arc<NoticeQueue>,
}

impl NoticeSink for TaggedNotices {
    fn notice(&self, notice: Notice) {
        let mut queue = self.queue.0.lock().unwrap_or_else(|e| e.into_inner());
        queue.push_back((self.broker, notice));
        // A runaway stream must not grow this without bound.
        while queue.len() > 200 {
            queue.pop_front();
        }
    }
}

/// What a broker's row in Settings shows.
#[derive(Clone, Debug)]
pub enum Connection {
    /// No credentials are saved.
    NotConfigured,
    /// Credentials are saved but no connection is running (it is starting, or was disconnected
    /// from this session).
    Saved,
    /// A runtime is running; its snapshot says how it is doing.
    Running(Arc<BrokerSnapshot>),
}

pub struct BrokerHost {
    secrets: Arc<dyn SecretStore>,
    journal: Arc<dyn PendingJournal>,
    env: Env,
    config: fn(RuntimeConfig) -> RuntimeConfig,
    runtimes: Mutex<BTreeMap<BrokerId, Arc<BrokerRuntime>>>,
    waker: Arc<AppWaker>,
    notices: Arc<NoticeQueue>,
    /// Bumped on connect and disconnect (runtime changes arrive through the waker).
    changes: AtomicU64,
}

static GLOBAL: OnceLock<Arc<BrokerHost>> = OnceLock::new();

impl BrokerHost {
    pub fn new(
        secrets: Arc<dyn SecretStore>,
        journal: Arc<dyn PendingJournal>,
        env: Env,
        config: fn(RuntimeConfig) -> RuntimeConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            secrets,
            journal,
            env,
            config,
            runtimes: Mutex::new(BTreeMap::new()),
            waker: Arc::new(AppWaker::default()),
            notices: Arc::new(NoticeQueue::default()),
            changes: AtomicU64::new(0),
        })
    }

    /// The host every window and Settings share, created on first use with the app's store.
    pub fn install(store: SharedStore) -> Arc<Self> {
        Arc::clone(GLOBAL.get_or_init(|| {
            Self::new(
                Arc::new(SharedSecrets),
                Arc::new(StoreJournal::new(store)),
                Env::default(),
                |config| config,
            )
        }))
    }

    /// The shared host, once [`BrokerHost::install`] has run.
    pub fn global() -> Option<Arc<Self>> {
        GLOBAL.get().cloned()
    }

    /// Lets runtimes ask for a frame. Called once, with the application's context.
    pub fn attach(&self, context: &eframe::egui::Context) {
        let _ = self.waker.context.set(context.clone());
    }

    /// Rises whenever anything a window draws from a broker may have changed.
    pub fn revision(&self) -> u64 {
        self.waker.wakes.load(Ordering::Acquire) + self.changes.load(Ordering::Acquire)
    }

    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::AcqRel);
        self.waker.wake();
    }

    /// Every broker this build can connect to.
    pub fn brokers(&self) -> Vec<&'static dyn Broker> {
        degenview_brokers::registry::all()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<BrokerId, Arc<BrokerRuntime>>> {
        self.runtimes.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn runtime(&self, id: BrokerId) -> Option<Arc<BrokerRuntime>> {
        self.lock().get(&id).cloned()
    }

    /// The latest snapshot of a connected broker.
    #[allow(dead_code)] // The trading panel reads it next.
    pub fn snapshot(&self, id: BrokerId) -> Option<Arc<BrokerSnapshot>> {
        self.runtime(id).map(|runtime| runtime.snapshot())
    }

    /// The brokers with a running connection.
    #[allow(dead_code)] // The trading panel lists them next.
    pub fn connected(&self) -> Vec<BrokerId> {
        self.lock().keys().copied().collect()
    }

    pub fn connection(&self, id: BrokerId) -> Connection {
        if let Some(runtime) = self.runtime(id) {
            Connection::Running(runtime.snapshot())
        } else if self.secrets.is_configured(id, PROFILE) {
            Connection::Saved
        } else {
            Connection::NotConfigured
        }
    }

    /// Checks credentials by connecting with them (reads only: nothing is placed or changed), off
    /// the interface thread. Nothing is saved.
    pub fn validate(
        &self,
        id: BrokerId,
        credentials: Credentials,
    ) -> Receiver<Result<Session, BrokerError>> {
        let (sender, receiver) = mpsc::channel();
        let env = self.env.clone();
        let broker = degenview_brokers::registry::by_id(id);
        std::thread::spawn(move || {
            let result = match broker {
                Some(broker) => broker
                    .connect(&credentials, &env)
                    .map(|connection| connection.session().clone()),
                None => Err(BrokerError::invalid("That broker is not available.")),
            };
            let _ = sender.send(result);
        });
        receiver
    }

    /// Saves checked credentials and starts the connection. A save that fails still connects for
    /// this session; the error says the credentials were not kept.
    pub fn adopt(&self, id: BrokerId, credentials: Credentials) -> Result<(), String> {
        let saved = self
            .secrets
            .save(id, PROFILE, &credentials)
            .map_err(|error| error.0);
        self.start(id, credentials);
        saved
    }

    /// Connects with the saved credentials, if there are any. Reads the secret store, which may
    /// prompt, so it runs on its own thread.
    pub fn connect_saved(self: &Arc<Self>, id: BrokerId) {
        if self.runtime(id).is_some() || !self.secrets.is_configured(id, PROFILE) {
            return;
        }
        let host = Arc::clone(self);
        std::thread::spawn(move || {
            if let Ok(Some(credentials)) = host.secrets.load(id, PROFILE) {
                host.start(id, credentials);
            }
        });
    }

    /// Connects every broker that has saved credentials.
    pub fn connect_all_saved(self: &Arc<Self>) {
        for broker in self.brokers() {
            self.connect_saved(broker.descriptor().id);
        }
    }

    fn start(&self, id: BrokerId, credentials: Credentials) {
        let Some(broker) = degenview_brokers::registry::by_id(id) else {
            return;
        };
        let config = (self.config)(RuntimeConfig::new(
            self.env.clone(),
            Arc::clone(&self.journal),
            Arc::clone(&self.waker) as Arc<dyn Waker>,
            Arc::new(TaggedNotices {
                broker: id,
                queue: Arc::clone(&self.notices),
            }),
        ));
        let runtime = Arc::new(BrokerRuntime::start(broker, credentials, config));
        // Replacing a running connection drops the old one, which stops it without waiting.
        self.lock().insert(id, runtime);
        self.changed();
    }

    /// Stops the connection and removes the saved credentials from this Mac. It does **not**
    /// revoke the key at the broker; the user does that there.
    pub fn disconnect(&self, id: BrokerId) -> Result<(), String> {
        // Dropped, not joined: the interface thread never waits on a network request.
        self.lock().remove(&id);
        let removed = self.secrets.delete(id, PROFILE).map_err(|error| error.0);
        self.changed();
        removed
    }

    /// Notices that arrived since the last call, oldest first.
    #[allow(dead_code)] // The notification layer drains them next.
    pub fn drain_notices(&self) -> Vec<(BrokerId, Notice)> {
        self.notices
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }
}

/// The process-wide secret store as a `SecretStore` the host can own.
struct SharedSecrets;

impl SecretStore for SharedSecrets {
    fn load(
        &self,
        broker: BrokerId,
        profile: &str,
    ) -> Result<Option<Credentials>, degenview_brokers::SecretStoreError> {
        BrokerSecrets::shared().load(broker, profile)
    }

    fn save(
        &self,
        broker: BrokerId,
        profile: &str,
        credentials: &Credentials,
    ) -> Result<(), degenview_brokers::SecretStoreError> {
        BrokerSecrets::shared().save(broker, profile, credentials)
    }

    fn delete(
        &self,
        broker: BrokerId,
        profile: &str,
    ) -> Result<(), degenview_brokers::SecretStoreError> {
        BrokerSecrets::shared().delete(broker, profile)
    }

    fn is_configured(&self, broker: BrokerId, profile: &str) -> bool {
        BrokerSecrets::shared().is_configured(broker, profile)
    }
}
