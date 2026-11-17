//! The language the interface is in now: one value for the whole process, read by every `tr!`.

use std::sync::atomic::{AtomicU64, Ordering};

use super::locale::Locale;
use super::preference::Preference;

/// The language in use, the choice that led to it and how many times either changed, packed in one
/// word so a reader never sees a language from one change and a revision from another. Reading is
/// a single load, which matters: every `tr!` of every frame reads it.
///
/// Bits 0-7 are the language (its position in the table), 8-15 the choice (0 for "System", else the
/// language's position plus one), 16-63 the revision.
struct Active(AtomicU64);

const LOCALE_MASK: u64 = 0xff;
const CHOICE_SHIFT: u32 = 8;
const REVISION_SHIFT: u32 = 16;

impl Active {
    const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    fn locale(&self) -> Locale {
        Locale::from_index((self.0.load(Ordering::Acquire) & LOCALE_MASK) as u8)
    }

    fn preference(&self) -> Preference {
        let choice = (self.0.load(Ordering::Acquire) >> CHOICE_SHIFT) & LOCALE_MASK;
        match choice {
            0 => Preference::System,
            fixed => Preference::Fixed(Locale::from_index((fixed - 1) as u8)),
        }
    }

    fn revision(&self) -> u64 {
        self.0.load(Ordering::Acquire) >> REVISION_SHIFT
    }

    fn set(&self, preference: Preference, locale: Locale) {
        let choice = match preference {
            Preference::System => 0,
            Preference::Fixed(fixed) => u64::from(fixed.index()) + 1,
        };
        let state = u64::from(locale.index()) | choice << CHOICE_SHIFT;
        // A repeat of what is already set is not a change, and repaints nothing.
        let _ = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                let unchanged = old & ((1 << REVISION_SHIFT) - 1);
                (unchanged != state)
                    .then(|| state | ((old >> REVISION_SHIFT) + 1) << REVISION_SHIFT)
            });
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
    ACTIVE.preference()
}

/// Changes whenever the language does, so a window with nothing animating knows to draw again, and
/// so text kept from an earlier frame (which `tr!` does not do for you) is made again.
pub fn revision() -> u64 {
    // A test that asks for a language on its own thread (`with_locale`) changes what this thread
    // reads, so it changes the revision this thread sees too: text kept from an earlier frame is made
    // again there, as it is when the language is changed in Settings.
    #[cfg(test)]
    if let Some(locale) = OVERRIDE.get() {
        return ACTIVE.revision() ^ ((u64::from(locale.index()) + 1) << 56);
    }
    ACTIVE.revision()
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
        assert_eq!(current(), Locale::ENGLISH);
        with_locale(Locale::PSEUDO, || {
            assert_eq!(current(), Locale::PSEUDO);
            with_locale(Locale::ENGLISH, || assert_eq!(current(), Locale::ENGLISH));
            assert_eq!(current(), Locale::PSEUDO);
        });
        assert_eq!(current(), Locale::ENGLISH);
    }

    #[test]
    fn the_language_is_put_back_after_a_panic() {
        let result = std::panic::catch_unwind(|| with_locale(Locale::PSEUDO, || panic!("boom")));
        assert!(result.is_err());
        assert_eq!(current(), Locale::ENGLISH);
    }

    #[test]
    fn a_change_of_language_or_of_choice_is_a_new_revision_and_a_repeat_is_not() {
        let active = Active::new();
        assert_eq!(active.locale(), Locale::ENGLISH);
        assert_eq!(active.preference(), Preference::System);
        let fixed = Preference::Fixed(Locale::PSEUDO);

        active.set(fixed, Locale::PSEUDO);
        assert_eq!(active.locale(), Locale::PSEUDO);
        assert_eq!(active.preference(), fixed);
        assert_eq!(active.revision(), 1);

        active.set(fixed, Locale::PSEUDO);
        assert_eq!(active.revision(), 1);

        // The same language reached by another choice is a change too: Settings shows the choice.
        active.set(Preference::System, Locale::PSEUDO);
        assert_eq!(active.revision(), 2);
        assert_eq!(active.preference(), Preference::System);
        assert_eq!(active.locale(), Locale::PSEUDO);
    }

    #[test]
    fn a_fixed_english_is_not_the_same_choice_as_the_system() {
        let active = Active::new();
        active.set(Preference::Fixed(Locale::ENGLISH), Locale::ENGLISH);
        assert_eq!(active.preference(), Preference::Fixed(Locale::ENGLISH));
        assert_eq!(active.revision(), 1);
    }
}
