//! The least-squares line behind the regression trend, and the bars it is fitted to. Pure: no
//! egui types, so the maths is tested without a window.
//!
//! The line is `y = intercept + slope * x` where `x` is the bar's position in the run, `0` for
//! the first bar and `n - 1` for the last. Using the position rather than the time keeps the fit
//! the one the chart shows when bars are missing (weekends, a halted market). Both ends of the
//! run are included.
//!
//! The band is `sigma`, the standard deviation of the prices around the line, taken over the
//! whole run (divided by `n`, not `n - 1`); the bands sit at a chosen number of sigmas from the
//! line. TradingView does not document its formula, so the choice of divisor is inferred. Pearson's
//! R is the correlation of the price with the bar position.
//!
//! On a logarithmic price scale the fit is made on `ln(price)`, so the line is a constant growth
//! rate and the bands scale with the price.

use super::Kline;
use super::chart_type::PriceSource;

/// The fitted line of a run of bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Regression {
    pub slope: f64,
    pub intercept: f64,
    /// The standard deviation of the prices around the line (in `ln` units when `log`).
    pub sigma: f64,
    /// Pearson's correlation coefficient of the price with the bar position, `0` for a run
    /// whose prices do not vary.
    pub r: f64,
    pub count: usize,
    /// Fitted on `ln(price)`; `value_at` and `band_at` turn the result back into a price.
    pub log: bool,
}

impl Regression {
    /// The base line's price at bar position `x` (which may lie outside the run).
    pub fn value_at(&self, x: f64) -> f64 {
        self.price(self.intercept + self.slope * x)
    }

    /// The price `deviations` sigmas from the base line at bar position `x`.
    pub fn band_at(&self, x: f64, deviations: f64) -> f64 {
        self.price(self.intercept + self.slope * x + deviations * self.sigma)
    }

    fn price(&self, fitted: f64) -> f64 {
        if self.log { fitted.exp() } else { fitted }
    }
}

/// Fits a line through `values` taken at positions `0..n`. `None` for fewer than two values or
/// any value that is not finite. Two passes about the means, so a large price level does not eat
/// the precision of the small differences that matter.
pub fn fit(values: &[f64]) -> Option<Regression> {
    let count = values.len();
    if count < 2 || !values.iter().all(|value| value.is_finite()) {
        return None;
    }
    let n = count as f64;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = values.iter().sum::<f64>() / n;
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for (index, value) in values.iter().enumerate() {
        let dx = index as f64 - mean_x;
        let dy = value - mean_y;
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    let slope = sxy / sxx;
    let intercept = mean_y - slope * mean_x;
    let residuals: f64 = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let miss = value - (intercept + slope * index as f64);
            miss * miss
        })
        .sum();
    let sigma = (residuals / n).sqrt();
    let r = if syy > 0.0 {
        (sxy / (sxx * syy).sqrt()).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    Some(Regression {
        slope,
        intercept,
        sigma,
        r,
        count,
        log: false,
    })
}

/// The bars `from_ms` and `to_ms` fall on, as an inclusive index range into `bars` (sorted by
/// open time), whichever order the times come in. A time before the first bar is the first, one
/// after the last is the last. `None` for no bars.
pub fn bar_span(bars: &[Kline], from_ms: i64, to_ms: i64) -> Option<(usize, usize)> {
    if bars.is_empty() {
        return None;
    }
    let bar_at = |time: i64| {
        bars.partition_point(|bar| bar.open_time <= time)
            .saturating_sub(1)
    };
    let (lo, hi) = (bar_at(from_ms.min(to_ms)), bar_at(from_ms.max(to_ms)));
    Some((lo, hi))
}

/// The regression of `source` over the bars the two times fall on. `None` when they fall on one
/// bar, or when a logarithmic fit is asked for and a price is not positive (a linear fit would
/// then be the honest answer, so the caller asks again with `log` off).
pub fn fit_bars(
    bars: &[Kline],
    from_ms: i64,
    to_ms: i64,
    source: PriceSource,
    log: bool,
) -> Option<Regression> {
    let (lo, hi) = bar_span(bars, from_ms, to_ms)?;
    let run = &bars[lo..=hi];
    let mut values: Vec<f64> = run.iter().map(|bar| source.value(bar)).collect();
    if log {
        if values.iter().any(|value| *value <= 0.0) {
            return None;
        }
        for value in &mut values {
            *value = value.ln();
        }
    }
    fit(&values).map(|fitted| Regression { log, ..fitted })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close_bar(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 59_999,
            open: close - 1.0,
            high: close + 2.0,
            low: close - 3.0,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn a_straight_run_fits_exactly_with_no_spread() {
        let fitted = fit(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        assert!((fitted.slope - 1.0).abs() < 1e-12);
        assert!((fitted.intercept - 1.0).abs() < 1e-12);
        assert!(fitted.sigma < 1e-12);
        assert!((fitted.r - 1.0).abs() < 1e-12);
        assert_eq!(fitted.count, 5);
        assert!((fitted.value_at(6.0) - 7.0).abs() < 1e-12, "it runs on");
    }

    #[test]
    fn a_worked_example_matches_the_hand_calculation() {
        // y = 2, 4, 5, 4, 5: means 2 and 4; Sxy = 6, Sxx = 10, Syy = 6.
        let fitted = fit(&[2.0, 4.0, 5.0, 4.0, 5.0]).unwrap();
        assert!((fitted.slope - 0.6).abs() < 1e-12);
        assert!((fitted.intercept - 2.8).abs() < 1e-12);
        // Residuals -0.8, 0.6, 1.0, -0.6, -0.2: squares sum to 2.4; sigma = sqrt(2.4 / 5).
        assert!((fitted.sigma - 0.48f64.sqrt()).abs() < 1e-12);
        // R = Sxy / sqrt(Sxx * Syy) = 6 / sqrt(60).
        assert!((fitted.r - 6.0 / 60f64.sqrt()).abs() < 1e-12);
        assert!((fitted.band_at(0.0, 2.0) - (2.8 + 2.0 * 0.48f64.sqrt())).abs() < 1e-12);
        assert!((fitted.band_at(0.0, -2.0) - (2.8 - 2.0 * 0.48f64.sqrt())).abs() < 1e-12);
    }

    #[test]
    fn a_falling_run_has_a_negative_slope_and_r() {
        let fitted = fit(&[9.0, 7.0, 5.0, 3.0]).unwrap();
        assert!((fitted.slope + 2.0).abs() < 1e-12);
        assert!((fitted.r + 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_flat_run_has_zero_slope_zero_sigma_and_no_correlation() {
        let fitted = fit(&[5.0, 5.0, 5.0]).unwrap();
        assert_eq!((fitted.slope, fitted.sigma, fitted.r), (0.0, 0.0, 0.0));
        assert_eq!(fitted.intercept, 5.0);
    }

    #[test]
    fn too_few_or_unusable_values_fit_nothing() {
        assert!(fit(&[]).is_none());
        assert!(fit(&[3.0]).is_none());
        assert!(fit(&[1.0, f64::NAN]).is_none());
        assert!(fit(&[1.0, f64::INFINITY]).is_none());
        assert!(fit(&[1.0, 2.0]).is_some());
    }

    #[test]
    fn a_large_price_level_does_not_cost_the_small_differences() {
        let base = 1.0e12;
        let fitted = fit(&[base, base + 1.0, base + 2.0, base + 3.0]).unwrap();
        assert!((fitted.slope - 1.0).abs() < 1e-3);
        assert!(fitted.sigma < 1e-3);
    }

    #[test]
    fn the_span_is_the_bars_the_times_fall_on_in_either_order() {
        let bars: Vec<Kline> = (0..10).map(|i| close_bar(i * 60_000, 100.0)).collect();
        assert_eq!(bar_span(&bars, 65_000, 305_000), Some((1, 5)));
        assert_eq!(bar_span(&bars, 305_000, 65_000), Some((1, 5)));
        // On a bar's open time exactly: that bar.
        assert_eq!(bar_span(&bars, 120_000, 240_000), Some((2, 4)));
        // Outside the data: clamped to the ends.
        assert_eq!(bar_span(&bars, -1_000_000, 99_000_000), Some((0, 9)));
        assert_eq!(bar_span(&[], 0, 1), None);
    }

    #[test]
    fn a_run_is_fitted_to_its_source_and_includes_both_ends() {
        let bars: Vec<Kline> = [10.0, 12.0, 14.0, 16.0]
            .iter()
            .enumerate()
            .map(|(i, close)| close_bar(i as i64 * 60_000, *close))
            .collect();
        let close = fit_bars(&bars, 0, 180_000, PriceSource::Close, false).unwrap();
        assert_eq!(close.count, 4);
        assert!((close.slope - 2.0).abs() < 1e-12);
        assert!((close.intercept - 10.0).abs() < 1e-12);
        // The high is two above the close: the same slope, shifted up.
        let high = fit_bars(&bars, 0, 180_000, PriceSource::High, false).unwrap();
        assert!((high.intercept - 12.0).abs() < 1e-12);
        // Both times on one bar: nothing to fit.
        assert!(fit_bars(&bars, 60_000, 90_000, PriceSource::Close, false).is_none());
    }

    #[test]
    fn a_logarithmic_fit_is_a_constant_growth_rate() {
        let prices = [100.0, 110.0, 121.0, 133.1];
        let bars: Vec<Kline> = prices
            .iter()
            .enumerate()
            .map(|(i, close)| close_bar(i as i64 * 60_000, *close))
            .collect();
        let fitted = fit_bars(&bars, 0, 180_000, PriceSource::Close, true).unwrap();
        assert!(fitted.log);
        assert!((fitted.slope - 1.1f64.ln()).abs() < 1e-9);
        assert!((fitted.value_at(3.0) - 133.1).abs() < 1e-6);
        assert!(fitted.sigma < 1e-9, "it grows at exactly one rate");
        // A price that is not positive cannot be fitted in logs.
        let mut broken = bars;
        broken[1].close = 0.0;
        assert!(fit_bars(&broken, 0, 180_000, PriceSource::Close, true).is_none());
    }
}
