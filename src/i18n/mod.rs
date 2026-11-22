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
//! The English text is the key of the catalogs (gettext's `msgid`), so the code reads as it did
//! before, and a message nobody translated shows in English. The catalogs are gettext `.po` files;
//! dates and times are written by `datetime` with CLDR's patterns. A message is checked when it
//! compiles (`check`). `docs/localization.md` has the rules and how a language is added.
//!
//! The text a macro returns is for the frame being drawn: a language chosen in Settings changes
//! it, so do not keep it past a frame. `revision()` changes with the language, for state that
//! must be made again.

mod active;
mod catalog;
pub mod check;
pub mod datetime;
#[cfg(test)]
mod extract;
mod locale;
mod plural_forms;
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
    if locale.is_pseudo() {
        return pseudo::text(key);
    }
    catalog::of(locale)
        .and_then(|catalog| catalog.text(context, key))
        .unwrap_or(key)
}

/// The text of a counted message for `count`: `one` or `other` in English, the form the language
/// uses in any other. A catalog finds the message by `one`, as gettext does.
pub fn translate_counted(count: u64, one: &'static str, other: &'static str) -> &'static str {
    let locale = current();
    let english = if count == 1 { one } else { other };
    if locale.is_pseudo() {
        return pseudo::text(english);
    }
    catalog::of(locale)
        .and_then(|catalog| catalog.counted(one, count))
        .unwrap_or(english)
}

/// A message in the language in use. See the module documentation.
macro_rules! tr {
    ($key:literal) => {{
        const { $crate::i18n::check::plain($key) };
        $crate::i18n::translate("", $key)
    }};
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        const {
            $crate::i18n::check::message(
                $key,
                &[$(stringify!($name)),+],
                &[$(stringify!($name)),+],
            )
        };
        $crate::i18n::fill(
            $crate::i18n::translate("", $key),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    }};
}

/// A message with a context. See the module documentation.
// Kept for the messages that need it as the interface moves over; none does yet.
#[allow(unused_macros)]
macro_rules! tr_c {
    ($context:literal, $key:literal) => {{
        const { $crate::i18n::check::plain($key) };
        $crate::i18n::translate($context, $key)
    }};
    ($context:literal, $key:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        const {
            $crate::i18n::check::message(
                $key,
                &[$(stringify!($name)),+],
                &[$(stringify!($name)),+],
            )
        };
        $crate::i18n::fill(
            $crate::i18n::translate($context, $key),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    }};
}

/// A counted message. See the module documentation.
macro_rules! tr_n {
    ($count:expr, $one:literal, $other:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        const {
            // Either form may leave the count out ("One alert"); `other` must use the rest.
            $crate::i18n::check::message(
                $one,
                &["count" $(, stringify!($name))*],
                &[],
            );
            $crate::i18n::check::message(
                $other,
                &["count" $(, stringify!($name))*],
                &[$(stringify!($name)),*],
            );
        };
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

#[allow(unused_imports)]
pub(crate) use {tr, tr_c, tr_n};
