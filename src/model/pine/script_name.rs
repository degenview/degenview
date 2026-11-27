//! Script names double as file names inside the library folder, and scripts are synced between
//! computers. So a name given in any way (typed, from a file, from another computer, from an
//! import) is made into one every system accepts as a file name, rather than refused: Windows
//! refuses more than the others, and a name it refuses could not follow its script there.

use crate::i18n::tr;
use icu_normalizer::ComposingNormalizerBorrowed;
use std::collections::HashSet;

/// The longest name made, in bytes: a 255-byte file name also has to hold `.pine` and the number
/// a taken name gets.
pub const MAX_NAME_BYTES: usize = 200;
pub const FILE_EXTENSION: &str = "pine";

/// Nothing of the name is left to call a file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyName;

impl std::fmt::Display for EmptyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(tr!("Enter a name for the script."))
    }
}

impl std::error::Error for EmptyName {}

/// `raw` as the name a script is stored under: without a typed `.pine`, with what no file name may
/// hold replaced by `-`, and shortened to [`MAX_NAME_BYTES`]. A name that is already one is
/// unchanged.
pub fn portable(raw: &str) -> Result<String, EmptyName> {
    let mut name = fitted(&replaced(raw), MAX_NAME_BYTES).to_owned();
    if is_device(&name) {
        name = fitted(&format!("Script {name}"), MAX_NAME_BYTES).to_owned();
    }
    if name.is_empty() {
        return Err(EmptyName);
    }
    Ok(name)
}

/// `name` followed by `suffix` (a number, "copy", where a conflicting version came from), the name
/// giving way when the two are too long: the suffix is what tells them apart. `name` is kept as it
/// is where it fits, since it may be a script's own.
pub fn with_suffix(name: &str, suffix: &str) -> String {
    let suffix = replaced(suffix);
    let room = MAX_NAME_BYTES.saturating_sub(suffix.len());
    let name = match name.len() <= room {
        true => name,
        false => fitted(name, room),
    };
    format!("{name}{suffix}")
}

/// What tells names apart: two names with one key are one file on some system (macOS and Windows
/// ignore case, APFS also how an accent is spelt), so a synced library may hold only one of them.
pub fn key(name: &str) -> String {
    let lower = name.to_lowercase();
    ComposingNormalizerBorrowed::new_nfc()
        .normalize(&lower)
        .into_owned()
}

/// `base`, or the first of "base 2", "base 3", … whose [`key`] is not in `taken`.
pub fn numbered(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(&key(base)) {
        return base.to_owned();
    }
    let mut n = 2;
    loop {
        let candidate = with_suffix(base, &format!(" {n}"));
        if !taken.contains(&key(&candidate)) {
            return candidate;
        }
        n += 1;
    }
}

/// `text` with what no file name may hold replaced.
fn replaced(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect()
}

/// `name` within `limit` bytes and without what a file name may not start or end with: a
/// leading period hides a file, Windows drops trailing periods and spaces.
fn fitted(name: &str, limit: usize) -> &str {
    let suffix = format!(".{FILE_EXTENSION}");
    let mut name = name.trim_start_matches(|c: char| c == '.' || c.is_whitespace());
    let mut end = name.len().min(limit);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name = &name[..end];
    // Taking one ending off can uncover another.
    loop {
        let mut shorter = name.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
        if let Some(start) = shorter.len().checked_sub(suffix.len())
            && start > 0
            && shorter
                .get(start..)
                .is_some_and(|end| end.eq_ignore_ascii_case(&suffix))
        {
            shorter = &shorter[..start];
        }
        if shorter.len() == name.len() {
            return name;
        }
        name = shorter;
    }
}

/// The names Windows keeps for devices, with or without an extension.
fn is_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let stem = stem.to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|port| {
        stem.strip_prefix(port).is_some_and(|rest| {
            let mut digits = rest.chars();
            // The superscripts count too.
            matches!(digits.next(), Some('0'..='9' | '¹' | '²' | '³')) && digits.next().is_none()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_already_portable_is_unchanged() {
        for name in ["My Script", "RSI (14) - v2.1", "console", "COM10", "日本語"] {
            assert_eq!(portable(name).as_deref(), Ok(name));
        }
    }

    #[test]
    fn what_a_file_name_cannot_hold_is_replaced_not_refused() {
        for (given, name) in [
            ("  My Script.PINE ", "My Script"),
            ("a/b\\c:d", "a-b-c-d"),
            ("RSI | MACD?", "RSI - MACD-"),
            ("a<b>c*d\"e", "a-b-c-d-e"),
            ("a\tb\u{0}c", "a-b-c"),
            (".hidden", "hidden"),
            ("../evil", "-evil"),
            ("ends with dots...", "ends with dots"),
            ("twice.pine.pine", "twice"),
            ("CON", "Script CON"),
            ("com1.v2", "Script com1.v2"),
            ("LPT²", "Script LPT²"),
            ("conin$", "Script conin$"),
            (".pine", "pine"),
        ] {
            assert_eq!(portable(given).as_deref(), Ok(name), "{given:?}");
        }
    }

    #[test]
    fn a_long_name_is_shortened_between_characters() {
        assert_eq!(portable(&"x".repeat(251)).unwrap().len(), MAX_NAME_BYTES);
        let rockets = portable(&"\u{1F680}".repeat(120)).unwrap();
        assert_eq!(rockets, "\u{1F680}".repeat(MAX_NAME_BYTES / 4));
    }

    #[test]
    fn making_a_name_portable_twice_changes_nothing_more() {
        let long = format!("{}.pine.{}", "a".repeat(193), "b".repeat(30));
        let devices = format!("nul.{}", "c".repeat(300));
        for given in [
            long.as_str(),
            devices.as_str(),
            " . a . pine . ",
            "x .pine .",
            "ÆØ.pine",
            "é.PiNe.PINE",
        ] {
            let once = portable(given).unwrap();
            assert!(once.len() <= MAX_NAME_BYTES, "{once}");
            assert_eq!(portable(&once).as_deref(), Ok(once.as_str()), "{given:?}");
        }
    }

    #[test]
    fn a_suffix_is_kept_whole_and_the_name_gives_way() {
        assert_eq!(with_suffix("Mine", " 2"), "Mine 2");
        assert_eq!(with_suffix("A | B", " copy"), "A | B copy", "its own name");
        assert_eq!(with_suffix("S", " (from a/b)"), "S (from a-b)");
        for base in ["x".repeat(200), "é".repeat(100)] {
            let numbered = with_suffix(&base, " 12");
            assert!(numbered.ends_with(" 12") && numbered.len() <= MAX_NAME_BYTES);
            assert_eq!(portable(&numbered).as_deref(), Ok(numbered.as_str()));
        }
    }

    #[test]
    fn names_that_are_one_file_somewhere_are_one_name() {
        assert_eq!(key("Caf\u{c9}"), key("cafe\u{301}"));
        assert_ne!(key("Cafe"), key("Caf\u{e9}"));
        let taken = HashSet::from([key("Cafe\u{301}"), key("CAF\u{c9} 2")]);
        assert_eq!(numbered("Caf\u{e9}", &taken), "Caf\u{e9} 3");
        assert_eq!(numbered("Tea", &taken), "Tea");
    }

    #[test]
    fn only_a_name_with_nothing_left_is_refused() {
        for given in ["", "   ", "...", " . "] {
            assert_eq!(portable(given), Err(EmptyName), "{given:?}");
        }
        assert_eq!(EmptyName.to_string(), "Enter a name for the script.");
    }
}
