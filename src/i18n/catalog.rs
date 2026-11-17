//! A language's translations: one JSON file per locale in `assets/locales/`, compiled in.
//!
//! ```json
//! {
//!   "messages": {
//!     "Save": "Guardar",
//!     "{count} alerts": { "one": "{count} alerta", "other": "{count} alertas" }
//!   },
//!   "contexts": {
//!     "month abbreviation": { "Jan": "ene" }
//!   }
//! }
//! ```
//!
//! A message is found by its English text, which is the key; a counted message by its `other`
//! (plural) text. Text that means different things in different places has a context, the first
//! argument of `tr_c!`. A message a catalog lacks is shown in English.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use super::locale::Locale;
use super::plural::Category;

/// A catalog file as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    #[serde(default)]
    pub messages: BTreeMap<String, Entry>,
    #[serde(default)]
    pub contexts: BTreeMap<String, BTreeMap<String, Entry>>,
}

/// One translation: a text, or the forms of a counted message by plural category.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Entry {
    Text(String),
    Plural(BTreeMap<Category, String>),
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("the catalog is not valid JSON of the expected shape: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the counted message {0:?} has no `other` form")]
    MissingOther(String),
}

impl File {
    pub fn parse(json: &str) -> Result<File, CatalogError> {
        let file: File = serde_json::from_str(json)?;
        let counted = file.messages.iter().chain(file.contexts.values().flatten());
        for (key, entry) in counted {
            if let Entry::Plural(forms) = entry
                && !forms.contains_key(&Category::Other)
            {
                return Err(CatalogError::MissingOther(key.clone()));
            }
        }
        Ok(file)
    }
}

/// The text of a message in a language, with its plural forms when it has them.
#[derive(Clone, Copy, Debug)]
pub enum Message {
    Text(&'static str),
    Plural([Option<&'static str>; Category::COUNT]),
}

impl Message {
    fn from_entry(entry: &Entry) -> Message {
        match entry {
            Entry::Text(text) => Message::Text(leak(text)),
            Entry::Plural(forms) => {
                let mut by_category = [None; Category::COUNT];
                for (category, text) in forms {
                    by_category[category.index()] = Some(leak(text));
                }
                Message::Plural(by_category)
            }
        }
    }

    /// The text a message without a count shows.
    pub fn text(&self) -> &'static str {
        match self {
            Message::Text(text) => text,
            Message::Plural(forms) => forms[Category::Other.index()].unwrap_or_default(),
        }
    }

    /// The form for `category`, the `other` form when the language gave none for it.
    pub fn form(&self, category: Category) -> &'static str {
        match self {
            Message::Text(text) => text,
            Message::Plural(forms) => forms[category.index()]
                .or(forms[Category::Other.index()])
                .unwrap_or_default(),
        }
    }
}

/// Catalogs live as long as the process (one per language, read once), so their text is leaked
/// to be `'static`, like the English text it replaces.
fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// A catalog ready for lookups.
#[derive(Debug)]
pub struct Catalog {
    messages: HashMap<&'static str, Message>,
    contexts: HashMap<&'static str, HashMap<&'static str, Message>>,
}

impl Catalog {
    pub fn from_file(file: &File) -> Catalog {
        let build = |entries: &BTreeMap<String, Entry>| -> HashMap<&'static str, Message> {
            entries
                .iter()
                .map(|(key, entry)| (leak(key), Message::from_entry(entry)))
                .collect()
        };
        Catalog {
            messages: build(&file.messages),
            contexts: file
                .contexts
                .iter()
                .map(|(context, entries)| (leak(context), build(entries)))
                .collect(),
        }
    }

    /// The translation of `key`, in `context` when that is not empty.
    pub fn get(&self, context: &str, key: &str) -> Option<&Message> {
        if context.is_empty() {
            self.messages.get(key)
        } else {
            self.contexts.get(context)?.get(key)
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.messages.len() + self.contexts.values().map(HashMap::len).sum::<usize>()
    }
}

/// The catalog of `locale`, read once; `None` for English and the pseudo-locale, which are
/// made from the source text, and for a file that does not parse (the tests fail on one, so a
/// shipped build never has it: this falls back to English instead of panicking).
pub fn of(locale: Locale) -> Option<&'static Catalog> {
    static CATALOGS: [std::sync::OnceLock<Option<Catalog>>; Locale::ALL.len()] =
        [const { std::sync::OnceLock::new() }; Locale::ALL.len()];
    let index = Locale::ALL.iter().position(|known| *known == locale)?;
    let json = source(locale)?;
    CATALOGS[index]
        .get_or_init(|| File::parse(json).ok().map(|file| Catalog::from_file(&file)))
        .as_ref()
}

/// The embedded JSON of a locale's catalog. A language is added here with its file.
pub const fn source(locale: Locale) -> Option<&'static str> {
    match locale {
        Locale::English | Locale::Pseudo => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "messages": {
            "Save": "Guardar",
            "{count} alerts": { "one": "{count} alerta", "other": "{count} alertas" }
        },
        "contexts": { "month abbreviation": { "Jan": "ene" } }
    }"#;

    fn sample() -> Catalog {
        Catalog::from_file(&File::parse(SAMPLE).expect("sample parses"))
    }

    #[test]
    fn a_message_is_found_by_its_english_text_and_a_context_by_both() {
        let catalog = sample();
        assert_eq!(catalog.get("", "Save").map(Message::text), Some("Guardar"));
        assert_eq!(
            catalog.get("month abbreviation", "Jan").map(Message::text),
            Some("ene")
        );
        assert!(catalog.get("", "Jan").is_none());
        assert!(catalog.get("month abbreviation", "Save").is_none());
        assert!(catalog.get("", "Open").is_none());
        assert_eq!(catalog.len(), 3);
    }

    #[test]
    fn a_counted_message_has_a_form_per_category_and_falls_back_to_other() {
        let catalog = sample();
        let counted = catalog.get("", "{count} alerts").expect("counted");
        assert_eq!(counted.form(Category::One), "{count} alerta");
        assert_eq!(counted.form(Category::Other), "{count} alertas");
        assert_eq!(counted.form(Category::Few), "{count} alertas");
        assert_eq!(counted.text(), "{count} alertas");
    }

    #[test]
    fn a_plain_text_serves_every_category() {
        let catalog = sample();
        let save = catalog.get("", "Save").expect("save");
        assert_eq!(save.form(Category::One), "Guardar");
        assert_eq!(save.form(Category::Many), "Guardar");
    }

    #[test]
    fn a_counted_message_without_other_is_refused() {
        let json = r#"{"messages":{"{n} x":{"one":"{n} y"}}}"#;
        assert!(matches!(
            File::parse(json),
            Err(CatalogError::MissingOther(key)) if key == "{n} x"
        ));
    }

    #[test]
    fn unknown_sections_and_categories_are_refused() {
        assert!(File::parse(r#"{"mesages":{}}"#).is_err());
        assert!(File::parse(r#"{"messages":{"a":{"some":"b","other":"c"}}}"#).is_err());
        assert!(File::parse("not json").is_err());
        assert!(File::parse("{}").is_ok());
    }

    #[test]
    fn english_and_the_pseudo_locale_have_no_catalog() {
        assert!(of(Locale::English).is_none());
        assert!(of(Locale::Pseudo).is_none());
    }
}
