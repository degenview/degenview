//! The Elliott wave drawings: how many pivots each takes, how its waves are named, and how a
//! wave degree writes those names.
//!
//! These are manual drawings. Nothing here finds waves in the price, checks the textbook rules
//! (wave 2 never retraces past the start of wave 1, wave 3 is never the shortest, ...) or moves a
//! pivot the user placed: TradingView's help states those rules for its automatic indicator, and
//! says nothing of the drawing tools enforcing them. Pure: no egui types.

use super::drawing_tool::Tool;
use serde::{Deserialize, Serialize};

/// Which Elliott wave a [`Geometry::Wave`](super::drawings::Geometry::Wave) is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveKind {
    /// 1 2 3 4 5.
    Impulse,
    /// A B C.
    Correction,
    /// A B C D E.
    Triangle,
    /// W X Y.
    DoubleCombo,
    /// W X Y X Z.
    TripleCombo,
}

impl WaveKind {
    #[cfg(test)]
    pub const ALL: [Self; 5] = [
        Self::Impulse,
        Self::Correction,
        Self::Triangle,
        Self::DoubleCombo,
        Self::TripleCombo,
    ];

    /// The waves' names before a degree dresses them. The second X of a triple combination is
    /// an X again: the repeat is the notation, so it is never renamed to keep names unique.
    pub const fn names(self) -> &'static [&'static str] {
        match self {
            Self::Impulse => &["1", "2", "3", "4", "5"],
            Self::Correction => &["A", "B", "C"],
            Self::Triangle => &["A", "B", "C", "D", "E"],
            Self::DoubleCombo => &["W", "X", "Y"],
            Self::TripleCombo => &["W", "X", "Y", "X", "Z"],
        }
    }

    /// How many pivots the drawing is placed with: where the first wave starts, then the end of
    /// each wave. **Inferred** from the names TradingView gives each tool; its help does not
    /// state a click count.
    pub const fn points(self) -> usize {
        self.names().len() + 1
    }

    pub const fn tool(self) -> Tool {
        match self {
            Self::Impulse => Tool::ElliottImpulse,
            Self::Correction => Tool::ElliottCorrection,
            Self::Triangle => Tool::ElliottTriangle,
            Self::DoubleCombo => Tool::ElliottDoubleCombo,
            Self::TripleCombo => Tool::ElliottTripleCombo,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::ElliottImpulse => Some(Self::Impulse),
            Tool::ElliottCorrection => Some(Self::Correction),
            Tool::ElliottTriangle => Some(Self::Triangle),
            Tool::ElliottDoubleCombo => Some(Self::DoubleCombo),
            Tool::ElliottTripleCombo => Some(Self::TripleCombo),
            _ => None,
        }
    }

    /// Whether the names are numbers (an impulse's), which a degree writes in its own numerals
    /// and not as letters.
    const fn numbered(self) -> bool {
        matches!(self, Self::Impulse)
    }
}

/// How a degree writes a number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Numerals {
    UpperRoman,
    Arabic,
    LowerRoman,
}

/// How a degree brackets a name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Brackets {
    None,
    Single,
    Double,
}

/// A wave degree: the scale of the move, which sets how its waves are written.
///
/// The nine standard degrees and their notation are the table in TradingView's own wiki for its
/// Elliott tools (a `Minor` impulse is `1 2 3 4 5`, an `Intermediate` one `(1) (2) (3) (4) (5)`,
/// a `Minute` correction `((a)) ((b)) ((c))`). TradingView's list has six more, below
/// Subminuette (Micro, Submicro, Miniscule) and above Grand Supercycle (Submillennium,
/// Millennium, Supermillennium), whose notation could not be found: they are not offered.
/// Letters of the combination waves (W X Y Z) follow the case of the degree's correction
/// letters, which is **inferred**. The wiki's table names two of its degrees differently (Circle
/// for Cycle, Major for Primary); they are matched by place in the list. A new drawing starts at
/// Intermediate: the Charting Library's default is degree 7 of its 15, which is Intermediate
/// counting up from the smallest or down from the largest.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Degree {
    GrandSupercycle,
    Supercycle,
    Cycle,
    Primary,
    #[default]
    Intermediate,
    Minor,
    Minute,
    Minuette,
    Subminuette,
}

impl Degree {
    pub const ALL: [Self; 9] = [
        Self::GrandSupercycle,
        Self::Supercycle,
        Self::Cycle,
        Self::Primary,
        Self::Intermediate,
        Self::Minor,
        Self::Minute,
        Self::Minuette,
        Self::Subminuette,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::GrandSupercycle => "Grand Supercycle",
            Self::Supercycle => "Supercycle",
            Self::Cycle => "Cycle",
            Self::Primary => "Primary",
            Self::Intermediate => "Intermediate",
            Self::Minor => "Minor",
            Self::Minute => "Minute",
            Self::Minuette => "Minuette",
            Self::Subminuette => "Subminuette",
        }
    }

    const fn brackets(self) -> Brackets {
        match self {
            Self::GrandSupercycle | Self::Primary | Self::Minute => Brackets::Double,
            Self::Supercycle | Self::Intermediate | Self::Minuette => Brackets::Single,
            Self::Cycle | Self::Minor | Self::Subminuette => Brackets::None,
        }
    }

    const fn numerals(self) -> Numerals {
        match self {
            Self::GrandSupercycle | Self::Supercycle | Self::Cycle => Numerals::UpperRoman,
            Self::Primary | Self::Intermediate | Self::Minor => Numerals::Arabic,
            Self::Minute | Self::Minuette | Self::Subminuette => Numerals::LowerRoman,
        }
    }

    /// Whether the letters of a correction are lower case (`a b c`).
    const fn lower_letters(self) -> bool {
        !matches!(self, Self::Primary | Self::Intermediate | Self::Minor)
    }

    /// `name` (one of [`WaveKind::names`]) written in this degree: its numerals or letters, in
    /// its brackets.
    fn write(self, kind: WaveKind, name: &str) -> String {
        let core = if kind.numbered() {
            let number: usize = name.parse().unwrap_or(0);
            match self.numerals() {
                Numerals::Arabic => number.to_string(),
                Numerals::UpperRoman => roman(number).to_owned(),
                Numerals::LowerRoman => roman(number).to_lowercase(),
            }
        } else if self.lower_letters() {
            name.to_lowercase()
        } else {
            name.to_owned()
        };
        match self.brackets() {
            Brackets::None => core,
            Brackets::Single => format!("({core})"),
            Brackets::Double => format!("(({core}))"),
        }
    }

    /// The label of every pivot of a `kind` wave in this degree: the starting pivot has none,
    /// then one per wave.
    pub fn pivot_labels(self, kind: WaveKind) -> Vec<String> {
        std::iter::once(String::new())
            .chain(kind.names().iter().map(|name| self.write(kind, name)))
            .collect()
    }

    /// An impulse's first label in this degree, to show what the degree looks like.
    pub fn sample(self) -> String {
        self.write(WaveKind::Impulse, "1")
    }
}

/// The roman numeral of 1 to 5; the waves never go further. Anything else is empty.
fn roman(number: usize) -> &'static str {
    match number {
        1 => "I",
        2 => "II",
        3 => "III",
        4 => "IV",
        5 => "V",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(degree: Degree, kind: WaveKind) -> Vec<String> {
        degree.pivot_labels(kind)[1..].to_vec()
    }

    #[test]
    fn every_kind_is_one_tool_with_a_pivot_for_the_start_and_each_wave() {
        let counts = [6, 4, 6, 4, 6];
        for (kind, count) in WaveKind::ALL.into_iter().zip(counts) {
            assert_eq!(WaveKind::of_tool(kind.tool()), Some(kind));
            assert_eq!(kind.points(), count, "{kind:?}");
            assert_eq!(Degree::default().pivot_labels(kind).len(), count);
        }
    }

    #[test]
    fn the_start_of_the_first_wave_has_no_label() {
        for kind in WaveKind::ALL {
            assert_eq!(Degree::default().pivot_labels(kind)[0], "");
        }
    }

    #[test]
    fn minor_writes_plain_numbers_and_capital_letters() {
        assert_eq!(
            labels(Degree::Minor, WaveKind::Impulse),
            ["1", "2", "3", "4", "5"]
        );
        assert_eq!(labels(Degree::Minor, WaveKind::Correction), ["A", "B", "C"]);
        assert_eq!(
            labels(Degree::Minor, WaveKind::Triangle),
            ["A", "B", "C", "D", "E"]
        );
        assert_eq!(
            labels(Degree::Minor, WaveKind::DoubleCombo),
            ["W", "X", "Y"]
        );
    }

    #[test]
    fn the_triple_combination_keeps_its_second_x() {
        let triple = labels(Degree::Minor, WaveKind::TripleCombo);
        assert_eq!(triple, ["W", "X", "Y", "X", "Z"]);
        assert_eq!(triple.iter().filter(|label| *label == "X").count(), 2);
        // In brackets too.
        assert_eq!(
            labels(Degree::Intermediate, WaveKind::TripleCombo),
            ["(W)", "(X)", "(Y)", "(X)", "(Z)"]
        );
    }

    #[test]
    fn each_degree_is_written_as_in_tradingviews_table() {
        let impulse = |degree| labels(degree, WaveKind::Impulse).join(" ");
        let correction = |degree| labels(degree, WaveKind::Correction).join(" ");
        let table = [
            (
                Degree::GrandSupercycle,
                "((I)) ((II)) ((III)) ((IV)) ((V))",
                "((a)) ((b)) ((c))",
            ),
            (Degree::Supercycle, "(I) (II) (III) (IV) (V)", "(a) (b) (c)"),
            (Degree::Cycle, "I II III IV V", "a b c"),
            (
                Degree::Primary,
                "((1)) ((2)) ((3)) ((4)) ((5))",
                "((A)) ((B)) ((C))",
            ),
            (Degree::Intermediate, "(1) (2) (3) (4) (5)", "(A) (B) (C)"),
            (Degree::Minor, "1 2 3 4 5", "A B C"),
            (
                Degree::Minute,
                "((i)) ((ii)) ((iii)) ((iv)) ((v))",
                "((a)) ((b)) ((c))",
            ),
            (Degree::Minuette, "(i) (ii) (iii) (iv) (v)", "(a) (b) (c)"),
            (Degree::Subminuette, "i ii iii iv v", "a b c"),
        ];
        assert_eq!(table.len(), Degree::ALL.len());
        for (degree, numbered, lettered) in table {
            assert_eq!(impulse(degree), numbered, "{degree:?}");
            assert_eq!(correction(degree), lettered, "{degree:?}");
        }
    }

    #[test]
    fn changing_the_degree_rewrites_every_label_and_only_the_labels() {
        let minor = Degree::Minor.pivot_labels(WaveKind::Triangle);
        let minute = Degree::Minute.pivot_labels(WaveKind::Triangle);
        assert_eq!(minor.len(), minute.len());
        assert!(minor[1..].iter().zip(&minute[1..]).all(|(a, b)| a != b));
    }

    #[test]
    fn labels_use_only_characters_every_font_has() {
        for degree in Degree::ALL {
            for kind in WaveKind::ALL {
                for label in degree.pivot_labels(kind) {
                    assert!(
                        label
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '(' || c == ')'),
                        "{degree:?} {kind:?} {label}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_default_degree_is_intermediate_and_the_samples_show_each_notation() {
        assert_eq!(Degree::default(), Degree::Intermediate);
        assert_eq!(Degree::Intermediate.sample(), "(1)");
        assert_eq!(Degree::Minor.sample(), "1");
        assert_eq!(Degree::Primary.sample(), "((1))");
        assert_eq!(Degree::Cycle.sample(), "I");
        assert_eq!(roman(0), "");
        assert_eq!(roman(9), "");
    }
}
