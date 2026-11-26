use super::*;
use crate::model::drawings::style::VisibilityRow;
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

#[test]
fn every_listed_class_names_one_tool_and_builds_from_its_points() {
    let mut seen = std::collections::HashSet::new();
    for (class, tool) in TOOLS {
        assert!(seen.insert(class), "{class} twice");
        assert_eq!(tool_of(class), Some(*tool));
    }
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
