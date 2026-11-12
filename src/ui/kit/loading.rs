//! The busy indicator: a small candlestick chart that scrolls by, new candles arriving on the right
//! as old ones leave on the left, so it reads as a market ticking rather than a generic wheel.
//!
//! It is a square of any size. Small (a button or a field) it is a few gray bodies in the text
//! colour; larger it gains wicks and takes the chart's up and down colours, since there a picture
//! is wanted and colour carries the "market" meaning. `color` forces one colour at any size, as
//! `egui::Spinner` does, so a call site can swap it in unchanged.
//!
//! The candles are a fixed deterministic series, so the picture is a pure function of the clock:
//! no state is kept between frames, and any two indicators on screen show the same market.

use crate::ui::accessibility;
use crate::ui::kit::{self, REGULAR};
use crate::ui::theme;
use eframe::egui::{
    Color32, Painter, Rect, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetType, pos2, vec2,
};

/// Seconds a candle takes to move one slot to the left.
const STEP: f64 = 0.6;
/// The side of an indicator when the caller does not say.
const DEFAULT_SIZE: f32 = 24.0;
/// From this side up the candles take the chart's colours instead of one flat colour.
const MARKET_FROM: f32 = 28.0;
/// From this side up the candles have wicks (below it they would be a smear).
const WICKS_FROM: f32 = 20.0;
/// A body is never thinner than this, so a candle that opened where it closed is still seen.
const MIN_BODY: f32 = 1.5;

/// One candle, every value in `0.0..=1.0` from the bottom of the picture to its top.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Candle {
    open: f32,
    close: f32,
    high: f32,
    low: f32,
}

impl Candle {
    fn rising(self) -> bool {
        self.close >= self.open
    }
}

/// Where the closes of the series sit: a swing with a faster ripple on it, so consecutive candles differ enough to have real bodies.
fn close_at(index: i64) -> f32 {
    let i = index as f64;
    (0.5 + 0.3 * (1.7 * i + 0.4).sin() + 0.12 * (3.1 * i + 1.7).sin()) as f32
}

/// A number in `0.0..1.0` that looks arbitrary but is the same every time for the same inputs.
fn hash(index: i64, salt: u64) -> f32 {
    let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// The `index`th candle of the series. Each opens where the one before closed, so the bodies
/// form one continuous line like a real chart's.
fn candle(index: i64) -> Candle {
    let open = close_at(index - 1);
    let close = close_at(index);
    let wick = |salt| 0.04 + 0.08 * hash(index, salt);
    Candle {
        open,
        close,
        high: (open.max(close) + wick(1)).min(1.0),
        low: (open.min(close) - wick(2)).max(0.0),
    }
}

/// How many candles fit a picture this big: few enough that each stays a readable bar.
fn candle_count(size: f32) -> usize {
    if size < 20.0 {
        3
    } else if size < 40.0 {
        4
    } else {
        5
    }
}

/// How far in from the right edge, in slots, a new candle takes to finish forming.
const GROW_SLOTS: f32 = 1.5;

/// One candle on its way across the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    /// The candle as it ends up; its colour is decided by this, so it never flips while forming.
    candle: Candle,
    /// Where its centre is, as a fraction of the picture's width.
    x: f32,
    /// `0.0` when it is entering or leaving at an edge, `1.0` once it is in.
    alpha: f32,
    /// How much of the candle has formed: `0.0` as it enters, `1.0` once it is done.
    progress: f32,
}

impl Placed {
    /// The candle as drawn now. A new one opens at the previous close as a dot, then its body
    /// grows up or down to its close and the wicks reach out, quickly at first and then settling,
    /// like a live candle ticking toward where it closes.
    fn shown(self) -> Candle {
        let eased = 1.0 - (1.0 - self.progress).powi(3);
        let Candle {
            open,
            close,
            high,
            low,
        } = self.candle;
        let close_now = open + (close - open) * eased;
        Candle {
            open,
            close: close_now,
            high: open.max(close_now) + (high - open.max(close)) * eased,
            low: open.min(close_now) - (open.min(close) - low) * eased,
        }
    }
}

/// The candles on screen `time` seconds in. The slots move continuously, so a candle that leaves
/// on the left is the same one that was a slot further right a moment before.
fn placed(count: usize, time: f64) -> Vec<Placed> {
    let scroll = time / STEP;
    let base = scroll.floor() as i64;
    let fraction = (scroll - base as f64) as f32;
    let slot = 1.0 / count as f32;
    (0..=count)
        .map(|k| {
            let x = (k as f32 + 0.5 - fraction) * slot;
            // A candle fades out over the slot at the left edge and in over half a slot at the
            // right, so none pops in or out and a new one is seen forming.
            let edge = (x / slot).min((1.0 - x) / (slot * 0.5));
            let alpha = edge.clamp(0.0, 1.0);
            Placed {
                candle: candle(base + k as i64),
                x,
                alpha: alpha * alpha * (3.0 - 2.0 * alpha),
                progress: ((1.0 - x) / (slot * GROW_SLOTS)).clamp(0.0, 1.0),
            }
        })
        .filter(|placed| (0.0..=1.0).contains(&placed.x))
        .collect()
}

/// The colours of a picture: one for everything, or one for rising and one for falling candles.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tint {
    Single(Color32),
    Market { up: Color32, down: Color32 },
}

impl Tint {
    fn resolve(size: f32, color: Option<Color32>) -> Self {
        match color {
            Some(color) => Self::Single(color),
            None if size >= MARKET_FROM => Self::Market {
                up: theme::UP,
                down: theme::DOWN,
            },
            None => Self::Single(theme::text_secondary()),
        }
    }

    fn of(self, candle: Candle) -> Color32 {
        match self {
            Self::Single(color) => color,
            Self::Market { up, down } => {
                if candle.rising() {
                    up
                } else {
                    down
                }
            }
        }
    }
}

/// Draws the candles of `time` seconds in `rect`, clipped to it.
fn paint(painter: &Painter, rect: Rect, time: f64, tint: Tint) {
    let size = rect.width().min(rect.height());
    let count = candle_count(size);
    let painter = painter.with_clip_rect(rect);
    // A pixel of room above and below, so the stroke of a wick at the extremes is not cut.
    let plot = rect.shrink2(vec2(0.0, 1.0));
    let slot = plot.width() / count as f32;
    let body_width = (slot * 0.56).max(2.0);
    let radius = (body_width / 5.0).min(1.5);
    let wick_width = (size / 28.0).clamp(1.0, 1.5);
    let y = |value: f32| plot.bottom() - value * plot.height();
    for placed in placed(count, time) {
        let color = tint.of(placed.candle).gamma_multiply(placed.alpha);
        let shown = placed.shown();
        let x = plot.left() + placed.x * plot.width();
        if size >= WICKS_FROM {
            painter.line_segment(
                [pos2(x, y(shown.high)), pos2(x, y(shown.low))],
                Stroke::new(wick_width, color),
            );
        }
        let (top, bottom) = (
            y(shown.open.max(shown.close)),
            y(shown.open.min(shown.close)),
        );
        let middle = (top + bottom) / 2.0;
        let half = ((bottom - top) / 2.0).max(MIN_BODY / 2.0);
        painter.rect_filled(
            Rect::from_min_max(
                pos2(x - body_width / 2.0, middle - half),
                pos2(x + body_width / 2.0, middle + half),
            ),
            radius,
            color,
        );
    }
}

/// A scrolling candlestick chart that says "working on it". Add it like a spinner, or put it in a
/// rect you laid out with [`paint_at`](Self::paint_at).
#[derive(Clone, Copy, Debug)]
#[must_use = "a loading indicator draws only when added to a Ui"]
pub struct LoadingIndicator {
    size: f32,
    color: Option<Color32>,
}

impl LoadingIndicator {
    pub fn new() -> Self {
        Self {
            size: DEFAULT_SIZE,
            color: None,
        }
    }

    /// The side of the square it takes, in points.
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// One colour for every candle, at any size.
    pub fn color(mut self, color: impl Into<Color32>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Draws it in `rect` (the smaller side is its size) without taking space from the layout.
    /// Keeps the window repainting for as long as it is on screen.
    pub fn paint_at(self, ui: &Ui, rect: Rect) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        let tint = Tint::resolve(rect.width().min(rect.height()), self.color);
        paint(ui.painter(), rect, ui.input(|input| input.time), tint);
        ui.ctx().request_repaint();
    }
}

/// The side of the indicator in [`message`].
const MESSAGE_INDICATOR: f32 = 40.0;
const MESSAGE_GAP: f32 = 10.0;

/// A large indicator with `text` under it, both centred in `area`: the loading state of a pane
/// that has nothing else to show yet. Painted, so it takes no room from the layout.
pub fn message(ui: &Ui, area: Rect, text: &str) {
    let label = kit::galley(
        ui,
        text,
        12.0,
        REGULAR,
        theme::text_secondary(),
        area.width(),
    );
    let top = area.center().y - (MESSAGE_INDICATOR + MESSAGE_GAP + label.size().y) / 2.0;
    LoadingIndicator::new().size(MESSAGE_INDICATOR).paint_at(
        ui,
        Rect::from_center_size(
            pos2(area.center().x, top + MESSAGE_INDICATOR / 2.0),
            Vec2::splat(MESSAGE_INDICATOR),
        ),
    );
    ui.painter().galley(
        pos2(
            area.center().x - label.size().x / 2.0,
            top + MESSAGE_INDICATOR + MESSAGE_GAP,
        ),
        label,
        Color32::PLACEHOLDER,
    );
}

impl Default for LoadingIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for LoadingIndicator {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), Sense::hover());
        self.paint_at(ui, rect);
        accessibility::label_as(response, WidgetType::ProgressIndicator, "Loading")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, Pos2};

    #[test]
    fn the_series_stays_in_the_picture_and_forms_one_line() {
        for index in -200..200 {
            let c = candle(index);
            for value in [c.open, c.close, c.high, c.low] {
                assert!((0.0..=1.0).contains(&value), "{index}: {c:?}");
            }
            assert!(c.high >= c.open.max(c.close), "{index}: {c:?}");
            assert!(c.low <= c.open.min(c.close), "{index}: {c:?}");
            assert_eq!(c.open, candle(index - 1).close, "{index}");
        }
    }

    #[test]
    fn the_series_has_both_rising_and_falling_candles() {
        let rising = (0..40).filter(|&i| candle(i).rising()).count();
        assert!((10..=30).contains(&rising), "{rising} of 40 rise");
    }

    #[test]
    fn bigger_pictures_hold_more_candles() {
        assert_eq!(candle_count(12.0), 3);
        assert_eq!(candle_count(19.9), 3);
        assert_eq!(candle_count(24.0), 4);
        assert_eq!(candle_count(44.0), 5);
    }

    #[test]
    fn every_placed_candle_is_inside_the_picture_and_fades_at_the_edges() {
        for count in [3, 4, 5] {
            for step in 0..400 {
                let time = f64::from(step) * 0.037 - 3.0;
                let candles = placed(count, time);
                assert!(candles.len() >= count, "{count} at {time}");
                for placed in candles {
                    assert!((0.0..=1.0).contains(&placed.x), "{placed:?}");
                    assert!((0.0..=1.0).contains(&placed.alpha), "{placed:?}");
                    if placed.x < 0.01 || placed.x > 0.99 {
                        assert!(placed.alpha < 0.05, "{placed:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_candle_moves_one_slot_each_step_and_keeps_its_shape() {
        let count = 4;
        let slot = 1.0 / count as f32;
        let time = 1.234;
        let now = placed(count, time);
        let later = placed(count, time + STEP);
        // The one in the leftmost slot leaves; every other is still there, a slot over.
        for before in now.iter().filter(|placed| placed.x >= slot) {
            let after = later
                .iter()
                .find(|placed| placed.candle == before.candle)
                .expect("a candle still on screen one step later");
            assert!(
                (before.x - slot - after.x).abs() < 1e-4,
                "{before:?} then {after:?}"
            );
        }
    }

    #[test]
    fn a_candle_grows_from_its_open_to_its_close_as_it_comes_in() {
        let mut grew = false;
        for count in [3, 4, 5] {
            for step in 0..600 {
                let time = f64::from(step) * 0.023;
                for placed in placed(count, time) {
                    let shown = placed.shown();
                    let done = placed.candle;
                    assert_eq!(shown.open, done.open, "{placed:?}");
                    assert!(
                        shown.high >= shown.open.max(shown.close) - 1e-6,
                        "{placed:?}"
                    );
                    assert!(
                        shown.low <= shown.open.min(shown.close) + 1e-6,
                        "{placed:?}"
                    );
                    // Never past where it finishes, on either side.
                    assert!(shown.high <= done.high + 1e-6 && shown.low >= done.low - 1e-6);
                    assert!(
                        (shown.close - done.open).abs() <= (done.close - done.open).abs() + 1e-6,
                        "{placed:?}"
                    );
                    if placed.progress >= 1.0 {
                        for (a, b) in [
                            (shown.close, done.close),
                            (shown.high, done.high),
                            (shown.low, done.low),
                        ] {
                            assert!((a - b).abs() < 1e-5, "{placed:?}");
                        }
                    } else if placed.progress > 0.2 && placed.progress < 0.6 {
                        grew = true;
                    }
                }
            }
        }
        assert!(grew, "some candle is seen part way");
    }

    #[test]
    fn a_new_candle_starts_as_a_dot_at_the_previous_close() {
        let fresh = Placed {
            candle: candle(7),
            x: 1.0,
            alpha: 0.0,
            progress: 0.0,
        };
        let shown = fresh.shown();
        assert_eq!(shown.close, shown.open);
        assert_eq!(shown.high, shown.open);
        assert_eq!(shown.low, shown.open);
        assert_eq!(shown.open, candle(6).close);
    }

    #[test]
    fn the_rightmost_candles_are_the_ones_still_forming() {
        let candles = placed(4, 0.2);
        let newest = candles.last().expect("candles");
        let oldest = candles.first().expect("candles");
        assert!(newest.progress < 1.0);
        assert_eq!(oldest.progress, 1.0);
    }

    #[test]
    fn it_is_flat_gray_small_and_the_charts_colours_large() {
        let rising = candle(0);
        assert!(rising.rising());
        let falling = (0..).map(candle).find(|c| !c.rising()).expect("one falls");
        let small = Tint::resolve(16.0, None);
        assert_eq!(small, Tint::Single(theme::text_secondary()));
        let large = Tint::resolve(44.0, None);
        assert_eq!(large.of(rising), theme::UP);
        assert_eq!(large.of(falling), theme::DOWN);
        let forced = Tint::resolve(44.0, Some(Color32::WHITE));
        assert_eq!(forced.of(rising), Color32::WHITE);
        assert_eq!(forced.of(falling), Color32::WHITE);
    }

    fn settled_repaint(visible: bool) -> std::time::Duration {
        let ctx = egui::Context::default();
        let mut out = None;
        for step in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0))),
                time: Some(f64::from(step) * 0.1),
                ..Default::default()
            };
            let mut frame = ctx.run_ui(raw, |ui| {
                if visible {
                    ui.add(LoadingIndicator::new().size(44.0));
                }
            });
            frame.textures_delta.clear();
            out = Some(frame);
        }
        out.expect("frames")
            .viewport_output
            .values()
            .map(|viewport| viewport.repaint_delay)
            .min()
            .expect("a viewport")
    }

    #[test]
    fn it_keeps_the_window_painting_only_while_shown() {
        assert!(settled_repaint(true) < std::time::Duration::from_millis(100));
        assert!(settled_repaint(false) >= std::time::Duration::from_millis(100));
    }

    #[test]
    fn it_takes_the_square_it_was_given() {
        let ctx = egui::Context::default();
        let mut size = Vec2::ZERO;
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            size = ui.add(LoadingIndicator::new().size(40.0)).rect.size();
        });
        frame.textures_delta.clear();
        assert_eq!(size, Vec2::splat(40.0));
    }

    /// The indicator in both themes: each size, then one frame every 0.15 s of a 44 pt one to see
    /// the motion. `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_loading_indicator
    /// -- --ignored` writes them.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_loading_indicator() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme::AppTheme;
        if gallery_dir().is_none() {
            return;
        }
        let _restore = theme::RestoreDefaultTheme;
        for (suffix, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let mut screen = Screen::new(vec2(560.0, 190.0));
            theme::apply(&screen.ctx, app_theme);
            let mut last = None;
            for _ in 0..2 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let ground = ui.max_rect();
                    ui.painter().rect_filled(ground, 0.0, theme::background());
                    let mut x = 16.0;
                    for size in [12.0, 16.0, 20.0, 24.0, 32.0, 44.0, 64.0] {
                        let rect = Rect::from_min_size(pos2(x, 16.0), Vec2::splat(size));
                        let tint = Tint::resolve(size, None);
                        paint(ui.painter(), rect, 0.3, tint);
                        x += size + 22.0;
                    }
                    for k in 0..8 {
                        let rect = Rect::from_min_size(
                            pos2(16.0 + k as f32 * 66.0, 108.0),
                            Vec2::splat(44.0),
                        );
                        paint(
                            ui.painter(),
                            rect,
                            f64::from(k) * 0.15,
                            Tint::resolve(44.0, None),
                        );
                    }
                }));
            }
            screen.save(
                &format!("loading-indicator-{suffix}"),
                last.expect("a frame"),
            );
        }
    }
}
