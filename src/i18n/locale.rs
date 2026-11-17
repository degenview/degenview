//! The languages the interface can be shown in, and how one is chosen.

use super::plural::Category;

/// One interface language. `English` is the language the source is written in; every other
/// locale is a catalog of translations of the English text (`docs/localization.md`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Locale {
    English,
    /// Accented, longer English with every message wrapped in brackets: a language for developers
    /// and tests, which shows clipped text, fixed widths and text that was never put through
    /// `tr!`. `en-XA` is the standard tag for it.
    Pseudo,
}

impl Locale {
    pub const ALL: [Locale; 2] = [Locale::English, Locale::Pseudo];

    /// The BCP 47 tag, as it is saved and as `DEGENVIEW_LOCALE` names it.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Pseudo => "en-XA",
        }
    }

    /// The language subtag of [`tag`](Self::tag).
    const fn language(self) -> &'static str {
        match self {
            Self::English | Self::Pseudo => "en",
        }
    }

    /// The language's own name for itself, which is how a language picker lists it: someone who
    /// cannot read the current language must still find theirs.
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Pseudo => "Pseudo-locale (en-XA)",
        }
    }

    /// Whether this is a language people read, and not a tool for testing the interface.
    pub const fn is_natural(self) -> bool {
        !matches!(self, Self::Pseudo)
    }

    /// The languages Settings offers: every natural one, and the pseudo-locale in debug builds.
    pub fn selectable() -> Vec<Locale> {
        Self::ALL
            .into_iter()
            .filter(|locale| locale.is_natural() || cfg!(debug_assertions))
            .collect()
    }

    /// The locale a saved or typed tag names, ignoring case and `_` against `-`. Only an exact
    /// tag matches: `negotiate` is what maps `en-US` to English.
    pub fn from_tag(tag: &str) -> Option<Locale> {
        let tag = tag.trim().replace('_', "-");
        Self::ALL
            .into_iter()
            .find(|locale| locale.tag().eq_ignore_ascii_case(&tag))
    }

    /// The first of `requested` (the system's languages, most wanted first) that this app has, by
    /// exact tag and then by language (`es-MX` gets Spanish). English when none match.
    pub fn negotiate<S: AsRef<str>>(requested: &[S]) -> Locale {
        requested
            .iter()
            .find_map(|tag| {
                Self::from_tag(tag.as_ref()).or_else(|| Self::by_language(tag.as_ref()))
            })
            .unwrap_or(Locale::English)
    }

    fn by_language(tag: &str) -> Option<Locale> {
        let language = tag.split(['-', '_']).next()?;
        Self::ALL
            .into_iter()
            .filter(|locale| locale.is_natural())
            .find(|locale| locale.language().eq_ignore_ascii_case(language))
    }

    /// Which plural form of a message the language uses for `count` (CLDR's rules).
    pub fn plural_category(self, count: u64) -> Category {
        match self {
            Self::English | Self::Pseudo => {
                if count == 1 {
                    Category::One
                } else {
                    Category::Other
                }
            }
        }
    }
}

/// The locale the interface uses: the one named for this run (`DEGENVIEW_LOCALE`), else the
/// person's choice, else the first of the system's languages the app has.
pub fn resolve<S: AsRef<str>>(
    preference: super::Preference,
    forced: Option<Locale>,
    system: &[S],
) -> Locale {
    if let Some(locale) = forced {
        return locale;
    }
    match preference {
        super::Preference::Fixed(locale) => locale,
        super::Preference::System => Locale::negotiate(system),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Preference;

    #[test]
    fn a_tag_names_its_locale_whatever_its_case_or_separator() {
        assert_eq!(Locale::from_tag("en"), Some(Locale::English));
        assert_eq!(Locale::from_tag("EN"), Some(Locale::English));
        assert_eq!(Locale::from_tag("en_xa"), Some(Locale::Pseudo));
        assert_eq!(Locale::from_tag(" en-XA "), Some(Locale::Pseudo));
        assert_eq!(Locale::from_tag("en-US"), None);
        assert_eq!(Locale::from_tag(""), None);
    }

    #[test]
    fn every_tag_names_its_own_locale() {
        for locale in Locale::ALL {
            assert_eq!(Locale::from_tag(locale.tag()), Some(locale));
        }
    }

    #[test]
    fn the_system_languages_are_matched_in_order_by_tag_then_language() {
        assert_eq!(Locale::negotiate(&["en-US", "es"]), Locale::English);
        assert_eq!(Locale::negotiate(&["de-DE", "en-GB"]), Locale::English);
        assert_eq!(Locale::negotiate(&["en_AU"]), Locale::English);
        assert_eq!(Locale::negotiate::<&str>(&[]), Locale::English);
        assert_eq!(Locale::negotiate(&["xx"]), Locale::English);
    }

    #[test]
    fn the_pseudo_locale_is_only_chosen_by_its_exact_tag() {
        assert_eq!(Locale::negotiate(&["en-XA"]), Locale::Pseudo);
        // A machine set to English never lands on it.
        assert_eq!(Locale::negotiate(&["en"]), Locale::English);
        assert_eq!(Locale::negotiate(&["en-US"]), Locale::English);
    }

    #[test]
    fn a_locale_named_for_the_run_beats_the_choice_and_the_system() {
        let system = ["en-US"];
        assert_eq!(
            resolve(
                Preference::Fixed(Locale::English),
                Some(Locale::Pseudo),
                &system
            ),
            Locale::Pseudo
        );
        assert_eq!(
            resolve(Preference::Fixed(Locale::Pseudo), None, &system),
            Locale::Pseudo
        );
        assert_eq!(
            resolve(Preference::System, None, &["en-XA"]),
            Locale::Pseudo
        );
        assert_eq!(resolve(Preference::System, None, &system), Locale::English);
    }

    #[test]
    fn english_counts_one_as_singular_and_everything_else_as_other() {
        for locale in Locale::ALL {
            assert_eq!(locale.plural_category(1), Category::One);
            assert_eq!(locale.plural_category(0), Category::Other);
            assert_eq!(locale.plural_category(2), Category::Other);
            assert_eq!(locale.plural_category(1_000_000), Category::Other);
        }
    }

    #[test]
    fn the_picker_lists_natural_languages_and_the_pseudo_locale_only_in_debug() {
        let selectable = Locale::selectable();
        assert!(selectable.contains(&Locale::English));
        assert_eq!(selectable.contains(&Locale::Pseudo), cfg!(debug_assertions));
    }
}
