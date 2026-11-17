//! The languages the system is set to, most wanted first, as BCP 47 tags (`en-US`, `es-419`).
//! `i18n::Locale::negotiate` picks the one the app has.

/// The system's languages. Empty when the system does not say.
pub fn preferred() -> Vec<String> {
    platform::preferred()
}

/// The languages in the environment, as the C library reads them: `LANGUAGE` (a colon-separated
/// list) first, then the first of `LC_ALL`, `LC_MESSAGES` and `LANG` that is set. `C` and `POSIX`
/// mean no language.
#[cfg(any(test, not(any(target_os = "macos", windows))))]
fn from_environment(lookup: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let set = |name: &str| lookup(name).filter(|value| !value.is_empty());
    let mut tags = Vec::new();
    let sources = set("LANGUAGE")
        .into_iter()
        .flat_map(|list| list.split(':').map(str::to_owned).collect::<Vec<_>>())
        .chain(["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(set));
    for source in sources {
        // `de_DE.UTF-8@euro` is the language and region alone.
        let tag = source
            .split(['.', '@'])
            .next()
            .unwrap_or_default()
            .replace('_', "-");
        if !tag.is_empty() && tag != "C" && tag != "POSIX" && !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_foundation::NSLocale;

    pub fn preferred() -> Vec<String> {
        NSLocale::preferredLanguages()
            .iter()
            .map(|tag| tag.to_string())
            .collect()
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

    pub fn preferred() -> Vec<String> {
        let mut count = 0u32;
        let mut length = 0u32;
        // SAFETY: the first call only asks how long the answer is (a null buffer is allowed
        // then); the second fills a buffer of exactly that many UTF-16 units. Both counters are
        // live locals.
        unsafe {
            if GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &raw mut count,
                std::ptr::null_mut(),
                &raw mut length,
            ) == 0
                || length == 0
            {
                return Vec::new();
            }
            let mut buffer = vec![0u16; length as usize];
            if GetUserPreferredUILanguages(
                MUI_LANGUAGE_NAME,
                &raw mut count,
                buffer.as_mut_ptr(),
                &raw mut length,
            ) == 0
            {
                return Vec::new();
            }
            // The answer is each name followed by a NUL, and one more NUL at the end.
            buffer
                .split(|unit| *unit == 0)
                .filter(|name| !name.is_empty())
                .map(String::from_utf16_lossy)
                .collect()
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub fn preferred() -> Vec<String> {
        super::from_environment(|name| std::env::var(name).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> Vec<String> {
        from_environment(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn language_is_a_list_and_comes_before_the_locale() {
        assert_eq!(
            environment(&[("LANGUAGE", "de:fr"), ("LANG", "en_US.UTF-8")]),
            ["de", "fr", "en-US"]
        );
    }

    #[test]
    fn the_first_locale_variable_that_is_set_wins() {
        assert_eq!(
            environment(&[("LC_ALL", "es_MX.UTF-8"), ("LANG", "en_US.UTF-8")]),
            ["es-MX"]
        );
        assert_eq!(
            environment(&[("LC_ALL", ""), ("LC_MESSAGES", "pt_BR"), ("LANG", "en")]),
            ["pt-BR"]
        );
    }

    #[test]
    fn encoding_and_modifier_are_dropped_and_repeats_are_kept_once() {
        assert_eq!(
            environment(&[("LANGUAGE", "de_DE@euro:de-DE"), ("LANG", "de_DE.UTF-8")]),
            ["de-DE"]
        );
    }

    #[test]
    fn c_and_posix_and_nothing_mean_no_language() {
        assert!(environment(&[("LANG", "C")]).is_empty());
        assert!(environment(&[("LANG", "C.UTF-8")]).is_empty());
        assert!(environment(&[("LC_ALL", "POSIX")]).is_empty());
        assert!(environment(&[]).is_empty());
    }
}
