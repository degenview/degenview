//! Point & Figure: columns of Xs (rising) and Os (falling) on a grid of boxes of one size.
//!
//! Price falls into box `floor(price / size)`. A column extends while price enters new boxes in
//! its direction, and gives way to a column of the other kind once price has retraced
//! `reversal` boxes from the column's far end (three by default). The new column starts one box
//! in from the old one's end, so a reversal always covers at least `reversal` boxes.
//!
//! The first column begins when price first leaves the box of the first price read, and includes
//! that box. Time plays no part: a column holds as many source candles as it took.
//!
//! A bar is one column: `open` is where the column starts and `close` where it ends (box edges),
//! `high` and `low` are its extremes. Only the last column can still grow, so it is the one left
//! unconfirmed.

use super::{BarShape, SyntheticBar, price_path};
use crate::model::Kline;
use crate::model::chart::kind::ConstructionSource;

struct Column {
    rising: bool,
    low: i64,
    high: i64,
    first_source: usize,
    last_source: usize,
    volume: f64,
}

fn box_of(price: f64, size: f64) -> i64 {
    (price / size + 1e-9).floor() as i64
}

pub(super) fn build(
    source: &[Kline],
    size: f64,
    reversal: u32,
    price_source: ConstructionSource,
    forming: Option<usize>,
) -> Vec<SyntheticBar> {
    let reversal = i64::from(reversal.max(1));
    let mut finished: Vec<Column> = Vec::new();
    let mut current: Option<Column> = None;
    let mut anchor: Option<i64> = None;
    let mut volume = 0.0;
    let mut first: Option<usize> = None;

    for (index, candle) in source.iter().enumerate() {
        volume += candle.volume;
        first.get_or_insert(index);
        let (path, count) = price_path(candle, price_source);
        for &price in &path[..count] {
            let boxed = box_of(price, size);
            let anchor = *anchor.get_or_insert(boxed);
            match current.as_mut() {
                None => {
                    if boxed != anchor {
                        current = Some(Column {
                            rising: boxed > anchor,
                            low: boxed.min(anchor),
                            high: boxed.max(anchor),
                            first_source: first.unwrap_or(index),
                            last_source: index,
                            volume: 0.0,
                        });
                    }
                }
                Some(column) if column.rising => {
                    if boxed > column.high {
                        column.high = boxed;
                        column.last_source = index;
                    } else if column.high - boxed >= reversal {
                        let (end, mut done) = (column.high, current.take().expect("column"));
                        done.volume = volume;
                        volume = 0.0;
                        finished.push(done);
                        first = Some(index);
                        current = Some(Column {
                            rising: false,
                            low: boxed,
                            high: end - 1,
                            first_source: index,
                            last_source: index,
                            volume: 0.0,
                        });
                    }
                }
                Some(column) => {
                    if boxed < column.low {
                        column.low = boxed;
                        column.last_source = index;
                    } else if boxed - column.low >= reversal {
                        let (end, mut done) = (column.low, current.take().expect("column"));
                        done.volume = volume;
                        volume = 0.0;
                        finished.push(done);
                        first = Some(index);
                        current = Some(Column {
                            rising: true,
                            low: end + 1,
                            high: boxed,
                            first_source: index,
                            last_source: index,
                            volume: 0.0,
                        });
                    }
                }
            }
        }
    }
    if let Some(mut column) = current {
        column.volume = volume;
        finished.push(column);
    }

    let count = finished.len();
    finished
        .into_iter()
        .enumerate()
        .map(|(position, column)| {
            let bottom = column.low as f64 * size;
            let top = (column.high + 1) as f64 * size;
            SyntheticBar {
                open: if column.rising { bottom } else { top },
                high: top,
                low: bottom,
                close: if column.rising { top } else { bottom },
                volume: column.volume,
                first_source: column.first_source,
                last_source: column.last_source,
                open_time: source[column.first_source].open_time,
                close_time: source[column.last_source].close_time,
                confirmed: position + 1 < count && forming != Some(column.last_source),
                shape: BarShape::Column {
                    rising: column.rising,
                    boxes: (column.high - column.low + 1) as u32,
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn run(values: &[f64], size: f64, reversal: u32) -> Vec<SyntheticBar> {
        build(
            &closes(values),
            size,
            reversal,
            ConstructionSource::Close,
            None,
        )
    }

    /// `(rising, bottom, top)` of each column.
    fn columns(bars: &[SyntheticBar]) -> Vec<(bool, f64, f64)> {
        bars.iter()
            .map(|bar| (bar.is_up(), bar.low, bar.high))
            .collect()
    }

    #[test]
    fn a_rise_fills_boxes_from_the_first_box_up() {
        // Box 10 (10..11) up to box 13 (13..14): X column of four boxes, 10…14.
        let bars = run(&[10.2, 11.5, 13.9], 1.0, 3);
        assert_eq!(columns(&bars), vec![(true, 10.0, 14.0)]);
        assert!(matches!(
            bars[0].shape,
            BarShape::Column {
                rising: true,
                boxes: 4
            }
        ));
    }

    #[test]
    fn a_pullback_short_of_the_reversal_changes_nothing() {
        // Column tops out in box 14; box 12 is two boxes back, under the three needed.
        let bars = run(&[10.0, 14.5, 12.5], 1.0, 3);
        assert_eq!(columns(&bars), vec![(true, 10.0, 15.0)]);
    }

    #[test]
    fn three_boxes_back_starts_an_o_column_one_box_in() {
        // Top box 14 (14..15). Box 11 is three back: O column from box 13 down to box 11.
        let bars = run(&[10.0, 14.5, 11.5], 1.0, 3);
        assert_eq!(
            columns(&bars),
            vec![(true, 10.0, 15.0), (false, 11.0, 14.0)]
        );
        assert!(matches!(
            bars[1].shape,
            BarShape::Column {
                rising: false,
                boxes: 3
            }
        ));
        assert_eq!((bars[1].open, bars[1].close), (14.0, 11.0));
    }

    #[test]
    fn a_rally_reverses_the_o_column_the_same_way() {
        let bars = run(&[10.0, 14.5, 11.5, 14.1], 1.0, 3);
        // O column low box 11; box 14 is three above: X column from box 12 up to box 14.
        assert_eq!(
            columns(&bars),
            vec![(true, 10.0, 15.0), (false, 11.0, 14.0), (true, 12.0, 15.0)]
        );
    }

    #[test]
    fn the_box_size_and_exact_edges_use_the_grid() {
        // Size 5: 24.9 is box 4 (20..25); 25.0 is box 5 and enters a new box.
        assert_eq!(columns(&run(&[22.0, 24.9], 5.0, 3)), vec![]);
        assert_eq!(
            columns(&run(&[22.0, 25.0], 5.0, 3)),
            vec![(true, 20.0, 30.0)]
        );
        // A decimal edge that is a hair under in binary still lands in the upper box.
        assert_eq!(run(&[0.1, 0.1 + 0.2], 0.1, 3).len(), 1);
    }

    #[test]
    fn a_one_box_reversal_flips_on_any_new_box_against_the_column() {
        let bars = run(&[10.0, 13.0, 11.9], 1.0, 1);
        assert_eq!(
            columns(&bars),
            vec![(true, 10.0, 14.0), (false, 11.0, 13.0)]
        );
    }

    #[test]
    fn only_the_last_column_is_unconfirmed() {
        let bars = run(&[10.0, 14.5, 11.5, 14.1], 1.0, 3);
        assert_eq!(
            bars.iter().map(|bar| bar.confirmed).collect::<Vec<_>>(),
            vec![true, true, false]
        );
    }

    #[test]
    fn the_high_low_source_can_extend_and_reverse_inside_one_candle() {
        // A falling candle is read open, high, low, close: 12 → 15 → 10 → 11.
        let source = vec![
            candle(0, 12.0, 12.0, 12.0, 12.0),
            candle(1, 12.0, 15.0, 10.0, 11.0),
        ];
        let bars = build(&source, 1.0, 3, ConstructionSource::HighLow, None);
        assert_eq!(
            columns(&bars),
            vec![(true, 12.0, 16.0), (false, 10.0, 15.0)]
        );
    }

    #[test]
    fn a_prefix_of_the_candles_gives_a_prefix_of_the_columns() {
        let values = [10.0, 14.5, 11.5, 14.1, 9.0, 13.0];
        let full = run(&values, 1.0, 3);
        let prefix = run(&values[..4], 1.0, 3);
        let settled = prefix.len() - 1;
        assert_eq!(&full[..settled], &prefix[..settled]);
    }
}
