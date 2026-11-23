//! The interface language: reads the saved choice at startup, and applies and saves a new one from
//! Settings. `i18n` holds the language in use; this is what chooses it.

use std::time::{Duration, Instant};

use crate::i18n::tr;
use crate::i18n::{self, Locale, Preference};
use crate::service::system::language;
use crate::store::Store;

/// Names the language for this run, over the saved choice and the system's: a tag such as `en-XA`
/// (the pseudo-locale), for looking at the interface in another language without changing Settings.
pub const FORCE_ENV: &str = "DEGENVIEW_LOCALE";

/// Puts the saved language in use. Before the first window draws.
pub fn init(store: &Store) {
    apply(store.language_preference());
}

/// How a process that is not the app keeps up with the language chosen in Settings: the alert
/// agent has no window to change it, so it reads the saved choice every couple of seconds and
/// applies it when it changed. Reads only; the app is the one that writes the setting.
pub struct Follower {
    applied: Preference,
    checked: Instant,
}

impl Follower {
    const EVERY: Duration = Duration::from_secs(2);

    /// Puts the saved language in use, as `init` does, and starts following it. The choice is read
    /// once: the one applied is the one remembered, so a change saved a moment later is still seen
    /// as a change by the next poll.
    pub fn start(store: &Store) -> Self {
        let saved = store.language_preference();
        apply(saved);
        Self {
            applied: saved,
            checked: Instant::now(),
        }
    }

    /// Applies the saved language when it is not the one in use. Cheap to call every tick.
    pub fn follow(&mut self, store: &Store) {
        if let Some(preference) = self.changed(store, Instant::now()) {
            apply(preference);
        }
    }

    /// The saved choice when it differs from the one applied, looked at no more often than
    /// `EVERY`.
    fn changed(&mut self, store: &Store, now: Instant) -> Option<Preference> {
        if now.saturating_duration_since(self.checked) < Self::EVERY {
            return None;
        }
        self.checked = now;
        let saved = store.language_preference();
        (saved != self.applied).then(|| {
            self.applied = saved;
            saved
        })
    }
}

/// Puts `preference` in use and saves it. The language is applied even when the save fails, so
/// the window shows what was asked for; the error says it will not last.
pub fn choose(store: &Store, preference: Preference) -> Result<(), String> {
    apply(preference);
    store
        .set_language_preference(preference)
        .map_err(|error| tr!("Could not save the language: {error}", error = error))
}

fn apply(preference: Preference) {
    let forced = std::env::var(FORCE_ENV)
        .ok()
        .and_then(|tag| Locale::from_tag(&tag));
    let locale = i18n::resolve(
        preference,
        forced,
        &language::preferred(),
        &Locale::selectable(),
    );
    i18n::set(preference, locale);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only English is chosen here: another language would change what every test on every thread
    /// reads, and the pseudo-locale is looked at through `i18n::with_locale`.
    #[test]
    fn a_choice_is_applied_and_saved_and_read_back_at_startup() {
        if std::env::var_os(FORCE_ENV).is_some() {
            // A run that names its own language is not what is under test.
            return;
        }
        let store = Store::open_in_memory().expect("store");
        let english = Preference::Fixed(Locale::ENGLISH);
        choose(&store, english).expect("saves");
        assert_eq!(i18n::preference(), english);
        assert_eq!(i18n::current(), Locale::ENGLISH);
        assert_eq!(store.language_preference(), english);

        // A later start reads it from the store.
        choose(&store, Preference::System).expect("saves");
        store.set_language_preference(english).expect("saves");
        init(&store);
        assert_eq!(i18n::preference(), english);

        choose(&store, Preference::System).expect("saves");
        assert_eq!(i18n::preference(), Preference::System);
        assert_eq!(i18n::current(), Locale::ENGLISH);
    }

    #[test]
    fn a_follower_sees_a_change_made_in_the_app_and_not_before_it_is_due() {
        let store = Store::open_in_memory().expect("store");
        // System is the choice of a new store, and resolves to English whatever the machine.
        let mut follower = Follower::start(&store);
        assert_eq!(follower.applied, store.language_preference());
        assert_eq!(follower.applied, Preference::System);
        let later = follower.checked + Follower::EVERY;
        let fixed = Preference::Fixed(Locale::PSEUDO);

        store.set_language_preference(fixed).expect("save");
        // Too soon: nothing is read.
        let soon = follower.checked + Follower::EVERY / 2;
        assert_eq!(follower.changed(&store, soon), None);
        // Due: the new choice is reported once, then it is the one applied.
        assert_eq!(follower.changed(&store, later), Some(fixed));
        let after = later + Follower::EVERY;
        assert_eq!(follower.changed(&store, after), None);

        store
            .set_language_preference(Preference::System)
            .expect("save");
        assert_eq!(
            follower.changed(&store, after + Follower::EVERY),
            Some(Preference::System)
        );
    }
}
