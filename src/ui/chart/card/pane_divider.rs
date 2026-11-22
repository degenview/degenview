//! The lines between the price chart and its script panes, and between one pane and the next: a
//! drag on one moves height between the two panes it separates. A pane's height is saved with its
//! script (`PineInstance::pane_share`), so it survives a restart and follows the pane when the
//! scripts are reordered.

use crate::egui::{CursorIcon, Id, Rect, Sense, Ui};

use super::ChartCard;
use super::frame::FrameLayout;
use crate::model::chart::canvas::{self, PaneHeights, PaneSpec};

/// How far either side of a line the pointer still grabs it.
const GRAB: f32 = 3.0;

/// A drag in progress: the pane whose top line is held, and the chart as the drag found it. Every
/// frame is measured from that split, so rounding never builds up over a slow drag.
#[derive(Clone, Debug)]
pub(super) struct PaneDrag {
    pane: u64,
    total: f32,
    panes: Vec<PaneSpec>,
    start: PaneHeights,
}

impl PaneDrag {
    /// Whether the chart still has the panes, folded the same way, and the height the drag began
    /// on. If not (a script was added or removed, the window was resized) the drag is over: its
    /// start no longer says where anything is.
    fn is_on(&self, layout: &FrameLayout) -> bool {
        self.total == layout.full_rect.height()
            && self.panes.len() == layout.pane_specs.len()
            && self
                .panes
                .iter()
                .zip(&layout.pane_specs)
                .all(|(at_start, now)| at_start.id == now.id && at_start.collapsed == now.collapsed)
    }
}

impl ChartCard {
    /// Lets the pointer drag the line above each expanded pane. Runs before the chart's own input
    /// so a press on a line is not a pan, a drawing anchor or a hold. Like the pane's own buttons
    /// it takes the pointer whatever tool is armed.
    pub(super) fn handle_pane_dividers(&mut self, ui: &Ui, chart_id: u64, layout: &FrameLayout) {
        if layout.maximized.is_some()
            || layout.heights.main_collapsed
            || self
                .pane_drag
                .as_ref()
                .is_some_and(|drag| !drag.is_on(layout))
        {
            self.pane_drag = None;
        }
        if layout.maximized.is_some() || layout.heights.main_collapsed {
            return;
        }
        let total = layout.full_rect.height();
        for (index, (pane, rect)) in layout.pane_specs.iter().zip(&layout.pane_rects).enumerate() {
            if pane.collapsed {
                continue;
            }
            let grab =
                Rect::from_x_y_ranges(rect.x_range(), (rect.top() - GRAB)..=(rect.top() + GRAB));
            let id = Id::new(("pane-divider", chart_id, pane.id));
            let response = ui.interact(grab, id, Sense::drag());
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
            }
            if response.drag_started() {
                self.pane_drag = Some(PaneDrag {
                    pane: pane.id,
                    total,
                    panes: layout.pane_specs.clone(),
                    start: layout.heights.clone(),
                });
            }
            if response.dragged()
                && let Some(drag) = self.pane_drag.as_ref().filter(|drag| drag.pane == pane.id)
                && let Some(origin) = ui.input(|input| input.pointer.press_origin())
                && let Some(now) = response.interact_pointer_pos()
            {
                let shares =
                    canvas::drag_divider(total, &drag.start, &drag.panes, index, origin.y - now.y);
                for (id, share) in shares {
                    if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
                        instance.pane_share = Some(share);
                    }
                }
                ui.ctx().request_repaint();
            }
            if response.drag_stopped() {
                self.pane_drag = None;
            }
        }
    }
}
