//! How a drawing looks: stroke, fill, text, line ends, extension and arrow statistics. Pure, no
//! egui types, and kept apart from the geometry so a style can be saved as a template, copied to
//! another drawing or become a tool's default without carrying any coordinates.
//!
//! Colours are straight `[r, g, b, a]` bytes where the alpha is the opacity (as in the chart
//! canvas), and every struct decodes from an older or partial row through `#[serde(default)]`.

use super::Timeframe;
use super::chart_type::{LineDash, Rgba};
use serde::{Deserialize, Serialize};

/// The thickest stroke a drawing may carry, in logical pixels.
pub const MAX_STROKE_WIDTH: f32 = 60.0;
/// The range of a drawing's text size, in points.
pub const TEXT_SIZE_RANGE: (f32, f32) = (8.0, 72.0);

/// The blue the generic style (and the trend line) is drawn in.
pub const BLUE: Rgba = [41, 98, 255, 255];

/// The blue a trend line has always been drawn in.
pub const TREND_LINE_COLOR: Rgba = [10, 132, 255, 255];
/// The width a trend line has always been drawn at.
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
        }
    }
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
    /// The two end prices printed beside an arrow.
    pub price_labels: bool,
    pub stats: ArrowStats,
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
        }
    }
}

impl DrawingStyle {
    /// How a trend line looked before it had a style: a solid blue 1.5 px segment. A row
    /// without a style decodes to this, so no stored line changes.
    pub fn trend_line() -> Self {
        let mut style = Self::default();
        style.stroke.color = TREND_LINE_COLOR;
        style.stroke.width = TREND_LINE_WIDTH;
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
