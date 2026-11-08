//! How an overlay's values become heights. One engine for every overlay, so no painter repeats
//! the arithmetic. Pure.
//!
//! * **Main** scale: the overlay is *rebased* onto the main series. Its first visible value is
//!   the baseline and is drawn at the main series' reference price (the close of the first shown
//!   bar), then scaled by the ratio, `reference * value / baseline`. On the chart's Percent or
//!   Indexed axis that reads as the overlay's own percent change from its first visible value,
//!   which is what TradingView's "Same % scale" shows (both series start at 0% on the first
//!   visible bar). A ratio is multiplicative, so the logarithmic axis works unchanged.
//! * **Own** scale: the group's visible values are fitted to the plot height with a margin. No
//!   axis is drawn; the value is in the tag and the legend.
//!
//! The baseline follows the first *visible* value of the overlay, so it moves with a pan. When an
//! overlay has no value at the first visible bar, its own first visible value is the baseline
//! (a DegenView decision; TradingView's behaviour there was not verified).

use super::price_scale::{PriceScaleMode, display};

/// The first value in `visible` that a ratio can use. `None` when there is none: the overlay has
/// no baseline and is not drawn on the main scale.
pub fn baseline(visible: &[Option<f64>]) -> Option<f64> {
    visible
        .iter()
        .flatten()
        .copied()
        .find(|value| value.is_finite() && *value > 0.0)
}

/// `value` rebased so `baseline` sits at `reference` (the main series' reference price).
pub fn rebase(value: f64, baseline: f64, reference: f64) -> f64 {
    reference * value / baseline
}

/// What the legend and crosshair read for a main-scale overlay: its change in the chart's mode
/// (percent, or indexed), else the real price.
pub fn legend_value(mode: PriceScaleMode, baseline: Option<f64>, value: f64) -> f64 {
    match mode {
        PriceScaleMode::Percent | PriceScaleMode::IndexedTo100 => {
            display(mode, baseline, value).unwrap_or(value)
        }
        PriceScaleMode::Regular | PriceScaleMode::Logarithmic => value,
    }
}

/// The fit of an own-scale group: a price range mapped to the plot's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnFit {
    pub low: f64,
    pub high: f64,
}

/// Share of the plot height left empty above and below an own-scale group.
pub const OWN_MARGIN: f64 = 0.08;

impl OwnFit {
    /// The range of every value of `groups` (each a visible slice), or `None` when there is
    /// none. A flat group (one value, or all equal) gets a small range around it so it draws as a
    /// level line in the middle instead of dividing by zero.
    pub fn of(groups: &[&[Option<f64>]]) -> Option<Self> {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for value in groups.iter().flat_map(|slice| slice.iter().flatten()) {
            if value.is_finite() {
                low = low.min(*value);
                high = high.max(*value);
            }
        }
        if !(low.is_finite() && high.is_finite()) {
            return None;
        }
        if high - low <= f64::EPSILON * high.abs().max(1.0) {
            let pad = (high.abs() * 0.01).max(f64::MIN_POSITIVE);
            return Some(Self {
                low: low - pad,
                high: high + pad,
            });
        }
        Some(Self { low, high })
    }

    /// Where `value` sits in the plot: `0.0` at the bottom edge, `1.0` at the top, after the
    /// margin. Not clamped, so a value outside the fit leaves the plot and is clipped.
    pub fn fraction(self, value: f64) -> f64 {
        let t = (value - self.low) / (self.high - self.low);
        OWN_MARGIN + t * (1.0 - 2.0 * OWN_MARGIN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baseline_is_the_first_usable_visible_value() {
        assert_eq!(baseline(&[None, Some(4.0), Some(5.0)]), Some(4.0));
        assert_eq!(
            baseline(&[None, Some(f64::NAN), Some(0.0), Some(-1.0)]),
            None
        );
        assert_eq!(baseline(&[]), None);
    }

    #[test]
    fn the_baseline_moves_with_the_window() {
        let values = [Some(10.0), Some(20.0), Some(40.0)];
        assert_eq!(baseline(&values[0..]), Some(10.0));
        assert_eq!(baseline(&values[1..]), Some(20.0));
    }

    #[test]
    fn rebasing_puts_the_baseline_at_the_reference_and_keeps_ratios() {
        // BTC at 100_000 against DOGE at 0.2: both start at the main reference, a doubling reads
        // as a doubling.
        let reference = 100_000.0;
        assert_eq!(rebase(0.2, 0.2, reference), reference);
        assert_eq!(rebase(0.4, 0.2, reference), 200_000.0);
        assert_eq!(rebase(0.1, 0.2, reference), 50_000.0);
    }

    #[test]
    fn a_rebased_overlay_reads_as_percent_on_the_percent_axis() {
        let reference = 4_000.0;
        let baseline = 0.2;
        let rebased = rebase(0.25, baseline, reference);
        // The axis computes change from the reference; it must equal the overlay's own change.
        let axis = display(PriceScaleMode::Percent, Some(reference), rebased).unwrap();
        assert!((axis - 25.0).abs() < 1e-9, "{axis}");
        let indexed = display(PriceScaleMode::IndexedTo100, Some(reference), rebased).unwrap();
        assert!((indexed - 125.0).abs() < 1e-9);
    }

    #[test]
    fn the_legend_reads_the_overlays_own_change_in_relative_modes() {
        let value = legend_value(PriceScaleMode::Percent, Some(50.0), 60.0);
        assert!((value - 20.0).abs() < 1e-9);
        assert!(
            (legend_value(PriceScaleMode::IndexedTo100, Some(50.0), 60.0) - 120.0).abs() < 1e-9
        );
        assert_eq!(
            legend_value(PriceScaleMode::Regular, Some(50.0), 60.0),
            60.0
        );
        assert_eq!(
            legend_value(PriceScaleMode::Logarithmic, Some(50.0), 60.0),
            60.0
        );
        // No baseline: the real price rather than nothing.
        assert_eq!(legend_value(PriceScaleMode::Percent, None, 60.0), 60.0);
    }

    #[test]
    fn an_own_fit_spans_the_group_and_leaves_a_margin() {
        let a = [Some(10.0), None, Some(30.0)];
        let b = [Some(20.0), Some(50.0)];
        let fit = OwnFit::of(&[&a, &b]).unwrap();
        assert_eq!((fit.low, fit.high), (10.0, 50.0));
        assert!((fit.fraction(10.0) - OWN_MARGIN).abs() < 1e-12);
        assert!((fit.fraction(50.0) - (1.0 - OWN_MARGIN)).abs() < 1e-12);
        assert!((fit.fraction(30.0) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn a_flat_or_empty_group_does_not_divide_by_zero() {
        let flat = [Some(7.0), Some(7.0)];
        let fit = OwnFit::of(&[&flat]).unwrap();
        assert!(fit.high > fit.low);
        assert!((fit.fraction(7.0) - 0.5).abs() < 1e-9);
        assert_eq!(OwnFit::of(&[&[None, None]]), None);
        assert_eq!(OwnFit::of(&[]), None);
        assert_eq!(OwnFit::of(&[&[Some(f64::NAN)]]), None);
    }

    #[test]
    fn magnitudes_far_apart_fit_the_same_height() {
        let btc = [Some(90_000.0), Some(110_000.0)];
        let doge = [Some(0.18), Some(0.22)];
        let (a, b) = (OwnFit::of(&[&btc]).unwrap(), OwnFit::of(&[&doge]).unwrap());
        assert!((a.fraction(110_000.0) - b.fraction(0.22)).abs() < 1e-9);
    }
}
