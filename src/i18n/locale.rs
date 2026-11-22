//! The languages the interface can be shown in, and how one is chosen.
//!
//! The languages are one table, [`LANGUAGES`]: a language is a row there and a catalog file
//! beside the others in `assets/locales/`. Tags are BCP 47 (`icu_locale_core`) and a language the
//! system asks for that the app lacks falls back along CLDR's chain (`es-MX` → `es-419` → `es`,
//! `zh-Hant-TW` → `zh-Hant`), as `icu_locale` defines it.

use icu_locale::LocaleFallbacker;
use icu_locale::fallback::{LocaleFallbackConfig, LocaleFallbackerBorrowed};
use icu_locale_core::{LanguageIdentifier, Locale as IcuLocale};

/// What a table row is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    /// The language the source is written in. It has no catalog.
    Source,
    /// A language people read, translated by a catalog.
    // Constructed by the rows `docs/localization.md` tells a language to add.
    #[allow(dead_code)]
    Natural,
    /// A tool for testing the interface, made from the source text (see `pseudo`).
    Pseudo,
}

/// One language of the interface.
struct Language {
    /// The BCP 47 tag, as it is saved and as `DEGENVIEW_LOCALE` names it.
    tag: &'static str,
    /// The language's own name for itself, which is how a language picker lists it: someone who
    /// cannot read the current language must still find theirs.
    native_name: &'static str,
    kind: Kind,
    /// The text of the language's `.po` catalog, compiled in.
    catalog: Option<&'static str>,
}

/// Every language, English first. Add a language here, with its catalog (`docs/localization.md`).
const LANGUAGES: [Language; 2] = [
    Language {
        tag: "en",
        native_name: "English",
        kind: Kind::Source,
        catalog: None,
    },
    Language {
        tag: "en-XA",
        native_name: "Pseudo-locale (en-XA)",
        kind: Kind::Pseudo,
        catalog: None,
    },
];

/// One interface language: a handle on a row of the table, cheap to copy and compare.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Locale(u8);

impl Locale {
    /// The language the source is written in.
    pub const ENGLISH: Locale = Locale(0);
    /// Accented, longer English with every message wrapped in brackets: a language for developers
    /// and tests, which shows clipped text, fixed widths and text that was never put through
    /// `tr!`. `en-XA` is the standard tag for it.
    #[cfg(test)]
    pub const PSEUDO: Locale = Locale(1);

    /// How many languages there are.
    pub(super) const COUNT: usize = LANGUAGES.len();

    /// Every language, English first.
    pub fn all() -> impl Iterator<Item = Locale> {
        (0..LANGUAGES.len() as u8).map(Locale)
    }

    /// The position in the table, for state that stores a language in an atomic.
    pub(super) const fn index(self) -> u8 {
        self.0
    }

    /// The language at `index`, English when there is none.
    pub(super) const fn from_index(index: u8) -> Locale {
        if (index as usize) < LANGUAGES.len() {
            Locale(index)
        } else {
            Self::ENGLISH
        }
    }

    fn row(self) -> &'static Language {
        &LANGUAGES[usize::from(self.0)]
    }

    /// The BCP 47 tag, as it is saved and as `DEGENVIEW_LOCALE` names it.
    pub fn tag(self) -> &'static str {
        self.row().tag
    }

    /// The language's name in that language.
    pub fn native_name(self) -> &'static str {
        self.row().native_name
    }

    /// The compiled-in catalog text, `None` for the source language and the pseudo-locale.
    pub(super) fn catalog_source(self) -> Option<&'static str> {
        self.row().catalog
    }

    /// Whether this is a language people read, and not a tool for testing the interface.
    pub fn is_natural(self) -> bool {
        self.row().kind != Kind::Pseudo
    }

    /// Whether messages are made from the source text and not looked up in a catalog.
    pub(super) fn is_pseudo(self) -> bool {
        self.row().kind == Kind::Pseudo
    }

    /// The languages Settings offers: every natural one, and the pseudo-locale in debug builds.
    pub fn selectable() -> Vec<Locale> {
        Self::all()
            .filter(|locale| locale.is_natural() || cfg!(debug_assertions))
            .collect()
    }

    /// The locale ICU4X formats dates for. The pseudo-locale writes dates as English.
    pub fn icu(self) -> IcuLocale {
        let tag = if self.is_pseudo() { "en" } else { self.tag() };
        tag.parse().unwrap_or(IcuLocale::UNKNOWN)
    }

    /// The locale a saved or typed tag names, whatever its case, and `_` for `-`. Only an exact
    /// tag matches: `negotiate` is what maps `en-US` to English.
    pub fn from_tag(tag: &str) -> Option<Locale> {
        let wanted = parse_tag(tag)?;
        Self::all().find(|locale| parse_tag(locale.tag()).as_ref() == Some(&wanted))
    }

    /// The first of `requested` (the system's languages, most wanted first) that this app has: by
    /// exact tag, then along the tag's fallback chain (`es-MX` gets Spanish). English when none
    /// match. The pseudo-locale is chosen by its exact tag only.
    pub fn negotiate<S: AsRef<str>>(requested: &[S]) -> Locale {
        let fallbacker = LocaleFallbacker::new();
        requested
            .iter()
            .find_map(|tag| {
                Self::from_tag(tag.as_ref())
                    .or_else(|| Self::by_fallback(&fallbacker, tag.as_ref()))
            })
            .unwrap_or(Locale::ENGLISH)
    }

    fn by_fallback(fallbacker: &LocaleFallbackerBorrowed<'_>, tag: &str) -> Option<Locale> {
        let natural: Vec<Locale> = Self::all().filter(|locale| locale.is_natural()).collect();
        let tags: Vec<&str> = natural.iter().map(|locale| locale.tag()).collect();
        fallback_match(fallbacker, tag, &tags).map(|index| natural[index])
    }
}

/// The position in `available` of the first tag on the fallback chain of `requested`.
fn fallback_match(
    fallbacker: &LocaleFallbackerBorrowed<'_>,
    requested: &str,
    available: &[&str],
) -> Option<usize> {
    let locale: IcuLocale = requested.trim().replace('_', "-").parse().ok()?;
    let available: Vec<Option<String>> = available
        .iter()
        .map(|tag| parse_tag(tag).map(|tag| tag.to_string()))
        .collect();
    let mut chain = fallbacker
        .for_config(LocaleFallbackConfig::default())
        .fallback_for((&locale).into());
    while !chain.get().is_unknown() {
        let step = chain.get().to_string();
        let found = available.iter().position(|tag| tag.as_ref() == Some(&step));
        if found.is_some() {
            return found;
        }
        chain.step();
    }
    None
}

fn parse_tag(tag: &str) -> Option<LanguageIdentifier> {
    LanguageIdentifier::try_from_str(&tag.trim().replace('_', "-")).ok()
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
        assert_eq!(Locale::from_tag("en"), Some(Locale::ENGLISH));
        assert_eq!(Locale::from_tag("EN"), Some(Locale::ENGLISH));
        assert_eq!(Locale::from_tag("en_xa"), Some(Locale::PSEUDO));
        assert_eq!(Locale::from_tag(" en-XA "), Some(Locale::PSEUDO));
        assert_eq!(Locale::from_tag("en-US"), None);
        assert_eq!(Locale::from_tag(""), None);
        assert_eq!(Locale::from_tag("not a tag"), None);
    }

    #[test]
    fn every_tag_is_valid_and_names_its_own_locale() {
        for locale in Locale::all() {
            assert!(parse_tag(locale.tag()).is_some(), "{}", locale.tag());
            assert_eq!(Locale::from_tag(locale.tag()), Some(locale));
            assert!(!locale.native_name().is_empty());
        }
    }

    #[test]
    fn the_source_language_and_the_pseudo_locale_have_no_catalog() {
        assert!(Locale::ENGLISH.catalog_source().is_none());
        assert!(Locale::PSEUDO.catalog_source().is_none());
    }

    #[test]
    fn the_system_languages_are_matched_in_order_by_tag_then_fallback() {
        assert_eq!(Locale::negotiate(&["en-US", "es"]), Locale::ENGLISH);
        assert_eq!(Locale::negotiate(&["de-DE", "en-GB"]), Locale::ENGLISH);
        assert_eq!(Locale::negotiate(&["en_AU"]), Locale::ENGLISH);
        assert_eq!(Locale::negotiate::<&str>(&[]), Locale::ENGLISH);
        assert_eq!(Locale::negotiate(&["xx"]), Locale::ENGLISH);
        assert_eq!(Locale::negotiate(&["not a tag"]), Locale::ENGLISH);
    }

    #[test]
    fn a_region_falls_back_along_cldrs_chain_to_a_language_the_app_has() {
        let fallbacker = LocaleFallbacker::new();
        let have = ["pt", "es", "zh-Hant", "de"];
        let found = |requested| fallback_match(&fallbacker, requested, &have).map(|at| have[at]);
        assert_eq!(found("es-MX"), Some("es"));
        assert_eq!(found("es_419"), Some("es"));
        assert_eq!(found("pt-PT"), Some("pt"));
        assert_eq!(found("zh-Hant-TW"), Some("zh-Hant"));
        assert_eq!(found("de-AT"), Some("de"));
        assert_eq!(found("fr-FR"), None);
        // Another script is another language: Simplified Chinese is not Traditional.
        assert_eq!(found("zh-Hans-CN"), None);
    }

    #[test]
    fn the_pseudo_locale_is_only_chosen_by_its_exact_tag() {
        assert_eq!(Locale::negotiate(&["en-XA"]), Locale::PSEUDO);
        // A machine set to English never lands on it.
        assert_eq!(Locale::negotiate(&["en"]), Locale::ENGLISH);
        assert_eq!(Locale::negotiate(&["en-US"]), Locale::ENGLISH);
    }

    #[test]
    fn a_locale_named_for_the_run_beats_the_choice_and_the_system() {
        let system = ["en-US"];
        assert_eq!(
            resolve(
                Preference::Fixed(Locale::ENGLISH),
                Some(Locale::PSEUDO),
                &system
            ),
            Locale::PSEUDO
        );
        assert_eq!(
            resolve(Preference::Fixed(Locale::PSEUDO), None, &system),
            Locale::PSEUDO
        );
        assert_eq!(
            resolve(Preference::System, None, &["en-XA"]),
            Locale::PSEUDO
        );
        assert_eq!(resolve(Preference::System, None, &system), Locale::ENGLISH);
    }

    #[test]
    fn the_picker_lists_natural_languages_and_the_pseudo_locale_only_in_debug() {
        let selectable = Locale::selectable();
        assert!(selectable.contains(&Locale::ENGLISH));
        assert_eq!(selectable.contains(&Locale::PSEUDO), cfg!(debug_assertions));
    }

    #[test]
    fn icu_formats_the_pseudo_locale_as_english() {
        assert_eq!(Locale::PSEUDO.icu().to_string(), "en");
        assert_eq!(Locale::ENGLISH.icu().to_string(), "en");
    }

    #[test]
    fn an_index_past_the_table_is_english() {
        assert_eq!(Locale::from_index(1), Locale::PSEUDO);
        assert_eq!(Locale::from_index(200), Locale::ENGLISH);
    }
}
