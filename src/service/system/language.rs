//! The languages the system is set to, most wanted first, as BCP 47 tags (`en-US`, `es-419`).
//! `i18n::Locale::negotiate` picks the one the app has.
//!
//! `sys-locale` asks the system the way each one expects (`NSLocale` on macOS,
//! `GetUserPreferredUILanguages` on Windows, `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and `LANG`
//! elsewhere), so the app has no platform code of its own for it.

/// The system's languages. Empty when the system does not say.
pub fn preferred() -> Vec<String> {
    sys_locale::get_locales().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_system_says_is_text_negotiation_can_take() {
        // Whatever this machine is set to (and nothing in a bare CI container).
        let tags = preferred();
        let _ = crate::i18n::Locale::negotiate(&tags);
    }
}
