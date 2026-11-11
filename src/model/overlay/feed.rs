//! What the chart is told about one overlay market's data: the candles (shared, never copied per
//! frame) and whether they are loading. Pure; the workspace's `OverlayHub` produces these and the
//! chart card only reads them.

use crate::model::Kline;
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
    /// The overlay cannot be drawn on this chart, and why ("Same market as the chart").
    Blocked(&'static str),
}

/// One overlay market's candles and their state, cheap to clone.
#[derive(Clone, Debug)]
pub struct OverlayFeed {
    pub candles: Arc<Vec<Kline>>,
    /// Bumps on every change of the candles, a live tick included.
    pub revision: u64,
    pub status: OverlayStatus,
}

impl OverlayFeed {
    pub fn empty(status: OverlayStatus) -> Self {
        Self {
            candles: Arc::new(Vec::new()),
            revision: 0,
            status,
        }
    }
}
