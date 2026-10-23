//! Drawings on a chart: pinned to market coordinates (time and price), never to pixels, so a
//! drawing survives zoom, scroll, a resize and a timeframe switch.
//!
//! Fibonacci, the brush and the ruler have variants of their own;
//! every other tool is one [`Drawing::Shape`] whose typed [`Geometry`] owns its anchors. Styling,
//! text and per-timeframe visibility live beside the geometry, so moving, restyling and hiding a
//! drawing never touch each other.

use super::Timeframe;
use super::drawing_style::{DrawingStyle, Visibility};
use super::drawing_tool::Tool;
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

impl Geometry {
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
            | Self::Polyline { points, .. } => points.clone(),
            Self::ArrowMarker { a, b }
            | Self::Arrow { a, b }
            | Self::Rectangle { a, b }
            | Self::Ellipse { a, b } => vec![*a, *b],
            Self::Leader { at, label, .. }
            | Self::Signpost {
                anchor: at, label, ..
            } => vec![*at, *label],
            Self::ArrowMark { at, .. } | Self::Annotation { at, .. } => vec![*at],
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
            | Self::Polyline { points, .. } => points.iter_mut().collect(),
            Self::ArrowMarker { a, b }
            | Self::Arrow { a, b }
            | Self::Rectangle { a, b }
            | Self::Ellipse { a, b } => vec![a, b],
            Self::Leader { at, label, .. }
            | Self::Signpost {
                anchor: at, label, ..
            } => vec![at, label],
            Self::ArrowMark { at, .. } | Self::Annotation { at, .. } => vec![at],
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
            Tool::Cursor | Tool::Fib | Tool::Brush | Tool::Ruler => return None,
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
    Fib {
        id: u64,
        a: Anchor,
        b: Anchor,
        locked: bool,
        #[serde(default)]
        hidden: bool,
        #[serde(default)]
        visibility: Visibility,
    },
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
            Self::Fib { id, .. }
            | Self::Brush { id, .. }
            | Self::Ruler { id, .. }
            | Self::Shape { id, .. } => *id,
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

    /// A new drawing by `tool` through `anchors`, in `style`. `None` when the anchors do not
    /// make that shape; the caller commits nothing then.
    pub fn build(tool: Tool, id: u64, anchors: &[Anchor], style: DrawingStyle) -> Option<Self> {
        let style = style.sanitized();
        let at = |index: usize| anchors.get(index).copied();
        Some(match tool {
            Tool::Cursor => return None,
            Tool::Fib => Self::Fib {
                id,
                a: at(0)?,
                b: at(1)?,
                locked: false,
                hidden: false,
                visibility: Visibility::ALL,
            },
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
            Self::Fib { .. } => Some(Tool::Fib),
            Self::Brush { .. } => Some(Tool::Brush),
            Self::Ruler { .. } => Some(Tool::Ruler),
            Self::Shape { geometry, .. } => Some(geometry.tool()),
            Self::Unreadable(_) => None,
        }
    }

    pub fn locked(&self) -> bool {
        match self {
            Self::Fib { locked, .. } | Self::Brush { locked, .. } | Self::Shape { locked, .. } => {
                *locked
            }
            Self::Ruler { .. } | Self::Unreadable(_) => false,
        }
    }

    pub fn hidden(&self) -> bool {
        match self {
            Self::Fib { hidden, .. } | Self::Brush { hidden, .. } | Self::Shape { hidden, .. } => {
                *hidden
            }
            Self::Ruler { .. } | Self::Unreadable(_) => false,
        }
    }

    pub fn visibility(&self) -> Visibility {
        match self {
            Self::Fib { visibility, .. }
            | Self::Brush { visibility, .. }
            | Self::Shape { visibility, .. } => *visibility,
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
            Self::Fib { locked, .. } | Self::Brush { locked, .. } | Self::Shape { locked, .. } => {
                *locked = value
            }
            Self::Ruler { .. } | Self::Unreadable(_) => {}
        }
    }

    pub fn set_hidden(&mut self, value: bool) {
        match self {
            Self::Fib { hidden, .. } | Self::Brush { hidden, .. } | Self::Shape { hidden, .. } => {
                *hidden = value
            }
            Self::Ruler { .. } | Self::Unreadable(_) => {}
        }
    }

    pub fn set_visibility(&mut self, value: Visibility) {
        match self {
            Self::Fib { visibility, .. }
            | Self::Brush { visibility, .. }
            | Self::Shape { visibility, .. } => *visibility = value,
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

    /// Every anchor in placement order.
    pub fn anchors(&self) -> Vec<Anchor> {
        match self {
            Self::Fib { a, b, .. } | Self::Ruler { a, b, .. } => vec![*a, *b],
            Self::Brush { points, .. } => points.clone(),
            Self::Shape { geometry, .. } => geometry.anchors(),
            Self::Unreadable(_) => Vec::new(),
        }
    }

    /// Mutable access to every anchor in placement order, to move or edit them.
    pub fn anchors_mut(&mut self) -> Vec<&mut Anchor> {
        match self {
            Self::Fib { a, b, .. } | Self::Ruler { a, b, .. } => vec![a, b],
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
            Self::Fib { id, .. }
            | Self::Brush { id, .. }
            | Self::Ruler { id, .. }
            | Self::Shape { id, .. } => *id = new_id,
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
                drawings.retain(|d| d.id() != id);
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
            {"Fib":{"id":2,"a":{"time_ms":10,"price":1.5},"b":{"time_ms":20,"price":2.5},"locked":true}},
            {"Brush":{"id":3,"points":[{"time_ms":1,"price":1.0}],"width":2.0,"locked":false}}
        ]"#;
        let drawings: Vec<Drawing> = serde_json::from_str(old).unwrap();
        assert_eq!(drawings.len(), 2);
        assert!(drawings[0].visibility().is_all());
        assert!(drawings[0].locked());
        let Drawing::Brush { style, width, .. } = &drawings[1] else {
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
        assert!(matches!(holder.drawings[1], Drawing::Fib { id: 2, .. }));
        assert!(matches!(holder.drawings[2], Drawing::Unreadable(_)));
        let again = serde_json::to_value(&holder).unwrap();
        let original: serde_json::Value = serde_json::from_str(stored).unwrap();
        // The unreadable rows come back exactly; the readable one re-encodes with its defaults.
        assert_eq!(again["drawings"][0], original["drawings"][0]);
        assert_eq!(again["drawings"][2], original["drawings"][2]);
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
        let anchors: Vec<Anchor> = (0..4).map(|i| anchor(i * 10, 1.0 + i as f64)).collect();
        for tool in Tool::GROUPED {
            let count = match tool.spec().placement {
                super::super::drawing_tool::Placement::Points(count) => count as usize,
                super::super::drawing_tool::Placement::Single => 1,
                _ => 3,
            };
            let drawing = Drawing::build(tool, 5, &anchors[..count], tool.default_style())
                .unwrap_or_else(|| panic!("{tool:?} did not build"));
            assert_eq!(drawing.tool(), Some(tool));
            // A disjoint channel is placed with three clicks and keeps a fourth anchor, the end
            // of its second edge.
            let kept = if tool == Tool::DisjointChannel {
                4
            } else {
                count
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
        style
    }

    #[test]
    fn every_new_geometry_round_trips_with_a_customised_style() {
        let anchors: Vec<Anchor> = (0..4)
            .map(|i| anchor(i * 60_000, 100.0 + i as f64 * 7.5))
            .collect();
        for tool in Tool::GROUPED {
            let count = match tool.spec().placement {
                super::super::drawing_tool::Placement::Points(count) => count as usize,
                super::super::drawing_tool::Placement::Single => 1,
                _ => continue,
            };
            let mut drawing = Drawing::build(tool, 3, &anchors[..count], rich_style()).unwrap();
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
        for tool in Tool::GROUPED
            .into_iter()
            .filter(|tool| *tool != Tool::Brush)
        {
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
