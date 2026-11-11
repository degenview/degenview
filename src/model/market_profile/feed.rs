//! What a chart card is handed to draw an order-flow chart type: the finished sets, never the raw
//! trades and candles (a million trades are not copied into a card).

use super::footprint::FootprintSet;
use std::sync::Arc;

/// The data of the selected order-flow type, as of the last time the workspace computed it. Cloning
/// is a pointer copy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileFeed {
    pub footprint: Option<Arc<FootprintSet>>,
    /// Finer data is still being fetched (the chart draws what it has meanwhile).
    pub loading: bool,
    /// The last request for finer data failed.
    pub error: Option<Arc<str>>,
}

impl ProfileFeed {
    pub fn loading() -> Self {
        Self {
            loading: true,
            ..Self::default()
        }
    }
}
