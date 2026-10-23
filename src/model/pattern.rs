//! The chart patterns (XABCD, Cypher, Head and shoulders, ABCD, Triangle, Three drives): how many
//! pivots each takes, what its pivots are called, and the Fibonacci ratios it prints between
//! its legs.
//!
//! Everything here is worked from the stored anchors' real prices, never from pixels, so a
//! ratio is the same on a linear, a logarithmic, a percent or an indexed price axis. Pure: no
//! egui types.
//!
//! Each ratio names its **numerator and denominator legs** and the **measuring line** it is
//! printed on. The tools do not share one ratio function because they do not measure the same
//! legs: TradingView's XABCD page measures a Gartley's D against XA, a Cypher's against XC.

use super::drawing_tool::Tool;
use super::drawings::Anchor;
use serde::{Deserialize, Serialize};

/// The most a printed ratio may be; anything beyond it is a degenerate drawing, not a ratio.
pub const MAX_RATIO: f64 = 1.0e6;

/// Which chart pattern a [`Geometry::Pattern`](super::drawings::Geometry::Pattern) is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    Xabcd,
    Cypher,
    HeadAndShoulders,
    Abcd,
    Triangle,
    ThreeDrives,
}

impl PatternKind {
    #[cfg(test)]
    pub const ALL: [Self; 6] = [
        Self::Xabcd,
        Self::Cypher,
        Self::HeadAndShoulders,
        Self::Abcd,
        Self::Triangle,
        Self::ThreeDrives,
    ];

    /// How many pivots the pattern is placed with.
    ///
    /// XABCD, Cypher, ABCD and Triangle are **doc** (TradingView's help pages: five, five, four
    /// and four points). Head and shoulders has seven and Three drives six: **inferred**,
    /// because TradingView's articles for them describe the formation and not the tool (its
    /// Three drives page names five pivots, 1, 2, 3, A and C, and the point on which the first
    /// drive starts; its Coordinates tab lists seven fields, which the page does not explain).
    pub const fn points(self) -> usize {
        match self {
            Self::Xabcd | Self::Cypher => 5,
            Self::HeadAndShoulders => 7,
            Self::Abcd | Self::Triangle => 4,
            Self::ThreeDrives => 6,
        }
    }

    pub const fn tool(self) -> Tool {
        match self {
            Self::Xabcd => Tool::XabcdPattern,
            Self::Cypher => Tool::CypherPattern,
            Self::HeadAndShoulders => Tool::HeadAndShoulders,
            Self::Abcd => Tool::AbcdPattern,
            Self::Triangle => Tool::TrianglePattern,
            Self::ThreeDrives => Tool::ThreeDrivesPattern,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::XabcdPattern => Some(Self::Xabcd),
            Tool::CypherPattern => Some(Self::Cypher),
            Tool::HeadAndShoulders => Some(Self::HeadAndShoulders),
            Tool::AbcdPattern => Some(Self::Abcd),
            Tool::TrianglePattern => Some(Self::Triangle),
            Tool::ThreeDrivesPattern => Some(Self::ThreeDrives),
            _ => None,
        }
    }

    /// The name of each pivot, in placement order; an empty name is a pivot that is not labelled.
    pub const fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Xabcd | Self::Cypher => &["X", "A", "B", "C", "D"],
            Self::Abcd | Self::Triangle => &["A", "B", "C", "D"],
            Self::HeadAndShoulders => &["", "Left Shoulder", "", "Head", "", "Right Shoulder", ""],
            Self::ThreeDrives => &["", "1", "A", "2", "C", "3"],
        }
    }

    /// The pivots the neckline runs through (zero-based), for Head and shoulders: the two
    /// troughs either side of the head.
    pub const fn neckline(self) -> Option<(usize, usize)> {
        match self {
            Self::HeadAndShoulders => Some((2, 4)),
            _ => None,
        }
    }
}

/// One measured ratio and where it is printed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ratio {
    /// The numerator leg over the denominator leg, both as absolute price distances.
    pub value: f64,
    /// The pivot the measuring line starts at (the start of the reference leg).
    pub from: usize,
    /// The pivot it ends at (the end of the leg the ratio is about).
    pub to: usize,
}

/// The price distance between pivots `i` and `j`.
fn leg(points: &[Anchor], i: usize, j: usize) -> Option<f64> {
    let (a, b) = (points.get(i)?, points.get(j)?);
    let length = (b.price - a.price).abs();
    length.is_finite().then_some(length)
}

/// `numerator` over `denominator`, or `None` when the denominator is nothing (a zero-length
/// reference leg has no ratio) or the result is not a number worth printing.
pub fn ratio_of(numerator: f64, denominator: f64) -> Option<f64> {
    if !(numerator.is_finite() && denominator.is_finite()) || denominator <= 0.0 {
        return None;
    }
    let value = numerator / denominator;
    (value.is_finite() && value <= MAX_RATIO).then_some(value)
}

/// Pivot `numerator.0`-`numerator.1` over pivot `denominator.0`-`denominator.1`, printed on the
/// line from `from` to `to`.
fn measure(
    points: &[Anchor],
    numerator: (usize, usize),
    denominator: (usize, usize),
    from: usize,
    to: usize,
) -> Option<Ratio> {
    let value = ratio_of(
        leg(points, numerator.0, numerator.1)?,
        leg(points, denominator.0, denominator.1)?,
    )?;
    Some(Ratio { value, from, to })
}

/// The ratios `kind` prints for `points`, in the order of their measuring lines. A ratio whose
/// reference leg has no length is left out rather than shown as NaN or infinity.
///
/// - **XABCD** (X=0 A=1 B=2 C=3 D=4): B is AB over XA (printed on X-B); C is BC over AB (A-C);
///   D is CD over BC (B-D), and AD over XA (X-D), the measure a Gartley's 0.786 and a
///   Butterfly's 1.272 are quoted in. The rules are **doc** (TradingView's XABCD page); that the
///   tool prints these four, on these lines, is **inferred**.
/// - **Cypher**: B is AB over XA (X-B); C is XC over XA, its extension (X-C); D is CD over XC,
///   the retracement of the whole XC leg (X-D). **doc** for the rules.
/// - **ABCD** (A=0 .. D=3): BC over AB (A-C) and CD over BC (B-D). **doc** for the rules.
/// - **Three drives** (start=0, 1, A, 2, C, 3): A retraces drive 1 (start-A); drive 2 extends A
///   (1-2); C retraces drive 2 (A-C); drive 3 extends C (2-3). **doc** for the rules.
pub fn ratios(kind: PatternKind, points: &[Anchor]) -> Vec<Ratio> {
    if points.len() != kind.points() {
        return Vec::new();
    }
    let wanted: [Option<Ratio>; 4] = match kind {
        PatternKind::Xabcd => [
            measure(points, (1, 2), (0, 1), 0, 2),
            measure(points, (2, 3), (1, 2), 1, 3),
            measure(points, (3, 4), (2, 3), 2, 4),
            measure(points, (1, 4), (0, 1), 0, 4),
        ],
        PatternKind::Cypher => [
            measure(points, (1, 2), (0, 1), 0, 2),
            measure(points, (0, 3), (0, 1), 0, 3),
            measure(points, (3, 4), (0, 3), 0, 4),
            None,
        ],
        PatternKind::Abcd => [
            measure(points, (1, 2), (0, 1), 0, 2),
            measure(points, (2, 3), (1, 2), 1, 3),
            None,
            None,
        ],
        PatternKind::ThreeDrives => [
            measure(points, (1, 2), (0, 1), 0, 2),
            measure(points, (2, 3), (1, 2), 1, 3),
            measure(points, (3, 4), (2, 3), 2, 4),
            measure(points, (4, 5), (3, 4), 3, 5),
        ],
        PatternKind::HeadAndShoulders | PatternKind::Triangle => [None; 4],
    };
    wanted.into_iter().flatten().collect()
}

/// A ratio as printed: `precision` places, trailing zeros dropped (`0.618`, `1.272`, `0.5`, `1`).
pub fn format_ratio(value: f64, precision: u8) -> String {
    let places = usize::from(precision.min(6));
    let text = format!("{value:.places$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// Whether pivot `index` of a zigzag is a high (the line turns down there) rather than a low. An
/// end pivot is a high when it is higher than its only neighbour. A pivot level with its
/// neighbours counts as a high, so a flat zigzag still has labels on a side.
pub fn is_high(points: &[Anchor], index: usize) -> bool {
    let price = |i: usize| points.get(i).map(|anchor| anchor.price);
    let here = match price(index) {
        Some(price) => price,
        None => return true,
    };
    let before = index.checked_sub(1).and_then(price);
    let after = price(index + 1);
    match (before, after) {
        (Some(before), Some(after)) => here >= before && here >= after,
        (Some(only), None) | (None, Some(only)) => here >= only,
        (None, None) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(price: f64) -> Anchor {
        Anchor { time_ms: 0, price }
    }

    fn series(prices: &[f64]) -> Vec<Anchor> {
        prices.iter().map(|price| at(*price)).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn every_kind_is_one_tool_and_back_with_its_documented_pivot_count() {
        for kind in PatternKind::ALL {
            assert_eq!(PatternKind::of_tool(kind.tool()), Some(kind));
            assert_eq!(kind.labels().len(), kind.points(), "{kind:?}");
        }
        assert_eq!(PatternKind::Xabcd.points(), 5);
        assert_eq!(PatternKind::Cypher.points(), 5);
        assert_eq!(PatternKind::Abcd.points(), 4);
        assert_eq!(PatternKind::Triangle.points(), 4);
    }

    #[test]
    fn a_bullish_gartley_prints_its_textbook_ratios() {
        // X=100 A=200 (XA=100); B retraces 0.618 of XA to 138.2; C retraces half of AB (61.8)
        // to 169.1; D at the 0.786 retracement of XA is 200 - 78.6 = 121.4.
        let points = series(&[100.0, 200.0, 138.2, 169.1, 121.4]);
        let measured = ratios(PatternKind::Xabcd, &points);
        let values: Vec<f64> = measured.iter().map(|ratio| ratio.value).collect();
        assert_eq!(measured.len(), 4);
        assert!(close(values[0], 61.8 / 100.0), "AB / XA");
        assert!(close(values[1], 30.9 / 61.8), "BC / AB");
        // CD = 169.1 - 121.4 = 47.7 over BC = 30.9.
        assert!(close(values[2], 47.7 / 30.9), "CD / BC");
        assert!(close(values[3], 78.6 / 100.0), "AD / XA");
        // Each is printed on the line from the start of its reference leg to its own pivot.
        let lines: Vec<(usize, usize)> = measured.iter().map(|r| (r.from, r.to)).collect();
        assert_eq!(lines, [(0, 2), (1, 3), (2, 4), (0, 4)]);
    }

    #[test]
    fn a_bearish_pattern_gives_the_same_ratios_as_its_mirror() {
        let bull = series(&[100.0, 200.0, 138.2, 169.1, 121.4]);
        // The same pattern upside down: 300 - price.
        let bear: Vec<Anchor> = bull.iter().map(|a| at(300.0 - a.price)).collect();
        let a = ratios(PatternKind::Xabcd, &bull);
        let b = ratios(PatternKind::Xabcd, &bear);
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(&b) {
            assert!(close(a.value, b.value));
        }
    }

    #[test]
    fn the_cypher_measures_c_and_d_against_xa_and_xc_not_as_xabcd_does() {
        // Bullish Cypher: X=100 A=200; B retraces half of XA to 150; C extends 1.272 of XA from
        // X to 227.2 (XC = 127.2); D retraces 0.786 of the whole XC leg back from C.
        let d = 227.2 - 0.786 * (227.2 - 100.0);
        let points = series(&[100.0, 200.0, 150.0, 227.2, d]);
        let measured = ratios(PatternKind::Cypher, &points);
        let values: Vec<f64> = measured.iter().map(|ratio| ratio.value).collect();
        assert_eq!(measured.len(), 3);
        assert!(close(values[0], 50.0 / 100.0), "AB / XA");
        assert!(close(values[1], 127.2 / 100.0), "XC / XA, the extension");
        assert!(close(values[2], 0.786), "CD / XC, the retracement of XC");
        let lines: Vec<(usize, usize)> = measured.iter().map(|r| (r.from, r.to)).collect();
        assert_eq!(lines, [(0, 2), (0, 3), (0, 4)]);
        // The XABCD tool would have called the same five points something else entirely.
        assert_ne!(ratios(PatternKind::Xabcd, &points).len(), measured.len());
    }

    #[test]
    fn abcd_measures_bc_over_ab_and_cd_over_bc() {
        // AB = 20 (100 -> 120), BC = 12.36 (a 0.618 retracement), CD = 20 (= AB).
        let points = series(&[100.0, 120.0, 107.64, 127.64]);
        let measured = ratios(PatternKind::Abcd, &points);
        assert_eq!(measured.len(), 2);
        assert!(close(measured[0].value, 12.36 / 20.0));
        assert!(close(measured[1].value, 20.0 / 12.36));
    }

    #[test]
    fn three_drives_measure_each_retracement_and_each_extension_against_the_leg_before() {
        // start 100, drive 1 to 120, A back to 110 (0.5), drive 2 to 125 (1.5 of A),
        // C back to 117.5 (0.5), drive 3 to 130 (1.6667 of C).
        let points = series(&[100.0, 120.0, 110.0, 125.0, 117.5, 130.0]);
        let values: Vec<f64> = ratios(PatternKind::ThreeDrives, &points)
            .iter()
            .map(|ratio| ratio.value)
            .collect();
        assert_eq!(values.len(), 4);
        assert!(close(values[0], 10.0 / 20.0));
        assert!(close(values[1], 15.0 / 10.0));
        assert!(close(values[2], 7.5 / 15.0));
        assert!(close(values[3], 12.5 / 7.5));
    }

    #[test]
    fn patterns_with_no_ratios_print_none() {
        let flat = series(&[1.0, 2.0, 1.0, 3.0, 1.0, 2.0, 1.0]);
        assert!(ratios(PatternKind::HeadAndShoulders, &flat).is_empty());
        assert!(ratios(PatternKind::Triangle, &flat[..4]).is_empty());
    }

    #[test]
    fn degenerate_legs_have_no_ratio_instead_of_nan_or_infinity() {
        // XA has no length: nothing is measured against it.
        let points = series(&[100.0, 100.0, 90.0, 95.0, 92.0]);
        let measured = ratios(PatternKind::Xabcd, &points);
        // AB/XA and AD/XA are gone; BC/AB and CD/BC remain.
        let lines: Vec<(usize, usize)> = measured.iter().map(|r| (r.from, r.to)).collect();
        assert_eq!(lines, [(1, 3), (2, 4)]);
        assert!(measured.iter().all(|r| r.value.is_finite()));
        // Every pivot on one price: no ratio at all.
        assert!(ratios(PatternKind::Xabcd, &series(&[5.0; 5])).is_empty());
        // Negative and huge prices are just prices.
        let wide = series(&[-1.0e12, 1.0e12, -5.0e11, 5.0e11, 0.0]);
        assert!(
            ratios(PatternKind::Xabcd, &wide)
                .iter()
                .all(|r| r.value.is_finite() && r.value <= MAX_RATIO)
        );
        // A ratio past the limit is a degenerate drawing, not a ratio.
        assert_eq!(ratio_of(1.0e9, 1.0), None);
        assert_eq!(ratio_of(1.0, 0.0), None);
        assert_eq!(ratio_of(f64::NAN, 1.0), None);
        assert_eq!(ratio_of(1.0, f64::INFINITY), None);
    }

    #[test]
    fn the_wrong_number_of_pivots_measures_nothing() {
        assert!(ratios(PatternKind::Xabcd, &series(&[1.0, 2.0, 3.0])).is_empty());
        assert!(ratios(PatternKind::Abcd, &series(&[1.0; 5])).is_empty());
    }

    #[test]
    fn ratios_are_printed_to_the_precision_with_no_trailing_zeros() {
        assert_eq!(format_ratio(0.618, 3), "0.618");
        assert_eq!(format_ratio(1.2720001, 3), "1.272");
        assert_eq!(format_ratio(0.5, 3), "0.5");
        assert_eq!(format_ratio(1.0, 3), "1");
        assert_eq!(format_ratio(0.7861, 2), "0.79");
        assert_eq!(format_ratio(2.0, 0), "2");
        assert_eq!(format_ratio(0.4, 0), "0");
        assert_eq!(format_ratio(1.0 / 3.0, 9), "0.333333");
    }

    #[test]
    fn a_pivot_is_a_high_when_it_is_at_least_as_high_as_its_neighbours() {
        let zigzag = series(&[1.0, 5.0, 2.0, 6.0, 3.0]);
        let highs: Vec<bool> = (0..5).map(|i| is_high(&zigzag, i)).collect();
        assert_eq!(highs, [false, true, false, true, false]);
        let flat = series(&[2.0, 2.0, 2.0]);
        assert!((0..3).all(|i| is_high(&flat, i)));
        assert!(is_high(&zigzag, 99));
    }

    #[test]
    fn head_and_shoulders_has_a_neckline_and_the_others_have_none() {
        assert_eq!(PatternKind::HeadAndShoulders.neckline(), Some((2, 4)));
        assert_eq!(PatternKind::Xabcd.neckline(), None);
    }
}
