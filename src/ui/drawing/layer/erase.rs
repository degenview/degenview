//! The Eraser cursor on the drawing layer: a press on the plot starts a sweep, every drawing the
//! pointer passes over is marked (drawn faint), and letting go removes them all as one batch of
//! `DrawingCommand::Remove`, so a sweep is one undo step and a click is a sweep of one.
//!
//! What can be erased is exactly what can be picked: the same `hit_test` as selection, so a
//! hidden drawing, another timeframe's, `Unreadable` rows and anything that is not a drawing
//! (candles, scripts, alerts, orders) are out of reach. A locked drawing is protected.

use super::*;

/// The furthest apart (px) the points tested along a sweep lie, so a quick drag cannot step over a
/// thin line.
const SAMPLE_STEP: f32 = 4.0;

/// A sweep in progress: where the pointer was last tested and what it passed over.
pub(super) struct Sweep {
    last: Pos2,
    pub(super) ids: Vec<u64>,
}

/// A primary press that began in one frame.
struct Press {
    /// Where the button went down.
    origin: Pos2,
    /// The button also came up in the same frame.
    released: bool,
}

/// The first primary press among a frame's raw events, and whether a release followed it.
fn primary_press(events: &[egui::Event]) -> Option<Press> {
    let mut press: Option<Press> = None;
    for event in events {
        if let egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            ..
        } = event
        {
            match (&mut press, pressed) {
                (None, true) => {
                    press = Some(Press {
                        origin: *pos,
                        released: false,
                    })
                }
                (Some(press), false) => press.released = true,
                _ => {}
            }
        }
    }
    press
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
        let (down, press, escape, focused) = ui.input(|input| {
            (
                input.pointer.primary_down(),
                primary_press(&input.events),
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
                // The button came up this frame, possibly with the last of the move: that stretch
                // is tested too, or what it crossed would survive.
                if let Some(point) = pointer {
                    self.sweep_to(&mut sweep, proj.clamp(point), frame, proj);
                }
                frame
                    .commands
                    .extend(sweep.ids.into_iter().map(DrawingCommand::Remove));
            }
            return;
        }

        // A press that began this frame, from the frame's own events: its position is not the
        // pointer's (the move can arrive with it) and a press and release sharing the frame leave
        // no press origin and no held button for the response to report.
        if let Some(press) = press
            && proj.plot.contains(press.origin)
            && !self.cancelled_drag
            && (response.is_pointer_button_down_on()
                // Over before the frame was out: the plot's own gesture if the pointer ended on the
                // plot and no widget over it (an alert tag, an order pill) took the press.
                || (press.released && response.hovered()))
        {
            let mut sweep = Sweep {
                last: press.origin,
                ids: Vec::new(),
            };
            self.sweep_to(&mut sweep, press.origin, frame, proj);
            if let Some(now) = pointer {
                self.sweep_to(&mut sweep, proj.clamp(now), frame, proj);
            }
            if press.released {
                frame
                    .commands
                    .extend(sweep.ids.into_iter().map(DrawingCommand::Remove));
            } else {
                self.sweep = Some(sweep);
            }
            return;
        }
        let Some(point) = inside.filter(|_| response.hovered()) else {
            return;
        };
        match self.hit_test(frame, point, proj, false) {
            Some(hit) if hit.locked => self.erase_blocked = true,
            hit => self.hover = hit,
        }
        if response.secondary_clicked() {
            // The drawing menu belongs to the pointer; the chart's own menu stays reachable.
            frame.background_click = Some(point);
        }
    }

    /// Tests the pointer's way from where the sweep last was to `to`, marking each unlocked
    /// drawing it crosses once.
    fn sweep_to(&mut self, sweep: &mut Sweep, to: Pos2, frame: &DrawingFrame, proj: &Projection) {
        let from = sweep.last;
        // Never further apart than `SAMPLE_STEP`, however far it moved: the pointer is clamped to
        // the plot, so even the fastest frame is a few hundred points, each over the cached shapes.
        let steps = (((to - from).length() / SAMPLE_STEP).ceil() as usize).max(1);
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
