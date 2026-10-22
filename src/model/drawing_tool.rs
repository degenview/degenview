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
use serde::{Deserialize, Serialize};

/// A drawing tool armed on the tool strip. `Cursor` is the crosshair (default).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Tool {
    #[default]
    Cursor,
    TrendLine,
    Fib,
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
}

/// Where a tool sits in the toolbar's menus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolGroup {
    /// The crosshair, which draws nothing.
    Pointer,
    /// Fibonacci: the strip's own tile.
    Lines,
    Measure,
    Brushes,
    Arrows,
    Shapes,
    /// The trend line and the other lines: the first menu of the Lines tile.
    TrendLines,
    Channels,
    Pitchforks,
}

impl ToolGroup {
    /// The groups listed in the brushes, arrows and shapes menu, in order.
    pub const MENU: [Self; 3] = [Self::Brushes, Self::Arrows, Self::Shapes];
    /// The groups listed in the Lines tile's menu, in order.
    pub const LINE_MENU: [Self; 3] = [Self::TrendLines, Self::Channels, Self::Pitchforks];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Pointer => "Cursor",
            Self::Lines | Self::TrendLines => "Lines",
            Self::Measure => "Measure",
            Self::Brushes => "Brushes",
            Self::Arrows => "Arrows",
            Self::Shapes => "Shapes",
            Self::Channels => "Channels",
            Self::Pitchforks => "Pitchforks",
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
pub struct Caps(u16);

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
    pub const GROUPED: [Self; 33] = [
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
    ];

    pub const fn spec(self) -> ToolSpec {
        use Placement::{Freehand, Open, Points, Single};
        use ToolGroup::{
            Arrows, Brushes, Channels, Lines, Measure, Pitchforks, Pointer, Shapes, TrendLines,
        };
        let shape_line = Caps::LINE.with(Caps::FILL);
        // What every two-point line offers: text along it, ends, statistics and price tags.
        let line = Caps::LINE
            .with(Caps::EXTEND)
            .with(Caps::ENDS)
            .with(Caps::MIDDLE_POINT)
            .with(Caps::PRICE_LABELS)
            .with(Caps::STATS)
            .with(Caps::TEXT);
        // A line at one price or one time.
        let level_line = Caps::LINE
            .with(Caps::TEXT)
            .with(Caps::PRICE_LABELS)
            .with(Caps::ORIENTATION);
        let channel = shape_line.with(Caps::EXTEND).with(Caps::TEXT);
        let fork = Caps::LINE
            .with(Caps::FILL)
            .with(Caps::EXTEND)
            .with(Caps::LEVELS);
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
            Self::Fib => spec("Fib Retracement", Lines, Points(2), Caps::NONE),
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
        }
    }

    pub const fn label(self) -> &'static str {
        self.spec().label
    }

    pub const fn caps(self) -> Caps {
        self.spec().caps
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
        // The Charting Library's line: its blue, 2 px, nothing extended, no tags.
        let line_style = |change: fn(&mut DrawingStyle)| {
            let mut style = tinted([41, 98, 255, 255], false);
            change(&mut style);
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
            Self::FlatChannel => tinted([255, 152, 0, 255], true),
            Self::DisjointChannel => tinted([8, 153, 129, 255], true),
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
            Self::Cursor | Self::Fib | Self::Ruler => DrawingStyle::default(),
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
            _ => None,
        }
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
        assert_eq!(Tool::GROUPED.len(), 33);
        for (index, tool) in Tool::GROUPED.iter().enumerate() {
            assert!(!Tool::GROUPED[..index].contains(tool), "{tool:?} repeats");
            let group = tool.spec().group;
            assert!(
                ToolGroup::MENU.contains(&group) != ToolGroup::LINE_MENU.contains(&group),
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
        assert_eq!(Tool::line_tools().count(), 17);
        // The Fibonacci tile and the crosshair are in neither menu.
        assert!(!Tool::GROUPED.contains(&Tool::Fib));
        assert_eq!(Tool::Fib.spec().group, ToolGroup::Lines);
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
