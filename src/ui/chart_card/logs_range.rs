//! The bars the Pine logs pane lists: marking them with a drag on the chart, and the band that
//! shows them afterwards.
//!
//! Like the locked cursor line the band is view state: neither a drawing nor part of linked
//! navigation, never saved, placed through the chart's own projection so it stays on its bars
//! through pans and zooms. The drag reuses the replay picker's plumbing (`ReplayChartFrame`,
//! `PickKind`): the owner turns `selecting` on, this paints the pending band and answers with
//! `picked_range`; the owner stores it in the pane's filter and hands it back through
//! [`ChartCard::set_logs_range`].

use super::{ChartCard, Projection, View};
use crate::model::canvas::COLLAPSED_PANE_HEIGHT;
use crate::ui::canvas_render::CanvasStyle;
use crate::ui::replay_bar;
use crate::ui::replay_overlay::{self, ReplayChartFrame};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Painter, Pos2, Rect, Response, Stroke, Vec2,
};

/// The candle whose slot holds `x`, the nearest end when `x` is past the candles: a drag that
/// leaves the plot still ends on the first or last candle shown.
fn candle_clamped(
    x: f32,
    plot_left: f32,
    slot_width: f32,
    lead: f32,
    start: usize,
    end: usize,
) -> Option<usize> {
    if slot_width <= 0.0 || end == 0 {
        return None;
    }
    let slot = ((x - plot_left) / slot_width - lead).floor() as isize;
    let first = start as isize;
    let last = end as isize - 1;
    if last < first {
        return None;
    }
    Some((slot + first).clamp(first, last) as usize)
}

fn candle_x(index: usize, plot: Rect, view: &View) -> f32 {
    plot.left()
        + view.slot_width * ((index as f32 - view.start as f32) + 0.5)
        + view.slot_width * view.lead
}

/// "Oct 9, 14:03 – 14:45", or "Oct 9, 14:03" for one bar.
fn span_text(from_ms: i64, to_ms: i64) -> String {
    let from = replay_bar::compact_text(from_ms);
    if from_ms == to_ms {
        return from;
    }
    let to = replay_bar::compact_text(to_ms);
    match (from.split_once(", "), to.split_once(", ")) {
        (Some((day, _)), Some((other, time))) if day == other => format!("{from} – {time}"),
        _ => format!("{from} – {to}"),
    }
}

fn paint_tag(painter: &Painter, plot: Rect, centre_x: f32, y: f32, text: String, fill: Color32) {
    let font = FontId::monospace(10.5);
    let galley = painter.layout_no_wrap(text.clone(), font.clone(), Color32::WHITE);
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let x = centre_x.clamp(plot.left() + size.x / 2.0, plot.right() - size.x / 2.0);
    let tag = Rect::from_center_size(Pos2::new(x, y), size);
    painter.rect_filled(tag, 999.0, fill);
    painter.text(
        tag.center(),
        Align2::CENTER_CENTER,
        text,
        font,
        Color32::WHITE,
    );
}

impl View {
    fn candle_clamped(&self, x: f32, plot_left: f32) -> Option<usize> {
        candle_clamped(
            x,
            plot_left,
            self.slot_width,
            self.lead,
            self.start,
            self.end,
        )
    }
}

impl ChartCard {
    /// The bars the logs pane is listing (open times, both ends included), if it is limiting them.
    #[cfg(test)]
    pub fn logs_range(&self) -> Option<(i64, i64)> {
        self.logs_range
    }

    /// Shows the band over the bars the logs pane lists, or takes it away.
    pub fn set_logs_range(&mut self, range: Option<(i64, i64)>) {
        self.logs_range = range;
    }

    /// "Drag over the bars" mode: the candle under the pointer is outlined, a drag paints the band
    /// from where it began, and letting go (or a click on one candle) answers with the open times
    /// of its ends in `replay.picked_range`. Panning is off meanwhile, see `drawing_claims` in
    /// `body.rs`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_range_pick(
        &self,
        ui: &egui::Ui,
        response: &Response,
        painter: &Painter,
        plot: Rect,
        time_of: &dyn Fn(usize) -> i64,
        view: &View,
        replay: &mut ReplayChartFrame,
    ) {
        let anchor_id = Id::new(("logs-range-anchor", response.id));
        let accent = replay_overlay::range_accent();
        let pointer = response
            .hover_pos()
            .or_else(|| ui.input(|input| input.pointer.latest_pos()));
        let under = pointer
            .filter(|point| plot.contains(*point) || response.dragged() || response.drag_stopped())
            .and_then(|point| view.candle_clamped(point.x, plot.left()));
        let Some(under) = under else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if response.drag_started_by(egui::PointerButton::Primary) {
            let origin = ui
                .input(|input| input.pointer.press_origin())
                .and_then(|point| view.candle_clamped(point.x, plot.left()))
                .unwrap_or(under);
            ui.ctx()
                .data_mut(|data| data.insert_temp(anchor_id, origin));
        }
        let anchor: Option<usize> = ui.ctx().data(|data| data.get_temp(anchor_id));
        let from = anchor.filter(|_| response.dragged() || response.drag_stopped());
        let (first, last) = match from {
            Some(anchor) => (anchor.min(under), anchor.max(under)),
            None => (under, under),
        };
        let half = view.slot_width / 2.0;
        let band = Rect::from_min_max(
            Pos2::new(candle_x(first, plot, view) - half, plot.top()),
            Pos2::new(candle_x(last, plot, view) + half, plot.bottom()),
        )
        .intersect(plot);
        let clipped = painter.with_clip_rect(plot);
        clipped.rect_filled(band, 0.0, accent.gamma_multiply(0.18));
        for x in [band.left(), band.right()] {
            clipped.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.5, accent),
            );
        }
        let text = if from.is_some() {
            format!("Logs · {}", span_text(time_of(first), time_of(last)))
        } else {
            "Logs · drag over the bars".to_owned()
        };
        paint_tag(
            painter,
            plot,
            band.center().x,
            plot.top() + 12.0,
            text,
            accent,
        );

        if response.drag_stopped() {
            if from.is_some() {
                replay.picked_range = Some((time_of(first), time_of(last)));
            }
            ui.ctx().data_mut(|data| data.remove::<usize>(anchor_id));
        } else if response.clicked() {
            replay.picked_range = Some((time_of(under), time_of(under)));
            ui.ctx().data_mut(|data| data.remove::<usize>(anchor_id));
        }
    }

    /// The band over the bars the pane lists, across the plot and every script pane.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_logs_range(
        &self,
        painter: &Painter,
        projection: &Projection,
        plot: Rect,
        pane_rects: &[Rect],
        _style: &CanvasStyle,
        main_visible: bool,
    ) {
        let Some((from_ms, to_ms)) = self.logs_range else {
            return;
        };
        let half = projection.slot_width / 2.0;
        let left = projection.x(from_ms.min(to_ms)) - half;
        let right = projection.x(from_ms.max(to_ms)) + half;
        if !(left.is_finite() && right.is_finite()) || right < plot.left() || left > plot.right() {
            return;
        }
        let accent = replay_overlay::range_accent();
        let mut spans = Vec::new();
        if main_visible {
            spans.push((plot.top(), plot.bottom()));
        }
        spans.extend(
            pane_rects
                .iter()
                .filter(|pane| pane.height() > COLLAPSED_PANE_HEIGHT)
                .map(|pane| (pane.top(), pane.bottom())),
        );
        for (top, bottom) in spans {
            let band = Rect::from_min_max(
                Pos2::new(left.max(plot.left()), top),
                Pos2::new(right.min(plot.right()), bottom),
            );
            painter.rect_filled(band, 0.0, accent.gamma_multiply(0.12));
            for x in [left, right] {
                if x >= plot.left() && x <= plot.right() {
                    painter.line_segment(
                        [Pos2::new(x, top), Pos2::new(x, bottom)],
                        Stroke::new(1.0, accent.gamma_multiply(0.7)),
                    );
                }
            }
        }
        if main_visible {
            let centre = (left.max(plot.left()) + right.min(plot.right())) / 2.0;
            paint_tag(
                painter,
                plot,
                centre,
                plot.bottom() - 12.0,
                "Logs".to_owned(),
                accent,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_candle_is_clamped_to_the_ones_shown() {
        // Slots of 100px, candles 3..8 shown.
        let at = |x| candle_clamped(x, 0.0, 100.0, 0.0, 3, 8);
        assert_eq!(at(-500.0), Some(3));
        assert_eq!(at(50.0), Some(3));
        assert_eq!(at(250.0), Some(5));
        assert_eq!(at(9_000.0), Some(7));
        assert_eq!(candle_clamped(10.0, 0.0, 0.0, 0.0, 3, 8), None);
        assert_eq!(candle_clamped(10.0, 0.0, 100.0, 0.0, 0, 0), None);
    }

    #[test]
    fn a_span_names_the_day_once() {
        let day = 86_400_000;
        let a = 1_700_000_000_000;
        assert!(span_text(a, a).contains(", "));
        let same = span_text(a, a + 3_600_000);
        assert_eq!(same.matches(", ").count(), 1, "{same}");
        let two = span_text(a, a + 2 * day);
        assert_eq!(two.matches(", ").count(), 2, "{two}");
    }
}
