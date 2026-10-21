//! Drawings on a chart: pinned to market coordinates (time and price), never to pixels, so a
//! drawing survives zoom, scroll, a resize and a timeframe switch.
//!
//! The four original drawings (trend line, Fibonacci, brush, ruler) keep their serialized shape;
//! every new tool is one [`Drawing::Shape`] whose typed [`Geometry`] owns its anchors. Styling,
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
}

impl Geometry {
    /// The tool that draws this geometry.
    pub fn tool(&self) -> Tool {
        match self {
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
            Self::ArrowMark { at, .. } => vec![*at],
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
            Self::ArrowMark { at, .. } => vec![at],
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
            Tool::Cursor | Tool::TrendLine | Tool::Fib | Tool::Brush | Tool::Ruler => return None,
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
            _ => return None,
        };
        Some(geometry)
    }
}

/// A drawing on a chart, keyed by a stable id.
///
/// The four original variants keep their serialized shape; their later fields (`hidden`,
/// `visibility`, the brush's `style`) default so an old row decodes unchanged.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum Drawing {
    TrendLine {
        id: u64,
        a: Anchor,
        b: Anchor,
        locked: bool,
        #[serde(default)]
        hidden: bool,
        #[serde(default)]
        visibility: Visibility,
        /// Colour, width, dash and extension. A row without one is the blue segment it always
        /// was, and a line still in that look is written without it, so nothing stored changes.
        #[serde(
            default = "DrawingStyle::trend_line",
            skip_serializing_if = "is_plain_trend_line"
        )]
        style: DrawingStyle,
    },
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
    /// Every tool added after the originals.
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

fn is_plain_trend_line(style: &DrawingStyle) -> bool {
    *style == DrawingStyle::trend_line()
}

impl Drawing {
    pub fn id(&self) -> u64 {
        match self {
            Self::TrendLine { id, .. }
            | Self::Fib { id, .. }
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
            Tool::TrendLine => Self::TrendLine {
                id,
                a: at(0)?,
                b: at(1)?,
                locked: false,
                hidden: false,
                visibility: Visibility::ALL,
                style,
            },
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
            Self::TrendLine { .. } => Some(Tool::TrendLine),
            Self::Fib { .. } => Some(Tool::Fib),
            Self::Brush { .. } => Some(Tool::Brush),
            Self::Ruler { .. } => Some(Tool::Ruler),
            Self::Shape { geometry, .. } => Some(geometry.tool()),
            Self::Unreadable(_) => None,
        }
    }

    pub fn locked(&self) -> bool {
        match self {
            Self::TrendLine { locked, .. }
            | Self::Fib { locked, .. }
            | Self::Brush { locked, .. }
            | Self::Shape { locked, .. } => *locked,
            Self::Ruler { .. } | Self::Unreadable(_) => false,
        }
    }

    pub fn hidden(&self) -> bool {
        match self {
            Self::TrendLine { hidden, .. }
            | Self::Fib { hidden, .. }
            | Self::Brush { hidden, .. }
            | Self::Shape { hidden, .. } => *hidden,
            Self::Ruler { .. } | Self::Unreadable(_) => false,
        }
    }

    pub fn visibility(&self) -> Visibility {
        match self {
            Self::TrendLine { visibility, .. }
            | Self::Fib { visibility, .. }
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
            Self::TrendLine { locked, .. }
            | Self::Fib { locked, .. }
            | Self::Brush { locked, .. }
            | Self::Shape { locked, .. } => *locked = value,
            Self::Ruler { .. } | Self::Unreadable(_) => {}
        }
    }

    pub fn set_hidden(&mut self, value: bool) {
        match self {
            Self::TrendLine { hidden, .. }
            | Self::Fib { hidden, .. }
            | Self::Brush { hidden, .. }
            | Self::Shape { hidden, .. } => *hidden = value,
            Self::Ruler { .. } | Self::Unreadable(_) => {}
        }
    }

    pub fn set_visibility(&mut self, value: Visibility) {
        match self {
            Self::TrendLine { visibility, .. }
            | Self::Fib { visibility, .. }
            | Self::Brush { visibility, .. }
            | Self::Shape { visibility, .. } => *visibility = value,
            Self::Ruler { .. } | Self::Unreadable(_) => {}
        }
    }

    /// The style, for the drawings that carry one (the trend line, the brush and every shape).
    pub fn style(&self) -> Option<&DrawingStyle> {
        match self {
            Self::TrendLine { style, .. }
            | Self::Brush { style, .. }
            | Self::Shape { style, .. } => Some(style),
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
            Self::TrendLine { style, .. } | Self::Shape { style, .. } => *style = value,
            _ => {}
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Shape { text, .. } => text,
            _ => "",
        }
    }

    pub fn set_text(&mut self, value: &str) {
        if let Self::Shape { text, .. } = self {
            *text = value.chars().take(MAX_TEXT_CHARS).collect();
        }
    }

    /// Every anchor in placement order.
    pub fn anchors(&self) -> Vec<Anchor> {
        match self {
            Self::TrendLine { a, b, .. } | Self::Fib { a, b, .. } | Self::Ruler { a, b, .. } => {
                vec![*a, *b]
            }
            Self::Brush { points, .. } => points.clone(),
            Self::Shape { geometry, .. } => geometry.anchors(),
            Self::Unreadable(_) => Vec::new(),
        }
    }

    /// Mutable access to every anchor in placement order, to move or edit them.
    pub fn anchors_mut(&mut self) -> Vec<&mut Anchor> {
        match self {
            Self::TrendLine { a, b, .. } | Self::Fib { a, b, .. } | Self::Ruler { a, b, .. } => {
                vec![a, b]
            }
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
            Self::TrendLine { id, .. }
            | Self::Fib { id, .. }
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
            {"TrendLine":{"id":1,"a":{"time_ms":10,"price":1.5},"b":{"time_ms":20,"price":2.5},"locked":false}},
            {"Fib":{"id":2,"a":{"time_ms":10,"price":1.5},"b":{"time_ms":20,"price":2.5},"locked":true}},
            {"Brush":{"id":3,"points":[{"time_ms":1,"price":1.0}],"width":2.0,"locked":false}}
        ]"#;
        let drawings: Vec<Drawing> = serde_json::from_str(old).unwrap();
        assert_eq!(drawings.len(), 3);
        assert!(matches!(
            drawings[0],
            Drawing::TrendLine {
                id: 1,
                hidden: false,
                ..
            }
        ));
        assert!(drawings[0].visibility().is_all());
        assert!(drawings[1].locked());
        let Drawing::Brush { style, width, .. } = &drawings[2] else {
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
            {"TrendLine":{"id":2,"a":{"time_ms":0,"price":1.0},"b":{"time_ms":1,"price":2.0},"locked":false}},
            "not even an object"
        ]}"#;
        let holder: Holder = serde_json::from_str(stored).unwrap();
        assert_eq!(holder.drawings.len(), 3);
        assert!(matches!(holder.drawings[0], Drawing::Unreadable(_)));
        assert!(matches!(
            holder.drawings[1],
            Drawing::TrendLine { id: 2, .. }
        ));
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
    fn a_trend_line_without_a_style_is_the_line_it_always_was() {
        use crate::model::chart_type::LineDash;
        let old = r#"{"TrendLine":{"id":1,"a":{"time_ms":10,"price":1.5},"b":{"time_ms":20,"price":2.5},"locked":false}}"#;
        let line: Drawing = serde_json::from_str(old).unwrap();
        assert_eq!(line.style(), Some(&DrawingStyle::trend_line()));
        assert_eq!(line.style().unwrap().stroke.dash, LineDash::Solid);
        assert!(!line.style().unwrap().extend_right);
        // Still in that look, it is written without a style: nothing stored changes.
        let written = serde_json::to_value(&line).unwrap();
        assert!(written["TrendLine"].get("style").is_none(), "{written}");
        // The GUI's default for the tool builds the same line.
        let built = Drawing::build(
            Tool::TrendLine,
            1,
            &[anchor(10, 1.5), anchor(20, 2.5)],
            Tool::TrendLine.default_style(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&built).unwrap()["TrendLine"]["a"],
            written["TrendLine"]["a"]
        );
        assert!(
            serde_json::to_value(&built).unwrap()["TrendLine"]
                .get("style")
                .is_none()
        );
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
        restyled.set_style(DrawingStyle::trend_line());
        assert!(
            serde_json::to_value(&restyled).unwrap()["TrendLine"]
                .get("style")
                .is_none()
        );
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
            assert_eq!(drawing.anchors().len(), count.max(1), "{tool:?}");
            assert_eq!(drawing.id(), 5);
        }
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
