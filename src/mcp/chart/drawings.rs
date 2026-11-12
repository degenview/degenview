//! Drawing arguments as an MCP client sends them, checked before anything is queued.
//!
//! Everything here is a pure function of the arguments, so a request with a bad time, price,
//! colour or id is refused on the tool thread and never reaches the app. What needs the chart's
//! candles (snapping a time to its bar, reading a bar's high or low) is left as a spec and
//! resolved on the app thread by `mcp::chart::data`.

use crate::format::parse_timestamp;
use crate::mcp::bridge::McpError;
use crate::model::chart::kind::{LineDash, Rgba};
use crate::model::drawings::MAX_TEXT_CHARS;
use crate::model::drawings::style::MAX_STROKE_WIDTH;
use schemars::JsonSchema;
use serde::Deserialize;

/// Drawings one `degenview_add_drawings` call may create (one undo step in the app).
pub const MAX_DRAWINGS_PER_CALL: usize = 25;
/// Drawings a chart may hold once a client has added to it. The app itself has no cap; this keeps
/// a looping client from burying a chart.
pub const MAX_DRAWINGS_PER_CHART: usize = 500;
/// Drawings one `degenview_delete_drawings` call may remove.
pub const MAX_DELETES_PER_CALL: usize = 50;
/// Latest time accepted: the end of year 9999.
const MAX_TIME_MS: i64 = 253_402_300_799_000;
/// The alpha of a fill colour given without one, matching the app's own default fill.
const DEFAULT_FILL_ALPHA: u8 = 51;

/// The drawings a client can create and edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpDrawingTool {
    /// A line through two points (`a`, `b`), optionally prolonged to the left and/or right.
    TrendLine,
    /// A box between two opposite corners (`a`, `b`), optionally prolonged left and/or right.
    Rectangle,
    /// A line from `a` through `b`, running on to the right edge of the chart.
    Ray,
    /// A line through `a` and `b`, running to both edges of the chart.
    ExtendedLine,
    /// An arrow from `a` to `b`.
    Arrow,
    /// Fibonacci retracement levels between `a` (the start of the move) and `b` (its end).
    FibRetracement,
    /// A horizontal line across the chart at the price of `a` (give no `b`; the time is
    /// ignored).
    HorizontalLine,
    /// A horizontal ray from `a` to the right edge (give no `b`).
    HorizontalRay,
    /// A vertical line at the time of `a` (give no `b`; the price is ignored but still needed).
    VerticalLine,
    /// A horizontal and a vertical line crossing at `a` (give no `b`).
    Crossline,
    /// A text label at `a` (give no `b`); needs `text`.
    Text,
}

impl McpDrawingTool {
    pub const ALL: [Self; 11] = [
        Self::TrendLine,
        Self::Rectangle,
        Self::Ray,
        Self::ExtendedLine,
        Self::Arrow,
        Self::FibRetracement,
        Self::HorizontalLine,
        Self::HorizontalRay,
        Self::VerticalLine,
        Self::Crossline,
        Self::Text,
    ];

    /// The app's tool.
    pub fn tool(self) -> crate::model::Tool {
        use crate::model::Tool;
        match self {
            Self::TrendLine => Tool::TrendLine,
            Self::Rectangle => Tool::Rectangle,
            Self::Ray => Tool::Ray,
            Self::ExtendedLine => Tool::ExtendedLine,
            Self::Arrow => Tool::Arrow,
            Self::FibRetracement => Tool::FibRetracement,
            Self::HorizontalLine => Tool::HorizontalLine,
            Self::HorizontalRay => Tool::HorizontalRay,
            Self::VerticalLine => Tool::VerticalLine,
            Self::Crossline => Tool::Crossline,
            Self::Text => Tool::Text,
        }
    }

    /// The tool this server can edit, if `tool` is one.
    pub fn of_tool(tool: crate::model::Tool) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.tool() == tool)
    }

    /// How many points place it: one for a level, a marker or a label, two for the rest.
    pub fn points(self) -> usize {
        match self {
            Self::HorizontalLine
            | Self::HorizontalRay
            | Self::VerticalLine
            | Self::Crossline
            | Self::Text => 1,
            _ => 2,
        }
    }

    /// Whether a line width and dash apply: a text label has no line to draw them on.
    pub fn takes_stroke(self) -> bool {
        self != Self::Text
    }

    /// Whether a fill colour applies.
    pub fn takes_fill(self) -> bool {
        self == Self::Rectangle
    }

    /// Whether text applies (a label inside a box, or the label itself).
    pub fn takes_text(self) -> bool {
        matches!(self, Self::Rectangle | Self::Text)
    }

    /// Whether the tool has its own left and right extension to set.
    pub fn takes_extension(self) -> bool {
        matches!(self, Self::TrendLine | Self::Rectangle)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::TrendLine => "trend lines",
            Self::Rectangle => "rectangles",
            Self::Ray => "rays",
            Self::ExtendedLine => "extended lines",
            Self::Arrow => "arrows",
            Self::FibRetracement => "Fibonacci retracements",
            Self::HorizontalLine => "horizontal lines",
            Self::HorizontalRay => "horizontal rays",
            Self::VerticalLine => "vertical lines",
            Self::Crossline => "crosslines",
            Self::Text => "text labels",
        }
    }

    /// The patch is refused when it names something this tool does not take.
    pub fn check_patch(self, patch: &StylePatch) -> Result<(), McpError> {
        if (patch.width.is_some() || patch.dash.is_some()) && !self.takes_stroke() {
            return Err(invalid(format!(
                "width and dash do not apply to {}",
                self.name()
            )));
        }
        if patch.fill_color.is_some() && !self.takes_fill() {
            return Err(invalid(format!(
                "fill_color does not apply to {}",
                self.name()
            )));
        }
        if patch.text.is_some() && !self.takes_text() {
            return Err(invalid(format!("text does not apply to {}", self.name())));
        }
        if (patch.extend_left.is_some() || patch.extend_right.is_some()) && !self.takes_extension()
        {
            return Err(invalid(format!(
                "extend_left and extend_right do not apply to {}",
                self.name()
            )));
        }
        Ok(())
    }
}

/// A moment: ISO-8601 UTC text as `degenview_get_candles` writes it, or epoch milliseconds.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TimeArg {
    Millis(i64),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceField {
    Open,
    High,
    Low,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpDash {
    Solid,
    Dashed,
    Dotted,
}

impl McpDash {
    fn line_dash(self) -> LineDash {
        match self {
            Self::Solid => LineDash::Solid,
            Self::Dashed => LineDash::Dashed,
            Self::Dotted => LineDash::Dotted,
        }
    }
}

/// One point of a drawing.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnchorArg {
    /// When: a `time` copied from `degenview_get_candles` (e.g. `2026-10-20T00:00:00Z`) or epoch
    /// milliseconds. Inside the loaded candles it snaps to the start of the bar it falls in;
    /// before or after them it is kept as given, so a line can run into the future.
    pub time: TimeArg,
    /// The price level, as a plain number above zero. Give this or `price_from`, not both.
    pub price: Option<f64>,
    /// Take the price from the bar at `time` instead: its `open`, `high`, `low` or `close`.
    /// Needs a time inside the loaded candles.
    pub price_from: Option<PriceField>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PriceSpec {
    Value(f64),
    Field(PriceField),
}

/// A checked anchor, waiting for the candles to place it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorSpec {
    pub time_ms: i64,
    pub price: PriceSpec,
}

/// Style changes a client asked for; `None` leaves that part alone (or at the tool's default
/// when the drawing is new).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StylePatch {
    pub color: Option<Rgba>,
    pub width: Option<f32>,
    pub dash: Option<LineDash>,
    pub extend_left: Option<bool>,
    pub extend_right: Option<bool>,
    /// A fill colour; rectangles only. Without an alpha it is translucent.
    pub fill_color: Option<Rgba>,
    /// A label; rectangles only. An empty string clears it.
    pub text: Option<String>,
}

impl StylePatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A drawing to create.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDrawing {
    pub tool: McpDrawingTool,
    pub a: AnchorSpec,
    /// Present exactly for the tools that take two points.
    pub b: Option<AnchorSpec>,
    pub patch: StylePatch,
}

/// Changes to a drawing that exists.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingEdit {
    pub drawing_id: u64,
    pub a: Option<AnchorSpec>,
    pub b: Option<AnchorSpec>,
    pub patch: StylePatch,
    pub hidden: Option<bool>,
}

/// A drawing to create, as sent.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewDrawingArg {
    pub tool: McpDrawingTool,
    /// First point: one end of a trend line, or one corner of a rectangle.
    pub a: AnchorArg,
    /// Second point: the other end, or the opposite corner. Required for two-point tools
    /// (trend_line, rectangle, ray, extended_line, arrow, fib_retracement), refused for
    /// one-point tools (horizontal_line, horizontal_ray, vertical_line, crossline, text).
    pub b: Option<AnchorArg>,
    /// Line colour as `#RRGGBB` or `#RRGGBBAA`. Default: the app's own for that tool.
    #[schemars(length(max = 9))]
    pub color: Option<String>,
    /// Line width in pixels, 0.5 to 60. Not for `text`, which has no line.
    #[schemars(range(min = 0.5, max = 60.0))]
    pub width: Option<f64>,
    /// Line dash. Not for `text`.
    pub dash: Option<McpDash>,
    /// Prolong to the left edge of the chart.
    pub extend_left: Option<bool>,
    /// Prolong to the right edge of the chart.
    pub extend_right: Option<bool>,
    /// Rectangles only: the fill as `#RRGGBB` (translucent) or `#RRGGBBAA`.
    #[schemars(length(max = 9))]
    pub fill_color: Option<String>,
    /// Rectangles: a label inside the box. Text: the label itself (required).
    #[schemars(length(max = 500))]
    pub text: Option<String>,
}

fn invalid(message: impl Into<String>) -> McpError {
    McpError::InvalidArgument(message.into())
}

/// `#RRGGBB` or `#RRGGBBAA` (the `#` is optional), with whether an alpha was given.
fn parse_color(text: &str) -> Option<(Rgba, bool)> {
    let hex = text.trim().trim_start_matches('#');
    if !hex.is_ascii() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).ok();
    match hex.len() {
        6 => Some(([byte(0)?, byte(2)?, byte(4)?, 255], false)),
        8 => Some(([byte(0)?, byte(2)?, byte(4)?, byte(6)?], true)),
        _ => None,
    }
}

/// The milliseconds in the digits after a decimal point (`5` is 500, `250Z` is 250, `1239` is
/// 123: finer than a millisecond is cut off, never rounded up into the next one). One to nine
/// digits and an optional `Z`; anything else, including no digits at all, is not a fraction.
fn fraction_millis(fraction: &str) -> Option<i64> {
    let digits = fraction.strip_suffix(['Z', 'z']).unwrap_or(fraction);
    if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut millis = 0;
    for position in 0..3 {
        let digit = digits
            .as_bytes()
            .get(position)
            .map_or(0, |b| i64::from(b - b'0'));
        millis = millis * 10 + digit;
    }
    Some(millis)
}

/// Epoch milliseconds from a client's time: an integer, digits as text, or ISO-8601 UTC with
/// optional fractional seconds (kept to the millisecond) and a `Z`. Offsets other than UTC are
/// refused.
pub fn parse_time(arg: &TimeArg) -> Result<i64, McpError> {
    let bad = |what: &str| {
        invalid(format!(
            "{what} is not a time; use e.g. 2026-10-20T00:00:00Z"
        ))
    };
    let ms = match arg {
        TimeArg::Millis(ms) => *ms,
        TimeArg::Text(text) => {
            let text = text.trim();
            if text.is_empty() || text.len() > 40 {
                return Err(bad("An empty or very long value"));
            }
            if text.chars().all(|c| c.is_ascii_digit()) {
                text.parse::<i64>().map_err(|_| bad(text))?
            } else {
                let (head, fraction) = match text.split_once('.') {
                    Some((head, fraction)) => (head, Some(fraction)),
                    None => (text, None),
                };
                let whole = parse_timestamp(head).ok_or_else(|| bad(text))?;
                let millis = match fraction {
                    None => 0,
                    Some(fraction) => fraction_millis(fraction).ok_or_else(|| bad(text))?,
                };
                whole.checked_add(millis).ok_or_else(|| bad(text))?
            }
        }
    };
    if !(1..=MAX_TIME_MS).contains(&ms) {
        return Err(invalid(format!("Time {ms} is outside the supported range")));
    }
    Ok(ms)
}

impl AnchorArg {
    /// Checks the time and the price choice; `name` ("a" or "b") only labels errors.
    pub fn check(&self, name: &str) -> Result<AnchorSpec, McpError> {
        let time_ms = parse_time(&self.time).map_err(|e| match e {
            McpError::InvalidArgument(message) => invalid(format!("Point {name}: {message}")),
            other => other,
        })?;
        let price = match (self.price, self.price_from) {
            (Some(_), Some(_)) => {
                return Err(invalid(format!(
                    "Point {name}: give price or price_from, not both"
                )));
            }
            (None, None) => {
                return Err(invalid(format!(
                    "Point {name}: give price or price_from (open, high, low or close)"
                )));
            }
            (Some(price), None) => {
                if !price.is_finite() || price <= 0.0 {
                    return Err(invalid(format!(
                        "Point {name}: price must be a number above zero"
                    )));
                }
                PriceSpec::Value(price)
            }
            (None, Some(field)) => PriceSpec::Field(field),
        };
        Ok(AnchorSpec { time_ms, price })
    }
}

impl StylePatch {
    /// Checks every style argument; the same rules for a new drawing and an edit.
    pub fn check(
        color: Option<&str>,
        width: Option<f64>,
        dash: Option<McpDash>,
        extend: (Option<bool>, Option<bool>),
        fill_color: Option<&str>,
        text: Option<&str>,
    ) -> Result<Self, McpError> {
        let colour = |name: &str, value: &str| {
            parse_color(value).ok_or_else(|| {
                invalid(format!(
                    "{name} must be #RRGGBB or #RRGGBBAA, not '{value}'"
                ))
            })
        };
        let width = match width {
            None => None,
            Some(width)
                if width.is_finite() && (0.5..=f64::from(MAX_STROKE_WIDTH)).contains(&width) =>
            {
                Some(width as f32)
            }
            Some(width) => {
                return Err(invalid(format!(
                    "width must be between 0.5 and {MAX_STROKE_WIDTH}, not {width}"
                )));
            }
        };
        let text = match text {
            None => None,
            Some(text) => {
                if text.chars().count() > MAX_TEXT_CHARS {
                    return Err(invalid(format!(
                        "text is limited to {MAX_TEXT_CHARS} characters"
                    )));
                }
                if text.chars().any(|c| c.is_control() && c != '\n') {
                    return Err(invalid("text must not contain control characters"));
                }
                Some(text.to_owned())
            }
        };
        Ok(Self {
            color: color
                .map(|value| colour("color", value))
                .transpose()?
                .map(|(rgba, _)| rgba),
            width,
            dash: dash.map(McpDash::line_dash),
            extend_left: extend.0,
            extend_right: extend.1,
            fill_color: fill_color
                .map(|value| colour("fill_color", value))
                .transpose()?
                .map(|(mut rgba, had_alpha)| {
                    if !had_alpha {
                        rgba[3] = DEFAULT_FILL_ALPHA;
                    }
                    rgba
                }),
            text,
        })
    }
}

impl NewDrawingArg {
    pub fn check(&self) -> Result<NewDrawing, McpError> {
        let patch = StylePatch::check(
            self.color.as_deref(),
            self.width,
            self.dash,
            (self.extend_left, self.extend_right),
            self.fill_color.as_deref(),
            self.text.as_deref(),
        )?;
        self.tool.check_patch(&patch)?;
        if self.tool == McpDrawingTool::Text
            && patch
                .text
                .as_deref()
                .is_none_or(|text| text.trim().is_empty())
        {
            return Err(invalid("a text drawing needs text"));
        }
        let b = match (self.tool.points(), &self.b) {
            (2, Some(b)) => Some(b.check("b")?),
            (2, None) => {
                return Err(invalid(format!("{:?} needs a second point `b`", self.tool)));
            }
            (_, None) => None,
            (_, Some(_)) => {
                return Err(invalid(format!(
                    "{:?} takes one point: leave `b` out",
                    self.tool
                )));
            }
        };
        Ok(NewDrawing {
            tool: self.tool,
            a: self.a.check("a")?,
            b,
            patch,
        })
    }
}

/// A drawing id from `degenview_list_drawings`.
pub fn parse_drawing_id(raw: &str) -> Result<u64, McpError> {
    raw.trim()
        .parse::<u64>()
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(|| invalid("drawing_id must be an id returned by degenview_list_drawings"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(time: TimeArg, price: Option<f64>, from: Option<PriceField>) -> AnchorArg {
        AnchorArg {
            time,
            price,
            price_from: from,
        }
    }

    fn text(value: &str) -> TimeArg {
        TimeArg::Text(value.to_owned())
    }

    #[test]
    fn times_are_read_from_iso_text_digits_or_milliseconds() {
        let day = 1_760_918_400_000;
        for arg in [
            text("2025-10-20T00:00:00Z"),
            text("2025-10-20T00:00:00.000Z"),
            text("2025-10-20T00:00:00.0Z"),
            text("2025-10-20 00:00:00"),
            text("2025-10-20"),
            text(" 1760918400000 "),
            TimeArg::Millis(day),
        ] {
            assert_eq!(parse_time(&arg).unwrap(), day, "{arg:?}");
        }
        assert_eq!(
            parse_time(&text("2025-10-20T01:30:00Z")).unwrap(),
            day + 5_400_000
        );
    }

    #[test]
    fn fractional_seconds_are_kept_to_the_millisecond_and_never_rounded_up() {
        let day = 1_760_918_400_000;
        for (written, extra) in [
            ("2025-10-20T00:00:00.999Z", 999),
            ("2025-10-20T00:00:00.5Z", 500),
            ("2025-10-20T00:00:00.25", 250),
            ("2025-10-20T00:00:00.007Z", 7),
            ("2025-10-20T00:00:00.1239Z", 123),
            ("2025-10-20T00:00:00.999999999Z", 999),
        ] {
            assert_eq!(
                parse_time(&text(written)).unwrap(),
                day + extra,
                "{written}"
            );
        }
    }

    #[test]
    fn bad_times_are_refused() {
        for arg in [
            text(""),
            text("tomorrow"),
            text("2025-13-01T00:00:00Z"),
            text("2025-02-30T00:00:00Z"),
            text("2025-10-20T00:00:00+02:00"),
            text("2025-10-20T00:00:00.5x"),
            text("2025-10-20T00:00:00."),
            text("2025-10-20T00:00:00.Z"),
            text("2025-10-20T00:00:00.5ZZ"),
            text("2025-10-20T00:00:00.1234567890Z"),
            text(&"9".repeat(41)),
            text("99999999999999999999"),
            TimeArg::Millis(0),
            TimeArg::Millis(-5),
            TimeArg::Millis(MAX_TIME_MS + 1),
        ] {
            assert!(
                matches!(parse_time(&arg), Err(McpError::InvalidArgument(_))),
                "{arg:?}"
            );
        }
    }

    #[test]
    fn an_anchor_needs_exactly_one_way_to_say_its_price() {
        let ok = |a: AnchorArg| a.check("a");
        let t = || text("2025-10-20T00:00:00Z");
        assert_eq!(
            ok(anchor(t(), Some(1.5), None)).unwrap().price,
            PriceSpec::Value(1.5)
        );
        assert_eq!(
            ok(anchor(t(), None, Some(PriceField::Low))).unwrap().price,
            PriceSpec::Field(PriceField::Low)
        );
        // Prices a DEX token really has.
        assert!(ok(anchor(t(), Some(0.000_000_278), None)).is_ok());
        for bad in [
            anchor(t(), Some(1.0), Some(PriceField::High)),
            anchor(t(), None, None),
            anchor(t(), Some(0.0), None),
            anchor(t(), Some(-3.0), None),
            anchor(t(), Some(f64::NAN), None),
            anchor(t(), Some(f64::INFINITY), None),
            anchor(text("nope"), Some(1.0), None),
        ] {
            let error = ok(bad.clone()).unwrap_err();
            assert!(
                matches!(&error, McpError::InvalidArgument(m) if m.contains("Point a")),
                "{bad:?}: {error:?}"
            );
        }
    }

    #[test]
    fn colours_widths_and_text_are_checked() {
        let check = |color, width, fill, text| {
            StylePatch::check(color, width, None, (None, None), fill, text)
        };
        let patch = check(Some("#FFC107"), Some(2.5), Some("#2962FF"), Some("Supply")).unwrap();
        assert_eq!(patch.color, Some([255, 193, 7, 255]));
        assert_eq!(patch.width, Some(2.5));
        // A fill without an alpha is translucent; one with an alpha is taken as given.
        assert_eq!(patch.fill_color, Some([41, 98, 255, DEFAULT_FILL_ALPHA]));
        assert_eq!(
            check(None, None, Some("2962FF80"), None)
                .unwrap()
                .fill_color,
            Some([41, 98, 255, 0x80])
        );
        assert!(check(None, None, None, Some("")).unwrap().text.is_some());
        assert!(check(None, None, None, None).unwrap().is_empty());
        for bad in [
            check(Some("red"), None, None, None),
            check(Some("#12345"), None, None, None),
            check(Some("#GGGGGG"), None, None, None),
            check(Some("#ÄÄÄ"), None, None, None),
            check(None, Some(0.0), None, None),
            check(None, Some(61.0), None, None),
            check(None, Some(f64::NAN), None, None),
            check(None, None, Some("#12"), None),
            check(None, None, None, Some("bell\u{7}")),
        ] {
            assert!(matches!(bad, Err(McpError::InvalidArgument(_))), "{bad:?}");
        }
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(check(None, None, None, Some(&long)).is_err());
        assert!(check(None, None, None, Some(&"x".repeat(MAX_TEXT_CHARS))).is_ok());
    }

    #[test]
    fn a_trend_line_has_no_fill_or_label() {
        let arg = |fill: Option<&str>, label: Option<&str>| NewDrawingArg {
            tool: McpDrawingTool::TrendLine,
            a: anchor(TimeArg::Millis(1_000), Some(1.0), None),
            b: Some(anchor(TimeArg::Millis(2_000), Some(2.0), None)),
            color: None,
            width: None,
            dash: None,
            extend_left: None,
            extend_right: Some(true),
            fill_color: fill.map(str::to_owned),
            text: label.map(str::to_owned),
        };
        let line = arg(None, None).check().unwrap();
        assert_eq!(line.patch.extend_right, Some(true));
        assert!(arg(Some("#FFFFFF"), None).check().is_err());
        assert!(arg(None, Some("hi")).check().is_err());
    }

    #[test]
    fn drawing_ids_are_plain_nonzero_numbers() {
        assert_eq!(parse_drawing_id(" 12 ").unwrap(), 12);
        for bad in ["", "0", "-1", "1.5", "abc", "99999999999999999999999"] {
            assert!(parse_drawing_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_tool_takes_only_the_style_fields_that_show() {
        use crate::model::chart::kind::LineDash;
        let one = |patch: StylePatch| patch;
        for tool in McpDrawingTool::ALL {
            let fields = [
                (
                    "width",
                    one(StylePatch {
                        width: Some(2.0),
                        ..StylePatch::default()
                    }),
                    tool.takes_stroke(),
                ),
                (
                    "dash",
                    one(StylePatch {
                        dash: Some(LineDash::Dashed),
                        ..StylePatch::default()
                    }),
                    tool.takes_stroke(),
                ),
                (
                    "fill_color",
                    one(StylePatch {
                        fill_color: Some([1, 2, 3, 4]),
                        ..StylePatch::default()
                    }),
                    tool.takes_fill(),
                ),
                (
                    "text",
                    one(StylePatch {
                        text: Some("x".into()),
                        ..StylePatch::default()
                    }),
                    tool.takes_text(),
                ),
                (
                    "extend_left",
                    one(StylePatch {
                        extend_left: Some(true),
                        ..StylePatch::default()
                    }),
                    tool.takes_extension(),
                ),
                (
                    "extend_right",
                    one(StylePatch {
                        extend_right: Some(true),
                        ..StylePatch::default()
                    }),
                    tool.takes_extension(),
                ),
                (
                    "color",
                    one(StylePatch {
                        color: Some([1, 2, 3, 255]),
                        ..StylePatch::default()
                    }),
                    true,
                ),
            ];
            for (field, patch, applies) in fields {
                assert_eq!(
                    tool.check_patch(&patch).is_ok(),
                    applies,
                    "{field} on {}",
                    tool.name()
                );
            }
        }
        assert!(!McpDrawingTool::Text.takes_stroke());
        assert!(McpDrawingTool::Crossline.takes_stroke());
    }
}
