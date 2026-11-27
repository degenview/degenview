//! TradingView's drawings as DegenView's. One table says which tool each class is; the points are
//! TradingView's placement points, which DegenView's tools are built from too, and the look is
//! carried over where both have the setting.

use super::drawing::RawDrawing;
use crate::model::annotation::image_content::Content;
use crate::model::annotation::link_card::{Link, LinkCard, LinkMeta};
use crate::model::annotation::table::{
    COLUMN_WIDTH_RANGE, MAX_CELL_CHARS, MAX_COLUMNS, MAX_ROWS, ROW_HEIGHT_RANGE, TableData,
};
use crate::model::drawings::Geometry;
use crate::model::drawings::style::{DrawingStyle, Visibility, VisibilityRow};
use crate::model::fib::style::FibStyle;
use crate::model::{Anchor, Drawing, Tool};
use serde_json::Value;
use style::{has_percent_bands, style, words};

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

/// A pitchfork's kind is also one of its settings, which wins over the class it was drawn as.
fn variant(raw: &RawDrawing, tool: Tool) -> Tool {
    use Tool::{InsidePitchfork, ModifiedSchiffPitchfork, Pitchfork, SchiffPitchfork};
    if !matches!(
        tool,
        Pitchfork | SchiffPitchfork | ModifiedSchiffPitchfork | InsidePitchfork
    ) {
        return tool;
    }
    // TradingView's `LineToolPitchforkStyle`: Original, Schiff, Inside, Schiff2.
    match raw.props.get("style").and_then(Value::as_i64) {
        Some(0) => Pitchfork,
        Some(1) => ModifiedSchiffPitchfork,
        Some(2) => InsidePitchfork,
        Some(3) => SchiffPitchfork,
        _ => tool,
    }
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
    let tool = variant(raw, tool_of(&raw.tool).ok_or(Unsupported::Tool)?);
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
    // The speed fan's and the Gann tools' levels are stored in ways whose match with ours is not
    // known; the spiral has none.
    converted.default_levels =
        FibStyle::for_tool(tool).is_some() && !style::reads_levels(tool) && tool != Tool::FibSpiral;
    converted.percent_bands = tool == Tool::AnchoredVwap && has_percent_bands(raw);
    Ok(converted)
}

/// The switches every drawing has, and its words.
fn finish(raw: &RawDrawing, mut drawing: Drawing) -> Converted {
    let text = words(raw);
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
    let post = raw.post.as_ref().ok_or(Unsupported::Points)?;
    let link = Link::parse(&post.url).map_err(|_| Unsupported::Points)?;
    let mut card = LinkCard::new(link);
    // What TradingView showed on its card, so ours shows it too without asking X.
    let meta = LinkMeta {
        title: post.name.clone(),
        author: post.handle.clone(),
        excerpt: post.text.clone(),
        ..Default::default()
    }
    .sanitized();
    card.meta = (!meta.is_empty()).then_some(meta);
    Drawing::build_with(Tool::Post, id, &[at], style, &Content::Link(card))
        .ok_or(Unsupported::Points)
}

mod style;
#[cfg(test)]
mod tests;
