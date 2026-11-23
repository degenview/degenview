//! The pseudo-locale (`en-XA`): English that is still readable but accented, about 40% longer and
//! wrapped in brackets.
//!
//! * A message that was never put through `tr!` shows without accents.
//! * A label that does not fit, or a fixed width that cut the text, shows the missing `]`.
//! * Accented capitals test line height and the font's coverage.
//! * The extra length stands in for German and Russian, which run longer than English.
//!
//! Only letters Latin-1 and Latin Extended-A have an accented form of are changed, so every
//! glyph is in the fonts the app always has. `{name}` values are left alone.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// How much longer the text gets, as a share of its letters.
const GROWTH: f32 = 0.4;

/// `text` in the pseudo-locale. A message is transformed once and kept for the life of the
/// process, which keeps the answer `'static` like the key (the set of messages is finite).
pub fn text(key: &'static str) -> &'static str {
    static CACHE: Mutex<Option<HashMap<&'static str, &'static str>>> = Mutex::new(None);
    // A panic elsewhere while the cache was held leaves it valid: it only ever gains entries.
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    let cache = cache.get_or_insert_with(HashMap::new);
    cache
        .entry(key)
        .or_insert_with(|| Box::leak(transform(key).into_boxed_str()))
}

fn transform(source: &str) -> String {
    let mut out = String::with_capacity(source.len() * 2 + 4);
    out.push('[');
    let mut letters = 0usize;
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                out.push_str("{{");
                chars.next();
            }
            '{' => {
                out.push('{');
                for inner in chars.by_ref() {
                    out.push(inner);
                    if inner == '}' {
                        break;
                    }
                }
            }
            _ => {
                if ch.is_alphabetic() {
                    letters += 1;
                }
                out.push(accent(ch));
            }
        }
    }
    if letters > 0 {
        out.push(' ');
        out.extend(std::iter::repeat_n(
            '~',
            (letters as f32 * GROWTH).ceil() as usize,
        ));
    }
    out.push(']');
    out
}

fn accent(ch: char) -> char {
    let lower = match ch.to_ascii_lowercase() {
        'a' => 'å',
        'c' => 'ç',
        'd' => 'ð',
        'e' => 'é',
        'g' => 'ĝ',
        'h' => 'ĥ',
        'i' => 'î',
        'j' => 'ĵ',
        'k' => 'ķ',
        'l' => 'ļ',
        'n' => 'ñ',
        'o' => 'ö',
        'p' => 'þ',
        'r' => 'ŕ',
        's' => 'š',
        't' => 'ţ',
        'u' => 'û',
        'w' => 'ŵ',
        'y' => 'ý',
        'z' => 'ž',
        _ => return ch,
    };
    if ch.is_ascii_uppercase() {
        lower.to_uppercase().next().unwrap_or(lower)
    } else {
        lower
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_accented_lengthened_and_bracketed() {
        assert_eq!(text("Save"), "[Šåvé ~~]");
        assert_eq!(text("OK"), "[ÖĶ ~]");
    }

    #[test]
    fn values_and_escaped_braces_are_left_as_they_are() {
        assert_eq!(text("{count} alerts"), "[{count} åļéŕţš ~~~]");
        assert_eq!(text("{{ok}} {name}"), "[{{öķ}} {name} ~]");
    }

    #[test]
    fn a_message_without_letters_is_only_bracketed() {
        assert_eq!(text("123"), "[123]");
        assert_eq!(text(""), "[]");
    }

    #[test]
    fn the_same_message_is_the_same_text() {
        assert!(std::ptr::eq(text("Open"), text("Open")));
    }

    #[test]
    fn every_ascii_letter_stays_in_latin_one_or_extended_a() {
        for ch in ('a'..='z').chain('A'..='Z') {
            let accented = accent(ch);
            assert!(
                accented == ch || ('\u{00C0}'..='\u{017F}').contains(&accented),
                "{ch} became {accented}"
            );
        }
    }
}
