//! The cursor modes on a card: which one the workspace chose, and the Dot cursor's own mark.
//!
//! The lines, tags and readout a mode keeps are decided in `crosshair.rs` from
//! `CursorMode::caps`; the system pointer in `navigation.rs`. Nothing here is saved with the
//! chart: the mode belongs to the workspace and arrives each frame.

use super::*;
use crate::model::CursorMode;

/// The Dot's diameter in points: small enough to sit on one candle's body.
const DOT_RADIUS: f32 = 3.0;

impl ChartCard {
    /// Sets the workspace's cursor, pushed each frame before the card is drawn.
    pub fn set_cursor_mode(&mut self, mode: CursorMode) {
        self.cursor = mode;
        if mode != CursorMode::Dot {
            self.dot_shown = false;
        }
        if !mode.caps().highlights {
            self.demo.clear();
        }
    }

    /// The Dot cursor: a small mark under the pointer where the system pointer was hidden. It
    /// reads the live pointer, so it never lags the hand the way the crosshair's shared column
    /// (one frame behind) can, and only the chart the pointer is over paints it.
    pub(super) fn paint_dot(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        chart_id: u64,
        crosshair: &CrosshairInfo,
    ) {
        if !self.dot_shown || crosshair.frozen || crosshair.owner_id != chart_id {
            return;
        }
        let Some(pointer) = painter.ctx().input(|input| input.pointer.hover_pos()) else {
            return;
        };
        if plot.contains(pointer) {
            painter.circle(
                pointer,
                DOT_RADIUS,
                theme::text_primary(),
                Stroke::new(1.0, theme::background()),
            );
        }
    }
}
