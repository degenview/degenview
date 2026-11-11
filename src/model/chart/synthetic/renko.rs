//! Renko: bricks of a fixed size that form when price moves a full brick beyond the last one.
//!
//! * A brick continues the trend when price travels one box past the last brick's close.
//! * It reverses only after price travels one box past the last brick's *open*, i.e. two boxes
//!   from the close, so small reversals do not draw.
//! * One source candle can form several bricks (a run) or none.
//!
//! The first brick is measured from the first price read (the first close, or the first open with
//! the high/low source). TradingView does not document how it aligns the first brick, so no
//! grid snapping is assumed. The size is resolved once from the whole series (see
//! [`SizeRule::resolve`](crate::model::chart::kind::SizeRule::resolve)).
//!
//! The partly formed brick is not shown: only bricks price has completed exist, and the ones a
//! still-forming candle has completed so far are unconfirmed. With `wicks`, a brick's wick reaches
//! the highest high and lowest low of the candles since the previous brick.

use super::{BarShape, Pending, SyntheticBar, price_path, tolerance};
use crate::model::Kline;
use crate::model::chart::kind::ConstructionSource;

#[derive(Clone, Copy)]
struct Brick {
    open: f64,
    close: f64,
}

impl Brick {
    fn rising(self) -> bool {
        self.close > self.open
    }
}

pub(super) fn build(
    source: &[Kline],
    size: f64,
    price_source: ConstructionSource,
    wicks: bool,
    forming: Option<usize>,
) -> Vec<SyntheticBar> {
    let slack = tolerance(size);
    let mut bars: Vec<SyntheticBar> = Vec::new();
    let mut last: Option<Brick> = None;
    let mut reference: Option<f64> = None;
    let mut pending: Option<Pending> = None;

    for (index, candle) in source.iter().enumerate() {
        match pending.as_mut() {
            Some(pending) => pending.absorb(candle),
            None => pending = Some(Pending::start(index, candle)),
        }
        let (path, count) = price_path(candle, price_source);
        for &price in &path[..count] {
            let anchor = *reference.get_or_insert(price);
            loop {
                let next = match last {
                    None if price >= anchor + size - slack => Brick {
                        open: anchor,
                        close: anchor + size,
                    },
                    None if price <= anchor - size + slack => Brick {
                        open: anchor,
                        close: anchor - size,
                    },
                    None => break,
                    Some(brick) if brick.rising() => {
                        if price >= brick.close + size - slack {
                            Brick {
                                open: brick.close,
                                close: brick.close + size,
                            }
                        } else if price <= brick.open - size + slack {
                            Brick {
                                open: brick.open,
                                close: brick.open - size,
                            }
                        } else {
                            break;
                        }
                    }
                    Some(brick) => {
                        if price <= brick.close - size + slack {
                            Brick {
                                open: brick.close,
                                close: brick.close - size,
                            }
                        } else if price >= brick.open + size - slack {
                            Brick {
                                open: brick.open,
                                close: brick.open + size,
                            }
                        } else {
                            break;
                        }
                    }
                };
                let formed = pending.take().unwrap_or(Pending::start(index, candle));
                let (top, bottom) = (next.open.max(next.close), next.open.min(next.close));
                bars.push(SyntheticBar {
                    open: next.open,
                    high: if wicks { top.max(formed.high) } else { top },
                    low: if wicks {
                        bottom.min(formed.low)
                    } else {
                        bottom
                    },
                    close: next.close,
                    volume: formed.volume,
                    first_source: formed.first_source,
                    last_source: index,
                    open_time: source[formed.first_source].open_time,
                    close_time: candle.close_time,
                    confirmed: forming != Some(index),
                    shape: BarShape::Ohlc,
                });
                last = Some(next);
                // A further brick of the same candle starts empty: its volume and extremes went
                // with the first.
                pending = Some(Pending {
                    first_source: index,
                    volume: 0.0,
                    high: next.close,
                    low: next.close,
                });
            }
        }
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn run(values: &[f64], size: f64) -> Vec<(f64, f64)> {
        let source = closes(values);
        build(&source, size, ConstructionSource::Close, false, None)
            .iter()
            .map(|bar| (bar.open, bar.close))
            .collect()
    }

    #[test]
    fn a_trend_continues_one_box_at_a_time() {
        assert_eq!(
            run(&[10.0, 10.9, 11.0, 12.0], 1.0),
            vec![(10.0, 11.0), (11.0, 12.0)]
        );
    }

    #[test]
    fn touching_the_threshold_forms_a_brick() {
        // 0.1 + 0.2 sits a hair under 0.3 in binary; the slack still counts it.
        assert_eq!(run(&[0.1, 0.1 + 0.2], 0.2).len(), 1);
    }

    #[test]
    fn a_reversal_needs_two_boxes_from_the_close() {
        // Up to 12 (bricks 10→11, 11→12). 11.0 is one box back: nothing. 10.0 is two: a down
        // brick opens at the last brick's open (11) and closes one box lower (10).
        assert_eq!(
            run(&[10.0, 12.0, 11.0], 1.0),
            vec![(10.0, 11.0), (11.0, 12.0)]
        );
        assert_eq!(
            run(&[10.0, 12.0, 11.0, 10.0], 1.0),
            vec![(10.0, 11.0), (11.0, 12.0), (11.0, 10.0)]
        );
    }

    #[test]
    fn one_candle_can_run_several_bricks_and_a_quiet_one_none() {
        let bricks = run(&[10.0, 10.2, 13.5, 13.6], 1.0);
        assert_eq!(bricks, vec![(10.0, 11.0), (11.0, 12.0), (12.0, 13.0)]);
    }

    #[test]
    fn a_drop_builds_down_bricks_and_a_bounce_reverses_on_the_open() {
        let bricks = run(&[10.0, 8.0, 9.0, 10.0], 1.0);
        assert_eq!(
            bricks,
            vec![(10.0, 9.0), (9.0, 8.0), (9.0, 10.0)],
            "up brick opens at the last down brick's open (9) and closes at 10"
        );
    }

    #[test]
    fn the_high_low_source_reads_the_candles_path() {
        // A rising candle is read open, low, high, close: 10 → 9.5 → 12 → 11.5. Up bricks to 12.
        let source = vec![candle(0, 10.0, 12.0, 9.5, 11.5)];
        let bars = build(&source, 1.0, ConstructionSource::HighLow, false, None);
        assert_eq!(
            bars.iter()
                .map(|bar| (bar.open, bar.close))
                .collect::<Vec<_>>(),
            vec![(10.0, 11.0), (11.0, 12.0)]
        );
        // The close source sees only 11.5: one brick.
        let closes_only = build(&source, 1.0, ConstructionSource::Close, false, None);
        assert_eq!(
            closes_only.len(),
            0,
            "11.5 is under 10 + 1 from a first price of 11.5"
        );
    }

    #[test]
    fn wicks_reach_the_extremes_since_the_last_brick() {
        let source = vec![
            candle(0, 10.0, 10.0, 10.0, 10.0),
            candle(1, 10.0, 11.6, 9.4, 11.2),
        ];
        let plain = build(&source, 1.0, ConstructionSource::Close, false, None);
        let wicked = build(&source, 1.0, ConstructionSource::Close, true, None);
        assert_eq!((plain[0].high, plain[0].low), (11.0, 10.0));
        assert_eq!((wicked[0].high, wicked[0].low), (11.6, 9.4));
    }

    #[test]
    fn volume_goes_to_the_brick_the_candles_made() {
        let source = closes(&[10.0, 10.5, 12.5]);
        let bars = build(&source, 1.0, ConstructionSource::Close, false, None);
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].volume, 30.0);
        assert_eq!(bars[1].volume, 0.0);
        assert_eq!((bars[0].first_source, bars[0].last_source), (0, 2));
        assert_eq!((bars[1].first_source, bars[1].last_source), (2, 2));
    }

    #[test]
    fn bricks_of_the_forming_candle_are_unconfirmed() {
        let source = closes(&[10.0, 11.0, 12.0]);
        let bars = build(&source, 1.0, ConstructionSource::Close, false, Some(2));
        assert_eq!(bars.len(), 2);
        assert!(bars[0].confirmed);
        assert!(!bars[1].confirmed);
    }

    #[test]
    fn a_prefix_of_the_candles_gives_a_prefix_of_the_bricks() {
        let values = [10.0, 12.5, 11.0, 9.0, 9.4, 12.0];
        let full = run(&values, 1.0);
        let prefix = run(&values[..4], 1.0);
        assert_eq!(&full[..prefix.len()], &prefix[..]);
    }
}
