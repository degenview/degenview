//! Drawings on a chart: pinned to market coordinates (time and price), never to pixels, so a
//! drawing survives zoom, scroll, a resize and a timeframe switch.
//!
//! Fibonacci, the brush and the ruler have variants of their own;
//! every other tool is one [`Drawing::Shape`] whose typed [`Geometry`] owns its anchors. Styling,
//! text and per-timeframe visibility live beside the geometry, so moving, restyling and hiding a
//! drawing never touch each other.

use super::Timeframe;
use super::bars_pattern::BarsPattern;
use super::cycle::CycleKind;
use super::drawing_style::{DrawingStyle, Visibility};
use super::drawing_tool::Tool;
use super::elliott::WaveKind;
use super::fib::FibKind;
use super::image_content::{Content, ImageData};
use super::link_card::LinkCard;
use super::measure::MeasureKind;
use super::pattern::PatternKind;
use super::position::Side;
use super::table::TableData;
use serde::{Deserialize, Serialize};

/// The most vertices one drawing may hold: a safety limit on memory and rendering, far above
/// anything drawn by hand.
pub const MAX_POINTS: usize = 5_000;
/// The most characters of text one drawing may carry.
pub const MAX_TEXT_CHARS: usize = 500;
/// The most a note that is meant to hold a paragraph may carry. Far below what a synced
/// drawing may hold in one string.
pub const LONG_TEXT_CHARS: usize = 2_000;

/// One end of a hand-drawn line, pinned to data rather than pixels: continuous
/// time plus price, so a drawing survives zoom, scroll, and timeframe switches.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Anchor {
    pub time_ms: i64,
    pub price: f64,
}

impl Anchor {
    pub fn is_finite(&self) -> bool {
        self.price.is_finite()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrowDirection {
    Up,
    Down,
}

/// The text and note tools that sit on one anchor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationKind {
    /// A box of text with its top-left corner on the anchor.
    Text,
    /// A speech bubble whose tail points at the anchor.
    Comment,
    /// The price at the anchor, in a tag that points at it.
    PriceLabel,
    /// A small flag on a pole planted at the anchor.
    FlagMark,
    /// A pin planted at the anchor; its note opens when the pin is clicked.
    Pin,
}

impl AnnotationKind {
    pub const fn tool(self) -> Tool {
        match self {
            Self::Text => Tool::Text,
            Self::Comment => Tool::Comment,
            Self::PriceLabel => Tool::PriceLabel,
            Self::FlagMark => Tool::FlagMark,
            Self::Pin => Tool::Pin,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::Text => Some(Self::Text),
            Tool::Comment => Some(Self::Comment),
            Tool::PriceLabel => Some(Self::PriceLabel),
            Tool::FlagMark => Some(Self::FlagMark),
            Tool::Pin => Some(Self::Pin),
            _ => None,
        }
    }
}

/// The note tools that point from a label at an anchor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaderKind {
    /// A card joined to its anchor by a leader line ending in a dot.
    Note,
    /// The same with the price of the anchor written first.
    PriceNote,
    /// A speech bubble whose tail points at the anchor.
    Callout,
}

impl LeaderKind {
    pub const fn tool(self) -> Tool {
        match self {
            Self::Note => Tool::Note,
            Self::PriceNote => Tool::PriceNote,
            Self::Callout => Tool::Callout,
        }
    }

    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::Note => Some(Self::Note),
            Tool::PriceNote => Some(Self::PriceNote),
            Tool::Callout => Some(Self::Callout),
            _ => None,
        }
    }
}

/// The shape of a drawing: only the anchors that define it, never derived corners, so a shape
/// cannot drift into something its tool could not have drawn.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Geometry {
    Highlighter {
        points: Vec<Anchor>,
    },
    /// A thick filled arrow from `a` to `b`.
    ArrowMarker {
        a: Anchor,
        b: Anchor,
    },
    /// A line from `a` to `b` with optional heads, labels and statistics.
    Arrow {
        a: Anchor,
        b: Anchor,
    },
    /// A fixed-size up or down arrow pinned at one anchor.
    ArrowMark {
        at: Anchor,
        direction: ArrowDirection,
    },
    /// Opposite corners of an axis-aligned rectangle.
    Rectangle {
        a: Anchor,
        b: Anchor,
    },
    /// `a`→`b` is one edge; `width` is a point that sets the perpendicular extent. The other
    /// two corners are derived from them.
    RotatedRectangle {
        a: Anchor,
        b: Anchor,
        width: Anchor,
    },
    Path {
        points: Vec<Anchor>,
    },
    Polyline {
        points: Vec<Anchor>,
        closed: bool,
    },
    /// The centre and a point on the circumference: the radius is the on-screen distance
    /// between them, so the circle stays round whatever the axis scales.
    Circle {
        center: Anchor,
        edge: Anchor,
    },
    /// Opposite corners of the ellipse's bounding box.
    Ellipse {
        a: Anchor,
        b: Anchor,
    },
    Triangle {
        points: [Anchor; 3],
    },
    /// The circular arc from `start` to `end` that passes through `through`.
    Arc {
        start: Anchor,
        end: Anchor,
        through: Anchor,
    },
    /// A quadratic Bézier from `a` to `b` pulled towards `control`.
    Curve {
        a: Anchor,
        b: Anchor,
        control: Anchor,
    },
    /// A cubic Bézier from `a` to `b` with two independent controls.
    DoubleCurve {
        a: Anchor,
        b: Anchor,
        c1: Anchor,
        c2: Anchor,
    },
    /// The segment from `a` to `b`, continued past either end by the style's extension.
    TrendLine {
        a: Anchor,
        b: Anchor,
    },
    /// From `a`, through `b` and on beyond it.
    Ray {
        a: Anchor,
        b: Anchor,
    },
    /// A trend line with a statistics box.
    InfoLine {
        a: Anchor,
        b: Anchor,
    },
    /// Through `a` and `b`, without end in either direction.
    ExtendedLine {
        a: Anchor,
        b: Anchor,
    },
    /// A trend line with its angle to the horizontal.
    TrendAngle {
        a: Anchor,
        b: Anchor,
    },
    /// A line across the whole plot at one price; its time is where the handle was left.
    HorizontalLine {
        at: Anchor,
    },
    /// From `at` to the right edge of the plot, at its price.
    HorizontalRay {
        at: Anchor,
    },
    /// A line from the top to the bottom of the plot at one time; its price is where the
    /// handle was left.
    VerticalLine {
        at: Anchor,
    },
    /// A horizontal and a vertical line through one point.
    Crossline {
        at: Anchor,
    },
    /// `a`→`b` is the first edge; the second runs through `c` parallel to it.
    ParallelChannel {
        a: Anchor,
        b: Anchor,
        c: Anchor,
    },
    /// Least squares through the bars from `a` to `b`. Only the times are read: the prices
    /// are where the handles were left, and the line is fitted to the bars.
    Regression {
        a: Anchor,
        b: Anchor,
    },
    /// `a`→`b` is a sloped edge; the other is level at `c`'s price and spans the same times.
    /// `c`'s time is kept at the middle of `a` and `b`.
    FlatChannel {
        a: Anchor,
        b: Anchor,
        c: Anchor,
    },
    /// Two parallel edges that need not share their times: `a`→`b`, and one through `c`
    /// ending at `d`'s time. `d`'s price is not read; the edge ends where it meets that time.
    DisjointChannel {
        a: Anchor,
        b: Anchor,
        c: Anchor,
        d: Anchor,
    },
    /// Three pivots; the variant says where the median starts and ends.
    Pitchfork {
        variant: PitchforkVariant,
        p1: Anchor,
        p2: Anchor,
        p3: Anchor,
    },
    /// A text or note tool on one anchor.
    Annotation {
        kind: AnnotationKind,
        at: Anchor,
    },
    /// A label pointing at `at`; `label` is where its box is centred.
    Leader {
        kind: LeaderKind,
        at: Anchor,
        label: Anchor,
    },
    /// An event marker at `anchor` with its plate straight above or below it: `label` is the
    /// plate's centre, always on the anchor's bar, so its time is not read.
    Signpost {
        anchor: Anchor,
        label: Anchor,
    },
    /// A grid of text cells, at a chart point or pinned to the screen.
    Table(TableData),
    /// A stored picture between two corners.
    Image(ImageData),
    /// A link to an X post, drawn as a card whose top-left corner is at `anchor`: the post's own
    /// time when that bar was on screen where it was placed, else where it was placed.
    Card {
        anchor: Anchor,
        card: LinkCard,
    },
    /// A Fibonacci drawing through its anchors: exactly `kind.points()` of them, in placement order.
    Fib {
        kind: FibKind,
        points: Vec<Anchor>,
    },
    /// A chart pattern through its pivots: exactly `kind.points()` of them, in placement order.
    Pattern {
        kind: PatternKind,
        points: Vec<Anchor>,
    },
    /// An Elliott wave through its pivots: the start of the first wave, then the end of each.
    Wave {
        kind: WaveKind,
        points: Vec<Anchor>,
    },
    /// A cycle set by two anchors, whose distance in bars is its period.
    Cycle {
        kind: CycleKind,
        a: Anchor,
        b: Anchor,
    },
    /// A hypothetical trade: its entry, its target and its stop. The target's and the stop's times
    /// are the drawing's right edge, so they are always the same.
    Position {
        side: Side,
        entry: Anchor,
        target: Anchor,
        stop: Anchor,
    },
    /// An entry (`source`) and an exit (`target`) for a trade, and the time between them.
    Forecast {
        source: Anchor,
        target: Anchor,
    },
    /// A run of the chart's own bars, copied and set down.
    Bars(BarsPattern),
    /// Hypothetical candles from `start` to `end`, made from `seed` alone.
    Ghost {
        start: Anchor,
        end: Anchor,
        seed: u64,
    },
    /// The wedge of a circle: `radius` sets its centre-to-edge distance and where it starts, `tip`
    /// where it ends.
    Sector {
        center: Anchor,
        radius: Anchor,
        tip: Anchor,
    },
    /// The VWAP from `at`'s bar to the latest. Only the time is read.
    Vwap {
        at: Anchor,
    },
    /// The volume profile of the bars between `a` and `b`. Only the times are read.
    FixedProfile {
        a: Anchor,
        b: Anchor,
    },
    /// The volume profile from `at`'s bar to the latest. Only the time is read.
    AnchoredProfile {
        at: Anchor,
    },
    /// A measurer between two anchors.
    Measure {
        kind: MeasureKind,
        a: Anchor,
        b: Anchor,
    },
}

/// Which Andrews pitchfork a [`Geometry::Pitchfork`] is.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitchforkVariant {
    /// The median runs from P1 through the middle of P2 and P3.
    #[default]
    Standard,
    /// As Standard, but from P1's time at the price halfway between P1 and P2.
    Schiff,
    /// As Standard, but from the middle of P1 and P2 in both time and price.
    ModifiedSchiff,
    /// From the middle of P1 and P2 to P3, with the levels measured across P1 and P2.
    Inside,
}

impl PitchforkVariant {
    pub const ALL: [Self; 4] = [
        Self::Standard,
        Self::Schiff,
        Self::ModifiedSchiff,
        Self::Inside,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Original",
            Self::Schiff => "Schiff",
            Self::ModifiedSchiff => "Modified Schiff",
            Self::Inside => "Inside",
        }
    }

    pub const fn tool(self) -> Tool {
        match self {
            Self::Standard => Tool::Pitchfork,
            Self::Schiff => Tool::SchiffPitchfork,
            Self::ModifiedSchiff => Tool::ModifiedSchiffPitchfork,
            Self::Inside => Tool::InsidePitchfork,
        }
    }

    /// The variant `tool` draws, for the four pitchfork tools.
    pub const fn of_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::Pitchfork => Some(Self::Standard),
            Tool::SchiffPitchfork => Some(Self::Schiff),
            Tool::ModifiedSchiffPitchfork => Some(Self::ModifiedSchiff),
            Tool::InsidePitchfork => Some(Self::Inside),
            _ => None,
        }
    }
}

/// The anchor `d` of a disjoint channel built from three clicks: its second edge as long as its
/// first, ending at the matching time. The price is not read.
fn disjoint_end(a: Anchor, b: Anchor, c: Anchor) -> Anchor {
    Anchor {
        time_ms: c
            .time_ms
            .saturating_add(b.time_ms.saturating_sub(a.time_ms)),
        price: c.price + (b.price - a.price),
    }
}

/// How far a position's target and stop are from its entry, as a fraction of the price, when only the
/// entry is known (the layer sizes a new one from what is on screen instead).
const DEFAULT_TARGET: f64 = 0.04;
const DEFAULT_STOP: f64 = 0.02;
/// How wide a position is, in milliseconds, when only the entry is known.
const DEFAULT_WIDTH_MS: i64 = 24 * 60 * 60 * 1000;

impl Geometry {
    /// A position of `side` at `entry` with a target and a stop at the default distances and the
    /// default width: for a preview and for a position made from its entry alone.
    pub fn default_position(side: Side, entry: Anchor) -> Self {
        let unit = (entry.price.abs() * DEFAULT_TARGET).max(f64::MIN_POSITIVE);
        let (target, stop) = (
            entry.price + side.sign() * unit,
            entry.price - side.sign() * unit * (DEFAULT_STOP / DEFAULT_TARGET),
        );
        let time_ms = entry.time_ms.saturating_add(DEFAULT_WIDTH_MS);
        Self::Position {
            side,
            entry,
            target: Anchor {
                time_ms,
                price: target,
            },
            stop: Anchor {
                time_ms,
                price: stop,
            },
        }
    }

    /// The seed of a ghost feed that starts at `start`: the same start always draws the same
    /// candles. (The dialog can change the seed to draw another run.)
    pub fn ghost_seed(start: Anchor) -> u64 {
        (start.time_ms as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(17)
            ^ start.price.to_bits()
    }

    /// The tool that draws this geometry.
    pub fn tool(&self) -> Tool {
        match self {
            Self::TrendLine { .. } => Tool::TrendLine,
            Self::Ray { .. } => Tool::Ray,
            Self::InfoLine { .. } => Tool::InfoLine,
            Self::ExtendedLine { .. } => Tool::ExtendedLine,
            Self::TrendAngle { .. } => Tool::TrendAngle,
            Self::HorizontalLine { .. } => Tool::HorizontalLine,
            Self::HorizontalRay { .. } => Tool::HorizontalRay,
            Self::VerticalLine { .. } => Tool::VerticalLine,
            Self::Crossline { .. } => Tool::Crossline,
            Self::ParallelChannel { .. } => Tool::ParallelChannel,
            Self::Regression { .. } => Tool::RegressionTrend,
            Self::FlatChannel { .. } => Tool::FlatChannel,
            Self::DisjointChannel { .. } => Tool::DisjointChannel,
            Self::Pitchfork { variant, .. } => variant.tool(),
            Self::Annotation { kind, .. } => kind.tool(),
            Self::Leader { kind, .. } => kind.tool(),
            Self::Signpost { .. } => Tool::Signpost,
            Self::Table(_) => Tool::Table,
            Self::Image(_) => Tool::Image,
            Self::Card { .. } => Tool::Post,
            Self::Fib { kind, .. } => kind.tool(),
            Self::Pattern { kind, .. } => kind.tool(),
            Self::Wave { kind, .. } => kind.tool(),
            Self::Cycle { kind, .. } => kind.tool(),
            Self::Position {
                side: Side::Long, ..
            } => Tool::LongPosition,
            Self::Position {
                side: Side::Short, ..
            } => Tool::ShortPosition,
            Self::Forecast { .. } => Tool::PositionForecast,
            Self::Bars(_) => Tool::BarsPattern,
            Self::Ghost { .. } => Tool::GhostFeed,
            Self::Sector { .. } => Tool::Sector,
            Self::Vwap { .. } => Tool::AnchoredVwap,
            Self::FixedProfile { .. } => Tool::FixedRangeVolumeProfile,
            Self::AnchoredProfile { .. } => Tool::AnchoredVolumeProfile,
            Self::Measure { kind, .. } => kind.tool(),
            Self::Highlighter { .. } => Tool::Highlighter,
            Self::ArrowMarker { .. } => Tool::ArrowMarker,
            Self::Arrow { .. } => Tool::Arrow,
            Self::ArrowMark {
                direction: ArrowDirection::Up,
                ..
            } => Tool::ArrowMarkUp,
            Self::ArrowMark {
                direction: ArrowDirection::Down,
                ..
            } => Tool::ArrowMarkDown,
            Self::Rectangle { .. } => Tool::Rectangle,
            Self::RotatedRectangle { .. } => Tool::RotatedRectangle,
            Self::Path { .. } => Tool::Path,
            Self::Polyline { .. } => Tool::Polyline,
            Self::Circle { .. } => Tool::Circle,
            Self::Ellipse { .. } => Tool::Ellipse,
            Self::Triangle { .. } => Tool::Triangle,
            Self::Arc { .. } => Tool::Arc,
            Self::Curve { .. } => Tool::Curve,
            Self::DoubleCurve { .. } => Tool::DoubleCurve,
        }
    }

    /// Every anchor, in the order they were placed.
    pub fn anchors(&self) -> Vec<Anchor> {
        match self {
            Self::Highlighter { points }
            | Self::Path { points }
            | Self::Polyline { points, .. }
            | Self::Fib { points, .. }
            | Self::Pattern { points, .. }
            | Self::Wave { points, .. } => points.clone(),
            Self::ArrowMarker { a, b }
            | Self::Arrow { a, b }
            | Self::Rectangle { a, b }
            | Self::Cycle { a, b, .. }
            | Self::FixedProfile { a, b }
            | Self::Measure { a, b, .. }
            | Self::Forecast {
                source: a,
                target: b,
            }
            | Self::Ghost {
                start: a, end: b, ..
            }
            | Self::Ellipse { a, b } => vec![*a, *b],
            Self::Position {
                entry,
                target,
                stop,
                ..
            } => vec![*entry, *target, *stop],
            Self::Sector {
                center,
                radius,
                tip,
            } => vec![*center, *radius, *tip],
            Self::Bars(pattern) => vec![pattern.at],
            Self::Vwap { at } | Self::AnchoredProfile { at } => vec![*at],
            Self::Leader { at, label, .. }
            | Self::Signpost {
                anchor: at, label, ..
            } => vec![*at, *label],
            Self::ArrowMark { at, .. } | Self::Annotation { at, .. } => vec![*at],
            // A pinned table is not at a chart point: it has no anchor to move or to show.
            Self::Table(table) if table.screen.is_none() => vec![table.at],
            Self::Table(_) => Vec::new(),
            Self::Image(image) => vec![image.at],
            Self::Card { anchor, .. } => vec![*anchor],
            Self::RotatedRectangle { a, b, width } => vec![*a, *b, *width],
            Self::Circle { center, edge } => vec![*center, *edge],
            Self::Triangle { points } => points.to_vec(),
            Self::Arc {
                start,
                end,
                through,
            } => vec![*start, *end, *through],
            Self::Curve { a, b, control } => vec![*a, *b, *control],
            Self::DoubleCurve { a, b, c1, c2 } => vec![*a, *b, *c1, *c2],
            Self::TrendLine { a, b }
            | Self::Ray { a, b }
            | Self::InfoLine { a, b }
            | Self::ExtendedLine { a, b }
            | Self::TrendAngle { a, b }
            | Self::Regression { a, b } => vec![*a, *b],
            Self::HorizontalLine { at }
            | Self::HorizontalRay { at }
            | Self::VerticalLine { at }
            | Self::Crossline { at } => vec![*at],
            Self::ParallelChannel { a, b, c } | Self::FlatChannel { a, b, c } => {
                vec![*a, *b, *c]
            }
            Self::DisjointChannel { a, b, c, d } => vec![*a, *b, *c, *d],
            Self::Pitchfork { p1, p2, p3, .. } => vec![*p1, *p2, *p3],
        }
    }

    /// Mutable access to every anchor, in placement order.
    pub fn anchors_mut(&mut self) -> Vec<&mut Anchor> {
        match self {
            Self::Highlighter { points }
            | Self::Path { points }
            | Self::Polyline { points, .. }
            | Self::Fib { points, .. }
            | Self::Pattern { points, .. }
            | Self::Wave { points, .. } => points.iter_mut().collect(),
            Self::ArrowMarker { a, b }
            | Self::Arrow { a, b }
            | Self::Rectangle { a, b }
            | Self::Cycle { a, b, .. }
            | Self::FixedProfile { a, b }
            | Self::Measure { a, b, .. }
            | Self::Forecast {
                source: a,
                target: b,
            }
            | Self::Ghost {
                start: a, end: b, ..
            }
            | Self::Ellipse { a, b } => vec![a, b],
            Self::Position {
                entry,
                target,
                stop,
                ..
            } => vec![entry, target, stop],
            Self::Sector {
                center,
                radius,
                tip,
            } => vec![center, radius, tip],
            Self::Bars(pattern) => vec![&mut pattern.at],
            Self::Vwap { at } | Self::AnchoredProfile { at } => vec![at],
            Self::Leader { at, label, .. }
            | Self::Signpost {
                anchor: at, label, ..
            } => vec![at, label],
            Self::ArrowMark { at, .. } | Self::Annotation { at, .. } => vec![at],
            Self::Table(table) if table.screen.is_none() => vec![&mut table.at],
            Self::Table(_) => Vec::new(),
            Self::Image(image) => vec![&mut image.at],
            Self::Card { anchor, .. } => vec![anchor],
            Self::RotatedRectangle { a, b, width } => vec![a, b, width],
            Self::Circle { center, edge } => vec![center, edge],
            Self::Triangle { points } => points.iter_mut().collect(),
            Self::Arc {
                start,
                end,
                through,
            } => vec![start, end, through],
            Self::Curve { a, b, control } => vec![a, b, control],
            Self::DoubleCurve { a, b, c1, c2 } => vec![a, b, c1, c2],
            Self::TrendLine { a, b }
            | Self::Ray { a, b }
            | Self::InfoLine { a, b }
            | Self::ExtendedLine { a, b }
            | Self::TrendAngle { a, b }
            | Self::Regression { a, b } => vec![a, b],
            Self::HorizontalLine { at }
            | Self::HorizontalRay { at }
            | Self::VerticalLine { at }
            | Self::Crossline { at } => vec![at],
            Self::ParallelChannel { a, b, c } | Self::FlatChannel { a, b, c } => vec![a, b, c],
            Self::DisjointChannel { a, b, c, d } => vec![a, b, c, d],
            Self::Pitchfork { p1, p2, p3, .. } => vec![p1, p2, p3],
        }
    }

    /// Which parts of anchor `index` mean something, as `(time, price)`. The others are where
    /// a handle was left and are never read, so the Coordinates tab does not offer them.
    pub fn reads(&self, index: usize) -> (bool, bool) {
        match (self, index) {
            (Self::HorizontalLine { .. }, _) | (Self::FlatChannel { .. }, 2) => (false, true),
            (Self::VerticalLine { .. }, _)
            | (Self::Regression { .. }, _)
            | (Self::DisjointChannel { .. }, 3) => (true, false),
            // A signpost's plate stays on its anchor's bar: only its height is the user's.
            (Self::Signpost { .. }, 1) => (false, true),
            // A position's stop is under its target: only its price is its own.
            (Self::Position { .. }, 2) => (false, true),
            // What a VWAP or a profile reads is the bar, not the price the handle was left at.
            (Self::Vwap { .. } | Self::AnchoredProfile { .. } | Self::FixedProfile { .. }, _) => {
                (true, false)
            }
            (
                Self::Measure {
                    kind: MeasureKind::Date,
                    ..
                },
                _,
            ) => (true, false),
            _ => (true, true),
        }
    }

    /// `self`, just edited from `original`, brought back to what its tool can be: a horizontal
    /// line keeps its time and a vertical one its price (the handle slid along the line), and a
    /// flat channel's level edge keeps its time at the middle of the sloped one.
    pub fn constrain(&mut self, original: &Self) {
        match (&mut *self, original) {
            (Self::HorizontalLine { at }, Self::HorizontalLine { at: before }) => {
                at.time_ms = before.time_ms;
            }
            (Self::VerticalLine { at }, Self::VerticalLine { at: before }) => {
                at.price = before.price;
            }
            (Self::FlatChannel { a, b, c }, _) => {
                c.time_ms = a.time_ms / 2 + b.time_ms / 2 + (a.time_ms % 2 + b.time_ms % 2) / 2;
            }
            (Self::Signpost { anchor, label }, _) => label.time_ms = anchor.time_ms,
            (Self::Position { target, stop, .. }, _) => stop.time_ms = target.time_ms,
            _ => {}
        }
    }

    /// Whether the geometry is one a tool could have drawn: finite coordinates, enough
    /// vertices and not an absurd number of them.
    pub fn is_valid(&self) -> bool {
        let anchors = self.anchors();
        let enough = match self {
            Self::Highlighter { .. } => !anchors.is_empty(),
            Self::Path { .. } => anchors.len() >= 2,
            Self::Polyline { closed, .. } => anchors.len() >= if *closed { 3 } else { 2 },
            Self::Table(table) => table.is_valid(),
            Self::Image(image) => image.is_valid(),
            Self::Card { card, .. } => card.is_valid(),
            // A pattern or a wave has its tool's number of pivots and no other.
            Self::Fib { kind, .. } => anchors.len() == kind.points(),
            Self::Pattern { kind, .. } => anchors.len() == kind.points(),
            Self::Wave { kind, .. } => anchors.len() == kind.points(),
            Self::Bars(pattern) => pattern.is_valid(),
            _ => true,
        };
        enough && anchors.len() <= MAX_POINTS && anchors.iter().all(Anchor::is_finite)
    }

    /// What `tool` would draw if the pointer sat on the last of `anchors` now: the anchors
    /// placed so far plus the one following the pointer, with any still missing filled in so
    /// the preview is already a recognisable shape (a curve starts out straight, a triangle as a
    /// line). Never rejected for being degenerate, because a preview may be; `None` only for a
    /// tool that draws no geometry or for no anchors.
    pub fn preview(tool: Tool, anchors: &[Anchor]) -> Option<Self> {
        let last = *anchors.last()?;
        if !anchors.iter().all(Anchor::is_finite) {
            return None;
        }
        let first = anchors[0];
        let at = |index: usize| anchors.get(index).copied().unwrap_or(last);
        let mid = |a: Anchor, b: Anchor| Anchor {
            time_ms: a.time_ms + (b.time_ms - a.time_ms) / 2,
            price: (a.price + b.price) / 2.0,
        };
        let along = |a: Anchor, b: Anchor, t: f64| Anchor {
            time_ms: a.time_ms + ((b.time_ms - a.time_ms) as f64 * t) as i64,
            price: a.price + (b.price - a.price) * t,
        };
        Some(match tool {
            Tool::Highlighter => Self::Highlighter {
                points: anchors.to_vec(),
            },
            Tool::ArrowMarker => Self::ArrowMarker { a: first, b: last },
            Tool::Arrow => Self::Arrow { a: first, b: last },
            Tool::ArrowMarkUp => Self::ArrowMark {
                at: last,
                direction: ArrowDirection::Up,
            },
            Tool::ArrowMarkDown => Self::ArrowMark {
                at: last,
                direction: ArrowDirection::Down,
            },
            Tool::Rectangle => Self::Rectangle { a: first, b: last },
            Tool::RotatedRectangle => Self::RotatedRectangle {
                a: first,
                b: at(1),
                width: last,
            },
            Tool::Path => Self::Path {
                points: anchors.to_vec(),
            },
            Tool::Polyline => Self::Polyline {
                points: anchors.to_vec(),
                closed: false,
            },
            Tool::Circle => Self::Circle {
                center: first,
                edge: last,
            },
            Tool::Ellipse => Self::Ellipse { a: first, b: last },
            Tool::Triangle => Self::Triangle {
                points: [first, at(1), at(2)],
            },
            Tool::Arc => Self::Arc {
                start: first,
                end: at(1),
                through: last,
            },
            Tool::Curve => {
                if anchors.len() >= 3 {
                    Self::Curve {
                        a: first,
                        b: anchors[1],
                        control: last,
                    }
                } else {
                    // Straight until the control is placed.
                    Self::Curve {
                        a: first,
                        b: last,
                        control: mid(first, last),
                    }
                }
            }
            Tool::DoubleCurve => match anchors.len() {
                0..=2 => Self::DoubleCurve {
                    a: first,
                    b: last,
                    c1: along(first, last, 1.0 / 3.0),
                    c2: along(first, last, 2.0 / 3.0),
                },
                3 => Self::DoubleCurve {
                    a: first,
                    b: anchors[1],
                    c1: last,
                    c2: last,
                },
                _ => Self::DoubleCurve {
                    a: first,
                    b: anchors[1],
                    c1: anchors[2],
                    c2: last,
                },
            },
            Tool::TrendLine => Self::TrendLine { a: first, b: last },
            Tool::Ray => Self::Ray { a: first, b: last },
            Tool::InfoLine => Self::InfoLine { a: first, b: last },
            Tool::ExtendedLine => Self::ExtendedLine { a: first, b: last },
            Tool::TrendAngle => Self::TrendAngle { a: first, b: last },
            Tool::HorizontalLine => Self::HorizontalLine { at: last },
            Tool::HorizontalRay => Self::HorizontalRay { at: last },
            Tool::VerticalLine => Self::VerticalLine { at: last },
            Tool::Crossline => Self::Crossline { at: last },
            // The first edge follows the pointer until it is placed; then the pointer pulls the
            // second edge out beside it.
            Tool::ParallelChannel => Self::ParallelChannel {
                a: first,
                b: at(1),
                c: last,
            },
            Tool::FlatChannel => {
                let (a, b) = (first, at(1));
                Self::FlatChannel {
                    a,
                    b,
                    c: Anchor {
                        time_ms: mid(a, b).time_ms,
                        price: last.price,
                    },
                }
            }
            Tool::DisjointChannel => {
                let (a, b) = (first, at(1));
                let c = if anchors.len() >= 3 { last } else { a };
                Self::DisjointChannel {
                    a,
                    b,
                    c,
                    d: disjoint_end(a, b, c),
                }
            }
            Tool::RegressionTrend => Self::Regression { a: first, b: last },
            Tool::Pitchfork
            | Tool::SchiffPitchfork
            | Tool::ModifiedSchiffPitchfork
            | Tool::InsidePitchfork => Self::Pitchfork {
                variant: PitchforkVariant::of_tool(tool)?,
                p1: first,
                p2: at(1),
                p3: last,
            },
            Tool::Table => Self::Table(TableData::new(last)),
            Tool::Note | Tool::PriceNote | Tool::Callout => Self::Leader {
                kind: LeaderKind::of_tool(tool)?,
                at: first,
                label: last,
            },
            Tool::Signpost => Self::Signpost {
                anchor: first,
                label: Anchor {
                    time_ms: first.time_ms,
                    price: last.price,
                },
            },
            Tool::Text | Tool::Comment | Tool::PriceLabel | Tool::FlagMark | Tool::Pin => {
                Self::Annotation {
                    kind: AnnotationKind::of_tool(tool)?,
                    at: last,
                }
            }
            // The pivots placed so far, then the pointer for every one still to come, so the
            // preview is already the whole shape: it moves with the pointer and settles as each
            // pivot is placed.
            Tool::FibRetracement => {
                let kind = FibKind::of_tool(tool)?;
                Self::Fib {
                    kind,
                    points: (0..kind.points()).map(at).collect(),
                }
            }
            Tool::XabcdPattern
            | Tool::CypherPattern
            | Tool::HeadAndShoulders
            | Tool::AbcdPattern
            | Tool::TrianglePattern
            | Tool::ThreeDrivesPattern => {
                let kind = PatternKind::of_tool(tool)?;
                Self::Pattern {
                    kind,
                    points: (0..kind.points()).map(at).collect(),
                }
            }
            Tool::ElliottImpulse
            | Tool::ElliottCorrection
            | Tool::ElliottTriangle
            | Tool::ElliottDoubleCombo
            | Tool::ElliottTripleCombo => {
                let kind = WaveKind::of_tool(tool)?;
                Self::Wave {
                    kind,
                    points: (0..kind.points()).map(at).collect(),
                }
            }
            Tool::CyclicLines | Tool::TimeCycles | Tool::SineLine => Self::Cycle {
                kind: CycleKind::of_tool(tool)?,
                a: first,
                b: last,
            },
            // The layer sizes a position from the view; this is its entry alone.
            Tool::LongPosition => Self::default_position(Side::Long, last),
            Tool::ShortPosition => Self::default_position(Side::Short, last),
            Tool::PositionForecast => Self::Forecast {
                source: first,
                target: last,
            },
            Tool::GhostFeed => Self::Ghost {
                start: first,
                end: last,
                seed: Self::ghost_seed(first),
            },
            Tool::Sector => Self::Sector {
                center: first,
                radius: at(1),
                tip: last,
            },
            Tool::AnchoredVwap => Self::Vwap { at: last },
            Tool::FixedRangeVolumeProfile => Self::FixedProfile { a: first, b: last },
            Tool::AnchoredVolumeProfile => Self::AnchoredProfile { at: last },
            Tool::PriceRange | Tool::DateRange | Tool::DateAndPriceRange => Self::Measure {
                kind: MeasureKind::of_tool(tool)?,
                a: first,
                b: last,
            },
            // A picture has to be chosen before it can be shown, and a bars pattern copies the bars
            // the chart has, which a geometry alone does not hold: the layer previews them.
            Tool::Cursor
            | Tool::Brush
            | Tool::Ruler
            | Tool::Image
            | Tool::Post
            | Tool::BarsPattern => {
                return None;
            }
        })
    }

    /// The geometry `tool` draws through `anchors`, or `None` when they do not fit it (wrong
    /// count, not finite, or a shape with no extent such as a rectangle of one point).
    pub fn from_anchors(tool: Tool, anchors: &[Anchor]) -> Option<Self> {
        if anchors.len() > MAX_POINTS || !anchors.iter().all(Anchor::is_finite) {
            return None;
        }
        let geometry = match (tool, anchors) {
            (Tool::Highlighter, points) if !points.is_empty() => Self::Highlighter {
                points: points.to_vec(),
            },
            (Tool::ArrowMarker, [a, b]) if a != b => Self::ArrowMarker { a: *a, b: *b },
            (Tool::Arrow, [a, b]) if a != b => Self::Arrow { a: *a, b: *b },
            (Tool::ArrowMarkUp, [at]) => Self::ArrowMark {
                at: *at,
                direction: ArrowDirection::Up,
            },
            (Tool::ArrowMarkDown, [at]) => Self::ArrowMark {
                at: *at,
                direction: ArrowDirection::Down,
            },
            (Tool::Rectangle, [a, b]) if a != b => Self::Rectangle { a: *a, b: *b },
            (Tool::RotatedRectangle, [a, b, width]) if a != b => Self::RotatedRectangle {
                a: *a,
                b: *b,
                width: *width,
            },
            (Tool::Path, points) if points.len() >= 2 => Self::Path {
                points: points.to_vec(),
            },
            (Tool::Polyline, points) if points.len() >= 2 => Self::Polyline {
                points: points.to_vec(),
                closed: false,
            },
            (Tool::Circle, [center, edge]) if center != edge => Self::Circle {
                center: *center,
                edge: *edge,
            },
            (Tool::Ellipse, [a, b]) if a != b => Self::Ellipse { a: *a, b: *b },
            (Tool::Triangle, [a, b, c]) => Self::Triangle {
                points: [*a, *b, *c],
            },
            (Tool::Arc, [start, end, through]) if start != end => Self::Arc {
                start: *start,
                end: *end,
                through: *through,
            },
            (Tool::Curve, [a, b, control]) if a != b => Self::Curve {
                a: *a,
                b: *b,
                control: *control,
            },
            (Tool::DoubleCurve, [a, b, c1, c2]) if a != b => Self::DoubleCurve {
                a: *a,
                b: *b,
                c1: *c1,
                c2: *c2,
            },
            (Tool::TrendLine, [a, b]) if a != b => Self::TrendLine { a: *a, b: *b },
            (Tool::Ray, [a, b]) if a != b => Self::Ray { a: *a, b: *b },
            (Tool::InfoLine, [a, b]) if a != b => Self::InfoLine { a: *a, b: *b },
            (Tool::ExtendedLine, [a, b]) if a != b => Self::ExtendedLine { a: *a, b: *b },
            (Tool::TrendAngle, [a, b]) if a != b => Self::TrendAngle { a: *a, b: *b },
            (Tool::HorizontalLine, [at]) => Self::HorizontalLine { at: *at },
            (Tool::HorizontalRay, [at]) => Self::HorizontalRay { at: *at },
            (Tool::VerticalLine, [at]) => Self::VerticalLine { at: *at },
            (Tool::Crossline, [at]) => Self::Crossline { at: *at },
            (tool, [at, label]) if at != label && LeaderKind::of_tool(tool).is_some() => {
                Self::Leader {
                    kind: LeaderKind::of_tool(tool)?,
                    at: *at,
                    label: *label,
                }
            }
            // The plate is straight above or below the anchor, never on it.
            (Tool::Signpost, [anchor, label]) if anchor.price != label.price => Self::Signpost {
                anchor: *anchor,
                label: Anchor {
                    time_ms: anchor.time_ms,
                    price: label.price,
                },
            },
            (Tool::Table, [at]) => Self::Table(TableData::new(*at)),
            (tool, [at]) if AnnotationKind::of_tool(tool).is_some() => Self::Annotation {
                kind: AnnotationKind::of_tool(tool)?,
                at: *at,
            },
            (Tool::ParallelChannel, [a, b, c]) if a != b => Self::ParallelChannel {
                a: *a,
                b: *b,
                c: *c,
            },
            // A regression needs two different bars to fit a line through.
            (Tool::RegressionTrend, [a, b]) if a.time_ms != b.time_ms => {
                Self::Regression { a: *a, b: *b }
            }
            (Tool::FlatChannel, [a, b, c]) if a != b => Self::FlatChannel {
                a: *a,
                b: *b,
                c: Anchor {
                    time_ms: a.time_ms / 2 + b.time_ms / 2 + (a.time_ms % 2 + b.time_ms % 2) / 2,
                    price: c.price,
                },
            },
            (Tool::DisjointChannel, [a, b, c]) if a != b => Self::DisjointChannel {
                a: *a,
                b: *b,
                c: *c,
                d: disjoint_end(*a, *b, *c),
            },
            // A Fibonacci drawing needs all its anchors, and not every one on the same point.
            (tool, points)
                if FibKind::of_tool(tool).is_some_and(|kind| kind.points() == points.len())
                    && points.windows(2).any(|pair| pair[0] != pair[1]) =>
            {
                Self::Fib {
                    kind: FibKind::of_tool(tool)?,
                    points: points.to_vec(),
                }
            }
            // A pattern needs all its pivots, and not every one on the same point.
            (tool, points)
                if PatternKind::of_tool(tool).is_some_and(|kind| kind.points() == points.len())
                    && points.windows(2).any(|pair| pair[0] != pair[1]) =>
            {
                Self::Pattern {
                    kind: PatternKind::of_tool(tool)?,
                    points: points.to_vec(),
                }
            }
            (tool, points)
                if WaveKind::of_tool(tool).is_some_and(|kind| kind.points() == points.len())
                    && points.windows(2).any(|pair| pair[0] != pair[1]) =>
            {
                Self::Wave {
                    kind: WaveKind::of_tool(tool)?,
                    points: points.to_vec(),
                }
            }
            (Tool::LongPosition, [entry]) => Self::default_position(Side::Long, *entry),
            (Tool::ShortPosition, [entry]) => Self::default_position(Side::Short, *entry),
            (Tool::LongPosition | Tool::ShortPosition, [entry, target, stop]) => {
                let side = if tool == Tool::LongPosition {
                    Side::Long
                } else {
                    Side::Short
                };
                Self::Position {
                    side,
                    entry: *entry,
                    target: *target,
                    // The stop is under the target.
                    stop: Anchor {
                        time_ms: target.time_ms,
                        price: stop.price,
                    },
                }
            }
            (Tool::PositionForecast, [source, target]) if source != target => Self::Forecast {
                source: *source,
                target: *target,
            },
            (Tool::GhostFeed, [start, end]) if start != end => Self::Ghost {
                start: *start,
                end: *end,
                seed: Self::ghost_seed(*start),
            },
            (Tool::Sector, [center, radius, tip]) if center != radius => Self::Sector {
                center: *center,
                radius: *radius,
                tip: *tip,
            },
            (Tool::AnchoredVwap, [at]) => Self::Vwap { at: *at },
            (Tool::AnchoredVolumeProfile, [at]) => Self::AnchoredProfile { at: *at },
            // A profile of a range needs a range: two different times.
            (Tool::FixedRangeVolumeProfile, [a, b]) if a.time_ms != b.time_ms => {
                Self::FixedProfile { a: *a, b: *b }
            }
            (Tool::PriceRange | Tool::DateRange | Tool::DateAndPriceRange, [a, b]) if a != b => {
                Self::Measure {
                    kind: MeasureKind::of_tool(tool)?,
                    a: *a,
                    b: *b,
                }
            }
            // A cycle's period is the distance in time between its anchors.
            (tool, [a, b]) if CycleKind::of_tool(tool).is_some() && a.time_ms != b.time_ms => {
                Self::Cycle {
                    kind: CycleKind::of_tool(tool)?,
                    a: *a,
                    b: *b,
                }
            }
            (tool, [p1, p2, p3]) if p1 != p2 && PitchforkVariant::of_tool(tool).is_some() => {
                Self::Pitchfork {
                    variant: PitchforkVariant::of_tool(tool)?,
                    p1: *p1,
                    p2: *p2,
                    p3: *p3,
                }
            }
            _ => return None,
        };
        Some(geometry)
    }
}

/// A drawing on a chart, keyed by a stable id.
///
/// Fibonacci, the brush and the ruler have variants of their own; every other tool, the trend
/// line included, is one [`Drawing::Shape`] whose geometry says which.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum Drawing {
    Brush {
        id: u64,
        points: Vec<Anchor>,
        width: f32,
        locked: bool,
        #[serde(default = "brush_style")]
        style: DrawingStyle,
        #[serde(default)]
        hidden: bool,
        #[serde(default)]
        visibility: Visibility,
    },
    Ruler {
        id: u64,
        a: Anchor,
        b: Anchor,
    },
    /// Every other tool.
    Shape {
        id: u64,
        geometry: Geometry,
        #[serde(default)]
        style: DrawingStyle,
        #[serde(default)]
        text: String,
        #[serde(default)]
        locked: bool,
        #[serde(default)]
        hidden: bool,
        #[serde(default)]
        visibility: Visibility,
    },
    /// A stored drawing this version cannot decode (a newer tool, or damage). It is kept and
    /// written back byte for byte, never drawn, edited or counted, so nothing a newer build
    /// saved is lost by opening the chart here.
    #[serde(skip)]
    Unreadable(serde_json::Value),
}

fn brush_style() -> DrawingStyle {
    Tool::Brush.default_style()
}

impl Drawing {
    pub fn id(&self) -> u64 {
        match self {
            Self::Brush { id, .. } | Self::Ruler { id, .. } | Self::Shape { id, .. } => *id,
            Self::Unreadable(_) => 0,
        }
    }

    /// The id a row was saved with, even one this version cannot read, so a new drawing never
    /// takes an id a newer version stored.
    pub fn stored_id(&self) -> u64 {
        match self {
            Self::Unreadable(raw) => raw
                .as_object()
                .and_then(|variants| variants.values().next())
                .and_then(|body| body.get("id"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            other => other.id(),
        }
    }

    /// A drawing by any tool for a test that goes through every tool: a picture tool gets a
    /// picture to show, the rest are built as usual. Not for the app: a real picture is chosen.
    #[cfg(test)]
    pub fn sample(tool: Tool, id: u64, anchors: &[Anchor], style: DrawingStyle) -> Option<Self> {
        if tool == Tool::BarsPattern {
            // The bars a person would have copied from a chart, set down at the last anchor.
            let at = *anchors.last()?;
            let bars = (0..5)
                .map(|index| {
                    let step = index as f64;
                    [step, step + 2.0, step - 1.0, step + 1.0]
                })
                .collect();
            let pattern = BarsPattern {
                at,
                from_ms: anchors[0].time_ms,
                to_ms: anchors[0].time_ms + 4 * 60_000,
                bars,
                mode: Default::default(),
                mirrored: false,
                flipped: false,
            };
            return Self::from_geometry(id, Geometry::Bars(pattern), style);
        }
        if tool == Tool::Post {
            let link = "https://x.com/degenview/status/1750000000000000000";
            let card = LinkCard::new(super::link_card::Link::parse(link).ok()?);
            Self::build_with(tool, id, anchors, style, &Content::Link(card))
        } else if tool.caps().has(super::Caps::CONTENT) {
            let picture = Content::Image(super::image_content::ImageContent::new(
                "a".repeat(64),
                "sample.png",
                [200, 100],
            ));
            Self::build_with(tool, id, anchors, style, &picture)
        } else {
            Self::build(tool, id, anchors, style)
        }
    }

    /// A new drawing of what was chosen (a picture) through `anchors`. `None` when they do not
    /// make it.
    pub fn build_with(
        tool: Tool,
        id: u64,
        anchors: &[Anchor],
        style: DrawingStyle,
        content: &Content,
    ) -> Option<Self> {
        let geometry = match (tool, anchors, content) {
            (Tool::Image, [at], Content::Image(picture)) => {
                let image = ImageData::new(*at, picture);
                image.is_valid().then_some(Geometry::Image(image))?
            }
            (Tool::Post, [at], Content::Link(card)) => {
                let card = card.clone().sanitized();
                card.is_valid()
                    .then_some(Geometry::Card { anchor: *at, card })?
            }
            _ => return None,
        };
        Some(Self::Shape {
            id,
            geometry,
            style: style.sanitized(),
            text: String::new(),
            locked: false,
            hidden: false,
            visibility: Visibility::ALL,
        })
    }

    /// A new drawing of a geometry made by something other than its anchors alone (a bars
    /// pattern, which copies the chart's bars), in `style`. `None` when the geometry is not valid.
    pub fn from_geometry(id: u64, geometry: Geometry, style: DrawingStyle) -> Option<Self> {
        geometry.is_valid().then(|| Self::Shape {
            id,
            geometry,
            style: style.sanitized(),
            text: String::new(),
            locked: false,
            hidden: false,
            visibility: Visibility::ALL,
        })
    }

    /// A new drawing by `tool` through `anchors`, in `style`. `None` when the anchors do not
    /// make that shape; the caller commits nothing then.
    pub fn build(tool: Tool, id: u64, anchors: &[Anchor], style: DrawingStyle) -> Option<Self> {
        let style = style.sanitized();
        let at = |index: usize| anchors.get(index).copied();
        Some(match tool {
            Tool::Cursor => return None,
            Tool::Ruler => Self::Ruler {
                id,
                a: at(0)?,
                b: at(1)?,
            },
            Tool::Brush => {
                if anchors.is_empty()
                    || anchors.len() > MAX_POINTS
                    || !anchors.iter().all(Anchor::is_finite)
                {
                    return None;
                }
                Self::Brush {
                    id,
                    points: anchors.to_vec(),
                    width: style.stroke.width,
                    locked: false,
                    style,
                    hidden: false,
                    visibility: Visibility::ALL,
                }
            }
            tool => Self::Shape {
                id,
                geometry: Geometry::from_anchors(tool, anchors)?,
                style,
                text: String::new(),
                locked: false,
                hidden: false,
                visibility: Visibility::ALL,
            },
        })
    }

    /// The ruler is a transient measurement: never saved, never in undo history.
    pub fn is_ruler(&self) -> bool {
        matches!(self, Self::Ruler { .. })
    }

    /// The tool that draws this, `None` for an unreadable row.
    pub fn tool(&self) -> Option<Tool> {
        match self {
            Self::Brush { .. } => Some(Tool::Brush),
            Self::Ruler { .. } => Some(Tool::Ruler),
            Self::Shape { geometry, .. } => Some(geometry.tool()),
            Self::Unreadable(_) => None,
        }
    }

    pub fn locked(&self) -> bool {
        match self {
            Self::Brush { locked, .. } | Self::Shape { locked, .. } => *locked,
            Self::Ruler { .. } | Self::Unreadable(_) => false,
        }
    }

    pub fn hidden(&self) -> bool {
        match self {
            Self::Brush { hidden, .. } | Self::Shape { hidden, .. } => *hidden,
            Self::Ruler { .. } | Self::Unreadable(_) => false,
        }
    }

    pub fn visibility(&self) -> Visibility {
        match self {
            Self::Brush { visibility, .. } | Self::Shape { visibility, .. } => *visibility,
            Self::Ruler { .. } | Self::Unreadable(_) => Visibility::ALL,
        }
    }

    /// Whether the drawing is drawn and can be picked: not hidden, shown on `timeframe`, and
    /// readable. The drawing itself is never changed by this; it is a predicate on the view.
    pub fn is_visible(&self, timeframe: Timeframe) -> bool {
        !matches!(self, Self::Unreadable(_)) && !self.hidden() && self.visibility().shows(timeframe)
    }

    pub fn set_locked(&mut self, value: bool) {
        match self {
            Self::Brush { locked, .. } | Self::Shape { locked, .. } => *locked = value,
            Self::Ruler { .. } | Self::Unreadable(_) => {}
        }
    }

    pub fn set_hidden(&mut self, value: bool) {
        match self {
            Self::Brush { hidden, .. } | Self::Shape { hidden, .. } => *hidden = value,
            Self::Ruler { .. } | Self::Unreadable(_) => {}
        }
    }

    pub fn set_visibility(&mut self, value: Visibility) {
        match self {
            Self::Brush { visibility, .. } | Self::Shape { visibility, .. } => *visibility = value,
            Self::Ruler { .. } | Self::Unreadable(_) => {}
        }
    }

    /// The style, for the drawings that carry one (the brush and every shape).
    pub fn style(&self) -> Option<&DrawingStyle> {
        match self {
            Self::Brush { style, .. } | Self::Shape { style, .. } => Some(style),
            _ => None,
        }
    }

    /// Replaces the style (sanitized). The brush's own `width` follows its stroke width.
    pub fn set_style(&mut self, value: DrawingStyle) {
        let value = value.sanitized();
        match self {
            Self::Brush { style, width, .. } => {
                *width = value.stroke.width;
                *style = value;
            }
            Self::Shape { style, .. } => *style = value,
            _ => {}
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Shape { text, .. } => text,
            _ => "",
        }
    }

    /// The most characters of text this drawing may carry.
    pub fn max_text(&self) -> usize {
        self.tool().map_or(MAX_TEXT_CHARS, Tool::max_text)
    }

    /// Sets the text of a drawing that has any (a shape); the rest ignore it.
    pub fn set_text(&mut self, value: &str) {
        let limit = self.max_text();
        if let Self::Shape { text, .. } = self {
            *text = value.chars().take(limit).collect();
        }
    }

    /// Whether the drawing's text is typed in place on the chart (a line's, a channel's, a
    /// note's): its tool says so with [`Caps::INLINE_TEXT`]. The text of a shape or an arrow is
    /// edited in the dialog, as it always was.
    pub fn edits_text_in_place(&self) -> bool {
        self.tool()
            .is_some_and(|tool| tool.caps().has(super::Caps::INLINE_TEXT))
    }

    /// Sets one cell of a table drawing. `false` for another drawing or a cell outside the grid.
    pub fn set_table_cell(&mut self, row: usize, column: usize, text: &str) -> bool {
        if let Some(Geometry::Table(table)) = self.geometry_mut() {
            table.set_cell(row, column, text)
        } else {
            false
        }
    }

    /// What to say of a drawing whose words are not its own text (a table's size), for lists.
    pub fn summary(&self) -> Option<String> {
        match self.geometry()? {
            Geometry::Table(table) => Some(table.summary()),
            Geometry::Image(image) => Some(image.name.clone()).filter(|name| !name.is_empty()),
            Geometry::Card { card, .. } => Some(card.summary()),
            Geometry::Bars(pattern) => Some(format!("{} bars", pattern.bars.len())),
            _ => None,
        }
    }

    /// Every anchor in placement order.
    pub fn anchors(&self) -> Vec<Anchor> {
        match self {
            Self::Ruler { a, b, .. } => vec![*a, *b],
            Self::Brush { points, .. } => points.clone(),
            Self::Shape { geometry, .. } => geometry.anchors(),
            Self::Unreadable(_) => Vec::new(),
        }
    }

    /// Mutable access to every anchor in placement order, to move or edit them.
    pub fn anchors_mut(&mut self) -> Vec<&mut Anchor> {
        match self {
            Self::Ruler { a, b, .. } => vec![a, b],
            Self::Brush { points, .. } => points.iter_mut().collect(),
            Self::Shape { geometry, .. } => geometry.anchors_mut(),
            Self::Unreadable(_) => Vec::new(),
        }
    }

    /// The geometry of a shape drawing.
    pub fn geometry(&self) -> Option<&Geometry> {
        match self {
            Self::Shape { geometry, .. } => Some(geometry),
            _ => None,
        }
    }

    pub fn geometry_mut(&mut self) -> Option<&mut Geometry> {
        match self {
            Self::Shape { geometry, .. } => Some(geometry),
            _ => None,
        }
    }

    /// This drawing under a new id (a duplicate).
    pub fn with_id(mut self, new_id: u64) -> Self {
        match &mut self {
            Self::Brush { id, .. } | Self::Ruler { id, .. } | Self::Shape { id, .. } => {
                *id = new_id
            }
            Self::Unreadable(_) => {}
        }
        self
    }
}

/// A mutation the renderer requests. The workspace applies it against the undo
/// stack; the renderer never mutates the model directly.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawingCommand {
    Add(Drawing),
    Replace {
        id: u64,
        drawing: Drawing,
    },
    /// A restyle: like `Replace`, but consecutive restyles of one drawing in quick succession
    /// (dragging a colour picker) merge into one undo step.
    Style {
        id: u64,
        drawing: Drawing,
    },
    Remove(u64),
    /// Moves a drawing in the stacking order: positive steps bring it forward (later, drawn
    /// over more), negative send it back.
    Reorder {
        id: u64,
        step: i32,
    },
}

/// Applies a batch of commands to a chart's drawings. Returns whether anything other than the
/// transient ruler changed, which is what makes the batch one undo step: a batch that only
/// adds a ruler, replaces a drawing with itself or names one that is gone is no edit at all.
pub fn apply_commands(drawings: &mut Vec<Drawing>, commands: Vec<DrawingCommand>) -> bool {
    let mut changed = false;
    for command in commands {
        match command {
            DrawingCommand::Add(drawing) => {
                changed |= !drawing.is_ruler();
                drawings.push(drawing);
            }
            DrawingCommand::Replace { id, drawing } | DrawingCommand::Style { id, drawing } => {
                if let Some(slot) = drawings.iter_mut().find(|d| d.id() == id)
                    && *slot != drawing
                {
                    changed |= !slot.is_ruler();
                    *slot = drawing;
                }
            }
            DrawingCommand::Remove(id) => {
                let before = drawings.len();
                let ruler = drawings.iter().any(|d| d.id() == id && d.is_ruler());
                // By the id a row was saved with, so one this version cannot read can be deleted.
                drawings.retain(|d| d.stored_id() != id);
                changed |= drawings.len() != before && !ruler;
            }
            DrawingCommand::Reorder { id, step } => {
                if let Some(index) = drawings.iter().position(|d| d.id() == id) {
                    let last = drawings.len() as i64 - 1;
                    let target = (index as i64 + i64::from(step)).clamp(0, last) as usize;
                    if target != index && !drawings[index].is_ruler() {
                        let drawing = drawings.remove(index);
                        drawings.insert(target, drawing);
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// The newest drawing format this build reads.
///
/// A new tool needs nothing here: it is a new `Geometry` variant, which an older build cannot
/// decode, so it keeps the drawing as [`Drawing::Unreadable`] and writes it back untouched. A new
/// field on a tool that already exists is the dangerous change: an older build would decode the
/// drawing, ignore the field, and write it back without it (and sync the stripped copy over the
/// newer one). So a build that adds a field an older build would drop must write `"since": N`
/// (N = this constant plus one, and this constant raised to N) inside the drawing's body, and an
/// older build then treats the drawing as unreadable. See `docs/agent-guides/charts.md`.
pub const DRAWING_FORMAT: u32 = 1;

/// Whether a stored drawing says it was written for a newer format than this build reads: a
/// `since` number in its body (`{"Shape": {"id": 1, "since": 2, ...}}`) above [`DRAWING_FORMAT`].
fn written_for_a_newer_format(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .and_then(|variant| variant.values().next())
        .and_then(|body| body.get("since"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|since| since > u64::from(DRAWING_FORMAT))
}

/// Serde adapter for a chart's stored drawings that decodes each one on its own. A row this
/// version cannot read stays in the list as [`Drawing::Unreadable`] and is written back as it
/// was, instead of failing the whole chart (and with it the tab or layout that holds it).
pub mod lenient {
    use super::Drawing;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        drawings: &[Drawing],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let values: Result<Vec<serde_json::Value>, _> = drawings
            .iter()
            .map(|drawing| match drawing {
                Drawing::Unreadable(raw) => Ok(raw.clone()),
                other => serde_json::to_value(other),
            })
            .collect();
        values
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Drawing>, D::Error> {
        let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
        Ok(values
            .into_iter()
            .map(|value| {
                if super::written_for_a_newer_format(&value) {
                    return Drawing::Unreadable(value);
                }
                serde_json::from_value::<Drawing>(value.clone())
                    .ok()
                    .filter(drawing_is_sane)
                    .unwrap_or(Drawing::Unreadable(value))
            })
            .collect())
    }

    /// A decoded drawing that still could not be drawn (a shape with no vertices, a
    /// non-finite price) is kept as unreadable too rather than reaching the renderer.
    fn drawing_is_sane(drawing: &Drawing) -> bool {
        match drawing {
            Drawing::Shape { geometry, .. } => geometry.is_valid(),
            Drawing::Brush { points, .. } => {
                points.len() <= super::MAX_POINTS && points.iter().all(super::Anchor::is_finite)
            }
            other => other.anchors().iter().all(super::Anchor::is_finite),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(time_ms: i64, price: f64) -> Anchor {
        Anchor { time_ms, price }
    }

    #[derive(Debug, Deserialize, PartialEq, Serialize)]
    struct Holder {
        #[serde(default, with = "lenient")]
        drawings: Vec<Drawing>,
    }

    #[test]
    fn rows_written_before_styles_existed_decode_unchanged() {
        // The exact JSON the previous version wrote for each original drawing.
        let old = r#"[
            {"Brush":{"id":3,"points":[{"time_ms":1,"price":1.0}],"width":2.0,"locked":false}}
        ]"#;
        let drawings: Vec<Drawing> = serde_json::from_str(old).unwrap();
        assert_eq!(drawings.len(), 1);
        assert!(drawings[0].visibility().is_all());
        assert!(!drawings[0].locked());
        let Drawing::Brush { style, width, .. } = &drawings[0] else {
            panic!("brush");
        };
        assert_eq!(*width, 2.0);
        assert_eq!(
            *style,
            Tool::Brush.default_style(),
            "the old orange survives"
        );
    }

    #[test]
    fn a_shape_round_trips_through_json() {
        let drawing = Drawing::build(
            Tool::Curve,
            9,
            &[anchor(0, 1.0), anchor(100, 2.0), anchor(50, 3.0)],
            Tool::Curve.default_style(),
        )
        .unwrap();
        let json = serde_json::to_string(&drawing).unwrap();
        let back: Drawing = serde_json::from_str(&json).unwrap();
        assert_eq!(back, drawing);
        assert_eq!(back.tool(), Some(Tool::Curve));
    }

    #[test]
    fn a_shape_row_missing_its_optional_fields_still_decodes() {
        let json = r#"{"Shape":{"id":4,"geometry":{"rectangle":{"a":{"time_ms":0,"price":1.0},"b":{"time_ms":5,"price":2.0}}}}}"#;
        let drawing: Drawing = serde_json::from_str(json).unwrap();
        assert_eq!(drawing.tool(), Some(Tool::Rectangle));
        assert!(!drawing.locked() && !drawing.hidden());
        assert_eq!(drawing.text(), "");
    }

    /// The JSON of a real trend line this build wrote, with `extra` added to its body.
    fn stored_trend_line(id: u64, extra: serde_json::Value) -> serde_json::Value {
        let drawing = Drawing::build(
            Tool::TrendLine,
            id,
            &[anchor(0, 1.0), anchor(60_000, 2.0)],
            Tool::TrendLine.default_style(),
        )
        .expect("a trend line");
        let mut value = serde_json::to_value(drawing).unwrap();
        let body = value
            .as_object_mut()
            .and_then(|variants| variants.values_mut().next())
            .and_then(serde_json::Value::as_object_mut)
            .expect("a body");
        for (key, field) in extra.as_object().unwrap() {
            body.insert(key.clone(), field.clone());
        }
        value
    }

    #[test]
    fn a_drawing_written_for_a_newer_format_is_kept_whole_even_when_this_build_could_decode_it() {
        // A trend line this build reads fine, plus a field it does not know and the marker a newer
        // build puts on a drawing that uses it: decoding would silently drop the field.
        let newer = stored_trend_line(
            7,
            serde_json::json!({"since": DRAWING_FORMAT + 1, "shadow": {"blur": 4}}),
        );
        let older = stored_trend_line(8, serde_json::json!({"shadow": {"blur": 4}}));
        let stored = serde_json::json!({"drawings": [newer.clone(), older]});
        let holder: Holder = serde_json::from_value(stored).unwrap();
        assert!(
            matches!(&holder.drawings[0], Drawing::Unreadable(raw) if *raw == newer),
            "kept as it was: {:?}",
            holder.drawings[0]
        );
        // Not drawn, selected or edited, but it keeps its id for merging.
        assert_eq!(holder.drawings[0].stored_id(), 7);
        assert!(holder.drawings[0].tool().is_none());
        // Without the marker the same drawing is an ordinary one: the gate is the marker alone.
        assert!(matches!(holder.drawings[1], Drawing::Shape { id: 8, .. }));
        // And it is written back byte for byte.
        let again = serde_json::to_value(&holder).unwrap();
        assert_eq!(again["drawings"][0], newer);
    }

    #[test]
    fn a_since_this_build_reads_or_does_not_understand_does_not_hide_a_drawing() {
        for since in [
            serde_json::json!(0),
            serde_json::json!(DRAWING_FORMAT),
            serde_json::json!("two"),
            serde_json::json!(-3),
            serde_json::json!(null),
        ] {
            let stored = serde_json::json!({
                "drawings": [stored_trend_line(1, serde_json::json!({ "since": since }))]
            });
            let holder: Holder = serde_json::from_value(stored).unwrap();
            assert!(
                matches!(holder.drawings[0], Drawing::Shape { id: 1, .. }),
                "{since}"
            );
        }
    }

    #[test]
    fn an_unreadable_row_is_kept_and_written_back_verbatim() {
        let stored = r#"{"drawings":[
            {"Shape":{"id":1,"geometry":{"hologram":{"x":1}}}},
            {"Fib":{"id":2,"a":{"time_ms":0,"price":1.0},"b":{"time_ms":1,"price":2.0},"locked":false}},
            "not even an object"
        ]}"#;
        let holder: Holder = serde_json::from_str(stored).unwrap();
        assert_eq!(holder.drawings.len(), 3);
        assert!(matches!(holder.drawings[0], Drawing::Unreadable(_)));
        // The retracement of an earlier build was a variant of its own, which this version lacks.
        assert!(matches!(holder.drawings[1], Drawing::Unreadable(_)));
        assert_eq!(holder.drawings[1].stored_id(), 2);
        assert!(matches!(holder.drawings[2], Drawing::Unreadable(_)));
        let again = serde_json::to_value(&holder).unwrap();
        let original: serde_json::Value = serde_json::from_str(stored).unwrap();
        // The unreadable rows come back exactly.
        for index in 0..3 {
            assert_eq!(again["drawings"][index], original["drawings"][index]);
        }
        let reread: Holder = serde_json::from_value(again).unwrap();
        assert_eq!(reread, holder);
    }

    #[test]
    fn a_trend_line_is_one_shape_with_a_geometry_like_every_other_line() {
        let line = Drawing::build(
            Tool::TrendLine,
            1,
            &[anchor(10, 1.5), anchor(20, 2.5)],
            Tool::TrendLine.default_style(),
        )
        .unwrap();
        assert!(matches!(line.geometry(), Some(Geometry::TrendLine { .. })));
        assert_eq!(line.tool(), Some(Tool::TrendLine));
        assert_eq!(line.style(), Some(&DrawingStyle::trend_line()));
        let written = serde_json::to_value(&line).unwrap();
        assert!(written["Shape"]["geometry"].get("trend_line").is_some());
        // Text sits in the same drawing, set and cleared like any other, and the line is the
        // same line throughout.
        let mut texted = line.clone();
        texted.set_text("support");
        assert_eq!(texted.text(), "support");
        assert_eq!(texted.anchors(), line.anchors());
        assert_eq!(texted.tool(), Some(Tool::TrendLine));
        texted.set_text("");
        assert_eq!(texted, line);
    }

    #[test]
    fn a_styled_trend_line_keeps_its_style_through_json_and_is_sanitized() {
        use crate::model::chart_type::LineDash;
        let mut style = DrawingStyle::trend_line();
        style.stroke.color = [255, 193, 7, 255];
        style.stroke.dash = LineDash::Dashed;
        style.extend_right = true;
        let line = Drawing::build(
            Tool::TrendLine,
            4,
            &[anchor(0, 1.0), anchor(60_000, 2.0)],
            style,
        )
        .unwrap();
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains("\"style\""), "{json}");
        let back: Drawing = serde_json::from_str(&json).unwrap();
        assert_eq!(back, line);
        assert_eq!(back.style().unwrap().stroke.color, [255, 193, 7, 255]);
        assert!(back.style().unwrap().extend_right);
        assert!(!back.style().unwrap().extend_left);

        let mut restyled = line.clone();
        let mut wild = style;
        wild.stroke.width = f32::NAN;
        restyled.set_style(wild);
        assert!(restyled.style().unwrap().stroke.width.is_finite());
        // Moving the anchors and the lock still work on a styled line.
        restyled.set_locked(true);
        assert!(restyled.locked());
    }

    #[test]
    fn a_missing_list_decodes_as_empty_and_a_degenerate_shape_is_unreadable() {
        let holder: Holder = serde_json::from_str("{}").unwrap();
        assert!(holder.drawings.is_empty());
        let empty_path = r#"{"drawings":[{"Shape":{"id":1,"geometry":{"path":{"points":[]}}}}]}"#;
        let holder: Holder = serde_json::from_str(empty_path).unwrap();
        assert!(matches!(holder.drawings[0], Drawing::Unreadable(_)));
    }

    #[test]
    fn building_checks_the_anchor_count_and_extent() {
        let a = anchor(0, 1.0);
        let b = anchor(10, 2.0);
        let style = DrawingStyle::default();
        assert!(Drawing::build(Tool::Rectangle, 1, &[a, b], style).is_some());
        assert!(Drawing::build(Tool::Rectangle, 1, &[a], style).is_none());
        assert!(Drawing::build(Tool::Rectangle, 1, &[a, a], style).is_none());
        assert!(Drawing::build(Tool::Triangle, 1, &[a, b, b], style).is_some());
        assert!(Drawing::build(Tool::Triangle, 1, &[a, b], style).is_none());
        assert!(Drawing::build(Tool::DoubleCurve, 1, &[a, b, a, b], style).is_some());
        assert!(Drawing::build(Tool::ArrowMarkUp, 1, &[a], style).is_some());
        assert!(Drawing::build(Tool::ArrowMarkUp, 1, &[a, b], style).is_none());
        assert!(Drawing::build(Tool::Path, 1, &[a], style).is_none());
        assert!(Drawing::build(Tool::Polyline, 1, &[a, b], style).is_some());
        assert!(Drawing::build(Tool::Cursor, 1, &[a, b], style).is_none());
        assert!(Drawing::build(Tool::Brush, 1, &[], style).is_none());
        assert!(Drawing::build(Tool::Brush, 1, &[a], style).is_some());
        let nan = anchor(0, f64::NAN);
        assert!(Drawing::build(Tool::Rectangle, 1, &[a, nan], style).is_none());
        let too_many = vec![a; MAX_POINTS + 1];
        assert!(Drawing::build(Tool::Path, 1, &too_many, style).is_none());
    }

    #[test]
    fn every_grouped_tool_builds_and_reports_itself() {
        let anchors: Vec<Anchor> = (0..7).map(|i| anchor(i * 10, 1.0 + i as f64)).collect();
        for tool in Tool::GROUPED {
            let count = match tool.spec().placement {
                super::super::drawing_tool::Placement::Points(count) => count as usize,
                super::super::drawing_tool::Placement::Single => 1,
                _ => 3,
            };
            let drawing = Drawing::sample(tool, 5, &anchors[..count], tool.default_style())
                .unwrap_or_else(|| panic!("{tool:?} did not build"));
            assert_eq!(drawing.tool(), Some(tool));
            // A disjoint channel is placed with three clicks and keeps a fourth anchor, the end
            // of its second edge.
            let kept = match tool {
                Tool::DisjointChannel => 4,
                // A position keeps its entry, target and stop; a bars pattern the one anchor it is
                // set down at.
                Tool::LongPosition | Tool::ShortPosition => 3,
                Tool::BarsPattern => 1,
                _ => count,
            };
            assert_eq!(drawing.anchors().len(), kept.max(1), "{tool:?}");
            assert_eq!(drawing.id(), 5);
        }
    }

    fn rich_style() -> DrawingStyle {
        let mut style = DrawingStyle::default();
        style.pitchfork_levels[3].line.enabled = true;
        style.pitchfork_levels[3].ratio = 0.7;
        style.channel_levels[1].line.enabled = true;
        style.regression.upper_dev = 1.5;
        style.regression.source = crate::model::chart_type::PriceSource::Hl2;
        style.use_one_color = true;
        style.time_label = true;
        style.hide_text = true;
        style.text.orientation = crate::model::drawing_style::TextOrientation::Vertical;
        style.border.enabled = true;
        style.border.color = [10, 20, 30, 200];
        style.wrap = true;
        style.box_width = 180.0;
        style.pattern.labels = false;
        style.pattern.ratios = false;
        style.pattern.precision = 5;
        style.pattern.degree = crate::model::elliott::Degree::Minute;
        style.position.account = 25_000.0;
        style.position.risk_percent = false;
        style.position.risk = 125.5;
        style.position.compact = true;
        style.ghost.average = 1.25;
        style.ghost.draw_wick = false;
        style.vwap.source = crate::model::chart_type::PriceSource::Ohlc4;
        style.vwap.bands[2].enabled = true;
        style.profile.rows = 48;
        style.profile.volume = crate::model::forecast_style::ProfileVolume::Delta;
        style.profile.right = true;
        style.measure.volume = false;
        style
    }

    #[test]
    fn every_new_geometry_round_trips_with_a_customised_style() {
        let anchors: Vec<Anchor> = (0..7)
            .map(|i| anchor(i * 60_000, 100.0 + i as f64 * 7.5))
            .collect();
        for tool in Tool::GROUPED {
            let count = match tool.spec().placement {
                super::super::drawing_tool::Placement::Points(count) => count as usize,
                super::super::drawing_tool::Placement::Single => 1,
                _ => continue,
            };
            let mut drawing = Drawing::sample(tool, 3, &anchors[..count], rich_style()).unwrap();
            drawing.set_text("on the line");
            let json = serde_json::to_string(&drawing).unwrap();
            let back: Drawing = serde_json::from_str(&json).unwrap();
            assert_eq!(back, drawing, "{tool:?}: {json}");
            assert_eq!(back.tool(), Some(tool));
            assert_eq!(back.text(), drawing.text());
        }
    }

    #[test]
    fn a_style_that_uses_none_of_the_new_features_is_written_without_them() {
        let style = serde_json::to_value(DrawingStyle::default()).unwrap();
        let keys: Vec<&str> = style
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for new in [
            "pitchfork_levels",
            "channel_levels",
            "regression",
            "use_one_color",
            "time_label",
            "hide_text",
            "border",
            "wrap",
            "box_width",
            "pattern",
            "position",
            "ghost",
            "vwap",
            "profile",
            "measure",
        ] {
            assert!(!keys.contains(&new), "{new} would rewrite stored drawings");
        }
        assert!(
            style["text"].get("orientation").is_none(),
            "the default text orientation is not stored"
        );
        // Used, they are written, and a row without them reads back as the default.
        let used = serde_json::to_value(rich_style()).unwrap();
        assert!(used.get("pitchfork_levels").is_some() && used.get("time_label").is_some());
        let partial: DrawingStyle =
            serde_json::from_str(r#"{"stroke":{"width":3.0},"pitchfork_levels":[]}"#)
                .unwrap_or_default();
        assert_eq!(
            partial,
            DrawingStyle::default(),
            "a wrong-sized array is no style"
        );
        let bare: DrawingStyle = serde_json::from_str(r#"{"stroke":{"width":3.0}}"#).unwrap();
        assert_eq!(
            bare.pitchfork_levels,
            DrawingStyle::default().pitchfork_levels
        );
        assert_eq!(bare.regression, DrawingStyle::default().regression);
    }

    #[test]
    fn every_forecast_volume_and_measurement_tool_says_which_it_is_and_round_trips() {
        let anchors: Vec<Anchor> = (0..7)
            .map(|i| anchor(i * 60_000, 100.0 + (i % 3) as f64 * 7.5 + i as f64))
            .collect();
        for tool in Tool::forecast_tools() {
            let count = match tool.spec().placement {
                super::super::drawing_tool::Placement::Points(count) => count as usize,
                super::super::drawing_tool::Placement::Single => 1,
                other => panic!("{tool:?} is placed by {other:?}"),
            };
            let drawing = Drawing::sample(tool, 7, &anchors[..count], rich_style())
                .unwrap_or_else(|| panic!("{tool:?}"));
            assert_eq!(drawing.tool(), Some(tool));
            let written = serde_json::to_value(&drawing).unwrap();
            let back: Drawing = serde_json::from_value(written.clone()).unwrap();
            assert_eq!(back, drawing, "{tool:?}: {written}");
            // A default style leaves every new option out of the row.
            let plain = Drawing::sample(tool, 7, &anchors[..count], tool.default_style()).unwrap();
            let plain = serde_json::to_value(&plain).unwrap();
            for key in ["position", "ghost", "vwap", "profile", "measure", "pattern"] {
                assert!(
                    plain["Shape"]["style"].get(key).is_none(),
                    "{tool:?} writes {key}"
                );
            }
        }
    }

    #[test]
    fn a_position_is_a_box_whose_target_and_stop_share_its_right_edge() {
        use super::super::position::Side;
        let entry = anchor(1_000, 100.0);
        let long = Geometry::from_anchors(
            Tool::LongPosition,
            &[entry, anchor(9_000, 110.0), anchor(4_000, 95.0)],
        )
        .unwrap();
        // The stop is under the target whatever time the third click was at.
        assert_eq!(
            long.anchors(),
            [entry, anchor(9_000, 110.0), anchor(9_000, 95.0)]
        );
        assert_eq!(long.tool(), Tool::LongPosition);
        let short = Geometry::from_anchors(
            Tool::ShortPosition,
            &[entry, anchor(9_000, 90.0), anchor(9_000, 105.0)],
        )
        .unwrap();
        assert!(matches!(
            short,
            Geometry::Position {
                side: Side::Short,
                ..
            }
        ));
        assert_eq!(short.tool(), Tool::ShortPosition);
        // Dragging the stop sideways changes nothing about when it is: only its price is its own.
        let mut dragged = long.clone();
        dragged.anchors_mut()[2].time_ms = 1;
        dragged.anchors_mut()[2].price = 92.0;
        dragged.constrain(&long);
        assert_eq!(dragged.anchors()[2], anchor(9_000, 92.0));
        assert_eq!(dragged.reads(2), (false, true));
        assert_eq!(dragged.reads(1), (true, true));
        // Dragging the target sideways takes the stop with it.
        let mut wider = long.clone();
        wider.anchors_mut()[1].time_ms = 20_000;
        wider.constrain(&long);
        assert_eq!(wider.anchors()[2].time_ms, 20_000);
        // Every anchor must be a number.
        assert!(
            Geometry::from_anchors(
                Tool::LongPosition,
                &[entry, anchor(9_000, f64::NAN), anchor(9_000, 95.0)]
            )
            .is_none()
        );
    }

    #[test]
    fn a_position_from_its_entry_alone_has_a_target_a_stop_and_a_width() {
        use super::super::position::Side;
        let entry = anchor(0, 100.0);
        for (tool, side) in [
            (Tool::LongPosition, Side::Long),
            (Tool::ShortPosition, Side::Short),
        ] {
            let geometry = Geometry::from_anchors(tool, &[entry]).unwrap();
            let [entry, target, stop] = geometry.anchors()[..] else {
                panic!("three anchors");
            };
            // The profit is on the winning side and the stop on the losing one, with twice the
            // reward as risk.
            assert!(side.sign() * (target.price - entry.price) > 0.0, "{tool:?}");
            assert!(side.sign() * (entry.price - stop.price) > 0.0, "{tool:?}");
            let reward = (target.price - entry.price).abs();
            let risk = (entry.price - stop.price).abs();
            assert!((reward / risk - 2.0).abs() < 1e-9);
            assert!(target.time_ms > entry.time_ms);
            assert_eq!(target.time_ms, stop.time_ms);
        }
        // Even a price of nothing makes a position that can be drawn.
        let free = Geometry::from_anchors(Tool::LongPosition, &[anchor(0, 0.0)]).unwrap();
        assert!(free.is_valid());
    }

    #[test]
    fn the_range_tools_read_what_they_measure() {
        let (a, b) = (anchor(0, 1.0), anchor(60_000, 2.0));
        let price = Geometry::from_anchors(Tool::PriceRange, &[a, b]).unwrap();
        let date = Geometry::from_anchors(Tool::DateRange, &[a, b]).unwrap();
        let both = Geometry::from_anchors(Tool::DateAndPriceRange, &[a, b]).unwrap();
        assert_eq!(price.tool(), Tool::PriceRange);
        assert_eq!(date.tool(), Tool::DateRange);
        assert_eq!(both.tool(), Tool::DateAndPriceRange);
        // A date range has no price to edit.
        assert_eq!(
            (date.reads(0), date.reads(1)),
            ((true, false), (true, false))
        );
        assert_eq!(price.reads(0), (true, true));
        assert_eq!(both.reads(1), (true, true));
        // Two anchors on one point measure nothing.
        assert!(Geometry::from_anchors(Tool::PriceRange, &[a, a]).is_none());
    }

    #[test]
    fn the_volume_tools_read_the_bar_and_need_a_range_where_they_have_one() {
        let (a, b) = (anchor(0, 1.0), anchor(60_000, 2.0));
        let vwap = Geometry::from_anchors(Tool::AnchoredVwap, &[a]).unwrap();
        let anchored = Geometry::from_anchors(Tool::AnchoredVolumeProfile, &[a]).unwrap();
        let fixed = Geometry::from_anchors(Tool::FixedRangeVolumeProfile, &[a, b]).unwrap();
        for geometry in [&vwap, &anchored, &fixed] {
            assert_eq!(geometry.reads(0), (true, false));
        }
        assert_eq!(fixed.reads(1), (true, false));
        // A range of one moment is no range, however far apart its prices are.
        assert!(
            Geometry::from_anchors(Tool::FixedRangeVolumeProfile, &[a, anchor(0, 9.0)]).is_none()
        );
        assert!(Geometry::from_anchors(Tool::AnchoredVwap, &[a, b]).is_none());
    }

    #[test]
    fn a_ghost_feed_always_starts_with_the_same_seed_for_the_same_start() {
        let (a, b) = (anchor(5, 1.0), anchor(60_000, 2.0));
        let first = Geometry::from_anchors(Tool::GhostFeed, &[a, b]).unwrap();
        let again = Geometry::from_anchors(Tool::GhostFeed, &[a, anchor(120_000, 3.0)]).unwrap();
        let other = Geometry::from_anchors(Tool::GhostFeed, &[anchor(6, 1.0), b]).unwrap();
        let seed = |geometry: &Geometry| match geometry {
            Geometry::Ghost { seed, .. } => *seed,
            _ => panic!("a ghost feed"),
        };
        assert_eq!(
            seed(&first),
            seed(&again),
            "the end does not change the run's seed"
        );
        assert_ne!(seed(&first), seed(&other));
        assert_eq!(Geometry::ghost_seed(a), seed(&first));
        // A preview and the finished drawing agree.
        assert_eq!(Geometry::preview(Tool::GhostFeed, &[a, b]), Some(first));
    }

    #[test]
    fn a_sector_needs_a_radius_and_a_bars_pattern_needs_the_chart() {
        let (c, r, t) = (anchor(0, 1.0), anchor(60_000, 1.0), anchor(60_000, 3.0));
        assert!(Geometry::from_anchors(Tool::Sector, &[c, r, t]).is_some());
        assert!(Geometry::from_anchors(Tool::Sector, &[c, c, t]).is_none());
        // Copying bars needs bars: three clicks alone cannot make one, nor preview one.
        assert!(Geometry::from_anchors(Tool::BarsPattern, &[c, r, t]).is_none());
        assert!(Geometry::preview(Tool::BarsPattern, &[c, r]).is_none());
        // The one a person made (the sample a test uses) is valid and has its one anchor.
        let sample = Drawing::sample(Tool::BarsPattern, 1, &[c, r, t], DrawingStyle::default())
            .expect("a pattern");
        assert_eq!(sample.anchors(), [t]);
        assert_eq!(sample.tool(), Some(Tool::BarsPattern));
    }

    #[test]
    fn a_bars_pattern_with_no_bars_or_a_broken_number_is_unreadable() {
        let good = r#"{"Shape":{"id":1,"geometry":{"bars":{"at":{"time_ms":1,"price":2.0},"from_ms":0,"to_ms":1,"bars":[[0.0,1.0,-1.0,0.5]]}}}}"#;
        let empty = r#"{"Shape":{"id":2,"geometry":{"bars":{"at":{"time_ms":1,"price":2.0},"from_ms":0,"to_ms":1,"bars":[]}}}}"#;
        let stored = format!(r#"{{"drawings":[{good},{empty}]}}"#);
        let holder: Holder = serde_json::from_str(&stored).unwrap();
        assert_eq!(holder.drawings[0].tool(), Some(Tool::BarsPattern));
        assert!(matches!(holder.drawings[1], Drawing::Unreadable(_)));
    }

    #[test]
    fn every_pattern_wave_and_cycle_is_one_shape_that_says_which_and_round_trips() {
        let anchors: Vec<Anchor> = (0..7)
            .map(|i| anchor(i * 60_000, 100.0 + (i % 3) as f64 * 7.5 + i as f64))
            .collect();
        for tool in Tool::pattern_tools() {
            let count = match tool.spec().placement {
                super::super::drawing_tool::Placement::Points(count) => count as usize,
                other => panic!("{tool:?} is placed by {other:?}"),
            };
            let drawing = Drawing::build(tool, 7, &anchors[..count], tool.default_style())
                .unwrap_or_else(|| panic!("{tool:?}"));
            assert_eq!(drawing.tool(), Some(tool));
            assert_eq!(drawing.anchors(), anchors[..count]);
            let written = serde_json::to_value(&drawing).unwrap();
            let geometry = &written["Shape"]["geometry"];
            let key = geometry
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap()
                .as_str();
            assert!(
                matches!(key, "pattern" | "wave" | "cycle"),
                "{tool:?} is stored as {key}"
            );
            // The default options are not written into every drawing.
            assert!(
                written["Shape"]["style"].get("pattern").is_none(),
                "{tool:?}"
            );
            let back: Drawing = serde_json::from_value(written).unwrap();
            assert_eq!(back, drawing);
        }
    }

    #[test]
    fn a_pattern_with_the_wrong_number_of_pivots_is_unreadable_not_drawn() {
        // A stored pattern whose pivots do not match its kind is a damaged row: kept, never drawn.
        let stored = r#"{"drawings":[
            {"Shape":{"id":1,"geometry":{"pattern":{"kind":"xabcd","points":[
                {"time_ms":0,"price":1.0},{"time_ms":1,"price":2.0}]}}}},
            {"Shape":{"id":2,"geometry":{"wave":{"kind":"impulse","points":[
                {"time_ms":0,"price":1.0}]}}}},
            {"Shape":{"id":3,"geometry":{"cycle":{"kind":"sine_line",
                "a":{"time_ms":0,"price":1.0},"b":{"time_ms":9,"price":2.0}}}}}
        ]}"#;
        let holder: Holder = serde_json::from_str(stored).unwrap();
        assert!(matches!(holder.drawings[0], Drawing::Unreadable(_)));
        assert!(matches!(holder.drawings[1], Drawing::Unreadable(_)));
        assert_eq!(holder.drawings[2].tool(), Some(Tool::SineLine));
        // And a pattern kind this version does not know stays as it was.
        let future = r#"{"drawings":[{"Shape":{"id":4,"geometry":{"pattern":{"kind":"wolfe_wave","points":[]}}}}]}"#;
        let holder: Holder = serde_json::from_str(future).unwrap();
        assert!(matches!(holder.drawings[0], Drawing::Unreadable(_)));
    }

    #[test]
    fn building_a_pattern_checks_the_pivot_count_and_that_they_are_not_all_one_point() {
        let a = anchor(0, 1.0);
        let style = DrawingStyle::default();
        let spread: Vec<Anchor> = (0..9).map(|i| anchor(i * 10, i as f64)).collect();
        for tool in Tool::pattern_tools()
            .filter(|tool| tool.spec().group != super::super::ToolGroup::Cycles)
        {
            let count = match tool.spec().placement {
                super::super::drawing_tool::Placement::Points(count) => count as usize,
                _ => unreachable!(),
            };
            assert!(
                Drawing::build(tool, 1, &spread[..count], style).is_some(),
                "{tool:?}"
            );
            assert!(
                Drawing::build(tool, 1, &spread[..count - 1], style).is_none(),
                "{tool:?}"
            );
            assert!(
                Drawing::build(tool, 1, &spread[..count + 1], style).is_none(),
                "{tool:?}"
            );
            assert!(
                Drawing::build(tool, 1, &vec![a; count], style).is_none(),
                "{tool:?}"
            );
        }
        // A cycle's two anchors must be at different times.
        assert!(Drawing::build(Tool::CyclicLines, 1, &[a, anchor(0, 9.0)], style).is_none());
        assert!(Drawing::build(Tool::CyclicLines, 1, &[a, anchor(1, 1.0)], style).is_some());
    }

    #[test]
    fn degenerate_placements_make_no_new_shape() {
        let (a, b, c) = (anchor(0, 1.0), anchor(60_000, 2.0), anchor(30_000, 3.0));
        let style = DrawingStyle::default();
        for tool in [
            Tool::Ray,
            Tool::InfoLine,
            Tool::ExtendedLine,
            Tool::TrendAngle,
            Tool::ParallelChannel,
            Tool::FlatChannel,
            Tool::DisjointChannel,
            Tool::Pitchfork,
            Tool::SchiffPitchfork,
            Tool::ModifiedSchiffPitchfork,
            Tool::InsidePitchfork,
        ] {
            let count = match tool.spec().placement {
                super::super::drawing_tool::Placement::Points(count) => count as usize,
                _ => unreachable!(),
            };
            let same = vec![a; count];
            assert!(Drawing::build(tool, 1, &same, style).is_none(), "{tool:?}");
            let anchors = [a, b, c];
            assert!(Drawing::build(tool, 1, &anchors[..count], style).is_some());
            assert!(Drawing::build(tool, 1, &anchors[..count - 1], style).is_none());
        }
        // A regression through one bar has nothing to fit.
        assert!(Drawing::build(Tool::RegressionTrend, 1, &[a, anchor(0, 9.0)], style).is_none());
        assert!(Drawing::build(Tool::RegressionTrend, 1, &[a, b], style).is_some());
        // A level or a cross needs just its point.
        for tool in [
            Tool::HorizontalLine,
            Tool::HorizontalRay,
            Tool::VerticalLine,
            Tool::Crossline,
        ] {
            assert!(Drawing::build(tool, 1, &[a], style).is_some(), "{tool:?}");
            assert!(
                Drawing::build(tool, 1, &[a, b], style).is_none(),
                "{tool:?}"
            );
        }
    }

    #[test]
    fn a_disjoint_channel_starts_as_long_as_its_first_edge() {
        let (a, b, c) = (anchor(0, 10.0), anchor(60_000, 14.0), anchor(90_000, 20.0));
        let Some(Geometry::DisjointChannel { d, .. }) =
            Geometry::from_anchors(Tool::DisjointChannel, &[a, b, c])
        else {
            panic!("disjoint");
        };
        assert_eq!(d.time_ms, 150_000, "ends one edge-length after it starts");
        // The preview of the same three clicks is the same shape.
        assert_eq!(
            Geometry::preview(Tool::DisjointChannel, &[a, b, c]),
            Geometry::from_anchors(Tool::DisjointChannel, &[a, b, c])
        );
    }

    #[test]
    fn pitchfork_tools_map_to_their_variants_and_back() {
        for variant in PitchforkVariant::ALL {
            assert_eq!(PitchforkVariant::of_tool(variant.tool()), Some(variant));
        }
        assert_eq!(PitchforkVariant::of_tool(Tool::Rectangle), None);
        let anchors = [anchor(0, 1.0), anchor(10, 5.0), anchor(20, 2.0)];
        for variant in PitchforkVariant::ALL {
            let drawing =
                Drawing::build(variant.tool(), 1, &anchors, DrawingStyle::default()).unwrap();
            assert_eq!(drawing.tool(), Some(variant.tool()));
            let Some(Geometry::Pitchfork { variant: made, .. }) = drawing.geometry() else {
                panic!("pitchfork");
            };
            assert_eq!(*made, variant);
        }
    }

    #[test]
    fn editing_keeps_each_tool_what_it_is() {
        let at = anchor(5_000, 10.0);
        let moved = anchor(9_000, 12.0);
        let mut horizontal = Geometry::HorizontalLine { at: moved };
        horizontal.constrain(&Geometry::HorizontalLine { at });
        assert_eq!(
            horizontal,
            Geometry::HorizontalLine {
                at: anchor(5_000, 12.0)
            },
            "a level line slides along its price only"
        );
        let mut vertical = Geometry::VerticalLine { at: moved };
        vertical.constrain(&Geometry::VerticalLine { at });
        assert_eq!(
            vertical,
            Geometry::VerticalLine {
                at: anchor(9_000, 10.0)
            }
        );
        // A horizontal ray's origin moves freely, and a crossline's point too.
        let mut ray = Geometry::HorizontalRay { at: moved };
        ray.constrain(&Geometry::HorizontalRay { at });
        assert_eq!(ray, Geometry::HorizontalRay { at: moved });
        // A flat channel keeps its level edge in the middle of its sloped one.
        let mut flat = Geometry::FlatChannel {
            a: anchor(0, 1.0),
            b: anchor(100, 3.0),
            c: anchor(7, 2.0),
        };
        flat.constrain(&flat.clone());
        assert_eq!(
            flat.anchors()[2],
            anchor(50, 2.0),
            "only the price of the level edge is its own"
        );
        // What each anchor reads.
        assert_eq!(horizontal.reads(0), (false, true));
        assert_eq!(vertical.reads(0), (true, false));
        assert_eq!(flat.reads(2), (false, true));
        assert_eq!(flat.reads(0), (true, true));
        let regression = Geometry::Regression {
            a: anchor(0, 1.0),
            b: anchor(9, 1.0),
        };
        assert_eq!(
            (regression.reads(0), regression.reads(1)),
            ((true, false), (true, false))
        );
    }

    #[test]
    fn a_preview_is_always_a_recognisable_shape() {
        let a = anchor(0, 1.0);
        let b = anchor(30, 4.0);
        // One anchor placed plus the pointer: every tool previews, even degenerate.
        for tool in Tool::GROUPED.into_iter().filter(|tool| {
            !matches!(
                tool,
                Tool::Brush | Tool::Image | Tool::Post | Tool::BarsPattern
            )
        }) {
            assert!(Geometry::preview(tool, &[a]).is_some(), "{tool:?}");
            assert!(Geometry::preview(tool, &[a, b]).is_some(), "{tool:?}");
        }
        assert!(Geometry::preview(Tool::Rectangle, &[]).is_none());
        assert!(Geometry::preview(Tool::Cursor, &[a, b]).is_none());
        // A curve is straight until its control is placed, then bends to the pointer.
        let Some(Geometry::Curve { control, .. }) = Geometry::preview(Tool::Curve, &[a, b]) else {
            panic!("curve");
        };
        assert_eq!(control, anchor(15, 2.5));
        let bent = anchor(15, 9.0);
        let Some(Geometry::Curve {
            a: ca,
            b: cb,
            control,
        }) = Geometry::preview(Tool::Curve, &[a, b, bent])
        else {
            panic!("curve");
        };
        assert_eq!((ca, cb, control), (a, b, bent));
        // A double curve places its controls one after the other.
        let Some(Geometry::DoubleCurve { c1, c2, .. }) =
            Geometry::preview(Tool::DoubleCurve, &[a, b, bent])
        else {
            panic!("double curve");
        };
        assert_eq!((c1, c2), (bent, bent));
        // The full set of anchors previews the same shape it commits.
        let c = anchor(10, 7.0);
        let d = anchor(20, 0.0);
        assert_eq!(
            Geometry::preview(Tool::DoubleCurve, &[a, b, c, d]),
            Geometry::from_anchors(Tool::DoubleCurve, &[a, b, c, d])
        );
        assert_eq!(
            Geometry::preview(Tool::Triangle, &[a, b, c]),
            Geometry::from_anchors(Tool::Triangle, &[a, b, c])
        );
        assert!(Geometry::preview(Tool::Rectangle, &[a, anchor(1, f64::NAN)]).is_none());
    }

    fn rect(id: u64) -> Drawing {
        Drawing::build(
            Tool::Rectangle,
            id,
            &[anchor(0, 1.0), anchor(10, 2.0)],
            DrawingStyle::default(),
        )
        .unwrap()
    }

    #[test]
    fn commands_apply_in_order_and_report_real_edits_only() {
        let mut drawings = vec![rect(1), rect(2), rect(3)];
        // A no-op batch changes nothing and is no edit.
        assert!(!apply_commands(
            &mut drawings,
            vec![
                DrawingCommand::Replace {
                    id: 2,
                    drawing: rect(2)
                },
                DrawingCommand::Replace {
                    id: 99,
                    drawing: rect(99)
                },
                DrawingCommand::Remove(99),
                DrawingCommand::Reorder { id: 99, step: 1 },
            ]
        ));
        assert_eq!(drawings.len(), 3);
        // A real batch reports one change, however many commands it holds.
        let mut moved = rect(2);
        moved.set_locked(true);
        assert!(apply_commands(
            &mut drawings,
            vec![
                DrawingCommand::Replace {
                    id: 2,
                    drawing: moved
                },
                DrawingCommand::Add(rect(4)),
                DrawingCommand::Remove(1),
            ]
        ));
        let ids: Vec<u64> = drawings.iter().map(Drawing::id).collect();
        assert_eq!(ids, vec![2, 3, 4]);
        assert!(drawings[0].locked());
    }

    #[test]
    fn reordering_moves_a_drawing_through_the_stack_and_stops_at_the_ends() {
        let mut drawings = vec![rect(1), rect(2), rect(3)];
        assert!(apply_commands(
            &mut drawings,
            vec![DrawingCommand::Reorder { id: 1, step: 1 }]
        ));
        assert_eq!(
            drawings.iter().map(Drawing::id).collect::<Vec<_>>(),
            vec![2, 1, 3]
        );
        assert!(apply_commands(
            &mut drawings,
            vec![DrawingCommand::Reorder { id: 1, step: 99 }]
        ));
        assert_eq!(
            drawings.iter().map(Drawing::id).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        assert!(!apply_commands(
            &mut drawings,
            vec![DrawingCommand::Reorder { id: 1, step: 1 }]
        ));
        assert!(apply_commands(
            &mut drawings,
            vec![DrawingCommand::Reorder { id: 1, step: -9 }]
        ));
        assert_eq!(
            drawings.iter().map(Drawing::id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn a_ruler_is_never_an_edit() {
        let ruler = Drawing::Ruler {
            id: 9,
            a: anchor(0, 1.0),
            b: anchor(5, 2.0),
        };
        let mut drawings = vec![rect(1)];
        assert!(!apply_commands(
            &mut drawings,
            vec![DrawingCommand::Add(ruler.clone())]
        ));
        assert_eq!(drawings.len(), 2);
        let mut longer = ruler;
        longer.anchors_mut()[1].price = 7.0;
        assert!(!apply_commands(
            &mut drawings,
            vec![DrawingCommand::Replace {
                id: 9,
                drawing: longer
            }]
        ));
        assert!(!apply_commands(
            &mut drawings,
            vec![DrawingCommand::Remove(9)]
        ));
        assert_eq!(drawings.len(), 1);
    }

    #[test]
    fn an_unreadable_row_still_reports_the_id_it_was_saved_with() {
        let future: Drawing = lenient_one(r#"{"Shape":{"id":41,"geometry":{"hologram":{}}}}"#);
        assert_eq!(future.id(), 0);
        assert_eq!(future.stored_id(), 41);
        assert_eq!(lenient_one(r#""garbage""#).stored_id(), 0);
        assert_eq!(rect(7).stored_id(), 7);
    }

    fn lenient_one(json: &str) -> Drawing {
        let holder: Holder = serde_json::from_str(&format!(r#"{{"drawings":[{json}]}}"#)).unwrap();
        holder.drawings.into_iter().next().unwrap()
    }

    #[test]
    fn anchors_can_be_replaced_one_by_one() {
        let mut geometry = Geometry::from_anchors(
            Tool::DoubleCurve,
            &[
                anchor(0, 1.0),
                anchor(10, 2.0),
                anchor(3, 5.0),
                anchor(7, 5.0),
            ],
        )
        .unwrap();
        *geometry.anchors_mut()[3] = anchor(8, 9.0);
        assert_eq!(geometry.anchors()[3], anchor(8, 9.0));
        assert_eq!(geometry.anchors().len(), 4);
        assert_eq!(geometry.anchors_mut().len(), 4);
    }

    #[test]
    fn visibility_is_a_predicate_and_hidden_hides() {
        let mut drawing = Drawing::build(
            Tool::Rectangle,
            1,
            &[anchor(0, 1.0), anchor(1, 2.0)],
            DrawingStyle::default(),
        )
        .unwrap();
        assert!(drawing.is_visible(Timeframe::Hour));
        drawing.set_visibility(Visibility::ALL.with(Timeframe::Hour, false));
        assert!(!drawing.is_visible(Timeframe::Hour));
        assert!(drawing.is_visible(Timeframe::Day));
        drawing.set_hidden(true);
        assert!(!drawing.is_visible(Timeframe::Day));
        assert!(!Drawing::Unreadable(serde_json::Value::Null).is_visible(Timeframe::Day));
    }

    #[test]
    fn text_is_capped_and_a_brush_width_follows_its_style() {
        let mut drawing = Drawing::build(
            Tool::Ellipse,
            1,
            &[anchor(0, 1.0), anchor(1, 2.0)],
            DrawingStyle::default(),
        )
        .unwrap();
        drawing.set_text(&"x".repeat(MAX_TEXT_CHARS + 50));
        assert_eq!(drawing.text().chars().count(), MAX_TEXT_CHARS);
        let mut brush = Drawing::build(
            Tool::Brush,
            2,
            &[anchor(0, 1.0)],
            Tool::Brush.default_style(),
        )
        .unwrap();
        let mut style = *brush.style().unwrap();
        style.stroke.width = 7.0;
        brush.set_style(style);
        assert!(matches!(brush, Drawing::Brush { width, .. } if width == 7.0));
    }
}
