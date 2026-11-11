//! Kagi: one connected line that changes direction only when price turns back by the reversal
//! amount, and changes thickness when it breaks the previous shoulder or waist.
//!
//! A bar here is one vertical segment (`open` where it starts, `close` where it ends); the
//! horizontal step to the next segment is drawn at that level. While price keeps going the same
//! way the segment just grows. When it retraces a full reversal amount from the segment's end the
//! segment is finished and a new one begins at that end, going the other way.
//!
//! Thickness (yang/yin): a rising segment turns thick when it passes the previous peak, a falling
//! segment turns thin when it passes the previous trough, and until then keeps the thickness it
//! had. The first segment is thick if it rises and thin if it falls. The segment where the change
//! happens records the level, so the renderer can switch thickness in the middle of it.
//!
//! The first segment starts when price first moves a reversal amount from the first price read.
//! Only the last segment can still grow, so it is the one left unconfirmed.

use super::{BarShape, SyntheticBar, price_path, tolerance};
use crate::model::Kline;
use crate::model::chart::kind::ConstructionSource;

struct Segment {
    start: f64,
    end: f64,
    rising: bool,
    thick: bool,
    /// The level the thickness changed at, within this segment.
    switch: Option<f64>,
    first_source: usize,
    volume: f64,
    last_source: usize,
}

pub(super) fn build(
    source: &[Kline],
    reversal: f64,
    price_source: ConstructionSource,
    forming: Option<usize>,
) -> Vec<SyntheticBar> {
    let slack = tolerance(reversal);
    let mut finished: Vec<Segment> = Vec::new();
    let mut current: Option<Segment> = None;
    let mut reference: Option<f64> = None;
    let mut last_peak: Option<f64> = None;
    let mut last_trough: Option<f64> = None;
    let mut volume_since_segment = 0.0;
    let mut first_since_segment: Option<usize> = None;

    for (index, candle) in source.iter().enumerate() {
        volume_since_segment += candle.volume;
        first_since_segment.get_or_insert(index);
        let (path, count) = price_path(candle, price_source);
        for &price in &path[..count] {
            let anchor = *reference.get_or_insert(price);
            match current.as_mut() {
                None => {
                    let moved = price - anchor;
                    if moved.abs() >= reversal - slack {
                        current = Some(Segment {
                            start: anchor,
                            end: price,
                            rising: moved > 0.0,
                            thick: moved > 0.0,
                            switch: None,
                            first_source: first_since_segment.unwrap_or(index),
                            volume: 0.0,
                            last_source: index,
                        });
                    }
                }
                Some(segment) => {
                    let extends = if segment.rising {
                        price > segment.end
                    } else {
                        price < segment.end
                    };
                    let turned = if segment.rising {
                        price <= segment.end - reversal + slack
                    } else {
                        price >= segment.end + reversal - slack
                    };
                    if extends {
                        segment.end = price;
                        segment.last_source = index;
                        if segment.rising {
                            if !segment.thick && last_peak.is_some_and(|peak| price > peak) {
                                segment.thick = true;
                                segment.switch = last_peak;
                            }
                        } else if segment.thick && last_trough.is_some_and(|trough| price < trough)
                        {
                            segment.thick = false;
                            segment.switch = last_trough;
                        }
                    } else if turned {
                        let rising = !segment.rising;
                        let thick = segment.thick;
                        let end = segment.end;
                        if segment.rising {
                            last_peak = Some(end);
                        } else {
                            last_trough = Some(end);
                        }
                        let done = current.take().expect("segment");
                        finished.push(done);
                        current = Some(Segment {
                            start: end,
                            end: price,
                            rising,
                            thick,
                            switch: None,
                            first_source: index,
                            volume: 0.0,
                            last_source: index,
                        });
                        // The new segment starts thin or thick as the old one ended; a turn alone
                        // does not change it.
                        let started = current.as_mut().expect("segment");
                        if started.rising {
                            if !started.thick && last_peak.is_some_and(|peak| price > peak) {
                                started.thick = true;
                                started.switch = last_peak;
                            }
                        } else if started.thick && last_trough.is_some_and(|trough| price < trough)
                        {
                            started.thick = false;
                            started.switch = last_trough;
                        }
                        // Volume since the last segment closed belongs to the one that closed.
                        if let Some(done) = finished.last_mut() {
                            done.volume = volume_since_segment;
                        }
                        volume_since_segment = 0.0;
                        first_since_segment = Some(index);
                    }
                }
            }
        }
    }
    if let Some(mut segment) = current {
        segment.volume = volume_since_segment;
        finished.push(segment);
    }

    let count = finished.len();
    finished
        .into_iter()
        .enumerate()
        .map(|(position, segment)| SyntheticBar {
            open: segment.start,
            high: segment.start.max(segment.end),
            low: segment.start.min(segment.end),
            close: segment.end,
            volume: segment.volume,
            first_source: segment.first_source,
            last_source: segment.last_source,
            open_time: source[segment.first_source].open_time,
            close_time: source[segment.last_source].close_time,
            confirmed: position + 1 < count && forming != Some(segment.last_source),
            shape: BarShape::Kagi {
                thick: segment.thick,
                switch: segment.switch,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    fn run(values: &[f64], reversal: f64) -> Vec<SyntheticBar> {
        build(&closes(values), reversal, ConstructionSource::Close, None)
    }

    fn spans(bars: &[SyntheticBar]) -> Vec<(f64, f64)> {
        bars.iter().map(|bar| (bar.open, bar.close)).collect()
    }

    #[test]
    fn the_first_segment_waits_for_a_reversal_amount_of_movement() {
        assert!(run(&[10.0, 10.5, 9.8], 1.0).is_empty());
        assert_eq!(spans(&run(&[10.0, 10.5, 11.0], 1.0)), vec![(10.0, 11.0)]);
    }

    #[test]
    fn a_segment_grows_until_price_turns_back_a_full_reversal() {
        // Up to 14; 13.1 is only 0.9 back: still the same segment. 13.0 reverses.
        let bars = run(&[10.0, 12.0, 14.0, 13.1, 13.0], 1.0);
        assert_eq!(spans(&bars), vec![(10.0, 14.0), (14.0, 13.0)]);
        assert!(bars[0].is_up() && !bars[1].is_up());
    }

    #[test]
    fn the_horizontal_step_is_the_previous_end() {
        let bars = run(&[10.0, 12.0, 11.0, 13.0], 1.0);
        assert_eq!(spans(&bars), vec![(10.0, 12.0), (12.0, 11.0), (11.0, 13.0)]);
        assert_eq!(bars[1].open, bars[0].close);
        assert_eq!(bars[2].open, bars[1].close);
    }

    #[test]
    fn thickness_turns_on_breaking_the_previous_shoulder_and_off_below_the_waist() {
        // Up to 12 (thick), down to 10.5 (still thick), up to 11.5 (below the 12 shoulder: still
        // thick), up past 12 to 13 (thick already), down to 9 (breaks the waist... none yet),
        // then check a thin → thick switch below.
        let bars = run(&[10.0, 12.0, 10.5, 13.0, 9.0], 1.0);
        assert_eq!(
            spans(&bars),
            vec![(10.0, 12.0), (12.0, 10.5), (10.5, 13.0), (13.0, 9.0)]
        );
        // The first fall (to 10.5) has no previous trough, so it stays thick; the next fall to 9
        // passes the 10.5 trough and goes thin at that level.
        let thick: Vec<bool> = bars
            .iter()
            .map(|bar| matches!(bar.shape, BarShape::Kagi { thick: true, .. }))
            .collect();
        assert_eq!(thick, vec![true, true, true, false]);
        match bars[3].shape {
            BarShape::Kagi { switch, .. } => assert_eq!(switch, Some(10.5)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_thin_line_turns_thick_when_it_rises_past_the_previous_peak() {
        // Falls first (thin), bounces to 11 (peak), falls to 9, rallies to 12: past the 11 peak.
        let bars = run(&[10.0, 8.0, 11.0, 9.5, 12.0], 1.0);
        assert_eq!(
            spans(&bars),
            vec![(10.0, 8.0), (8.0, 11.0), (11.0, 9.5), (9.5, 12.0)]
        );
        match bars[3].shape {
            BarShape::Kagi { thick, switch } => {
                assert!(thick);
                assert_eq!(switch, Some(11.0));
            }
            _ => unreachable!(),
        }
        assert!(matches!(bars[0].shape, BarShape::Kagi { thick: false, .. }));
    }

    #[test]
    fn only_the_last_segment_is_unconfirmed() {
        let bars = run(&[10.0, 12.0, 10.0, 12.0], 1.0);
        assert_eq!(bars.len(), 3);
        assert!(bars[0].confirmed && bars[1].confirmed && !bars[2].confirmed);
    }

    #[test]
    fn a_prefix_of_the_candles_gives_a_prefix_of_the_confirmed_segments() {
        let values = [10.0, 12.0, 10.0, 12.5, 9.0, 13.0];
        let full = run(&values, 1.0);
        let prefix = run(&values[..5], 1.0);
        let settled = prefix.len() - 1;
        assert_eq!(&full[..settled], &prefix[..settled]);
    }
}
