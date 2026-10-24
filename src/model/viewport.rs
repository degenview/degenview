//! The one description of what part of a chart the user is looking at.
//!
//! Every pan, zoom and scale gesture edits a [`ChartViewport`]; the plot, the grid, the series,
//! indicators, Pine visuals, drawings and the crosshair all read the window and the price range it
//! resolves to. Nothing keeps an offset of its own, and nothing here touches data: a viewport only
//! says which bars and which prices are on screen.
//!
//! Horizontal state counts in bars and is continuous: the chart follows the pointer by fractions of
//! a bar. Vertical state is either automatic (the visible bars decide the range) or manual (the
//! user has placed the range). Everything is pure so the gestures can be tested without a window.

use serde::{Deserialize, Serialize};

/// The default number of bars across the plot.
pub const DEFAULT_VISIBLE: usize = 120;
/// The fewest bars the plot may show.
pub const MIN_VISIBLE: usize = 20;
/// The most bars `set_visible` restores from a saved layout.
pub const MAX_RESTORED_VISIBLE: usize = 2000;
/// How far the price axis may stretch the automatic range.
pub const ZOOM_LIMITS: (f64, f64) = (0.25, 20.0);
/// A manual range may not shrink below this share of its centre, so the scale never degenerates.
const MIN_RELATIVE_SPAN: f64 = 1e-9;

/// A price span, `minimum < maximum`, in price units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PriceRange {
    pub minimum: f64,
    pub maximum: f64,
}

impl PriceRange {
    pub fn new(minimum: f64, maximum: f64) -> Self {
        Self { minimum, maximum }
    }

    /// Whether the range can be drawn on a scale of this kind.
    pub fn is_valid(&self, logarithmic: bool) -> bool {
        self.minimum.is_finite()
            && self.maximum.is_finite()
            && self.maximum > self.minimum
            && (!logarithmic || self.minimum > 0.0)
    }

    /// The range in the scale's own space: the prices, or their logarithms.
    fn in_scale_space(&self, logarithmic: bool) -> (f64, f64) {
        if logarithmic {
            (self.minimum.ln(), self.maximum.ln())
        } else {
            (self.minimum, self.maximum)
        }
    }

    fn from_scale_space(low: f64, high: f64, logarithmic: bool) -> Option<Self> {
        let range = if logarithmic {
            Self::new(low.exp(), high.exp())
        } else {
            Self::new(low, high)
        };
        range.is_valid(logarithmic).then_some(range)
    }

    /// The range moved by `fraction` of its own height (positive moves toward higher prices), in
    /// the scale's space. `None` where the result could not be drawn.
    pub fn shifted(&self, fraction: f64, logarithmic: bool) -> Option<Self> {
        if !self.is_valid(logarithmic) || !fraction.is_finite() {
            return None;
        }
        let (low, high) = self.in_scale_space(logarithmic);
        let delta = (high - low) * fraction;
        Self::from_scale_space(low + delta, high + delta, logarithmic)
    }

    /// The range stretched by `factor` about its centre (above 1 shows more), in the scale's space.
    pub fn scaled(&self, factor: f64, logarithmic: bool) -> Option<Self> {
        if !self.is_valid(logarithmic) || !factor.is_finite() || factor <= 0.0 {
            return None;
        }
        let (low, high) = self.in_scale_space(logarithmic);
        let (mid, half) = ((low + high) / 2.0, (high - low) / 2.0 * factor);
        let next = Self::from_scale_space(mid - half, mid + half, logarithmic)?;
        // A span lost to rounding is not a scale either.
        let centre = ((next.minimum + next.maximum) / 2.0)
            .abs()
            .max(f64::MIN_POSITIVE);
        ((next.maximum - next.minimum) / centre >= MIN_RELATIVE_SPAN).then_some(next)
    }
}

/// How the price axis picks its range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerticalViewport {
    /// The axis drag's stretch of the automatic range about its centre; `1.0` is none.
    pub zoom: f64,
    /// The range the user placed by dragging the chart. While set, the visible bars no longer
    /// decide the range (autoscale is off) until it is cleared.
    pub manual: Option<PriceRange>,
}

impl Default for VerticalViewport {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            manual: None,
        }
    }
}

impl VerticalViewport {
    #[cfg(test)]
    pub fn is_auto(&self) -> bool {
        self.manual.is_none()
    }

    /// Whether anything differs from a freshly opened chart.
    #[cfg(test)]
    pub fn is_default(&self) -> bool {
        self.is_auto() && self.zoom == 1.0
    }

    /// The range to draw, given what autoscale would show: the manual range if there is one, else
    /// the automatic one stretched by `zoom` about its centre.
    pub fn resolve(&self, auto: PriceRange, logarithmic: bool) -> PriceRange {
        if let Some(manual) = self.manual.filter(|range| range.is_valid(logarithmic)) {
            return manual;
        }
        if self.zoom == 1.0 {
            return auto;
        }
        auto.scaled(1.0 / self.zoom, logarithmic).unwrap_or(auto)
    }

    /// Back to autoscale at the natural scale.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Takes the chart by the body and moves it `dy` points down the plot of `height`: prices and
    /// overlays follow the pointer. `origin` is the range shown when the drag began, so a drag is
    /// always measured from its start and never accumulates. Autoscale goes off.
    pub fn pan(&mut self, origin: PriceRange, dy: f32, height: f32, logarithmic: bool) {
        if height <= 0.0 {
            return;
        }
        // Screen y grows downward, prices upward: following the pointer down shows higher prices.
        let fraction = f64::from(dy) / f64::from(height);
        if let Some(range) = origin.shifted(fraction, logarithmic) {
            self.manual = Some(range);
        }
    }

    /// Drags the price axis from `origin`: `factor` is how much more of the range is shown. In
    /// autoscale it stretches the automatic range; on a manual range it stretches that range about
    /// its centre.
    pub fn stretch(&mut self, origin: &VerticalViewport, factor: f64, logarithmic: bool) {
        match origin.manual {
            Some(manual) => {
                if let Some(range) = manual.scaled(factor, logarithmic) {
                    self.manual = Some(range);
                }
            }
            None => {
                self.zoom = (origin.zoom / factor).clamp(ZOOM_LIMITS.0, ZOOM_LIMITS.1);
                self.manual = None;
            }
        }
    }
}

/// Where the bars sit across the plot, and how many fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HorizontalViewport {
    /// Bars across the plot, not counting the free slots past the newest.
    pub visible: usize,
    /// How far the view is scrolled back from the newest bar, in bars. Positive shows older
    /// history; negative scrolls past the newest bar into empty space. Fractional while dragging.
    pub right_offset: f64,
}

impl Default for HorizontalViewport {
    fn default() -> Self {
        Self {
            visible: DEFAULT_VISIBLE,
            right_offset: 0.0,
        }
    }
}

impl HorizontalViewport {
    /// The offsets the view may take over `total` bars: back until only [`MIN_VISIBLE`] bars of
    /// history are left, forward until only that many bars of the series remain on screen.
    pub fn offset_limits(&self, total: usize) -> (f64, f64) {
        let oldest = total.saturating_sub(MIN_VISIBLE) as f64;
        let furthest = self.visible.saturating_sub(MIN_VISIBLE) as f64;
        (-furthest, oldest.max(-furthest))
    }

    /// Whether the newest bar is held where the right margin puts it.
    pub fn follows_latest(&self) -> bool {
        self.right_offset.abs() < 1e-6
    }

    /// Whether the newest bar's centre is inside the plot, with `reserved` free slots past it.
    /// Scrolled back far enough and it leaves on the right; the left edge is only crossed by an
    /// offset [`offset_limits`](Self::offset_limits) would not allow.
    pub fn shows_newest(&self, reserved: usize) -> bool {
        self.right_offset < reserved as f64 + 0.5 && self.right_offset > 0.5 - self.visible as f64
    }

    /// Keeps the offset a number the window can use and inside its limits.
    pub fn clamp(&mut self, total: usize) {
        let (low, high) = self.offset_limits(total);
        self.right_offset = if self.right_offset.is_finite() {
            self.right_offset.clamp(low, high)
        } else {
            0.0
        };
    }

    /// Drags the chart `dx` points to the right from where it was when the drag began at
    /// `origin`, over bars `slot_width` points wide. The chart follows the pointer exactly:
    /// dragging right reveals older bars. Fractions of a bar are kept.
    pub fn pan(&mut self, origin: f64, dx: f32, slot_width: f32, total: usize) {
        if slot_width > 0.0 && slot_width.is_finite() {
            self.right_offset = origin + f64::from(dx) / f64::from(slot_width);
        }
        self.clamp(total);
    }

    /// Moves by whole `bars` (the scroll buttons and keys).
    pub fn scroll_by(&mut self, bars: f64, total: usize) {
        self.right_offset += bars;
        self.clamp(total);
    }

    /// The most bars the plot may show over `total`: all of them, but never fewer than the default,
    /// so a market with little history (a young pool, a coarse timeframe) keeps the same bar width
    /// as a deep one and leaves the free space on the left instead of jumping to fit.
    pub fn max_visible(total: usize) -> usize {
        total.max(DEFAULT_VISIBLE)
    }

    pub fn set_visible(&mut self, count: usize, total: usize) {
        self.visible = count.clamp(MIN_VISIBLE, Self::max_visible(total));
        self.clamp(total);
    }

    /// Drags the time axis: `factor` is how many more bars are shown, the newest bar staying put.
    pub fn stretch(&mut self, origin_visible: usize, factor: f64, total: usize) {
        if factor.is_finite() && factor > 0.0 {
            let wanted = (origin_visible as f64 * factor).round() as usize;
            self.set_visible(wanted, total);
        }
    }
}

/// How far a drag must stray vertically before it takes the price range with it, so a drag meant
/// to scroll through time does not switch autoscale off by hand jitter. The distance already
/// travelled counts once it does: the chart never lags the pointer.
pub const VERTICAL_ENGAGE: f32 = 4.0;

/// One drag of the chart body, from the press to the release. Everything is measured from where
/// the drag began, so the chart stays under the pointer and no event is counted twice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanGesture {
    offset: f64,
    shown: PriceRange,
    vertical: VerticalViewport,
    engaged: bool,
}

impl PanGesture {
    /// Starts a drag over `view`, whose price axis shows `shown`.
    pub fn begin(view: &ChartViewport, shown: PriceRange) -> Self {
        Self {
            offset: view.horizontal.right_offset,
            shown,
            vertical: view.vertical,
            engaged: false,
        }
    }

    /// `bars` arrived after the newest bar while the drag was under way: the offset it measures
    /// from counts them, or the chart would jump by that many bars.
    pub fn arrived(&mut self, bars: f64) {
        self.offset += bars;
    }

    /// Applies the pointer's total movement since the press. `slot_width` and `plot_height` are the
    /// plot's current measures, `total` the bars the chart has.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        view: &mut ChartViewport,
        (dx, dy): (f32, f32),
        slot_width: f32,
        plot_height: f32,
        total: usize,
        logarithmic: bool,
    ) {
        view.horizontal.pan(self.offset, dx, slot_width, total);
        self.engaged |= dy.abs() >= VERTICAL_ENGAGE;
        if self.engaged {
            view.vertical = self.vertical;
            view.vertical.pan(self.shown, dy, plot_height, logarithmic);
        }
    }
}

/// The whole view of one chart.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChartViewport {
    pub horizontal: HorizontalViewport,
    pub vertical: VerticalViewport,
}

impl ChartViewport {
    /// Back to the newest bars at the natural scale.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The state a layout keeps beyond the bar count and the axis stretch it already has.
    pub fn saved(&self) -> SavedViewport {
        SavedViewport {
            right_offset: self.horizontal.right_offset,
            price_range: self.vertical.manual,
        }
    }

    /// Restores what [`Self::saved`] kept, refusing numbers a scale cannot use.
    pub fn restore(&mut self, saved: SavedViewport) {
        self.horizontal.right_offset = if saved.right_offset.is_finite() {
            saved.right_offset
        } else {
            0.0
        };
        self.vertical.manual = saved.price_range.filter(|range| range.is_valid(false));
    }
}

/// The part of the viewport a saved chart keeps next to its bar count and axis stretch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedViewport {
    /// Bars scrolled back from the newest; negative is empty space past it.
    #[serde(default)]
    pub right_offset: f64,
    /// A price range the user placed, when autoscale was off.
    #[serde(default)]
    pub price_range: Option<PriceRange>,
}

#[cfg(test)]
mod tests;
