//! The drawing tools: which exist, how each is placed, and which style controls it offers.
//!
//! One table (`Tool::spec`) answers every question about a tool, so the toolbar, the floating
//! toolbar, the properties dialog and the interaction layer never grow a per-tool `if` ladder.
//! Pure: no egui types.

use super::chart_type::{LineDash, Rgba};
use super::drawing_style::{
    ArrowStats, DrawingStyle, HorizontalAlign, LineEnd, StatsPosition, TextOrientation,
    VerticalAlign,
};
use super::drawings::{LONG_TEXT_CHARS, MAX_TEXT_CHARS};
use super::fib_style::FibStyle;
use serde::{Deserialize, Serialize};

/// A drawing tool armed on the tool strip. `Cursor` is the crosshair (default).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Tool {
    #[default]
    Cursor,
    TrendLine,
    FibRetracement,
    FibExtension,
    FibChannel,
    FibTimeZone,
    FibSpeedFan,
    FibTrendTime,
    FibCircles,
    FibSpiral,
    FibSpeedArcs,
    FibWedge,
    Pitchfan,
    Brush,
    Ruler,
    Highlighter,
    ArrowMarker,
    Arrow,
    ArrowMarkUp,
    ArrowMarkDown,
    Rectangle,
    RotatedRectangle,
    Path,
    Circle,
    Ellipse,
    Polyline,
    Triangle,
    Arc,
    Curve,
    DoubleCurve,
    Ray,
    InfoLine,
    ExtendedLine,
    TrendAngle,
    HorizontalLine,
    HorizontalRay,
    VerticalLine,
    Crossline,
    ParallelChannel,
    RegressionTrend,
    FlatChannel,
    DisjointChannel,
    Pitchfork,
    SchiffPitchfork,
    ModifiedSchiffPitchfork,
    InsidePitchfork,
    Text,
    Comment,
    PriceLabel,
    FlagMark,
    Pin,
    Note,
    PriceNote,
    Callout,
    Signpost,
    Table,
    Image,
    Post,
    XabcdPattern,
    CypherPattern,
    HeadAndShoulders,
    AbcdPattern,
    TrianglePattern,
    ThreeDrivesPattern,
    ElliottImpulse,
    ElliottCorrection,
    ElliottTriangle,
    ElliottDoubleCombo,
    ElliottTripleCombo,
    CyclicLines,
    TimeCycles,
    SineLine,
    LongPosition,
    ShortPosition,
    PositionForecast,
    BarsPattern,
    GhostFeed,
    Sector,
    AnchoredVwap,
    FixedRangeVolumeProfile,
    AnchoredVolumeProfile,
    PriceRange,
    DateRange,
    DateAndPriceRange,
}

/// Where a tool sits in the toolbar's menus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolGroup {
    /// The crosshair, which draws nothing.
    Pointer,
    /// The Fibonacci and Gann tile's first section.
    Fibonacci,
    Gann,
    Measure,
    Brushes,
    Arrows,
    Shapes,
    /// The trend line and the other lines: the first menu of the Lines tile.
    TrendLines,
    Channels,
    Pitchforks,
    /// Text on the chart: notes, labels, markers.
    TextNotes,
    /// Pictures and links, which need something chosen before they can be placed.
    Content,
    /// The harmonic and structural chart patterns: the first menu of the Patterns tile.
    ChartPatterns,
    ElliottWaves,
    Cycles,
    /// Positions, forecasts and the sector: the first menu of the Forecast tile.
    Forecasting,
    VolumeBased,
    Measurers,
}

impl ToolGroup {
    /// The groups listed in the brushes, arrows and shapes menu, in order.
    pub const MENU: [Self; 3] = [Self::Brushes, Self::Arrows, Self::Shapes];
    /// The groups listed in the Lines tile's menu, in order.
    pub const LINE_MENU: [Self; 3] = [Self::TrendLines, Self::Channels, Self::Pitchforks];
    /// The groups listed in the text and notes tile's menu, in order.
    pub const NOTE_MENU: [Self; 2] = [Self::TextNotes, Self::Content];
    /// The groups listed in the patterns, waves and cycles tile's menu, in order.
    pub const PATTERN_MENU: [Self; 3] = [Self::ChartPatterns, Self::ElliottWaves, Self::Cycles];
    /// The groups listed in the forecasting, volume and measurement tile's menu, in order.
    pub const FORECAST_MENU: [Self; 3] = [Self::Forecasting, Self::VolumeBased, Self::Measurers];
    /// The groups listed in the Fibonacci and Gann tile's menu, in order.
    pub const FIB_MENU: [Self; 2] = [Self::Fibonacci, Self::Gann];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Pointer => "Cursor",
            Self::TrendLines => "Lines",
            Self::Fibonacci => "Fibonacci",
            Self::Gann => "Gann",
            Self::Measure => "Measure",
            Self::Brushes => "Brushes",
            Self::Arrows => "Arrows",
            Self::Shapes => "Shapes",
            Self::Channels => "Channels",
            Self::Pitchforks => "Pitchforks",
            Self::TextNotes => "Text and notes",
            Self::Content => "Content",
            Self::ChartPatterns => "Chart patterns",
            Self::ElliottWaves => "Elliott waves",
            Self::Cycles => "Cycles",
            Self::Forecasting => "Forecasting and measurement",
            Self::VolumeBased => "Volume-based",
            Self::Measurers => "Measurers",
        }
    }
}

/// How a tool's anchors are placed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placement {
    /// Press, drag, release: every sampled pointer position is a point.
    Freehand,
    /// One click.
    Single,
    /// Exactly this many anchors. Drag places the first two; clicks place the rest.
    Points(u8),
    /// Any number of vertices, each placed by a click, finished by a double-click, Enter or
    /// Escape. `closable` lets a click on the first vertex close the shape.
    Open { closable: bool },
}

/// The style controls a tool offers. A tool shows exactly these in its floating toolbar and its
/// properties dialog; nothing else is drawn or editable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Caps(u64);

impl Caps {
    pub const NONE: Self = Self(0);
    pub const COLOR: Self = Self(1);
    pub const WIDTH: Self = Self(1 << 1);
    pub const DASH: Self = Self(1 << 2);
    pub const FILL: Self = Self(1 << 3);
    pub const TEXT: Self = Self(1 << 4);
    pub const EXTEND: Self = Self(1 << 5);
    pub const MIDDLE_LINE: Self = Self(1 << 6);
    pub const ENDS: Self = Self(1 << 7);
    pub const STATS: Self = Self(1 << 8);
    pub const PRICE_LABELS: Self = Self(1 << 9);
    pub const MIDDLE_POINT: Self = Self(1 << 10);
    /// Extra lines at editable ratios (a channel's, a pitchfork's).
    pub const LEVELS: Self = Self(1 << 11);
    /// The regression trend's source, deviations and lines.
    pub const REGRESSION: Self = Self(1 << 12);
    /// Text that can be turned along, across or up the line.
    pub const ORIENTATION: Self = Self(1 << 13);
    /// A tag on the time axis.
    pub const TIME_LABEL: Self = Self(1 << 14);
    /// Text typed in place on the chart, where it sits (a line's, a note's), rather than in the
    /// dialog alone.
    pub const INLINE_TEXT: Self = Self(1 << 15);
    /// A text box: its background, border and wrapping.
    pub const BOX: Self = Self(1 << 16);
    /// Nothing without its text: placing it opens the editor, and it is added once it has some.
    pub const TEXT_FIRST: Self = Self(1 << 17);
    /// The tool writes its text itself (a price): the dialog offers the font, not the content.
    pub const AUTO_TEXT: Self = Self(1 << 18);
    /// A note that is shown only once its marker is clicked (a pin's).
    pub const COLLAPSIBLE: Self = Self(1 << 19);
    /// An emoji at the anchor, shown or not, and what it sits on.
    pub const EMOJI: Self = Self(1 << 20);
    /// A grid of editable cells: edited cell by cell, pinned to the chart or to the screen.
    pub const TABLE: Self = Self(1 << 21);
    /// Something chosen first (a picture, a link) and placed by a click; it has a Content page.
    pub const CONTENT: Self = Self(1 << 22);
    /// A chart pattern's pivot labels and the Fibonacci ratios printed between its legs.
    pub const PATTERN: Self = Self(1 << 23);
    /// An Elliott wave's degree, which writes its labels.
    pub const WAVE: Self = Self(1 << 24);
    /// A position's account, risk and quantity, its stats and its zone colours.
    pub const POSITION: Self = Self(1 << 25);
    /// A bars pattern's mode and reflections.
    pub const BARS: Self = Self(1 << 26);
    /// A ghost feed's average range, variance and candle colours.
    pub const GHOST: Self = Self(1 << 27);
    /// An anchored VWAP's source and bands.
    pub const VWAP: Self = Self(1 << 28);
    /// A volume profile's rows, value area and look.
    pub const PROFILE: Self = Self(1 << 29);
    /// What a measurer prints.
    pub const MEASURE: Self = Self(1 << 30);
    /// A position forecast's success and failure colours.
    pub const FORECAST: Self = Self(1 << 31);
    /// A Fibonacci or Gann tool's level lists and the switches of its `FibStyle`.
    pub const FIB: Self = Self(1 << 32);
    /// Colour and thickness.
    pub const STROKE: Self = Self(Self::COLOR.0 | Self::WIDTH.0);
    /// Colour, thickness and line pattern.
    pub const LINE: Self = Self(Self::STROKE.0 | Self::DASH.0);

    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// These capabilities without `other`.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn has(self, other: Self) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }

    #[cfg(test)]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToolSpec {
    pub label: &'static str,
    pub group: ToolGroup,
    pub placement: Placement,
    pub caps: Caps,
}

const fn spec(label: &'static str, group: ToolGroup, placement: Placement, caps: Caps) -> ToolSpec {
    ToolSpec {
        label,
        group,
        placement,
        caps,
    }
}

/// How a drawing's anchors snap to the bars' open, high, low and close.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MagnetMode {
    #[default]
    Off,
    /// Snaps only when the pointer is already near a price of the bar.
    Weak,
    /// Always snaps to the nearest price of the nearest bar.
    Strong,
}

impl MagnetMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Magnet off",
            Self::Weak => "Weak magnet",
            Self::Strong => "Strong magnet",
        }
    }

    /// The next mode when the toolbar button is clicked.
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Weak,
            Self::Weak => Self::Strong,
            Self::Strong => Self::Off,
        }
    }

    /// The mode in force while the invert key (Ctrl/⌘) is held: it turns the magnet on when it is
    /// off (weak) and off when it is on.
    pub const fn inverted(self, held: bool) -> Self {
        match (held, self) {
            (false, mode) => mode,
            (true, Self::Off) => Self::Weak,
            (true, _) => Self::Off,
        }
    }
}

/// The orange the Brush has always drawn in.
const BRUSH_ORANGE: Rgba = [255, 159, 10, 255];
const FILL_ALPHA: u8 = 51;

impl Tool {
    /// Every tool that lives in a flyout menu (the Lines tile's or the brushes, arrows and
    /// shapes tile's), in display order within its group.
    pub const GROUPED: [Self; 82] = [
        Self::FibRetracement,
        Self::FibExtension,
        Self::FibChannel,
        Self::FibTimeZone,
        Self::FibSpeedFan,
        Self::FibTrendTime,
        Self::FibCircles,
        Self::FibSpiral,
        Self::FibSpeedArcs,
        Self::FibWedge,
        Self::Pitchfan,
        Self::TrendLine,
        Self::Ray,
        Self::InfoLine,
        Self::ExtendedLine,
        Self::TrendAngle,
        Self::HorizontalLine,
        Self::HorizontalRay,
        Self::VerticalLine,
        Self::Crossline,
        Self::ParallelChannel,
        Self::RegressionTrend,
        Self::FlatChannel,
        Self::DisjointChannel,
        Self::Pitchfork,
        Self::SchiffPitchfork,
        Self::ModifiedSchiffPitchfork,
        Self::InsidePitchfork,
        Self::Text,
        Self::Note,
        Self::PriceNote,
        Self::Pin,
        Self::Table,
        Self::Callout,
        Self::Comment,
        Self::PriceLabel,
        Self::Signpost,
        Self::FlagMark,
        Self::Image,
        Self::Post,
        Self::Brush,
        Self::Highlighter,
        Self::ArrowMarker,
        Self::Arrow,
        Self::ArrowMarkUp,
        Self::ArrowMarkDown,
        Self::Rectangle,
        Self::RotatedRectangle,
        Self::Path,
        Self::Circle,
        Self::Ellipse,
        Self::Polyline,
        Self::Triangle,
        Self::Arc,
        Self::Curve,
        Self::DoubleCurve,
        Self::XabcdPattern,
        Self::CypherPattern,
        Self::HeadAndShoulders,
        Self::AbcdPattern,
        Self::TrianglePattern,
        Self::ThreeDrivesPattern,
        Self::ElliottImpulse,
        Self::ElliottCorrection,
        Self::ElliottTriangle,
        Self::ElliottDoubleCombo,
        Self::ElliottTripleCombo,
        Self::CyclicLines,
        Self::TimeCycles,
        Self::SineLine,
        Self::LongPosition,
        Self::ShortPosition,
        Self::PositionForecast,
        Self::BarsPattern,
        Self::GhostFeed,
        Self::Sector,
        Self::AnchoredVwap,
        Self::FixedRangeVolumeProfile,
        Self::AnchoredVolumeProfile,
        Self::PriceRange,
        Self::DateRange,
        Self::DateAndPriceRange,
    ];

    pub const fn spec(self) -> ToolSpec {
        use Placement::{Freehand, Open, Points, Single};
        use ToolGroup::{
            Arrows, Brushes, Channels, ChartPatterns, Content, Cycles, ElliottWaves, Fibonacci,
            Forecasting, Measure, Measurers, Pitchforks, Pointer, Shapes, TextNotes, TrendLines,
            VolumeBased,
        };
        let shape_line = Caps::LINE.with(Caps::FILL);
        // A Fibonacci tool: its trend line's stroke, its background and its level lists.
        let fib_levels = Caps::LINE.with(Caps::FILL).with(Caps::FIB);
        // What every two-point line offers: text along it, ends, statistics and price tags.
        let line = Caps::LINE
            .with(Caps::EXTEND)
            .with(Caps::ENDS)
            .with(Caps::MIDDLE_POINT)
            .with(Caps::PRICE_LABELS)
            .with(Caps::STATS)
            .with(Caps::TEXT)
            .with(Caps::INLINE_TEXT);
        // A line at one price or one time.
        let level_line = Caps::LINE
            .with(Caps::TEXT)
            .with(Caps::INLINE_TEXT)
            .with(Caps::PRICE_LABELS)
            .with(Caps::ORIENTATION);
        let channel = shape_line
            .with(Caps::EXTEND)
            .with(Caps::TEXT)
            .with(Caps::INLINE_TEXT);
        let fork = Caps::LINE
            .with(Caps::FILL)
            .with(Caps::EXTEND)
            .with(Caps::LEVELS);
        // A box of text on the chart: typed in place, with a background and a border.
        let note = Caps::BOX
            .with(Caps::FILL)
            .with(Caps::TEXT)
            .with(Caps::INLINE_TEXT);
        // A pattern: its border and background, the font of its labels, and what it prints.
        let pattern = Caps::LINE
            .with(Caps::FILL)
            .with(Caps::AUTO_TEXT)
            .with(Caps::PATTERN);
        // An Elliott wave: its line, the font of its labels, and the degree that writes them.
        let wave = Caps::LINE.with(Caps::AUTO_TEXT).with(Caps::WAVE);
        // A position: its line, the font of its stats, the prices on the axis and its account.
        let position = Caps::COLOR
            .with(Caps::WIDTH)
            .with(Caps::AUTO_TEXT)
            .with(Caps::PRICE_LABELS)
            .with(Caps::POSITION);
        // A measurer: its line and background, what it prints, and a note.
        let measure = shape_line
            .with(Caps::MEASURE)
            .with(Caps::TEXT)
            .with(Caps::AUTO_TEXT);
        match self {
            Self::Cursor => spec("Crosshair", Pointer, Single, Caps::NONE),
            Self::TrendLine => spec("Trend line", TrendLines, Points(2), line),
            Self::Ray => spec("Ray", TrendLines, Points(2), line),
            Self::InfoLine => spec("Info line", TrendLines, Points(2), line),
            Self::ExtendedLine => spec("Extended line", TrendLines, Points(2), line),
            Self::TrendAngle => spec(
                "Trend angle",
                TrendLines,
                Points(2),
                line.without(Caps::ENDS),
            ),
            Self::HorizontalLine => spec(
                "Horizontal line",
                TrendLines,
                Single,
                level_line.without(Caps::ORIENTATION),
            ),
            Self::HorizontalRay => spec(
                "Horizontal ray",
                TrendLines,
                Single,
                level_line.without(Caps::ORIENTATION),
            ),
            Self::VerticalLine => spec(
                "Vertical line",
                TrendLines,
                Single,
                level_line
                    .without(Caps::PRICE_LABELS)
                    .with(Caps::TIME_LABEL),
            ),
            Self::Crossline => spec(
                "Crossline",
                TrendLines,
                Single,
                Caps::LINE.with(Caps::PRICE_LABELS).with(Caps::TIME_LABEL),
            ),
            Self::ParallelChannel => spec(
                "Parallel channel",
                Channels,
                Points(3),
                channel.with(Caps::MIDDLE_LINE).with(Caps::LEVELS),
            ),
            Self::RegressionTrend => spec(
                "Regression trend",
                Channels,
                Points(2),
                Caps::FILL.with(Caps::EXTEND).with(Caps::REGRESSION),
            ),
            Self::FlatChannel => spec(
                "Flat top/bottom",
                Channels,
                Points(3),
                channel.with(Caps::PRICE_LABELS),
            ),
            Self::DisjointChannel => spec(
                "Disjoint channel",
                Channels,
                Points(3),
                channel.with(Caps::PRICE_LABELS),
            ),
            Self::Pitchfork => spec("Pitchfork", Pitchforks, Points(3), fork),
            Self::SchiffPitchfork => spec("Schiff pitchfork", Pitchforks, Points(3), fork),
            Self::ModifiedSchiffPitchfork => {
                spec("Modified Schiff pitchfork", Pitchforks, Points(3), fork)
            }
            Self::InsidePitchfork => spec("Inside pitchfork", Pitchforks, Points(3), fork),
            Self::Text => spec("Text", TextNotes, Single, note.with(Caps::TEXT_FIRST)),
            Self::Comment => spec("Comment", TextNotes, Single, note.with(Caps::TEXT_FIRST)),
            Self::PriceLabel => spec(
                "Price label",
                TextNotes,
                Single,
                Caps::BOX.with(Caps::FILL).with(Caps::AUTO_TEXT),
            ),
            Self::FlagMark => spec("Flag mark", TextNotes, Single, Caps::COLOR),
            Self::Pin => spec(
                "Pin",
                TextNotes,
                Single,
                note.with(Caps::COLOR).with(Caps::COLLAPSIBLE),
            ),
            Self::Note => spec(
                "Note",
                TextNotes,
                Points(2),
                note.with(Caps::STROKE)
                    .with(Caps::DASH)
                    .with(Caps::TEXT_FIRST),
            ),
            Self::PriceNote => spec("Price note", TextNotes, Points(2), note.with(Caps::COLOR)),
            Self::Callout => spec("Callout", TextNotes, Points(2), note.with(Caps::TEXT_FIRST)),
            Self::Image => spec("Image", Content, Single, Caps::CONTENT),
            Self::Post => spec("Post", Content, Single, Caps::CONTENT),
            Self::Table => spec(
                "Table",
                TextNotes,
                Single,
                Caps::BOX
                    .with(Caps::FILL)
                    .with(Caps::AUTO_TEXT)
                    .with(Caps::TABLE),
            ),
            Self::Signpost => spec(
                "Signpost",
                TextNotes,
                Points(2),
                Caps::FILL
                    .with(Caps::TEXT)
                    .with(Caps::INLINE_TEXT)
                    .with(Caps::TEXT_FIRST)
                    .with(Caps::EMOJI),
            ),
            Self::FibRetracement => spec(
                "Fib retracement",
                Fibonacci,
                Points(2),
                fib_levels.with(Caps::EXTEND),
            ),
            Self::FibExtension => spec(
                "Trend-based fib extension",
                Fibonacci,
                Points(3),
                fib_levels.with(Caps::EXTEND),
            ),
            Self::FibChannel => spec(
                "Fib channel",
                Fibonacci,
                Points(3),
                fib_levels.with(Caps::EXTEND),
            ),
            Self::FibTimeZone => spec("Fib time zone", Fibonacci, Points(2), fib_levels),
            Self::FibSpeedFan => spec(
                "Fib speed resistance fan",
                Fibonacci,
                Points(2),
                Caps::FILL.with(Caps::FIB),
            ),
            Self::FibTrendTime => spec("Trend-based fib time", Fibonacci, Points(3), fib_levels),
            Self::FibCircles => spec("Fib circles", Fibonacci, Points(2), fib_levels),
            Self::FibSpiral => spec(
                "Fib spiral",
                Fibonacci,
                Points(2),
                Caps::LINE.with(Caps::FIB),
            ),
            Self::FibSpeedArcs => spec(
                "Fib speed resistance arcs",
                Fibonacci,
                Points(2),
                fib_levels,
            ),
            Self::FibWedge => spec("Fib wedge", Fibonacci, Points(3), fib_levels),
            Self::Pitchfan => spec("Pitchfan", Fibonacci, Points(3), fib_levels),
            Self::Ruler => spec("Ruler", Measure, Points(2), Caps::NONE),
            Self::Brush => spec("Brush", Brushes, Freehand, Caps::STROKE),
            Self::Highlighter => spec("Highlighter", Brushes, Freehand, Caps::STROKE),
            Self::ArrowMarker => spec(
                "Arrow marker",
                Arrows,
                Points(2),
                Caps::STROKE.with(Caps::TEXT),
            ),
            Self::Arrow => spec(
                "Arrow",
                Arrows,
                Points(2),
                Caps::LINE
                    .with(Caps::ENDS)
                    .with(Caps::MIDDLE_POINT)
                    .with(Caps::PRICE_LABELS)
                    .with(Caps::STATS)
                    .with(Caps::TEXT),
            ),
            Self::ArrowMarkUp => spec(
                "Arrow mark up",
                Arrows,
                Single,
                Caps::COLOR.with(Caps::TEXT),
            ),
            Self::ArrowMarkDown => spec(
                "Arrow mark down",
                Arrows,
                Single,
                Caps::COLOR.with(Caps::TEXT),
            ),
            Self::Rectangle => spec(
                "Rectangle",
                Shapes,
                Points(2),
                shape_line
                    .with(Caps::TEXT)
                    .with(Caps::EXTEND)
                    .with(Caps::MIDDLE_LINE),
            ),
            Self::RotatedRectangle => spec("Rotated rectangle", Shapes, Points(3), shape_line),
            Self::Path => spec(
                "Path",
                Shapes,
                Open { closable: false },
                Caps::LINE.with(Caps::ENDS),
            ),
            Self::Circle => spec("Circle", Shapes, Points(2), shape_line.with(Caps::TEXT)),
            Self::Ellipse => spec("Ellipse", Shapes, Points(2), shape_line.with(Caps::TEXT)),
            Self::Polyline => spec("Polyline", Shapes, Open { closable: true }, shape_line),
            Self::Triangle => spec("Triangle", Shapes, Points(3), shape_line),
            Self::Arc => spec("Arc", Shapes, Points(3), shape_line),
            Self::Curve => spec(
                "Curve",
                Shapes,
                Points(3),
                shape_line.with(Caps::ENDS).with(Caps::EXTEND),
            ),
            Self::DoubleCurve => spec(
                "Double curve",
                Shapes,
                Points(4),
                shape_line.with(Caps::ENDS).with(Caps::EXTEND),
            ),
            Self::XabcdPattern => spec("XABCD pattern", ChartPatterns, Points(5), pattern),
            Self::CypherPattern => spec("Cypher pattern", ChartPatterns, Points(5), pattern),
            Self::HeadAndShoulders => spec("Head and shoulders", ChartPatterns, Points(7), pattern),
            Self::AbcdPattern => spec(
                "ABCD pattern",
                ChartPatterns,
                Points(4),
                pattern.without(Caps::FILL),
            ),
            Self::TrianglePattern => spec("Triangle pattern", ChartPatterns, Points(4), pattern),
            Self::ThreeDrivesPattern => {
                spec("Three drives pattern", ChartPatterns, Points(6), pattern)
            }
            Self::ElliottImpulse => spec(
                "Elliott impulse wave (1\u{b7}2\u{b7}3\u{b7}4\u{b7}5)",
                ElliottWaves,
                Points(6),
                wave,
            ),
            Self::ElliottCorrection => spec(
                "Elliott correction wave (A\u{b7}B\u{b7}C)",
                ElliottWaves,
                Points(4),
                wave,
            ),
            Self::ElliottTriangle => spec(
                "Elliott triangle wave (A\u{b7}B\u{b7}C\u{b7}D\u{b7}E)",
                ElliottWaves,
                Points(6),
                wave,
            ),
            Self::ElliottDoubleCombo => spec(
                "Elliott double combo wave (W\u{b7}X\u{b7}Y)",
                ElliottWaves,
                Points(4),
                wave,
            ),
            Self::ElliottTripleCombo => spec(
                "Elliott triple combo wave (W\u{b7}X\u{b7}Y\u{b7}X\u{b7}Z)",
                ElliottWaves,
                Points(6),
                wave,
            ),
            Self::CyclicLines => spec("Cyclic lines", Cycles, Points(2), Caps::LINE),
            Self::TimeCycles => spec(
                "Time cycles",
                Cycles,
                Points(2),
                Caps::LINE.with(Caps::FILL),
            ),
            Self::SineLine => spec("Sine line", Cycles, Points(2), Caps::LINE),
            Self::LongPosition => spec("Long position", Forecasting, Single, position),
            Self::ShortPosition => spec("Short position", Forecasting, Single, position),
            Self::PositionForecast => spec(
                "Position forecast",
                Forecasting,
                Points(2),
                Caps::LINE.with(Caps::AUTO_TEXT).with(Caps::FORECAST),
            ),
            Self::BarsPattern => spec(
                "Bars pattern",
                Forecasting,
                Points(3),
                Caps::COLOR.with(Caps::BARS),
            ),
            Self::GhostFeed => spec("Ghost feed", Forecasting, Points(2), Caps::GHOST),
            Self::Sector => spec("Sector", Forecasting, Points(3), shape_line),
            Self::AnchoredVwap => spec(
                "Anchored VWAP",
                VolumeBased,
                Single,
                Caps::LINE.with(Caps::VWAP),
            ),
            Self::FixedRangeVolumeProfile => spec(
                "Fixed range volume profile",
                VolumeBased,
                Points(2),
                Caps::PROFILE,
            ),
            Self::AnchoredVolumeProfile => spec(
                "Anchored volume profile",
                VolumeBased,
                Single,
                Caps::PROFILE,
            ),
            Self::PriceRange => spec("Price range", Measurers, Points(2), measure),
            Self::DateRange => spec("Date range", Measurers, Points(2), measure),
            Self::DateAndPriceRange => spec("Date and price range", Measurers, Points(2), measure),
        }
    }

    pub const fn label(self) -> &'static str {
        self.spec().label
    }

    pub const fn caps(self) -> Caps {
        self.spec().caps
    }

    /// The most characters of text a drawing by this tool may carry.
    pub const fn max_text(self) -> usize {
        match self {
            Self::Pin | Self::Note | Self::Callout => LONG_TEXT_CHARS,
            _ => MAX_TEXT_CHARS,
        }
    }

    /// The tools of one menu group, in menu order.
    pub fn in_group(group: ToolGroup) -> impl Iterator<Item = Self> {
        Self::GROUPED
            .into_iter()
            .filter(move |tool| tool.spec().group == group)
    }

    /// The tool's own look, before the user changes it (TradingView's defaults, except the
    /// Brush, which keeps the orange it has always had).
    pub fn default_style(self) -> DrawingStyle {
        let tinted = |color: Rgba, fill: bool| DrawingStyle::colored(color, FILL_ALPHA, fill);
        // A filled box with white text and a border, as the Charting Library's bubbles.
        let text_box = |fill: Rgba, border: Rgba, bordered: bool| {
            let mut style = DrawingStyle::colored(fill, fill[3], true);
            style.text.color = [255, 255, 255, 255];
            style.border.color = border;
            style.border.enabled = bordered;
            style
        };
        // The Charting Library's line: its blue, 2 px, nothing extended, no tags.
        // A Fibonacci tool: the Charting Library's grey 2 px trend line, a 20 % background in each
        // level's own colour, and 12 pt labels (which take each level's colour).
        let fib_style = |tool: Self, trend: Rgba, dash: LineDash| {
            let mut style = tinted(trend, true);
            style.stroke.dash = dash;
            style.text.size = 12.0;
            style.fib = FibStyle::for_tool(tool);
            style
        };
        let line_style = |change: fn(&mut DrawingStyle)| {
            let mut style = tinted([41, 98, 255, 255], false);
            // Text below the line, as the Charting Library has it.
            style.text.valign = VerticalAlign::Bottom;
            change(&mut style);
            style
        };
        // A pattern, as the Charting Library draws a new one: a 2 px line, a background at the
        // given opacity (none for the ABCD pattern, which has no background) and 12 px white
        // labels on plates of the line's colour.
        let pattern_style = |color: Rgba, fill_alpha: u8| {
            let mut style = DrawingStyle::colored(color, fill_alpha, fill_alpha > 0);
            style.stroke.width = 2.0;
            style.text.color = [255, 255, 255, 255];
            style.text.size = 12.0;
            style
        };
        // An Elliott wave: its line and its labels in one colour, 2 px, labels a little larger
        // than a pattern's and bold (**inferred**: the library has no label overrides for them).
        let wave_style = |color: Rgba| {
            let mut style = DrawingStyle::colored(color, 0, false);
            style.stroke.width = 2.0;
            style.text.size = 14.0;
            style.text.bold = true;
            style
        };
        let mut style = match self {
            Self::Brush => tinted(BRUSH_ORANGE, false),
            Self::Highlighter => {
                let mut style = tinted([242, 54, 69, FILL_ALPHA], false);
                style.stroke.width = 20.0;
                style
            }
            Self::ArrowMarker => {
                let mut style = tinted([30, 83, 229, 255], false);
                style.text.size = 16.0;
                style.text.bold = true;
                style
            }
            Self::Arrow => {
                let mut style = tinted([41, 98, 255, 255], false);
                style.head_end = LineEnd::Arrow;
                style
            }
            Self::ArrowMarkUp => tinted([8, 153, 129, 255], false),
            Self::ArrowMarkDown => tinted([204, 47, 60, 255], false),
            Self::Rectangle => tinted([156, 39, 176, 255], true),
            Self::RotatedRectangle => tinted([76, 175, 80, 255], true),
            Self::Path => {
                let mut style = tinted([41, 98, 255, 255], false);
                style.head_end = LineEnd::Arrow;
                style
            }
            Self::Circle => tinted([255, 152, 0, 255], true),
            Self::Ellipse => tinted([242, 54, 69, 255], true),
            Self::Polyline => tinted([0, 188, 212, 255], true),
            Self::Triangle => tinted([8, 153, 129, 255], true),
            Self::Arc => tinted([233, 30, 99, 255], true),
            Self::Curve => tinted([41, 98, 255, 255], false),
            Self::DoubleCurve => tinted([103, 58, 183, 255], false),
            Self::TrendLine => DrawingStyle::trend_line(),
            Self::Ray => line_style(|style| {
                style.extend_right = true;
                style.stats.position = StatsPosition::Right;
            }),
            Self::ExtendedLine => line_style(|style| {
                style.extend_left = true;
                style.extend_right = true;
                style.stats.position = StatsPosition::Right;
            }),
            Self::InfoLine => line_style(|style| {
                style.stats = ArrowStats {
                    price_range: true,
                    percent: true,
                    bars: true,
                    time: true,
                    distance: true,
                    angle: true,
                    position: StatsPosition::Center,
                    always_show: true,
                };
            }),
            Self::TrendAngle => line_style(|style| {
                style.text.size = 12.0;
                style.stats.position = StatsPosition::Right;
            }),
            Self::HorizontalLine | Self::HorizontalRay => line_style(|style| {
                style.text.size = 12.0;
                style.text.valign = VerticalAlign::Top;
                style.price_labels = true;
            }),
            Self::VerticalLine => line_style(|style| {
                style.text.align = HorizontalAlign::Right;
                style.text.valign = VerticalAlign::Top;
                style.text.orientation = TextOrientation::Vertical;
                style.time_label = true;
            }),
            Self::Crossline => line_style(|style| {
                style.price_labels = true;
                style.time_label = true;
            }),
            Self::ParallelChannel => {
                let mut style = tinted([41, 98, 255, 255], true);
                style.middle_line.enabled = true;
                style.middle_line.width = 1.0;
                style.middle_line.dash = LineDash::Dashed;
                style.text.align = HorizontalAlign::Left;
                style.text.valign = VerticalAlign::Bottom;
                style
            }
            Self::FlatChannel | Self::DisjointChannel => {
                let color = if self == Self::FlatChannel {
                    [255, 152, 0, 255]
                } else {
                    [8, 153, 129, 255]
                };
                let mut style = tinted(color, true);
                style.text.align = HorizontalAlign::Left;
                style.text.valign = VerticalAlign::Bottom;
                style
            }
            Self::RegressionTrend => {
                let mut style = tinted([41, 98, 255, 255], true);
                // The bands are filled at the Charting Library's 70 % transparency.
                style.fill.color = [41, 98, 255, 77];
                style
            }
            Self::Pitchfork
            | Self::SchiffPitchfork
            | Self::ModifiedSchiffPitchfork
            | Self::InsidePitchfork => {
                // The median is the stroke; the bands take their colours from the levels.
                let mut style = tinted([242, 54, 69, 255], true);
                style.fill.color = [242, 54, 69, FILL_ALPHA];
                style
            }
            Self::Text => {
                // The Charting Library's: blue text, nothing behind it, no border.
                let mut style = tinted([41, 98, 255, 255], false);
                style.fill.color = [41, 98, 255, 64];
                style.border.color = [112, 112, 112, 255];
                style.box_width = 200.0;
                style.text.align = HorizontalAlign::Left;
                style
            }
            Self::Comment => {
                let mut style = text_box([41, 98, 255, 255], [41, 98, 255, 255], true);
                style.text.size = 16.0;
                style
            }
            Self::PriceLabel => {
                let mut style = text_box([41, 98, 255, 255], [41, 98, 255, 255], true);
                style.text.bold = true;
                style
            }
            Self::FlagMark => tinted([41, 98, 255, 255], false),
            Self::Note => {
                // The Charting Library's: a blue card, its marker and leader the same blue.
                let mut style = text_box([41, 98, 255, 179], [41, 98, 255, 255], true);
                style.stroke.color = [41, 98, 255, 255];
                style.stroke.width = 1.5;
                style
            }
            Self::PriceNote => {
                // Not in the Charting Library: the price tag's look on a leader.
                let mut style = text_box([41, 98, 255, 255], [41, 98, 255, 255], true);
                style.stroke.color = [41, 98, 255, 255];
                style.stroke.width = 1.5;
                style.text.bold = true;
                style
            }
            Self::Table => {
                // Not in the Charting Library: a slate card with a grey grid and light text, so
                // it reads over the candles on a dark or a light chart.
                let mut style = tinted([230, 234, 242, 255], true);
                style.fill.color = [38, 43, 56, 235];
                style.border.enabled = true;
                style.border.color = [112, 125, 150, 255];
                style.text.size = 13.0;
                style.text.align = HorizontalAlign::Left;
                style
            }
            Self::Callout => {
                let mut style = text_box([0, 151, 167, 179], [0, 151, 167, 255], true);
                style.border.width = 2.0;
                style.box_width = 200.0;
                style
            }
            Self::Signpost => {
                // The plate colour is the fill; the stem and the dot at the anchor follow it.
                let mut style = text_box([41, 98, 255, 255], [41, 98, 255, 255], false);
                style.stroke.color = [41, 98, 255, 255];
                style.stroke.width = 1.5;
                style.text.size = 12.0;
                style
            }
            Self::Pin => {
                // Not in the Charting Library: the note's look is a blue card like its Note.
                let mut style = text_box([41, 98, 255, 179], [41, 98, 255, 255], false);
                style.stroke.color = [41, 98, 255, 255];
                style.wrap = true;
                style.box_width = 220.0;
                style.text.align = HorizontalAlign::Left;
                // A note longer than the plot is cut at its end, so it starts at the top.
                style.text.valign = VerticalAlign::Top;
                style
            }
            Self::XabcdPattern => pattern_style([8, 153, 129, 255], 38),
            Self::CypherPattern => pattern_style([41, 98, 255, 255], 38),
            Self::HeadAndShoulders => pattern_style([8, 153, 129, 255], 38),
            Self::AbcdPattern => pattern_style([8, 153, 129, 255], 0),
            Self::TrianglePattern => pattern_style([103, 58, 183, 255], 38),
            Self::ThreeDrivesPattern => {
                let mut style = pattern_style([103, 58, 183, 255], 128);
                style.fill.color = [149, 40, 204, 128];
                style
            }
            Self::ElliottImpulse | Self::ElliottCorrection => wave_style([61, 133, 198, 255]),
            Self::ElliottTriangle => wave_style([255, 152, 0, 255]),
            Self::ElliottDoubleCombo | Self::ElliottTripleCombo => wave_style([106, 168, 79, 255]),
            Self::CyclicLines => {
                let mut style = tinted([128, 204, 219, 255], false);
                style.stroke.width = 1.0;
                style
            }
            Self::TimeCycles => {
                let mut style = tinted([21, 153, 128, 255], true);
                style.fill.color = [106, 168, 79, 128];
                style
            }
            Self::SineLine => tinted([21, 153, 128, 255], false),
            Self::LongPosition | Self::ShortPosition => {
                // The library's: a 1 px grey line, white 12 px stats, the prices on the axis.
                let mut style = tinted([128, 128, 128, 255], false);
                style.stroke.width = 1.0;
                style.text.color = [255, 255, 255, 255];
                style.text.size = 12.0;
                style.price_labels = true;
                style
            }
            Self::PositionForecast => {
                let mut style = tinted([41, 98, 255, 255], false);
                style.stroke.width = 2.0;
                style.text.color = [255, 255, 255, 255];
                style.text.size = 12.0;
                style
            }
            Self::BarsPattern => tinted([41, 98, 255, 255], false),
            Self::GhostFeed => DrawingStyle::default(),
            // Not in the Charting Library: a blue border and background, as the other shapes.
            Self::Sector => tinted([41, 98, 255, 255], true),
            Self::AnchoredVwap => {
                let mut style = tinted([30, 136, 229, 255], false);
                style.stroke.width = 1.0;
                style
            }
            Self::FixedRangeVolumeProfile | Self::AnchoredVolumeProfile => DrawingStyle::default(),
            Self::PriceRange | Self::DateRange | Self::DateAndPriceRange => {
                // The library's: a 2 px blue line, the background 60 % transparent, and 12 px
                // custom text in the same blue (the label is white on its grey plate).
                let mut style = tinted([41, 98, 255, 255], true);
                style.stroke.width = 2.0;
                style.fill.color = [41, 98, 255, 102];
                style.text.size = 12.0;
                style
            }
            Self::FibRetracement
            | Self::FibExtension
            | Self::FibTrendTime
            | Self::FibCircles
            | Self::FibSpeedArcs => fib_style(self, [120, 123, 134, 255], LineDash::Dashed),
            // The fan draws no line of its own: each ray has the look of its level.
            Self::FibSpeedFan => fib_style(self, [120, 123, 134, 255], LineDash::Solid),
            Self::FibSpiral => {
                // The library's spiral: a 1 px turquoise line and nothing else.
                let mut style = fib_style(self, [0, 188, 212, 255], LineDash::Solid);
                style.stroke.width = 1.0;
                style.fill.enabled = false;
                style
            }
            Self::FibWedge => fib_style(self, [128, 128, 128, 255], LineDash::Solid),
            // The median of a pitchfan is the line itself.
            Self::Pitchfan => fib_style(self, [242, 54, 69, 255], LineDash::Solid),
            // A channel has no line of its own: its base line is its first level.
            Self::FibChannel => fib_style(self, [120, 123, 134, 255], LineDash::Solid),
            Self::FibTimeZone => {
                // Grey, 1 px and with no background, as the library has it.
                let mut style = fib_style(self, [128, 128, 128, 255], LineDash::Dashed);
                style.stroke.width = 1.0;
                style.fill.enabled = false;
                style
            }
            // A card takes its look from the theme, and a picture has none to style.
            Self::Cursor | Self::Ruler | Self::Image | Self::Post => DrawingStyle::default(),
        };
        if matches!(self, Self::Rectangle) {
            style.middle_line.dash = LineDash::Dashed;
        }
        style
    }

    /// The key that arms the tool with Alt (Option on a Mac), where TradingView has one.
    pub const fn hotkey(self) -> Option<char> {
        match self {
            Self::TrendLine => Some('T'),
            Self::HorizontalLine => Some('H'),
            Self::HorizontalRay => Some('J'),
            Self::VerticalLine => Some('V'),
            Self::Crossline => Some('C'),
            Self::FibRetracement => Some('F'),
            _ => None,
        }
    }

    /// The tools of the Forecast tile's menu, in order.
    #[cfg(test)]
    pub fn forecast_tools() -> impl Iterator<Item = Self> {
        Self::GROUPED
            .into_iter()
            .filter(|tool| ToolGroup::FORECAST_MENU.contains(&tool.spec().group))
    }

    /// The tools of the Patterns tile's menu, in order.
    #[cfg(test)]
    pub fn pattern_tools() -> impl Iterator<Item = Self> {
        Self::GROUPED
            .into_iter()
            .filter(|tool| ToolGroup::PATTERN_MENU.contains(&tool.spec().group))
    }

    /// The tools of the Fibonacci and Gann tile's menu, in order.
    #[cfg(test)]
    pub fn fib_tools() -> impl Iterator<Item = Self> {
        Self::GROUPED
            .into_iter()
            .filter(|tool| ToolGroup::FIB_MENU.contains(&tool.spec().group))
    }

    /// The tools of the Lines tile's menu, in order.
    #[cfg(test)]
    pub fn line_tools() -> impl Iterator<Item = Self> {
        Self::GROUPED
            .into_iter()
            .filter(|tool| ToolGroup::LINE_MENU.contains(&tool.spec().group))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_menus_list_every_tool_once_in_their_groups() {
        assert_eq!(Tool::GROUPED.len(), 82);
        for (index, tool) in Tool::GROUPED.iter().enumerate() {
            assert!(!Tool::GROUPED[..index].contains(tool), "{tool:?} repeats");
            let group = tool.spec().group;
            let menus = [
                ToolGroup::MENU.as_slice(),
                &ToolGroup::LINE_MENU,
                &ToolGroup::NOTE_MENU,
                &ToolGroup::PATTERN_MENU,
                &ToolGroup::FORECAST_MENU,
                &ToolGroup::FIB_MENU,
            ];
            assert_eq!(
                menus.iter().filter(|menu| menu.contains(&group)).count(),
                1,
                "{tool:?} is in exactly one menu"
            );
        }
        let counts = |groups: &[ToolGroup]| -> Vec<usize> {
            groups
                .iter()
                .map(|group| Tool::in_group(*group).count())
                .collect()
        };
        assert_eq!(counts(&ToolGroup::MENU), [2, 4, 10]);
        assert_eq!(counts(&ToolGroup::LINE_MENU), [9, 4, 4]);
        assert_eq!(counts(&ToolGroup::NOTE_MENU), [10, 2]);
        assert_eq!(counts(&ToolGroup::PATTERN_MENU), [6, 5, 3]);
        assert_eq!(counts(&ToolGroup::FORECAST_MENU), [6, 3, 3]);
        assert_eq!(counts(&ToolGroup::FIB_MENU), [11, 0]);
        assert_eq!(Tool::line_tools().count(), 17);
        assert_eq!(Tool::pattern_tools().count(), 14);
        assert_eq!(Tool::forecast_tools().count(), 12);
        assert_eq!(Tool::fib_tools().count(), 11);
        // The crosshair, the brush's own tile and the ruler are in no menu of theirs.
        assert!(!Tool::GROUPED.contains(&Tool::Ruler));
    }

    #[test]
    fn the_new_tools_place_as_tradingview_does() {
        let anchors = |tool: Tool| match tool.spec().placement {
            Placement::Points(count) => Some(count),
            Placement::Single => Some(1),
            _ => None,
        };
        for tool in [
            Tool::TrendLine,
            Tool::Ray,
            Tool::InfoLine,
            Tool::ExtendedLine,
            Tool::TrendAngle,
            Tool::RegressionTrend,
        ] {
            assert_eq!(anchors(tool), Some(2), "{tool:?}");
        }
        for tool in [
            Tool::HorizontalLine,
            Tool::HorizontalRay,
            Tool::VerticalLine,
            Tool::Crossline,
        ] {
            assert_eq!(anchors(tool), Some(1), "{tool:?}");
        }
        for tool in [
            Tool::ParallelChannel,
            Tool::FlatChannel,
            Tool::DisjointChannel,
            Tool::Pitchfork,
            Tool::SchiffPitchfork,
            Tool::ModifiedSchiffPitchfork,
            Tool::InsidePitchfork,
        ] {
            assert_eq!(anchors(tool), Some(3), "{tool:?}");
        }
    }

    #[test]
    fn hotkeys_are_the_documented_alt_letters() {
        let keys: Vec<(Tool, char)> = Tool::GROUPED
            .into_iter()
            .filter_map(|tool| tool.hotkey().map(|key| (tool, key)))
            .collect();
        assert_eq!(
            keys,
            [
                (Tool::FibRetracement, 'F'),
                (Tool::TrendLine, 'T'),
                (Tool::HorizontalLine, 'H'),
                (Tool::HorizontalRay, 'J'),
                (Tool::VerticalLine, 'V'),
                (Tool::Crossline, 'C'),
            ]
        );
    }

    #[test]
    fn the_new_tools_start_in_the_charting_librarys_look() {
        let ray = Tool::Ray.default_style();
        assert!(ray.extend_right && !ray.extend_left);
        let extended = Tool::ExtendedLine.default_style();
        assert!(extended.extend_left && extended.extend_right);
        let info = Tool::InfoLine.default_style();
        assert!(info.stats.always_show && info.stats.any());
        assert!(info.stats.angle && info.stats.distance && info.stats.time);
        assert!(Tool::HorizontalLine.default_style().price_labels);
        let vertical = Tool::VerticalLine.default_style();
        assert!(vertical.time_label);
        assert_eq!(
            vertical.text.orientation,
            super::super::drawing_style::TextOrientation::Vertical
        );
        let cross = Tool::Crossline.default_style();
        assert!(cross.price_labels && cross.time_label);
        let parallel = Tool::ParallelChannel.default_style();
        assert!(parallel.middle_line.enabled && parallel.fill.enabled);
        assert_eq!(parallel.stroke.color, [41, 98, 255, 255]);
        assert_eq!(
            Tool::FlatChannel.default_style().stroke.color,
            [255, 152, 0, 255]
        );
        assert_eq!(
            Tool::DisjointChannel.default_style().stroke.color,
            [8, 153, 129, 255]
        );
        let fork = Tool::Pitchfork.default_style();
        assert_eq!(fork.stroke.color, [242, 54, 69, 255]);
        let on: Vec<f32> = fork
            .pitchfork_levels
            .iter()
            .filter(|level| level.line.enabled)
            .map(|level| level.ratio)
            .collect();
        assert_eq!(on, [0.5, 1.0]);
        // All four variants start alike: the variant is geometry, not look.
        assert_eq!(fork, Tool::InsidePitchfork.default_style());
    }

    #[test]
    fn caps_offer_each_new_tool_only_what_it_can_honour() {
        assert!(Tool::TrendLine.caps().has(Caps::TEXT) && Tool::TrendLine.caps().has(Caps::STATS));
        assert!(!Tool::TrendAngle.caps().has(Caps::ENDS));
        assert!(Tool::VerticalLine.caps().has(Caps::ORIENTATION));
        assert!(Tool::VerticalLine.caps().has(Caps::TIME_LABEL));
        assert!(!Tool::VerticalLine.caps().has(Caps::PRICE_LABELS));
        assert!(Tool::HorizontalLine.caps().has(Caps::PRICE_LABELS));
        assert!(!Tool::HorizontalLine.caps().has(Caps::EXTEND));
        assert!(Tool::RegressionTrend.caps().has(Caps::REGRESSION));
        assert!(!Tool::RegressionTrend.caps().has(Caps::TEXT));
        assert!(
            Tool::Pitchfork.caps().has(Caps::LEVELS) && !Tool::Pitchfork.caps().has(Caps::TEXT)
        );
        assert!(Tool::ParallelChannel.caps().has(Caps::MIDDLE_LINE));
        assert!(Tool::ParallelChannel.caps().has(Caps::LEVELS));
        assert!(Tool::Crossline.caps().has(Caps::TIME_LABEL));
        assert!(Caps::STROKE.without(Caps::WIDTH).has(Caps::COLOR));
        assert!(!Caps::STROKE.without(Caps::WIDTH).has(Caps::WIDTH));
    }

    #[test]
    fn every_tool_has_a_label_and_a_default_style_in_range() {
        for tool in Tool::GROUPED {
            assert!(!tool.label().is_empty());
            let style = tool.default_style();
            assert_eq!(style, style.sanitized(), "{tool:?}");
        }
    }

    #[test]
    fn caps_say_what_each_tool_offers() {
        assert!(Tool::Rectangle.caps().has(Caps::FILL));
        assert!(Tool::Rectangle.caps().has(Caps::MIDDLE_LINE));
        assert!(Tool::Rectangle.caps().has(Caps::EXTEND));
        assert!(!Tool::Triangle.caps().has(Caps::TEXT));
        assert!(Tool::Arrow.caps().has(Caps::STATS));
        assert!(!Tool::Arrow.caps().has(Caps::FILL));
        // The marks are colour and text only: their size is fixed.
        assert!(Tool::ArrowMarkUp.caps().has(Caps::COLOR));
        assert!(!Tool::ArrowMarkUp.caps().has(Caps::WIDTH));
        assert!(Tool::Cursor.caps().is_empty());
        assert!(!Caps::NONE.has(Caps::NONE));
    }

    #[test]
    fn placement_counts_match_the_documented_anchor_numbers() {
        let points = |tool: Tool| match tool.spec().placement {
            Placement::Points(count) => Some(count),
            _ => None,
        };
        assert_eq!(points(Tool::Rectangle), Some(2));
        assert_eq!(points(Tool::RotatedRectangle), Some(3));
        assert_eq!(points(Tool::Triangle), Some(3));
        assert_eq!(points(Tool::Curve), Some(3));
        assert_eq!(points(Tool::DoubleCurve), Some(4));
        assert_eq!(
            Tool::Polyline.spec().placement,
            Placement::Open { closable: true }
        );
        assert_eq!(
            Tool::Path.spec().placement,
            Placement::Open { closable: false }
        );
        assert_eq!(Tool::Highlighter.spec().placement, Placement::Freehand);
    }

    #[test]
    fn the_magnet_cycles_and_the_invert_key_flips_it() {
        assert_eq!(MagnetMode::Off.next(), MagnetMode::Weak);
        assert_eq!(MagnetMode::Weak.next(), MagnetMode::Strong);
        assert_eq!(MagnetMode::Strong.next(), MagnetMode::Off);
        assert_eq!(MagnetMode::Off.inverted(false), MagnetMode::Off);
        assert_eq!(MagnetMode::Off.inverted(true), MagnetMode::Weak);
        assert_eq!(MagnetMode::Strong.inverted(true), MagnetMode::Off);
        assert_eq!(MagnetMode::Weak.inverted(false), MagnetMode::Weak);
    }

    #[test]
    fn defaults_follow_the_documented_colours() {
        assert_eq!(
            Tool::Brush.default_style().stroke.color,
            [255, 159, 10, 255]
        );
        assert_eq!(
            Tool::ArrowMarkUp.default_style().stroke.color,
            [8, 153, 129, 255]
        );
        assert_eq!(
            Tool::ArrowMarkDown.default_style().stroke.color,
            [204, 47, 60, 255]
        );
        assert_eq!(Tool::Highlighter.default_style().stroke.width, 20.0);
        assert!(Tool::Rectangle.default_style().fill.enabled);
        assert!(!Tool::Curve.default_style().fill.enabled);
        assert_eq!(Tool::Arrow.default_style().head_end, LineEnd::Arrow);
        assert_eq!(
            Tool::Rectangle.default_style().middle_line.dash,
            LineDash::Dashed
        );
    }
}
