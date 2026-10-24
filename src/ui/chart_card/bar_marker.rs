//! Scrolling the chart to a bar, and the marker that says which bar it is.
//!
//! The Pine logs pane's "Scroll to bar" uses this. Like the locked cursor line, the marker is view
//! state: neither a drawing nor part of linked navigation, never saved, and gone after a few
//! seconds. The scroll itself only moves the viewport, so it never reruns a script or touches a
//! drawing; linked charts follow through the same time-range sync as any other pan.

use super::{ChartCard, Projection};
use crate::format;
use crate::model::Kline;
use crate::model::Timeframe;
use crate::model::canvas::COLLAPSED_PANE_HEIGHT;
use crate::ui::canvas_render::{self, CanvasStyle};
use crate::ui::theme;
use eframe::egui::{Align2, Color32, Painter, Pos2, Rect, Stroke, Vec2};
use std::time::{Duration, Instant};

/// How long the marker stays up.
pub const MARKER_LIFETIME: Duration = Duration::from_secs(5);

/// How a scroll to a time ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scrolled {
    /// The bar holding the time is on screen, marked.
    Done,
    /// The time is before the oldest bar held: the view went to that bar, and older history may
    /// bring the time within reach.
    BeforeHistory,
    /// There are no bars.
    NoBars,
}

/// The label a marked bar carries: the date for a daily or longer bar, the date and time below.
pub fn marker_label(open_time_ms: i64, timeframe: Option<Timeframe>) -> String {
    let full = format::timestamp(open_time_ms);
    match timeframe {
        Some(Timeframe::Hour) | None => full,
        Some(_) => full.split(' ').next().unwrap_or(&full).to_owned(),
    }
}

impl ChartCard {
    /// Scrolls so the bar of `candles` holding `time_ms` sits in the middle of the view, and marks
    /// it. A time past the newest bar means the newest.
    pub fn scroll_to_time(&mut self, time_ms: i64, candles: &[Kline]) -> Scrolled {
        let total = candles.len();
        let Some(first) = candles.first() else {
            return Scrolled::NoBars;
        };
        let horizontal = &mut self.viewport.horizontal;
        if time_ms < first.open_time {
            horizontal.right_offset = horizontal.offset_limits(total).1;
            return Scrolled::BeforeHistory;
        }
        let index = candles
            .partition_point(|candle| candle.open_time <= time_ms)
            .saturating_sub(1);
        horizontal.right_offset = (total - 1 - index) as f64 - horizontal.visible as f64 / 2.0;
        horizontal.clamp(total);
        self.bar_marker = Some((candles[index].open_time, Instant::now()));
        Scrolled::Done
    }

    /// How many bars the view is scrolled back from the newest.
    #[cfg(test)]
    pub fn scrolled_back(&self) -> f64 {
        self.viewport.horizontal.right_offset
    }

    /// The open time of the bar the marker stands on, while it is up.
    #[cfg(test)]
    pub fn bar_marker(&self) -> Option<i64> {
        self.bar_marker
            .filter(|(_, since)| since.elapsed() < MARKER_LIFETIME)
            .map(|(time_ms, _)| time_ms)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_bar_marker(
        &self,
        painter: &Painter,
        projection: &Projection,
        plot: Rect,
        rect: Rect,
        pane_rects: &[Rect],
        style: &CanvasStyle,
        main_visible: bool,
    ) {
        let Some((time_ms, since)) = self.bar_marker else {
            return;
        };
        let age = since.elapsed();
        if age >= MARKER_LIFETIME {
            return;
        }
        painter
            .ctx()
            .request_repaint_after(MARKER_LIFETIME - age + Duration::from_millis(20));
        let x = projection.x(time_ms);
        if !x.is_finite() || x < plot.left() || x > plot.right() {
            return;
        }
        let accent = theme::SYSTEM_ACCENT;
        let line = canvas_render::LineStyle {
            stroke: Stroke::new(1.5, accent),
            dash: crate::model::chart_type::LineDash::Solid,
        };
        if main_visible {
            canvas_render::vertical(painter, x, (plot.top(), plot.bottom()), line);
        }
        for pane in pane_rects {
            if pane.height() > COLLAPSED_PANE_HEIGHT {
                canvas_render::vertical(painter, x, (pane.top(), pane.bottom()), line);
            }
        }
        let label = marker_label(time_ms, self.timeframe);
        let width = (label.chars().count() as f32 * 6.4 + 20.0) * style.time_size / 9.0;
        let tag = Rect::from_center_size(
            Pos2::new(x, rect.bottom() - style.time_axis_height() / 2.0),
            Vec2::new(width, style.time_axis_height() - 6.0),
        )
        .intersect(Rect::from_min_max(
            Pos2::new(plot.left(), plot.bottom()),
            Pos2::new(plot.right(), rect.bottom()),
        ));
        painter.rect_filled(tag, 3.0, accent);
        painter.text(
            tag.center(),
            Align2::CENTER_CENTER,
            label,
            style.time_font(),
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candles(count: i64) -> Vec<Kline> {
        (0..count)
            .map(|index| Kline {
                open_time: index * 3_600_000,
                close_time: (index + 1) * 3_600_000 - 1,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.5,
                volume: 1.0,
            })
            .collect()
    }

    #[test]
    fn a_time_inside_the_bars_centres_its_bar_and_marks_it() {
        let bars = candles(500);
        let mut card = ChartCard::default();
        // 250.5 hours in: the bar that opened at hour 250.
        let outcome = card.scroll_to_time(250 * 3_600_000 + 1_800_000, &bars);
        assert_eq!(outcome, Scrolled::Done);
        let horizontal = card.viewport.horizontal;
        let centre = (499.0 - horizontal.right_offset) - horizontal.visible as f64 / 2.0;
        assert!((centre - 250.0).abs() < 1.0, "centred on {centre}");
        assert_eq!(card.bar_marker(), Some(250 * 3_600_000));
    }

    #[test]
    fn a_time_past_the_newest_bar_means_the_newest() {
        let bars = candles(100);
        let mut card = ChartCard::default();
        assert_eq!(
            card.scroll_to_time(10_000 * 3_600_000, &bars),
            Scrolled::Done
        );
        assert_eq!(card.bar_marker(), Some(99 * 3_600_000));
        assert!(card.viewport.horizontal.right_offset <= 0.0);
    }

    #[test]
    fn a_time_before_the_oldest_bar_goes_to_the_oldest_and_says_so() {
        let bars = candles(500);
        let mut card = ChartCard::default();
        assert_eq!(card.scroll_to_time(-5, &bars), Scrolled::BeforeHistory);
        let (_, oldest) = card.viewport.horizontal.offset_limits(500);
        assert_eq!(card.viewport.horizontal.right_offset, oldest);
        assert_eq!(card.bar_marker(), None, "nothing to mark yet");
    }

    #[test]
    fn no_bars_is_no_scroll() {
        let mut card = ChartCard::default();
        let before = card.viewport.horizontal;
        assert_eq!(card.scroll_to_time(5, &[]), Scrolled::NoBars);
        assert_eq!(card.viewport.horizontal, before);
    }

    #[test]
    fn scrolling_never_touches_scripts_or_the_time_lock() {
        let bars = candles(300);
        let mut card = ChartCard::default();
        card.set_time_lock(Some(5));
        card.add_pine_instance(None, "S", "//@version=6\nindicator(\"S\")\n");
        card.scroll_to_time(100 * 3_600_000, &bars);
        assert_eq!(card.time_lock(), Some(5));
        assert_eq!(card.pine_instances().len(), 1);
    }

    #[test]
    fn a_marker_that_ran_out_is_gone() {
        let card = ChartCard {
            bar_marker: Some((7, Instant::now() - MARKER_LIFETIME - Duration::from_secs(1))),
            ..ChartCard::default()
        };
        assert_eq!(card.bar_marker(), None);
    }

    #[test]
    fn the_label_has_the_detail_of_the_timeframe() {
        let time = 1_700_000_000_000;
        assert_eq!(
            marker_label(time, Some(Timeframe::Hour)),
            "2023-11-14 22:13"
        );
        assert_eq!(marker_label(time, Some(Timeframe::Day)), "2023-11-14");
        assert_eq!(marker_label(time, Some(Timeframe::Month)), "2023-11-14");
        assert_eq!(marker_label(time, None), "2023-11-14 22:13");
    }
}
