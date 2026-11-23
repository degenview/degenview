//! A language's translations: a gettext catalog, `assets/locales/<tag>.po`, compiled in.
//!
//! The format is GNU gettext's, so translators work in the tools they know (Poedit, Weblate,
//! Crowdin, Transifex) and `msgmerge` carries a catalog from one version of the source to the next.
//! The message is its English text (`msgid`); a counted one is found by its singular and gives one
//! `msgstr[i]` per plural form, the form a count takes being what the catalog's own `Plural-Forms`
//! header says. A `tr_c!` context is the entry's `msgctxt`. `messages.pot` is the template for a
//! new language.
//!
//! An entry that is untranslated or marked fuzzy is skipped, as gettext does at run time, so the
//! message shows in English.

use std::collections::HashMap;
use std::ops::Range;

use ferrocat_po::{MsgStr, parse_po};

use super::locale::Locale;
use super::plural_forms::{PluralForms, PluralFormsError};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("the catalog is not a valid .po file: {0}")]
    Po(String),
    #[error(transparent)]
    Plural(#[from] PluralFormsError),
    #[error("{0:?} is plural but gives one translation, or gives forms and is not plural")]
    KindMismatch(String),
    #[error("{key:?} has {found} plural forms and the catalog's header says {wanted}")]
    FormCount {
        key: String,
        found: usize,
        wanted: usize,
    },
}

/// The text of a message in a language: one string, or one per plural form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    Text(&'static str),
    Plural(Vec<&'static str>),
}

/// A catalog ready for lookups.
#[derive(Debug)]
pub struct Catalog {
    /// By context (empty for none), then by English text.
    messages: HashMap<&'static str, HashMap<&'static str, Message>>,
    plural: PluralForms,
}

impl Catalog {
    /// Reads a `.po` file. The text of every translation lives as long as the process (a catalog
    /// is read once per language), so one buffer holds them all and is leaked once, which makes
    /// each a `&'static str` like the English text it replaces.
    pub fn parse(po: &str) -> Result<Catalog, CatalogError> {
        let file = parse_po(po).map_err(|error| CatalogError::Po(error.to_string()))?;
        // gettext's own default when a catalog does not say.
        let plural = match file
            .headers
            .iter()
            .find(|header| header.key == "Plural-Forms")
        {
            Some(header) => PluralForms::from_header(&header.value)?,
            None => PluralForms::english(),
        };

        // Collected as ranges of one buffer, which cannot be sliced until it stops growing.
        struct Entry {
            context: Range<usize>,
            key: Range<usize>,
            forms: Vec<Range<usize>>,
            counted: bool,
        }
        let mut buffer = String::new();
        let mut push = |text: &str| {
            let start = buffer.len();
            buffer.push_str(text);
            start..buffer.len()
        };
        let mut entries = Vec::new();
        for item in &file.items {
            // What gettext leaves out at run time: retired, doubtful and untranslated entries.
            let fuzzy = item.flags.iter().any(|flag| flag == "fuzzy");
            if item.obsolete || fuzzy {
                continue;
            }
            let (forms, counted): (Vec<&str>, bool) = match (&item.msgstr, &item.msgid_plural) {
                (MsgStr::Singular(text), None) => (vec![text], false),
                (MsgStr::Plural(forms), Some(_)) => {
                    if forms.len() != plural.count() {
                        return Err(CatalogError::FormCount {
                            key: item.msgid.clone(),
                            found: forms.len(),
                            wanted: plural.count(),
                        });
                    }
                    (forms.iter().map(String::as_str).collect(), true)
                }
                (MsgStr::None, _) => continue,
                _ => return Err(CatalogError::KindMismatch(item.msgid.clone())),
            };
            if forms.iter().any(|form| form.is_empty()) {
                continue;
            }
            let context = push(item.msgctxt.as_deref().unwrap_or(""));
            let key = push(&item.msgid);
            let forms = forms.into_iter().map(&mut push).collect();
            entries.push(Entry {
                context,
                key,
                forms,
                counted,
            });
        }

        let buffer: &'static str = Box::leak(buffer.into_boxed_str());
        let mut messages: HashMap<&'static str, HashMap<&'static str, Message>> = HashMap::new();
        for entry in entries {
            let forms: Vec<&'static str> = entry
                .forms
                .iter()
                .map(|range| &buffer[range.clone()])
                .collect();
            let message = if entry.counted {
                Message::Plural(forms)
            } else {
                Message::Text(forms[0])
            };
            messages
                .entry(&buffer[entry.context])
                .or_default()
                .insert(&buffer[entry.key], message);
        }
        Ok(Catalog { messages, plural })
    }

    /// The translation of `key`, in `context` (empty when it has none).
    pub fn get(&self, context: &str, key: &str) -> Option<&Message> {
        self.messages.get(context)?.get(key)
    }

    /// The text of a plain message.
    pub fn text(&self, context: &str, key: &str) -> Option<&'static str> {
        match self.get(context, key)? {
            Message::Text(text) => Some(text),
            Message::Plural(_) => None,
        }
    }

    /// The form of the counted message whose singular is `one` that `count` takes.
    pub fn counted(&self, one: &str, count: u64) -> Option<&'static str> {
        match self.get("", one)? {
            Message::Plural(forms) => forms.get(self.plural.index(count)).copied(),
            Message::Text(_) => None,
        }
    }

    /// Every entry, for the tests that keep a catalog and the source in step.
    #[cfg(test)]
    pub fn entries(&self) -> impl Iterator<Item = (&'static str, &'static str, &Message)> {
        self.messages.iter().flat_map(|(context, messages)| {
            messages
                .iter()
                .map(move |(key, message)| (*context, *key, message))
        })
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.messages.values().map(HashMap::len).sum()
    }
}

/// The catalog of `locale`, read once; `None` for English and the pseudo-locale, which are made
/// from the source text. A catalog that does not parse is reported once and then shows English,
/// instead of taking the app down; the tests fail on one, so a release never has it.
pub fn of(locale: Locale) -> Option<&'static Catalog> {
    static CATALOGS: [std::sync::OnceLock<Option<Catalog>>; Locale::COUNT] =
        [const { std::sync::OnceLock::new() }; Locale::COUNT];
    let source = locale.catalog_source()?;
    CATALOGS[usize::from(locale.index())]
        .get_or_init(|| match Catalog::parse(source) {
            Ok(catalog) => Some(catalog),
            Err(error) => {
                eprintln!(
                    "DegenView: the {} catalog is unusable: {error}",
                    locale.tag()
                );
                debug_assert!(false, "the {} catalog is unusable: {error}", locale.tag());
                None
            }
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = r#"msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"
"Language: ru\n"
"Plural-Forms: nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);\n"
"#;

    fn russian(body: &str) -> Result<Catalog, CatalogError> {
        Catalog::parse(&format!("{HEADER}\n{body}"))
    }

    fn sample() -> Catalog {
        russian(
            r#"msgid "Save"
msgstr "Сохранить"

msgctxt "month abbreviation"
msgid "May"
msgstr "май"

msgid "{count} alert"
msgid_plural "{count} alerts"
msgstr[0] "{count} оповещение"
msgstr[1] "{count} оповещения"
msgstr[2] "{count} оповещений"
"#,
        )
        .expect("sample parses")
    }

    #[test]
    fn a_message_is_found_by_its_english_text_and_a_context_by_both() {
        let catalog = sample();
        assert_eq!(catalog.text("", "Save"), Some("Сохранить"));
        assert_eq!(catalog.text("month abbreviation", "May"), Some("май"));
        assert_eq!(catalog.text("", "May"), None);
        assert_eq!(catalog.text("month abbreviation", "Save"), None);
        assert_eq!(catalog.text("", "Open"), None);
        assert_eq!(catalog.len(), 3);
    }

    #[test]
    fn a_counted_message_takes_the_form_the_catalog_header_says() {
        let catalog = sample();
        let form = |n| catalog.counted("{count} alert", n).expect("counted");
        assert_eq!(form(1), "{count} оповещение");
        assert_eq!(form(2), "{count} оповещения");
        assert_eq!(form(5), "{count} оповещений");
        assert_eq!(form(21), "{count} оповещение");
        assert_eq!(form(111), "{count} оповещений");
    }

    #[test]
    fn a_plain_message_is_not_a_counted_one_and_the_other_way_round() {
        let catalog = sample();
        assert_eq!(catalog.counted("Save", 1), None);
        assert_eq!(catalog.text("", "{count} alert"), None);
    }

    #[test]
    fn untranslated_and_fuzzy_entries_are_left_to_english() {
        let catalog = russian(
            r#"msgid "Open"
msgstr ""

#, fuzzy
msgid "Close"
msgstr "Закрыть?"

msgid "Save"
msgstr "Сохранить"
"#,
        )
        .expect("parses");
        assert_eq!(catalog.text("", "Open"), None);
        assert_eq!(catalog.text("", "Close"), None);
        assert_eq!(catalog.text("", "Save"), Some("Сохранить"));
    }

    #[test]
    fn a_counted_message_with_the_wrong_number_of_forms_is_refused() {
        let error = russian(
            r#"msgid "{count} alert"
msgid_plural "{count} alerts"
msgstr[0] "a"
msgstr[1] "b"
"#,
        )
        .expect_err("two forms for three");
        assert!(matches!(
            error,
            CatalogError::FormCount {
                found: 2,
                wanted: 3,
                ..
            }
        ));
    }

    #[test]
    fn a_catalog_that_is_not_po_or_has_a_bad_formula_is_refused() {
        // A line that is none of gettext's. Telling a catalog that is valid from one that is not is
        // `msgfmt`'s work, which `tests::the_catalogs_and_layouts_are_valid_gettext` does for what
        // ships; this reader's is to read a valid one as GNU gettext does.
        assert!(Catalog::parse("msgid \"a\"\nwhat is this\nmsgstr \"b\"\n").is_err());
        let bad = HEADER.replace("n%10==1", "n%10==");
        assert!(matches!(Catalog::parse(&bad), Err(CatalogError::Plural(_))));
    }

    #[test]
    fn english_and_the_pseudo_locale_have_no_catalog() {
        assert!(of(Locale::ENGLISH).is_none());
        assert!(of(Locale::PSEUDO).is_none());
    }
}
