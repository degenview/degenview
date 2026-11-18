//! The Demonstration cursor on a card: dots and freehand strokes for explaining a chart, which stay
//! a moment and fade away (`model::cursor::demo`).
//!
//! The marks are the card's view state, painted over the live plot only: never a drawing, never
//! saved, never in a snapshot (`paint_snapshot` has no pointer layer), and gone when the cursor
//! changes or the card shows another market.

use super::*;
use crate::model::cursor::demo::{Mark, MarkShape, Marks};
use crate::ui::kit;
use std::time::Duration;

const STROKE_WIDTH: f32 = 4.0;
const DOT_RADIUS: f32 = 6.0;
/// The most a repaint is asked for while marks fade, about one frame at 60 Hz.
const FADE_REPAINT: Duration = Duration::from_millis(16);

impl ChartCard {
    /// One frame of the cursor's input: Alt (Option) with a click puts a dot, a press and drag
    /// draws a stroke, and the marks that have faded are dropped. Takes the plot drag while a
    /// stroke is drawn (`demonstration_claims_drag`). Painting is `paint_demonstration`, after the
    /// chart has painted: the input pass runs before it, so anything drawn here would be under the
    /// candles and a script's background.
    pub(super) fn handle_demonstration(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        proj: &Projection,
        plot: Rect,
    ) {
        if !self.cursor.caps().highlights {
            self.demo.clear();
            return;
        }
        let primary = egui::PointerButton::Primary;
        let (now, pointer, press, down, alt, focused) = ui.input(|input| {
            (
                input.time,
                input.pointer.latest_pos(),
                input.pointer.press_origin(),
                input.pointer.primary_down(),
                input.modifiers.alt,
                input.focused,
            )
        });
        if response.drag_started_by(primary)
            && let Some(origin) = press.filter(|point| plot.contains(*point))
        {
            self.demo.begin(proj.anchor(origin), now);
        }
        if response.dragged_by(primary)
            && self.demo.is_drawing()
            && let Some(point) = pointer
        {
            self.demo.extend(proj.anchor(plot.clamp(point)));
        }
        if self.demo.is_drawing() && (!down || !focused || response.drag_stopped_by(primary)) {
            self.demo.end(now);
        }
        if alt
            && response.clicked_by(primary)
            && let Some(point) = response
                .interact_pointer_pos()
                .filter(|point| plot.contains(*point))
        {
            self.demo.dot(proj.anchor(point), now);
        }
        self.demo.prune(now);
        // Frames only while something changes: every one while drawing or fading, one at the end
        // of the hold otherwise.
        match self.demo.next_change(now) {
            Some(wait) if wait <= 0.0 => ui.ctx().request_repaint_after(FADE_REPAINT),
            Some(wait) => ui
                .ctx()
                .request_repaint_after(Duration::from_secs_f64(wait)),
            None => {}
        }
    }

    /// A stroke is being drawn, so the chart must not pan under it.
    pub(super) fn demonstration_claims_drag(&self) -> bool {
        self.demo.is_drawing()
    }

    /// Paints the marks over everything the chart painted, clipped to the plot, at their place
    /// with their fade. Called once the frame is painted.
    pub(super) fn paint_demonstration(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        proj: &Projection,
        now: f64,
    ) {
        let painter = painter.with_clip_rect(plot);
        for mark in self.demo.marks() {
            paint_mark(&painter, proj, mark, Marks::alpha(mark, now));
        }
    }
}

fn paint_mark(painter: &egui::Painter, proj: &Projection, mark: &Mark, alpha: f32) {
    let ink = kit::ORANGE.gamma_multiply(alpha);
    match &mark.shape {
        MarkShape::Dot(at) => {
            painter.circle_filled(proj.point(*at), DOT_RADIUS, ink);
        }
        MarkShape::Stroke(points) => {
            let screen: Vec<Pos2> = points.iter().map(|point| proj.point(*point)).collect();
            if let [only] = screen[..] {
                painter.circle_filled(only, STROKE_WIDTH / 2.0, ink);
                return;
            }
            // Round ends, so a short stroke is a pill and not a stub.
            for end in [screen.first(), screen.last()].into_iter().flatten() {
                painter.circle_filled(*end, STROKE_WIDTH / 2.0, ink);
            }
            painter.add(egui::Shape::line(screen, Stroke::new(STROKE_WIDTH, ink)));
        }
    }
}
