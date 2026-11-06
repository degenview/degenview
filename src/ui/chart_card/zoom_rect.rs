//! The Zoom In tool on a chart: drag a rectangle over the plot and the view becomes it.
//!
//! The tool is a mode the workspace arms (`ChartCard::set_zoom_armed`), not a drawing: the
//! rectangle is transient card state that is never stored, never in the Object tree, the undo
//! stack, a template or linked navigation. The maths is `model::viewport::zoom`; this file owns the
//! pointer, the preview and the viewport edit. The edit is made inside the frame, so linked charts
//! follow through the usual `end_sync_frame` diff and the price range stays local.

use eframe::egui::{self, CursorIcon, Painter, Pos2, Rect, Response, Stroke, StrokeKind};

use super::navigation::{Region, Regions};
use super::{ChartCard, View};
use crate::model::viewport::ChartViewport;
use crate::model::viewport::zoom::{self, MAX_BACK, Selection, Slots};
use crate::ui::kit;

/// The rectangle being dragged.
#[derive(Clone, Copy, Debug)]
struct Drag {
    origin: Pos2,
    /// Escape, a second button or a lost window ended it: the rest of the drag does nothing.
    cancelled: bool,
}

/// Everything the Zoom In tool keeps on a card. View state: never saved.
#[derive(Default)]
pub(super) struct ZoomTool {
    /// The workspace's mode is on, so a drag on the plot selects instead of panning or drawing.
    armed: bool,
    drag: Option<Drag>,
    /// The views zoomed away from, newest last: what Zoom out goes back to.
    back: Vec<ChartViewport>,
    /// A zoom was applied since the workspace last asked; it puts the tool down.
    zoomed: bool,
    /// The frame this card was last drawn in, to notice it went unseen (another tab).
    last_frame: u64,
}

impl ChartCard {
    /// Arms or disarms the Zoom In mode. Disarming drops a half-made rectangle.
    pub fn set_zoom_armed(&mut self, armed: bool) {
        self.zoom.armed = armed;
        if !armed {
            self.zoom.drag = None;
        }
    }

    /// Whether a zoom was applied since the last call. The workspace puts the tool down on it.
    pub fn take_zoomed(&mut self) -> bool {
        std::mem::take(&mut self.zoom.zoomed)
    }

    /// Whether Zoom out has a view to go back to.
    pub fn can_zoom_out(&self) -> bool {
        !self.zoom.back.is_empty()
    }

    /// Goes back to the view the last zoom came from. Made outside a frame, so the card remembers
    /// where the view was and the next frame tells linked charts about the change.
    pub fn zoom_out(&mut self) {
        if let Some(before) = self.zoom.back.pop() {
            self.scroll_from.get_or_insert(self.viewport.horizontal);
            self.viewport = before;
        }
    }

    /// Forgets the views to zoom out to: they mean nothing on another series.
    pub(super) fn forget_zoom_history(&mut self) {
        self.zoom.back.clear();
        self.zoom.drag = None;
    }

    /// Whether a rectangle is being dragged, which is not a pan.
    pub(super) fn zoom_claims_drag(&self) -> bool {
        self.zoom.drag.is_some()
    }

    /// Whether the tool answers the pointer on this chart this frame: armed, with a main plot to
    /// zoom and nothing else (the replay start picker) asking for the pointer.
    pub(super) fn zoom_is_live(&self, main_visible: bool, picking: bool) -> bool {
        self.zoom.armed && main_visible && !picking
    }

    /// One frame of the tool's pointer: starts, cancels and ends the rectangle. Returns whether
    /// an Escape was spent cancelling it, so the workspace's own Escape does not also put the tool
    /// down.
    pub(super) fn handle_zoom(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        view: Option<&View>,
        regions: &Regions,
        total: usize,
        live: bool,
    ) -> bool {
        let primary = egui::PointerButton::Primary;
        let (focused, pointer, press, primary_down, secondary_down, escape) = ui.input(|input| {
            (
                input.focused,
                input.pointer.latest_pos(),
                input.pointer.press_origin(),
                input.pointer.primary_down(),
                input.pointer.secondary_down(),
                input.key_pressed(egui::Key::Escape),
            )
        });
        // A rectangle must not outlive what it began under: the card going unseen (another tab),
        // the window losing focus, or the tool being put down.
        let frame_nr = ui.ctx().cumulative_frame_nr();
        if self.zoom.drag.is_some() && (!focused || !live || frame_nr > self.zoom.last_frame + 1) {
            self.zoom.drag = None;
        }
        self.zoom.last_frame = frame_nr;
        if !live {
            return false;
        }

        // The cursor says what a drag does here: over the plot, or while one is under way.
        if self.zoom.drag.is_some()
            || response
                .hover_pos()
                .is_some_and(|point| regions.of(point) == Region::Plot)
        {
            ui.ctx().set_cursor_icon(CursorIcon::ZoomIn);
        }

        if response.drag_started_by(primary)
            && self.gesture.is_none()
            && let Some(origin) = press.filter(|point| regions.of(*point) == Region::Plot)
        {
            self.zoom.drag = Some(Drag {
                origin,
                cancelled: false,
            });
        }

        let mut consumed = false;
        if let Some(drag) = self.zoom.drag.as_mut()
            && !drag.cancelled
        {
            if escape {
                drag.cancelled = true;
                consumed = true;
            } else if secondary_down {
                drag.cancelled = true;
            }
        }

        if response.drag_stopped_by(primary) {
            if let Some(drag) = self.zoom.drag.take()
                && !drag.cancelled
                && let Some(view) = view
            {
                let end = pointer.unwrap_or(drag.origin);
                let corners = Selection::between(
                    (drag.origin.x, drag.origin.y),
                    (end.x, end.y),
                    (
                        regions.plot.left(),
                        regions.plot.top(),
                        regions.plot.right(),
                        regions.plot.bottom(),
                    ),
                );
                self.apply_zoom(&corners, view, regions.plot, total);
            }
        } else if self.zoom.drag.is_some() && !primary_down {
            // The button went up somewhere egui did not report it.
            self.zoom.drag = None;
        }
        consumed
    }

    /// Makes `selection` the view: the bars across it, and the prices between its edges. A
    /// rectangle too narrow to be a choice changes nothing (the tool stays armed); one too flat
    /// zooms the time axis only, and so does one whose prices cannot be drawn.
    fn apply_zoom(&mut self, selection: &Selection, view: &View, plot: Rect, total: usize) {
        let slots = Slots {
            first: view.start as f64 - f64::from(view.lead),
            left: f64::from(plot.left()),
            width: f64::from(view.slot_width),
        };
        let Some(horizontal) = zoom::horizontal(selection, slots, total, self.reserved_slots())
        else {
            return;
        };
        let prices = selection
            .spans_price()
            .then(|| {
                zoom::price_range(
                    view.scale.price(selection.top),
                    view.scale.price(selection.bottom),
                    view.scale.logarithmic,
                )
            })
            .flatten();

        if self.zoom.back.len() >= MAX_BACK {
            self.zoom.back.remove(0);
        }
        self.zoom.back.push(self.viewport);
        self.viewport.horizontal = horizontal;
        if let Some(range) = prices {
            self.viewport.vertical.manual = Some(range);
        }
        self.zoom.zoomed = true;
    }

    /// The rectangle being dragged, over the plot. Painted by the live overlays, above the
    /// drawings and below every menu, in the neutral ink the rest of the chrome uses.
    pub(super) fn paint_zoom(&self, ui: &egui::Ui, painter: &Painter, plot: Rect) {
        let Some(drag) = self.zoom.drag.filter(|drag| !drag.cancelled) else {
            return;
        };
        let end = ui
            .input(|input| input.pointer.latest_pos())
            .unwrap_or(drag.origin);
        let corners = Selection::between(
            (drag.origin.x, drag.origin.y),
            (end.x, end.y),
            (plot.left(), plot.top(), plot.right(), plot.bottom()),
        );
        let rect = Rect::from_min_max(
            Pos2::new(corners.left, corners.top),
            Pos2::new(corners.right, corners.bottom),
        );
        let painter = painter.with_clip_rect(plot);
        painter.rect_filled(rect, 0.0, kit::primary(0.10));
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, kit::primary(0.55)),
            StrokeKind::Inside,
        );
    }
}
