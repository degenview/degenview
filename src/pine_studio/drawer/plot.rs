//! The Report tab's charts: money along a run of steps, with its scale at the right edge (the top,
//! the bottom and the initial capital, which is also a dashed line across), and under the pointer a
//! hairline, a dot on each series and the values there.

use crate::ui::pine::report::PineReportFormat;
use crate::ui::{kit, theme};
use eframe::egui::{Color32, Galley, Painter, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use std::sync::Arc;

/// The room kept above and below the series, so the top and bottom labels centre on its extremes.
const INSET: f32 = 6.0;
/// The space between the series and its scale.
const SCALE_GAP: f32 = 8.0;
/// The padding inside the readout and its distance from the hairline.
const READOUT_PAD: f32 = 6.0;
const READOUT_OFFSET: f32 = 8.0;

/// Green when `end` finished at or above the capital, red below: the colour of a chart's series.
pub(super) fn tone(end: f64, capital: f64) -> Color32 {
    if end >= capital {
        theme::WIN
    } else {
        theme::LOSS
    }
}

/// A chart laid out in the width it was given: where each step and value lands.
pub(super) struct Plot {
    painter: Painter,
    area: Rect,
    low: f64,
    span: f64,
    last: usize,
}

impl Plot {
    /// Allocates a chart `height` tall across the width for `steps` points (at least two) whose
    /// values lie in `low..=high`, paints its scale and the capital's dashed line, and returns it
    /// with the step under the pointer.
    pub(super) fn show(
        ui: &mut Ui,
        height: f32,
        steps: usize,
        low: f64,
        high: f64,
        capital: f64,
    ) -> (Self, Option<usize>) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(rect);
        let (low, high) = (low.min(capital), high.max(capital));
        let span = high - low;
        let label = |value: f64, color: Color32| {
            kit::galley_on(
                &painter,
                &PineReportFormat::axis(value, span),
                kit::size::CAPTION,
                kit::REGULAR,
                color,
                f32::INFINITY,
            )
        };
        let top = label(high, theme::text_tertiary());
        let bottom = label(low, theme::text_tertiary());
        let base = label(capital, theme::text_secondary());
        let gutter = top.size().x.max(bottom.size().x).max(base.size().x) + SCALE_GAP;
        let area = Rect::from_min_max(
            pos2(rect.left(), rect.top() + INSET),
            pos2(rect.right() - gutter, rect.bottom() - INSET),
        );
        let plot = Self {
            painter,
            area,
            low,
            span: span.max(1e-9),
            last: steps.max(2) - 1,
        };

        let base_y = plot.y(capital);
        plot.painter.extend(Shape::dashed_line(
            &[pos2(area.left(), base_y), pos2(area.right(), base_y)],
            Stroke::new(1.0, theme::with_alpha(theme::text_secondary(), 0.5)),
            3.0,
            3.0,
        ));
        let clear_of_base = |y: f32, galley: &Galley| (y - base_y).abs() >= galley.size().y;
        for (y, galley) in [(area.top(), top), (area.bottom(), bottom)] {
            if clear_of_base(y, &galley) {
                plot.scale_label(y, galley);
            }
        }
        plot.scale_label(base_y, base);

        let hovered = response
            .hover_pos()
            .filter(|pointer| pointer.x <= area.right())
            .map(|pointer| plot.step_at(pointer.x));
        (plot, hovered)
    }

    pub(super) fn x(&self, step: usize) -> f32 {
        self.area.left() + self.area.width() * step as f32 / self.last as f32
    }

    pub(super) fn y(&self, value: f64) -> f32 {
        self.area.bottom() - ((value - self.low) / self.span) as f32 * self.area.height()
    }

    /// The step nearest to `x`.
    fn step_at(&self, x: f32) -> usize {
        let fraction = ((x - self.area.left()) / self.area.width()).clamp(0.0, 1.0);
        (fraction * self.last as f32).round() as usize
    }

    pub(super) fn painter(&self) -> &Painter {
        &self.painter
    }

    /// A series as a line through its values, one per step.
    pub(super) fn line(&self, values: impl Iterator<Item = f64>, color: Color32) {
        let points = values
            .enumerate()
            .map(|(step, value)| pos2(self.x(step), self.y(value)))
            .collect();
        self.painter
            .add(Shape::line(points, Stroke::new(1.5, color)));
    }

    /// What the pointer is over at `step`: a hairline down the chart, a dot at each of `dots`, and
    /// `lines` in a small box beside the hairline (the first line stronger), on whichever side has
    /// room and in whichever half the first dot leaves free.
    pub(super) fn readout(&self, step: usize, dots: &[(f64, Color32)], lines: &[String]) {
        let x = self.x(step);
        let painter = &self.painter;
        painter.vline(
            x,
            self.area.top() - INSET..=self.area.bottom() + INSET,
            Stroke::new(1.0, theme::with_alpha(theme::text_secondary(), 0.6)),
        );
        for &(value, color) in dots {
            let at = pos2(x, self.y(value));
            painter.circle_filled(at, 4.0, theme::background());
            painter.circle_filled(at, 2.75, color);
        }
        let room = self.area.width() - READOUT_OFFSET - READOUT_PAD * 2.0;
        let galleys: Vec<Arc<Galley>> = lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let (weight, color) = if index == 0 {
                    (kit::MEDIUM, theme::text_primary())
                } else {
                    (kit::REGULAR, theme::text_secondary())
                };
                kit::galley_on(painter, line, kit::size::CAPTION, weight, color, room)
            })
            .collect();
        let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
        let height: f32 = galleys.iter().map(|g| g.size().y).sum();
        let size = vec2(width, height) + vec2(READOUT_PAD * 2.0, READOUT_PAD);
        let right_side = x + READOUT_OFFSET + size.x <= self.area.right();
        let left = if right_side {
            x + READOUT_OFFSET
        } else {
            (x - READOUT_OFFSET - size.x).max(self.area.left())
        };
        let dot_in_top_half = dots
            .first()
            .is_some_and(|&(value, _)| self.y(value) < self.area.center().y);
        let top = if dot_in_top_half {
            self.area.bottom() + INSET - size.y
        } else {
            self.area.top() - INSET
        };
        let frame = Rect::from_min_size(pos2(left, top), size);
        painter.rect_filled(frame, 5.0, theme::background());
        painter.rect_stroke(
            frame,
            5.0,
            Stroke::new(1.0, kit::hairline()),
            eframe::egui::StrokeKind::Inside,
        );
        let mut y = frame.top() + READOUT_PAD / 2.0;
        for galley in galleys {
            let height = galley.size().y;
            painter.galley(
                pos2(frame.left() + READOUT_PAD, y),
                galley,
                Color32::PLACEHOLDER,
            );
            y += height;
        }
    }

    /// A scale label right-aligned at the chart's edge, centred on `y`.
    fn scale_label(&self, y: f32, galley: Arc<Galley>) {
        let size = galley.size();
        let right = self.painter.clip_rect().right();
        self.painter.galley(
            pos2(right - size.x, y - size.y / 2.0),
            galley,
            Color32::PLACEHOLDER,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn steps_and_values_span_the_area_and_the_pointer_finds_the_nearest_step() {
        egui::__run_test_ui(|ui| {
            let (plot, _) = Plot::show(ui, 80.0, 11, 900.0, 1_200.0, 1_000.0);
            assert_eq!(plot.x(0), plot.area.left());
            assert_eq!(plot.x(10), plot.area.right());
            assert!((plot.y(900.0) - plot.area.bottom()).abs() < 1e-3);
            assert!((plot.y(1_200.0) - plot.area.top()).abs() < 1e-3);
            let tenth = plot.area.width() / 10.0;
            assert_eq!(plot.step_at(plot.area.left() + tenth * 3.4), 3);
            assert_eq!(plot.step_at(plot.area.left() + tenth * 3.6), 4);
            assert_eq!(plot.step_at(plot.area.right() + 50.0), 10);
        });
    }

    #[test]
    fn the_capital_is_always_on_the_scale() {
        egui::__run_test_ui(|ui| {
            let (plot, _) = Plot::show(ui, 80.0, 5, 1_100.0, 1_300.0, 1_000.0);
            assert!((plot.y(1_000.0) - plot.area.bottom()).abs() < 1e-3);
        });
    }

    #[test]
    fn a_series_finishing_below_the_capital_is_red() {
        assert_eq!(tone(1_010.0, 1_000.0), theme::WIN);
        assert_eq!(tone(1_000.0, 1_000.0), theme::WIN);
        assert_eq!(tone(990.0, 1_000.0), theme::LOSS);
    }
}
