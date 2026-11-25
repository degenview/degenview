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
const LANGUAGES: [Language; 4] = [
    Language {
        tag: "en",
        native_name: "English",
        kind: Kind::Source,
        catalog: None,
    },
    Language {
        tag: "es",
        native_name: "Español",
        kind: Kind::Natural,
        catalog: Some(include_str!("../../assets/locales/es.po")),
    },
    Language {
        tag: "zh",
        native_name: "简体中文",
        kind: Kind::Natural,
        catalog: Some(include_str!("../../assets/locales/zh.po")),
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
    pub const PSEUDO: Locale = Locale(3);

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

    /// The first of `requested` (the system's languages, most wanted first) that is among
    /// `offered`: by exact tag, then along the tag's CLDR fallback chain (`es-MX` gets Spanish).
    /// English when none is. A language that is not offered is never chosen, whatever the system
    /// says: the pseudo-locale, for one, is a tool of debug builds.
    pub fn negotiate<S: AsRef<str>>(requested: &[S], offered: &[Locale]) -> Locale {
        let fallbacker = LocaleFallbacker::new();
        let tags: Vec<&str> = offered.iter().map(|locale| locale.tag()).collect();
        requested
            .iter()
            .find_map(|tag| fallback_match(&fallbacker, tag.as_ref(), &tags))
            .map_or(Locale::ENGLISH, |index| offered[index])
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

/// The locale the interface uses: the one named for this run (`DEGENVIEW_LOCALE`, which may name
/// any language), else the person's choice when this build offers it, else the first of the
/// system's languages that is offered. `offered` is [`Locale::selectable`]: what the picker lists
/// is what can be in use, so a choice the picker cannot show (a pseudo-locale a debug build saved,
/// read by a release build) is left in the store and not applied.
pub fn resolve<S: AsRef<str>>(
    preference: super::Preference,
    forced: Option<Locale>,
    system: &[S],
    offered: &[Locale],
) -> Locale {
    if let Some(locale) = forced {
        return locale;
    }
    match preference {
        super::Preference::Fixed(locale) if offered.contains(&locale) => locale,
        super::Preference::Fixed(_) | super::Preference::System => {
            Locale::negotiate(system, offered)
        }
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

    const BOTH: [Locale; 2] = [Locale::ENGLISH, Locale::PSEUDO];
    const ENGLISH_ONLY: [Locale; 1] = [Locale::ENGLISH];

    #[test]
    fn the_system_languages_are_matched_in_order_by_tag_then_fallback() {
        assert_eq!(Locale::negotiate(&["en-US", "es"], &BOTH), Locale::ENGLISH);
        assert_eq!(
            Locale::negotiate(&["de-DE", "en-GB"], &BOTH),
            Locale::ENGLISH
        );
        assert_eq!(Locale::negotiate(&["en_AU"], &BOTH), Locale::ENGLISH);
        assert_eq!(Locale::negotiate::<&str>(&[], &BOTH), Locale::ENGLISH);
        assert_eq!(Locale::negotiate(&["xx"], &BOTH), Locale::ENGLISH);
        assert_eq!(Locale::negotiate(&["not a tag"], &BOTH), Locale::ENGLISH);
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
    fn chinese_is_simplified_and_answers_every_region_that_writes_it_and_no_other_script() {
        let fallbacker = LocaleFallbacker::new();
        let have = ["es", "zh"];
        let found = |requested| fallback_match(&fallbacker, requested, &have).map(|at| have[at]);
        for requested in ["zh", "zh-CN", "zh_CN", "zh-SG", "zh-Hans", "zh-Hans-CN"] {
            assert_eq!(found(requested), Some("zh"), "{requested}");
        }
        for requested in ["zh-TW", "zh-HK", "zh-Hant", "zh-Hant-TW"] {
            assert_eq!(found(requested), None, "{requested}");
        }
    }

    #[test]
    fn the_pseudo_locale_is_only_chosen_by_its_exact_tag() {
        assert_eq!(Locale::negotiate(&["en-XA"], &BOTH), Locale::PSEUDO);
        assert_eq!(Locale::negotiate(&["en_xa"], &BOTH), Locale::PSEUDO);
        // A machine set to English never lands on it.
        assert_eq!(Locale::negotiate(&["en"], &BOTH), Locale::ENGLISH);
        assert_eq!(Locale::negotiate(&["en-US"], &BOTH), Locale::ENGLISH);
    }

    #[test]
    fn a_language_the_build_does_not_offer_is_never_chosen() {
        // A release build offers English only.
        assert_eq!(
            Locale::negotiate(&["en-XA"], &ENGLISH_ONLY),
            Locale::ENGLISH
        );
        assert_eq!(
            Locale::negotiate(&["en-XA", "en"], &ENGLISH_ONLY),
            Locale::ENGLISH
        );
    }

    #[test]
    fn a_locale_named_for_the_run_beats_the_choice_and_the_system() {
        let system = ["en-US"];
        let pseudo = Preference::Fixed(Locale::PSEUDO);
        let english = Preference::Fixed(Locale::ENGLISH);
        assert_eq!(
            resolve(english, Some(Locale::PSEUDO), &system, &BOTH),
            Locale::PSEUDO
        );
        // The override is explicit: it names a language the build does not list, too.
        assert_eq!(
            resolve(
                Preference::System,
                Some(Locale::PSEUDO),
                &system,
                &ENGLISH_ONLY
            ),
            Locale::PSEUDO
        );
        assert_eq!(resolve(pseudo, None, &system, &BOTH), Locale::PSEUDO);
        assert_eq!(
            resolve(Preference::System, None, &["en-XA"], &BOTH),
            Locale::PSEUDO
        );
        assert_eq!(
            resolve(Preference::System, None, &system, &BOTH),
            Locale::ENGLISH
        );
    }

    #[test]
    fn a_choice_the_build_does_not_offer_is_not_applied() {
        // The pseudo-locale a debug build saved, read by a release build that offers English only.
        let saved = Preference::Fixed(Locale::PSEUDO);
        assert_eq!(
            resolve(saved, None, &["en-US"], &ENGLISH_ONLY),
            Locale::ENGLISH
        );
        assert_eq!(
            resolve(saved, None, &["en-XA"], &ENGLISH_ONLY),
            Locale::ENGLISH
        );
        assert_eq!(resolve(saved, None, &["en-US"], &BOTH), Locale::PSEUDO);
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
        assert_eq!(Locale::from_index(Locale::PSEUDO.index()), Locale::PSEUDO);
        assert_eq!(Locale::from_index(200), Locale::ENGLISH);
    }
}
