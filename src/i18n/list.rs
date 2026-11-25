//! Lists of short phrases or names, joined the way the language joins them (CLDR's "and" list
//! pattern through ICU4X, `icu_list`): `a, b, and c` in English, `a, b y c` in Spanish, `a、b和c` in
//! Chinese.
//!
//! This is for a run of items shown as one line, such as which weekdays a statistic covers or which
//! charts a link leaves out. A sentence is written with `tr!`; a list inside a sentence is built here
//! and passed to it as a value. A separator typed in the code (`", "`) is English punctuation. Two
//! measurements side by side (a count of bars and a duration) are not a list: they are separated by
//! ` · `, which no language writes differently. CLDR's "unit" pattern is not used for lists of
//! names: it is for measurements, and Chinese writes those with no separator.

use super::{Locale, current};
use icu_list::ListFormatter;
use icu_list::options::{ListFormatterOptions, ListLength};
use icu_locale_core::Locale as IcuLocale;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn formatter() -> Rc<ListFormatter> {
    thread_local! {
        static CACHE: RefCell<HashMap<Locale, Rc<ListFormatter>>> = RefCell::new(HashMap::new());
    }
    let locale = current();
    CACHE.with(|cache| {
        Rc::clone(cache.borrow_mut().entry(locale).or_insert_with(|| {
            let options = ListFormatterOptions::default().with_length(ListLength::Wide);
            // The locale is one of the table's tags, which ICU4X has data for, and any it lacks falls
            // back to the root locale, so this does not fail.
            let formatter = ListFormatter::try_new_and((&locale.icu()).into(), options)
                .or_else(|_| ListFormatter::try_new_and((&IcuLocale::UNKNOWN).into(), options))
                .expect("ICU4X lists in the root locale");
            Rc::new(formatter)
        }))
    })
}

/// `items` as one line in the language in use. Empty for no items, the item itself for one.
pub fn join_list<S: AsRef<str>>(items: &[S]) -> String {
    let formatter = formatter();
    formatter
        .format(items.iter().map(|item| item.as_ref()))
        .to_string()
}
