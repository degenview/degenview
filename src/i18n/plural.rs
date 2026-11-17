//! Plural categories, the unit a catalog writes the forms of a counted message in.

use serde::Deserialize;

/// The six forms CLDR distinguishes. A language uses some of them: English `one` and `other`,
/// Russian `one`, `few`, `many` and `other`, Japanese only `other`. `Locale::plural_category`
/// says which one a count takes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Category {
    pub const COUNT: usize = 6;

    pub const fn index(self) -> usize {
        self as usize
    }
}
