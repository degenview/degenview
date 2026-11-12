//! The locked vertical cursor line: a line pinned to a time, kept when the pointer moves on.
//!
//! It belongs to the chart (view state, like the maximized pane), is placed through the chart's
//! own projection so it stays on its time through pans, zooms and resizes, and is clipped to the
//! plot and to each script pane. It is neither a drawing nor part of linked-chart navigation.

use super::{ChartCard, Projection};
use crate::format;
use crate::model::chart::canvas::COLLAPSED_PANE_HEIGHT;
use crate::ui::chart::canvas::{self, CanvasStyle};
use eframe::egui::{Align2, Color32, Painter, Pos2, Rect, Vec2};

impl ChartCard {
    /// The time the vertical cursor line is locked at, if it is.
    pub fn time_lock(&self) -> Option<i64> {
        self.time_lock
    }

    /// Locks the line at `time_ms`, or releases it.
    pub fn set_time_lock(&mut self, time_ms: Option<i64>) {
        self.time_lock = time_ms;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_time_lock(
        &self,
        painter: &Painter,
        projection: &Projection,
        plot: Rect,
        rect: Rect,
        pane_rects: &[Rect],
        style: &CanvasStyle,
        main_visible: bool,
    ) {
        let Some(time_ms) = self.time_lock else {
            return;
        };
        let x = projection.x(time_ms);
        if !x.is_finite() || x < plot.left() || x > plot.right() {
            return;
        }
        let line = style.crosshair;
        if main_visible {
            canvas::vertical(painter, x, (plot.top(), plot.bottom()), line);
        }
        for pane in pane_rects {
            if pane.height() > COLLAPSED_PANE_HEIGHT {
                canvas::vertical(painter, x, (pane.top(), pane.bottom()), line);
            }
        }
        let width = 116.0 * style.time_size / 9.0;
        let tag = Rect::from_center_size(
            Pos2::new(x, rect.bottom() - style.time_axis_height() / 2.0),
            Vec2::new(width, style.time_axis_height() - 6.0),
        )
        .intersect(Rect::from_min_max(
            Pos2::new(plot.left(), plot.bottom()),
            Pos2::new(plot.right(), rect.bottom()),
        ));
        painter.rect_filled(tag, 3.0, Color32::from_gray(79));
        painter.text(
            tag.center(),
            Align2::CENTER_CENTER,
            format::timestamp(time_ms),
            style.time_font(),
            Color32::WHITE,
        );
    }
}
