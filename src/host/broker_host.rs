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
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use degenview_brokers::model::pending::{
    JournalError, PendingJournal, PendingSubmission, SubmissionOutcome,
};
use degenview_brokers::model::reducer::Notice;
use degenview_brokers::{
    AccountId, Broker, BrokerError, BrokerId, BrokerRuntime, BrokerSnapshot, ClientOrderId,
    Credentials, Env, NoticeSink, ProductId, Quote, RuntimeConfig, SecretStore, Session, Waker,
};

use crate::model::broker;
use crate::model::broker::prefs::{AllPrefs, BrokerPrefs};
use crate::service::keychain_broker_secrets::BrokerSecrets;
use crate::store::SharedStore;
use crate::trading::task::Task;

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
        // Keyed by account too: the same client id in two portfolios is two orders.
        entries.retain(|e| {
            !(e.broker == entry.broker
                && e.account == entry.account
                && e.client_id == entry.client_id)
        });
        entries.push(entry.clone());
        self.write(&entries)
    }

    fn resolve(
        &self,
        broker: BrokerId,
        account: &AccountId,
        client_id: &ClientOrderId,
        _outcome: &SubmissionOutcome,
    ) -> Result<(), JournalError> {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.read()?;
        let before = entries.len();
        entries.retain(|e| {
            !(e.broker == broker && &e.account == account && &e.client_id == client_id)
        });
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

/// The account a connected runtime trades in, for tests that place orders through it.
#[cfg(test)]
pub(crate) fn account_of(runtime: &BrokerRuntime) -> AccountId {
    runtime
        .snapshot()
        .account
        .as_ref()
        .map(|account| account.id.clone())
        .expect("a connected runtime names its account")
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

impl Connection {
    /// Credentials are saved but no connection is up (yet).
    #[cfg(test)]
    pub fn is_saved(&self) -> bool {
        matches!(self, Self::Saved)
    }
}

/// The running connections, and which connection the user currently wants of each broker.
///
/// Starting a connection takes time (a Keychain prompt, the store's lock, the network), and the user
/// can disconnect or replace the key meanwhile. Every request for a connection takes a number from
/// `wanted`; a start may publish its runtime only if the number it holds is still the broker's
/// current one, checked under the same lock that publishes it. Disconnecting changes the number, so
/// a start that was in flight can never bring back a connection (and a key) that was removed, and of
/// two keys entered in a row only the newer can end up running.
#[derive(Default)]
struct Slots {
    running: BTreeMap<BrokerId, Arc<BrokerRuntime>>,
    wanted: BTreeMap<BrokerId, u64>,
}

/// What a question to the secret store needs, without the host.
struct Prober {
    secrets: Arc<dyn SecretStore>,
    saved: Arc<Mutex<SavedKeys>>,
    waker: Arc<AppWaker>,
}

impl Prober {
    fn run(&self, id: BrokerId) {
        let found = self.secrets.is_configured(id, PROFILE);
        {
            let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
            saved.known.insert(id, found);
            saved.asked.remove(&id);
        }
        self.waker.wake();
    }
}

/// Work for the secrets thread.
type Job = Box<dyn FnOnce() + Send>;

/// An operation on a connection that is answered later: polled from the frame loop.
pub type Operation = Task<Result<(), String>>;

/// What the secret store last said about each broker, and which questions are still on their way.
#[derive(Default)]
struct SavedKeys {
    known: BTreeMap<BrokerId, bool>,
    asked: std::collections::BTreeSet<BrokerId>,
}

pub struct BrokerHost {
    secrets: Arc<dyn SecretStore>,
    journal: Arc<dyn PendingJournal>,
    /// Where the choices about trading are kept; none in tests that need none.
    store: Option<SharedStore>,
    prefs: Mutex<AllPrefs>,
    env: Env,
    config: fn(RuntimeConfig) -> RuntimeConfig,
    slots: Mutex<Slots>,
    /// Numbers the connections the user asks for, so each is told apart from the one before it.
    epochs: AtomicU64,
    /// Where every read, write and delete of a secret happens, in the order they were asked for.
    jobs: Mutex<Sender<Job>>,
    /// Whether a key is saved, as the secret store last said. `connection()` reads only this.
    saved: Arc<Mutex<SavedKeys>>,
    waker: Arc<AppWaker>,
    notices: Arc<NoticeQueue>,
    /// Bumped on connect and disconnect (runtime changes arrive through the waker).
    changes: AtomicU64,
}

impl BrokerHost {
    pub fn new(
        secrets: Arc<dyn SecretStore>,
        journal: Arc<dyn PendingJournal>,
        store: Option<SharedStore>,
        env: Env,
        config: fn(RuntimeConfig) -> RuntimeConfig,
    ) -> Arc<Self> {
        let prefs = store
            .as_ref()
            .and_then(|store| store.lock().ok()?.setting::<AllPrefs>(broker::prefs::KEY))
            .unwrap_or_default();
        // The secret store may prompt or hang. Everything that touches it runs here, one job at a
        // time and in the order asked, so the interface thread never waits on it and a save
        // followed by a removal cannot swap. The thread ends when the host is dropped.
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("broker-secrets".to_owned())
            .spawn(move || {
                for job in queue {
                    job();
                }
            })
            .expect("the secrets thread");
        Arc::new(Self {
            secrets,
            journal,
            store,
            prefs: Mutex::new(prefs),
            env,
            config,
            slots: Mutex::new(Slots::default()),
            epochs: AtomicU64::new(1),
            jobs: Mutex::new(jobs),
            saved: Arc::new(Mutex::new(SavedKeys::default())),
            waker: Arc::new(AppWaker::default()),
            notices: Arc::new(NoticeQueue::default()),
            changes: AtomicU64::new(0),
        })
    }

    /// The host the application runs on: the operating system's secret store, the real exchange
    /// addresses and the app's store for the journal and the choices. One per process; every window
    /// reaches it through the trading hub.
    pub fn production(store: SharedStore) -> Arc<Self> {
        Self::new(
            Arc::new(SharedSecrets),
            Arc::new(StoreJournal::new(store.clone())),
            Some(store),
            production_env(),
            |config| config,
        )
    }

    /// Lets runtimes ask for a frame. Called once, with the application's context.
    pub fn attach(&self, context: &eframe::egui::Context) {
        let _ = self.waker.context.set(context.clone());
    }

    /// Rises whenever anything a window draws from a broker may have changed.
    pub fn revision(&self) -> u64 {
        self.waker.wakes.load(Ordering::Acquire) + self.changes.load(Ordering::Acquire)
    }

    /// Something a window draws changed (also called by the trading hub when the enabled venues do).
    pub(super) fn changed(&self) {
        self.changes.fetch_add(1, Ordering::AcqRel);
        self.waker.wake();
    }

    /// How the user trades at `id`: confirmations, notifications, chart overlays.
    pub fn prefs(&self, id: BrokerId) -> BrokerPrefs {
        self.prefs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .of(id.as_str())
    }

    /// Applies the choices for `id` at once and saves them (to this Mac only) on a thread of their
    /// own. The Settings page calls this while the shared store is locked, so saving here would wait
    /// for the lock the caller holds. A save that fails leaves the choices applied for this session.
    pub fn set_prefs(self: &Arc<Self>, id: BrokerId, prefs: BrokerPrefs) {
        self.prefs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set(id.as_str(), prefs);
        self.changed();
        let Some(store) = self.store.clone() else {
            return;
        };
        let host = Arc::clone(self);
        std::thread::spawn(move || {
            // The newest choices, read once the store's lock is held: a thread that read before
            // waiting for it would write older choices after newer ones.
            if let Ok(store) = store.lock() {
                let all = host.prefs.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let _ = store.set_setting(broker::prefs::KEY, &all);
            }
        });
    }

    /// Says that something on screen wants this market's best prices. Cheap; call it every frame
    /// the prices are drawn. The connection reads them every couple of seconds for as long as this
    /// goes on, and not at all once it stops: nothing polls for a chart nobody is looking at.
    pub fn want_quote(&self, id: BrokerId, product: &ProductId) {
        if let Some(runtime) = self.runtime(id) {
            runtime.want_quote(product);
        }
    }

    /// The market's best prices, when they are fresh. A price that is too old is `None`, never
    /// shown as if it were current. (The rule is the crate's: `model::quotes`.)
    pub fn quote(&self, id: BrokerId, product: &ProductId) -> Option<Quote> {
        self.runtime(id)?.fresh_quote(product)
    }

    /// Takes the prices that arrived and asks for the ones wanted, once a frame, from the app.
    /// Never waits: a request is a message to the runtime and its answer is picked up next time.
    pub fn tick_quotes(&self) {
        let runtimes: Vec<Arc<BrokerRuntime>> = self.lock().running.values().cloned().collect();
        let mut changed = false;
        for runtime in runtimes {
            changed |= runtime.tick_quotes();
        }
        if changed {
            self.waker.wake();
        }
    }

    /// Every broker this build can connect to.
    pub fn brokers(&self) -> Vec<&'static dyn Broker> {
        degenview_brokers::registry::all()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Slots> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The connection the user wants of this broker right now.
    fn wanted(&self, id: BrokerId) -> u64 {
        self.lock().wanted.get(&id).copied().unwrap_or(0)
    }

    /// Makes every connection that is still starting for this broker out of date, and returns the
    /// number of the next one.
    fn renew(&self, id: BrokerId) -> u64 {
        let next = self.epochs.fetch_add(1, Ordering::AcqRel);
        self.lock().wanted.insert(id, next);
        next
    }

    pub fn runtime(&self, id: BrokerId) -> Option<Arc<BrokerRuntime>> {
        self.lock().running.get(&id).cloned()
    }

    /// The latest snapshot of a connected broker.
    #[allow(dead_code)] // The trading panel reads it next.
    pub fn snapshot(&self, id: BrokerId) -> Option<Arc<BrokerSnapshot>> {
        self.runtime(id).map(|runtime| runtime.snapshot())
    }

    /// The brokers with a running connection.
    #[allow(dead_code)] // The trading panel lists them next.
    pub fn connected(&self) -> Vec<BrokerId> {
        self.lock().running.keys().copied().collect()
    }

    pub fn connection(&self, id: BrokerId) -> Connection {
        if let Some(runtime) = self.runtime(id) {
            Connection::Running(runtime.snapshot())
        } else if self.key_saved(id) {
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

    /// Replaces the record of unconfirmed orders with something that is not one, as a damaged
    /// database might.
    #[cfg(test)]
    pub(crate) fn corrupt_pending_record_for_test(&self) {
        if let Some(store) = &self.store {
            store
                .lock()
                .expect("the store")
                .set_setting(PENDING_KEY, &"not a list of orders")
                .expect("write the damaged record");
        }
    }

    /// Queues `job` for the secrets thread.
    fn on_secrets_thread(&self, job: impl FnOnce() + Send + 'static) {
        let _ = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(Box::new(job));
    }

    /// Whether a key is saved, as far as the secret store has said. Never asks it from the caller's
    /// thread: the first time it is wanted the question is queued and the answer is "no" until it
    /// comes (the screens redraw when it does).
    fn key_saved(&self, id: BrokerId) -> bool {
        let first = {
            let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
            match saved.known.get(&id) {
                Some(known) => return *known,
                None => saved.asked.insert(id),
            }
        };
        if first {
            let probe = self.prober();
            self.on_secrets_thread(move || probe.run(id));
        }
        false
    }

    fn prober(&self) -> Prober {
        Prober {
            secrets: Arc::clone(&self.secrets),
            saved: Arc::clone(&self.saved),
            waker: Arc::clone(&self.waker),
        }
    }

    /// Asks the secret store whether a key is saved and remembers the answer. Secrets thread only.
    fn probe(&self, id: BrokerId) {
        self.prober().run(id);
        self.changed();
    }

    /// Saves checked credentials and starts the connection. A save that fails still connects for
    /// this session; the answer says the credentials were not kept.
    ///
    /// Returns at once: the save runs on the secrets thread (the secret store may prompt or hang),
    /// and the connection starts on a thread of its own after it. The answer is the save's.
    pub fn adopt(self: &Arc<Self>, id: BrokerId, credentials: Credentials) -> Operation {
        // Taken before anything is queued: a disconnect that comes first makes this start stale.
        let epoch = self.renew(id);
        let (answer, receiver) = mpsc::channel();
        let host = Arc::clone(self);
        self.on_secrets_thread(move || {
            // Replaced or disconnected while it waited its turn: nothing to keep.
            let saved = if host.wanted(id) == epoch {
                host.secrets
                    .save(id, PROFILE, &credentials)
                    .map_err(|error| error.0)
            } else {
                Ok(())
            };
            host.probe(id);
            let starter = Arc::clone(&host);
            std::thread::spawn(move || starter.start(id, epoch, credentials));
            let _ = answer.send(saved);
        });
        operation(receiver)
    }

    /// Connects with the saved credentials, if there are any. Everything that touches the secret
    /// store (it may prompt) runs on the secrets thread.
    pub fn connect_saved(self: &Arc<Self>, id: BrokerId) {
        if self.runtime(id).is_some() {
            return;
        }
        // What was wanted when this was asked for. Loading the key may sit on a Keychain prompt for
        // as long as the user takes; if they disconnect or enter another key meanwhile, this start
        // is out of date and publishes nothing.
        let epoch = self.wanted(id);
        let host = Arc::clone(self);
        self.on_secrets_thread(move || {
            host.probe(id);
            if !host.secrets.is_configured(id, PROFILE) {
                return;
            }
            if let Ok(Some(credentials)) = host.secrets.load(id, PROFILE) {
                let starter = Arc::clone(&host);
                std::thread::spawn(move || starter.start(id, epoch, credentials));
            }
        });
    }

    /// Connects a broker that needs no credential (a test environment) on its own thread. Nothing
    /// is read from or written to the secret store. A broker that does need one is left alone.
    pub fn connect_without_credentials(self: &Arc<Self>, id: BrokerId) {
        let takes_none = degenview_brokers::registry::by_id(id)
            .is_some_and(|broker| broker.descriptor().credential_fields.is_empty());
        if !takes_none || self.runtime(id).is_some() {
            return;
        }
        let epoch = self.wanted(id);
        let host = Arc::clone(self);
        std::thread::spawn(move || host.start(id, epoch, Credentials::new()));
    }

    /// Connects every broker that has saved credentials.
    pub fn connect_all_saved(self: &Arc<Self>) {
        for broker in self.brokers() {
            self.connect_saved(broker.descriptor().id);
        }
    }

    fn start(&self, id: BrokerId, epoch: u64, credentials: Credentials) {
        let Some(broker) = degenview_brokers::registry::by_id(id) else {
            return;
        };
        // Out of date before it began: do not even connect with a key that was taken back.
        if self.wanted(id) != epoch {
            return;
        }
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
        {
            let mut slots = self.lock();
            if slots.wanted.get(&id).copied().unwrap_or(0) != epoch {
                // Disconnected, or another key entered, while this was starting: dropping the
                // runtime stops it, and nothing is published.
                return;
            }
            // Replacing a running connection drops the old one, which stops it without waiting.
            slots.running.insert(id, runtime);
        }
        self.changed();
    }

    /// Stops the connection now and removes the saved credentials from this Mac. It does **not**
    /// revoke the key at the broker; the user does that there.
    ///
    /// The connection stops before this returns; the removal runs on the secrets thread, behind
    /// anything asked earlier, and the answer is its result.
    pub fn disconnect(self: &Arc<Self>, id: BrokerId) -> Operation {
        // Dropped, not joined: the interface thread never waits on a network request. The wanted
        // connection changes first, under the lock that publishes one, so a start that is still in
        // flight finds itself out of date instead of putting the connection back.
        let stopped = {
            let next = self.epochs.fetch_add(1, Ordering::AcqRel);
            let mut slots = self.lock();
            slots.wanted.insert(id, next);
            slots.running.remove(&id)
        };
        drop(stopped);
        self.changed();
        let (answer, receiver) = mpsc::channel();
        let host = Arc::clone(self);
        self.on_secrets_thread(move || {
            let removed = host.secrets.delete(id, PROFILE).map_err(|error| error.0);
            host.probe(id);
            let _ = answer.send(removed);
        });
        operation(receiver)
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

/// A task answered by whatever arrives on `receiver`; a sender dropped without answering says so.
fn operation(receiver: Receiver<Result<(), String>>) -> Operation {
    Task::new(move || match receiver.try_recv() {
        Ok(answer) => Some(answer),
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => {
            Some(Err("The secret store stopped answering.".to_owned()))
        }
    })
}

/// The environment live trading runs in: the real brokers. A developer build with
/// `dev-mock-broker` can point it at a local mock exchange instead (loopback only: the crate refuses
/// anything else, so a stray variable can never send a signed request elsewhere).
fn production_env() -> Env {
    #[allow(unused_mut)]
    let mut env = Env::default();
    #[cfg(feature = "dev-mock-broker")]
    {
        if let Ok(url) = std::env::var("DEGENVIEW_MOCK_BROKER_URL") {
            env = env.with_loopback_base_url(url);
        }
        if let Ok(url) = std::env::var("DEGENVIEW_MOCK_BROKER_WS_URL") {
            env = env.with_loopback_ws_url(url);
        }
    }
    env
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
