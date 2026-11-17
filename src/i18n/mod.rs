//! Interface languages: what is shown in the language the person reads. A leaf module like
//! `format` and `platform`, which every layer may use and which imports no other (the layering
//! test keeps it so); the language is chosen and saved by `host::language`.
//!
//! User-facing text is written in English at the place it is used and put through a macro, which
//! returns the text in the language in use:
//!
//! * `tr!("Save")` is a `&'static str`, so it goes wherever a literal went.
//! * `tr!("{symbol} crossed {price}", symbol = s, price = p)` is a `String` with the values put in
//!   by name, because a translation moves them where its language wants them. Never build a
//!   sentence from pieces with `format!`.
//! * `tr_n!(count, "{count} alert", "{count} alerts")` picks the plural form the language uses
//!   for `count`, and passes it as `{count}`.
//! * `tr_c!("month abbreviation", "Jan")` is for English text that translates differently in
//!   different places (a verb and a noun spelt alike).
//!
//! The English text is the key of the catalogs, so the code reads as it did before, and a message
//! nobody translated shows in English. `docs/localization.md` has the rules and how a language is
//! added.

mod active;
pub mod calendar;
mod catalog;
#[cfg(test)]
mod extract;
mod locale;
mod plural;
mod preference;
mod pseudo;
mod template;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub use active::with_locale;
pub use active::{current, preference, revision, set};
pub use locale::{Locale, resolve};
pub use preference::Preference;
pub use template::fill;

/// The text of a message in the language in use, `context` being `""` when it has none.
pub fn translate(context: &'static str, key: &'static str) -> &'static str {
    let locale = current();
    if locale == Locale::English {
        return key;
    }
    if locale == Locale::Pseudo {
        return pseudo::text(key);
    }
    catalog::of(locale)
        .and_then(|catalog| catalog.get(context, key))
        .map_or(key, catalog::Message::text)
}

/// The text of a counted message for `count`: `one` or `other` in English, the form the language
/// uses in any other. A catalog finds the message by `other`.
pub fn translate_counted(count: u64, one: &'static str, other: &'static str) -> &'static str {
    let locale = current();
    let english = if count == 1 { one } else { other };
    if locale == Locale::English {
        return english;
    }
    if locale == Locale::Pseudo {
        return pseudo::text(english);
    }
    catalog::of(locale)
        .and_then(|catalog| catalog.get("", other))
        .map_or(english, |message| {
            message.form(locale.plural_category(count))
        })
}

/// A message in the language in use. See the module documentation.
macro_rules! tr {
    ($key:literal) => {
        $crate::i18n::translate("", $key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::translate("", $key),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

/// A message with a context. See the module documentation.
macro_rules! tr_c {
    ($context:literal, $key:literal) => {
        $crate::i18n::translate($context, $key)
    };
    ($context:literal, $key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::translate($context, $key),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

/// A counted message. See the module documentation.
macro_rules! tr_n {
    ($count:expr, $one:literal, $other:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        let count = ($count) as u64;
        $crate::i18n::fill(
            $crate::i18n::translate_counted(count, $one, $other),
            &[
                ("count", &count as &dyn ::std::fmt::Display)
                $(, (stringify!($name), &$value as &dyn ::std::fmt::Display))*
            ],
        )
    }};
}

pub(crate) use {tr, tr_c, tr_n};
