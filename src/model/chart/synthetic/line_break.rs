//! Line Break (three-line break by default): a new line forms only when a close breaks out of the
//! recent lines, so noise inside them draws nothing.
//!
//! * **Continuation:** a close beyond the last line's end (above a rising line, below a falling
//!   one) adds a line in the same direction, opening at that line's close.
//! * **Reversal:** a close beyond the *whole run* of the last `lines` lines, on the other side,
//!   adds a line the other way, opening at the last line's open.
//!
//! The first line forms on the first close that differs from the first close, opening there. A
//! source candle makes at most one line, from its close. TradingView's text does not spell out
//! where a reversal line opens or how the first line starts; these are the textbook rules.

use super::{BarShape, Pending, SyntheticBar};
use crate::model::Kline;

#[derive(Clone, Copy)]
struct Line {
    open: f64,
    close: f64,
}

impl Line {
    fn rising(self) -> bool {
        self.close > self.open
    }

    fn top(self) -> f64 {
        self.open.max(self.close)
    }

    fn bottom(self) -> f64 {
        self.open.min(self.close)
    }
}

pub(super) fn build(source: &[Kline], lines: usize, forming: Option<usize>) -> Vec<SyntheticBar> {
    let mut bars: Vec<SyntheticBar> = Vec::new();
    let mut shapes: Vec<Line> = Vec::new();
    let mut reference: Option<f64> = None;
    let mut pending: Option<Pending> = None;

    for (index, candle) in source.iter().enumerate() {
        match pending.as_mut() {
            Some(pending) => pending.absorb(candle),
            None => pending = Some(Pending::start(index, candle)),
        }
        let close = candle.close;
        let anchor = *reference.get_or_insert(close);
        let next = match shapes.last().copied() {
            None if close != anchor => Some(Line {
                open: anchor,
                close,
            }),
            None => None,
            Some(last) => {
                let window = &shapes[shapes.len().saturating_sub(lines)..];
                let highest = window
                    .iter()
                    .map(|line| line.top())
                    .fold(f64::MIN, f64::max);
                let lowest = window
                    .iter()
                    .map(|line| line.bottom())
                    .fold(f64::MAX, f64::min);
                if last.rising() {
                    if close > last.close {
                        Some(Line {
                            open: last.close,
                            close,
                        })
                    } else if close < lowest {
                        Some(Line {
                            open: last.open,
                            close,
                        })
                    } else {
                        None
                    }
                } else if close < last.close {
                    Some(Line {
                        open: last.close,
                        close,
                    })
                } else if close > highest {
                    Some(Line {
                        open: last.open,
                        close,
                    })
                } else {
                    None
                }
            }
        };
        let Some(line) = next else { continue };
        let formed = pending.take().unwrap_or(Pending::start(index, candle));
        bars.push(SyntheticBar {
            open: line.open,
            high: line.top(),
            low: line.bottom(),
            close: line.close,
            volume: formed.volume,
            first_source: formed.first_source,
            last_source: index,
            open_time: source[formed.first_source].open_time,
            close_time: candle.close_time,
            confirmed: forming != Some(index),
            shape: BarShape::Ohlc,
        });
        shapes.push(line);
        pending = Some(Pending {
            first_source: index,
            volume: 0.0,
            high: candle.high,
            low: candle.low,
        });
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn run(values: &[f64], lines: usize) -> Vec<(f64, f64)> {
        build(&closes(values), lines, None)
            .iter()
            .map(|bar| (bar.open, bar.close))
            .collect()
    }

    #[test]
    fn closes_that_do_not_break_the_last_line_draw_nothing() {
        assert_eq!(run(&[10.0, 12.0, 11.5, 12.0, 11.9], 3), vec![(10.0, 12.0)]);
    }

    #[test]
    fn a_higher_close_continues_from_the_last_close() {
        assert_eq!(
            run(&[10.0, 12.0, 13.0, 15.0], 3),
            vec![(10.0, 12.0), (12.0, 13.0), (13.0, 15.0)]
        );
    }

    #[test]
    fn a_reversal_must_break_the_whole_window() {
        // Rising lines 10→12, 12→13, 13→15. Three-line window low is 10: 11 does not reverse.
        assert_eq!(run(&[10.0, 12.0, 13.0, 15.0, 11.0], 3).len(), 3);
        // 9.9 does, opening at the last line's open (13).
        let lines = run(&[10.0, 12.0, 13.0, 15.0, 9.9], 3);
        assert_eq!(lines.last(), Some(&(13.0, 9.9)));
    }

    #[test]
    fn a_shorter_window_reverses_sooner() {
        // With one line, any close below the last line's bottom reverses it.
        let lines = run(&[10.0, 12.0, 13.0, 15.0, 12.5], 1);
        assert_eq!(lines.last(), Some(&(13.0, 12.5)));
    }

    #[test]
    fn falling_trend_mirrors_the_rising_one() {
        assert_eq!(
            run(&[10.0, 8.0, 7.0, 12.0], 3),
            vec![(10.0, 8.0), (8.0, 7.0), (8.0, 12.0)],
            "a close above the three-line high (10) reverses, opening at the last line's open"
        );
    }

    #[test]
    fn a_candle_makes_at_most_one_line_and_flat_start_waits() {
        assert_eq!(run(&[10.0, 10.0, 10.0], 3), vec![]);
        let source = vec![
            candle(0, 10.0, 10.0, 10.0, 10.0),
            candle(1, 10.0, 99.0, 1.0, 11.0),
        ];
        assert_eq!(build(&source, 3, None).len(), 1);
    }

    #[test]
    fn the_forming_candles_line_is_unconfirmed_and_volume_accumulates() {
        let source = closes(&[10.0, 10.0, 12.0]);
        let bars = build(&source, 3, Some(2));
        assert_eq!(bars.len(), 1);
        assert!(!bars[0].confirmed);
        assert_eq!(bars[0].volume, 30.0);
        assert_eq!((bars[0].first_source, bars[0].last_source), (0, 2));
    }

    #[test]
    fn a_prefix_of_the_candles_gives_a_prefix_of_the_lines() {
        let values = [10.0, 12.0, 13.0, 15.0, 9.0, 8.0, 16.0];
        let full = run(&values, 3);
        let prefix = run(&values[..5], 3);
        assert_eq!(&full[..prefix.len()], &prefix[..]);
    }
}
