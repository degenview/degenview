//! TradingView's drawings as DegenView's. One table says which tool each class is; the points are
//! TradingView's placement points, which DegenView's tools are built from too, and the look is
//! carried over where both have the setting.

use super::color::{self, with_transparency};
use super::drawing::RawDrawing;
use crate::model::annotation::image_content::Content;
use crate::model::annotation::link_card::{Link, LinkCard};
use crate::model::annotation::table::TableData;
use crate::model::chart::kind::{LineDash, Rgba};
use crate::model::drawings::Geometry;
use crate::model::drawings::style::{DrawingStyle, LineEnd, Visibility, VisibilityRow};
use crate::model::fib::style::{FibLevel, Levels};
use crate::model::{Anchor, Drawing, Tool};
use serde_json::Value;

/// Why a drawing is left out.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Unsupported {
    /// DegenView has no such tool.
    Tool,
    /// The picture lives on TradingView's servers.
    Picture,
    /// A position's distances are in price steps and neither the step nor the risk figures say
    /// how long one is.
    PriceStep,
    /// The points do not make the shape.
    Points,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    pub drawing: Drawing,
    /// Text was longer than a DegenView drawing holds.
    pub cut_text: bool,
}

/// TradingView's class name to the DegenView tool.
const TOOLS: &[(&str, Tool)] = &[
    ("LineToolTrendLine", Tool::TrendLine),
    ("LineToolRay", Tool::Ray),
    ("LineToolInfoLine", Tool::InfoLine),
    ("LineToolExtended", Tool::ExtendedLine),
    ("LineToolTrendAngle", Tool::TrendAngle),
    ("LineToolHorzLine", Tool::HorizontalLine),
    ("LineToolHorzRay", Tool::HorizontalRay),
    ("LineToolVertLine", Tool::VerticalLine),
    ("LineToolCrossLine", Tool::Crossline),
    ("LineToolParallelChannel", Tool::ParallelChannel),
    ("LineToolRegressionTrend", Tool::RegressionTrend),
    ("LineToolFlatBottom", Tool::FlatChannel),
    ("LineToolDisjointAngle", Tool::DisjointChannel),
    ("LineToolPitchfork", Tool::Pitchfork),
    // TradingView numbers these the other way round than their names suggest.
    ("LineToolSchiffPitchfork2", Tool::SchiffPitchfork),
    ("LineToolSchiffPitchfork", Tool::ModifiedSchiffPitchfork),
    ("LineToolInsidePitchfork", Tool::InsidePitchfork),
    ("LineToolFibRetracement", Tool::FibRetracement),
    ("LineToolTrendBasedFibExtension", Tool::FibExtension),
    ("LineToolFibChannel", Tool::FibChannel),
    ("LineToolFibTimeZone", Tool::FibTimeZone),
    ("LineToolFibSpeedResistanceFan", Tool::FibSpeedFan),
    ("LineToolTrendBasedFibTime", Tool::FibTrendTime),
    ("LineToolFibCircles", Tool::FibCircles),
    ("LineToolFibSpiral", Tool::FibSpiral),
    ("LineToolFibSpeedResistanceArcs", Tool::FibSpeedArcs),
    ("LineToolFibWedge", Tool::FibWedge),
    ("LineToolPitchfan", Tool::Pitchfan),
    ("LineToolGannSquare", Tool::GannBox),
    ("LineToolGannFixed", Tool::GannSquareFixed),
    ("LineToolGannComplex", Tool::GannSquare),
    ("LineToolGannFan", Tool::GannFan),
    ("LineToolBrush", Tool::Brush),
    ("LineToolHighlighter", Tool::Highlighter),
    ("LineToolArrowMarker", Tool::ArrowMarker),
    ("LineToolArrow", Tool::Arrow),
    ("LineToolArrowMarkUp", Tool::ArrowMarkUp),
    ("LineToolArrowMarkDown", Tool::ArrowMarkDown),
    ("LineToolRectangle", Tool::Rectangle),
    ("LineToolRotatedRectangle", Tool::RotatedRectangle),
    ("LineToolPath", Tool::Path),
    ("LineToolCircle", Tool::Circle),
    ("LineToolPolyline", Tool::Polyline),
    ("LineToolTriangle", Tool::Triangle),
    ("LineToolArc", Tool::Arc),
    ("LineToolBezierQuadro", Tool::Curve),
    ("LineToolBezierCubic", Tool::DoubleCurve),
    ("LineToolText", Tool::Text),
    ("LineToolNote", Tool::Pin),
    ("LineToolTextNote", Tool::Note),
    ("LineToolPriceNote", Tool::PriceNote),
    ("LineToolCallout", Tool::Callout),
    ("LineToolComment", Tool::Comment),
    // The older name of the comment bubble.
    ("LineToolBalloon", Tool::Comment),
    ("LineToolPriceLabel", Tool::PriceLabel),
    ("LineToolSignpost", Tool::Signpost),
    ("LineToolFlagMark", Tool::FlagMark),
    ("LineToolTable", Tool::Table),
    ("LineToolTweet", Tool::Post),
    ("LineTool5PointsPattern", Tool::XabcdPattern),
    ("LineToolCypherPattern", Tool::CypherPattern),
    ("LineToolHeadAndShoulders", Tool::HeadAndShoulders),
    ("LineToolABCD", Tool::AbcdPattern),
    ("LineToolTrianglePattern", Tool::TrianglePattern),
    ("LineToolThreeDrivers", Tool::ThreeDrivesPattern),
    ("LineToolElliottImpulse", Tool::ElliottImpulse),
    ("LineToolElliottCorrection", Tool::ElliottCorrection),
    ("LineToolElliottTriangle", Tool::ElliottTriangle),
    ("LineToolElliottDoubleCombo", Tool::ElliottDoubleCombo),
    ("LineToolElliottTripleCombo", Tool::ElliottTripleCombo),
    ("LineToolCircleLines", Tool::CyclicLines),
    ("LineToolTimeCycles", Tool::TimeCycles),
    ("LineToolSineLine", Tool::SineLine),
    ("LineToolRiskRewardLong", Tool::LongPosition),
    ("LineToolRiskRewardShort", Tool::ShortPosition),
    ("LineToolPrediction", Tool::PositionForecast),
    ("LineToolProjection", Tool::Sector),
    ("LineToolAnchoredVWAP", Tool::AnchoredVwap),
    (
        "LineToolFixedRangeVolumeProfile",
        Tool::FixedRangeVolumeProfile,
    ),
    ("LineToolAnchoredVolumeProfile", Tool::AnchoredVolumeProfile),
    // The older name of the fixed range profile.
    ("LineToolVbPFixed", Tool::FixedRangeVolumeProfile),
    ("LineToolPriceRange", Tool::PriceRange),
    ("LineToolDateRange", Tool::DateRange),
    ("LineToolDateAndPriceRange", Tool::DateAndPriceRange),
];

pub fn tool_of(class: &str) -> Option<Tool> {
    TOOLS
        .iter()
        .find(|(name, _)| *name == class)
        .map(|(_, tool)| *tool)
}

/// A name to show for a class DegenView has no tool for: `LineToolEmoji` is `Emoji`.
pub fn class_label(class: &str) -> &str {
    class.strip_prefix("LineTool").unwrap_or(class)
}

/// `raw` as a DegenView drawing with `id`. `tick` is the market's price step, when known.
pub fn convert(raw: &RawDrawing, id: u64, tick: Option<f64>) -> Result<Converted, Unsupported> {
    if raw.tool == "LineToolImage" {
        return Err(Unsupported::Picture);
    }
    let tool = tool_of(&raw.tool).ok_or(Unsupported::Tool)?;
    let style = style(raw, tool);
    let anchors: Vec<Anchor> = raw
        .points
        .iter()
        .map(|point| Anchor {
            time_ms: point.time_ms(),
            price: point.price,
        })
        .collect();
    let first = *anchors.first().ok_or(Unsupported::Points)?;
    let drawing = match tool {
        Tool::Post => post(raw, id, first, style)?,
        Tool::Table => Drawing::from_geometry(id, Geometry::Table(table(raw, first)), style)
            .ok_or(Unsupported::Points)?,
        Tool::LongPosition | Tool::ShortPosition => {
            let anchors = position(raw, tool, &anchors, tick)?;
            Drawing::build(tool, id, &anchors, style).ok_or(Unsupported::Points)?
        }
        _ => Drawing::build(tool, id, &anchors, style).ok_or(Unsupported::Points)?,
    };
    Ok(finish(raw, drawing))
}

/// The switches every drawing has, and its words.
fn finish(raw: &RawDrawing, mut drawing: Drawing) -> Converted {
    let text = raw.str("text").unwrap_or("").trim_end();
    drawing.set_text(text);
    let cut_text = !text.is_empty()
        && matches!(&drawing, Drawing::Shape { text: kept, .. }
            if kept.chars().count() < text.chars().count());
    let frozen = raw.flag("frozen").unwrap_or(false);
    let shown = raw.flag("visible").unwrap_or(true);
    let visible_on = visibility(raw);
    match &mut drawing {
        Drawing::Brush {
            locked,
            hidden,
            visibility,
            ..
        }
        | Drawing::Shape {
            locked,
            hidden,
            visibility,
            ..
        } => {
            *locked = frozen;
            *hidden = !shown;
            *visibility = visible_on;
        }
        Drawing::Ruler { .. } | Drawing::Unreadable(_) => {}
    }
    Converted { drawing, cut_text }
}

fn visibility(raw: &RawDrawing) -> Visibility {
    let Some(rows) = raw
        .props
        .get("intervalsVisibilities")
        .and_then(Value::as_object)
    else {
        return Visibility::ALL;
    };
    let on = |key: &str| rows.get(key).and_then(Value::as_bool).unwrap_or(true);
    let months = on("months");
    [
        (VisibilityRow::Minutes, on("minutes")),
        (VisibilityRow::Hours, on("hours")),
        (VisibilityRow::Days, on("days")),
        (VisibilityRow::Weeks, on("weeks")),
        (VisibilityRow::Months, months),
        (VisibilityRow::Quarters, months),
        (VisibilityRow::Years, months),
    ]
    .into_iter()
    .fold(Visibility::ALL, |visibility, (row, shown)| {
        visibility.with_row(row, shown)
    })
}

/// A colour setting, under whichever name the tool keeps it.
fn colour(raw: &RawDrawing, keys: &[&str]) -> Option<Rgba> {
    keys.iter()
        .find_map(|key| raw.str(key).and_then(color::parse))
}

fn dash(code: i64) -> LineDash {
    match code {
        1 => LineDash::Dotted,
        2 => LineDash::Dashed,
        _ => LineDash::Solid,
    }
}

/// The tool's own default look, with what TradingView's drawing sets.
fn style(raw: &RawDrawing, tool: Tool) -> DrawingStyle {
    let mut style = tool.default_style();
    // A note's connector and marker are its stroke; a text tool's `color` is its text.
    let writes = matches!(
        tool,
        Tool::Text | Tool::Comment | Tool::PriceLabel | Tool::Callout | Tool::Note
    );
    let stroke_keys: &[&str] = if writes {
        &["lineColor", "markerColor"]
    } else if tool == Tool::Pin {
        &["markerColor"]
    } else {
        &["linecolor", "lineColor", "color"]
    };
    if let Some(color) = colour(raw, stroke_keys) {
        style.stroke.color = color;
    }
    if let Some(width) = raw
        .number("linewidth")
        .or_else(|| raw.number("lineWidth"))
        .or_else(|| (tool == Tool::Highlighter).then(|| raw.number("width"))?)
    {
        style.stroke.width = width as f32;
    }
    if let Some(code) = raw.props.get("linestyle").and_then(Value::as_i64) {
        style.stroke.dash = dash(code);
    }
    if let Some(color) = colour(raw, &["backgroundColor", "background"]) {
        let transparency = raw
            .number("backgroundTransparency")
            .or_else(|| raw.number("transparency"));
        // An `rgba` colour carries its own opacity.
        style.fill.color = match transparency {
            Some(transparency) if color[3] == 255 => with_transparency(color, transparency),
            _ => color,
        };
        if let Some(enabled) = raw
            .flag("fillBackground")
            .or_else(|| raw.flag("drawBackground"))
        {
            style.fill.enabled = enabled;
        }
    }
    let text_keys: &[&str] = if writes && tool != Tool::PriceLabel {
        &["textColor", "textcolor", "color"]
    } else {
        &["textColor", "textcolor"]
    };
    if let Some(color) = colour(raw, text_keys) {
        style.text.color = color;
    }
    if tool == Tool::PriceLabel
        && let Some(color) = colour(raw, &["color"])
    {
        style.text.color = color;
    }
    if let Some(size) = raw.number("fontsize").or_else(|| raw.number("fontSize")) {
        style.text.size = size as f32;
    }
    if let Some(bold) = raw.flag("bold") {
        style.text.bold = bold;
    }
    if let Some(italic) = raw.flag("italic") {
        style.text.italic = italic;
    }
    if let Some(color) = colour(raw, &["borderColor"]) {
        style.border.color = color;
        style.border.enabled = raw.flag("drawBorder").unwrap_or(style.border.enabled);
    }
    if let Some(extend) = raw
        .flag("extendLeft")
        .or_else(|| raw.flag("extendLinesLeft"))
    {
        style.extend_left = extend;
    }
    if let Some(extend) = raw.flag("extendRight").or_else(|| raw.flag("extendLines")) {
        style.extend_right = extend;
    }
    // TradingView numbers a line end: 0 plain, 1 arrow.
    for (key, end) in [
        ("leftEnd", &mut style.head_start),
        ("rightEnd", &mut style.head_end),
    ] {
        if let Some(code) = raw.props.get(key).and_then(Value::as_i64) {
            *end = if code == 1 {
                LineEnd::Arrow
            } else {
                LineEnd::Plain
            };
        }
    }
    if let Some(shown) = raw.flag("showPriceLabels") {
        style.price_labels = shown;
    }
    if let Some(shown) = raw.flag("showMiddlePoint") {
        style.middle_point = shown;
    }
    if matches!(tool, Tool::LongPosition | Tool::ShortPosition) {
        position_options(raw, &mut style);
    }
    if matches!(tool, Tool::FibRetracement | Tool::FibExtension) {
        fib(raw, &mut style);
    }
    style.sanitized()
}

fn position_options(raw: &RawDrawing, style: &mut DrawingStyle) {
    let options = &mut style.position;
    if let Some(account) = raw.number("accountSize") {
        options.account = account;
    }
    if let Some(risk) = raw.number("risk") {
        options.risk = risk;
        options.risk_percent = raw.str("riskDisplayMode") != Some("money");
    }
    if let Some(lot) = raw.number("lotSize") {
        options.lot = lot;
    }
    if let Some(color) = colour(raw, &["profitBackground"]) {
        options.profit = color;
    }
    if let Some(color) = colour(raw, &["stopBackground"]) {
        options.stop = color;
    }
    if let Some(compact) = raw.flag("compact") {
        options.compact = compact;
    }
    if let Some(always) = raw.flag("alwaysShowStats") {
        options.always_stats = always;
    }
}

/// A retracement's own levels: `level1`…`level24`, as `[ratio, colour, shown]` or as an object.
fn fib(raw: &RawDrawing, style: &mut DrawingStyle) {
    let levels: Vec<FibLevel> = (1..=24)
        .filter_map(|index| {
            let value = raw.props.get(&format!("level{index}"))?;
            let (ratio, color, shown) = match value {
                Value::Array(parts) => (
                    parts.first()?.as_f64()?,
                    parts.get(1).and_then(Value::as_str),
                    parts.get(2).and_then(Value::as_bool),
                ),
                level => (
                    level.get("coeff")?.as_f64()?,
                    level.get("color").and_then(Value::as_str),
                    level.get("visible").and_then(Value::as_bool),
                ),
            };
            Some(FibLevel {
                ratio: ratio as f32,
                enabled: shown.unwrap_or(true),
                color: color.and_then(color::parse).unwrap_or(style.stroke.color),
                ..FibLevel::default()
            })
        })
        .collect();
    let Some(mut own) = style.fib else {
        return;
    };
    if !levels.is_empty() {
        own.levels = Levels::from_levels(&levels);
    }
    if let Some(reverse) = raw.flag("reverse") {
        own.reverse = reverse;
    }
    if let Some(shown) = raw.flag("showCoeffs") {
        own.show_labels = shown;
    }
    if let Some(shown) = raw.flag("showPrices") {
        own.show_prices = shown;
    }
    if let Some(percent) = raw.flag("coeffsAsPercents") {
        own.as_percent = percent;
    }
    style.fib = Some(own);
}

/// Entry, target and stop. TradingView keeps the distances in price steps; without the step they
/// follow from its own risk figures (the money lost at the stop over the quantity).
fn position(
    raw: &RawDrawing,
    tool: Tool,
    anchors: &[Anchor],
    tick: Option<f64>,
) -> Result<[Anchor; 3], Unsupported> {
    let [entry, end, ..] = anchors else {
        return Err(Unsupported::Points);
    };
    let side = if tool == Tool::LongPosition {
        1.0
    } else {
        -1.0
    };
    let steps = |key: &str| raw.number(key).filter(|value| *value > 0.0);
    let (stop_steps, profit_steps) = steps("stopLevel")
        .zip(steps("profitLevel"))
        .ok_or(Unsupported::Points)?;
    let (stop, target) = match tick {
        Some(tick) => (stop_steps * tick, profit_steps * tick),
        None => {
            let quantity =
                steps("qty").ok_or(Unsupported::PriceStep)? * steps("lotSize").unwrap_or(1.0);
            let rate = |key: &str| raw.number(key).is_none_or(|rate| rate == 1.0);
            let account = steps("accountSize").ok_or(Unsupported::PriceStep)?;
            let at_stop = raw.number("amountStop").ok_or(Unsupported::PriceStep)?;
            if !rate("entryPointCurrencyRate") || !rate("closePointCurrencyRate") {
                return Err(Unsupported::PriceStep);
            }
            let stop = (account - at_stop) / quantity;
            if !(stop.is_finite() && stop > 0.0) {
                return Err(Unsupported::PriceStep);
            }
            (stop, stop / stop_steps * profit_steps)
        }
    };
    Ok([
        *entry,
        Anchor {
            time_ms: end.time_ms,
            price: entry.price + side * target,
        },
        Anchor {
            time_ms: end.time_ms,
            price: entry.price - side * stop,
        },
    ])
}

fn table(raw: &RawDrawing, at: Anchor) -> TableData {
    let mut table = TableData::new(at);
    let rows: Vec<Vec<&str>> = raw
        .props
        .get("cells")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    row.as_array()
                        .map(|cells| cells.iter().map(|c| c.as_str().unwrap_or("")).collect())
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns > 0 {
        let sizes = |key: &str, count: usize, fallback: f32| -> Vec<f32> {
            let own = raw.props.get(key).and_then(Value::as_array);
            (0..count)
                .map(|index| {
                    own.and_then(|sizes| sizes.get(index))
                        .and_then(Value::as_f64)
                        .filter(|size| *size > 0.0)
                        .map_or(fallback, |size| size as f32)
                })
                .collect()
        };
        table.columns = sizes("columnWidths", columns, table.columns[0]);
        table.rows = sizes("rowHeights", rows.len(), table.rows[0]);
        table.cells = vec![String::new(); columns * rows.len()];
        for (row, cells) in rows.iter().enumerate() {
            for (column, text) in cells.iter().enumerate() {
                table.set_cell(row, column, text);
            }
        }
    }
    table
}

fn post(
    raw: &RawDrawing,
    id: u64,
    at: Anchor,
    style: DrawingStyle,
) -> Result<Drawing, Unsupported> {
    let link = raw
        .post_url
        .as_deref()
        .and_then(|url| Link::parse(url).ok())
        .ok_or(Unsupported::Points)?;
    Drawing::build_with(
        Tool::Post,
        id,
        &[at],
        style,
        &Content::Link(LinkCard::new(link)),
    )
    .ok_or(Unsupported::Points)
}

#[cfg(test)]
mod tests;
