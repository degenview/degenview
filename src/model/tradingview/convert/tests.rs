use super::*;
use crate::model::chart::kind::PriceSource;
use crate::model::drawings::style::{HorizontalAlign, StatsPosition, VerticalAlign, VisibilityRow};
use crate::model::tradingview::fixtures::{self, T0, drawing};
use serde_json::{Value, json};

fn raw(source: &Value) -> RawDrawing {
    RawDrawing::read(source["id"].as_str().unwrap(), source).expect("a drawing")
}

fn shape(drawing: &Drawing) -> (&Geometry, &DrawingStyle, &str) {
    match drawing {
        Drawing::Shape {
            geometry,
            style,
            text,
            ..
        } => (geometry, style, text),
        other => panic!("not a shape: {other:?}"),
    }
}

#[test]
fn a_trend_line_keeps_its_points_and_look() {
    let converted = convert(&raw(&fixtures::trend_line("a")), 7, None).unwrap();
    assert_eq!(converted.drawing.id(), 7);
    let (geometry, style, _) = shape(&converted.drawing);
    let Geometry::TrendLine { a, b } = geometry else {
        panic!("{geometry:?}");
    };
    assert_eq!((a.time_ms, a.price), (T0 * 1000, 100.0));
    assert_eq!(b.time_ms, (T0 + 3_600) * 1000);
    assert_eq!(style.stroke.color, [255, 235, 59, 255]);
    assert_eq!(style.stroke.width, 3.0);
    assert_eq!(style.stroke.dash, LineDash::Dashed);
}

#[test]
fn a_point_past_the_last_bar_is_placed_by_its_offset() {
    let mut source = fixtures::trend_line("a");
    source["state"]["points"][1]["offset"] = json!(3);
    let converted = convert(&raw(&source), 1, None).unwrap();
    let (Geometry::TrendLine { b, .. }, ..) = shape(&converted.drawing) else {
        panic!();
    };
    assert_eq!(b.time_ms, (T0 + 3_600 + 3 * 3_600) * 1000);
}

fn position_source(extra: Value) -> Value {
    let mut props = json!({
        "stopLevel": 72678, "profitLevel": 145356, "accountSize": 1000.0, "risk": 25.0,
        "qty": 0.3439830485153697, "amountStop": 750.0,
        "entryPointCurrencyRate": 1.0, "closePointCurrencyRate": 1.0,
    });
    props
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap_or_default());
    drawing(
        "p",
        "LineToolRiskRewardLong",
        "COINBASE:BTCUSD",
        props,
        &[
            (0, 78_707.61),
            (19, 78_707.61),
            (3, 78_707.61),
            (13, 77_980.83),
        ],
    )
}

fn levels(converted: &Converted) -> (f64, f64, f64) {
    let (
        Geometry::Position {
            entry,
            target,
            stop,
            ..
        },
        ..,
    ) = shape(&converted.drawing)
    else {
        panic!();
    };
    (entry.price, target.price, stop.price)
}

#[test]
fn a_position_is_measured_in_price_steps() {
    let converted = convert(&raw(&position_source(json!({}))), 1, Some(0.01)).unwrap();
    let (entry, target, stop) = levels(&converted);
    assert!((entry - stop - 726.78).abs() < 1e-6, "{stop}");
    assert!((target - entry - 1453.56).abs() < 1e-6, "{target}");
}

#[test]
fn without_a_price_step_a_position_follows_from_its_risk_figures() {
    let converted = convert(&raw(&position_source(json!({}))), 1, None).unwrap();
    let (entry, target, stop) = levels(&converted);
    assert!((entry - stop - 726.78).abs() < 1e-6, "{stop}");
    assert!((target - entry - 1453.56).abs() < 1e-6, "{target}");
}

#[test]
fn a_position_nothing_measures_is_left_out() {
    let source = position_source(json!({"entryPointCurrencyRate": 0.9}));
    assert_eq!(convert(&raw(&source), 1, None), Err(Unsupported::PriceStep));
}

#[test]
fn what_has_no_tool_says_why() {
    let emoji = drawing("e", "LineToolEmoji", "A:B", json!({}), &[(0, 1.0)]);
    assert_eq!(convert(&raw(&emoji), 1, None), Err(Unsupported::Tool));
    let image = drawing("i", "LineToolImage", "A:B", json!({}), &[(0, 1.0)]);
    assert_eq!(convert(&raw(&image), 1, None), Err(Unsupported::Picture));
    let short = drawing("s", "LineToolTrendLine", "A:B", json!({}), &[(0, 1.0)]);
    assert_eq!(convert(&raw(&short), 1, None), Err(Unsupported::Points));
}

#[test]
fn locked_hidden_and_timeframe_visibility_carry_over() {
    let mut source = fixtures::trend_line("a");
    let state = &mut source["state"]["state"];
    state["frozen"] = json!(true);
    state["visible"] = json!(false);
    state["intervalsVisibilities"] = json!({"minutes": false, "days": false});
    let converted = convert(&raw(&source), 1, None).unwrap();
    let Drawing::Shape {
        locked,
        hidden,
        visibility,
        ..
    } = converted.drawing
    else {
        panic!();
    };
    assert!(locked && hidden);
    assert!(!visibility.shows_row(VisibilityRow::Minutes));
    assert!(!visibility.shows_row(VisibilityRow::Days));
    assert!(visibility.shows_row(VisibilityRow::Hours));
}

#[test]
fn long_text_is_cut_and_said_to_be() {
    let long = "x".repeat(5_000);
    let source = drawing(
        "t",
        "LineToolText",
        "A:B",
        json!({"text": long}),
        &[(0, 1.0)],
    );
    assert!(convert(&raw(&source), 1, None).unwrap().cut_text);
    let short = drawing(
        "t",
        "LineToolText",
        "A:B",
        json!({"text": "hi\n"}),
        &[(0, 1.0)],
    );
    let converted = convert(&raw(&short), 1, None).unwrap();
    assert!(!converted.cut_text);
    assert_eq!(shape(&converted.drawing).2, "hi");
}

#[test]
fn a_table_keeps_its_cells() {
    let source = drawing(
        "t",
        "LineToolTable",
        "A:B",
        json!({"cells": [["a", "b"], ["c", "d"], ["e", "f"]], "columnWidths": [100, 140]}),
        &[(0, 1.0)],
    );
    let converted = convert(&raw(&source), 1, None).unwrap();
    let (Geometry::Table(table), ..) = shape(&converted.drawing) else {
        panic!();
    };
    assert_eq!((table.row_count(), table.column_count()), (3, 2));
    assert_eq!(table.cell(2, 1), "f");
}

/// Points that make a shape for any tool: each later in time, at its own price.
fn points_for(count: usize) -> Vec<(i64, f64)> {
    (0..count)
        .map(|index| {
            let step = index as f64;
            (
                index as i64 * 3,
                100.0 + step * 7.0 + (step * 1.7).sin() * 11.0,
            )
        })
        .collect()
}

#[test]
fn every_listed_class_names_one_tool_and_builds_from_its_points() {
    use crate::model::drawings::tool::Placement;
    let mut seen = std::collections::HashSet::new();
    for (class, tool) in TOOLS {
        assert!(seen.insert(class), "{class} twice");
        assert_eq!(tool_of(class), Some(*tool));
        // These take their own settings beside the points, and have tests of their own.
        if matches!(tool, Tool::Post | Tool::LongPosition | Tool::ShortPosition) {
            continue;
        }
        let count = match tool.spec().placement {
            Placement::Points(count) => usize::from(count),
            Placement::Single => 1,
            _ => 3,
        };
        let source = drawing("d", class, "A:B", json!({}), &points_for(count));
        let converted = convert(&raw(&source), 1, None);
        assert!(
            converted.is_ok(),
            "{class} from {count} points: {converted:?}"
        );
    }
}

#[test]
fn a_table_past_the_limits_is_cut_to_them_and_said_to_be() {
    let wide: Vec<String> = (0..40).map(|n| n.to_string()).collect();
    let cells: Vec<&Vec<String>> = std::iter::repeat_n(&wide, 100).collect();
    let source = drawing(
        "t",
        "LineToolTable",
        "A:B",
        json!({"cells": cells, "columnWidths": [1, 9000], "rowHeights": [0, 2]}),
        &[(0, 1.0)],
    );
    let converted = convert(&raw(&source), 1, None).unwrap();
    assert!(converted.cut_text);
    let (Geometry::Table(table), ..) = shape(&converted.drawing) else {
        panic!();
    };
    assert!(table.is_valid());
    assert_eq!((table.row_count(), table.column_count()), (30, 12));
}

/// The classes behind TradingView's own tool names, as its web app registers them.
#[test]
fn the_schiff_pitchforks_are_the_ones_tradingview_means() {
    assert_eq!(
        tool_of("LineToolSchiffPitchfork2"),
        Some(Tool::SchiffPitchfork)
    );
    assert_eq!(
        tool_of("LineToolSchiffPitchfork"),
        Some(Tool::ModifiedSchiffPitchfork)
    );
}

#[test]
fn a_pinned_drawing_is_left_out_since_its_points_are_not_where_it_is() {
    for tool in ["LineToolText", "LineToolNote", "LineToolTable"] {
        let source = drawing("p", tool, "X:Y", json!({"anchored": true}), &[(0, 1.0)]);
        assert_eq!(
            convert(&raw(&source), 1, None),
            Err(Unsupported::Pinned),
            "{tool}"
        );
    }
}

#[test]
fn a_post_takes_a_price_before_it_converts() {
    let mut source = drawing(
        "t",
        "LineToolTweet",
        "X:Y",
        json!({"position": 40.0}),
        &[(0, 7.0)],
    );
    source["state"]["tweetData"] =
        json!({"tweetUrl": "https://x.com/someone/status/1234567890123456789"});
    let post = raw(&source);
    assert_eq!(post.beside_bar(), Some(40.0));
    assert_eq!(convert(&post, 1, None), Err(Unsupported::Unplaced));
    let converted = convert(&post.at_price(150.0), 1, None).unwrap();
    let (Geometry::Card { anchor, .. }, ..) = shape(&converted.drawing) else {
        panic!("{:?}", converted.drawing);
    };
    assert_eq!(anchor.price, 150.0, "the stored 7 means nothing");
}

#[test]
fn a_lines_words_keep_the_side_of_the_line_they_were_on() {
    for (stored, expected) in [
        ("bottom", VerticalAlign::Top),
        ("middle", VerticalAlign::Middle),
        ("top", VerticalAlign::Bottom),
    ] {
        let source = drawing(
            "l",
            "LineToolTrendLine",
            "X:Y",
            json!({"vertLabelsAlign": stored, "horzLabelsAlign": "left", "text": "here"}),
            &[(0, 1.0), (1, 2.0)],
        );
        let converted = convert(&raw(&source), 1, None).unwrap();
        let (_, style, _) = shape(&converted.drawing);
        assert_eq!(style.text.valign, expected, "{stored}");
        assert_eq!(style.text.align, HorizontalAlign::Left);
    }
}

#[test]
fn a_lines_measurements_are_the_ones_shown() {
    let source = drawing(
        "l",
        "LineToolTrendLine",
        "X:Y",
        json!({
            "showPriceRange": true, "showPercentPriceRange": true, "showBarsRange": false,
            "showDateTimeRange": true, "showDistance": false, "showAngle": true,
            "alwaysShowStats": true, "statsPosition": 0,
        }),
        &[(0, 1.0), (1, 2.0)],
    );
    let converted = convert(&raw(&source), 1, None).unwrap();
    let stats = shape(&converted.drawing).1.stats;
    assert!(stats.price_range && stats.percent && stats.time && stats.angle && stats.always_show);
    assert!(!stats.bars && !stats.distance);
    assert_eq!(stats.position, StatsPosition::Left);
}

#[test]
fn an_anchored_vwap_keeps_its_source_bands_and_colours() {
    let source = drawing(
        "v",
        "LineToolAnchoredVWAP",
        "X:Y",
        json!({
            "inputs": {
                "source": "ohlc4", "Bands Calculation Mode": "Standard Deviation",
                "calculate_stDev": false, "calculate_stDev_2": true, "bands_multiplier_2": 2.5,
            },
            "styles": {
                "VWAP": {"color": "#ff0000", "linewidth": 3},
                "UpperBand_2": {"color": "#00ff00"},
            },
            "areaBackground": {"fillBackground": false},
        }),
        &[(0, 1.0)],
    );
    let converted = convert(&raw(&source), 1, None).unwrap();
    assert!(!converted.percent_bands);
    let style = shape(&converted.drawing).1;
    assert_eq!(style.stroke.color, [255, 0, 0, 255]);
    assert_eq!(style.stroke.width, 3.0);
    assert_eq!(style.vwap.source, PriceSource::Ohlc4);
    assert!(!style.vwap.bands[0].enabled && style.vwap.bands[1].enabled);
    assert_eq!(style.vwap.bands[1].multiplier, 2.5);
    assert_eq!(style.vwap.bands[1].color, [0, 255, 0, 255]);
    assert!(!style.vwap.fill);
}

#[test]
fn percentage_bands_are_off_and_said_to_be() {
    let source = drawing(
        "v",
        "LineToolAnchoredVWAP",
        "X:Y",
        json!({"inputs": {"Bands Calculation Mode": "Percentage", "calculate_stDev": true}}),
        &[(0, 1.0)],
    );
    let converted = convert(&raw(&source), 1, None).unwrap();
    assert!(converted.percent_bands);
    assert!(!shape(&converted.drawing).1.vwap.bands[0].enabled);
}

#[test]
fn a_tables_alignment_and_a_labels_weight_carry_over() {
    let table = drawing(
        "t",
        "LineToolTable",
        "X:Y",
        json!({"horzAlign": "right", "cells": [["a"]]}),
        &[(0, 1.0)],
    );
    let converted = convert(&raw(&table), 1, None).unwrap();
    assert_eq!(
        shape(&converted.drawing).1.text.align,
        HorizontalAlign::Right
    );
    let label = drawing(
        "p",
        "LineToolPriceLabel",
        "X:Y",
        json!({"fontWeight": "normal"}),
        &[(0, 1.0)],
    );
    let converted = convert(&raw(&label), 1, None).unwrap();
    assert!(!shape(&converted.drawing).1.text.bold);
}
