//! Reframing a capture in the Studio: how much of the chart the image shows (Width and Height
//! dials) and where (dragging the preview), measured from the view the capture started with.
//!
//! A [`Framing`] is four numbers. [`Framing::viewport`] turns them, with a [`Base`] describing the
//! starting view, into the [`ChartViewport`] the composer draws the chart with. The chart's own
//! time scale is never changed: a bar keeps the width it had in the capture, so a wider plot shows
//! more bars and a dial at 2× half as many, and the candles are never stretched sideways. The price
//! scale is fitted instead: at 1× the starting price range fills the plot the composition gives
//! the chart whatever its height, so the newest candle and the last-price tag the capture showed are
//! never cropped, and the Height dial at 2× shows half that range (0.5× twice it). Dragging is limited to a quarter of the starting view each way, to the history that
//! is loaded and to a little empty space past the newest bar; past a limit the view gives way a
//! little (a rubber band) and [`Framing::settle`] puts it back on release.
//!
//! The ranges, steps and snap are the ones Vela documents; the quarter, the rubber band and what
//! counts as a bound are decisions, written next to the constants.

use crate::model::chart::viewport::{
    ChartViewport, HorizontalViewport, PriceRange, VerticalViewport,
};
use eframe::egui::Vec2;

/// The least a dial may be set to: half the starting magnification.
pub const MIN: f64 = 0.5;
/// The most a dial may be set to: twice the starting magnification.
pub const MAX: f64 = 2.0;
/// A dial released this close to 1× lands on 1×.
pub const SNAP: f64 = 0.04;
/// A dial's arrow-key step.
pub const FINE_STEP: f64 = 0.02;
/// A dial's Page Up / Page Down step.
pub const COARSE_STEP: f64 = 0.1;
/// How far the view may be moved either way, as a share of the starting view.
pub const POSITION_LIMIT: f64 = 0.25;
/// The most the view gives way past a limit while it is dragged, as a share of the starting view
/// (a decision: small enough to read as a wall, big enough to feel).
pub const RUBBER_REACH: f64 = 0.05;
/// How much of a pull past a limit the view follows at first.
const RESISTANCE: f64 = 0.5;
/// A key press on the preview moves the view this many bars across.
pub const KEY_BARS: f64 = 1.0;
/// A key press on the preview moves the view this share of the price range being shown up or down
/// (a decision: about what an arrow press moves the chart's own view by).
pub const KEY_PRICE: f64 = 0.02;
/// Dial values are kept to this many steps per unit, so repeated key steps never drift.
const GRID: f64 = 1_000.0;
/// The fewest bars an image may show; the plot of a narrow, zoomed-in capture is allowed to hold
/// fewer than the chart's own 20.
const MIN_BARS: usize = 2;

/// Which dial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dial {
    Width,
    Height,
}

/// How far a key moves a dial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// An arrow key.
    Fine,
    /// Page Up or Page Down.
    Coarse,
}

impl Step {
    pub const fn amount(self) -> f64 {
        match self {
            Self::Fine => FINE_STEP,
            Self::Coarse => COARSE_STEP,
        }
    }
}

/// How a capture is reframed. The default is the view it started with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    /// How much the image magnifies the time axis against the starting view: `2.0` shows half the
    /// bars, `0.5` twice as many.
    pub width: f64,
    /// How much the image magnifies the price axis: `2.0` shows half the price range, `0.5` twice
    /// as much.
    pub height: f64,
    /// Where the view is moved to, in bars; positive is toward newer bars.
    pub dx: f64,
    /// Where the view is moved to, as a share of the starting price range (in the scale's own
    /// space, so logarithmic for a log chart); positive is toward higher prices.
    pub dy: f64,
}

impl Default for Framing {
    fn default() -> Self {
        Self {
            width: 1.0,
            height: 1.0,
            dx: 0.0,
            dy: 0.0,
        }
    }
}

fn on_grid(value: f64) -> f64 {
    (value * GRID).round() / GRID
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

/// How far a view gives way when pulled `over` past a limit that lets it give way at most
/// `reach`: it follows `RESISTANCE` of the pull at first and slows to a halt, never reaching
/// `reach`. Zero for no pull.
pub fn resist(over: f64, reach: f64) -> f64 {
    if !(over > 0.0 && reach > 0.0) {
        return 0.0;
    }
    let pulled = over * RESISTANCE;
    pulled * reach / (pulled + reach)
}

/// `value` where it lies inside `(low, high)`, and past a limit only as far as [`resist`] lets it.
fn rubber(value: f64, (low, high): (f64, f64), reach: f64) -> f64 {
    if value < low {
        low - resist(low - value, reach)
    } else if value > high {
        high + resist(value - high, reach)
    } else {
        value
    }
}

/// The view a capture started with, as the composer needs to rebuild it.
///
/// Horizontal positions are slot coordinates: bar `i` fills `[i, i + 1)`, as in
/// [`Slots`](crate::model::chart::viewport::zoom::Slots). A slot is a bar, or one of the free
/// slots past the newest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Base {
    /// The slot coordinate at the capture's horizontal centre.
    pub start_bar_center: f64,
    /// The slots across the capture's plot: the bars it shows plus the free slots past the newest.
    pub bars_in_view: f64,
    /// The price range the capture's plot shows.
    pub price: PriceRange,
    pub logarithmic: bool,
    /// The bars the chart has.
    pub total_bars: usize,
    /// The free slots the chart keeps past the newest bar.
    pub reserved: usize,
    /// The capture's plot, in points.
    pub plot: Vec2,
}

impl Base {
    /// The starting view of a chart drawn as `horizontal` and `price` say, over `total_bars` with
    /// `reserved` free slots, whose plot is `plot` points across.
    #[cfg(test)]
    pub fn from_view(
        horizontal: &HorizontalViewport,
        price: PriceRange,
        logarithmic: bool,
        total_bars: usize,
        reserved: usize,
        plot: Vec2,
    ) -> Self {
        let slots = (horizontal.visible + reserved) as f64;
        Self {
            start_bar_center: (total_bars + reserved) as f64
                - horizontal.right_offset
                - slots / 2.0,
            bars_in_view: slots,
            price,
            logarithmic,
            total_bars,
            reserved,
            plot,
        }
    }

    /// The share of the capture's plot width that `plot` is: how much more room across the
    /// composition gives the chart. `1.0` where either has no width to compare.
    fn room_across(&self, plot: Vec2) -> f64 {
        let share = f64::from(plot.x) / f64::from(self.plot.x);
        if share.is_finite() && share > 0.0 {
            share
        } else {
            1.0
        }
    }

    /// The span of a price range in the scale's own space.
    fn span(&self, range: PriceRange) -> f64 {
        if self.logarithmic {
            (range.maximum / range.minimum).ln()
        } else {
            range.maximum - range.minimum
        }
    }
}

/// How far a [`Framing`]'s offsets may go: the least and most of each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub dx: (f64, f64),
    pub dy: (f64, f64),
}

impl Framing {
    /// Whether this is the view the capture started with (nothing for Reset view to undo).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn dial(&self, dial: Dial) -> f64 {
        match dial {
            Dial::Width => self.width,
            Dial::Height => self.height,
        }
    }

    /// Sets a dial, kept between [`MIN`] and [`MAX`]. A value that is not a number changes nothing.
    pub fn set_dial(&mut self, dial: Dial, value: f64) {
        if !value.is_finite() {
            return;
        }
        let value = value.clamp(MIN, MAX);
        match dial {
            Dial::Width => self.width = value,
            Dial::Height => self.height = value,
        }
    }

    #[cfg(test)]
    pub fn set_width(&mut self, value: f64) {
        self.set_dial(Dial::Width, value);
    }

    #[cfg(test)]
    pub fn set_height(&mut self, value: f64) {
        self.set_dial(Dial::Height, value);
    }

    /// A dial let go within [`SNAP`] of 1× lands on 1×.
    pub fn release_snap(&mut self) {
        let snapped = |value: f64| {
            if (value - 1.0).abs() <= SNAP + 1e-9 {
                1.0
            } else {
                value
            }
        };
        self.width = snapped(self.width);
        self.height = snapped(self.height);
    }

    /// Moves a dial one key step: up for a positive `sign`, down for a negative one, nothing for
    /// zero. The value stays on a fixed grid, so a dozen steps up and a dozen down meet again.
    pub fn step(&mut self, dial: Dial, step: Step, sign: i8) {
        let amount = step.amount() * f64::from(sign.signum());
        self.set_dial(dial, on_grid(self.dial(dial) + amount));
    }

    pub fn reset_dial(&mut self, dial: Dial) {
        self.set_dial(dial, 1.0);
    }

    /// Back to the centre of the starting view; Width and Height stay.
    pub fn reset_position(&mut self) {
        self.dx = 0.0;
        self.dy = 0.0;
    }

    /// Everything back to the starting view.
    pub fn reset_view(&mut self) {
        *self = Self::default();
    }

    /// The bars the image shows: whole, as a [`HorizontalViewport`] counts them, and the free
    /// slots on top. Rounding to whole bars moves a slot's width by at most half a slot over the
    /// plot's width.
    fn slots(&self, base: &Base, plot: Vec2) -> (usize, f64) {
        let across = base.room_across(plot);
        let wanted = base.bars_in_view * across / self.width.clamp(MIN, MAX);
        let free = base.reserved as f64;
        let visible = if (wanted - free).is_finite() {
            ((wanted - free).round().max(MIN_BARS as f64)) as usize
        } else {
            MIN_BARS
        };
        (visible, visible as f64 + free)
    }

    /// How far the view may go with these dials, for a composition whose plot is `plot` points.
    ///
    /// Across: not more than a quarter of the starting view either way, the first slot shown not
    /// before the oldest loaded bar, the last not more than `reserved` slots past the newest. A
    /// start that already shows more (a short history, a chart scrolled into empty space) keeps
    /// what it shows, so the centre is always inside the bounds. Down: a quarter of the starting
    /// price range.
    pub fn bounds(&self, base: &Base, plot: Vec2) -> Bounds {
        let (_, slots) = self.slots(base, plot);
        let reach = POSITION_LIMIT * base.bars_in_view;
        let started_left = base.start_bar_center - base.bars_in_view / 2.0;
        let started_right = base.start_bar_center + base.bars_in_view / 2.0;
        let oldest = started_left.min(0.0);
        let newest = started_right.max((base.total_bars + base.reserved) as f64);
        let low = (oldest - base.start_bar_center + slots / 2.0).clamp(-reach, 0.0);
        let high = (newest - base.start_bar_center - slots / 2.0).clamp(0.0, reach);
        Bounds {
            dx: (low, high),
            dy: (-POSITION_LIMIT, POSITION_LIMIT),
        }
    }

    /// The framing while the preview is dragged to `dx` bars and `dy` of the starting price range
    /// from the starting view (the drag's origin plus the pointer's movement, so a drag is always
    /// measured from where it began). Past a limit the view follows only a little of the pull.
    /// A value that is not a number leaves that offset where it was.
    pub fn drag(&self, base: &Base, plot: Vec2, dx: f64, dy: f64) -> Self {
        let bounds = self.bounds(base, plot);
        Self {
            dx: rubber(
                finite_or(dx, self.dx),
                bounds.dx,
                RUBBER_REACH * base.bars_in_view,
            ),
            dy: rubber(finite_or(dy, self.dy), bounds.dy, RUBBER_REACH),
            ..*self
        }
    }

    /// The framing while the preview is being dragged: `origin` (the framing when the pointer went
    /// down) moved by `delta`, the pointer's travel since then in composition points, so the picture
    /// follows the pointer: dragging right shows earlier bars, dragging down shows higher prices.
    /// `plot` is the composition's plot in points (see [`drag`](Self::drag)). Held at the limits
    /// with a rubber band like any drag; a capture with no plot to measure against is not moved.
    pub fn dragged(&self, base: &Base, plot: Vec2, origin: &Self, delta: Vec2) -> Self {
        let (across, down) = Self::points_to_offsets(base, plot, origin, delta);
        Self {
            width: origin.width,
            height: origin.height,
            ..*self
        }
        .drag(base, plot, origin.dx - across, origin.dy + down)
    }

    /// What a pointer travel of `delta` points is, in bars across and in shares of the starting
    /// price range up, at the dials `origin` has.
    fn points_to_offsets(base: &Base, plot: Vec2, origin: &Self, delta: Vec2) -> (f64, f64) {
        let slot = f64::from(base.plot.x) / base.bars_in_view * origin.width.clamp(MIN, MAX);
        let tall = f64::from(plot.y) * origin.height.clamp(MIN, MAX);
        let ratio = |points: f32, per: f64| {
            let moved = f64::from(points) / per;
            if moved.is_finite() { moved } else { 0.0 }
        };
        (ratio(delta.x, slot), ratio(delta.y, tall))
    }

    /// The framing after a key press that moves the picture `across` (positive: right) and `down`
    /// by one step, as an arrow key does: kept inside the limits, never into the rubber band.
    pub fn nudged(&self, base: &Base, plot: Vec2, across: i8, down: i8) -> Self {
        let dx = self.dx - KEY_BARS * f64::from(across.signum());
        let dy = self.dy + KEY_PRICE * f64::from(down.signum()) / self.height.clamp(MIN, MAX);
        self.drag(base, plot, dx, dy).settle(base, plot)
    }

    /// The framing on release: an offset past a limit goes back to it. Also what a change of a
    /// dial or of the composition needs, since the limits move with them.
    pub fn settle(&self, base: &Base, plot: Vec2) -> Self {
        let bounds = self.bounds(base, plot);
        Self {
            dx: finite_or(self.dx, 0.0).clamp(bounds.dx.0, bounds.dx.1),
            dy: finite_or(self.dy, 0.0).clamp(bounds.dy.0, bounds.dy.1),
            ..*self
        }
    }

    /// The view to draw the chart with in a composition whose plot is `plot` points across: the
    /// capture's bar width, divided by the Width dial, and the capture's price range, divided by
    /// the Height dial, centred on the capture's centre moved by the offsets. The price range
    /// fills the plot's height at any `plot`, so only its width matters here. The range is manual,
    /// so autoscale does not undo it.
    ///
    /// The offsets are held to the limits plus the room a rubber band may take, so a framing that
    /// was dragged past a limit still draws, and one that no longer fits a limit (a dial moved
    /// since) never shows more than that.
    pub fn viewport(&self, base: &Base, plot: Vec2) -> ChartViewport {
        let bounds = self.bounds(base, plot);
        let band = RUBBER_REACH * base.bars_in_view;
        let dx = finite_or(self.dx, 0.0).clamp(bounds.dx.0 - band, bounds.dx.1 + band);
        let dy =
            finite_or(self.dy, 0.0).clamp(bounds.dy.0 - RUBBER_REACH, bounds.dy.1 + RUBBER_REACH);

        let (visible, slots) = self.slots(base, plot);
        let centre = base.start_bar_center + dx;
        let horizontal = HorizontalViewport {
            visible,
            right_offset: (base.total_bars + base.reserved) as f64 - centre - slots / 2.0,
        };

        let log = base.logarithmic;
        let shown = base
            .price
            .scaled(1.0 / self.height.clamp(MIN, MAX), log)
            .unwrap_or(base.price);
        let moved = shown
            .shifted(dy * base.span(base.price) / base.span(shown), log)
            .unwrap_or(shown);
        ChartViewport {
            horizontal,
            vertical: VerticalViewport {
                zoom: 1.0,
                manual: moved.is_valid(log).then_some(moved),
            },
        }
    }
}

/// A dial's value as the Studio writes it beside the ruler: `1.28×`, `1×`, `0.75×`.
pub fn multiplier_text(value: f64) -> String {
    let text = format!("{:.2}", if value.is_finite() { value } else { 1.0 });
    format!("{}×", text.trim_end_matches('0').trim_end_matches('.'))
}
