//! The app's one view of anonymous usage counting: whether the user allows it, the random install
//! id, and the single send per launch. The request itself is `service::analytics`; the setting,
//! the notice and Settings ▸ About read and change what is here. See `docs/analytics.md`.
//!
//! Nothing is sent until the user has seen the notice (`acknowledged`), and turning counting off
//! forgets the install id. All keys are local to this Mac: none is in `sync::policy`.

use crate::service::analytics::{self, Config, Context};
use crate::service::update_check::Channel;
use crate::service::work_queue;
use crate::store::{SharedStore, Store};
use serde_json::Value;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// `bool`, absent means on: counting is opt-out.
pub const ENABLED_KEY: &str = "analytics.enabled";
/// `bool`: the notice was answered (or the user chose in Settings), so counting may begin.
pub const NOTICE_KEY: &str = "analytics.noticeShown";
/// `String`: the random id GA groups one install's launches by. Removed when counting is turned off.
pub const CLIENT_ID_KEY: &str = "analytics.clientId";
/// `bool`: GA accepted this install's `new_install` event.
pub const COUNTED_KEY: &str = "analytics.installCounted";

/// What the first-run notice says. `ABOUT_TEXT` says the same beside the Settings switch; keep
/// both in step with `analytics::Context`, the only thing ever sent besides the install id.
pub const NOTICE_TEXT: &str = "DegenView counts how many people use it. Once per launch it sends a random install ID, the app version, your operating system and CPU type, and your update channel. Google works out your country from the request; DegenView sends no location. Your charts, symbols, watchlists, portfolios, scripts and keys are never sent. You can turn this off in Settings \u{25B8} About.";

/// The sentence beside the Settings switch.
pub const ABOUT_TEXT: &str = "Sends a random install ID, app version, operating system, CPU type and update channel once per launch. Google derives your country from the request. Nothing about your charts, watchlists, portfolios, scripts or keys is sent.";

/// How long after launch the count waits, so it never competes with startup.
const FIRST_SEND_DELAY: Duration = Duration::from_secs(20);

type Job = Box<dyn FnOnce() + Send + 'static>;
type Submit = fn(Job);
type Transport = Arc<dyn Fn(&Config, &Value) -> bool + Send + Sync>;

struct State {
    /// What this process may send to; `None` in debug and test builds, under an opt-out
    /// environment variable and in builds without GA ids.
    config: Option<Config>,
    enabled: bool,
    acknowledged: bool,
    client_id: Option<String>,
    counted: bool,
    /// This launch has been dealt with (sent, or given up on); never try twice.
    done: bool,
    born: Instant,
}

#[derive(Clone)]
pub struct AnalyticsHost {
    shared: Arc<Mutex<State>>,
    submit: Submit,
    transport: Transport,
}

impl AnalyticsHost {
    pub fn load(store: &SharedStore) -> Self {
        Self::with(
            store,
            analytics::process_config(),
            |job| work_queue::ANALYTICS.submit(job),
            Arc::new(|config, body| analytics::send(config, body).is_ok()),
        )
    }

    fn with(
        store: &SharedStore,
        config: Option<Config>,
        submit: Submit,
        transport: Transport,
    ) -> Self {
        let read = |key: &str| {
            store
                .lock()
                .ok()
                .and_then(|store| store.setting_strict::<bool>(key).ok())
                .flatten()
        };
        let client_id = store
            .lock()
            .ok()
            .and_then(|store| store.setting_strict::<String>(CLIENT_ID_KEY).ok())
            .flatten()
            .filter(|id| !id.is_empty());
        Self {
            shared: Arc::new(Mutex::new(State {
                config,
                enabled: read(ENABLED_KEY).unwrap_or(true),
                acknowledged: read(NOTICE_KEY).unwrap_or(false),
                client_id,
                counted: read(COUNTED_KEY).unwrap_or(false),
                done: false,
                born: Instant::now(),
            })),
            submit,
            transport,
        }
    }

    /// A host that sends nothing, for drawing tests of the UIs that read it.
    #[cfg(test)]
    pub(crate) fn for_test(store: &SharedStore, available: bool) -> Self {
        Self::with(
            store,
            available.then(|| Config::new("G-TEST", "secret")).flatten(),
            |_job| {},
            Arc::new(|_, _| true),
        )
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether this build and environment count anyone at all. Settings shows its row, and the
    /// notice appears, only then: a build that sends nothing has nothing to turn off.
    pub fn available(&self) -> bool {
        self.lock().config.is_some()
    }

    /// The user's choice.
    pub fn enabled(&self) -> bool {
        self.lock().enabled
    }

    /// The notice is due: counting is possible and on, and the user has not yet seen it.
    pub fn notice_pending(&self) -> bool {
        let state = self.lock();
        state.config.is_some() && state.enabled && !state.acknowledged
    }

    /// "OK" on the notice: counting may begin.
    pub fn acknowledge(&self, store: &Store) {
        self.lock().acknowledged = true;
        let _ = store.set_setting(NOTICE_KEY, &true);
    }

    /// The Settings switch, or "Turn Off" on the notice. Choosing either way is an answer to the
    /// notice. Off forgets the install id and that it was counted, so turning it back on later
    /// starts a new anonymous install rather than reviving the old one.
    pub fn set_enabled(&self, store: &Store, enabled: bool) {
        {
            let mut state = self.lock();
            state.enabled = enabled;
            state.acknowledged = true;
            if !enabled {
                state.client_id = None;
                state.counted = false;
            }
        }
        let _ = store.set_setting(ENABLED_KEY, &enabled);
        let _ = store.set_setting(NOTICE_KEY, &true);
        if !enabled {
            let _ = store.remove_setting(CLIENT_ID_KEY);
            let _ = store.remove_setting(COUNTED_KEY);
        }
    }

    /// Called every frame: a few field reads until the one send of this launch is due.
    pub fn tick(&self, store: &SharedStore, channel: Channel) {
        let (config, mut client_id, counted) = {
            let mut state = self.lock();
            let Some(config) = state.config else { return };
            if state.done
                || !state.enabled
                || !state.acknowledged
                || state.born.elapsed() < FIRST_SEND_DELAY
            {
                return;
            }
            state.done = true;
            (config, state.client_id.clone(), state.counted)
        };
        if client_id.is_none() {
            let fresh = crate::store::sync::new_device_id();
            // An id that cannot be kept would count this install again on every launch.
            let saved = store
                .lock()
                .ok()
                .is_some_and(|store| store.set_setting(CLIENT_ID_KEY, &fresh).is_ok());
            if !saved {
                return;
            }
            self.lock().client_id = Some(fresh.clone());
            client_id = Some(fresh);
        }
        let Some(client_id) = client_id else { return };
        let body = analytics::payload(&client_id, !counted, &Context::current(channel));
        let transport = Arc::clone(&self.transport);
        let shared = Arc::clone(&self.shared);
        let store = Arc::clone(store);
        (self.submit)(Box::new(move || {
            if !transport(&config, &body) || counted {
                return;
            }
            // Counted: do not announce this install again. A user who turned counting off while
            // the request was in flight keeps it off, and nothing is written for them. The store
            // is locked before the state, as `set_enabled`'s caller (Settings, under the store
            // lock) does, so the two can never wait on each other.
            let Ok(store) = store.lock() else { return };
            let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
            if state.enabled && state.client_id.as_deref() == Some(client_id.as_str()) {
                state.counted = true;
                let _ = store.set_setting(COUNTED_KEY, &true);
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    fn config() -> Option<Config> {
        Config::new("G-TEST", "secret")
    }

    /// A host that runs its job at once and records each body it would have posted.
    fn host(
        store: &SharedStore,
        config: Option<Config>,
    ) -> (AnalyticsHost, Arc<Mutex<Vec<Value>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&sent);
        let host = AnalyticsHost::with(
            store,
            config,
            |job| job(),
            Arc::new(move |_, body| {
                seen.lock().unwrap().push(body.clone());
                true
            }),
        );
        (host, sent)
    }

    fn ready(host: &AnalyticsHost) {
        host.lock().born = Instant::now() - FIRST_SEND_DELAY - Duration::from_secs(1);
    }

    fn names(body: &Value) -> Vec<String> {
        body["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["name"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn counting_is_on_by_default_but_waits_for_the_notice_and_the_launch_delay() {
        let store = store();
        let (host, sent) = host(&store, config());
        assert!(host.enabled());
        assert!(host.notice_pending());
        ready(&host);
        host.tick(&store, Channel::Stable);
        assert!(sent.lock().unwrap().is_empty(), "sent before the notice");

        host.acknowledge(&store.lock().unwrap());
        assert!(!host.notice_pending());
        let (fresh, sent) = self::host(&store, config());
        fresh.tick(&store, Channel::Stable);
        assert!(
            sent.lock().unwrap().is_empty(),
            "sent inside the launch delay"
        );
        ready(&fresh);
        fresh.tick(&store, Channel::Stable);
        assert_eq!(sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_build_without_ids_sends_nothing_and_shows_nothing() {
        let store = store();
        let (host, sent) = host(&store, None);
        host.acknowledge(&store.lock().unwrap());
        ready(&host);
        host.tick(&store, Channel::Stable);
        assert!(!host.available());
        assert!(!host.notice_pending());
        assert!(sent.lock().unwrap().is_empty());
        assert_eq!(
            store.lock().unwrap().setting::<String>(CLIENT_ID_KEY),
            None,
            "no id is made when nothing is sent"
        );
    }

    #[test]
    fn the_first_launch_counts_the_install_and_later_ones_do_not() {
        let store = store();
        let (first, sent) = host(&store, config());
        first.acknowledge(&store.lock().unwrap());
        ready(&first);
        first.tick(&store, Channel::ReleaseCandidate);
        // The same launch never sends twice.
        first.tick(&store, Channel::ReleaseCandidate);
        let body = sent.lock().unwrap()[0].clone();
        assert_eq!(names(&body), ["new_install", "app_open"]);
        assert_eq!(body["events"][1]["params"]["update_channel"], "rc");
        assert_eq!(sent.lock().unwrap().len(), 1);

        let id = store
            .lock()
            .unwrap()
            .setting::<String>(CLIENT_ID_KEY)
            .unwrap();
        assert_eq!(body["client_id"], id.as_str());

        let (second, sent) = host(&store, config());
        ready(&second);
        second.tick(&store, Channel::Stable);
        let body = sent.lock().unwrap()[0].clone();
        assert_eq!(names(&body), ["app_open"]);
        assert_eq!(body["client_id"], id.as_str(), "the id is kept");
    }

    #[test]
    fn a_failed_post_is_announced_again_next_launch() {
        let store = store();
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let failing = AnalyticsHost::with(
            &store,
            config(),
            |job| job(),
            Arc::new(move |_, _| {
                counter.fetch_add(1, Ordering::SeqCst);
                false
            }),
        );
        failing.acknowledge(&store.lock().unwrap());
        ready(&failing);
        failing.tick(&store, Channel::Stable);
        failing.tick(&store, Channel::Stable);
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "no retry within a launch"
        );
        assert_eq!(store.lock().unwrap().setting::<bool>(COUNTED_KEY), None);

        let (next, sent) = host(&store, config());
        ready(&next);
        next.tick(&store, Channel::Stable);
        assert_eq!(names(&sent.lock().unwrap()[0]), ["new_install", "app_open"]);
        assert_eq!(
            store.lock().unwrap().setting::<bool>(COUNTED_KEY),
            Some(true)
        );
    }

    #[test]
    fn turning_it_off_sends_nothing_and_forgets_the_id() {
        let store = store();
        let (host, sent) = host(&store, config());
        host.acknowledge(&store.lock().unwrap());
        ready(&host);
        host.tick(&store, Channel::Stable);
        assert_eq!(sent.lock().unwrap().len(), 1);
        assert!(
            store
                .lock()
                .unwrap()
                .setting::<String>(CLIENT_ID_KEY)
                .is_some()
        );

        host.set_enabled(&store.lock().unwrap(), false);
        assert!(!host.enabled());
        assert_eq!(store.lock().unwrap().setting::<String>(CLIENT_ID_KEY), None);
        assert_eq!(store.lock().unwrap().setting::<bool>(COUNTED_KEY), None);

        let (after, sent) = self::host(&store, config());
        assert!(!after.enabled(), "the choice survives a restart");
        ready(&after);
        after.tick(&store, Channel::Stable);
        assert!(sent.lock().unwrap().is_empty());
        assert!(!after.notice_pending());
    }

    #[test]
    fn turning_it_off_before_the_notice_answers_the_notice() {
        let store = store();
        let (host, sent) = host(&store, config());
        host.set_enabled(&store.lock().unwrap(), false);
        ready(&host);
        host.tick(&store, Channel::Stable);
        assert!(sent.lock().unwrap().is_empty());
        assert!(!host.notice_pending());
    }

    #[test]
    fn turning_it_back_on_starts_a_new_anonymous_install() {
        let store = store();
        let (host, sent) = host(&store, config());
        host.acknowledge(&store.lock().unwrap());
        ready(&host);
        host.tick(&store, Channel::Stable);
        let old = sent.lock().unwrap()[0]["client_id"].clone();
        host.set_enabled(&store.lock().unwrap(), false);
        host.set_enabled(&store.lock().unwrap(), true);

        let (next, sent) = self::host(&store, config());
        ready(&next);
        next.tick(&store, Channel::Stable);
        let body = sent.lock().unwrap()[0].clone();
        assert_ne!(body["client_id"], old);
        assert_eq!(names(&body), ["new_install", "app_open"]);
    }

    #[test]
    fn none_of_the_keys_syncs() {
        for key in [ENABLED_KEY, NOTICE_KEY, CLIENT_ID_KEY, COUNTED_KEY] {
            assert_eq!(
                crate::sync::policy::setting_policy(key),
                crate::sync::policy::SettingPolicy::Local,
                "{key}"
            );
        }
    }
}
