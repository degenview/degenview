//! What a chart plot draws while a replay is open: the start-picker marker and the
//! "this chart is time-travelled" border and badge.

use crate::ui::{replay_bar, theme};
use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

/// What a click on a candle while selecting is for: it decides the marker's words and colour.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PickKind {
    /// "Start here": where a replay begins.
    #[default]
    Replay,
    /// "Stop here": the bar the debugger should stop at.
    StopAtBar,
}

/// Per-frame exchange between the workspace and one chart card's replay overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplayChartFrame {
    /// In: "click a candle" mode. Drawing input is bypassed while it is on.
    pub selecting: bool,
    /// In: a replay is open on this chart.
    pub active: bool,
    /// Out: `open_time` (ms) of the candle clicked while selecting.
    pub picked_ms: Option<i64>,
}

/// Index of the candle whose slot contains `x`, or `None` over blank space.
pub(super) fn candle_at(
    x: f32,
    plot_left: f32,
    slot_width: f32,
    lead: f32,
    start: usize,
    count: usize,
) -> Option<usize> {
    if slot_width <= 0.0 {
        return None;
    }
    let slot = ((x - plot_left) / slot_width - lead).floor() as isize;
    let index = usize::try_from(slot).ok()?.checked_add(start)?;
    (index < count).then_some(index)
}

/// Marker at `x`: a line, a "Start here" tag, and the hidden future dimmed.
pub(super) fn paint_selection(
    painter: &Painter,
    plot: Rect,
    x: f32,
    open_time_ms: i64,
    kind: PickKind,
) {
    let (words, accent) = match kind {
        PickKind::Replay => ("Start here", theme::REPLAY),
        PickKind::StopAtBar => ("Stop here", theme::editor_debug_line()),
    };
    let clipped = painter.with_clip_rect(plot);
    clipped.rect_filled(
        Rect::from_min_max(Pos2::new(x, plot.top()), plot.right_bottom()),
        0.0,
        Color32::from_black_alpha(36),
    );
    clipped.line_segment(
        [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
        Stroke::new(1.5, accent),
    );
    let text = format!("{words} · {}", replay_bar::compact_text(open_time_ms));
    let font = FontId::monospace(10.5);
    let galley = painter.layout_no_wrap(text.clone(), font.clone(), Color32::WHITE);
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let centre_x = x.clamp(plot.left() + size.x / 2.0, plot.right() - size.x / 2.0);
    let tag = Rect::from_center_size(Pos2::new(centre_x, plot.top() + 12.0), size);
    painter.rect_filled(tag, 999.0, accent);
    painter.text(
        tag.center(),
        Align2::CENTER_CENTER,
        text,
        font,
        if kind == PickKind::StopAtBar && theme::is_light() {
            Color32::WHITE
        } else if kind == PickKind::StopAtBar {
            Color32::BLACK
        } else {
            Color32::WHITE
        },
    );
}

/// The card itself has the replay border and header badge; show a plot badge only when
/// the narrow header cannot fit its own.
pub(super) fn paint_active(painter: &Painter, plot: Rect) {
    if plot.width() < 430.0 {
        replay_bar::badge(painter, Pos2::new(plot.right() - 8.0, plot.top() + 8.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candle_under_the_pointer() {
        // 10 slots of 100px starting at x=0, candles 3..8 shown, no blanks.
        assert_eq!(candle_at(50.0, 0.0, 100.0, 0.0, 3, 20), Some(3));
        assert_eq!(candle_at(250.0, 0.0, 100.0, 0.0, 3, 20), Some(5));
        // Blank slots on the left push candles right.
        assert_eq!(candle_at(150.0, 0.0, 100.0, 2.0, 0, 5), None);
        assert_eq!(candle_at(250.0, 0.0, 100.0, 2.0, 0, 5), Some(0));
        // Past the last candle.
        assert_eq!(candle_at(950.0, 0.0, 100.0, 0.0, 15, 20), None);
        assert_eq!(candle_at(-5.0, 0.0, 100.0, 0.0, 0, 20), None);
    }
}
