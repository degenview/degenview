//! How a drawing looks: stroke, fill, text, line ends, extension and arrow statistics. Pure, no
//! egui types, and kept apart from the geometry so a style can be saved as a template, copied to
//! another drawing or become a tool's default without carrying any coordinates.
//!
//! Colours are straight `[r, g, b, a]` bytes where the alpha is the opacity (as in the chart
//! canvas), and every struct decodes from an older or partial row through `#[serde(default)]`.

use super::Timeframe;
use super::chart_type::{LineDash, PriceSource, Rgba};
use serde::{Deserialize, Serialize};

/// The thickest stroke a drawing may carry, in logical pixels.
pub const MAX_STROKE_WIDTH: f32 = 60.0;
/// The range of a drawing's text size, in points.
pub const TEXT_SIZE_RANGE: (f32, f32) = (8.0, 72.0);

/// The blue the generic style (and the trend line) is drawn in.
pub const BLUE: Rgba = [41, 98, 255, 255];

/// The blue a trend line is drawn in.
pub const TREND_LINE_COLOR: Rgba = [10, 132, 255, 255];
/// The width a trend line is drawn at.
pub const TREND_LINE_WIDTH: f32 = 1.5;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct StrokeStyle {
    pub color: Rgba,
    pub width: f32,
    pub dash: LineDash,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            color: BLUE,
            width: 2.0,
            dash: LineDash::Solid,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct FillStyle {
    pub enabled: bool,
    pub color: Rgba,
}

impl Default for FillStyle {
    fn default() -> Self {
        Self {
            enabled: false,
            color: [BLUE[0], BLUE[1], BLUE[2], 51],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalAlign {
    Left,
    #[default]
    Center,
    Right,
}

impl HorizontalAlign {
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Center => "Center",
            Self::Right => "Right",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

impl VerticalAlign {
    pub const ALL: [Self; 3] = [Self::Top, Self::Middle, Self::Bottom];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Top => "Top",
            Self::Middle => "Middle",
            Self::Bottom => "Bottom",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct TextStyle {
    pub color: Rgba,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub align: HorizontalAlign,
    pub valign: VerticalAlign,
    /// How the text is turned on a line. Only lines read it, and it stays out of every stored
    /// row that keeps the default.
    #[serde(skip_serializing_if = "is_default_orientation")]
    pub orientation: TextOrientation,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            color: BLUE,
            size: 14.0,
            bold: false,
            italic: false,
            align: HorizontalAlign::Center,
            valign: VerticalAlign::Middle,
            orientation: TextOrientation::Along,
        }
    }
}

/// How the text on a line is turned.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOrientation {
    /// Turned to follow the line, never upside down.
    #[default]
    Along,
    Horizontal,
    /// Read from the bottom up.
    Vertical,
}

fn is_default_orientation(value: &TextOrientation) -> bool {
    *value == TextOrientation::Along
}

/// What one end of a line looks like.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnd {
    #[default]
    Plain,
    Arrow,
}

impl LineEnd {
    pub const ALL: [Self; 2] = [Self::Plain, Self::Arrow];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
            Self::Arrow => "Arrow",
        }
    }
}

/// A second line drawn through the middle of a shape (the rectangle's).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct MiddleLine {
    pub enabled: bool,
    pub color: Rgba,
    pub width: f32,
    pub dash: LineDash,
}

impl Default for MiddleLine {
    fn default() -> Self {
        Self {
            enabled: false,
            color: BLUE,
            width: 1.0,
            dash: LineDash::Dashed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsPosition {
    Left,
    #[default]
    Center,
    Right,
}

impl StatsPosition {
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Center => "Center",
            Self::Right => "Right",
        }
    }
}

/// Which measurements an arrow prints beside itself.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ArrowStats {
    pub price_range: bool,
    pub percent: bool,
    pub bars: bool,
    pub time: bool,
    pub distance: bool,
    pub angle: bool,
    pub position: StatsPosition,
    /// Show them on an unselected arrow too; off prints them only while it is selected.
    pub always_show: bool,
}

impl ArrowStats {
    pub fn any(&self) -> bool {
        self.price_range || self.percent || self.bars || self.time || self.distance || self.angle
    }
}

/// The widest a text box may be told to wrap at, in logical pixels.
pub const MAX_BOX_WIDTH: f32 = 2_000.0;
/// The narrowest, and the width a wrapping box has when its style does not say.
pub const MIN_BOX_WIDTH: f32 = 60.0;
pub const DEFAULT_BOX_WIDTH: f32 = 200.0;

/// The border of a text box (a note, a callout, a table). It is separate from the stroke, which a
/// note's connector and a pin's glyph use, because TradingView colours the two apart.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct BorderStyle {
    pub enabled: bool,
    pub color: Rgba,
    pub width: f32,
    pub dash: LineDash,
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self {
            enabled: false,
            color: BLUE,
            width: 1.0,
            dash: LineDash::Solid,
        }
    }
}

/// The pins a signpost can carry on its marker, the way TradingView offers an emoji there. The
/// app's text renderer cannot draw colour emoji (the colour fonts are bitmaps), so these are
/// monochrome symbols its fonts do carry, a test checks each, painted white on the marker's
/// disc. A fixed list, named rather than stored as characters, so a stored row says what it
/// means and cannot hold anything else.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Emoji {
    #[default]
    Star,
    Heart,
    Diamond,
    Spade,
    Club,
    Flag,
    Warning,
    Arrow,
    Up,
    Down,
    Dot,
    Check,
    Cross,
    Skull,
    Radiation,
    Scales,
    Crown,
    Bitcoin,
    Dollar,
    Euro,
    Swords,
    Infinity,
    Comet,
    Moon,
    Gear,
    Plane,
}

impl Emoji {
    pub const ALL: [Self; 26] = [
        Self::Star,
        Self::Heart,
        Self::Diamond,
        Self::Spade,
        Self::Club,
        Self::Flag,
        Self::Warning,
        Self::Arrow,
        Self::Up,
        Self::Down,
        Self::Dot,
        Self::Check,
        Self::Cross,
        Self::Skull,
        Self::Radiation,
        Self::Scales,
        Self::Crown,
        Self::Bitcoin,
        Self::Dollar,
        Self::Euro,
        Self::Swords,
        Self::Infinity,
        Self::Comet,
        Self::Moon,
        Self::Gear,
        Self::Plane,
    ];

    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Star => "\u{2605}",
            Self::Heart => "\u{2665}",
            Self::Diamond => "\u{2666}",
            Self::Spade => "\u{2660}",
            Self::Club => "\u{2663}",
            Self::Flag => "\u{2691}",
            Self::Warning => "\u{26A0}",
            Self::Arrow => "\u{27A4}",
            Self::Up => "\u{25B2}",
            Self::Down => "\u{25BC}",
            Self::Dot => "\u{25CF}",
            Self::Check => "\u{2713}",
            Self::Cross => "\u{2717}",
            Self::Skull => "\u{2620}",
            Self::Radiation => "\u{2622}",
            Self::Scales => "\u{2696}",
            Self::Crown => "\u{265B}",
            Self::Bitcoin => "\u{20BF}",
            Self::Dollar => "\u{0024}",
            Self::Euro => "\u{20AC}",
            Self::Swords => "\u{2694}",
            Self::Infinity => "\u{221E}",
            Self::Comet => "\u{2604}",
            Self::Moon => "\u{263E}",
            Self::Gear => "\u{2699}",
            Self::Plane => "\u{2708}",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Star => "Star",
            Self::Heart => "Heart",
            Self::Diamond => "Diamond",
            Self::Spade => "Spade",
            Self::Club => "Club",
            Self::Flag => "Flag",
            Self::Warning => "Warning",
            Self::Arrow => "Arrow",
            Self::Up => "Triangle up",
            Self::Down => "Triangle down",
            Self::Dot => "Dot",
            Self::Check => "Check",
            Self::Cross => "Cross",
            Self::Skull => "Skull",
            Self::Radiation => "Radiation",
            Self::Scales => "Scales",
            Self::Crown => "Crown",
            Self::Bitcoin => "Bitcoin",
            Self::Dollar => "Dollar",
            Self::Euro => "Euro",
            Self::Swords => "Swords",
            Self::Infinity => "Infinity",
            Self::Comet => "Comet",
            Self::Moon => "Moon",
            Self::Gear => "Gear",
            Self::Plane => "Plane",
        }
    }
}

/// How many extra lines a pitchfork can carry besides its median.
pub const PITCHFORK_LEVELS: usize = 9;
/// How many extra lines a parallel channel can carry besides its two edges and its middle line.
pub const CHANNEL_LEVELS: usize = 4;
/// The most a level may sit from the median or the base, as a ratio.
pub const MAX_LEVEL_RATIO: f32 = 100.0;
/// The most standard deviations a regression band may sit from its base line.
pub const MAX_DEVIATION: f32 = 50.0;

/// One extra line of a channel or a pitchfork: where it sits, as a ratio of the construction's
/// width, and how it is drawn. A pitchfork draws it on both sides of the median.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Level {
    pub ratio: f32,
    pub line: MiddleLine,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            ratio: 1.0,
            line: MiddleLine::default(),
        }
    }
}

fn level(ratio: f32, color: Rgba, enabled: bool) -> Level {
    Level {
        ratio,
        line: MiddleLine {
            enabled,
            color,
            width: 2.0,
            dash: LineDash::Solid,
        },
    }
}

/// The pitchfork's levels as the Charting Library draws a new one: 0.5 and 1 on, the rest
/// off until the user asks for them.
pub fn default_pitchfork_levels() -> [Level; PITCHFORK_LEVELS] {
    [
        level(0.25, [255, 183, 77, 255], false),
        level(0.382, [129, 199, 132, 255], false),
        level(0.5, [8, 153, 129, 255], true),
        level(0.618, [8, 153, 129, 255], false),
        level(0.75, [0, 188, 212, 255], false),
        level(1.0, [41, 98, 255, 255], true),
        level(1.5, [156, 39, 176, 255], false),
        level(1.75, [233, 30, 99, 255], false),
        level(2.0, [247, 124, 128, 255], false),
    ]
}

/// The parallel channel's extra lines: off until asked for (its middle line is `middle_line`).
pub fn default_channel_levels() -> [Level; CHANNEL_LEVELS] {
    let line = |ratio: f32| Level {
        ratio,
        line: MiddleLine {
            enabled: false,
            color: BLUE,
            width: 1.0,
            dash: LineDash::Dashed,
        },
    };
    [line(0.25), line(0.75), line(-0.25), line(1.25)]
}

/// The regression trend's inputs and its three lines.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct RegressionOptions {
    pub source: PriceSource,
    /// Standard deviations above the base line; the band is drawn at `upper_dev * sigma`.
    pub upper_dev: f32,
    /// Standard deviations from the base line, written with the sign it is drawn at (-2 is two
    /// below).
    pub lower_dev: f32,
    /// The least-squares line itself (`enabled` shows it).
    pub base: MiddleLine,
    pub upper: MiddleLine,
    pub lower: MiddleLine,
    /// Print Pearson's R, the correlation of the price with the bar number.
    pub pearson: bool,
}

impl Default for RegressionOptions {
    fn default() -> Self {
        let band = MiddleLine {
            enabled: true,
            color: [41, 98, 255, 77],
            width: 2.0,
            dash: LineDash::Solid,
        };
        Self {
            source: PriceSource::Close,
            upper_dev: 2.0,
            lower_dev: -2.0,
            base: MiddleLine {
                enabled: true,
                color: [242, 54, 69, 77],
                width: 1.0,
                dash: LineDash::Dashed,
            },
            upper: band,
            lower: band,
            pearson: true,
        }
    }
}

fn is_default_pitchfork_levels(levels: &[Level; PITCHFORK_LEVELS]) -> bool {
    *levels == default_pitchfork_levels()
}

fn is_default_channel_levels(levels: &[Level; CHANNEL_LEVELS]) -> bool {
    *levels == default_channel_levels()
}

fn is_default_regression(options: &RegressionOptions) -> bool {
    *options == RegressionOptions::default()
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_default_border(border: &BorderStyle) -> bool {
    *border == BorderStyle::default()
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

/// The signpost's plate colour, and so the colour its emoji sits on until it is changed.
const SIGNPOST_BLUE: Rgba = [41, 98, 255, 255];

fn default_emoji_fill() -> Rgba {
    SIGNPOST_BLUE
}

fn is_default_emoji_fill(value: &Rgba) -> bool {
    *value == SIGNPOST_BLUE
}

fn is_default_emoji(value: &Emoji) -> bool {
    *value == Emoji::default()
}

/// How a drawing looks. Every field after `stats` belongs to the lines, channels and pitchforks
/// and is left out of a stored drawing while it holds its default, so a rectangle does not carry
/// nine pitchfork levels around in every saved layout and every synced record.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DrawingStyle {
    pub stroke: StrokeStyle,
    pub fill: FillStyle,
    pub text: TextStyle,
    pub extend_left: bool,
    pub extend_right: bool,
    pub middle_line: MiddleLine,
    pub head_start: LineEnd,
    pub head_end: LineEnd,
    /// A dot at the middle of a line.
    pub middle_point: bool,
    /// The two end prices printed beside an arrow; a horizontal line's price tag on the axis.
    pub price_labels: bool,
    pub stats: ArrowStats,
    /// The pitchfork's extra lines, each drawn on both sides of the median.
    #[serde(skip_serializing_if = "is_default_pitchfork_levels")]
    pub pitchfork_levels: [Level; PITCHFORK_LEVELS],
    /// The parallel channel's extra lines, as ratios of the width from its first edge.
    #[serde(skip_serializing_if = "is_default_channel_levels")]
    pub channel_levels: [Level; CHANNEL_LEVELS],
    #[serde(skip_serializing_if = "is_default_regression")]
    pub regression: RegressionOptions,
    /// Draw every line of a pitchfork in the median's colour.
    #[serde(skip_serializing_if = "is_false")]
    pub use_one_color: bool,
    /// The time tag a vertical line or a crossline puts on the time axis.
    #[serde(skip_serializing_if = "is_false")]
    pub time_label: bool,
    /// Keep the text but do not draw it.
    #[serde(skip_serializing_if = "is_false")]
    pub hide_text: bool,
    /// The border of a text box. Only the tools with [`Caps::BOX`](super::Caps::BOX) read it.
    #[serde(skip_serializing_if = "is_default_border")]
    pub border: BorderStyle,
    /// A signpost's emoji at its anchor, whether it is shown, and what it sits on.
    #[serde(skip_serializing_if = "is_default_emoji")]
    pub emoji: Emoji,
    #[serde(skip_serializing_if = "is_false")]
    pub emoji_shown: bool,
    #[serde(
        default = "default_emoji_fill",
        skip_serializing_if = "is_default_emoji_fill"
    )]
    pub emoji_fill: Rgba,
    /// Wrap a text box's text at `box_width`.
    #[serde(skip_serializing_if = "is_false")]
    pub wrap: bool,
    /// The width a wrapping text box is laid out at, in logical pixels; `0` is the tool's own.
    #[serde(skip_serializing_if = "is_zero")]
    pub box_width: f32,
}

impl Default for DrawingStyle {
    fn default() -> Self {
        Self {
            stroke: StrokeStyle::default(),
            fill: FillStyle::default(),
            text: TextStyle::default(),
            extend_left: false,
            extend_right: false,
            middle_line: MiddleLine::default(),
            head_start: LineEnd::Plain,
            head_end: LineEnd::Plain,
            middle_point: false,
            price_labels: false,
            stats: ArrowStats::default(),
            pitchfork_levels: default_pitchfork_levels(),
            channel_levels: default_channel_levels(),
            regression: RegressionOptions::default(),
            use_one_color: false,
            time_label: false,
            hide_text: false,
            border: BorderStyle::default(),
            emoji: Emoji::default(),
            emoji_shown: false,
            emoji_fill: SIGNPOST_BLUE,
            wrap: false,
            box_width: 0.0,
        }
    }
}

impl DrawingStyle {
    /// The outer width of a wrapping text box in this style.
    pub fn wrapped_width(&self) -> f32 {
        if self.box_width > 0.0 {
            self.box_width.max(MIN_BOX_WIDTH)
        } else {
            DEFAULT_BOX_WIDTH
        }
    }

    /// A trend line's own look: a solid blue 1.5 px segment, its text in the same blue and
    /// below the line, as TradingView puts it.
    pub fn trend_line() -> Self {
        let mut style = Self::default();
        style.stroke.color = TREND_LINE_COLOR;
        style.stroke.width = TREND_LINE_WIDTH;
        style.text.color = TREND_LINE_COLOR;
        style.text.valign = VerticalAlign::Bottom;
        style
    }

    /// A style with `color` for the stroke, the text and the middle line, and `color` at
    /// `fill_alpha` for the fill.
    pub fn colored(color: Rgba, fill_alpha: u8, fill_enabled: bool) -> Self {
        let mut style = Self::default();
        style.stroke.color = color;
        style.text.color = color;
        style.middle_line.color = color;
        style.fill = FillStyle {
            enabled: fill_enabled,
            color: [color[0], color[1], color[2], fill_alpha],
        };
        style
    }

    /// The style with every value brought into its valid range: finite, positive widths and
    /// sizes, so a hand-edited or damaged row cannot break the renderer.
    pub fn sanitized(mut self) -> Self {
        let width = |value: f32, fallback: f32| {
            if value.is_finite() {
                value.clamp(0.5, MAX_STROKE_WIDTH)
            } else {
                fallback
            }
        };
        self.stroke.width = width(self.stroke.width, 2.0);
        self.middle_line.width = width(self.middle_line.width, 1.0);
        let ratio = |value: f32, fallback: f32| {
            if value.is_finite() {
                value.clamp(-MAX_LEVEL_RATIO, MAX_LEVEL_RATIO)
            } else {
                fallback
            }
        };
        for (index, level) in self.pitchfork_levels.iter_mut().enumerate() {
            level.ratio = ratio(level.ratio, default_pitchfork_levels()[index].ratio);
            level.line.width = width(level.line.width, 2.0);
        }
        for (index, level) in self.channel_levels.iter_mut().enumerate() {
            level.ratio = ratio(level.ratio, default_channel_levels()[index].ratio);
            level.line.width = width(level.line.width, 1.0);
        }
        let deviation = |value: f32, fallback: f32| {
            if value.is_finite() {
                value.clamp(-MAX_DEVIATION, MAX_DEVIATION)
            } else {
                fallback
            }
        };
        let regression = &mut self.regression;
        regression.upper_dev = deviation(regression.upper_dev, 2.0);
        regression.lower_dev = deviation(regression.lower_dev, -2.0);
        for line in [
            &mut regression.base,
            &mut regression.upper,
            &mut regression.lower,
        ] {
            line.width = width(line.width, 1.0);
        }
        self.border.width = width(self.border.width, 1.0);
        self.box_width = if self.box_width.is_finite() {
            self.box_width.clamp(0.0, MAX_BOX_WIDTH)
        } else {
            0.0
        };
        self.text.size = if self.text.size.is_finite() {
            self.text.size.clamp(TEXT_SIZE_RANGE.0, TEXT_SIZE_RANGE.1)
        } else {
            14.0
        };
        self
    }
}

/// The timeframes a drawing shows on. A mask over [`Timeframe::ALL`]; the default is every one.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Visibility {
    pub mask: u8,
}

const ALL_TIMEFRAMES: u8 = (1 << Timeframe::ALL.len()) - 1;

impl Default for Visibility {
    fn default() -> Self {
        Self::ALL
    }
}

impl Visibility {
    pub const ALL: Self = Self {
        mask: ALL_TIMEFRAMES,
    };

    /// Each timeframe owns one fixed bit, so the saved masks keep their meaning if the list of
    /// timeframes is ever reordered or extended.
    fn bit(timeframe: Timeframe) -> u8 {
        match timeframe {
            Timeframe::Hour => 1 << 0,
            Timeframe::Day => 1 << 1,
            Timeframe::Week => 1 << 2,
            Timeframe::Month => 1 << 3,
            Timeframe::Quarter => 1 << 4,
            Timeframe::Year => 1 << 5,
        }
    }

    /// The mask in force: one that names no timeframe (a damaged row) means every one, so a
    /// drawing can never be lost to being shown nowhere.
    fn effective(&self) -> u8 {
        match self.mask & ALL_TIMEFRAMES {
            0 => ALL_TIMEFRAMES,
            mask => mask,
        }
    }

    pub fn shows(&self, timeframe: Timeframe) -> bool {
        self.effective() & Self::bit(timeframe) != 0
    }

    pub fn is_all(&self) -> bool {
        self.effective() == ALL_TIMEFRAMES
    }

    /// `self` with `timeframe` shown or hidden. Hiding the last one is refused: a drawing that
    /// shows nowhere could not be found again to change it.
    pub fn with(mut self, timeframe: Timeframe, shown: bool) -> Self {
        let bit = Self::bit(timeframe);
        let current = self.effective();
        let next = if shown { current | bit } else { current & !bit };
        if next & ALL_TIMEFRAMES != 0 {
            self.mask = next & ALL_TIMEFRAMES;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_defaults_to_every_timeframe_and_decodes_from_nothing() {
        let visibility: Visibility = serde_json::from_str("{}").unwrap();
        assert!(visibility.is_all());
        for timeframe in Timeframe::ALL {
            assert!(visibility.shows(timeframe));
        }
    }

    #[test]
    fn a_damaged_visibility_mask_shows_everywhere_and_bits_are_fixed_per_timeframe() {
        let damaged = Visibility { mask: 0 };
        assert!(damaged.is_all());
        assert!(
            Timeframe::ALL
                .iter()
                .all(|timeframe| damaged.shows(*timeframe))
        );
        let beyond = Visibility { mask: 0b1100_0000 };
        assert!(beyond.is_all(), "bits no timeframe owns name nothing");
        // The saved form is stable: Day is bit 1, Week bit 2.
        let weekly_off = Visibility::ALL.with(Timeframe::Week, false);
        assert_eq!(weekly_off.mask, 0b11_1011);
        assert!(!weekly_off.shows(Timeframe::Week) && weekly_off.shows(Timeframe::Day));
    }

    #[test]
    fn visibility_toggles_one_timeframe_and_keeps_at_least_one() {
        let only_day = Timeframe::ALL
            .iter()
            .filter(|timeframe| **timeframe != Timeframe::Day)
            .fold(Visibility::ALL, |visibility, timeframe| {
                visibility.with(*timeframe, false)
            });
        assert!(only_day.shows(Timeframe::Day));
        assert!(!only_day.shows(Timeframe::Hour));
        assert!(!only_day.is_all());
        // The last visible timeframe cannot be hidden.
        assert_eq!(only_day.with(Timeframe::Day, false), only_day);
        assert!(only_day.with(Timeframe::Hour, true).shows(Timeframe::Hour));
    }

    #[test]
    fn a_style_decodes_from_a_partial_row() {
        let style: DrawingStyle =
            serde_json::from_str(r#"{"stroke":{"width":5.0},"extend_right":true}"#).unwrap();
        assert_eq!(style.stroke.width, 5.0);
        assert_eq!(style.stroke.color, BLUE);
        assert!(style.extend_right && !style.extend_left);
        assert_eq!(
            serde_json::from_str::<DrawingStyle>("{}").unwrap(),
            DrawingStyle::default()
        );
    }

    #[test]
    fn sanitizing_repairs_a_damaged_style() {
        let mut style = DrawingStyle::default();
        style.stroke.width = f32::NAN;
        style.middle_line.width = -3.0;
        style.text.size = f32::INFINITY;
        let style = style.sanitized();
        assert_eq!(style.stroke.width, 2.0);
        assert_eq!(style.middle_line.width, 0.5);
        assert_eq!(style.text.size, 14.0);
        let mut huge = DrawingStyle::default();
        huge.stroke.width = 1e9;
        huge.text.size = 1.0;
        let huge = huge.sanitized();
        assert_eq!(huge.stroke.width, MAX_STROKE_WIDTH);
        assert_eq!(huge.text.size, TEXT_SIZE_RANGE.0);
    }

    #[test]
    fn colored_tints_stroke_text_and_fill_together() {
        let style = DrawingStyle::colored([10, 20, 30, 255], 40, true);
        assert_eq!(style.stroke.color, [10, 20, 30, 255]);
        assert_eq!(style.text.color, [10, 20, 30, 255]);
        assert_eq!(style.fill.color, [10, 20, 30, 40]);
        assert!(style.fill.enabled);
    }
}
