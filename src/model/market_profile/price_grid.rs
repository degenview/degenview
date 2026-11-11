//! The row grid: price rows as integer tick indices.
//!
//! A row is `ticks_per_row` ticks tall and sits on a **global** grid (row `k` is `k × height` up to
//! `(k + 1) × height`), so rows of neighbouring bars and sessions line up and a row's identity is an
//! `i64`, not a float. A price within rounding noise of a row boundary belongs to the row *above* the
//! boundary (`0.1 + 0.2` is row 3 of a 0.1 grid, not row 2).

/// The most rows one bar or profile may span. A range that needs more is refused rather than
/// truncated, so a one-tick grid on a wide session cannot allocate tens of thousands of rows.
pub const MAX_ROWS: usize = 1000;

/// Tolerance, in ticks, under which a price counts as exactly on a tick: absolute plus relative to
/// the magnitude (a price of 1e5 on a 0.01 tick has 1e7 ticks and double precision of ~1e-9 there).
const ABSOLUTE_TOLERANCE: f64 = 1e-9;
const RELATIVE_TOLERANCE: f64 = 1e-12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceGrid {
    tick: f64,
    ticks_per_row: u32,
}

impl PriceGrid {
    /// `None` unless `tick` is a positive finite number and a row is at least one tick.
    pub fn new(tick: f64, ticks_per_row: u32) -> Option<Self> {
        (tick.is_finite() && tick > 0.0 && ticks_per_row >= 1).then_some(Self {
            tick,
            ticks_per_row,
        })
    }

    pub fn tick(&self) -> f64 {
        self.tick
    }

    pub fn ticks_per_row(&self) -> u32 {
        self.ticks_per_row
    }

    /// The height of a row in price.
    pub fn height(&self) -> f64 {
        self.tick * f64::from(self.ticks_per_row)
    }

    /// The tick a price is in (floor), snapping a price within rounding noise of a tick up to it.
    fn tick_index(&self, price: f64) -> Option<i64> {
        if !price.is_finite() {
            return None;
        }
        let quotient = price / self.tick;
        if quotient.abs() > 1e15 {
            return None;
        }
        let nearest = quotient.round();
        let tolerance = ABSOLUTE_TOLERANCE + quotient.abs() * RELATIVE_TOLERANCE;
        let index = if (quotient - nearest).abs() <= tolerance {
            nearest
        } else {
            quotient.floor()
        };
        Some(index as i64)
    }

    /// The row a price is in; `None` for a price that is not a number (or absurdly large).
    pub fn row_of(&self, price: f64) -> Option<i64> {
        self.tick_index(price)
            .map(|tick| tick.div_euclid(i64::from(self.ticks_per_row)))
    }

    /// The price at the bottom of a row.
    pub fn bottom(&self, row: i64) -> f64 {
        (row * i64::from(self.ticks_per_row)) as f64 * self.tick
    }

    /// The price at the top of a row (the bottom of the next).
    pub fn top(&self, row: i64) -> f64 {
        self.bottom(row + 1)
    }

    /// The price in the middle of a row.
    pub fn middle(&self, row: i64) -> f64 {
        (self.bottom(row) + self.top(row)) / 2.0
    }

    /// The rows `low..=high` span, lowest first. `None` for a range that is not a number or needs
    /// more than [`MAX_ROWS`].
    pub fn rows_spanning(&self, low: f64, high: f64) -> Option<(i64, i64)> {
        let (low, high) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        let (first, last) = (self.row_of(low)?, self.row_of(high)?);
        let count = last.checked_sub(first)?.checked_add(1)?;
        (count >= 1 && count as usize <= MAX_ROWS).then_some((first, last))
    }

    /// `volume` spread evenly over the prices `low..=high`: each row gets the share of the range it
    /// overlaps (a range with no height goes into one row). The shares sum to `volume` (they are
    /// normalised by the overlaps actually found, so rounding cannot create or lose volume). Rows
    /// come lowest first; `None` when the range is not a number, needs more than [`MAX_ROWS`] rows,
    /// or `volume` is not a non-negative number.
    pub fn spread(&self, low: f64, high: f64, volume: f64) -> Option<Vec<(i64, f64)>> {
        if !(volume.is_finite() && volume >= 0.0) {
            return None;
        }
        let (low, high) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        let (first, last) = self.rows_spanning(low, high)?;
        if first == last || high - low <= 0.0 {
            return Some(vec![(first, volume)]);
        }
        let overlaps: Vec<(i64, f64)> = (first..=last)
            .map(|row| {
                let overlap = self.top(row).min(high) - self.bottom(row).max(low);
                (row, overlap.max(0.0))
            })
            .filter(|(_, overlap)| *overlap > 0.0)
            .collect();
        let total: f64 = overlaps.iter().map(|(_, overlap)| overlap).sum();
        if total.is_nan() || total <= 0.0 {
            return Some(vec![(first, volume)]);
        }
        Some(
            overlaps
                .into_iter()
                .map(|(row, overlap)| (row, volume * overlap / total))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(tick: f64, per_row: u32) -> PriceGrid {
        PriceGrid::new(tick, per_row).unwrap()
    }

    #[test]
    fn a_grid_needs_a_positive_finite_tick_and_a_whole_row() {
        assert!(PriceGrid::new(0.0, 1).is_none());
        assert!(PriceGrid::new(-0.1, 1).is_none());
        assert!(PriceGrid::new(f64::NAN, 1).is_none());
        assert!(PriceGrid::new(f64::INFINITY, 1).is_none());
        assert!(PriceGrid::new(0.1, 0).is_none());
        assert!(PriceGrid::new(0.1, 1).is_some());
    }

    #[test]
    fn a_price_on_a_boundary_is_in_the_row_above_it_despite_float_noise() {
        let g = grid(0.1, 1);
        // 0.1 + 0.2 is 0.30000000000000004; 3 * 0.1 is 0.30000000000000004 too, 0.3 is 0.3.
        assert_eq!(g.row_of(0.1 + 0.2), Some(3));
        assert_eq!(g.row_of(0.3), Some(3));
        assert_eq!(g.row_of(0.29), Some(2));
        // The classic one that floors to 2: 0.7 / 0.1 = 6.999999999999999.
        assert_eq!(g.row_of(0.7), Some(7));
        assert_eq!(g.row_of(1.1), Some(11));
        assert_eq!(g.row_of(4.35), Some(43));
    }

    #[test]
    fn rows_of_several_ticks_sit_on_a_global_grid() {
        let g = grid(0.5, 4); // 2.0 tall
        assert_eq!(g.row_of(0.0), Some(0));
        assert_eq!(g.row_of(1.99), Some(0));
        assert_eq!(g.row_of(2.0), Some(1));
        assert_eq!(g.row_of(-0.01), Some(-1));
        assert_eq!(g.bottom(1), 2.0);
        assert_eq!(g.top(1), 4.0);
        assert_eq!(g.middle(1), 3.0);
        assert_eq!(g.height(), 2.0);
    }

    #[test]
    fn a_price_that_is_not_a_number_has_no_row() {
        let g = grid(0.01, 1);
        assert_eq!(g.row_of(f64::NAN), None);
        assert_eq!(g.row_of(f64::INFINITY), None);
        assert_eq!(g.row_of(1e300), None);
    }

    #[test]
    fn large_prices_on_small_ticks_stay_on_their_tick() {
        let g = grid(0.01, 1);
        assert_eq!(g.row_of(100_000.01), Some(10_000_001));
        assert_eq!(g.row_of(100_000.0), Some(10_000_000));
    }

    #[test]
    fn a_range_that_needs_too_many_rows_is_refused_not_truncated() {
        let g = grid(0.01, 1);
        assert!(g.rows_spanning(100.0, 101.0).is_some()); // 101 rows
        assert!(g.rows_spanning(100.0, 130.0).is_none()); // 3001 rows
        assert!(g.rows_spanning(f64::NAN, 1.0).is_none());
        assert!(g.spread(100.0, 130.0, 5.0).is_none());
    }

    #[test]
    fn a_spread_keeps_the_volume_and_follows_the_overlap() {
        let g = grid(1.0, 1);
        // 10.5 to 12.5: half a row of 10, a whole 11, half of 12 -> shares 1/4, 1/2, 1/4.
        let rows = g.spread(10.5, 12.5, 8.0).unwrap();
        assert_eq!(
            rows.iter().map(|(r, _)| *r).collect::<Vec<_>>(),
            [10, 11, 12]
        );
        let shares: Vec<f64> = rows.iter().map(|(_, v)| *v).collect();
        assert!((shares[0] - 2.0).abs() < 1e-9);
        assert!((shares[1] - 4.0).abs() < 1e-9);
        assert!((shares[2] - 2.0).abs() < 1e-9);
        assert!((shares.iter().sum::<f64>() - 8.0).abs() < 1e-12);
    }

    #[test]
    fn a_range_with_no_height_goes_in_one_row() {
        let g = grid(1.0, 1);
        assert_eq!(g.spread(10.4, 10.4, 3.0).unwrap(), vec![(10, 3.0)]);
    }

    #[test]
    fn a_range_ending_on_a_boundary_adds_no_empty_row() {
        let g = grid(1.0, 1);
        // 10.0 to 12.0 ends exactly on the 12 boundary: row 12 would hold zero overlap.
        let rows = g.spread(10.0, 12.0, 4.0).unwrap();
        assert_eq!(rows.iter().map(|(r, _)| *r).collect::<Vec<_>>(), [10, 11]);
        assert!((rows.iter().map(|(_, v)| v).sum::<f64>() - 4.0).abs() < 1e-12);
    }

    #[test]
    fn a_reversed_range_is_the_same_range() {
        let g = grid(1.0, 1);
        assert_eq!(g.spread(12.5, 10.5, 8.0), g.spread(10.5, 12.5, 8.0));
    }

    #[test]
    fn a_volume_that_is_not_a_number_is_refused() {
        let g = grid(1.0, 1);
        assert!(g.spread(1.0, 2.0, f64::NAN).is_none());
        assert!(g.spread(1.0, 2.0, -1.0).is_none());
        assert_eq!(
            g.spread(1.0, 2.0, 0.0)
                .unwrap()
                .iter()
                .map(|(_, v)| *v)
                .sum::<f64>(),
            0.0
        );
    }
}
