//! TradingView's settings as DegenView's. Each tool starts with DegenView's own default look and
//! takes what the drawing sets, where DegenView has the setting. TradingView names and nests these
//! differently from tool to tool; the names are those of the web app's own default settings.

use super::super::color::{self, with_transparency};
use super::super::drawing::RawDrawing;
use crate::model::Tool;
use crate::model::chart::kind::{LineDash, PriceSource, Rgba};
use crate::model::drawings::style::{
    DrawingStyle, HorizontalAlign, Level, LineEnd, MAX_BOX_WIDTH, MiddleLine, StatsPosition,
    VerticalAlign,
};
use crate::model::fib::style::{FibLevel, Levels};
use crate::model::forecast::style::{ProfileVolume, RowsLayout};
use crate::model::pattern::elliott::Degree;
use serde_json::Value;

/// A colour setting, under whichever name the tool keeps it.
fn colour(raw: &RawDrawing, keys: &[&str]) -> Option<Rgba> {
    keys.iter()
        .find_map(|key| raw.str(key).and_then(color::parse))
}

/// The colour `object[key]`, made as transparent as `object.transparency` says when it does not
/// carry its own opacity.
fn colour_in(object: &Value, key: &str) -> Option<Rgba> {
    let color = color::parse(object.get(key)?.as_str()?)?;
    Some(faded(
        color,
        object.get("transparency").and_then(Value::as_f64),
    ))
}

fn faded(color: Rgba, transparency: Option<f64>) -> Rgba {
    match transparency {
        // An `rgba` colour carries its own opacity.
        Some(transparency) if color[3] == 255 => with_transparency(color, transparency),
        _ => color,
    }
}

fn dash(code: i64) -> LineDash {
    match code {
        1 => LineDash::Dotted,
        2 => LineDash::Dashed,
        _ => LineDash::Solid,
    }
}

/// A line kept as an object: `{color, visible, <style>, <width>}`, under either spelling.
fn line_in(object: &Value, base: MiddleLine) -> MiddleLine {
    let any = |keys: [&str; 2]| keys.iter().find_map(|key| object.get(*key));
    MiddleLine {
        enabled: object
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(base.enabled),
        color: colour_in(object, "color").unwrap_or(base.color),
        width: any(["linewidth", "lineWidth"])
            .and_then(Value::as_f64)
            .map_or(base.width, |width| width as f32),
        dash: any(["linestyle", "lineStyle"])
            .and_then(Value::as_i64)
            .map_or(base.dash, dash),
    }
}

/// The tools whose levels are read: TradingView stores theirs as `level1`…, each
/// `[ratio, colour, shown]`, and numbers them as DegenView's defaults do.
pub(super) fn reads_levels(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::FibRetracement
            | Tool::FibExtension
            | Tool::FibChannel
            | Tool::FibTimeZone
            | Tool::FibTrendTime
            | Tool::FibCircles
            | Tool::FibSpeedArcs
            | Tool::FibWedge
            | Tool::Pitchfan
    )
}

/// A drawing's words. A channel keeps them as a label, with a switch of its own.
pub(super) fn words(raw: &RawDrawing) -> &str {
    let label = raw
        .str("labelText")
        .filter(|_| raw.flag("labelVisible") == Some(true));
    raw.str("text")
        .filter(|text| !text.is_empty())
        .or(label)
        .unwrap_or("")
        .trim_end()
}

/// The tool's own default look, with what TradingView's drawing sets.
pub(super) fn style(raw: &RawDrawing, tool: Tool) -> DrawingStyle {
    let mut style = tool.default_style();
    common(raw, tool, &mut style);
    match tool {
        Tool::TrendLine
        | Tool::Ray
        | Tool::InfoLine
        | Tool::ExtendedLine
        | Tool::TrendAngle
        | Tool::HorizontalLine
        | Tool::HorizontalRay => {
            line_label(raw, &mut style);
            line_stats(raw, &mut style);
        }
        Tool::Arrow => line_stats(raw, &mut style),
        // TradingView places a box's words against its edges as it does a line's.
        Tool::Rectangle => {
            line_label(raw, &mut style);
            middle_of_box(raw, &mut style);
        }
        Tool::ParallelChannel => {
            channel(raw, &mut style);
            channel_label(raw, &mut style);
        }
        Tool::FlatChannel | Tool::DisjointChannel => {
            channel_label(raw, &mut style);
            if let Some(shown) = raw.flag("showPrices") {
                style.price_labels = shown;
            }
        }
        Tool::RegressionTrend => regression(raw, &mut style),
        Tool::Pitchfork
        | Tool::SchiffPitchfork
        | Tool::ModifiedSchiffPitchfork
        | Tool::InsidePitchfork => pitchfork(raw, &mut style),
        Tool::Table => {
            if let Some(align) = raw.str("horzAlign").and_then(horizontal) {
                style.text.align = align;
            }
        }
        Tool::ArrowMarker => {
            if let Some(color) = colour(raw, &["backgroundColor"]) {
                style.stroke.color = color;
            }
        }
        Tool::ArrowMarkUp | Tool::ArrowMarkDown => {
            if let Some(color) = colour(raw, &["arrowColor"]) {
                style.stroke.color = color;
            }
            if let Some(color) = colour(raw, &["color"]) {
                style.text.color = color;
            }
        }
        Tool::FlagMark => {
            if let Some(color) = colour(raw, &["flagColor"]) {
                style.stroke.color = color;
            }
        }
        Tool::ElliottImpulse
        | Tool::ElliottCorrection
        | Tool::ElliottTriangle
        | Tool::ElliottDoubleCombo
        | Tool::ElliottTripleCombo => degree(raw, &mut style),
        Tool::AnchoredVwap => vwap(raw, &mut style),
        Tool::FixedRangeVolumeProfile | Tool::AnchoredVolumeProfile => profile(raw, &mut style),
        Tool::PriceRange | Tool::DateRange | Tool::DateAndPriceRange => measure(raw, &mut style),
        Tool::LongPosition | Tool::ShortPosition => position(raw, &mut style),
        _ => {}
    }
    if reads_levels(tool) {
        levels(raw, &mut style);
        if tool == Tool::Pitchfan
            && let Some(median) = raw.props.get("median")
        {
            let line = line_in(median, stroke_as_line(&style));
            take_stroke(&mut style, line);
        }
    }
    style.sanitized()
}

/// The settings most tools share.
fn common(raw: &RawDrawing, tool: Tool, style: &mut DrawingStyle) {
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
    if let Some(code) = ["linestyle", "lineStyle"]
        .iter()
        .find_map(|key| raw.props.get(*key).and_then(Value::as_i64))
    {
        style.stroke.dash = dash(code);
    }
    if let Some(color) = colour(raw, &["backgroundColor", "background"]) {
        let transparency = raw
            .number("backgroundTransparency")
            .or_else(|| raw.number("transparency"));
        style.fill.color = faded(color, transparency);
    }
    if let Some(enabled) = raw
        .flag("fillBackground")
        .or_else(|| raw.flag("drawBackground"))
    {
        style.fill.enabled = enabled;
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
            style.box_width = (width as f32).min(MAX_BOX_WIDTH);
        }
    }
    if let Some(color) = colour(raw, &["borderColor", "bordercolor"]) {
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
}

fn horizontal(name: &str) -> Option<HorizontalAlign> {
    match name {
        "left" => Some(HorizontalAlign::Left),
        "center" => Some(HorizontalAlign::Center),
        "right" => Some(HorizontalAlign::Right),
        _ => None,
    }
}

/// TradingView names the edge of the words that meets the line, so its `bottom` is above the line;
/// DegenView names the side of the line.
fn vertical(name: &str) -> Option<VerticalAlign> {
    match name {
        "bottom" => Some(VerticalAlign::Top),
        "middle" => Some(VerticalAlign::Middle),
        "top" => Some(VerticalAlign::Bottom),
        _ => None,
    }
}

/// Where a line's words sit on it.
fn line_label(raw: &RawDrawing, style: &mut DrawingStyle) {
    if let Some(align) = raw.str("horzLabelsAlign").and_then(horizontal) {
        style.text.align = align;
    }
    if let Some(valign) = raw.str("vertLabelsAlign").and_then(vertical) {
        style.text.valign = valign;
    }
}

/// A channel's label has settings of its own, beside the drawing's.
fn channel_label(raw: &RawDrawing, style: &mut DrawingStyle) {
    if raw.flag("labelVisible") != Some(true) {
        return;
    }
    if let Some(color) = colour(raw, &["labelTextColor"]) {
        style.text.color = color;
    }
    if let Some(size) = raw.number("labelFontSize") {
        style.text.size = size as f32;
    }
    if let Some(bold) = raw.flag("labelBold") {
        style.text.bold = bold;
    }
    if let Some(italic) = raw.flag("labelItalic") {
        style.text.italic = italic;
    }
    if let Some(align) = raw.str("labelHorzAlign").and_then(horizontal) {
        style.text.align = align;
    }
    if let Some(valign) = raw.str("labelVertAlign").and_then(vertical) {
        style.text.valign = valign;
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

fn middle_of_box(raw: &RawDrawing, style: &mut DrawingStyle) {
    let Some(middle) = raw.props.get("middleLine") else {
        return;
    };
    let line = &mut style.middle_line;
    if let Some(shown) = middle.get("showLine").and_then(Value::as_bool) {
        line.enabled = shown;
    }
    if let Some(color) = colour_in(middle, "lineColor") {
        line.color = color;
    }
    if let Some(width) = middle.get("lineWidth").and_then(Value::as_f64) {
        line.width = width as f32;
    }
    if let Some(code) = middle.get("lineStyle").and_then(Value::as_i64) {
        line.dash = dash(code);
    }
}

fn stroke_as_line(style: &DrawingStyle) -> MiddleLine {
    MiddleLine {
        enabled: true,
        color: style.stroke.color,
        width: style.stroke.width,
        dash: style.stroke.dash,
    }
}

fn take_stroke(style: &mut DrawingStyle, line: MiddleLine) {
    style.stroke.color = line.color;
    style.stroke.width = line.width;
    style.stroke.dash = line.dash;
}

/// A parallel channel keeps every line as a level: `level2` and `level6` are its edges, `level4`
/// its middle and the rest the four extra lines. Older channels keep a middle line of their own.
fn channel(raw: &RawDrawing, style: &mut DrawingStyle) {
    if let Some(edge) = raw.props.get("level2") {
        let line = line_in(edge, stroke_as_line(style));
        take_stroke(style, line);
    }
    if let Some(middle) = raw.props.get("level4") {
        style.middle_line = line_in(middle, style.middle_line);
    } else {
        if let Some(shown) = raw.flag("showMidline") {
            style.middle_line.enabled = shown;
        }
        if let Some(color) = colour(raw, &["midlinecolor"]) {
            style.middle_line.color = color;
        }
        if let Some(width) = raw.number("midlinewidth") {
            style.middle_line.width = width as f32;
        }
        if let Some(code) = raw.props.get("midlinestyle").and_then(Value::as_i64) {
            style.middle_line.dash = dash(code);
        }
    }
    // DegenView's extra lines are at 0.25, 0.75, -0.25 and 1.25, in that order.
    for (slot, key) in ["level3", "level5", "level1", "level7"].iter().enumerate() {
        let (Some(stored), Some(level)) = (raw.props.get(*key), style.channel_levels.get_mut(slot))
        else {
            continue;
        };
        if let Some(ratio) = stored.get("coeff").and_then(Value::as_f64) {
            level.ratio = ratio as f32;
        }
        level.line = line_in(stored, level.line);
    }
}

/// A pitchfork's median is its stroke; `level0`…`level8` are `[ratio, colour, shown, dash, width]`.
fn pitchfork(raw: &RawDrawing, style: &mut DrawingStyle) {
    if let Some(median) = raw.props.get("median") {
        let line = line_in(median, stroke_as_line(style));
        take_stroke(style, line);
    }
    for (index, level) in style.pitchfork_levels.iter_mut().enumerate() {
        let Some(parts) = raw
            .props
            .get(&format!("level{index}"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let base = *level;
        *level = Level {
            ratio: parts
                .first()
                .and_then(Value::as_f64)
                .map_or(base.ratio, |ratio| ratio as f32),
            line: MiddleLine {
                color: parts
                    .get(1)
                    .and_then(Value::as_str)
                    .and_then(color::parse)
                    .unwrap_or(base.line.color),
                enabled: parts
                    .get(2)
                    .and_then(Value::as_bool)
                    .unwrap_or(base.line.enabled),
                dash: parts
                    .get(3)
                    .and_then(Value::as_i64)
                    .map_or(base.line.dash, dash),
                width: parts
                    .get(4)
                    .and_then(Value::as_f64)
                    .map_or(base.line.width, |width| width as f32),
            },
        };
    }
    if let Some(transparency) = raw.number("transparency") {
        style.fill.color = with_transparency(style.fill.color, transparency);
    }
}

fn source(name: &str) -> Option<PriceSource> {
    Some(match name {
        "open" => PriceSource::Open,
        "high" => PriceSource::High,
        "low" => PriceSource::Low,
        "close" => PriceSource::Close,
        "hl2" => PriceSource::Hl2,
        "hlc3" => PriceSource::Hlc3,
        "ohlc4" => PriceSource::Ohlc4,
        _ => return None,
    })
}

/// A regression trend is a study in TradingView: `inputs` (spelt "diviation") and `styles`.
fn regression(raw: &RawDrawing, style: &mut DrawingStyle) {
    let options = &mut style.regression;
    if let Some(inputs) = raw.props.get("inputs") {
        if let Some(found) = inputs
            .get("source")
            .and_then(Value::as_str)
            .and_then(source)
        {
            options.source = found;
        }
        if let Some(up) = inputs.get("upper diviation").and_then(Value::as_f64) {
            options.upper_dev = up as f32;
        }
        if let Some(low) = inputs.get("lower diviation").and_then(Value::as_f64) {
            options.lower_dev = low as f32;
        }
    }
    let Some(styles) = raw.props.get("styles") else {
        return;
    };
    for (key, line) in [
        ("baseLine", &mut options.base),
        ("upLine", &mut options.upper),
        ("downLine", &mut options.lower),
    ] {
        if let Some(stored) = styles.get(key) {
            *line = line_in(stored, *line);
        }
    }
    let used = |key: &str| {
        raw.props
            .get("inputs")
            .and_then(|inputs| inputs.get(key))
            .and_then(Value::as_bool)
    };
    if let Some(shown) = used("use upper diviation") {
        options.upper.enabled = shown;
    }
    if let Some(shown) = used("use lower diviation") {
        options.lower.enabled = shown;
    }
    if let Some(shown) = styles.get("showPearsons").and_then(Value::as_bool) {
        options.pearson = shown;
    }
    if let Some(extend) = styles.get("extendLines").and_then(Value::as_bool) {
        style.extend_right = extend;
    }
}

/// TradingView's degrees run from Supermillennium (0); ours from Grand Supercycle, its 3. Those
/// beyond either end take the nearest.
fn degree(raw: &RawDrawing, style: &mut DrawingStyle) {
    let Some(stored) = raw.props.get("degree").and_then(Value::as_i64) else {
        return;
    };
    let index = stored.clamp(3, 11) as usize - 3;
    if let Some(found) = Degree::ALL.get(index) {
        style.pattern.degree = *found;
    }
}

fn has_percent_bands_in(inputs: &Value) -> bool {
    let on = |key: &str| inputs.get(key).and_then(Value::as_bool) == Some(true);
    inputs.get("Bands Calculation Mode").and_then(Value::as_str) == Some("Percentage")
        && (on("calculate_stDev") || on("calculate_stDev_2") || on("calculate_stDev_3"))
}

pub(super) fn has_percent_bands(raw: &RawDrawing) -> bool {
    raw.props.get("inputs").is_some_and(has_percent_bands_in)
}

/// An anchored VWAP, which TradingView keeps as a study: its settings are `inputs` and the look of
/// each of its plots is under `styles`.
fn vwap(raw: &RawDrawing, style: &mut DrawingStyle) {
    let input = |key: &str| raw.props.get("inputs").and_then(|inputs| inputs.get(key));
    let plot = |name: &str| raw.props.get("styles").and_then(|styles| styles.get(name));
    if let Some(stored) = plot("VWAP") {
        let line = line_in(stored, stroke_as_line(style));
        take_stroke(style, line);
    }
    if let Some(found) = input("source").and_then(Value::as_str).and_then(source) {
        style.vwap.source = found;
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
        if let Some(color) =
            plot(&format!("UpperBand{suffix}")).and_then(|stored| colour_in(stored, "color"))
        {
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

/// A volume profile, also a study: `inputs`, and its histograms and lines under `graphics`.
fn profile(raw: &RawDrawing, style: &mut DrawingStyle) {
    let options = &mut style.profile;
    if let Some(inputs) = raw.props.get("inputs") {
        let per_row = inputs.get("rowsLayout").and_then(Value::as_str) == Some("Ticks Per Row");
        options.layout = if per_row {
            RowsLayout::Ticks
        } else {
            RowsLayout::Rows
        };
        if let Some(count) = inputs.get("rows").and_then(Value::as_u64) {
            let count = u32::try_from(count).unwrap_or(u32::MAX);
            if per_row {
                options.ticks = count;
            } else {
                options.rows = count;
            }
        }
        if let Some(share) = inputs.get("vaVolume").and_then(Value::as_f64) {
            options.value_area = share as f32;
        }
        options.volume = match inputs.get("volume").and_then(Value::as_str) {
            Some("Total") => ProfileVolume::Total,
            Some("Delta") => ProfileVolume::Delta,
            Some("Up/Down") => ProfileVolume::UpDown,
            _ => options.volume,
        };
    }
    let Some(graphics) = raw.props.get("graphics") else {
        return;
    };
    let bars = graphics.get("hhists").and_then(|all| all.get("histBarsVA"));
    if let Some(bars) = bars {
        if let Some(width) = bars.get("percentWidth").and_then(Value::as_f64) {
            options.width = width as f32;
        }
        if let Some(direction) = bars.get("direction").and_then(Value::as_str) {
            options.right = direction == "right_to_left";
        }
        let color = |index: usize| {
            bars.get("colors")?
                .get(index)?
                .as_str()
                .and_then(color::parse)
        };
        if let Some(up) = color(0) {
            options.up = up;
        }
        if let Some(down) = color(1) {
            options.down = down;
        }
    }
    let line = |name: &str| graphics.get("horizlines").and_then(|all| all.get(name));
    if let Some(poc) = line("pocLines") {
        if let Some(shown) = poc.get("visible").and_then(Value::as_bool) {
            options.poc = shown;
        }
        if let Some(color) = colour_in(poc, "color") {
            options.poc_color = color;
        }
    }
    if let Some(high) = line("vahLines") {
        if let Some(shown) = high.get("visible").and_then(Value::as_bool) {
            options.value_area_lines = shown;
        }
        if let Some(color) = colour_in(high, "color") {
            options.value_area_color = color;
        }
    }
}

/// What a measurement's label says.
fn measure(raw: &RawDrawing, style: &mut DrawingStyle) {
    let options = &mut style.measure;
    for (key, shown) in [
        ("showPriceRange", &mut options.price),
        ("showPercentPriceRange", &mut options.percent),
        ("showPipsPriceRange", &mut options.ticks),
        ("showBarsRange", &mut options.bars),
        ("showDateTimeRange", &mut options.time),
        ("showVolume", &mut options.volume),
    ] {
        if let Some(value) = raw.flag(key) {
            *shown = value;
        }
    }
    if let Some(color) = colour(raw, &["labelBackgroundColor"]) {
        options.label = color;
    }
}

fn position(raw: &RawDrawing, style: &mut DrawingStyle) {
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

/// A fib tool's own levels: `level1`…`level24`, as `[ratio, colour, shown]` or as an object.
fn levels(raw: &RawDrawing, style: &mut DrawingStyle) {
    let found: Vec<FibLevel> = (1..=24)
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
    if !found.is_empty() {
        own.levels = Levels::from_levels(&found);
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
