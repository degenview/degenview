//! What the chart is told about one overlay market's data: the candles (shared, never copied per
//! frame), how they changed, and whether they are loading. Pure; the workspace's `OverlayHub`
//! produces these and the chart card only reads them.

use super::Kline;
use std::sync::Arc;

/// Where an overlay market's data stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverlayStatus {
    /// Waiting for a free worker (only a few overlay requests run at once).
    Queued,
    /// The first candles are on their way.
    Loading,
    /// Candles are held (possibly none: the market has no history on this timeframe).
    Ready,
    /// The last request failed. Candles from before it, if any, are kept.
    Error(Arc<str>),
}

/// One overlay market's candles and their state, cheap to clone.
#[derive(Clone, Debug)]
pub struct OverlayFeed {
    pub candles: Arc<Vec<Kline>>,
    /// Bumps on every change of the candles, a live tick included.
    pub revision: u64,
    /// Bumps only when more than the newest candles changed (a refresh that brought history, a
    /// page of older candles, a reload). While it stays put, only the tail needs aligning again.
    pub structure: u64,
    pub status: OverlayStatus,
}

impl OverlayFeed {
    pub fn empty(status: OverlayStatus) -> Self {
        Self {
            candles: Arc::new(Vec::new()),
            revision: 0,
            structure: 0,
            status,
        }
    }
}

/// How a new set of candles differs from the last.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change {
    None,
    /// Only the last few candles differ (a live bar, a new bar).
    Tail,
    Structure,
}

/// How many of the newest candles count as "the tail".
pub const TAIL: usize = 2;

pub fn classify(old: &[Kline], new: &[Kline]) -> Change {
    let shared = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    if old.is_empty() && !new.is_empty() {
        // The first candles: nothing aligned yet, so everything must be.
        Change::Structure
    } else if shared == old.len() && shared == new.len() {
        Change::None
    } else if old.len() - shared <= TAIL && new.len() - shared <= TAIL {
        Change::Tail
    } else {
        Change::Structure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(times: &[i64]) -> Vec<Kline> {
        times
            .iter()
            .map(|t| Kline {
                open_time: *t,
                close_time: *t,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            })
            .collect()
    }

    #[test]
    fn identical_candles_are_no_change() {
        assert_eq!(classify(&run(&[1, 2, 3]), &run(&[1, 2, 3])), Change::None);
        assert_eq!(classify(&[], &[]), Change::None);
    }

    #[test]
    fn a_new_bar_or_a_moved_last_price_is_a_tail_change() {
        let old = run(&[1, 2, 3]);
        let mut moved = old.clone();
        moved[2].close = 9.0;
        assert_eq!(classify(&old, &moved), Change::Tail);
        assert_eq!(classify(&old, &run(&[1, 2, 3, 4])), Change::Tail);
    }

    #[test]
    fn history_arriving_or_leaving_is_a_structure_change() {
        let old = run(&[5, 6, 7, 8]);
        assert_eq!(
            classify(&old, &run(&[1, 2, 3, 4, 5, 6, 7, 8])),
            Change::Structure
        );
        assert_eq!(classify(&old, &run(&[5, 6])), Change::Tail);
        assert_eq!(classify(&old, &run(&[])), Change::Structure);
        assert_eq!(classify(&[], &run(&[1, 2, 3])), Change::Structure);
    }
}
