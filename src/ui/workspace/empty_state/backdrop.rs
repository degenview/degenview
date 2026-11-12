//! The live trend line behind the top of a blank tab: a price that never stops wandering up and
//! down, scrolling left with its newest value at the right, a soft fill under it.
//!
//! The line is a pure function of the clock (smooth value noise over hashed lattice points), so it
//! keeps no state, never repeats and looks the same in any two frames at the same time.

use super::sparkline::fade_fill;
use crate::ui::kit::loading::hash;
use crate::ui::theme;
use eframe::egui::epaint::PathStroke;
use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};
use std::time::Duration;

/// The pause between frames while it plays (about 30 per second).
const FRAME: Duration = Duration::from_millis(33);
/// Horizontal distance between samples of the line.
const STEP: f32 = 5.0;
/// How far down the fill reaches before it is gone.
const FILL_DEPTH: f32 = 340.0;
/// Where the line wanders: this far below the top of the page, in a strip this tall. The page's
/// title starts below the strip, so no text is crossed by the line.
pub const LINE_TOP: f32 = 22.0;
pub const LINE_RANGE: f32 = 100.0;
/// Lattice cells that pass per second.
const SPEED: f64 = 0.22;
/// Lattice cells of the slow swing across the whole width.
const CELLS: f64 = 5.0;
/// How far the line stops short of the right edge, as a share of the width: the last price.
const END_INSET: f32 = 0.07;
/// Share of the width over which the left end fades in.
const FADE_IN: f32 = 0.3;
/// Share of the width, before the last price, over which the fill fades out.
const FADE_OUT: f32 = 0.12;

/// A number in `0.0..1.0` that changes smoothly with `at` and is arbitrary but fixed for a given
/// position and `salt`.
fn noise(at: f64, salt: u64) -> f32 {
    let cell = at.floor();
    let fraction = (at - cell) as f32;
    let eased = fraction * fraction * fraction * (fraction * (fraction * 6.0 - 15.0) + 10.0);
    let before = hash(cell as i64, salt);
    let after = hash(cell as i64 + 1, salt);
    before + (after - before) * eased
}

/// `count` levels in `0.0..=1.0` from the left of the picture to the right, at `time` seconds. A
/// slow swing with a quicker ripple on it, both drifting left, so peaks and dips keep arriving.
pub fn trend(time: f64, count: usize) -> Vec<f32> {
    let last = count.saturating_sub(1).max(1) as f64;
    (0..count)
        .map(|index| {
            let across = index as f64 / last;
            let swing = noise(time * SPEED + across * CELLS, 11);
            let ripple = noise(time * SPEED * 1.9 + across * CELLS * 2.7 + 40.0, 29);
            let level = 0.74 * swing + 0.26 * ripple;
            // Averaging pulls everything toward the middle; spread it back out.
            (0.5 + (level - 0.5) * 1.5).clamp(0.0, 1.0)
        })
        .collect()
}

fn smoothstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

/// Paints the picture in the top of `area` and, while the window is in front, asks for the next
/// frame. A background window keeps its last frame and costs nothing.
pub fn show(ui: &Ui, area: Rect) {
    let (time, focused) = ui.input(|input| (input.time, input.viewport().focused.unwrap_or(false)));
    paint(&ui.painter().with_clip_rect(area), area, time);
    if focused {
        ui.ctx().request_repaint_after(FRAME);
    }
}

/// The picture at `time`.
pub fn paint(painter: &Painter, area: Rect, time: f64) {
    let band = Rect::from_min_size(area.min, vec2(area.width(), FILL_DEPTH.min(area.height())));
    if band.width() < 120.0 || band.height() < LINE_TOP + LINE_RANGE {
        return;
    }
    let left = band.left();
    let end = band.right() - band.width() * END_INSET;
    let count = ((end - left) / STEP).ceil() as usize + 1;
    let line: Vec<Pos2> = trend(time, count)
        .into_iter()
        .enumerate()
        .map(|(index, level)| {
            pos2(
                left + (end - left) * index as f32 / (count - 1) as f32,
                band.top() + LINE_TOP + LINE_RANGE * (1.0 - level),
            )
        })
        .collect();
    let fade = band.width() * FADE_IN;
    let weight = move |x: f32| smoothstep((x - left) / fade);
    let fill_weight = move |x: f32| weight(x) * smoothstep((end - x) / (band.width() * FADE_OUT));
    let color = theme::UP;

    fade_fill(
        painter,
        &line,
        band.bottom(),
        color,
        theme::pick_value(0.24, 0.18),
        fill_weight,
    );
    // A halo under the stroke, then the stroke itself, each fading in from the left.
    for (width, alpha) in [(9.0, 0.05), (5.0, 0.1), (1.8, 0.95)] {
        painter.add(Shape::line(
            line.clone(),
            PathStroke::new_uv(width, move |_, point| {
                color.gamma_multiply(alpha * weight(point.x))
            }),
        ));
    }

    // The latest price: a dashed line out to the edge and a dot that pulses.
    let Some(&dot) = line.last() else {
        return;
    };
    painter.add(Shape::dashed_line(
        &[dot, pos2(band.right(), dot.y)],
        Stroke::new(1.0, color.gamma_multiply(0.35)),
        4.0,
        4.0,
    ));
    let pulse = ((time * 2.2).sin() * 0.5 + 0.5) as f32;
    painter.circle_filled(
        dot,
        5.0 + 6.0 * pulse,
        color.gamma_multiply(0.22 * (1.0 - pulse)),
    );
    painter.circle_filled(dot, 3.4, color);
    painter.circle_stroke(dot, 3.4, Stroke::new(1.0, Color32::from_black_alpha(40)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_stay_in_range_and_are_fixed_for_a_time() {
        let levels = trend(12.5, 200);
        assert_eq!(levels.len(), 200);
        assert!(levels.iter().all(|level| (0.0..=1.0).contains(level)));
        assert_eq!(levels, trend(12.5, 200));
    }

    #[test]
    fn the_line_wanders_in_space_and_in_time() {
        let now = trend(3.0, 200);
        let low = now.iter().cloned().fold(f32::MAX, f32::min);
        let high = now.iter().cloned().fold(f32::MIN, f32::max);
        assert!(high - low > 0.25, "a flat line is not a trend");
        assert_ne!(now, trend(13.0, 200), "it keeps moving");
    }

    #[test]
    fn it_moves_smoothly_from_one_frame_to_the_next() {
        let before = trend(7.0, 200);
        let after = trend(7.0 + 1.0 / 30.0, 200);
        let furthest = before
            .iter()
            .zip(&after)
            .map(|(before, after)| (before - after).abs())
            .fold(0.0, f32::max);
        assert!(furthest < 0.03, "no point jumps between frames: {furthest}");
    }

    #[test]
    fn it_never_comes_back_to_the_same_picture() {
        let first = trend(0.0, 120);
        assert!(
            (1..200).all(|second| trend(second as f64 * 3.7, 120) != first),
            "the noise lattice is hashed, not periodic"
        );
    }
}
