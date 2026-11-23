//! The interface language: reads the saved choice at startup, and applies and saves a new one from
//! Settings. `i18n` holds the language in use; this is what chooses it.

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
}
