//! The volume footprint of one chart bar, built from the finer candles inside it: what
//! `request.footprint()` hands a script. Pure: no egui, nothing read but the candles it is given.
//! See `docs/pine-footprint.md` for what is documented by TradingView and what is inferred.
//!
//! **What the data can and cannot say.** TradingView builds a footprint from tick, 1-second,
//! 1-minute or 60-minute data, by plan and age of the bar. This build has candles, 1 minute at best,
//! so each candle is one intrabar: its volume is **buy** when it closed above its open and **sell**
//! when it closed below (TradingView's own rule for an intrabar), and is spread over the prices it
//! traded between. The result is a coarser footprint than TradingView's, not the order flow of the
//! exchange.

use crate::model::Kline;
use crate::model::volume_profile::point_of_control_and_value_area;

/// The most rows one footprint may have. A bar whose range holds more (a wide bar and a tiny row
/// height) has no footprint, as a bar with no data has none: a script that asks for one row per tick
/// on a volatile market would otherwise build tens of thousands of rows for every bar. **Inferred**:
/// TradingView does not document a limit.
pub const MAX_ROWS: usize = 1000;

/// Slack when a price is turned into a row number, so a price on a row boundary lands in the row that
/// starts there whatever the rounding of the division.
const BOUNDARY_SLACK: f64 = 1e-7;

/// One price row: the prices `down..up` and the volume traded in them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeRow {
    pub down: f64,
    pub up: f64,
    pub buy: f64,
    pub sell: f64,
}

impl VolumeRow {
    pub fn total(&self) -> f64 {
        self.buy + self.sell
    }

    pub fn delta(&self) -> f64 {
        self.buy - self.sell
    }

    fn middle(&self) -> f64 {
        (self.down + self.up) / 2.0
    }
}

/// The rows of one bar from the lowest price up, with the rows the point of control and the value
/// area are.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub rows: Vec<VolumeRow>,
    /// Index of the point of control.
    pub poc: usize,
    /// The lowest and highest rows of the value area (inclusive); `low <= poc <= high`.
    pub value_area: (usize, usize),
    /// The rows' height, and the number of the lowest row on the grid (its bottom is
    /// `first_row * height`).
    height: f64,
    first_row: f64,
    imbalance_percent: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Buy,
    Sell,
}

impl Footprint {
    pub fn buy_volume(&self) -> f64 {
        self.rows.iter().map(|row| row.buy).sum()
    }

    pub fn sell_volume(&self) -> f64 {
        self.rows.iter().map(|row| row.sell).sum()
    }

    pub fn total_volume(&self) -> f64 {
        self.buy_volume() + self.sell_volume()
    }

    pub fn delta(&self) -> f64 {
        self.buy_volume() - self.sell_volume()
    }

    /// The row holding `price`: a row owns the prices from its bottom up to, not including, its top,
    /// except the highest row, which owns its top too (**inferred**: the bar's high may sit on it).
    pub fn row_at_price(&self, price: f64) -> Option<usize> {
        if !price.is_finite() {
            return None;
        }
        let scaled = price / self.height;
        let index = (scaled + BOUNDARY_SLACK).floor() - self.first_row;
        let count = self.rows.len() as f64;
        if index >= 0.0 && index < count {
            Some(index as usize)
        } else if (scaled - (self.first_row + count)).abs() <= BOUNDARY_SLACK {
            Some(self.rows.len() - 1)
        } else {
            None
        }
    }

    /// Whether row `index` has bought more than the row below sold, by the imbalance percentage
    /// (TradingView: a buy imbalance compares a row's buy volume with the sell volume of the row
    /// below). The lowest row has no row below, so it has none; a row below that sold nothing is
    /// out-bought by any buying (**inferred**).
    pub fn has_buy_imbalance(&self, index: usize) -> bool {
        let Some(below) = index.checked_sub(1).and_then(|i| self.rows.get(i)) else {
            return false;
        };
        let row = &self.rows[index];
        row.buy > 0.0 && row.buy >= self.imbalance_percent / 100.0 * below.sell
    }

    /// The mirror: a row's sell volume against the buy volume of the row above. The highest row has
    /// none.
    pub fn has_sell_imbalance(&self, index: usize) -> bool {
        let Some(above) = self.rows.get(index + 1) else {
            return false;
        };
        let row = &self.rows[index];
        row.sell > 0.0 && row.sell >= self.imbalance_percent / 100.0 * above.buy
    }
}

/// The footprint of the bar whose intrabars are `intrabars` (ascending, all inside the bar).
///
/// - Rows are `ticks_per_row * mintick` tall and sit on a **global grid** (row `k` spans
///   `k * height` to `(k + 1) * height`), so the rows of neighbouring bars line up; from the bar's
///   lowest row to its highest, empty rows between included. **Inferred**: TradingView does not say
///   where its rows start.
/// - An intrabar's volume is spread evenly over its low-to-high, each row getting the share of the
///   range it overlaps; a candle with no range puts it all in the row its price is in.
/// - An intrabar is **buy** volume when it closed above its open and **sell** when below. One that
///   closed on its open (a doji) goes by how it closed against the previous intrabar's close
///   (`previous` for the first one), and when that is level too keeps the previous class; with
///   nothing to go by it is buy. This is the rule of TradingView's help article.
/// - The point of control and value area are the volume profile's (`volume_profile`): the row of most
///   volume, then the value area grown from it by the heavier pair of rows beside it. TradingView says
///   only that its algorithm is "similar to" the volume profile's.
///
/// `None` when there is nothing to show: no volume, a price or size that is not a number, or more
/// than [`MAX_ROWS`] rows.
pub fn build(
    intrabars: &[Kline],
    previous: Option<&Kline>,
    mintick: f64,
    ticks_per_row: i64,
    value_area_percent: f64,
    imbalance_percent: f64,
) -> Option<Footprint> {
    let height = mintick * ticks_per_row as f64;
    if !(height.is_finite() && height > 0.0) {
        return None;
    }
    let usable = |bar: &Kline| {
        bar.high.is_finite()
            && bar.low.is_finite()
            && bar.volume.is_finite()
            && bar.volume > 0.0
            && bar.open.is_finite()
            && bar.close.is_finite()
    };
    let row_of = |price: f64| (price / height + BOUNDARY_SLACK).floor();
    // The rows each intrabar covers, and the rows of the whole bar.
    let spans: Vec<Option<(f64, f64)>> = intrabars
        .iter()
        .map(|bar| {
            usable(bar).then(|| {
                let (bottom, top) = (bar.low.min(bar.high), bar.high.max(bar.low));
                let first = row_of(bottom);
                let last = ((top / height - BOUNDARY_SLACK).ceil() - 1.0).max(first);
                (first, last)
            })
        })
        .collect();
    let lowest = spans.iter().flatten().map(|span| span.0).reduce(f64::min)?;
    let highest = spans.iter().flatten().map(|span| span.1).reduce(f64::max)?;
    if !(lowest.is_finite() && highest.is_finite()) || highest - lowest >= MAX_ROWS as f64 {
        return None;
    }
    let count = (highest - lowest) as usize + 1;
    let mut rows: Vec<VolumeRow> = (0..count)
        .map(|index| {
            let k = lowest + index as f64;
            VolumeRow {
                down: k * height,
                up: (k + 1.0) * height,
                buy: 0.0,
                sell: 0.0,
            }
        })
        .collect();

    let mut last_close = previous.map(|bar| bar.close);
    let mut last_side = Side::Buy;
    for (bar, span) in intrabars.iter().zip(&spans) {
        let side = if bar.close > bar.open {
            Side::Buy
        } else if bar.close < bar.open {
            Side::Sell
        } else {
            match last_close {
                Some(close) if bar.close > close => Side::Buy,
                Some(close) if bar.close < close => Side::Sell,
                _ => last_side,
            }
        };
        if bar.close.is_finite() {
            last_close = Some(bar.close);
        }
        last_side = side;
        let Some((first, last)) = *span else {
            continue;
        };
        let put = |rows: &mut [VolumeRow], row: f64, volume: f64| {
            let row = &mut rows[(row - lowest) as usize];
            match side {
                Side::Buy => row.buy += volume,
                Side::Sell => row.sell += volume,
            }
        };
        let (bottom, top) = (
            bar.low.min(bar.high) / height,
            bar.high.max(bar.low) / height,
        );
        if last <= first || top - bottom <= BOUNDARY_SLACK {
            put(&mut rows, first, bar.volume);
            continue;
        }
        // The share of the range inside each row, normalised so none of the volume is lost.
        let overlaps: Vec<f64> = (first as i64..=last as i64)
            .map(|k| ((k as f64 + 1.0).min(top) - (k as f64).max(bottom)).max(0.0))
            .collect();
        let covered: f64 = overlaps.iter().sum();
        if covered <= 0.0 {
            put(&mut rows, first, bar.volume);
            continue;
        }
        for (offset, overlap) in overlaps.iter().enumerate() {
            put(
                &mut rows,
                first + offset as f64,
                bar.volume * overlap / covered,
            );
        }
    }

    let totals: Vec<f64> = rows.iter().map(VolumeRow::total).collect();
    let total: f64 = totals.iter().sum();
    if !(total.is_finite() && total > 0.0) {
        return None;
    }
    let middles: Vec<f64> = rows.iter().map(VolumeRow::middle).collect();
    let range_middle = (rows[0].down + rows[count - 1].up) / 2.0;
    let (poc, value_area) = point_of_control_and_value_area(
        &totals,
        &middles,
        range_middle,
        value_area_percent / 100.0,
    );
    Some(Footprint {
        rows,
        poc,
        value_area,
        height,
        first_row: lowest,
        imbalance_percent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(open: f64, high: f64, low: f64, close: f64, volume: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 59_999,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn an_intrabar_is_buy_volume_when_it_closed_up_and_sell_when_down() {
        let footprint = build(
            &[
                candle(10.0, 11.0, 10.0, 11.0, 6.0),
                candle(11.0, 11.0, 10.0, 10.0, 4.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(close(footprint.buy_volume(), 6.0));
        assert!(close(footprint.sell_volume(), 4.0));
        assert!(close(footprint.delta(), 2.0));
        assert!(close(footprint.total_volume(), 10.0));
    }

    #[test]
    fn a_doji_goes_by_the_previous_close_then_keeps_the_previous_class() {
        // Closes above the previous intrabar's close: buy.
        let up = build(
            &[
                candle(10.0, 10.0, 8.0, 8.0, 1.0),
                candle(9.0, 9.5, 9.0, 9.0, 2.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(close(up.buy_volume(), 2.0));
        // Below it: sell.
        let down = build(
            &[
                candle(10.0, 12.0, 10.0, 12.0, 1.0),
                candle(11.0, 11.5, 11.0, 11.0, 2.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(close(down.sell_volume(), 2.0));
        // Level with it: the class of the intrabar before.
        let level = build(
            &[
                candle(10.0, 10.0, 8.0, 8.0, 1.0),
                candle(8.0, 8.5, 8.0, 8.0, 2.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(close(level.sell_volume(), 3.0));
        // The first intrabar compares with the bar before the footprint's.
        let before = candle(9.0, 9.0, 9.0, 9.0, 1.0);
        let first = build(
            &[candle(8.0, 8.0, 7.5, 8.0, 5.0)],
            Some(&before),
            0.5,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(close(first.sell_volume(), 5.0));
    }

    #[test]
    fn rows_are_the_height_of_the_ticks_on_a_grid_that_starts_at_zero() {
        let footprint = build(
            &[candle(100.2, 100.9, 100.1, 100.8, 3.0)],
            None,
            0.25,
            2,
            70.0,
            300.0,
        )
        .unwrap();
        // 0.5-high rows: 100.0-100.5 and 100.5-101.0.
        assert_eq!(footprint.rows.len(), 2);
        assert!(close(footprint.rows[0].down, 100.0));
        assert!(close(footprint.rows[0].up, 100.5));
        assert!(close(footprint.rows[1].down, 100.5));
        assert!(close(footprint.rows[1].up, 101.0));
    }

    #[test]
    fn an_intrabars_volume_is_shared_by_overlap_and_none_is_lost() {
        // Half the range is in each row.
        let footprint = build(
            &[candle(100.2, 100.8, 100.2, 100.8, 10.0)],
            None,
            0.5,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(close(footprint.rows[0].buy, 5.0));
        assert!(close(footprint.rows[1].buy, 5.0));
        // Two thirds of 100.1-100.7 is below 100.5.
        let uneven = build(
            &[candle(100.1, 100.7, 100.1, 100.7, 10.0)],
            None,
            0.5,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(close(uneven.rows[0].buy, 20.0 / 3.0));
        assert!(close(uneven.rows[1].buy, 10.0 / 3.0));
        assert!(close(uneven.total_volume(), 10.0));
    }

    #[test]
    fn a_range_ending_on_a_boundary_does_not_open_an_empty_row() {
        let footprint = build(
            &[candle(100.0, 101.0, 100.0, 101.0, 4.0)],
            None,
            0.5,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert_eq!(footprint.rows.len(), 2);
        assert!(close(footprint.rows[0].total(), 2.0));
        assert!(close(footprint.rows[1].total(), 2.0));
    }

    #[test]
    fn a_candle_with_no_range_goes_into_the_row_its_price_is_in() {
        let footprint = build(
            &[candle(100.7, 100.7, 100.7, 100.7, 5.0)],
            None,
            0.5,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert_eq!(footprint.rows.len(), 1);
        assert!(close(footprint.rows[0].down, 100.5));
        assert!(close(footprint.rows[0].total(), 5.0));
    }

    #[test]
    fn empty_rows_between_traded_ones_are_kept() {
        let footprint = build(
            &[
                candle(10.0, 10.4, 10.0, 10.4, 1.0),
                candle(13.0, 13.4, 13.0, 13.4, 1.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert_eq!(footprint.rows.len(), 4);
        assert!(close(footprint.rows[1].total(), 0.0));
        assert!(close(footprint.rows[2].total(), 0.0));
    }

    #[test]
    fn the_point_of_control_is_the_row_of_most_volume_and_a_tie_goes_to_the_middle() {
        let footprint = build(
            &[
                candle(10.0, 10.5, 10.0, 10.5, 1.0),
                candle(11.0, 11.5, 11.0, 11.5, 5.0),
                candle(12.0, 12.5, 12.0, 12.5, 2.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert_eq!(footprint.poc, 1);
        let tied = build(
            &[
                candle(10.0, 10.5, 10.0, 10.5, 3.0),
                candle(11.0, 11.5, 11.0, 11.5, 3.0),
                candle(12.0, 12.5, 12.0, 12.5, 3.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert_eq!(tied.poc, 1);
    }

    #[test]
    fn the_value_area_grows_from_the_poc_by_the_heavier_pair_of_rows() {
        // Rows 0..=5 hold 1, 2, 10, 4, 3, 1 (total 21); 70 % is 14.7.
        let volumes = [1.0, 2.0, 10.0, 4.0, 3.0, 1.0];
        let candles: Vec<Kline> = volumes
            .iter()
            .enumerate()
            .map(|(index, volume)| {
                let price = 10.0 + index as f64;
                candle(price, price + 0.5, price, price + 0.5, *volume)
            })
            .collect();
        let footprint = build(&candles, None, 1.0, 1, 70.0, 300.0).unwrap();
        assert_eq!(footprint.poc, 2);
        // Above (4 + 3 = 7) beats below (2 + 1 = 3): rows 2..=4 hold 17 >= 14.7.
        assert_eq!(footprint.value_area, (2, 4));
        let everything = build(&candles, None, 1.0, 1, 100.0, 300.0).unwrap();
        assert_eq!(everything.value_area, (0, 5));
    }

    #[test]
    fn imbalances_compare_a_row_with_its_neighbour_and_the_edges_have_none() {
        // Row 0 sells 1. Row 1 buys 3 (3 >= 3 * 1: a buy imbalance against row 0) and sells 5
        // against the 1 row 2 buys (5 >= 3 * 1: a sell imbalance).
        let mut footprint = build(
            &[
                candle(10.5, 10.5, 10.0, 10.0, 1.0),
                candle(11.0, 11.5, 11.0, 11.5, 3.0),
                candle(11.5, 11.5, 11.0, 11.0, 5.0),
                candle(12.0, 12.5, 12.0, 12.5, 1.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(!footprint.has_buy_imbalance(0), "no row below");
        assert!(footprint.has_buy_imbalance(1));
        assert!(footprint.has_sell_imbalance(1));
        assert!(
            !footprint.has_buy_imbalance(2),
            "1 bought against 5 sold below"
        );
        assert!(!footprint.has_sell_imbalance(2), "no row above");
        // A larger threshold takes the imbalances away.
        footprint.imbalance_percent = 600.0;
        assert!(!footprint.has_buy_imbalance(1));
        assert!(!footprint.has_sell_imbalance(1));
    }

    #[test]
    fn buying_against_a_row_that_sold_nothing_is_an_imbalance() {
        let footprint = build(
            &[
                candle(10.0, 10.5, 10.0, 10.5, 2.0),
                candle(11.0, 11.5, 11.0, 11.5, 2.0),
            ],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert!(footprint.has_buy_imbalance(1));
        assert!(!footprint.has_sell_imbalance(0), "it sold nothing");
    }

    #[test]
    fn a_price_belongs_to_the_row_it_falls_in() {
        let footprint = build(
            &[candle(10.0, 12.0, 10.0, 12.0, 4.0)],
            None,
            1.0,
            1,
            70.0,
            300.0,
        )
        .unwrap();
        assert_eq!(footprint.row_at_price(10.0), Some(0));
        assert_eq!(footprint.row_at_price(10.99), Some(0));
        assert_eq!(footprint.row_at_price(11.0), Some(1));
        assert_eq!(footprint.row_at_price(12.0), Some(1), "the top counts");
        assert_eq!(footprint.row_at_price(9.99), None);
        assert_eq!(footprint.row_at_price(12.01), None);
        assert_eq!(footprint.row_at_price(f64::NAN), None);
    }

    #[test]
    fn no_volume_or_too_many_rows_is_no_footprint() {
        assert!(build(&[], None, 1.0, 1, 70.0, 300.0).is_none());
        assert!(
            build(
                &[candle(1.0, 2.0, 1.0, 2.0, 0.0)],
                None,
                1.0,
                1,
                70.0,
                300.0
            )
            .is_none()
        );
        assert!(
            build(
                &[candle(1.0, 2.0, 1.0, 2.0, 1.0)],
                None,
                0.0,
                1,
                70.0,
                300.0
            )
            .is_none()
        );
        // A row a tick tall on a bar thousands of ticks wide.
        assert!(
            build(
                &[candle(100.0, 200.0, 100.0, 200.0, 1.0)],
                None,
                0.01,
                1,
                70.0,
                300.0
            )
            .is_none()
        );
        // The same bar with taller rows is fine.
        assert!(
            build(
                &[candle(100.0, 200.0, 100.0, 200.0, 1.0)],
                None,
                0.01,
                100,
                70.0,
                300.0
            )
            .is_some()
        );
    }
}
