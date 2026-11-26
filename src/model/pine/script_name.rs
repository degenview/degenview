//! Script names double as file names inside the library folder, and scripts are synced between
//! computers. So a name given in any way (typed, from a file, from another computer, from an
//! import) is made into one every system accepts as a file name, rather than refused: Windows
//! refuses more than the others, and a name it refuses could not follow its script there.

use crate::i18n::tr;

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
    let replaced: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let mut name = fitted(&replaced).to_owned();
    if is_device(&name) {
        name = fitted(&format!("Script {name}")).to_owned();
    }
    if name.is_empty() {
        return Err(EmptyName);
    }
    Ok(name)
}

/// `name` within the length limit and without what a file name may not start or end with: a
/// leading period hides a file, Windows drops trailing periods and spaces.
fn fitted(name: &str) -> &str {
    let suffix = format!(".{FILE_EXTENSION}");
    let mut name = name.trim_start_matches(|c: char| c == '.' || c.is_whitespace());
    let mut end = name.len().min(MAX_NAME_BYTES);
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
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|port| {
            stem.strip_prefix(port)
                .is_some_and(|rest| rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit())
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
    fn only_a_name_with_nothing_left_is_refused() {
        for given in ["", "   ", "...", " . "] {
            assert_eq!(portable(given), Err(EmptyName), "{given:?}");
        }
        assert_eq!(EmptyName.to_string(), "Enter a name for the script.");
    }
}
