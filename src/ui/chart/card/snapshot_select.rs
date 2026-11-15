//! Select portion on a chart: the workspace's selection owns the pointer, so the chart stands
//! back. The rectangle itself belongs to the workspace (it may cross charts); the card only
//! keeps its hands off, the way it does for the Zoom In tool.

use super::ChartCard;

impl ChartCard {
    /// Turns the workspace's Select portion mode on or off for this chart. Set every frame.
    pub fn set_snapshot_select(&mut self, selecting: bool) {
        self.snapshot_select = selecting;
    }

    /// Whether a workspace mode owns the plot's pointer this frame, so nothing is drawn, picked
    /// or panned: the Zoom In tool, with a main plot to zoom and nothing else (the replay start
    /// picker) asking for the pointer, or Select portion, which owns the whole chart.
    pub(super) fn pointer_is_held(&self, main_visible: bool, picking: bool) -> bool {
        self.snapshot_select || self.zoom_is_live(main_visible, picking)
    }
}
