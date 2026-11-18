//! The Eraser cursor on the drawing layer: a press on the plot starts a sweep, every drawing the
//! pointer passes over is marked (drawn faint), and letting go removes them all as one batch of
//! `DrawingCommand::Remove`, so a sweep is one undo step and a click is a sweep of one.
//!
//! What can be erased is exactly what can be picked: the same `hit_test` as selection, so a
//! hidden drawing, another timeframe's, `Unreadable` rows and anything that is not a drawing
//! (candles, scripts, alerts, orders) are out of reach. A locked drawing is protected.

use super::*;

/// How far apart (px) the points tested along a sweep lie, so a quick drag cannot step over a
/// thin line.
const SAMPLE_STEP: f32 = 4.0;
/// Most points tested for the stretch one frame covers.
const MAX_SAMPLES: usize = 64;

/// A sweep in progress: where the pointer was last tested and what it passed over.
pub(super) struct Sweep {
    last: Pos2,
    pub(super) ids: Vec<u64>,
}

impl DrawingLayer {
    /// One frame of the Eraser. Takes the place of selecting, editing and drawing. `unseen` is
    /// that the card was not drawn since the last frame (another tab showed): a sweep begun
    /// before that is dropped, as a gesture is, so a release made while away erases nothing.
    pub(super) fn update_eraser(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        proj: &Projection,
        frame: &mut DrawingFrame,
        pointer: Option<Pos2>,
        unseen: bool,
    ) {
        let (down, pressed, escape, focused) = ui.input(|input| {
            (
                input.pointer.primary_down(),
                input.pointer.primary_pressed(),
                input.key_pressed(egui::Key::Escape),
                input.focused,
            )
        });
        self.gesture = None;
        self.hover = None;
        self.erase_blocked = false;
        if !down {
            self.cancelled_drag = false;
        }
        let inside = pointer.filter(|point| proj.plot.contains(*point));

        if let Some(mut sweep) = self.sweep.take() {
            if escape || !focused || unseen {
                // Cancelled: nothing is erased, and the rest of the press does not pan.
                frame.escape_consumed |= escape;
                self.cancelled_drag = down;
            } else if down {
                if let Some(point) = pointer {
                    self.sweep_to(&mut sweep, proj.clamp(point), frame, proj);
                }
                self.sweep = Some(sweep);
            } else {
                frame
                    .commands
                    .extend(sweep.ids.into_iter().map(DrawingCommand::Remove));
            }
            return;
        }

        let Some(point) = inside.filter(|_| response.hovered()) else {
            return;
        };
        if pressed && !self.cancelled_drag && response.is_pointer_button_down_on() {
            let mut sweep = Sweep {
                last: point,
                ids: Vec::new(),
            };
            self.sweep_to(&mut sweep, point, frame, proj);
            self.sweep = Some(sweep);
            return;
        }
        match self.hit_test(frame, point, proj, false) {
            Some(hit) if hit.locked => self.erase_blocked = true,
            hit => self.hover = hit,
        }
        if response.secondary_clicked() {
            // The drawing menu belongs to the pointer; the chart's own menu stays reachable.
            frame.background_click = Some(point);
        }
    }

    /// One frame with the drawings out of reach (Demonstration): nothing is hovered, picked or
    /// edited, and a right-click still opens the chart's own menu.
    pub(super) fn update_inert(
        &mut self,
        response: &Response,
        proj: &Projection,
        frame: &mut DrawingFrame,
        pointer: Option<Pos2>,
    ) {
        self.gesture = None;
        self.hover = None;
        self.cancelled_drag = false;
        if response.secondary_clicked()
            && let Some(point) = pointer.filter(|point| proj.plot.contains(*point))
        {
            frame.background_click = Some(point);
        }
    }

    /// Tests the pointer's way from where the sweep last was to `to`, marking each unlocked
    /// drawing it crosses once.
    fn sweep_to(&mut self, sweep: &mut Sweep, to: Pos2, frame: &DrawingFrame, proj: &Projection) {
        let from = sweep.last;
        let steps = ((to - from).length() / SAMPLE_STEP).ceil() as usize;
        let steps = steps.clamp(1, MAX_SAMPLES);
        for step in 1..=steps {
            let at = from + (to - from) * (step as f32 / steps as f32);
            if let Some(hit) = self.hit_test(frame, at, proj, false)
                && !hit.locked
                && !sweep.ids.contains(&hit.id)
            {
                sweep.ids.push(hit.id);
            }
        }
        sweep.last = to;
    }

    /// Whether `id` is marked by the sweep in progress.
    pub(super) fn is_marked_for_erasing(&self, id: u64) -> bool {
        self.sweep
            .as_ref()
            .is_some_and(|sweep| sweep.ids.contains(&id))
    }
}
