//! The four price-scale modes of a chart and the maths that tells them apart. Pure: no egui, so
//! it is tested without a window.
//!
//! * **Regular**: the axis shows prices, equal price differences take equal heights.
//! * **Percent**: the axis shows the change from a reference price, `100 * (price - ref) / |ref|`.
//! * **Indexed to 100**: the same, plus 100, so the reference reads `100`.
//! * **Logarithmic**: the axis shows prices, equal ratios take equal heights.
//!
//! Percent and indexed are linear in the price: they move no pixel, only what the axis, its tags
//! and the crosshair say. Every price a chart stores, draws from or sends (drawing anchors,
//! alerts, orders) stays a real price; a display value never goes back into any of them.
//!
//! The reference is the close of the first bar the chart shows, so it follows a pan. This is what
//! TradingView's docs say ("the first value on the chart is the base"); that the first bar may be
//! cut by the plot's edge and still count is inferred, and no live TradingView chart was compared.

use super::price_format::places_for_step;
use crate::format::group_thousands;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A chart's price-scale mode. Exactly one is active, so the combinations that would contradict
/// (percent and logarithmic) cannot be written.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PriceScaleMode {
    #[default]
    Regular,
    Percent,
    IndexedTo100,
    Logarithmic,
}

impl PriceScaleMode {
    /// The order of the menu.
    pub const ALL: [Self; 4] = [
        Self::Regular,
        Self::Percent,
        Self::IndexedTo100,
        Self::Logarithmic,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Regular => "Regular",
            Self::Percent => "Percent",
            Self::IndexedTo100 => "Indexed to 100",
            Self::Logarithmic => "Logarithmic",
        }
    }

    /// The stored name.
    const fn name(self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::Percent => "percent",
            Self::IndexedTo100 => "indexed_to_100",
            Self::Logarithmic => "logarithmic",
        }
    }

    /// The mode a stored name stands for; a name this version does not know (a newer build's) is
    /// Regular, so a layout always opens.
    fn from_name(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|mode| mode.name() == name)
            .unwrap_or_default()
    }

    /// The legacy on/off log switch older builds stored.
    pub const fn from_log_flag(logarithmic: bool) -> Self {
        if logarithmic {
            Self::Logarithmic
        } else {
            Self::Regular
        }
    }

    /// Whether the axis shows a change from a reference rather than prices.
    pub const fn is_relative(self) -> bool {
        matches!(self, Self::Percent | Self::IndexedTo100)
    }

    pub const fn is_logarithmic(self) -> bool {
        matches!(self, Self::Logarithmic)
    }

    /// What a shortcut for `target` does: choosing the mode already active goes back to Regular,
    /// as TradingView's Alt+P and Alt+L do.
    pub fn toggled(self, target: Self) -> Self {
        if self == target {
            Self::Regular
        } else {
            target
        }
    }
}

impl Serialize for PriceScaleMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for PriceScaleMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_name(&String::deserialize(deserializer)?))
    }
}

/// The reference a close can serve as: a finite, non-zero price. Anything else (no bar yet, a
/// zero close) leaves the scale without one and the axis shows prices until there is one.
pub fn reference_from(close: f64) -> Option<f64> {
    (close.is_finite() && close != 0.0).then_some(close)
}

/// Whether a logarithmic scale can draw a view whose lowest price is `low`: a logarithm is
/// undefined at zero and below. A view that cannot is drawn linear (and the mode stays chosen,
/// so the log scale returns with the data); a price is never clamped to a tiny positive one.
pub fn can_draw_logarithmic(low: f64) -> bool {
    low.is_finite() && low > 0.0
}

/// What the axis reads at `price`. Regular and Logarithmic read the price itself; Percent and
/// Indexed read the change from `reference`, and nothing without one.
pub fn display(mode: PriceScaleMode, reference: Option<f64>, price: f64) -> Option<f64> {
    let value = match mode {
        PriceScaleMode::Regular | PriceScaleMode::Logarithmic => price,
        PriceScaleMode::Percent => percent(price, reference?),
        PriceScaleMode::IndexedTo100 => 100.0 + percent(price, reference?),
    };
    value.is_finite().then_some(value)
}

/// The price an axis `value` stands for: [`display`]'s inverse.
pub fn from_display(mode: PriceScaleMode, reference: Option<f64>, value: f64) -> Option<f64> {
    let price = match mode {
        PriceScaleMode::Regular | PriceScaleMode::Logarithmic => value,
        PriceScaleMode::Percent => from_percent(value, reference?),
        PriceScaleMode::IndexedTo100 => from_percent(value - 100.0, reference?),
    };
    price.is_finite().then_some(price)
}

/// The change from `reference` in percent. Divided by `|reference|`, so a fall below a negative
/// reference still reads negative.
fn percent(price: f64, reference: f64) -> f64 {
    100.0 * (price - reference) / reference.abs()
}

fn from_percent(percent: f64, reference: f64) -> f64 {
    reference + percent * reference.abs() / 100.0
}

/// An axis value of a relative mode as text. `step` is the distance to the next tick and gives
/// the places a tick needs; without one (a tag) two are written. Percent always carries its sign
/// and never reads `-0%`; thousands are grouped.
pub fn format_relative(mode: PriceScaleMode, value: f64, step: Option<f64>) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    let places = step.map_or(2, places_for_step).min(MAX_PLACES);
    let magnitude = format!("{:.places$}", value.abs());
    let negative = value < 0.0 && magnitude.chars().any(|c| c.is_ascii_digit() && c != '0');
    let sign = if negative { "-" } else { "" };
    match mode {
        PriceScaleMode::Percent => {
            if magnitude.chars().all(|c| c == '0' || c == '.') {
                return "0%".to_owned();
            }
            let sign = if negative { "-" } else { "+" };
            format!("{sign}{}%", group_thousands(&magnitude))
        }
        _ => format!("{sign}{}", group_thousands(&magnitude)),
    }
}

/// An index or a percentage never needs more than this many places.
const MAX_PLACES: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    const RELATIVE: [PriceScaleMode; 2] = [PriceScaleMode::Percent, PriceScaleMode::IndexedTo100];

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9 * left.abs().max(right.abs()).max(1.0)
    }

    #[test]
    fn percent_reads_the_change_from_the_reference() {
        let shown = |price| display(PriceScaleMode::Percent, Some(200.0), price).unwrap();
        assert!(close(shown(200.0), 0.0));
        assert!(close(shown(220.0), 10.0));
        assert!(close(shown(180.0), -10.0));
        assert!(close(shown(250.0), 25.0));
        assert!(close(shown(0.0), -100.0));
    }

    #[test]
    fn indexed_is_percent_plus_a_hundred_and_not_the_same_number() {
        let shown = |price| display(PriceScaleMode::IndexedTo100, Some(200.0), price).unwrap();
        assert!(close(shown(200.0), 100.0));
        assert!(close(shown(180.0), 90.0));
        assert!(close(shown(220.0), 110.0));
        assert!(close(shown(250.0), 125.0));
        // The same price: +10 percent, 110 indexed.
        assert_ne!(
            display(PriceScaleMode::Percent, Some(200.0), 220.0),
            display(PriceScaleMode::IndexedTo100, Some(200.0), 220.0)
        );
    }

    #[test]
    fn a_negative_reference_still_reads_a_rise_as_positive() {
        let shown = display(PriceScaleMode::Percent, Some(-50.0), -25.0).unwrap();
        assert!(
            close(shown, 50.0),
            "-50 to -25 is a rise of half the reference: {shown}"
        );
        let fall = display(PriceScaleMode::Percent, Some(-50.0), -75.0).unwrap();
        assert!(close(fall, -50.0));
    }

    #[test]
    fn the_inverse_returns_the_price_for_every_mode() {
        for mode in PriceScaleMode::ALL {
            for reference in [200.0, 0.0031, -40.0] {
                for price in [-75.0, 0.0, 0.0031, 180.0, 200.0, 12_345.678] {
                    let value = display(mode, Some(reference), price).unwrap();
                    let back = from_display(mode, Some(reference), value).unwrap();
                    assert!(
                        close(back, price),
                        "{mode:?} ref {reference} price {price}: {back}"
                    );
                }
            }
        }
        // And by hand, not through `display`: 110 indexed to a reference of 200 is 220.
        assert!(close(
            from_display(PriceScaleMode::IndexedTo100, Some(200.0), 110.0).unwrap(),
            220.0
        ));
        assert!(close(
            from_display(PriceScaleMode::Percent, Some(200.0), -10.0).unwrap(),
            180.0
        ));
    }

    #[test]
    fn regular_and_logarithmic_read_the_price_whatever_the_reference() {
        for mode in [PriceScaleMode::Regular, PriceScaleMode::Logarithmic] {
            assert_eq!(display(mode, None, -3.5), Some(-3.5));
            assert_eq!(display(mode, Some(10.0), 0.0), Some(0.0));
            assert_eq!(from_display(mode, None, 42.0), Some(42.0));
        }
    }

    #[test]
    fn a_relative_mode_without_a_reference_shows_nothing() {
        for mode in RELATIVE {
            assert_eq!(display(mode, None, 100.0), None);
            assert_eq!(from_display(mode, None, 100.0), None);
        }
    }

    #[test]
    fn no_reference_comes_from_nothing_or_zero_or_a_non_number() {
        assert_eq!(reference_from(200.0), Some(200.0));
        assert_eq!(reference_from(-3.0), Some(-3.0));
        for bad in [0.0, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(reference_from(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_overflowing_value_is_refused_and_never_infinite() {
        assert_eq!(display(PriceScaleMode::Percent, Some(1e-300), 1e300), None);
        assert_eq!(
            display(PriceScaleMode::Percent, Some(200.0), f64::NAN),
            None
        );
        assert_eq!(
            from_display(PriceScaleMode::Percent, Some(1e300), 1e300),
            None
        );
    }

    #[test]
    fn a_logarithm_needs_a_positive_floor() {
        assert!(can_draw_logarithmic(0.0000001));
        for low in [0.0, -1.0, f64::NAN, f64::NEG_INFINITY] {
            assert!(!can_draw_logarithmic(low), "{low}");
        }
    }

    #[test]
    fn percent_labels_carry_their_sign_and_never_read_minus_zero() {
        let tick = |value, step| format_relative(PriceScaleMode::Percent, value, Some(step));
        assert_eq!(tick(10.0, 10.0), "+10%");
        assert_eq!(tick(-20.0, 10.0), "-20%");
        assert_eq!(tick(0.0, 10.0), "0%");
        assert_eq!(tick(-0.0, 10.0), "0%");
        assert_eq!(tick(-0.0000001, 10.0), "0%");
        assert_eq!(tick(2.5, 2.5), "+2.5%");
        assert_eq!(tick(0.25, 0.25), "+0.25%");
        assert_eq!(tick(1_250.0, 250.0), "+1,250%");
        assert_eq!(tag(PriceScaleMode::Percent, 12.3456), "+12.35%");
        assert_eq!(tag(PriceScaleMode::Percent, -0.001), "0%");
    }

    #[test]
    fn index_labels_are_plain_numbers() {
        let tick = |value, step| format_relative(PriceScaleMode::IndexedTo100, value, Some(step));
        assert_eq!(tick(100.0, 10.0), "100");
        assert_eq!(tick(90.0, 10.0), "90");
        assert_eq!(tick(112.5, 2.5), "112.5");
        assert_eq!(tick(12_000.0, 1_000.0), "12,000");
        assert_eq!(tick(-5.0, 5.0), "-5");
        assert_eq!(tag(PriceScaleMode::IndexedTo100, 110.0), "110.00");
    }

    #[test]
    fn a_label_of_something_that_is_not_a_number_is_a_dash() {
        for mode in RELATIVE {
            assert_eq!(format_relative(mode, f64::NAN, None), "—");
            assert_eq!(format_relative(mode, f64::INFINITY, Some(1.0)), "—");
        }
    }

    fn tag(mode: PriceScaleMode, value: f64) -> String {
        format_relative(mode, value, None)
    }

    #[test]
    fn a_shortcut_for_the_active_mode_goes_back_to_regular() {
        let log = PriceScaleMode::Logarithmic;
        assert_eq!(PriceScaleMode::Regular.toggled(log), log);
        assert_eq!(log.toggled(log), PriceScaleMode::Regular);
        assert_eq!(PriceScaleMode::Percent.toggled(log), log);
        assert_eq!(
            log.toggled(PriceScaleMode::Percent),
            PriceScaleMode::Percent
        );
    }

    #[test]
    fn the_legacy_log_flag_maps_to_the_two_modes_it_could_mean() {
        assert_eq!(
            PriceScaleMode::from_log_flag(true),
            PriceScaleMode::Logarithmic
        );
        assert_eq!(
            PriceScaleMode::from_log_flag(false),
            PriceScaleMode::Regular
        );
    }

    #[test]
    fn modes_are_stored_by_name_and_an_unknown_name_is_regular() {
        for mode in PriceScaleMode::ALL {
            let text = serde_json::to_string(&mode).unwrap();
            assert_eq!(serde_json::from_str::<PriceScaleMode>(&text).unwrap(), mode);
        }
        assert_eq!(
            serde_json::to_string(&PriceScaleMode::IndexedTo100).unwrap(),
            "\"indexed_to_100\""
        );
        let future: PriceScaleMode = serde_json::from_str("\"inverted_percent\"").unwrap();
        assert_eq!(future, PriceScaleMode::Regular);
    }

    #[test]
    fn exactly_one_mode_is_logarithmic_and_two_are_relative() {
        let logs = PriceScaleMode::ALL
            .iter()
            .filter(|m| m.is_logarithmic())
            .count();
        let relative = PriceScaleMode::ALL
            .iter()
            .filter(|m| m.is_relative())
            .count();
        assert_eq!((logs, relative), (1, 2));
        assert!(
            PriceScaleMode::ALL
                .iter()
                .all(|m| !(m.is_logarithmic() && m.is_relative()))
        );
    }
}
