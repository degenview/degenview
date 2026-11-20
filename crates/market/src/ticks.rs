//! Round prices for a price scale's labels and grid lines.

/// The prices a scale labels, which the horizontal grid is drawn at too: one list, so a grid line
/// can never sit away from its label.
///
/// `minimum..=maximum` is the scale's range; `max_count` is how many fit at the label spacing.
/// A linear scale gets multiples of a "nice" step (1, 2, 2.5 or 5 × a power of ten). A
/// logarithmic one gets 1/2/5 × a power of ten, thinned until they fit, or the same linear
/// multiples while the range is too narrow for decades to mean anything.
pub fn price_ticks(minimum: f64, maximum: f64, logarithmic: bool, max_count: usize) -> Vec<f64> {
    if !(minimum.is_finite() && maximum.is_finite()) || minimum >= maximum || max_count == 0 {
        return Vec::new();
    }
    let max_count = max_count.min(MAX_TICKS);
    if logarithmic && minimum > 0.0 && maximum / minimum >= 3.0 {
        return logarithmic_ticks(minimum, maximum, max_count);
    }
    let step = nice_step((maximum - minimum) / max_count as f64);
    if !(step.is_finite() && step > 0.0) {
        return Vec::new();
    }
    let ticks = multiples(minimum, maximum, step);
    // A scale that has room for two labels never gets one: halve the step once.
    if ticks.len() < 2 && max_count >= 2 {
        let finer = multiples(minimum, maximum, step / 2.0);
        if finer.len() > ticks.len() {
            return finer;
        }
    }
    ticks
}

/// The multiples of `step` inside `minimum..=maximum`.
fn multiples(minimum: f64, maximum: f64, step: f64) -> Vec<f64> {
    let first = (minimum / step).ceil();
    let last = (maximum / step).floor();
    if !(first.is_finite() && last.is_finite()) || last - first > MAX_TICKS as f64 {
        return Vec::new();
    }
    (first as i64..=last as i64)
        .map(|index| index as f64 * step)
        .collect()
}

const MAX_TICKS: usize = 40;

/// The smallest 1, 2, 2.5 or 5 × 10ⁿ that is at least `raw`.
fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return f64::NAN;
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .map(|mantissa| mantissa * magnitude)
        .find(|step| *step >= raw * (1.0 - 1e-9))
        .unwrap_or(10.0 * magnitude)
}

fn logarithmic_ticks(minimum: f64, maximum: f64, max_count: usize) -> Vec<f64> {
    let (low_decade, high_decade) = (
        minimum.log10().floor() as i32,
        maximum.log10().ceil() as i32,
    );
    // The decades alone first, then ever more of them in each decade, while they still fit.
    let ladders: [&[f64]; 4] = [
        &[1.0],
        &[1.0, 3.0],
        &[1.0, 2.0, 5.0],
        &[1.0, 2.0, 3.0, 5.0, 7.0],
    ];
    let mut best = Vec::new();
    for mantissas in ladders {
        let ticks: Vec<f64> = (low_decade..=high_decade)
            .flat_map(|decade| mantissas.iter().map(move |m| m * 10f64.powi(decade)))
            .filter(|value| *value >= minimum && *value <= maximum)
            .collect();
        if ticks.len() > max_count {
            break;
        }
        best = ticks;
    }
    if best.is_empty() {
        // Fewer than one decade tick fits (a wide range, a tiny pane): thin the decades.
        let decades: Vec<f64> = (low_decade..=high_decade)
            .map(|decade| 10f64.powi(decade))
            .filter(|value| *value >= minimum && *value <= maximum)
            .collect();
        let stride = decades.len().div_ceil(max_count.max(1)).max(1);
        best = decades.into_iter().step_by(stride).collect();
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    #[test]
    fn linear_ticks_are_round_multiples_inside_the_range() {
        let ticks = price_ticks(98.0, 207.0, false, 6);
        assert!(!ticks.is_empty() && ticks.len() <= 6, "{ticks:?}");
        assert!(ticks.iter().all(|tick| (98.0..=207.0).contains(tick)));
        let step = ticks[1] - ticks[0];
        assert!(
            close(step, 20.0) || close(step, 25.0) || close(step, 50.0),
            "{step}"
        );
        assert!(ticks.windows(2).all(|pair| close(pair[1] - pair[0], step)));
        assert!(
            ticks
                .iter()
                .all(|tick| close(tick / step, (tick / step).round()))
        );
    }

    #[test]
    fn ticks_cope_with_tiny_and_huge_prices() {
        let tiny = price_ticks(0.000_001_2, 0.000_001_9, false, 5);
        assert!(!tiny.is_empty());
        assert!(
            tiny.iter()
                .all(|tick| *tick >= 0.000_001_2 && *tick <= 0.000_001_9)
        );
        let huge = price_ticks(1.0e9, 3.5e9, false, 5);
        assert!(!huge.is_empty());
        assert!(huge.iter().all(|tick| tick.is_finite()));
    }

    #[test]
    fn ticks_cross_zero_for_negative_values() {
        let ticks = price_ticks(-30.0, 40.0, false, 7);
        assert!(ticks.contains(&0.0), "{ticks:?}");
        assert!(ticks.iter().any(|tick| *tick < 0.0));
    }

    #[test]
    fn logarithmic_ticks_are_decades_and_thin_to_fit() {
        let roomy = price_ticks(1.0, 1000.0, true, 12);
        assert!(roomy.contains(&1.0) && roomy.contains(&10.0) && roomy.contains(&100.0));
        assert!(roomy.contains(&1000.0));
        assert!(roomy.len() > 4, "{roomy:?}");
        let tight = price_ticks(1.0, 1000.0, true, 4);
        assert_eq!(tight, vec![1.0, 10.0, 100.0, 1000.0]);
        let tighter = price_ticks(1e-3, 1e9, true, 4);
        assert!(!tighter.is_empty() && tighter.len() <= 4, "{tighter:?}");
    }

    #[test]
    fn a_narrow_logarithmic_range_falls_back_to_round_prices() {
        let ticks = price_ticks(100.0, 140.0, true, 6);
        assert!(ticks.len() >= 2);
        assert!(ticks.iter().all(|tick| (100.0..=140.0).contains(tick)));
    }

    #[test]
    fn ticks_are_empty_for_a_degenerate_scale() {
        assert!(price_ticks(5.0, 5.0, false, 6).is_empty());
        assert!(price_ticks(f64::NAN, 5.0, false, 6).is_empty());
        assert!(price_ticks(1.0, 5.0, false, 0).is_empty());
        assert!(price_ticks(9.0, 1.0, false, 4).is_empty());
    }
}
