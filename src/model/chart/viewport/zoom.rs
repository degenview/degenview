//! Zooming to a rectangle the user drew on the plot (the Zoom In tool).
//!
//! The rectangle is read in the plot's own coordinates and turned into the two things a
//! [`ChartViewport`](super::ChartViewport) holds: a bar count and offset, and a price range. Nothing
//! here knows about egui or about candles; the card feeds it the slot geometry it already draws
//! with, so the result is exactly what the pan and wheel gestures would produce for the same view.

use super::{HorizontalViewport, MIN_RELATIVE_SPAN, PriceRange};

/// The narrowest rectangle, in points, that counts as a selection. Below it the press was a click
/// with a little hand jitter, and a zoom to it would be extreme.
pub const MIN_SELECTION: f32 = 8.0;

/// How many zoomed-from views a chart remembers for Zoom out.
pub const MAX_BACK: usize = 8;

/// A rectangle on the plot, normalised (`left <= right`, `top <= bottom`, `y` grows downward) and
/// kept inside it, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Selection {
    /// The rectangle between two corners dragged in any direction, cut to `plot`
    /// (`left, top, right, bottom`). A corner that is not a number counts as the plot's edge, so
    /// the result is always finite.
    pub fn between(a: (f32, f32), b: (f32, f32), plot: (f32, f32, f32, f32)) -> Self {
        let (pl, pt, pr, pb) = plot;
        let fit = |v: f32, low: f32, high: f32| {
            if v.is_finite() {
                v.clamp(low, high)
            } else {
                low
            }
        };
        let (ax, bx) = (fit(a.0, pl, pr), fit(b.0, pl, pr));
        let (ay, by) = (fit(a.1, pt, pb), fit(b.1, pt, pb));
        Self {
            left: ax.min(bx),
            right: ax.max(bx),
            top: ay.min(by),
            bottom: ay.max(by),
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    /// Wide enough to zoom the time axis to. A narrower one is a click and zooms nothing.
    pub fn spans_time(&self) -> bool {
        self.width() >= MIN_SELECTION
    }

    /// Tall enough to zoom the price axis to. A flat sliver still zooms the time axis.
    pub fn spans_price(&self) -> bool {
        self.height() >= MIN_SELECTION
    }
}

/// How slots (one per bar, bar `i` filling `[i, i + 1)`) lie across the plot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slots {
    /// The slot coordinate at the plot's left edge: the first drawn bar's index minus its `lead`.
    pub first: f64,
    /// The plot's left edge, in points.
    pub left: f64,
    /// Points per slot.
    pub width: f64,
}

impl Slots {
    /// The (fractional) slot coordinate under a point's `x`.
    pub fn at(&self, x: f32) -> f64 {
        self.first + (f64::from(x) - self.left) / self.width
    }
}

/// The bar count and offset that make `selection` fill the plot, over `total` bars with `reserved`
/// free slots past the newest. `None` when the rectangle is too narrow or the geometry is unusable.
///
/// The bar count is whole, so the edges land within half a bar of the rectangle's; a rectangle
/// narrower than [`MIN_VISIBLE`](super::MIN_VISIBLE) bars grows about its centre. The result is
/// clamped like any pan.
pub fn horizontal(
    selection: &Selection,
    slots: Slots,
    total: usize,
    reserved: usize,
) -> Option<HorizontalViewport> {
    if !selection.spans_time() || !(slots.width.is_finite() && slots.width > 0.0) {
        return None;
    }
    let (from, to) = (slots.at(selection.left), slots.at(selection.right));
    let across = to - from;
    if !(from.is_finite() && to.is_finite() && across > 0.0) {
        return None;
    }
    let wanted = (across - reserved as f64).round().max(0.0) as usize;
    let mut view = HorizontalViewport::default();
    view.set_visible(wanted, total);
    // The plot shows `visible + reserved` slots; put the rectangle's centre in the middle of them.
    let shown = (view.visible + reserved) as f64;
    let right_edge = (from + to) / 2.0 + shown / 2.0;
    view.right_offset = (total + reserved) as f64 - right_edge;
    view.clamp(total);
    Some(view)
}

/// The price range between two prices read off the plot's price scale, for a manual range.
///
/// The prices come from the scale's own inverse, so a logarithmic scale yields a ratio-correct
/// range. `None` when the range cannot be drawn: not finite, empty, non-positive on a log scale, or
/// thinner than the scale can resolve.
pub fn price_range(a: f64, b: f64, logarithmic: bool) -> Option<PriceRange> {
    let range = PriceRange::new(a.min(b), a.max(b));
    if !range.is_valid(logarithmic) {
        return None;
    }
    let centre = range.minimum.abs().max(range.maximum.abs());
    let thin = (range.maximum - range.minimum) < centre * MIN_RELATIVE_SPAN;
    (!thin).then_some(range)
}
