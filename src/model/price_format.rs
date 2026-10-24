//! How a chart prints a price: adaptive precision, with a choice of how the very small ones read.
//!
//! * Automatic precision keeps a price readable, not exact: no decimals from 1,000, two from 1,
//!   then two significant digits below 1. `67,432`, `3,456`, `12.34`, `0.12`, `0.056`, `0.0031`.
//!   (The Swift app's `PriceFormatter` gave four to six places there; this is a deliberate change.
//!   An axis label still takes the places its tick spacing needs, see [`PriceFormat::format_tick`].)
//! * Under 0.001 the zeros are counted, CoinMarketCap style: `0.00000278` reads `0.0₅278`.
//! * A fixed number of decimal places replaces the automatic precision. The Swift app still
//!   counts the zeros of a tiny price then; [`SmallPrices::Decimals`] spells it out instead.
//!
//! Pure: no egui, no locale. Thousands are grouped with commas and the point is a point.

use crate::format::group_thousands;
use serde::{Deserialize, Serialize};

/// How a price under [`SMALL_BELOW`] is written.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SmallPrices {
    /// `0.0₅278`: the subscript counts the zeros after the point, three digits follow.
    #[default]
    ZeroCount,
    /// `0.00000278`: every zero written out, to the fixed places or (automatic) three digits
    /// past the zeros.
    Decimals,
}

impl SmallPrices {
    pub const ALL: [Self; 2] = [Self::ZeroCount, Self::Decimals];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ZeroCount => "Count the zeros",
            Self::Decimals => "Write out the zeros",
        }
    }

    /// A tiny price as this style writes it, for the settings preview.
    pub const fn example(self) -> &'static str {
        match self {
            Self::ZeroCount => "0.0₅278",
            Self::Decimals => "0.00000278",
        }
    }
}

/// A price below this counts its zeros (when it is allowed to).
pub const SMALL_BELOW: f64 = 0.001;
/// The most decimal places a chart can be pinned to.
pub const MAX_DECIMALS: u8 = 12;
/// Written out in full, a price never runs past this many decimals; beyond it the zeros are
/// counted whatever the style says (`0.000000000000000012` is unreadable).
const MAX_WRITTEN_DECIMALS: usize = 16;
/// Digits shown after the zeros of a small price.
const SMALL_DIGITS: usize = 3;
/// Past this a price has no decimals: it is large enough that they are noise.
const GROUPED_FROM: f64 = 1_000.0;

/// A chart's price display: pinned decimal places or automatic, and how a tiny price reads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PriceFormat {
    /// `None` chooses the places from the magnitude; `Some(n)` fixes them (at most
    /// [`MAX_DECIMALS`]).
    pub decimals: Option<u8>,
    pub small: SmallPrices,
}

impl PriceFormat {
    pub const AUTO: Self = Self {
        decimals: None,
        small: SmallPrices::ZeroCount,
    };

    /// `value` as this chart prints it. Not finite reads `—`; zero reads `0` (`0.00` once the
    /// places are fixed); a negative keeps its sign unless it rounds to nothing.
    pub fn format(self, value: f64) -> String {
        self.format_at(value, 0)
    }

    /// An axis tick: like [`format`](Self::format), but automatic precision never shows fewer
    /// places than `step` (the distance to the next tick) needs, so neighbouring labels differ.
    pub fn format_tick(self, value: f64, step: f64) -> String {
        self.format_at(value, places_for_step(step))
    }

    /// `value` as a bare number for the clipboard or an editor field: no thousands commas and no
    /// subscript, so it parses back. Pinned places are kept; otherwise it carries the chart's
    /// automatic places but never fewer than two from 1 up (a 118,151 label still copies its
    /// cents) and written-out zeros below 0.001. Not finite is empty.
    pub fn plain(self, value: f64) -> String {
        if !value.is_finite() {
            return String::new();
        }
        let magnitude = value.abs();
        let places = match self.decimals {
            Some(places) => usize::from(places.min(MAX_DECIMALS)),
            None if magnitude == 0.0 => 0,
            None if magnitude < SMALL_BELOW => {
                let (_, exponent) = scientific(magnitude);
                ((-exponent) as usize - 1 + SMALL_DIGITS).min(MAX_WRITTEN_DECIMALS)
            }
            None => auto_places(magnitude).max(if magnitude >= 1.0 { 2 } else { 0 }),
        };
        let text = format!("{magnitude:.places$}");
        if value < 0.0 && text.chars().any(|c| c.is_ascii_digit() && c != '0') {
            format!("-{text}")
        } else {
            text
        }
    }

    /// One step of the last place this chart prints `price` to: `1` for 67,432, `0.01` for 12.34,
    /// `0.001` for 0.056. It is not an exchange's tick (this build knows no market's real tick
    /// size); measurements that count ticks count these, and say so.
    pub fn tick_size(self, price: f64) -> f64 {
        let magnitude = price.abs();
        let places = match self.decimals {
            Some(places) => usize::from(places.min(MAX_DECIMALS)),
            None if !magnitude.is_finite() || magnitude == 0.0 => 2,
            None if magnitude < SMALL_BELOW => {
                let (_, exponent) = scientific(magnitude);
                ((-exponent) as usize - 1 + SMALL_DIGITS).min(MAX_WRITTEN_DECIMALS)
            }
            None => auto_places(magnitude),
        };
        10f64.powi(-(places as i32))
    }

    fn format_at(self, value: f64, at_least: usize) -> String {
        if !value.is_finite() {
            return "—".to_owned();
        }
        let magnitude = value.abs();
        let text = self.magnitude_text(magnitude, at_least);
        if value < 0.0 && text.chars().any(|c| c.is_ascii_digit() && c != '0') {
            format!("-{text}")
        } else {
            text
        }
    }

    fn magnitude_text(self, magnitude: f64, at_least: usize) -> String {
        let fixed = self
            .decimals
            .map(|places| usize::from(places.min(MAX_DECIMALS)));
        if magnitude == 0.0 {
            return fixed.map_or_else(|| "0".to_owned(), |places| format!("{:.places$}", 0.0));
        }
        if magnitude < SMALL_BELOW {
            return self.small_text(magnitude, fixed);
        }
        let places = fixed.unwrap_or_else(|| auto_places(magnitude).max(at_least));
        group_thousands(&format!("{magnitude:.places$}"))
    }

    fn small_text(self, magnitude: f64, fixed: Option<usize>) -> String {
        match (self.small, fixed) {
            (SmallPrices::Decimals, Some(places)) => format!("{magnitude:.places$}"),
            (SmallPrices::Decimals, None) => {
                let (_, exponent) = scientific(magnitude);
                // 2.78e-6 has five zeros; three digits after them.
                let places = (-exponent) as usize - 1 + SMALL_DIGITS;
                if places <= MAX_WRITTEN_DECIMALS {
                    format!("{magnitude:.places$}")
                } else {
                    counted_zeros(magnitude)
                }
            }
            (SmallPrices::ZeroCount, _) => counted_zeros(magnitude),
        }
    }
}

/// The places automatic precision gives a price (>= 0.001): none once it is large enough that
/// decimals are noise, two from 1, then enough to keep two significant digits (`0.12`, `0.056`,
/// `0.0031`). Under 0.001 the zeros are counted instead.
fn auto_places(magnitude: f64) -> usize {
    if magnitude >= GROUPED_FROM {
        0
    } else if magnitude >= 0.1 {
        2
    } else if magnitude >= 0.01 {
        3
    } else {
        4
    }
}

/// Decimal places that tell apart two ticks `step` apart: `0.5` needs one, `0.25` two, `5` none.
pub(super) fn places_for_step(step: f64) -> usize {
    if !step.is_finite() || step <= 0.0 {
        return 0;
    }
    let mut places = (-step.log10()).ceil().max(0.0) as usize;
    while places < usize::from(MAX_DECIMALS) {
        let scaled = step * 10f64.powi(places as i32);
        if (scaled - scaled.round()).abs() < 1e-6 * scaled.max(1.0) {
            break;
        }
        places += 1;
    }
    places.min(usize::from(MAX_DECIMALS))
}

/// `mantissa digits` (`"278"`) and the decimal exponent (`-6`) of `magnitude` rounded to three
/// significant digits. Rounding in the formatter, not a float loop, so `0.00099999` is `100`/`-3`
/// and never `1000`.
fn scientific(magnitude: f64) -> (String, i32) {
    let text = format!("{magnitude:.2e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    (
        mantissa.replace('.', ""),
        exponent.parse().unwrap_or_default(),
    )
}

/// `0.0₅278`: the Swift app's `0.0` + zero count + three digits, for a magnitude under 0.001.
fn counted_zeros(magnitude: f64) -> String {
    let (digits, exponent) = scientific(magnitude);
    let zeros = (-exponent - 1).max(0) as u32;
    format!("0.0{}{digits}", subscript(zeros))
}

fn subscript(number: u32) -> String {
    number
        .to_string()
        .chars()
        .filter_map(|digit| digit.to_digit(10))
        .filter_map(|digit| char::from_u32(0x2080 + digit))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTO: PriceFormat = PriceFormat::AUTO;

    fn fixed(places: u8) -> PriceFormat {
        PriceFormat {
            decimals: Some(places),
            small: SmallPrices::ZeroCount,
        }
    }

    fn written(decimals: Option<u8>) -> PriceFormat {
        PriceFormat {
            decimals,
            small: SmallPrices::Decimals,
        }
    }

    #[test]
    fn automatic_precision_keeps_two_significant_digits_under_one() {
        assert_eq!(AUTO.format(67_432.19), "67,432");
        assert_eq!(AUTO.format(1_000.0), "1,000");
        assert_eq!(AUTO.format(999.994), "999.99");
        assert_eq!(AUTO.format(45.231), "45.23");
        assert_eq!(AUTO.format(1.0), "1.00");
        assert_eq!(AUTO.format(0.999), "1.00");
        assert_eq!(AUTO.format(0.5), "0.50");
        assert_eq!(AUTO.format(0.123_456), "0.12");
        assert_eq!(AUTO.format(0.1), "0.10");
        assert_eq!(AUTO.format(0.056_78), "0.057");
        assert_eq!(AUTO.format(0.01), "0.010");
        assert_eq!(AUTO.format(0.009_1), "0.0091");
        assert_eq!(AUTO.format(0.003_14), "0.0031");
        assert_eq!(AUTO.format(0.001_234_6), "0.0012");
        assert_eq!(AUTO.format(1_234_567.8), "1,234,568");
    }

    #[test]
    fn every_automatic_label_below_a_thousand_has_the_places_of_its_size() {
        // The places only ever grow as the price shrinks, so a fall never loses a digit.
        let mut last = 0;
        for exponent in (-30..=3).rev() {
            let places = auto_places(10f64.powf(f64::from(exponent) / 10.0));
            assert!(places >= last, "{exponent}: {places} < {last}");
            last = places;
        }
    }

    #[test]
    fn a_tiny_price_counts_its_zeros() {
        assert_eq!(AUTO.format(0.000_002_78), "0.0₅278");
        assert_eq!(AUTO.format(0.000_9), "0.0₃900");
        assert_eq!(AUTO.format(0.000_012_34), "0.0₄123");
        assert_eq!(AUTO.format(0.000_000_000_5), "0.0₉500");
        assert_eq!(AUTO.format(1.5e-120), "0.0₁₁₉150");
    }

    #[test]
    fn counted_zeros_roll_over_instead_of_printing_a_fourth_digit() {
        // 0.00099999 rounds to 0.00100: two zeros and 100, as the Swift loop's overflow branch.
        assert_eq!(AUTO.format(0.000_999_99), "0.0₂100");
        assert_eq!(AUTO.format(0.000_099_999), "0.0₃100");
        assert_eq!(AUTO.format(0.000_1), "0.0₃100");
    }

    #[test]
    fn pinned_places_replace_the_automatic_precision() {
        assert_eq!(fixed(0).format(67_432.19), "67,432");
        assert_eq!(fixed(3).format(67_432.19), "67,432.190");
        assert_eq!(fixed(2).format(1_234_567.891), "1,234,567.89");
        assert_eq!(fixed(6).format(0.5), "0.500000");
        assert_eq!(fixed(2).format(0.0), "0.00");
        assert_eq!(fixed(40).format(1.5), "1.500000000000");
    }

    #[test]
    fn a_pinned_chart_still_counts_the_zeros_of_a_tiny_price_as_swift_does() {
        assert_eq!(fixed(2).format(0.000_002_78), "0.0₅278");
        assert_eq!(fixed(8).format(0.000_002_78), "0.0₅278");
    }

    #[test]
    fn writing_the_zeros_out_follows_the_places_or_the_magnitude() {
        assert_eq!(written(Some(8)).format(0.000_002_78), "0.00000278");
        assert_eq!(written(Some(4)).format(0.000_002_78), "0.0000");
        assert_eq!(written(None).format(0.000_002_78), "0.00000278");
        assert_eq!(written(None).format(0.000_000_012_345), "0.0000000123");
        // A price that would need more than sixteen decimals counts instead.
        assert_eq!(written(None).format(1.2e-20), "0.0₁₉120");
        // The prices above 0.001 are not small.
        assert_eq!(written(None).format(0.5), AUTO.format(0.5));
        assert_eq!(written(None).format(67_432.19), "67,432");
    }

    #[test]
    fn zero_negative_and_not_a_number() {
        assert_eq!(AUTO.format(0.0), "0");
        assert_eq!(AUTO.format(-0.0), "0");
        assert_eq!(AUTO.format(-12.3456), "-12.35");
        assert_eq!(AUTO.format(-0.000_002_78), "-0.0₅278");
        assert_eq!(fixed(2).format(-0.001), "0.00", "no minus on a zero");
        assert_eq!(
            written(Some(2)).format(-0.000_000_1),
            "0.00",
            "no minus on a zero"
        );
        assert_eq!(AUTO.format(f64::NAN), "—");
        assert_eq!(AUTO.format(f64::INFINITY), "—");
    }

    #[test]
    fn ticks_show_the_places_their_step_needs() {
        assert_eq!(AUTO.format_tick(67_432.5, 0.5), "67,432.5");
        assert_eq!(AUTO.format_tick(67_433.0, 0.5), "67,433.0");
        assert_eq!(AUTO.format_tick(67_432.25, 0.25), "67,432.25");
        assert_eq!(AUTO.format_tick(67_430.0, 10.0), "67,430");
        assert_eq!(AUTO.format_tick(12.34, 0.01), "12.34");
        // Never fewer places than the price's own precision, and never more than pinned.
        assert_eq!(AUTO.format_tick(1.5, 0.5), "1.50");
        assert_eq!(fixed(1).format_tick(67_432.26, 0.25), "67,432.3");
        // A tiny price does not take decimals from its step.
        assert_eq!(AUTO.format_tick(0.000_002_78, 0.000_000_1), "0.0₅278");
    }

    #[test]
    fn the_step_places() {
        assert_eq!(places_for_step(5.0), 0);
        assert_eq!(places_for_step(0.5), 1);
        assert_eq!(places_for_step(0.25), 2);
        assert_eq!(places_for_step(0.02), 2);
        assert_eq!(places_for_step(2.5), 1);
        assert_eq!(places_for_step(0.0), 0);
        assert_eq!(places_for_step(f64::NAN), 0);
    }

    #[test]
    fn the_settings_examples_are_what_the_formatter_writes() {
        for style in SmallPrices::ALL {
            let format = PriceFormat {
                decimals: None,
                small: style,
            };
            assert_eq!(format.format(0.000_002_78), style.example());
        }
    }

    #[test]
    fn the_style_reads_as_snake_case_and_defaults_to_counting() {
        assert_eq!(
            serde_json::to_string(&SmallPrices::Decimals).unwrap(),
            "\"decimals\""
        );
        assert_eq!(SmallPrices::default(), SmallPrices::ZeroCount);
    }

    #[test]
    fn plain_is_a_parseable_number_with_the_charts_places() {
        assert_eq!(AUTO.plain(118_150.99), "118150.99");
        assert_eq!(AUTO.plain(45.231), "45.23");
        assert_eq!(AUTO.plain(0.123_456), "0.12");
        assert_eq!(AUTO.plain(0.003_14), "0.0031");
        assert_eq!(AUTO.plain(0.000_002_78), "0.00000278");
        assert_eq!(AUTO.plain(0.0), "0");
        assert_eq!(AUTO.plain(-12.345), "-12.35");
        assert_eq!(AUTO.plain(f64::NAN), "");
        assert_eq!(fixed(3).plain(67_432.19), "67432.190");
        assert_eq!(written(Some(8)).plain(0.000_002_78), "0.00000278");
        for value in [118_150.99, 0.0031, 0.000_002_78, 7.5] {
            let text = AUTO.plain(value);
            let parsed: f64 = text.parse().expect("plain text parses");
            assert!((parsed - value).abs() <= value * 0.01, "{text}");
        }
    }

    #[test]
    fn a_tick_is_one_step_of_the_last_place_the_chart_prints() {
        let auto = PriceFormat::AUTO;
        assert_eq!(auto.tick_size(67_432.0), 1.0);
        assert_eq!(auto.tick_size(1_000.0), 1.0);
        assert_eq!(auto.tick_size(12.34), 0.01);
        assert_eq!(auto.tick_size(0.5), 0.01);
        assert_eq!(auto.tick_size(0.056), 0.001);
        assert_eq!(auto.tick_size(0.0031), 0.0001);
        // Under 0.001 it is the place of the third digit after the zeros: 0.0000278 -> 7 places.
        assert_eq!(auto.tick_size(0.0000278), 1e-7);
        // A sign changes nothing; nothing to go on is a cent.
        assert_eq!(auto.tick_size(-12.34), 0.01);
        assert_eq!(auto.tick_size(0.0), 0.01);
        assert_eq!(auto.tick_size(f64::NAN), 0.01);
        // Pinned places are the places.
        let pinned = PriceFormat {
            decimals: Some(3),
            small: SmallPrices::ZeroCount,
        };
        assert_eq!(pinned.tick_size(67_432.0), 0.001);
        assert_eq!(pinned.tick_size(0.0000278), 0.001);
        let none = PriceFormat {
            decimals: Some(0),
            small: SmallPrices::ZeroCount,
        };
        assert_eq!(none.tick_size(12.34), 1.0);
    }
}
