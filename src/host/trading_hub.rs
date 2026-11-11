//! Which trading venues the user has chosen, and a backend for each.
//!
//! A venue is *enabled* by the user, never by default: a new profile has none, and the trading
//! screens open on a list to choose from. Enabling is remembered in `trading.enabled`, local to
//! this Mac like the broker keys (what is connected here is not another Mac's business). A broker
//! with saved credentials counts as enabled without being listed, so a key saved before this
//! setting existed keeps working.
//!
//! The paper account is enabled on first run of this version for anyone who already has paper
//! trading data, so an upgrade takes nothing away.
//!
//! Nothing here names a particular venue: the brokers come from the crate's registry.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, OnceLock};

use super::broker_host::{BrokerHost, Connection, Operation};
use super::paper_host::PaperHost;
use crate::model::trading_venue::TradingVenue;
use crate::store::SharedStore;
use crate::trading::backend::TradingBackend;
use crate::trading::broker_backend::BrokerBackend;
use crate::trading::model::VenueInfo;
use crate::trading::paper_backend::PaperBackend;
use crate::trading::task::Task;

/// The venues the user enabled, as their ids (`paper`, or a broker's id).
pub const ENABLED_KEY: &str = "trading.enabled";
/// The venue last traded on, so a new tab opens on it.
pub const LAST_USED_KEY: &str = "trading.venue";

const PAPER_ID: &str = "paper";

/// The name `Paper trading` starts with, and the balance, when nothing has been changed.
const DEFAULT_ACCOUNT: &str = "Paper Trading";
const DEFAULT_BALANCE: f64 = 100_000.0;

fn venue_id(venue: TradingVenue) -> String {
    match venue {
        TradingVenue::Paper => PAPER_ID.to_owned(),
        TradingVenue::Broker(id) => id.as_str().to_owned(),
    }
}

#[derive(Default)]
struct Choices {
    enabled: BTreeSet<String>,
    last_used: Option<String>,
}

pub struct TradingHub {
    brokers: Arc<BrokerHost>,
    paper: Arc<Mutex<PaperHost>>,
    store: Option<SharedStore>,
    choices: Mutex<Choices>,
    /// One backend per venue, built once: they hold only handles, and the screens ask for one every
    /// frame.
    backends: Vec<(TradingVenue, Arc<dyn TradingBackend>)>,
}

static GLOBAL: OnceLock<Arc<TradingHub>> = OnceLock::new();

/// Whether the paper account holds anything the user made: orders, fills, positions, more than one
/// account, or an account they renamed or funded differently.
fn paper_was_used(host: &PaperHost) -> bool {
    let snapshot = host.snapshot();
    !snapshot.orders.is_empty()
        || !snapshot.fills.is_empty()
        || !snapshot.positions.is_empty()
        || snapshot.accounts.len() > 1
        || snapshot
            .accounts
            .iter()
            .any(|a| a.name != DEFAULT_ACCOUNT || a.initial_balance != DEFAULT_BALANCE)
}

impl TradingHub {
    pub fn new(
        brokers: Arc<BrokerHost>,
        paper: Arc<Mutex<PaperHost>>,
        store: Option<SharedStore>,
    ) -> Arc<Self> {
        let saved: Option<Vec<String>> = store
            .as_ref()
            .and_then(|store| store.lock().ok()?.setting(ENABLED_KEY));
        let last_used = store
            .as_ref()
            .and_then(|store| store.lock().ok()?.setting::<String>(LAST_USED_KEY));
        let first_run = saved.is_none();
        let mut enabled: BTreeSet<String> = saved.unwrap_or_default().into_iter().collect();
        if first_run && paper_was_used(&paper.lock().unwrap_or_else(|e| e.into_inner())) {
            enabled.insert(PAPER_ID.to_owned());
        }
        let mut backends: Vec<(TradingVenue, Arc<dyn TradingBackend>)> = vec![(
            TradingVenue::Paper,
            Arc::new(PaperBackend::new(Arc::clone(&paper))),
        )];
        for broker in brokers.brokers() {
            backends.push((
                TradingVenue::Broker(broker.descriptor().id),
                Arc::new(BrokerBackend::new(Arc::clone(&brokers), broker)),
            ));
        }
        let hub = Arc::new(Self {
            brokers,
            paper,
            store,
            choices: Mutex::new(Choices { enabled, last_used }),
            backends,
        });
        if first_run {
            // Remember the decision, so a later change in the paper data does not redo it.
            hub.save_enabled();
        }
        hub
    }

    /// The hub every window and Settings share, created on first use.
    pub fn install(
        brokers: Arc<BrokerHost>,
        paper: Arc<Mutex<PaperHost>>,
        store: SharedStore,
    ) -> Arc<Self> {
        Arc::clone(GLOBAL.get_or_init(|| Self::new(brokers, paper, Some(store))))
    }

    pub fn global() -> Option<Arc<Self>> {
        GLOBAL.get().cloned()
    }

    #[cfg(test)]
    pub(crate) fn set_global_for_tests(hub: Arc<Self>) -> Arc<Self> {
        Arc::clone(GLOBAL.get_or_init(|| hub))
    }

    #[cfg(test)]
    pub(crate) fn paper_for_tests(&self) -> Arc<Mutex<PaperHost>> {
        Arc::clone(&self.paper)
    }

    pub fn brokers(&self) -> &Arc<BrokerHost> {
        &self.brokers
    }

    fn choices(&self) -> std::sync::MutexGuard<'_, Choices> {
        self.choices.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The enabled venues changed: every window redraws (the broker host owns the app's counter).
    fn changed(&self) {
        self.brokers.changed();
    }

    /// The backend of a venue, whether or not it is enabled (Settings draws every one).
    pub fn backend(&self, venue: TradingVenue) -> Option<Arc<dyn TradingBackend>> {
        self.backends
            .iter()
            .find(|(known, _)| *known == venue)
            .map(|(_, backend)| Arc::clone(backend))
    }

    /// Every venue this build has, paper first, then the brokers in registry order.
    pub fn all_venues(&self) -> Vec<TradingVenue> {
        let mut venues = vec![TradingVenue::Paper];
        venues.extend(
            self.brokers
                .brokers()
                .into_iter()
                .map(|broker| TradingVenue::Broker(broker.descriptor().id)),
        );
        venues
    }

    pub fn infos(&self) -> Vec<VenueInfo> {
        self.all_venues()
            .into_iter()
            .filter_map(|venue| self.backend(venue).map(|backend| backend.info()))
            .collect()
    }

    pub fn is_enabled(&self, venue: TradingVenue) -> bool {
        if self.choices().enabled.contains(&venue_id(venue)) {
            return true;
        }
        // A key saved before the setting existed.
        matches!(venue, TradingVenue::Broker(id)
            if !matches!(self.brokers.connection(id), Connection::NotConfigured))
    }

    pub fn enabled_venues(&self) -> Vec<TradingVenue> {
        self.all_venues()
            .into_iter()
            .filter(|venue| self.is_enabled(*venue))
            .collect()
    }

    /// The venue a new tab opens on: the one last used if it is still enabled, else the first
    /// enabled one, else none (the list to choose from is shown).
    pub fn default_venue(&self) -> Option<TradingVenue> {
        let enabled = self.enabled_venues();
        let last = self.choices().last_used.clone();
        last.and_then(|last| enabled.iter().copied().find(|v| venue_id(*v) == last))
            .or_else(|| enabled.first().copied())
    }

    pub fn set_last_used(self: &Arc<Self>, venue: TradingVenue) {
        {
            let mut choices = self.choices();
            let id = venue_id(venue);
            if choices.last_used.as_deref() == Some(id.as_str()) {
                return;
            }
            choices.last_used = Some(id);
        }
        self.save_last_used();
    }

    /// Enables a venue and starts it: paper gets its account, a broker that needs no credential
    /// connects now. A broker that needs one is enabled by saving its key in Settings, not here.
    pub fn enable(self: &Arc<Self>, venue: TradingVenue) {
        self.choices().enabled.insert(venue_id(venue));
        self.changed();
        self.save_enabled();
        match venue {
            TradingVenue::Paper => {
                // Creating the account saves it, which takes the store's lock: not on this thread,
                // which may be drawing a window that holds it.
                let paper = Arc::clone(&self.paper);
                std::thread::spawn(move || {
                    let _ = paper
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .ensure_default_account();
                });
            }
            TradingVenue::Broker(id) => self.brokers.connect_without_credentials(id),
        }
    }

    /// Saves a checked key and connects with it, and remembers the venue as enabled. The answer
    /// comes later: it is what the secret store said to the save (the connection starts either way).
    pub fn connect(
        self: &Arc<Self>,
        id: degenview_brokers::BrokerId,
        credentials: degenview_brokers::Credentials,
    ) -> Operation {
        let venue = TradingVenue::Broker(id);
        let added = self.choices().enabled.insert(venue_id(venue));
        if added {
            self.changed();
            self.save_enabled();
        }
        self.brokers.adopt(id, credentials)
    }

    /// Stops using a venue. A broker is disconnected now and its saved key removed from this Mac
    /// (the answer says whether the secret store removed it); paper keeps its data and shows it
    /// again if it is enabled later.
    ///
    /// This, not the broker host, is how a venue is turned off: the choice is remembered here, so a
    /// broker that needs no key is not connected again at the next start.
    pub fn disable(self: &Arc<Self>, venue: TradingVenue) -> Operation {
        self.choices().enabled.remove(&venue_id(venue));
        self.changed();
        self.save_enabled();
        match venue {
            TradingVenue::Broker(id) => self.brokers.disconnect(id),
            TradingVenue::Paper => Task::ready(Ok(())),
        }
    }

    /// Connects what the user enabled, once at start: every saved key, and each enabled broker that
    /// needs none.
    pub fn connect_enabled(self: &Arc<Self>) {
        self.brokers.connect_all_saved();
        for venue in self.enabled_venues() {
            if let TradingVenue::Broker(id) = venue {
                self.brokers.connect_without_credentials(id);
            }
        }
    }

    /// Saves the newest choices on a thread of their own: Settings calls this while the shared
    /// store is locked. Each thread reads the choices once it holds the lock, so whichever runs last
    /// writes the latest, and two quick changes cannot reorder.
    fn save_enabled(self: &Arc<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let hub = Arc::clone(self);
        std::thread::spawn(move || {
            // Read with the store's lock held: a thread that read before waiting for it would write
            // an older choice after a newer one.
            if let Ok(store) = store.lock() {
                let enabled: Vec<String> = hub.choices().enabled.iter().cloned().collect();
                let _ = store.set_setting(ENABLED_KEY, &enabled);
            }
        });
    }

    fn save_last_used(self: &Arc<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let hub = Arc::clone(self);
        std::thread::spawn(move || {
            if let Ok(store) = store.lock() {
                let Some(last) = hub.choices().last_used.clone() else {
                    return;
                };
                let _ = store.set_setting(LAST_USED_KEY, &last);
            }
        });
    }
}
