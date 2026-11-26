//! TradingView's drawings as DegenView's. One table says which tool each class is; the points are
//! TradingView's placement points, which DegenView's tools are built from too, and the look is
//! carried over where both have the setting.

use super::color::{self, with_transparency};
use super::drawing::RawDrawing;
use crate::model::annotation::image_content::Content;
use crate::model::annotation::link_card::{Link, LinkCard};
use crate::model::annotation::table::{
    COLUMN_WIDTH_RANGE, MAX_CELL_CHARS, MAX_COLUMNS, MAX_ROWS, ROW_HEIGHT_RANGE, TableData,
};
use crate::model::chart::kind::{LineDash, PriceSource, Rgba};
use crate::model::drawings::Geometry;
use crate::model::drawings::style::{
    DrawingStyle, HorizontalAlign, LineEnd, StatsPosition, VerticalAlign, Visibility, VisibilityRow,
};
use crate::model::fib::style::{FibLevel, FibStyle, Levels};
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
    /// Pinned to a place on the screen, which DegenView's drawings are not.
    Pinned,
    /// A drawing beside a bar whose price has not been worked out ([`RawDrawing::at_price`]).
    Unplaced,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    pub drawing: Drawing,
    /// Text, or a table, was bigger than a DegenView drawing holds.
    pub cut_text: bool,
    /// A Fibonacci or Gann drawing whose levels are DegenView's, not TradingView's.
    pub default_levels: bool,
    /// An anchored VWAP whose bands are percentages, which DegenView's are not: they are off.
    pub percent_bands: bool,
}

/// TradingView's class name to the DegenView tool. Its Signpost and Three drives are not here:
/// how their stored points make ours is not known.
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
    ("LineToolFlagMark", Tool::FlagMark),
    ("LineToolTable", Tool::Table),
    ("LineToolTweet", Tool::Post),
    ("LineTool5PointsPattern", Tool::XabcdPattern),
    ("LineToolCypherPattern", Tool::CypherPattern),
    ("LineToolHeadAndShoulders", Tool::HeadAndShoulders),
    ("LineToolABCD", Tool::AbcdPattern),
    ("LineToolTrianglePattern", Tool::TrianglePattern),
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
    // A pinned drawing's points are not where it is.
    if raw.flag("anchored") == Some(true) {
        return Err(Unsupported::Pinned);
    }
    if !raw.is_priced() {
        return Err(Unsupported::Unplaced);
    }
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
    let mut cut_table = false;
    let drawing = match tool {
        Tool::Post => post(raw, id, first, style)?,
        Tool::Table => {
            let (table, cut) = table(raw, first);
            cut_table = cut;
            Drawing::from_geometry(id, Geometry::Table(table), style).ok_or(Unsupported::Points)?
        }
        Tool::LongPosition | Tool::ShortPosition => {
            let anchors = position(raw, tool, &anchors, tick)?;
            Drawing::build(tool, id, &anchors, style).ok_or(Unsupported::Points)?
        }
        _ => Drawing::build(tool, id, &anchors, style).ok_or(Unsupported::Points)?,
    };
    let mut converted = finish(raw, drawing);
    converted.cut_text |= cut_table;
    // A retracement's and an extension's levels are read; the others' are stored in ways not
    // known here.
    converted.default_levels = FibStyle::for_tool(tool).is_some()
        && !matches!(tool, Tool::FibRetracement | Tool::FibExtension);
    converted.percent_bands = tool == Tool::AnchoredVwap && has_percent_bands(raw);
    Ok(converted)
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
    Converted {
        drawing,
        cut_text,
        default_levels: false,
        percent_bands: false,
    }
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
    if let Some(bold) = raw
        .flag("bold")
        .or_else(|| raw.str("fontWeight").map(|weight| weight == "bold"))
    {
        style.text.bold = bold;
    }
    if let Some(italic) = raw.flag("italic") {
        style.text.italic = italic;
    }
    if let Some(wrap) = raw.flag("wordWrap") {
        style.wrap = wrap;
        if let Some(width) = raw.number("wordWrapWidth") {
            style.box_width = width as f32;
        }
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
    if matches!(
        tool,
        Tool::TrendLine
            | Tool::Ray
            | Tool::InfoLine
            | Tool::ExtendedLine
            | Tool::TrendAngle
            | Tool::HorizontalLine
            | Tool::HorizontalRay
    ) {
        line_label(raw, &mut style);
        line_stats(raw, &mut style);
    }
    if tool == Tool::Table
        && let Some(align) = raw.str("horzAlign").and_then(horizontal)
    {
        style.text.align = align;
    }
    if tool == Tool::AnchoredVwap {
        vwap(raw, &mut style);
    }
    if matches!(tool, Tool::LongPosition | Tool::ShortPosition) {
        position_options(raw, &mut style);
    }
    if matches!(tool, Tool::FibRetracement | Tool::FibExtension) {
        fib(raw, &mut style);
    }
    style.sanitized()
}

fn horizontal(name: &str) -> Option<HorizontalAlign> {
    match name {
        "left" => Some(HorizontalAlign::Left),
        "center" => Some(HorizontalAlign::Center),
        "right" => Some(HorizontalAlign::Right),
        _ => None,
    }
}

/// Where a line's words sit on it. TradingView names the edge of the words that meets the line,
/// so its `bottom` is above the line; DegenView names the side of the line.
fn line_label(raw: &RawDrawing, style: &mut DrawingStyle) {
    if let Some(align) = raw.str("horzLabelsAlign").and_then(horizontal) {
        style.text.align = align;
    }
    match raw.str("vertLabelsAlign") {
        Some("bottom") => style.text.valign = VerticalAlign::Top,
        Some("middle") => style.text.valign = VerticalAlign::Middle,
        Some("top") => style.text.valign = VerticalAlign::Bottom,
        _ => {}
    }
}

/// The measurements a line prints beside itself.
fn line_stats(raw: &RawDrawing, style: &mut DrawingStyle) {
    let stats = &mut style.stats;
    for (key, shown) in [
        ("showPriceRange", &mut stats.price_range),
        ("showPercentPriceRange", &mut stats.percent),
        ("showBarsRange", &mut stats.bars),
        ("showDateTimeRange", &mut stats.time),
        ("showDistance", &mut stats.distance),
        ("showAngle", &mut stats.angle),
        ("alwaysShowStats", &mut stats.always_show),
    ] {
        if let Some(value) = raw.flag(key) {
            *shown = value;
        }
    }
    // TradingView's fourth, "auto", has no counterpart: the tool's own place stands.
    match raw.props.get("statsPosition").and_then(Value::as_i64) {
        Some(0) => stats.position = StatsPosition::Left,
        Some(1) => stats.position = StatsPosition::Center,
        Some(2) => stats.position = StatsPosition::Right,
        _ => {}
    }
}

fn has_percent_bands(raw: &RawDrawing) -> bool {
    let Some(inputs) = raw.props.get("inputs") else {
        return false;
    };
    let on = |key: &str| inputs.get(key).and_then(Value::as_bool) == Some(true);
    inputs.get("Bands Calculation Mode").and_then(Value::as_str) == Some("Percentage")
        && (on("calculate_stDev") || on("calculate_stDev_2") || on("calculate_stDev_3"))
}

/// An anchored VWAP, which TradingView keeps as a study: its settings are `inputs` and the look of
/// each of its plots is under `styles`.
fn vwap(raw: &RawDrawing, style: &mut DrawingStyle) {
    let input = |key: &str| raw.props.get("inputs").and_then(|inputs| inputs.get(key));
    let plot = |name: &str| raw.props.get("styles").and_then(|styles| styles.get(name));
    let color_of = |plot: &Value| {
        let color = color::parse(plot.get("color")?.as_str()?)?;
        Some(match plot.get("transparency").and_then(Value::as_f64) {
            Some(transparency) if color[3] == 255 => with_transparency(color, transparency),
            _ => color,
        })
    };
    if let Some(line) = plot("VWAP") {
        if let Some(color) = color_of(line) {
            style.stroke.color = color;
        }
        if let Some(width) = line.get("linewidth").and_then(Value::as_f64) {
            style.stroke.width = width as f32;
        }
        if let Some(code) = line.get("linestyle").and_then(Value::as_i64) {
            style.stroke.dash = dash(code);
        }
    }
    if let Some(source) = input("source").and_then(Value::as_str) {
        style.vwap.source = match source {
            "open" => PriceSource::Open,
            "high" => PriceSource::High,
            "low" => PriceSource::Low,
            "close" => PriceSource::Close,
            "hl2" => PriceSource::Hl2,
            "hlc3" => PriceSource::Hlc3,
            "ohlc4" => PriceSource::Ohlc4,
            _ => style.vwap.source,
        };
    }
    let percent = has_percent_bands(raw);
    for (band, suffix) in style.vwap.bands.iter_mut().zip(["", "_2", "_3"]) {
        if let Some(on) = input(&format!("calculate_stDev{suffix}")).and_then(Value::as_bool) {
            band.enabled = on && !percent;
        }
        if let Some(multiplier) =
            input(&format!("bands_multiplier{suffix}")).and_then(Value::as_f64)
        {
            band.multiplier = multiplier as f32;
        }
        if let Some(color) = plot(&format!("UpperBand{suffix}")).and_then(color_of) {
            band.color = color;
        }
    }
    if let Some(fill) = raw
        .props
        .get("areaBackground")
        .and_then(|area| area.get("fillBackground"))
        .and_then(Value::as_bool)
    {
        style.vwap.fill = fill;
    }
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

/// The table within a DegenView table's limits, and whether anything did not fit.
fn table(raw: &RawDrawing, at: Anchor) -> (TableData, bool) {
    let mut table = TableData::new(at);
    let mut cut = false;
    let stored: &[Value] = raw
        .props
        .get("cells")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let cells_of = |row: &Value| row.as_array().map_or(0, Vec::len);
    let widest = stored
        .iter()
        .take(MAX_ROWS)
        .map(cells_of)
        .max()
        .unwrap_or(0);
    if widest == 0 {
        return (table, cut);
    }
    let (rows, columns) = (stored.len().min(MAX_ROWS), widest.min(MAX_COLUMNS));
    cut |= stored.len() > MAX_ROWS || widest > MAX_COLUMNS;
    let sizes = |key: &str, count: usize, fallback: f32, (low, high): (f32, f32)| -> Vec<f32> {
        let own = raw.props.get(key).and_then(Value::as_array);
        (0..count)
            .map(|index| {
                own.and_then(|sizes| sizes.get(index))
                    .and_then(Value::as_f64)
                    .filter(|size| *size > 0.0)
                    .map_or(fallback, |size| (size as f32).clamp(low, high))
            })
            .collect()
    };
    table.columns = sizes(
        "columnWidths",
        columns,
        table.columns[0],
        COLUMN_WIDTH_RANGE,
    );
    table.rows = sizes("rowHeights", rows, table.rows[0], ROW_HEIGHT_RANGE);
    table.cells = vec![String::new(); columns * rows];
    for (row, cells) in stored.iter().take(rows).enumerate() {
        let cells = cells.as_array().map_or(&[][..], Vec::as_slice);
        for (column, text) in cells.iter().take(columns).enumerate() {
            let text = text.as_str().unwrap_or("");
            cut |= text.chars().count() > MAX_CELL_CHARS;
            table.set_cell(row, column, text);
        }
    }
    (table, cut)
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
