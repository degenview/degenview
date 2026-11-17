//! The interface language the person chose (Settings ▸ Appearance ▸ Language), kept as one
//! `setting`. It is local to the Mac: `sync::record::policy` does not list it, so a Mac set to
//! another language is not moved by this one.

use super::Store;
use crate::i18n::Preference;

pub const KEY: &str = "language";

impl Store {
    /// The saved choice; follows the system when none was saved or when the saved value is one this
    /// version cannot read (which is left as it is until the person chooses again).
    pub fn language_preference(&self) -> Preference {
        self.setting::<Preference>(KEY).unwrap_or_default()
    }

    pub fn set_language_preference(&self, preference: Preference) -> rusqlite::Result<()> {
        self.set_setting(KEY, &preference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;

    #[test]
    fn nothing_saved_follows_the_system() {
        let store = Store::open_in_memory().expect("store");
        assert_eq!(store.language_preference(), Preference::System);
    }

    #[test]
    fn a_choice_is_read_back() {
        let store = Store::open_in_memory().expect("store");
        let fixed = Preference::Fixed(Locale::Pseudo);
        store.set_language_preference(fixed).expect("save");
        assert_eq!(store.language_preference(), fixed);
        store
            .set_language_preference(Preference::System)
            .expect("save");
        assert_eq!(store.language_preference(), Preference::System);
    }

    #[test]
    fn a_language_this_version_cannot_read_follows_the_system_and_is_kept() {
        let store = Store::open_in_memory().expect("store");
        store.set_setting(KEY, &"tlh").expect("save");
        assert_eq!(store.language_preference(), Preference::System);
        assert_eq!(store.setting::<String>(KEY).as_deref(), Some("tlh"));
    }
}
