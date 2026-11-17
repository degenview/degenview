//! The language the interface is in now: one value for the whole process, read by every `tr!`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use super::locale::Locale;
use super::preference::Preference;
use crate::lock::lock;

/// The language in use and the choice that led to it.
struct Active {
    locale: AtomicU8,
    revision: AtomicU64,
    preference: Mutex<Preference>,
}

impl Active {
    const fn new() -> Self {
        Self {
            locale: AtomicU8::new(0),
            revision: AtomicU64::new(0),
            preference: Mutex::new(Preference::System),
        }
    }

    fn locale(&self) -> Locale {
        Locale::ALL[usize::from(self.locale.load(Ordering::Relaxed))]
    }

    fn set(&self, preference: Preference, locale: Locale) {
        let index = Locale::ALL
            .iter()
            .position(|known| *known == locale)
            .unwrap_or(0);
        let mut held = lock(&self.preference);
        let changed =
            *held != preference || usize::from(self.locale.load(Ordering::Relaxed)) != index;
        *held = preference;
        self.locale.store(index as u8, Ordering::Relaxed);
        if changed {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
    }
}

static ACTIVE: Active = Active::new();

#[cfg(test)]
thread_local! {
    /// A language one test thread asks for, so tests that draw in another language do not change
    /// what the tests running beside them on other threads see.
    static OVERRIDE: std::cell::Cell<Option<Locale>> = const { std::cell::Cell::new(None) };
}

/// The language messages are looked up in.
pub fn current() -> Locale {
    #[cfg(test)]
    if let Some(locale) = OVERRIDE.get() {
        return locale;
    }
    ACTIVE.locale()
}

/// What the person chose, which is not always the language in use: "System" follows the machine.
pub fn preference() -> Preference {
    *lock(&ACTIVE.preference)
}

/// Changes whenever the language does, so a window with nothing animating knows to draw again.
pub fn revision() -> u64 {
    ACTIVE.revision.load(Ordering::Acquire)
}

/// Makes `locale` the language in use, remembering the `preference` that led to it.
pub fn set(preference: Preference, locale: Locale) {
    ACTIVE.set(preference, locale);
}

/// Runs `body` with `locale` as this thread's language and puts the previous one back after, even
/// when `body` panics.
#[cfg(test)]
pub fn with_locale<R>(locale: Locale, body: impl FnOnce() -> R) -> R {
    struct Restore(Option<Locale>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.set(self.0);
        }
    }
    let _restore = Restore(OVERRIDE.replace(Some(locale)));
    body()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_can_ask_for_a_language_on_its_own_thread() {
        assert_eq!(current(), Locale::English);
        with_locale(Locale::Pseudo, || {
            assert_eq!(current(), Locale::Pseudo);
            with_locale(Locale::English, || assert_eq!(current(), Locale::English));
            assert_eq!(current(), Locale::Pseudo);
        });
        assert_eq!(current(), Locale::English);
    }

    #[test]
    fn the_language_is_put_back_after_a_panic() {
        let result = std::panic::catch_unwind(|| with_locale(Locale::Pseudo, || panic!("boom")));
        assert!(result.is_err());
        assert_eq!(current(), Locale::English);
    }

    #[test]
    fn a_change_of_language_or_of_choice_is_a_new_revision_and_a_repeat_is_not() {
        let active = Active::new();
        assert_eq!(active.locale(), Locale::English);
        let fixed = Preference::Fixed(Locale::Pseudo);

        active.set(fixed, Locale::Pseudo);
        assert_eq!(active.locale(), Locale::Pseudo);
        assert_eq!(active.revision.load(Ordering::Acquire), 1);

        active.set(fixed, Locale::Pseudo);
        assert_eq!(active.revision.load(Ordering::Acquire), 1);

        // The same language reached by another choice is a change too: Settings shows the choice.
        active.set(Preference::System, Locale::Pseudo);
        assert_eq!(active.revision.load(Ordering::Acquire), 2);
        assert_eq!(*lock(&active.preference), Preference::System);
    }
}
