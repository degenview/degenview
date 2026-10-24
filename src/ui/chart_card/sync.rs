//! A chart's side of linked navigation (see [`crate::model::chart_sync`]).
//!
//! The card never keeps a second viewport: it reports what the user's navigation did to the one
//! it has, and applies what the workspace's hub hands it by editing that same viewport before the
//! frame reads it.

use super::ChartCard;
use crate::model::chart_sync::{ChartSyncSettings, HorizontalChange, SyncHub, TimeAxis};
use crate::model::{DataSource, Kline};

impl ChartCard {
    /// Whether this chart takes part in linked navigation.
    ///
    /// A price-based type (Renko, Kagi, Line Break, Point & Figure, Range) draws bars whose
    /// width is a price move, not elapsed time: its slots have no time to share, and inventing
    /// one would make the charts disagree about the date. Such a chart neither leads nor follows;
    /// every time-based type, and the line charts of prediction markets, do.
    pub fn takes_part_in_sync(&self, source: DataSource) -> bool {
        source.is_line_chart() || !self.appearance.style.effective_type(source).is_synthetic()
    }

    /// Brings the viewport to where the chart that last navigated left the others, before this
    /// frame reads it. `candles` are the bars the chart is about to draw. A chart the user is
    /// dragging is not moved from under the pointer.
    pub fn follow_sync(
        &mut self,
        hub: &mut SyncHub,
        id: u64,
        settings: ChartSyncSettings,
        source: DataSource,
        candles: &[Kline],
    ) -> bool {
        if self.gesture.is_some() || !self.takes_part_in_sync(source) {
            return false;
        }
        hub.deliver(
            id,
            settings,
            &TimeAxis::new(candles),
            &mut self.viewport.horizontal,
        )
    }

    /// Remembers the viewport as the frame's input starts from, once live bars have been
    /// accounted for: a bar arriving is not navigation.
    pub(super) fn begin_sync_frame(&mut self) {
        self.sync_baseline = Some(self.scroll_from.take().unwrap_or(self.viewport.horizontal));
    }

    /// What the user's navigation did to the horizontal viewport this frame, if anything the
    /// other charts share. Called after the body is drawn; the vertical viewport is not looked at.
    pub(super) fn end_sync_frame(&mut self, source: DataSource, candles: &[Kline]) {
        self.sync_change = None;
        let Some(before) = self.sync_baseline.take() else {
            return;
        };
        if self.takes_part_in_sync(source) {
            self.sync_change = HorizontalChange::between(
                &before,
                &self.viewport.horizontal,
                &TimeAxis::new(candles),
            );
        }
    }

    /// Hands what the user's navigation changed this frame to the hub, for the other charts to
    /// follow. A chart that merely followed has no change of its own to hand over. Returns
    /// whether anything was published.
    pub fn publish_sync(
        &mut self,
        hub: &mut SyncHub,
        id: u64,
        settings: ChartSyncSettings,
    ) -> bool {
        let change = self.sync_change.take();
        match change.filter(|_| settings.is_linked()) {
            Some(change) => {
                hub.publish(id, change);
                true
            }
            None => false,
        }
    }

    /// The horizontal viewport, for tests and diagnostics.
    #[cfg(test)]
    pub fn horizontal(&self) -> crate::model::viewport::HorizontalViewport {
        self.viewport.horizontal
    }
}
