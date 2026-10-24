//! What the candle and drawing tools say about a chart, and the plans for changing its drawings.
//!
//! Pure functions over plain values: the app thread hands in the chart's candles and drawings and
//! gets back a result to serialize or the drawing commands to apply. Nothing here touches the
//! workspace, egui or the store, so each rule is tested without the app.

use crate::format::civil_from_days;
use crate::mcp::bridge::McpError;
use crate::mcp::drawings::{
    AnchorSpec, DrawingEdit, MAX_DRAWINGS_PER_CHART, McpDrawingTool, NewDrawing, PriceField,
    PriceSpec, StylePatch,
};
use crate::mcp::types::{
    CandleRange, CandleRow, CandleSummary, CandlesResult, DrawingAnchorInfo, DrawingInfo,
    DrawingStyleInfo, DrawingsResult,
};
use crate::model::chart_type::{LineDash, Rgba};
use crate::model::drawing_style::DrawingStyle;
use crate::model::{Anchor, ChartType, Drawing, DrawingCommand, InstrumentId, Kline, Tool};

/// Bars returned when the caller does not say.
pub const DEFAULT_CANDLES: usize = 200;
/// The most bars one call returns.
pub const MAX_CANDLES: usize = 1000;
/// Drawings listed at most.
const MAX_LISTED: usize = 500;
/// Points shown for one drawing.
const MAX_ANCHORS_SHOWN: usize = 20;

const COLUMNS: [&str; 6] = ["time", "open", "high", "low", "close", "volume"];

/// ISO-8601 UTC to the second: `2026-10-20T00:00:00Z`. The form the tools take back as a `time`.
pub fn iso(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let clock = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        clock / 3_600,
        clock % 3_600 / 60,
        clock % 60
    )
}

// MARK: Candles

/// What the candles belong to.
pub struct CandleMeta {
    pub workspace_id: u64,
    pub chart_id: u64,
    pub instrument: InstrumentId,
    /// The workspace timeframe's label (`1D`).
    pub timeframe: String,
    pub chart_type: ChartType,
    /// Whether the market can serve older history than the chart holds.
    pub older_supported: bool,
    /// The replay position, when replay hides the bars after it.
    pub replay_ms: Option<i64>,
    pub now_ms: i64,
}

/// Whether the chart draws bars built from the candles (Heikin Ashi, Renko, …) rather than the
/// candles themselves.
fn builds_its_own_bars(chart_type: ChartType) -> bool {
    matches!(
        chart_type,
        ChartType::HeikinAshi
            | ChartType::Renko
            | ChartType::Kagi
            | ChartType::LineBreak
            | ChartType::PointAndFigure
            | ChartType::Range
    )
}

/// The newest `limit` bars that start before `before` (or the newest overall), as an agent wants
/// to read them.
pub fn candles_result(
    meta: &CandleMeta,
    candles: &[Kline],
    limit: usize,
    before: Option<i64>,
) -> CandlesResult {
    let limit = limit.clamp(1, MAX_CANDLES);
    let end = before.map_or(candles.len(), |before| {
        candles.partition_point(|bar| bar.open_time < before)
    });
    let start = end.saturating_sub(limit);
    let page = &candles[start..end];
    let provides_volume = meta.instrument.source.provides_volume();
    let bars: Vec<CandleRow> = page
        .iter()
        .map(|bar| {
            (
                iso(bar.open_time),
                bar.open,
                bar.high,
                bar.low,
                bar.close,
                (provides_volume && bar.volume.is_finite()).then_some(bar.volume),
            )
        })
        .collect();
    let summary = page.first().zip(page.last()).map(|(first, last)| {
        let high = page.iter().map(|bar| bar.high).fold(f64::MIN, f64::max);
        let low = page.iter().map(|bar| bar.low).fold(f64::MAX, f64::min);
        CandleSummary {
            high,
            low,
            first_open: first.open,
            last_close: last.close,
            change_percent: (first.open != 0.0)
                .then(|| ((last.close / first.open - 1.0) * 10_000.0).round() / 100.0),
        }
    });
    let mut notes = Vec::new();
    if builds_its_own_bars(meta.chart_type) {
        notes.push(format!(
            "The chart draws {} bars built from these candles. These are the market's own candles, \
             which is what drawings are anchored to.",
            meta.chart_type.slug()
        ));
    }
    if let Some(replay) = meta.replay_ms {
        notes.push(format!(
            "Replay is on: bars after {} are hidden, as on the chart.",
            iso(replay)
        ));
    }
    if end == candles.len() && candles.is_empty() {
        notes.push("The chart has no candles yet; it may still be loading.".to_owned());
    }
    CandlesResult {
        workspace_id: meta.workspace_id.to_string(),
        chart_id: meta.chart_id.to_string(),
        instrument: meta.instrument.key(),
        timeframe: meta.timeframe.clone(),
        chart_type: meta.chart_type.slug().to_owned(),
        columns: COLUMNS.iter().map(|name| (*name).to_owned()).collect(),
        count: bars.len(),
        loaded_bars: candles.len(),
        range: page
            .first()
            .zip(page.last())
            .map(|(first, last)| CandleRange {
                from: iso(first.open_time),
                to: iso(last.open_time),
            }),
        summary,
        has_older_loaded: start > 0,
        older_history_available: meta.older_supported,
        last_bar_forming: end == candles.len()
            && page.last().is_some_and(|bar| bar.close_time > meta.now_ms),
        notes,
        bars,
    }
}

// MARK: Listing drawings

fn hex(color: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color[0], color[1], color[2], color[3]
    )
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn style_info(style: &DrawingStyle) -> DrawingStyleInfo {
    DrawingStyleInfo {
        color: hex(style.stroke.color),
        width: f64::from(style.stroke.width),
        dash: match style.stroke.dash {
            LineDash::Solid => "solid",
            LineDash::Dashed => "dashed",
            LineDash::Dotted => "dotted",
        }
        .to_owned(),
        extend_left: style.extend_left,
        extend_right: style.extend_right,
        fill_color: style.fill.enabled.then(|| hex(style.fill.color)),
    }
}

/// Whether this server may change `drawing`: a trend line or rectangle, and not locked.
fn mcp_tool(tool: Option<Tool>) -> Option<McpDrawingTool> {
    match tool? {
        Tool::TrendLine => Some(McpDrawingTool::TrendLine),
        Tool::Rectangle => Some(McpDrawingTool::Rectangle),
        _ => None,
    }
}

/// One drawing as a client sees it. `None` for the transient ruler and for rows this version
/// cannot read.
pub fn drawing_info(drawing: &Drawing) -> Option<DrawingInfo> {
    if drawing.is_ruler() {
        return None;
    }
    let tool = drawing.tool()?;
    let anchors = drawing.anchors();
    Some(DrawingInfo {
        drawing_id: drawing.id().to_string(),
        tool: snake(&format!("{tool:?}")),
        anchor_count: anchors.len(),
        anchors: anchors
            .iter()
            .take(MAX_ANCHORS_SHOWN)
            .map(|anchor| DrawingAnchorInfo {
                time: iso(anchor.time_ms),
                price: anchor.price,
            })
            .collect(),
        style: drawing.style().map(style_info),
        text: Some(drawing.text())
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        locked: drawing.locked(),
        hidden: drawing.hidden(),
        editable: mcp_tool(Some(tool)).is_some() && !drawing.locked(),
    })
}

pub fn drawings_result(workspace_id: u64, chart_id: u64, drawings: &[Drawing]) -> DrawingsResult {
    let infos: Vec<DrawingInfo> = drawings.iter().filter_map(drawing_info).collect();
    DrawingsResult {
        workspace_id: workspace_id.to_string(),
        chart_id: chart_id.to_string(),
        total: infos.len(),
        truncated: infos.len() > MAX_LISTED,
        unreadable: drawings
            .iter()
            .filter(|drawing| matches!(drawing, Drawing::Unreadable(_)))
            .count(),
        drawings: infos.into_iter().take(MAX_LISTED).collect(),
    }
}

/// Drawings counted against the per-chart cap: everything but the transient ruler.
pub fn counted(drawings: &[Drawing]) -> usize {
    drawings
        .iter()
        .filter(|drawing| !drawing.is_ruler())
        .count()
}

// MARK: Placing and changing drawings

fn invalid(message: impl Into<String>) -> McpError {
    McpError::InvalidArgument(message.into())
}

/// The bar a time falls in, when the candles cover it: the last bar that starts at or before it,
/// unless it is past the end of the newest bar.
fn bar_at(candles: &[Kline], time_ms: i64) -> Option<&Kline> {
    let index = candles.partition_point(|bar| bar.open_time <= time_ms);
    let bar = candles.get(index.checked_sub(1)?)?;
    (index < candles.len() || time_ms <= bar.close_time.max(bar.open_time)).then_some(bar)
}

/// Places a checked anchor on the chart. A time inside the candles snaps to its bar's open (as
/// the pointer's magnet does) and `price_from` reads that bar; a time outside them is kept.
pub fn resolve_anchor(
    spec: &AnchorSpec,
    candles: &[Kline],
    name: &str,
) -> Result<Anchor, McpError> {
    let bar = bar_at(candles, spec.time_ms);
    let price = match spec.price {
        PriceSpec::Value(price) => price,
        PriceSpec::Field(field) => {
            let bar = bar.ok_or_else(|| {
                let covered = candles.first().zip(candles.last()).map_or_else(
                    || "the chart has no candles loaded".to_owned(),
                    |(first, last)| {
                        format!(
                            "the loaded candles run from {} to {}",
                            iso(first.open_time),
                            iso(last.open_time)
                        )
                    },
                );
                invalid(format!(
                    "Point {name}: price_from needs a time inside the loaded candles; {covered}"
                ))
            })?;
            match field {
                PriceField::Open => bar.open,
                PriceField::High => bar.high,
                PriceField::Low => bar.low,
                PriceField::Close => bar.close,
            }
        }
    };
    if !price.is_finite() || price <= 0.0 {
        return Err(invalid(format!(
            "Point {name}: the price is not above zero"
        )));
    }
    Ok(Anchor {
        time_ms: bar.map_or(spec.time_ms, |bar| bar.open_time),
        price,
    })
}

fn tool_of(tool: McpDrawingTool) -> Tool {
    match tool {
        McpDrawingTool::TrendLine => Tool::TrendLine,
        McpDrawingTool::Rectangle => Tool::Rectangle,
    }
}

/// Applies a client's style changes to `style` for a drawing by `tool`.
fn apply_patch(
    style: &mut DrawingStyle,
    patch: &StylePatch,
    tool: McpDrawingTool,
) -> Result<(), McpError> {
    let rectangle = tool == McpDrawingTool::Rectangle;
    if !rectangle && (patch.fill_color.is_some() || patch.text.is_some()) {
        return Err(invalid(
            "fill_color and text apply to rectangles, not trend lines",
        ));
    }
    if let Some(color) = patch.color {
        style.stroke.color = color;
        if rectangle {
            style.text.color = color;
            style.middle_line.color = color;
            if style.fill.enabled {
                style.fill.color = [color[0], color[1], color[2], style.fill.color[3]];
            }
        }
    }
    if let Some(width) = patch.width {
        style.stroke.width = width;
    }
    if let Some(dash) = patch.dash {
        style.stroke.dash = dash;
    }
    if let Some(extend) = patch.extend_left {
        style.extend_left = extend;
    }
    if let Some(extend) = patch.extend_right {
        style.extend_right = extend;
    }
    if let Some(fill) = patch.fill_color {
        style.fill.enabled = true;
        style.fill.color = fill;
    }
    Ok(())
}

/// A box with no width or no height cannot be seen, so it is not a rectangle.
fn check_extent(tool: McpDrawingTool, a: Anchor, b: Anchor) -> Result<(), McpError> {
    if a == b {
        return Err(invalid("The two points must differ"));
    }
    if tool == McpDrawingTool::Rectangle && (a.time_ms == b.time_ms || a.price == b.price) {
        return Err(invalid(
            "A rectangle needs corners at different times and different prices",
        ));
    }
    Ok(())
}

/// The id a new drawing takes: the chart's counter, or past the highest id already stored
/// (rows this version cannot read included), whichever is larger. The counter normally leads, but
/// an id must never repeat, so the drawings are the final word.
pub fn next_free_id(drawings: &[Drawing], counter: u64) -> u64 {
    drawings
        .iter()
        .map(|drawing| drawing.stored_id().saturating_add(1))
        .fold(counter.max(1), u64::max)
}

/// The drawings for `specs`, with ids from `first_id`. All or none: one bad spec refuses the call.
pub fn plan_add(
    existing: &[Drawing],
    candles: &[Kline],
    first_id: u64,
    specs: &[NewDrawing],
) -> Result<Vec<Drawing>, McpError> {
    if counted(existing) + specs.len() > MAX_DRAWINGS_PER_CHART {
        return Err(McpError::ResourceLimit(format!(
            "A chart holds at most {MAX_DRAWINGS_PER_CHART} drawings; this one has {}",
            counted(existing)
        )));
    }
    specs
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let label = |error: McpError| match error {
                McpError::InvalidArgument(message) => {
                    invalid(format!("Drawing {}: {message}", index + 1))
                }
                other => other,
            };
            let (a, b) = (
                resolve_anchor(&spec.a, candles, "a").map_err(label)?,
                resolve_anchor(&spec.b, candles, "b").map_err(label)?,
            );
            check_extent(spec.tool, a, b).map_err(label)?;
            let tool = tool_of(spec.tool);
            let mut style = tool.default_style();
            apply_patch(&mut style, &spec.patch, spec.tool).map_err(label)?;
            let mut drawing = Drawing::build(tool, first_id + index as u64, &[a, b], style)
                .ok_or_else(|| label(invalid("The points do not make that drawing")))?;
            if let Some(text) = &spec.patch.text {
                drawing.set_text(text);
            }
            Ok(drawing)
        })
        .collect()
}

/// The command that applies `edit`, or `None` when it would change nothing.
pub fn plan_edit(
    existing: &[Drawing],
    candles: &[Kline],
    edit: &DrawingEdit,
) -> Result<Option<DrawingCommand>, McpError> {
    let current = existing
        .iter()
        .find(|drawing| {
            drawing.id() == edit.drawing_id && !drawing.is_ruler() && drawing.tool().is_some()
        })
        .ok_or_else(|| {
            McpError::NotFound(format!(
                "No drawing {} on this chart (see degenview_list_drawings)",
                edit.drawing_id
            ))
        })?;
    let tool = mcp_tool(current.tool()).ok_or_else(|| {
        McpError::Unsupported("Only trend lines and rectangles can be changed over MCP".to_owned())
    })?;
    if current.locked() {
        return Err(McpError::Conflict(format!(
            "Drawing {} is locked; unlock it in DegenView first",
            edit.drawing_id
        )));
    }
    let mut next = current.clone();
    if edit.a.is_some() || edit.b.is_some() {
        let (a, b) = (
            edit.a
                .map(|spec| resolve_anchor(&spec, candles, "a"))
                .transpose()?,
            edit.b
                .map(|spec| resolve_anchor(&spec, candles, "b"))
                .transpose()?,
        );
        let mut points = next.anchors_mut();
        if let (Some(a), Some(slot)) = (a, points.get_mut(0)) {
            **slot = a;
        }
        if let (Some(b), Some(slot)) = (b, points.get_mut(1)) {
            **slot = b;
        }
        if let [first, second, ..] = next.anchors()[..] {
            check_extent(tool, first, second)?;
        }
    }
    if let Some(style) = next.style().copied() {
        let mut style = style;
        apply_patch(&mut style, &edit.patch, tool)?;
        next.set_style(style);
    } else if !edit.patch.is_empty() {
        return Err(McpError::Unsupported(
            "This drawing has no style to change".to_owned(),
        ));
    }
    if let Some(text) = &edit.patch.text {
        next.set_text(text);
    }
    if let Some(hidden) = edit.hidden {
        next.set_hidden(hidden);
    }
    Ok((next != *current).then_some(DrawingCommand::Replace {
        id: edit.drawing_id,
        drawing: next,
    }))
}

/// Checks `ids` can all be removed: each exists and none is locked. All or none.
pub fn plan_remove(existing: &[Drawing], ids: &[u64]) -> Result<Vec<DrawingCommand>, McpError> {
    let mut seen = Vec::new();
    for id in ids {
        if seen.contains(id) {
            continue;
        }
        let drawing = existing
            .iter()
            .find(|drawing| drawing.id() == *id && !drawing.is_ruler() && drawing.tool().is_some())
            .ok_or_else(|| {
                McpError::NotFound(format!(
                    "No drawing {id} on this chart (see degenview_list_drawings)"
                ))
            })?;
        if drawing.locked() {
            return Err(McpError::Conflict(format!(
                "Drawing {id} is locked; unlock it in DegenView first. Nothing was removed"
            )));
        }
        seen.push(*id);
    }
    Ok(seen.into_iter().map(DrawingCommand::Remove).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::drawings::StylePatch;
    use crate::model::DataSource;

    const DAY: i64 = 86_400_000;
    /// 2025-10-20T00:00:00Z.
    const T0: i64 = 1_760_918_400_000;

    fn bar(day: i64, open: f64, high: f64, low: f64, close: f64) -> Kline {
        Kline {
            open_time: T0 + day * DAY,
            close_time: T0 + (day + 1) * DAY - 1,
            open,
            high,
            low,
            close,
            volume: 10.0 + day as f64,
        }
    }

    fn bars(count: i64) -> Vec<Kline> {
        (0..count)
            .map(|day| {
                let base = 100.0 + day as f64;
                bar(day, base, base + 3.0, base - 2.0, base + 1.0)
            })
            .collect()
    }

    fn meta(source: DataSource, chart_type: ChartType) -> CandleMeta {
        CandleMeta {
            workspace_id: 1,
            chart_id: 2,
            instrument: InstrumentId::new(source, "BTCUSDT"),
            timeframe: "1D".into(),
            chart_type,
            older_supported: true,
            replay_ms: None,
            now_ms: T0 + 100 * DAY,
        }
    }

    #[test]
    fn times_are_written_to_the_second_in_utc() {
        assert_eq!(iso(T0), "2025-10-20T00:00:00Z");
        assert_eq!(iso(T0 + 3_661_000), "2025-10-20T01:01:01Z");
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            crate::mcp::drawings::parse_time(&crate::mcp::drawings::TimeArg::Text(iso(
                T0 + 3_661_000
            )))
            .unwrap(),
            T0 + 3_661_000
        );
    }

    #[test]
    fn candles_come_back_as_rows_under_named_columns() {
        let data = bars(5);
        let result = candles_result(
            &meta(DataSource::Binance, ChartType::Candles),
            &data,
            3,
            None,
        );
        assert_eq!(
            result.columns,
            ["time", "open", "high", "low", "close", "volume"]
        );
        assert_eq!((result.count, result.loaded_bars), (3, 5));
        assert_eq!(result.bars[0].0, iso(T0 + 2 * DAY));
        assert_eq!(
            result.bars[2],
            (iso(T0 + 4 * DAY), 104.0, 107.0, 102.0, 105.0, Some(14.0))
        );
        let range = result.range.unwrap();
        assert_eq!(
            (range.from.as_str(), range.to.as_str()),
            ("2025-10-22T00:00:00Z", "2025-10-24T00:00:00Z")
        );
        let summary = result.summary.unwrap();
        assert_eq!((summary.high, summary.low), (107.0, 100.0));
        assert_eq!((summary.first_open, summary.last_close), (102.0, 105.0));
        assert_eq!(summary.change_percent, Some(2.94));
        assert!(result.has_older_loaded);
        assert!(result.older_history_available);
        assert_eq!(result.instrument, "BINANCE:BTCUSDT");
        assert!(result.notes.is_empty());
    }

    #[test]
    fn paging_back_with_before_walks_the_loaded_bars_without_overlap() {
        let data = bars(10);
        let m = meta(DataSource::Binance, ChartType::Candles);
        let first = candles_result(&m, &data, 4, None);
        let oldest = T0 + 6 * DAY;
        assert_eq!(first.bars[0].0, iso(oldest));
        let second = candles_result(&m, &data, 4, Some(oldest));
        assert_eq!(second.bars.last().unwrap().0, iso(T0 + 5 * DAY));
        assert_eq!(second.bars[0].0, iso(T0 + 2 * DAY));
        let third = candles_result(&m, &data, 4, Some(T0 + 2 * DAY));
        assert_eq!(third.count, 2);
        assert!(!third.has_older_loaded);
        let none = candles_result(&m, &data, 4, Some(T0));
        assert_eq!(
            (none.count, none.range.is_none(), none.summary.is_none()),
            (0, true, true)
        );
        // A limit past the cap is the cap; zero is one.
        assert_eq!(
            candles_result(&m, &bars(1500), 5000, None).count,
            MAX_CANDLES
        );
        assert_eq!(candles_result(&m, &data, 0, None).count, 1);
    }

    #[test]
    fn the_forming_bar_is_only_flagged_on_the_newest_page() {
        let mut data = bars(4);
        let m = CandleMeta {
            now_ms: T0 + 3 * DAY + 1000,
            ..meta(DataSource::Binance, ChartType::Candles)
        };
        assert!(candles_result(&m, &data, 10, None).last_bar_forming);
        assert!(!candles_result(&m, &data, 2, Some(T0 + 2 * DAY)).last_bar_forming);
        data.pop();
        assert!(!candles_result(&m, &data, 10, None).last_bar_forming);
    }

    #[test]
    fn volume_is_null_where_the_provider_has_none() {
        let data = bars(2);
        let none = candles_result(
            &meta(DataSource::CoinGecko, ChartType::Candles),
            &data,
            5,
            None,
        );
        assert!(none.bars.iter().all(|row| row.5.is_none()));
        let some = candles_result(
            &meta(DataSource::Binance, ChartType::Candles),
            &data,
            5,
            None,
        );
        assert!(some.bars.iter().all(|row| row.5.is_some()));
    }

    #[test]
    fn tiny_prices_serialize_as_plain_numbers_an_agent_can_read_back() {
        let tiny = vec![bar(
            0,
            0.000_000_278,
            0.000_000_3,
            0.000_000_25,
            0.000_000_29,
        )];
        let result = candles_result(
            &meta(DataSource::DexScreener, ChartType::Candles),
            &tiny,
            5,
            None,
        );
        let json = serde_json::to_value(&result).unwrap();
        let open = json["bars"][0][1].as_f64().unwrap();
        assert_eq!(open, 0.000_000_278);
        assert!(json["bars"][0][0].is_string());
    }

    #[test]
    fn constructed_chart_types_and_replay_are_explained() {
        let data = bars(3);
        let mut m = meta(DataSource::Binance, ChartType::Renko);
        m.replay_ms = Some(T0 + DAY);
        let result = candles_result(&m, &data, 5, None);
        assert_eq!(result.chart_type, "renko");
        assert_eq!(result.notes.len(), 2);
        assert!(result.notes[0].contains("renko") && result.notes[0].contains("anchored"));
        assert!(result.notes[1].contains("Replay"));
        let empty = candles_result(&meta(DataSource::Binance, ChartType::Candles), &[], 5, None);
        assert_eq!(empty.count, 0);
        assert!(empty.notes[0].contains("no candles"));
    }

    fn spec(a: (i64, PriceSpec), b: (i64, PriceSpec), tool: McpDrawingTool) -> NewDrawing {
        NewDrawing {
            tool,
            a: AnchorSpec {
                time_ms: a.0,
                price: a.1,
            },
            b: AnchorSpec {
                time_ms: b.0,
                price: b.1,
            },
            patch: StylePatch::default(),
        }
    }

    fn value(price: f64) -> PriceSpec {
        PriceSpec::Value(price)
    }

    #[test]
    fn a_time_inside_the_candles_snaps_to_its_bar_and_outside_is_kept() {
        let data = bars(5);
        let at = |time_ms, price| AnchorSpec { time_ms, price };
        let inside = resolve_anchor(&at(T0 + DAY + 5_000, value(101.5)), &data, "a").unwrap();
        assert_eq!((inside.time_ms, inside.price), (T0 + DAY, 101.5));
        let on_open = resolve_anchor(&at(T0 + 2 * DAY, value(5.0)), &data, "a").unwrap();
        assert_eq!(on_open.time_ms, T0 + 2 * DAY);
        // Inside the newest bar, which is still open.
        let newest = resolve_anchor(&at(T0 + 4 * DAY + 1000, value(5.0)), &data, "a").unwrap();
        assert_eq!(newest.time_ms, T0 + 4 * DAY);
        // Past the end and before the start: kept as given.
        let future = resolve_anchor(&at(T0 + 9 * DAY + 7, value(5.0)), &data, "a").unwrap();
        assert_eq!(future.time_ms, T0 + 9 * DAY + 7);
        let past = resolve_anchor(&at(T0 - 3 * DAY + 7, value(5.0)), &data, "a").unwrap();
        assert_eq!(past.time_ms, T0 - 3 * DAY + 7);
    }

    #[test]
    fn price_from_reads_the_bar_and_needs_one() {
        let data = bars(5);
        let at = |time_ms, field| AnchorSpec {
            time_ms,
            price: PriceSpec::Field(field),
        };
        for (field, expected) in [
            (PriceField::Open, 102.0),
            (PriceField::High, 105.0),
            (PriceField::Low, 100.0),
            (PriceField::Close, 103.0),
        ] {
            let anchor = resolve_anchor(&at(T0 + 2 * DAY + 123, field), &data, "a").unwrap();
            assert_eq!(
                (anchor.time_ms, anchor.price),
                (T0 + 2 * DAY, expected),
                "{field:?}"
            );
        }
        for time in [T0 - DAY, T0 + 9 * DAY] {
            let error = resolve_anchor(&at(time, PriceField::High), &data, "b").unwrap_err();
            assert!(
                matches!(&error, McpError::InvalidArgument(m) if m.contains("Point b") && m.contains("2025-10-20T00:00:00Z")),
                "{error:?}"
            );
        }
        let error = resolve_anchor(&at(T0, PriceField::High), &[], "a").unwrap_err();
        assert!(matches!(&error, McpError::InvalidArgument(m) if m.contains("no candles")));
    }

    #[test]
    fn a_trend_line_and_a_rectangle_are_built_in_the_requested_style() {
        let data = bars(10);
        let mut line = spec(
            (T0 + DAY, PriceSpec::Field(PriceField::Low)),
            (T0 + 6 * DAY, PriceSpec::Field(PriceField::Low)),
            McpDrawingTool::TrendLine,
        );
        line.patch.color = Some([255, 193, 7, 255]);
        line.patch.dash = Some(LineDash::Dashed);
        line.patch.extend_right = Some(true);
        line.patch.width = Some(3.0);
        let mut zone = spec(
            (T0 + 2 * DAY, value(104.0)),
            (T0 + 5 * DAY, value(110.0)),
            McpDrawingTool::Rectangle,
        );
        zone.patch.color = Some([41, 98, 255, 255]);
        zone.patch.text = Some("Supply".into());
        let planned = plan_add(&[], &data, 7, &[line, zone]).unwrap();
        assert_eq!(planned.iter().map(Drawing::id).collect::<Vec<_>>(), [7, 8]);
        let Some(crate::model::Geometry::TrendLine { a, b }) = planned[0].geometry() else {
            panic!("trend line expected: {:?}", planned[0]);
        };
        let style = planned[0].style().unwrap();
        assert_eq!((a.time_ms, a.price), (T0 + DAY, 99.0));
        assert_eq!((b.time_ms, b.price), (T0 + 6 * DAY, 104.0));
        assert_eq!(style.stroke.color, [255, 193, 7, 255]);
        assert_eq!(style.stroke.dash, LineDash::Dashed);
        assert_eq!(style.stroke.width, 3.0);
        assert!(style.extend_right && !style.extend_left);
        assert_eq!(planned[1].text(), "Supply");
        assert_eq!(planned[1].tool(), Some(Tool::Rectangle));
        let rect = planned[1].style().unwrap();
        assert_eq!(rect.stroke.color, [41, 98, 255, 255]);
        assert_eq!(rect.text.color, [41, 98, 255, 255]);
        // What the tool lists is what was built.
        let info = drawing_info(&planned[0]).unwrap();
        assert_eq!(info.tool, "trend_line");
        assert_eq!(info.style.as_ref().unwrap().color, "#FFC107FF");
        assert_eq!(info.style.unwrap().dash, "dashed");
        assert!(info.editable);
        assert_eq!(
            drawing_info(&planned[1]).unwrap().text.as_deref(),
            Some("Supply")
        );
    }

    #[test]
    fn a_rectangle_fill_follows_the_colour_unless_one_is_given() {
        let data = bars(10);
        let base = || {
            spec(
                (T0, value(100.0)),
                (T0 + 3 * DAY, value(110.0)),
                McpDrawingTool::Rectangle,
            )
        };
        let mut tinted = base();
        tinted.patch.color = Some([10, 20, 30, 255]);
        let planned = plan_add(&[], &data, 1, &[tinted]).unwrap();
        let style = planned[0].style().unwrap();
        assert!(style.fill.enabled);
        assert_eq!(&style.fill.color[..3], &[10, 20, 30]);
        let mut filled = base();
        filled.patch.fill_color = Some([200, 0, 0, 51]);
        let planned = plan_add(&[], &data, 1, &[filled]).unwrap();
        assert_eq!(planned[0].style().unwrap().fill.color, [200, 0, 0, 51]);
    }

    #[test]
    fn a_bad_spec_refuses_the_whole_call_and_says_which() {
        let data = bars(10);
        let good = || {
            spec(
                (T0, value(100.0)),
                (T0 + 3 * DAY, value(110.0)),
                McpDrawingTool::TrendLine,
            )
        };
        let same_point = spec(
            (T0, value(100.0)),
            (T0, value(100.0)),
            McpDrawingTool::TrendLine,
        );
        let flat_box = spec(
            (T0, value(100.0)),
            (T0 + 3 * DAY, value(100.0)),
            McpDrawingTool::Rectangle,
        );
        let thin_box = spec(
            (T0, value(100.0)),
            (T0, value(110.0)),
            McpDrawingTool::Rectangle,
        );
        let outside = spec(
            (T0, PriceSpec::Field(PriceField::High)),
            (T0 + 99 * DAY, PriceSpec::Field(PriceField::High)),
            McpDrawingTool::TrendLine,
        );
        let mut line_with_text = good();
        line_with_text.patch.text = Some("no".into());
        for (why, bad) in [
            ("same point", same_point),
            ("flat box", flat_box),
            ("thin box", thin_box),
            ("price_from outside", outside),
            ("text on a line", line_with_text),
        ] {
            let error = plan_add(&[], &data, 1, &[good(), bad]).unwrap_err();
            assert!(
                matches!(&error, McpError::InvalidArgument(m) if m.starts_with("Drawing 2")),
                "{why}: {error:?}"
            );
        }
    }

    #[test]
    fn a_new_id_never_repeats_one_the_chart_already_holds() {
        assert_eq!(next_free_id(&[], 0), 1);
        assert_eq!(next_free_id(&[], 5), 5);
        let drawings = vec![
            line(7, false),
            Drawing::Unreadable(serde_json::json!({"Shape": {"id": 12}})),
        ];
        // The counter has fallen behind the stored ids: the drawings win.
        assert_eq!(next_free_id(&drawings, 2), 13);
        assert_eq!(next_free_id(&drawings, 40), 40);
    }

    #[test]
    fn a_chart_holds_a_bounded_number_of_drawings() {
        let data = bars(3);
        let one = spec(
            (T0, value(100.0)),
            (T0 + DAY, value(101.0)),
            McpDrawingTool::TrendLine,
        );
        let full: Vec<Drawing> = plan_add(&[], &data, 1, &vec![one.clone(); 25]).unwrap();
        let mut held = Vec::new();
        for _ in 0..MAX_DRAWINGS_PER_CHART / 25 {
            held.extend(full.clone());
        }
        assert_eq!(counted(&held), MAX_DRAWINGS_PER_CHART);
        assert!(matches!(
            plan_add(&held, &data, 1, std::slice::from_ref(&one)),
            Err(McpError::ResourceLimit(_))
        ));
        // A ruler is not a drawing.
        let ruler = Drawing::Ruler {
            id: 9,
            a: Anchor {
                time_ms: 1,
                price: 1.0,
            },
            b: Anchor {
                time_ms: 2,
                price: 2.0,
            },
        };
        assert_eq!(counted(std::slice::from_ref(&ruler)), 0);
        assert!(drawing_info(&ruler).is_none());
    }

    fn line(id: u64, locked: bool) -> Drawing {
        let mut drawing = Drawing::build(
            Tool::TrendLine,
            id,
            &[
                Anchor {
                    time_ms: T0,
                    price: 100.0,
                },
                Anchor {
                    time_ms: T0 + 3 * DAY,
                    price: 110.0,
                },
            ],
            Tool::TrendLine.default_style(),
        )
        .unwrap();
        drawing.set_locked(locked);
        drawing
    }

    fn edit(id: u64) -> DrawingEdit {
        DrawingEdit {
            drawing_id: id,
            a: None,
            b: None,
            patch: StylePatch::default(),
            hidden: None,
        }
    }

    #[test]
    fn editing_moves_points_restyles_and_hides() {
        let data = bars(10);
        let existing = vec![line(3, false)];
        let mut change = edit(3);
        change.b = Some(AnchorSpec {
            time_ms: T0 + 6 * DAY + 99,
            price: PriceSpec::Field(PriceField::High),
        });
        change.patch.color = Some([1, 2, 3, 255]);
        change.patch.extend_right = Some(true);
        change.hidden = Some(true);
        let Some(DrawingCommand::Replace { id, drawing }) =
            plan_edit(&existing, &data, &change).unwrap()
        else {
            panic!("a replacement was expected");
        };
        assert_eq!(id, 3);
        let anchors = drawing.anchors();
        assert_eq!((anchors[0].time_ms, anchors[0].price), (T0, 100.0));
        assert_eq!(
            (anchors[1].time_ms, anchors[1].price),
            (T0 + 6 * DAY, 109.0)
        );
        assert_eq!(drawing.style().unwrap().stroke.color, [1, 2, 3, 255]);
        assert!(drawing.style().unwrap().extend_right);
        assert!(drawing.hidden());
        // Saying what is already true changes nothing.
        let same = DrawingEdit {
            hidden: Some(false),
            patch: StylePatch {
                extend_right: Some(false),
                ..StylePatch::default()
            },
            ..edit(3)
        };
        assert_eq!(plan_edit(&existing, &data, &same).unwrap(), None);
    }

    #[test]
    fn editing_respects_locks_unknown_ids_and_unsupported_tools() {
        let data = bars(10);
        let fib = Drawing::build(
            Tool::FibRetracement,
            4,
            &[
                Anchor {
                    time_ms: T0,
                    price: 100.0,
                },
                Anchor {
                    time_ms: T0 + DAY,
                    price: 110.0,
                },
            ],
            DrawingStyle::default(),
        )
        .unwrap();
        let existing = vec![
            line(3, true),
            fib,
            Drawing::Unreadable(serde_json::json!({"X": 1})),
        ];
        let recolour = |id| DrawingEdit {
            patch: StylePatch {
                color: Some([9, 9, 9, 255]),
                ..StylePatch::default()
            },
            ..edit(id)
        };
        assert!(matches!(
            plan_edit(&existing, &data, &recolour(3)),
            Err(McpError::Conflict(_))
        ));
        assert!(matches!(
            plan_edit(&existing, &data, &recolour(4)),
            Err(McpError::Unsupported(_))
        ));
        assert!(matches!(
            plan_edit(&existing, &data, &recolour(99)),
            Err(McpError::NotFound(_))
        ));
        assert!(matches!(
            plan_edit(&existing, &data, &recolour(0)),
            Err(McpError::NotFound(_))
        ));
        // A line cannot take a fill, and a rectangle cannot collapse to a sliver.
        let existing = vec![line(3, false)];
        let fill = DrawingEdit {
            patch: StylePatch {
                fill_color: Some([1, 1, 1, 50]),
                ..StylePatch::default()
            },
            ..edit(3)
        };
        assert!(matches!(
            plan_edit(&existing, &data, &fill),
            Err(McpError::InvalidArgument(_))
        ));
        let zone = plan_add(
            &[],
            &data,
            5,
            &[spec(
                (T0, value(100.0)),
                (T0 + 3 * DAY, value(110.0)),
                McpDrawingTool::Rectangle,
            )],
        )
        .unwrap();
        let flatten = DrawingEdit {
            b: Some(AnchorSpec {
                time_ms: T0 + 3 * DAY,
                price: value(100.0),
            }),
            ..edit(5)
        };
        assert!(matches!(
            plan_edit(&zone, &data, &flatten),
            Err(McpError::InvalidArgument(_))
        ));
    }

    #[test]
    fn removal_is_all_or_nothing() {
        let existing = vec![line(1, false), line(2, true), line(3, false)];
        let commands = plan_remove(&existing, &[1, 3, 1]).unwrap();
        assert_eq!(
            commands,
            [DrawingCommand::Remove(1), DrawingCommand::Remove(3)]
        );
        let locked = plan_remove(&existing, &[1, 2]).unwrap_err();
        assert!(matches!(&locked, McpError::Conflict(m) if m.contains("Nothing was removed")));
        assert!(matches!(
            plan_remove(&existing, &[1, 9]),
            Err(McpError::NotFound(_))
        ));
        assert!(plan_remove(&existing, &[]).unwrap().is_empty());
    }

    #[test]
    fn the_listing_names_tools_in_snake_case_and_counts_what_it_cannot_read() {
        let mut brush = Drawing::build(
            Tool::Brush,
            6,
            &[
                Anchor {
                    time_ms: T0,
                    price: 100.0,
                },
                Anchor {
                    time_ms: T0 + DAY,
                    price: 101.0,
                },
            ],
            Tool::Brush.default_style(),
        )
        .unwrap();
        brush.set_locked(true);
        let drawings = vec![
            line(1, false),
            brush,
            Drawing::Unreadable(serde_json::json!({"Shape": {}})),
        ];
        let listed = drawings_result(1, 2, &drawings);
        assert_eq!(
            (listed.total, listed.unreadable, listed.truncated),
            (2, 1, false)
        );
        assert_eq!(listed.drawings[0].tool, "trend_line");
        assert_eq!(listed.drawings[1].tool, "brush");
        assert!(!listed.drawings[1].editable);
        assert!(listed.drawings[1].locked);
        assert_eq!(snake("DoubleCurve"), "double_curve");
    }

    #[test]
    fn every_pattern_wave_and_cycle_is_listed_read_only_by_name() {
        let at = |index: i64| Anchor {
            time_ms: T0 + index * DAY,
            price: 100.0 + (index % 3) as f64 * 5.0 + index as f64,
        };
        let anchors: Vec<Anchor> = (0..7).map(at).collect();
        for (tool, name) in [
            (Tool::XabcdPattern, "xabcd_pattern"),
            (Tool::CypherPattern, "cypher_pattern"),
            (Tool::HeadAndShoulders, "head_and_shoulders"),
            (Tool::AbcdPattern, "abcd_pattern"),
            (Tool::TrianglePattern, "triangle_pattern"),
            (Tool::ThreeDrivesPattern, "three_drives_pattern"),
            (Tool::ElliottImpulse, "elliott_impulse"),
            (Tool::ElliottCorrection, "elliott_correction"),
            (Tool::ElliottTriangle, "elliott_triangle"),
            (Tool::ElliottDoubleCombo, "elliott_double_combo"),
            (Tool::ElliottTripleCombo, "elliott_triple_combo"),
            (Tool::CyclicLines, "cyclic_lines"),
            (Tool::TimeCycles, "time_cycles"),
            (Tool::SineLine, "sine_line"),
        ] {
            let count = match tool.spec().placement {
                crate::model::Placement::Points(count) => usize::from(count),
                _ => 1,
            };
            let drawing = Drawing::build(tool, 1, &anchors[..count], tool.default_style())
                .unwrap_or_else(|| panic!("{tool:?}"));
            let info = drawing_info(&drawing).unwrap_or_else(|| panic!("{tool:?} is listed"));
            assert_eq!(info.tool, name);
            assert!(!info.editable, "{name} cannot be edited over MCP");
            assert_eq!(info.anchors.len(), count.min(MAX_ANCHORS_SHOWN), "{name}");
        }
    }

    #[test]
    fn every_forecast_volume_and_measurement_tool_is_listed_read_only_by_name() {
        let at = |index: i64| Anchor {
            time_ms: T0 + index * DAY,
            price: 100.0 + (index % 3) as f64 * 5.0 + index as f64,
        };
        let anchors: Vec<Anchor> = (0..3).map(at).collect();
        for (tool, name) in [
            (Tool::LongPosition, "long_position"),
            (Tool::ShortPosition, "short_position"),
            (Tool::PositionForecast, "position_forecast"),
            (Tool::BarsPattern, "bars_pattern"),
            (Tool::GhostFeed, "ghost_feed"),
            (Tool::Sector, "sector"),
            (Tool::AnchoredVwap, "anchored_vwap"),
            (Tool::FixedRangeVolumeProfile, "fixed_range_volume_profile"),
            (Tool::AnchoredVolumeProfile, "anchored_volume_profile"),
            (Tool::PriceRange, "price_range"),
            (Tool::DateRange, "date_range"),
            (Tool::DateAndPriceRange, "date_and_price_range"),
        ] {
            let count = match tool.spec().placement {
                crate::model::Placement::Points(count) => usize::from(count),
                _ => 1,
            };
            let drawing = Drawing::sample(tool, 1, &anchors[..count.min(3)], tool.default_style())
                .unwrap_or_else(|| panic!("{tool:?}"));
            let info = drawing_info(&drawing).unwrap_or_else(|| panic!("{tool:?} is listed"));
            assert_eq!(info.tool, name);
            assert!(!info.editable, "{name} cannot be edited over MCP");
        }
    }

    #[test]
    fn every_fibonacci_and_gann_tool_is_listed_read_only_by_name() {
        let at = |index: i64| Anchor {
            time_ms: T0 + index * DAY,
            price: 100.0 + (index % 3) as f64 * 5.0 + index as f64,
        };
        let anchors: Vec<Anchor> = (0..4).map(at).collect();
        let names = [
            (Tool::FibRetracement, "fib_retracement"),
            (Tool::FibExtension, "fib_extension"),
            (Tool::FibChannel, "fib_channel"),
            (Tool::FibTimeZone, "fib_time_zone"),
            (Tool::FibSpeedFan, "fib_speed_fan"),
            (Tool::FibTrendTime, "fib_trend_time"),
            (Tool::FibCircles, "fib_circles"),
            (Tool::FibSpiral, "fib_spiral"),
            (Tool::FibSpeedArcs, "fib_speed_arcs"),
            (Tool::FibWedge, "fib_wedge"),
            (Tool::Pitchfan, "pitchfan"),
        ];
        assert_eq!(
            names.len(),
            Tool::fib_tools().count(),
            "every tool is named"
        );
        for (tool, name) in names {
            let crate::model::Placement::Points(count) = tool.spec().placement else {
                panic!("{tool:?} is placed by points");
            };
            let drawing = Drawing::sample(
                tool,
                1,
                &anchors[..usize::from(count)],
                tool.default_style(),
            )
            .unwrap_or_else(|| panic!("{tool:?}"));
            let info = drawing_info(&drawing).unwrap_or_else(|| panic!("{tool:?} is listed"));
            assert_eq!(info.tool, name);
            assert_eq!(info.anchor_count, usize::from(count));
            assert!(!info.editable, "{name} cannot be edited over MCP");
        }
    }

    #[test]
    fn every_text_note_and_content_tool_is_listed_read_only_by_name() {
        let at = |index: i64| Anchor {
            time_ms: T0 + index * DAY,
            price: 100.0 + index as f64,
        };
        let anchors = [at(0), at(1), at(2)];
        for (tool, name) in [
            (Tool::Text, "text"),
            (Tool::Note, "note"),
            (Tool::PriceNote, "price_note"),
            (Tool::Pin, "pin"),
            (Tool::Table, "table"),
            (Tool::Callout, "callout"),
            (Tool::Comment, "comment"),
            (Tool::PriceLabel, "price_label"),
            (Tool::Signpost, "signpost"),
            (Tool::FlagMark, "flag_mark"),
            (Tool::Image, "image"),
            (Tool::Post, "post"),
        ] {
            let count = match tool.spec().placement {
                crate::model::Placement::Points(count) => usize::from(count),
                _ => 1,
            };
            let drawing = Drawing::sample(tool, 1, &anchors[..count], tool.default_style())
                .unwrap_or_else(|| panic!("{tool:?}"));
            let info = drawing_info(&drawing).unwrap_or_else(|| panic!("{tool:?} is listed"));
            assert_eq!(info.tool, name);
            assert!(!info.editable, "{name} cannot be edited over MCP");
        }
    }
}
