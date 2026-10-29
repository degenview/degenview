//! `plot`, `hline`, `plotshape`, `plotchar`, `plotcandle`, `fill`, `bgcolor` and `barcolor`,
//! ported from `PineRuntimeSession+Visuals.swift`. Each call site owns one output keyed by `site`
//! that grows by one sample per bar; the session evaluates the arguments in source order and calls
//! [`call`] with the site key. The first call fixes a plot's static attributes (title, color, style,
//! line width); later bars only append samples.

use std::collections::HashMap;

use super::builtins;
use super::drawings::{Arg, DEFAULT_COLOR, FromName, bind, parse};
use super::outputs::{
    ArrowOutput, CandleBar, CandleOutput, ColorOutput, DISPLAY_ALL, FillGradient, FillOutput,
    HorizontalLine, MarkerKind, MarkerLocation, MarkerOutput, MarkerShape, PlotOutput, PlotStyle,
    Size,
};
use super::runtime_value::{RefKind, RuntimeValue, color_value};
use super::state::RuntimeState;
use super::token::{Diagnostic, DiagnosticCategory, SourceRange};

const CANDLE_UP: u32 = 0x26a6_9aff;
const CANDLE_DOWN: u32 = 0xef53_50ff;
const DEFAULT_FILL_TRANSPARENCY: f64 = 90.0;

impl FromName for MarkerShape {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "shape.xcross" => Some(Self::Xcross),
            "shape.cross" => Some(Self::Cross),
            "shape.circle" => Some(Self::Circle),
            "shape.triangleup" => Some(Self::TriangleUp),
            "shape.triangledown" => Some(Self::TriangleDown),
            "shape.flag" => Some(Self::Flag),
            "shape.arrowup" => Some(Self::ArrowUp),
            "shape.arrowdown" => Some(Self::ArrowDown),
            "shape.labelup" => Some(Self::LabelUp),
            "shape.labeldown" => Some(Self::LabelDown),
            "shape.square" => Some(Self::Square),
            "shape.diamond" => Some(Self::Diamond),
            _ => None,
        }
    }
}

impl FromName for MarkerLocation {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "location.abovebar" => Some(Self::AboveBar),
            "location.belowbar" => Some(Self::BelowBar),
            "location.top" => Some(Self::Top),
            "location.bottom" => Some(Self::Bottom),
            "location.absolute" => Some(Self::Absolute),
            _ => None,
        }
    }
}

/// Whether `name` is a visual function this module handles.
pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "plot"
            | "hline"
            | "plotshape"
            | "plotchar"
            | "plotarrow"
            | "plotcandle"
            | "plotbar"
            | "fill"
            | "bgcolor"
            | "barcolor"
    )
}

/// Runs one visual call for call-site key `site`.
pub fn call(
    name: &str,
    args: &[Arg],
    site: usize,
    state: &mut RuntimeState,
    range: SourceRange,
) -> Result<RuntimeValue, Diagnostic> {
    match name {
        "plot" => plot(args, site, state),
        "hline" => hline(args, site, state),
        "plotshape" | "plotchar" => plot_marker(name, args, site, state),
        "plotarrow" => plot_arrow(args, site, state),
        "fill" => fill(args, site, state),
        "plotcandle" | "plotbar" => plot_ohlc(name, args, site, state),
        "bgcolor" | "barcolor" => color_series(name, args, site, state),
        _ => Err(unknown_function(name, range)),
    }
}

fn unknown_function(name: &str, range: SourceRange) -> Diagnostic {
    Diagnostic::error(
        "PINE4007",
        DiagnosticCategory::Runtime,
        format!("Unknown or unsupported function '{name}'."),
        range,
    )
}

fn text(bound: &HashMap<String, RuntimeValue>, key: &str) -> Option<String> {
    bound
        .get(key)
        .and_then(RuntimeValue::text_value)
        .map(str::to_owned)
}

fn width(value: Option<&RuntimeValue>, default: i64) -> usize {
    value
        .and_then(RuntimeValue::int_value)
        .unwrap_or(default)
        .max(0) as usize
}

/// A boolean argument: true only when the script gave `true`.
fn flag(value: Option<&RuntimeValue>) -> bool {
    matches!(value, Some(RuntimeValue::Bool(true)))
}

/// `show_last`: how many of the last bars to keep, when the script asked for a positive count.
fn last_bars(value: Option<&RuntimeValue>) -> Option<usize> {
    value
        .and_then(RuntimeValue::int_value)
        .filter(|count| *count > 0)
        .map(|count| count as usize)
}

fn display_mask(value: Option<&RuntimeValue>) -> u32 {
    value
        .and_then(RuntimeValue::int_value)
        .map(|value| value as u32)
        .unwrap_or(DISPLAY_ALL)
}

fn plot_style(name: Option<&str>) -> PlotStyle {
    match name {
        Some("plot.style_stepline" | "plot.style_steplinebr") => PlotStyle::Stepline,
        Some("plot.style_stepline_diamond") => PlotStyle::SteplineDiamond,
        Some("plot.style_histogram") => PlotStyle::Histogram,
        Some("plot.style_columns") => PlotStyle::Columns,
        Some("plot.style_area" | "plot.style_areabr") => PlotStyle::Area,
        Some("plot.style_circles") => PlotStyle::Circles,
        Some("plot.style_cross") => PlotStyle::Cross,
        // `linebr` too: a line already ends at an `na` bar.
        _ => PlotStyle::Line,
    }
}

fn plot(args: &[Arg], site: usize, state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "series",
            "title",
            "color",
            "linewidth",
            "style",
            "trackprice",
            "histbase",
            "offset",
        ],
    );
    let color = color_value(b.get("color"), Some(DEFAULT_COLOR)).unwrap_or(0);
    let output = state.plots.entry(site).or_insert_with(|| PlotOutput {
        id: site,
        title: text(&b, "title"),
        values: Vec::new(),
        color,
        line_width: width(b.get("linewidth"), 1),
        style: plot_style(b.get("style").and_then(RuntimeValue::text_value)),
        colors: Vec::new(),
        display: DISPLAY_ALL,
        hist_base: 0.0,
        offset: 0,
        force_overlay: false,
    });
    output
        .values
        .push(b.get("series").and_then(RuntimeValue::number));
    output.colors.push(Some(color));
    output.display = display_mask(b.get("display"));
    output.offset = b
        .get("offset")
        .and_then(RuntimeValue::int_value)
        .unwrap_or(0);
    output.force_overlay = flag(b.get("force_overlay"));
    output.hist_base = b
        .get("histbase")
        .and_then(RuntimeValue::number)
        .unwrap_or(0.0);
    Ok(RuntimeValue::Ref(RefKind::Plot, site))
}

fn hline(args: &[Arg], site: usize, state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(args, &["price", "title", "color"]);
    if let Some(price) = b.get("price").and_then(RuntimeValue::number) {
        state.hlines.insert(
            site,
            HorizontalLine {
                id: site,
                value: price,
                color: color_value(b.get("color"), Some(DEFAULT_COLOR)).unwrap_or(0),
                title: text(&b, "title"),
            },
        );
    }
    Ok(RuntimeValue::Void)
}

fn plot_marker(
    name: &str,
    args: &[Arg],
    site: usize,
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let is_shape = name == "plotshape";
    let kind_parameter = if is_shape { "style" } else { "char" };
    let b = bind(
        args,
        &[
            "series",
            "title",
            kind_parameter,
            "location",
            "color",
            "offset",
            "text",
            "textcolor",
            "editable",
            "size",
            "show_last",
        ],
    );
    let color = color_value(b.get("color"), Some(DEFAULT_COLOR)).unwrap_or(0);
    // The reference gives no default for `textcolor` here; the default plot blue is assumed.
    let text_color = color_value(b.get("textcolor"), Some(DEFAULT_COLOR));
    let marker = state.markers.entry(site).or_insert_with(|| MarkerOutput {
        id: site,
        kind: if is_shape {
            MarkerKind::Shape
        } else {
            MarkerKind::Character
        },
        title: text(&b, "title"),
        values: Vec::new(),
        character: if is_shape { None } else { text(&b, "char") },
        color,
        location: parse(
            b.get("location").and_then(RuntimeValue::text_value),
            MarkerLocation::AboveBar,
            MarkerLocation::AboveBar,
        ),
        style: parse(
            b.get("style").and_then(RuntimeValue::text_value),
            MarkerShape::TriangleUp,
            MarkerShape::TriangleUp,
        ),
        prices: Vec::new(),
        colors: Vec::new(),
        size: parse(
            b.get("size").and_then(RuntimeValue::text_value),
            Size::Auto,
            Size::Auto,
        ),
        texts: Vec::new(),
        text_colors: Vec::new(),
        offset: 0,
        show_last: None,
        display: DISPLAY_ALL,
        force_overlay: false,
    });
    let sample = b.get("series").cloned().unwrap_or(RuntimeValue::Na);
    marker
        .values
        .push(sample.bool().unwrap_or(sample.number().is_some()));
    marker.prices.push(sample.number());
    marker.colors.push(Some(color));
    marker
        .texts
        .push(text(&b, "text").filter(|text| !text.is_empty()));
    marker.text_colors.push(text_color);
    marker.offset = b
        .get("offset")
        .and_then(RuntimeValue::int_value)
        .unwrap_or(0);
    marker.show_last = last_bars(b.get("show_last"));
    marker.display = display_mask(b.get("display"));
    marker.force_overlay = flag(b.get("force_overlay"));
    Ok(RuntimeValue::Void)
}

/// `plotarrow(series, title, colorup, colordown, offset, minheight, maxheight, editable,
/// show_last, display, format, precision, force_overlay)`.
fn plot_arrow(
    args: &[Arg],
    site: usize,
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(
        args,
        &[
            "series",
            "title",
            "colorup",
            "colordown",
            "offset",
            "minheight",
            "maxheight",
            "editable",
            "show_last",
        ],
    );
    let output = state.arrows.entry(site).or_insert_with(|| ArrowOutput {
        id: site,
        title: text(&b, "title"),
        values: Vec::new(),
        up_colors: Vec::new(),
        down_colors: Vec::new(),
        offset: 0,
        show_last: None,
        min_height: 5.0,
        max_height: 100.0,
        display: DISPLAY_ALL,
        force_overlay: false,
    });
    output
        .values
        .push(b.get("series").and_then(RuntimeValue::number));
    output
        .up_colors
        .push(color_value(b.get("colorup"), Some(DEFAULT_COLOR)));
    output.down_colors.push(color_value(
        b.get("colordown"),
        builtins::color("color.red"),
    ));
    output.offset = b
        .get("offset")
        .and_then(RuntimeValue::int_value)
        .unwrap_or(0);
    output.show_last = last_bars(b.get("show_last"));
    output.min_height = b
        .get("minheight")
        .and_then(RuntimeValue::number)
        .unwrap_or(5.0)
        .max(0.0);
    output.max_height = b
        .get("maxheight")
        .and_then(RuntimeValue::number)
        .unwrap_or(100.0)
        .max(output.min_height);
    output.display = display_mask(b.get("display"));
    output.force_overlay = flag(b.get("force_overlay"));
    Ok(RuntimeValue::Void)
}

fn fill(args: &[Arg], site: usize, state: &mut RuntimeState) -> Result<RuntimeValue, Diagnostic> {
    let is_gradient = args
        .iter()
        .any(|arg| arg.name.as_deref() == Some("top_value"))
        || args.iter().filter(|arg| arg.name.is_none()).count() >= 5;
    let parameters: &[&str] = if is_gradient {
        &[
            "plot1",
            "plot2",
            "top_value",
            "bottom_value",
            "top_color",
            "bottom_color",
            "title",
        ]
    } else {
        &["plot1", "plot2", "color", "title"]
    };
    let b = bind(args, parameters);
    let (Some(plot1), Some(plot2)) = (b.get("plot1"), b.get("plot2")) else {
        return Ok(RuntimeValue::Void);
    };
    let (RuntimeValue::Ref(RefKind::Plot, first), RuntimeValue::Ref(RefKind::Plot, second)) =
        (plot1, plot2)
    else {
        return Ok(RuntimeValue::Void);
    };
    let (first, second) = (*first, *second);
    let output = state.fills.entry(site).or_insert_with(|| FillOutput {
        id: site,
        plot_a: first,
        plot_b: second,
        colors: Vec::new(),
        title: text(&b, "title"),
        gradients: Vec::new(),
    });
    if is_gradient {
        output.colors.push(None);
        output.gradients.push(gradient(&b));
    } else {
        let fallback = builtins::with_transparency(DEFAULT_COLOR, DEFAULT_FILL_TRANSPARENCY);
        output
            .colors
            .push(color_value(b.get("color"), Some(fallback)));
    }
    Ok(RuntimeValue::Void)
}

fn gradient(b: &HashMap<String, RuntimeValue>) -> Option<FillGradient> {
    let top = b.get("top_value").and_then(RuntimeValue::number)?;
    let bottom = b.get("bottom_value").and_then(RuntimeValue::number)?;
    let top_color = color_value(b.get("top_color"), None)?;
    let bottom_color = color_value(b.get("bottom_color"), None)?;
    Some(FillGradient {
        top,
        bottom,
        top_color,
        bottom_color,
    })
}

/// `plotcandle()` and `plotbar()`: one OHLC sample per bar from the script's own values, whatever
/// the chart's type is. The outputs are the script's: the renderer draws candles or bars as asked
/// and never as the chart's main series is drawn. A bar with any of the four values `na` draws
/// nothing.
fn plot_ohlc(
    name: &str,
    args: &[Arg],
    site: usize,
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let is_bar = name == "plotbar";
    let b = if is_bar {
        bind(
            args,
            &[
                "open",
                "high",
                "low",
                "close",
                "title",
                "color",
                "editable",
                "show_last",
                "display",
                "format",
                "precision",
                "force_overlay",
            ],
        )
    } else {
        bind(
            args,
            &[
                "open",
                "high",
                "low",
                "close",
                "title",
                "color",
                "wickcolor",
                "editable",
                "show_last",
                "bordercolor",
                "display",
            ],
        )
    };
    let output = state.candles.entry(site).or_insert_with(|| CandleOutput {
        id: site,
        title: text(&b, "title"),
        bars: Vec::new(),
        display: DISPLAY_ALL,
        bar_style: is_bar,
        editable: true,
        show_last: None,
        force_overlay: false,
    });
    output.display = display_mask(b.get("display"));
    output.editable = !matches!(b.get("editable"), Some(RuntimeValue::Bool(false)));
    output.show_last = last_bars(b.get("show_last"));
    output.force_overlay = flag(b.get("force_overlay"));
    let bar = match (
        b.get("open").and_then(RuntimeValue::number),
        b.get("high").and_then(RuntimeValue::number),
        b.get("low").and_then(RuntimeValue::number),
        b.get("close").and_then(RuntimeValue::number),
    ) {
        (Some(open), Some(high), Some(low), Some(close)) => {
            let body = color_value(
                b.get("color"),
                Some(if close >= open {
                    CANDLE_UP
                } else {
                    CANDLE_DOWN
                }),
            );
            Some(CandleBar {
                open,
                high,
                low,
                close,
                color: body,
                // A conventional bar has one colour: no wick or border of its own.
                wick_color: if is_bar {
                    body
                } else {
                    color_value(b.get("wickcolor"), body)
                },
                border_color: if is_bar {
                    body
                } else {
                    color_value(b.get("bordercolor"), body)
                },
            })
        }
        _ => None,
    };
    output.bars.push(bar);
    Ok(RuntimeValue::Void)
}

fn color_series(
    name: &str,
    args: &[Arg],
    site: usize,
    state: &mut RuntimeState,
) -> Result<RuntimeValue, Diagnostic> {
    let b = bind(args, &["color", "offset", "editable", "show_last", "title"]);
    let output = if name == "bgcolor" {
        state
            .backgrounds
            .entry(site)
            .or_insert_with(|| ColorOutput {
                id: site,
                title: text(&b, "title"),
                colors: Vec::new(),
                offset: 0,
                show_last: None,
            })
    } else {
        state.bar_colors.entry(site).or_insert_with(|| ColorOutput {
            id: site,
            title: text(&b, "title"),
            colors: Vec::new(),
            offset: 0,
            show_last: None,
        })
    };
    output.offset = b
        .get("offset")
        .and_then(RuntimeValue::int_value)
        .unwrap_or(0);
    output.show_last = last_bars(b.get("show_last"));
    output.colors.push(color_value(b.get("color"), None));
    Ok(RuntimeValue::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RuntimeValue::{Bool, Float, Na};

    fn arg(value: RuntimeValue) -> Arg {
        Arg { name: None, value }
    }

    fn named(name: &str, value: RuntimeValue) -> Arg {
        Arg {
            name: Some(name.to_owned()),
            value,
        }
    }

    fn run(
        state: &mut RuntimeState,
        name: &str,
        site: usize,
        args: &[Arg],
    ) -> Result<RuntimeValue, Diagnostic> {
        call(name, args, site, state, SourceRange::ZERO)
    }

    #[test]
    fn plot_accumulates_and_returns_its_site() {
        let mut state = RuntimeState::default();
        let first = run(&mut state, "plot", 0, &[arg(Float(1.0))]).unwrap();
        assert_eq!(first, RuntimeValue::Ref(RefKind::Plot, 0));
        run(&mut state, "plot", 0, &[arg(Float(2.0)), arg(Na)]).unwrap();
        let plot = &state.plots[&0];
        assert_eq!(plot.values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(plot.colors, vec![Some(DEFAULT_COLOR), Some(DEFAULT_COLOR)]);
        assert_eq!(plot.style, PlotStyle::Line);
    }

    #[test]
    fn plot_style_names_map_to_styles() {
        assert_eq!(plot_style(Some("plot.style_stepline")), PlotStyle::Stepline);
        assert_eq!(
            plot_style(Some("plot.style_histogram")),
            PlotStyle::Histogram
        );
        assert_eq!(plot_style(Some("plot.style_areabr")), PlotStyle::Area);
        assert_eq!(plot_style(Some("plot.style_linebr")), PlotStyle::Line);
        assert_eq!(plot_style(None), PlotStyle::Line);
    }

    #[test]
    fn hline_stores_only_a_numeric_price() {
        let mut state = RuntimeState::default();
        run(&mut state, "hline", 0, &[arg(Float(42.0))]).unwrap();
        assert_eq!(state.hlines[&0].value, 42.0);
        assert_eq!(state.hlines[&0].color, DEFAULT_COLOR);
        run(&mut state, "hline", 1, &[arg(Na)]).unwrap();
        assert!(!state.hlines.contains_key(&1));
    }

    #[test]
    fn plotshape_and_plotchar_read_text_textcolor_offset_and_show_last() {
        use RuntimeValue::{Color, Int, String as Text};
        let mut state = RuntimeState::default();
        // `series, title, style, location, color, offset, text, textcolor, editable, size,
        // show_last` by position, so a script can write the long form without names.
        let positional = [
            Bool(true),
            Text("t".into()),
            Text("shape.circle".into()),
            Text("location.belowbar".into()),
            Color(1),
            Int(2),
            Text("buy".into()),
            Color(2),
            Bool(true),
            Text("size.large".into()),
            Int(5),
        ]
        .map(arg);
        run(&mut state, "plotshape", 0, &positional).unwrap();
        let marker = &state.markers[&0];
        assert_eq!(marker.texts, [Some("buy".to_owned())]);
        assert_eq!(marker.text_colors, [Some(2)]);
        assert_eq!(marker.offset, 2);
        assert_eq!(marker.show_last, Some(5));
        assert_eq!(marker.style, MarkerShape::Circle);
        assert_eq!(marker.location, MarkerLocation::BelowBar);

        // By name, with no text: nothing to draw, and the text colour is the default blue.
        run(
            &mut state,
            "plotchar",
            1,
            &[arg(Bool(true)), named("char", Text("x".into()))],
        )
        .unwrap();
        let marker = &state.markers[&1];
        assert_eq!(marker.texts, [None]);
        assert_eq!(marker.text_colors, [Some(DEFAULT_COLOR)]);
        assert_eq!(marker.offset, 0);
        assert_eq!(marker.show_last, None);
    }

    #[test]
    fn plotshape_records_values_and_prices() {
        let mut state = RuntimeState::default();
        run(&mut state, "plotshape", 0, &[arg(Bool(true))]).unwrap();
        run(&mut state, "plotshape", 0, &[arg(Float(1.0))]).unwrap();
        let marker = &state.markers[&0];
        assert_eq!(marker.kind, MarkerKind::Shape);
        assert_eq!(marker.values, vec![true, true]);
        assert_eq!(marker.prices, vec![None, Some(1.0)]);
        assert_eq!(marker.style, MarkerShape::TriangleUp);
    }

    #[test]
    fn fill_flat_and_gradient_forms() {
        let mut state = RuntimeState::default();
        let p1 = run(&mut state, "plot", 0, &[arg(Float(1.0))]).unwrap();
        let p2 = run(&mut state, "plot", 1, &[arg(Float(2.0))]).unwrap();
        run(&mut state, "fill", 0, &[arg(p1.clone()), arg(p2.clone())]).unwrap();
        let flat = &state.fills[&0];
        assert!(flat.colors[0].is_some());
        assert!(flat.gradients.is_empty(), "a flat fill grows only `colors`");

        run(
            &mut state,
            "fill",
            1,
            &[
                arg(p1),
                arg(p2),
                arg(Float(1.0)),
                arg(Float(0.0)),
                arg(RuntimeValue::Color(0xFF00_00FF)),
                arg(RuntimeValue::Color(0x0000_00FF)),
            ],
        )
        .unwrap();
        let gradient = &state.fills[&1];
        assert!(gradient.colors[0].is_none());
        assert_eq!(gradient.gradients[0].unwrap().top, 1.0);
        assert_eq!(gradient.gradients[0].unwrap().top_color, 0xFF00_00FF);
    }

    #[test]
    fn plotcandle_appends_bars_or_none() {
        let mut state = RuntimeState::default();
        run(
            &mut state,
            "plotcandle",
            0,
            &[
                arg(Float(1.0)),
                arg(Float(2.0)),
                arg(Float(0.5)),
                arg(Float(1.5)),
            ],
        )
        .unwrap();
        run(&mut state, "plotcandle", 0, &[arg(Na)]).unwrap();
        let candle = &state.candles[&0];
        assert_eq!(candle.bars.len(), 2);
        assert!(candle.bars[0].is_some());
        assert!(candle.bars[1].is_none());
    }

    #[test]
    fn bgcolor_and_barcolor_append_optional_colors() {
        let mut state = RuntimeState::default();
        run(
            &mut state,
            "bgcolor",
            0,
            &[arg(RuntimeValue::Color(0xFF00_00FF))],
        )
        .unwrap();
        run(&mut state, "barcolor", 0, &[arg(Na)]).unwrap();
        assert_eq!(state.backgrounds[&0].colors, vec![Some(0xFF00_00FF)]);
        assert_eq!(state.bar_colors[&0].colors, vec![None]);
    }

    fn ohlc(open: f64, high: f64, low: f64, close: f64) -> Vec<Arg> {
        vec![
            arg(Float(open)),
            arg(Float(high)),
            arg(Float(low)),
            arg(Float(close)),
        ]
    }

    #[test]
    fn plotbar_is_its_own_output_kind_with_no_wick_or_border_of_its_own() {
        let mut state = RuntimeState::default();
        let mut args = ohlc(1.0, 2.0, 0.5, 1.5);
        args.push(named("color", RuntimeValue::Color(0x1122_33FF)));
        run(&mut state, "plotbar", 0, &args).unwrap();
        run(&mut state, "plotcandle", 1, &ohlc(1.0, 2.0, 0.5, 1.5)).unwrap();
        let bar = &state.candles[&0];
        assert!(bar.bar_style);
        let drawn = bar.bars[0].as_ref().unwrap();
        assert_eq!(drawn.color, Some(0x1122_33FF));
        assert_eq!(drawn.wick_color, drawn.color);
        assert!(!state.candles[&1].bar_style);
    }

    #[test]
    fn plotbar_with_a_missing_value_draws_nothing_and_keeps_its_slot() {
        let mut state = RuntimeState::default();
        run(&mut state, "plotbar", 0, &ohlc(1.0, 2.0, 0.5, 1.5)).unwrap();
        let mut gap = ohlc(1.0, 2.0, 0.5, 1.5);
        gap[1] = arg(Na);
        run(&mut state, "plotbar", 0, &gap).unwrap();
        assert_eq!(state.candles[&0].bars.len(), 2);
        assert!(state.candles[&0].bars[1].is_none());
    }

    #[test]
    fn dynamic_candle_colours_are_kept_per_bar_and_the_values_are_not_normalised() {
        let mut state = RuntimeState::default();
        let mut up = ohlc(1.0, 0.2, 3.0, 2.0);
        up.push(named("color", RuntimeValue::Color(0x00FF_00FF)));
        up.push(named("wickcolor", RuntimeValue::Color(0x0000_FFFF)));
        run(&mut state, "plotcandle", 0, &up).unwrap();
        let mut down = ohlc(2.0, 3.0, 1.0, 1.0);
        down.push(named("color", RuntimeValue::Color(0xFF00_0080)));
        run(&mut state, "plotcandle", 0, &down).unwrap();
        let bars = &state.candles[&0].bars;
        let first = bars[0].as_ref().unwrap();
        // Contradictory high and low stay as the script wrote them; the renderer widens them.
        assert_eq!((first.high, first.low), (0.2, 3.0));
        assert_eq!(first.color, Some(0x00FF_00FF));
        assert_eq!(first.wick_color, Some(0x0000_FFFF));
        assert_eq!(bars[1].as_ref().unwrap().color, Some(0xFF00_0080));
    }

    #[test]
    fn show_last_keeps_only_the_newest_bars_of_candles_and_colours() {
        let mut state = RuntimeState::default();
        for _ in 0..4 {
            let mut args = ohlc(1.0, 2.0, 0.5, 1.5);
            args.push(named("show_last", RuntimeValue::Int(2)));
            run(&mut state, "plotcandle", 0, &args).unwrap();
            run(
                &mut state,
                "barcolor",
                1,
                &[
                    arg(RuntimeValue::Color(0xFF00_00FF)),
                    named("show_last", RuntimeValue::Int(1)),
                ],
            )
            .unwrap();
        }
        let candles = state.candles[&0].clone().finished();
        assert_eq!(
            candles.bars.iter().map(Option::is_some).collect::<Vec<_>>(),
            vec![false, false, true, true]
        );
        let colours = state.bar_colors[&1].clone().finished();
        assert_eq!(colours.colors, vec![None, None, None, Some(0xFF00_00FF)]);
    }

    #[test]
    fn barcolor_offset_shifts_colours_to_later_bars() {
        let mut state = RuntimeState::default();
        for color in [0xFF00_00FF_u32, 0x00FF_00FF, 0x0000_FFFF] {
            run(
                &mut state,
                "barcolor",
                0,
                &[
                    arg(RuntimeValue::Color(color)),
                    named("offset", RuntimeValue::Int(1)),
                ],
            )
            .unwrap();
        }
        let shifted = state.bar_colors[&0].clone().finished();
        assert_eq!(
            shifted.colors,
            vec![None, Some(0xFF00_00FF), Some(0x00FF_00FF)],
            "the last bar's colour has no later bar to land on"
        );
    }

    #[test]
    fn force_overlay_and_editable_are_recorded() {
        let mut state = RuntimeState::default();
        let mut args = ohlc(1.0, 2.0, 0.5, 1.5);
        args.push(named("force_overlay", Bool(true)));
        args.push(named("editable", Bool(false)));
        run(&mut state, "plotcandle", 0, &args).unwrap();
        assert!(state.candles[&0].force_overlay);
        assert!(!state.candles[&0].editable);
        run(
            &mut state,
            "plot",
            1,
            &[arg(Float(1.0)), named("force_overlay", Bool(true))],
        )
        .unwrap();
        assert!(state.plots[&1].force_overlay);
    }
}
