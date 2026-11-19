//! The chart canvas: everything about how a chart looks that is not its data or its bars —
//! background, grid, pane separators, crosshair and its values box, watermark, scale text and
//! lines, the on-chart buttons and the margins around the price range. It rides inside
//! [`ChartAppearance`](super::ChartAppearance), so it is saved with the chart, the saved layouts
//! and the layout fingerprint, and travels with chart templates, without a second persistence
//! path.
//!
//! The canvas is presentation only: nothing here reaches the market data, the constructed bars,
//! the indicators or Pine, and a change to it never reruns any of them.
//!
//! A colour that is `None` follows the app theme, so a chart that was never customised changes
//! with the theme while an explicit colour survives a theme switch. Colours are straight
//! `[r, g, b, a]` bytes ([`Rgba`]); their alpha is the opacity the settings sheet offers.
//!
//! Pure: no egui types. The geometry the renderer shares with the settings (tick positions, the
//! visible window, the margin transform, the pane split) lives here so it is tested without a
//! window.

use crate::model::chart::kind::{LineDash, Rgba};
use serde::{Deserialize, Serialize};

// MARK: Background

/// Whether the chart is one colour or a vertical gradient.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundMode {
    #[default]
    Solid,
    Gradient,
}

impl BackgroundMode {
    pub const ALL: [Self; 2] = [Self::Solid, Self::Gradient];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Gradient => "Gradient",
        }
    }
}

/// The colours behind the whole chart: the plot, its scales and every pane share one background.
/// Each colour is kept while the other mode is selected, so switching back finds it as it was.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct BackgroundAppearance {
    pub mode: BackgroundMode,
    /// The solid colour; `None` follows the theme's chart surface.
    pub solid: Option<Rgba>,
    /// The gradient's top; `None` follows the theme's chart surface.
    pub top: Option<Rgba>,
    /// The gradient's bottom; `None` is the chart surface leaning towards the accent colour.
    pub bottom: Option<Rgba>,
}

// MARK: Lines

/// One kind of line on the canvas. Each setting exposes only the properties that mean something
/// for it (a grid line is always one point wide, a crosshair has a width).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct LineAppearance {
    pub visible: bool,
    /// `None` follows the theme.
    pub color: Option<Rgba>,
    pub dash: LineDash,
    /// In points, [`LINE_WIDTHS`].
    pub width: u8,
}

pub const LINE_WIDTHS: std::ops::RangeInclusive<u8> = 1..=4;

impl LineAppearance {
    pub const fn new(dash: LineDash) -> Self {
        Self {
            visible: true,
            color: None,
            dash,
            width: 1,
        }
    }

    /// The width forced into [`LINE_WIDTHS`], for a row a hand-edited value reached.
    pub fn sanitized(mut self) -> Self {
        self.width = self.width.clamp(*LINE_WIDTHS.start(), *LINE_WIDTHS.end());
        self
    }
}

impl Default for LineAppearance {
    fn default() -> Self {
        Self::new(LineDash::Solid)
    }
}

// MARK: Watermark

/// What the watermark says.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkMode {
    #[default]
    Hidden,
    /// The market and the chart's timeframe, always.
    Symbol,
    /// The same, only while a replay is running. With no replay the watermark is hidden.
    Replay,
}

impl WatermarkMode {
    pub const ALL: [Self; 3] = [Self::Hidden, Self::Symbol, Self::Replay];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Hidden => "Hidden",
            Self::Symbol => "Symbol",
            Self::Replay => "Replay mode",
        }
    }

    /// Whether the watermark is drawn while a replay is (or is not) running.
    pub const fn shows(self, replaying: bool) -> bool {
        match self {
            Self::Hidden => false,
            Self::Symbol => true,
            Self::Replay => replaying,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WatermarkAppearance {
    pub mode: WatermarkMode,
    /// `None` is the theme's ink at a faint opacity.
    pub color: Option<Rgba>,
}

// MARK: Scales

/// The text sizes the scales offer, in points.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleTextSize {
    #[default]
    Pt10,
    Pt11,
    Pt12,
    Pt13,
    Pt14,
    Pt16,
    Pt18,
}

impl ScaleTextSize {
    pub const ALL: [Self; 7] = [
        Self::Pt10,
        Self::Pt11,
        Self::Pt12,
        Self::Pt13,
        Self::Pt14,
        Self::Pt16,
        Self::Pt18,
    ];

    pub const fn points(self) -> f32 {
        match self {
            Self::Pt10 => 10.0,
            Self::Pt11 => 11.0,
            Self::Pt12 => 12.0,
            Self::Pt13 => 13.0,
            Self::Pt14 => 14.0,
            Self::Pt16 => 16.0,
            Self::Pt18 => 18.0,
        }
    }

    pub fn label(self) -> String {
        format!("{} pt", self.points())
    }

    /// The time axis reads one point smaller than the price axis, as the chart always drew it.
    pub const fn time_points(self) -> f32 {
        self.points() - 1.0
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ScaleAppearance {
    /// `None` is the theme's secondary text.
    pub text_color: Option<Rgba>,
    pub text_size: ScaleTextSize,
    /// The lines that bound the scales; `None` is the theme's border.
    pub line_color: Option<Rgba>,
}

// MARK: Buttons

/// When an on-chart button group is drawn. Hiding a group hides only the buttons: the keyboard,
/// the wheel and the trackpad keep working.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonVisibility {
    Always,
    OnHover,
    Never,
}

impl ButtonVisibility {
    pub const ALL: [Self; 3] = [Self::Always, Self::OnHover, Self::Never];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Always => "Always visible",
            Self::OnHover => "Visible on mouse over",
            Self::Never => "Never visible",
        }
    }

    /// Whether the buttons are wanted given whether the pointer is over the chart.
    pub const fn wanted(self, hovered: bool) -> bool {
        match self {
            Self::Always => true,
            Self::OnHover => hovered,
            Self::Never => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ChartButtons {
    /// Zoom, scroll and reset, at the foot of the plot.
    pub navigation: ButtonVisibility,
    /// Move, collapse, maximize and remove, on each script pane.
    pub pane: ButtonVisibility,
}

impl Default for ChartButtons {
    fn default() -> Self {
        Self {
            navigation: ButtonVisibility::OnHover,
            pane: ButtonVisibility::OnHover,
        }
    }
}

// MARK: Margins

/// The room around the price range, as TradingView's Margins section has it: the top and bottom
/// are a share of the plot's height, the right a number of bars.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ChartMargins {
    pub top: u8,
    pub bottom: u8,
    pub right: u16,
}

impl ChartMargins {
    /// A margin is at most this share of the plot's height…
    pub const MAX_VERTICAL: u8 = 45;
    /// …and both together leave at least a fifth of it for the price.
    pub const MAX_COMBINED: u8 = 80;
    pub const MAX_RIGHT: u16 = 200;

    /// The margins forced into range: each within its bounds, the vertical pair leaving room.
    pub fn sanitized(mut self) -> Self {
        self.top = self.top.min(Self::MAX_VERTICAL);
        self.bottom = self.bottom.min(Self::MAX_VERTICAL);
        self.right = self.right.min(Self::MAX_RIGHT);
        let combined = self.top + self.bottom;
        if combined > Self::MAX_COMBINED {
            // Both are ≤ 45, so the excess is ≤ 10: take it off the larger first.
            let excess = combined - Self::MAX_COMBINED;
            if self.top >= self.bottom {
                self.top -= excess;
            } else {
                self.bottom -= excess;
            }
        }
        self
    }
}

impl Default for ChartMargins {
    fn default() -> Self {
        Self {
            top: 10,
            bottom: 8,
            right: 10,
        }
    }
}

// MARK: The canvas

/// Every canvas setting of one chart.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ChartCanvas {
    pub background: BackgroundAppearance,
    pub vertical_grid: LineAppearance,
    pub horizontal_grid: LineAppearance,
    pub pane_separator: LineAppearance,
    pub crosshair: LineAppearance,
    pub watermark: WatermarkAppearance,
    pub scales: ScaleAppearance,
    pub buttons: ChartButtons,
    pub margins: ChartMargins,
    /// Whether the box of values (open, high, low, close, volume…) under the crosshair is shown.
    /// The crosshair itself is drawn either way.
    pub ohlc_readout: bool,
}

impl Default for ChartCanvas {
    fn default() -> Self {
        Self {
            background: BackgroundAppearance::default(),
            vertical_grid: LineAppearance::default(),
            horizontal_grid: LineAppearance::default(),
            pane_separator: LineAppearance::default(),
            // TradingView draws its crosshair dashed.
            crosshair: LineAppearance::new(LineDash::Dashed),
            watermark: WatermarkAppearance::default(),
            scales: ScaleAppearance::default(),
            buttons: ChartButtons::default(),
            margins: ChartMargins::default(),
            ohlc_readout: true,
        }
    }
}

impl ChartCanvas {
    /// Values a hand-edited or stale row may hold, forced into what the renderer accepts.
    pub fn sanitized(mut self) -> Self {
        self.vertical_grid = self.vertical_grid.sanitized();
        self.horizontal_grid = self.horizontal_grid.sanitized();
        self.pane_separator = self.pane_separator.sanitized();
        self.crosshair = self.crosshair.sanitized();
        self.margins = self.margins.sanitized();
        self
    }
}

/// How many labels a scale of `height` points holds at `spacing` between them.
pub fn tick_budget(height: f32, spacing: f32) -> usize {
    if !(height.is_finite() && spacing.is_finite()) || height <= 0.0 || spacing <= 0.0 {
        return 0;
    }
    ((height / spacing).floor() as usize).max(1)
}

/// Room the price axis keeps around its widest label: the label's right inset plus the margin a
/// price tag needs on both sides.
const AXIS_PADDING: f32 = 17.0;
/// The axis grows in steps of this many points, so a label gaining a pixel does not resize the plot.
const AXIS_STEP: f32 = 4.0;

/// The width of the price axis that holds `widest_label` points of text: never under `floor`, never
/// over `ceiling`, and rounded up to a whole step.
pub fn axis_width(widest_label: f32, floor: f32, ceiling: f32) -> f32 {
    let wanted = if widest_label.is_finite() {
        widest_label.max(0.0) + AXIS_PADDING
    } else {
        0.0
    };
    let stepped = (wanted / AXIS_STEP).ceil() * AXIS_STEP;
    stepped.clamp(floor, ceiling.max(floor))
}

// MARK: Time ticks

/// Which of the `len` shown bars carry a time label and a vertical grid line: every `step`-th bar
/// counted from the first bar of the whole series (`first_shown` is the index of the first shown
/// bar), so a label stays on its bar while the chart pans instead of sliding to another one.
/// `label_width` is how wide one label is drawn, in points.
pub fn time_ticks(first_shown: usize, len: usize, slot_width: f32, label_width: f32) -> Vec<usize> {
    let step = time_tick_step(slot_width, label_width);
    (0..len)
        .filter(|index| (first_shown + index).is_multiple_of(step))
        .collect()
}

/// The least room between two time labels' centres, in points.
const TIME_LABEL_SPACING: f32 = 100.0;
/// The least gap between two time labels, in points.
const TIME_LABEL_GAP: f32 = 16.0;

/// A label about every 100 points, however many bars share them (a price-based chart may show a
/// dozen bars across the plot), or further apart when labels `label_width` wide would not leave
/// a gap between them: a large axis font, or a picture's larger text.
pub fn time_tick_step(slot_width: f32, label_width: f32) -> usize {
    let slot = if slot_width.is_finite() {
        slot_width.max(1.0)
    } else {
        1.0
    };
    let label = if label_width.is_finite() {
        label_width.max(0.0)
    } else {
        0.0
    };
    let spacing = TIME_LABEL_SPACING.max(label + TIME_LABEL_GAP);
    ((spacing / slot).ceil() as usize).clamp(1, 1_000_000)
}

// MARK: Price range

/// The prices the scale spans for data from `low` to `high`, leaving `margins.top` and
/// `margins.bottom` percent of the plot's height above and below it. Margins change where the
/// data sits in the plot, never the prices themselves. A logarithmic scale leaves them in log
/// space, so they are the same share of the height at every price.
pub fn price_range(low: f64, high: f64, margins: ChartMargins, logarithmic: bool) -> (f64, f64) {
    if !(low.is_finite() && high.is_finite()) {
        return (0.0, 1.0);
    }
    let (low, high) = if low <= high {
        (low, high)
    } else {
        (high, low)
    };
    let margins = margins.sanitized();
    let top = f64::from(margins.top) / 100.0;
    let bottom = f64::from(margins.bottom) / 100.0;
    let usable = 1.0 - top - bottom;
    if logarithmic && low > 0.0 {
        let (low, high) = (low.ln(), high.ln());
        let (minimum, maximum) = spread(low, high, top, bottom, usable);
        let (minimum, maximum) = (minimum.exp(), maximum.exp());
        if minimum.is_finite() && maximum.is_finite() && minimum > 0.0 && minimum < maximum {
            return (minimum, maximum);
        }
        return (low.exp(), high.exp());
    }
    let (minimum, maximum) = spread(low, high, top, bottom, usable);
    if minimum.is_finite() && maximum.is_finite() && minimum < maximum {
        (minimum, maximum)
    } else {
        (low - 1.0, high + 1.0)
    }
}

fn spread(low: f64, high: f64, top: f64, bottom: f64, usable: f64) -> (f64, f64) {
    let middle = (low + high) / 2.0;
    // A flat series still needs a span, or it would have no scale at all.
    let floor = (middle.abs() * 1e-4).max(1e-9);
    let span = (high - low).max(floor);
    let total = span / usable.max(0.2);
    let centred = if high - low < floor {
        // Centre the flat series on its margins' weight rather than on one edge.
        middle - span / 2.0
    } else {
        low
    };
    (centred - total * bottom, centred + span + total * top)
}

// MARK: Panes

/// Height of a collapsed pane: its title strip.
pub const COLLAPSED_PANE_HEIGHT: f32 = 24.0;
/// The share of the chart the expanded script panes share.
pub const PANE_SHARE: f32 = 0.3;
/// The least the main chart keeps while panes share the rest.
const MIN_MAIN_HEIGHT: f32 = 90.0;
/// The smallest a maximized pane is worth drawing.
const MIN_MAXIMIZED_HEIGHT: f32 = 60.0;

/// What the layout needs to know of one script pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneSpec {
    pub id: u64,
    pub collapsed: bool,
}

/// The vertical split of a chart into its main area (plot and time axis) and its script panes.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneHeights {
    pub main: f32,
    /// One height per pane, in order.
    pub panes: Vec<f32>,
    /// The main area is only its time axis: a pane is maximized.
    pub main_collapsed: bool,
}

/// Splits `total` points of height. Expanded panes share [`PANE_SHARE`] of it, a collapsed pane is
/// its title strip, and a maximized pane takes everything but the strips and the time axis. A
/// chart too short for any of that keeps the plain split.
pub fn split_panes(
    total: f32,
    panes: &[PaneSpec],
    maximized: Option<u64>,
    time_axis: f32,
) -> PaneHeights {
    let total = if total.is_finite() {
        total.max(0.0)
    } else {
        0.0
    };
    if panes.is_empty() {
        return PaneHeights {
            main: total,
            panes: Vec::new(),
            main_collapsed: false,
        };
    }
    if let Some(id) = maximized
        && panes.iter().any(|pane| pane.id == id)
    {
        let strips = (panes.len() - 1) as f32 * COLLAPSED_PANE_HEIGHT;
        let room = total - time_axis - strips;
        if room >= MIN_MAXIMIZED_HEIGHT {
            return PaneHeights {
                main: time_axis,
                panes: panes
                    .iter()
                    .map(|pane| {
                        if pane.id == id {
                            room.round()
                        } else {
                            COLLAPSED_PANE_HEIGHT
                        }
                    })
                    .collect(),
                main_collapsed: true,
            };
        }
    }
    let expanded = panes.iter().filter(|pane| !pane.collapsed).count();
    let collapsed = panes.len() - expanded;
    let strips = collapsed as f32 * COLLAPSED_PANE_HEIGHT;
    let mut shared = if expanded == 0 {
        0.0
    } else {
        (total * PANE_SHARE).round()
    };
    // The main chart keeps its minimum: the panes give way, down to nothing but their strips.
    let allowed = (total - MIN_MAIN_HEIGHT.min(total * 0.6) - strips).max(0.0);
    shared = shared.min(allowed);
    let each = if expanded == 0 {
        0.0
    } else {
        (shared / expanded as f32).round()
    };
    let mut heights: Vec<f32> = panes
        .iter()
        .map(|pane| {
            if pane.collapsed {
                COLLAPSED_PANE_HEIGHT
            } else {
                each
            }
        })
        .collect();
    // Rounding leaves the last expanded pane the remainder, so the split fills the area exactly.
    if let Some(last) = panes.iter().rposition(|pane| !pane.collapsed) {
        let used: f32 = heights.iter().sum::<f32>() - heights[last];
        heights[last] = (shared + strips - used).max(0.0);
    }
    let main = (total - heights.iter().sum::<f32>()).max(0.0);
    PaneHeights {
        main,
        panes: heights,
        main_collapsed: false,
    }
}

#[cfg(test)]
mod tests;
