//! The point of control and value area, from the volume in each row or from TPO counts.
//!
//! Rows are lowest first. Both functions answer the row of the point of control and the inclusive
//! `(bottom, top)` rows of the value area, or `None` when there is nothing to measure.
//!
//! - **Volume** is the classical market-profile method already used by the volume-profile drawing and
//!   by Pine's `request.footprint()` ([`crate::model::volume_profile::point_of_control_and_value_area`]):
//!   grow by the heavier *pair* of rows beside the area. TradingView documents the footprint's value
//!   area only as "similar to" its Volume Profile indicators, so this is **inferred**.
//! - **TPO** follows TradingView's documented steps for the TPO value area: start at the point of
//!   control, compare the *single* row above the area with the single row below, add the one with
//!   more blocks (equal: the one closer to the point of control, still equal: the higher), repeat
//!   until the area holds the target share of all blocks. The step that reaches the target is taken
//!   whole (the area may hold more than the target); TradingView does not say, **inferred**.
//!
//! Two equal point-of-control rows are told apart by which is nearer the middle of the whole range,
//! then the lower row: TradingView does not document a tie rule for either, so this is **inferred**
//! and the same as the volume profile's.

use crate::model::volume_profile;

/// Volume: see the module docs. `totals` and `middles` are per row, equally long.
pub fn volume(
    totals: &[f64],
    middles: &[f64],
    range_middle: f64,
    fraction: f64,
) -> Option<(usize, (usize, usize))> {
    if totals.is_empty() || totals.len() != middles.len() {
        return None;
    }
    let total: f64 = totals.iter().sum();
    if !(total.is_finite() && total > 0.0) || totals.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return None;
    }
    Some(volume_profile::point_of_control_and_value_area(
        totals,
        middles,
        range_middle,
        fraction,
    ))
}

/// TPO: see the module docs. `counts` are the blocks in each row.
pub fn tpo(
    counts: &[u32],
    middles: &[f64],
    range_middle: f64,
    fraction: f64,
) -> Option<(usize, (usize, usize))> {
    if counts.is_empty() || counts.len() != middles.len() {
        return None;
    }
    let total: u64 = counts.iter().map(|c| u64::from(*c)).sum();
    if total == 0 {
        return None;
    }
    let poc = (0..counts.len())
        .max_by(|a, b| {
            counts[*a]
                .cmp(&counts[*b])
                .then_with(|| {
                    (middles[*b] - range_middle)
                        .abs()
                        .total_cmp(&(middles[*a] - range_middle).abs())
                })
                .then_with(|| b.cmp(a))
        })
        .unwrap_or(0);
    let target = total as f64 * fraction.clamp(0.0, 1.0);
    let (mut bottom, mut top) = (poc, poc);
    let mut held = f64::from(counts[poc]);
    while held < target && (bottom > 0 || top + 1 < counts.len()) {
        let above = (top + 1 < counts.len()).then(|| counts[top + 1]);
        let below = (bottom > 0).then(|| counts[bottom - 1]);
        let take_above = match (above, below) {
            (Some(a), Some(b)) => match a.cmp(&b) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                // Equal: the row closer to the point of control, then the higher.
                std::cmp::Ordering::Equal => {
                    let up = top + 1 - poc;
                    let down = poc - (bottom - 1);
                    up <= down
                }
            },
            (Some(_), None) => true,
            _ => false,
        };
        if take_above {
            top += 1;
            held += f64::from(counts[top]);
        } else {
            bottom -= 1;
            held += f64::from(counts[bottom]);
        }
    }
    Some((poc, (bottom, top)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn middles(count: usize) -> Vec<f64> {
        (0..count).map(|i| i as f64 + 0.5).collect()
    }

    // MARK: TPO

    #[test]
    fn the_tpo_poc_is_the_row_with_most_blocks() {
        let counts = [1, 3, 7, 4, 2];
        let (poc, _) = tpo(&counts, &middles(5), 2.5, 0.7).unwrap();
        assert_eq!(poc, 2);
    }

    #[test]
    fn the_tpo_value_area_adds_the_larger_single_neighbour_until_the_target() {
        // total 20, target 14. POC row 3 (6). above 4 (5) vs below 2 (3): add above -> 11.
        // then above 5 (2) vs below 2 (3): add below -> 14, reached.
        let counts = [1, 2, 3, 6, 5, 2, 1];
        let (poc, area) = tpo(&counts, &middles(7), 3.5, 0.7).unwrap();
        assert_eq!(poc, 3);
        assert_eq!(area, (2, 4));
        let held: u32 = counts[area.0..=area.1].iter().sum();
        assert_eq!(held, 14);
    }

    #[test]
    fn equal_neighbours_take_the_closer_row_then_the_higher() {
        // POC at 2 (5). Neighbours 1 and 3 both 2: equally close -> the higher (row 3).
        let counts = [0, 2, 5, 2, 0];
        let (_, area) = tpo(&counts, &middles(5), 2.5, 0.6).unwrap();
        // total 9, target 5.4: POC 5 is not enough; add the higher row 3 -> 7.
        assert_eq!(area, (2, 3));
        // A tie of 1 and 1 beside the POC goes to the higher row; the next step compares row 4 (3)
        // with row 1 (1) and takes the larger.
        let counts = [3, 1, 5, 1, 3];
        let (_, area) = tpo(&counts, &middles(5), 2.5, 1.0).unwrap();
        assert_eq!(area, (0, 4));
        let (_, area) = tpo(&counts, &middles(5), 2.5, 0.5).unwrap();
        // total 13, target 6.5: 5 -> row 3 (tie, higher) = 6 -> row 4 (3 beats 1) = 9.
        assert_eq!(area, (2, 4));
    }

    #[test]
    fn a_tie_on_distance_prefers_the_row_nearer_the_poc() {
        // After the area is (2, 3), the next candidates are row 4 (above, distance 2) and row 1
        // (below, distance 1). Equal counts of 2: the lower row is closer to the POC.
        let counts = [0, 2, 5, 2, 2];
        let (_, area) = tpo(&counts, &middles(5), 2.5, 0.9).unwrap();
        // total 11, target 9.9: 5 -> row 3 (higher on the first tie, 2 vs 2 at distance 1) = 7
        // -> now above row 4 (2) vs below row 1 (2): row 1 is closer -> 9 -> row 4 -> 11.
        assert_eq!(area, (1, 4));
        let (_, area) = tpo(&counts, &middles(5), 2.5, 0.75).unwrap();
        // target 8.25: 5 -> 7 -> row 1 (closer) -> 9, reached before row 4.
        assert_eq!(area, (1, 3));
    }

    #[test]
    fn the_tpo_poc_tie_goes_nearest_the_middle_then_lower() {
        let counts = [4, 1, 4, 1, 4];
        // Range middle 2.5 is the middle of row 2.
        let (poc, _) = tpo(&counts, &middles(5), 2.5, 0.1).unwrap();
        assert_eq!(poc, 2);
        // Middle at the bottom: rows 0 and 2 and 4 -> nearest is 0.
        let (poc, _) = tpo(&counts, &middles(5), 0.0, 0.1).unwrap();
        assert_eq!(poc, 0);
        // Two rows equally near the middle: the lower.
        let counts = [4, 4];
        let (poc, _) = tpo(&counts, &middles(2), 1.0, 0.1).unwrap();
        assert_eq!(poc, 0);
    }

    #[test]
    fn a_full_target_covers_every_row_and_empty_input_has_no_area() {
        let counts = [1, 2, 3];
        assert_eq!(tpo(&counts, &middles(3), 1.5, 1.0).unwrap().1, (0, 2));
        assert!(tpo(&[], &[], 0.0, 0.7).is_none());
        assert!(tpo(&[0, 0], &middles(2), 1.0, 0.7).is_none());
        assert!(tpo(&[1, 2], &middles(3), 1.0, 0.7).is_none());
    }

    #[test]
    fn a_zero_fraction_is_the_poc_alone() {
        let (poc, area) = tpo(&[1, 9, 1], &middles(3), 1.5, 0.0).unwrap();
        assert_eq!((poc, area), (1, (1, 1)));
    }

    // MARK: Volume

    #[test]
    fn volume_matches_the_profile_function_it_delegates_to() {
        let totals = [1.0, 3.0, 9.0, 4.0, 2.0];
        let m = middles(5);
        let direct = volume_profile::point_of_control_and_value_area(&totals, &m, 2.5, 0.7);
        assert_eq!(volume(&totals, &m, 2.5, 0.7), Some(direct));
        assert_eq!(direct.0, 2);
    }

    #[test]
    fn volume_refuses_empty_negative_and_non_finite_rows() {
        let m = middles(2);
        assert!(volume(&[], &[], 0.0, 0.7).is_none());
        assert!(volume(&[0.0, 0.0], &m, 1.0, 0.7).is_none());
        assert!(volume(&[1.0, f64::NAN], &m, 1.0, 0.7).is_none());
        assert!(volume(&[1.0, -1.0], &m, 1.0, 0.7).is_none());
        assert!(volume(&[1.0], &m, 1.0, 0.7).is_none());
    }
}
