//! The wider set of drawing tools over a real loopback socket: levels, rays, arrows, Fibonacci
//! retracements and text labels, next to the trend lines and rectangles that were there first.
//! The stub chart is workspace 1, chart 2, thirty daily bars from 2025-10-20.

use super::{Harness, call, error_code, rpc};
use crate::model::mcp::Permission;
use serde_json::{Value, json};

const DRAW: &[Permission] = &[
    Permission::ChartsCandlesRead,
    Permission::DrawingsRead,
    Permission::DrawingsWrite,
    Permission::DrawingsDelete,
];

fn chart() -> Value {
    json!({"workspace_id": "1", "chart_id": "2"})
}

fn with(base: Value, extra: Value) -> Value {
    let mut out = base;
    out.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    out
}

fn add(url: &str, drawings: Value) -> Value {
    call(
        url,
        "tok",
        "degenview_add_drawings",
        with(chart(), json!({"drawings": drawings})),
    )
}

fn rejected(url: &str, name: &str, arguments: Value) -> bool {
    let reply = rpc(
        url,
        Some("tok"),
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    );
    let json = reply.json();
    json.get("error").is_some() || json["result"]["isError"] == json!(true)
}

fn listed(url: &str) -> Vec<Value> {
    let result = call(url, "tok", "degenview_list_drawings", chart());
    result["structuredContent"]["drawings"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn every_new_tool_can_be_drawn_listed_and_is_editable() {
    let harness = Harness::start(&[("c", "tok", DRAW)]);
    let url = harness.url();
    let a = json!({"time": "2025-10-25T00:00:00Z", "price": 104.0});
    let b = json!({"time": "2025-11-05T00:00:00Z", "price": 112.0});
    let added = add(
        &url,
        json!([
            {"tool": "ray", "a": a, "b": b, "color": "#FF0000"},
            {"tool": "extended_line", "a": a, "b": b},
            {"tool": "arrow", "a": a, "b": b, "width": 3.0},
            {"tool": "fib_retracement", "a": a, "b": b},
            {"tool": "horizontal_line", "a": a, "dash": "dashed"},
            {"tool": "horizontal_ray", "a": a},
            {"tool": "vertical_line", "a": a},
            {"tool": "crossline", "a": a},
            {"tool": "text", "a": a, "text": "Breakout", "color": "#00FF00"},
        ]),
    );
    assert_eq!(added["isError"], json!(false), "{added}");
    assert_eq!(
        added["structuredContent"]["drawing_ids"]
            .as_array()
            .unwrap()
            .len(),
        9
    );

    let drawings = listed(&url);
    let tools: Vec<_> = drawings
        .iter()
        .map(|d| d["tool"].as_str().unwrap())
        .collect();
    assert_eq!(
        tools,
        [
            "ray",
            "extended_line",
            "arrow",
            "fib_retracement",
            "horizontal_line",
            "horizontal_ray",
            "vertical_line",
            "crossline",
            "text"
        ]
    );
    let anchors: Vec<_> = drawings
        .iter()
        .map(|d| d["anchor_count"].as_u64().unwrap())
        .collect();
    assert_eq!(anchors, [2, 2, 2, 2, 1, 1, 1, 1, 1]);
    assert!(
        drawings.iter().all(|d| d["editable"] == json!(true)),
        "{drawings:?}"
    );
    assert_eq!(drawings[0]["style"]["color"], "#FF0000FF");
    assert_eq!(drawings[2]["style"]["width"], 3.0);
    assert_eq!(drawings[4]["style"]["dash"], "dashed");
    assert_eq!(drawings[8]["text"], "Breakout");
    // The bar's open: the time snapped as for any drawing.
    assert_eq!(drawings[4]["anchors"][0]["time"], "2025-10-25T00:00:00Z");
    assert_eq!(drawings[4]["anchors"][0]["price"], 104.0);
}

#[test]
fn a_level_a_ray_and_a_label_can_be_moved_and_restyled() {
    let harness = Harness::start(&[("c", "tok", DRAW)]);
    let url = harness.url();
    let a = json!({"time": "2025-10-25T00:00:00Z", "price": 104.0});
    let b = json!({"time": "2025-11-05T00:00:00Z", "price": 112.0});
    add(
        &url,
        json!([
            {"tool": "horizontal_line", "a": a},
            {"tool": "ray", "a": a, "b": b},
            {"tool": "text", "a": a, "text": "One"},
        ]),
    );
    let edit = |id: &str, extra: Value| {
        call(
            &url,
            "tok",
            "degenview_update_drawing",
            with(with(chart(), json!({"drawing_id": id})), extra),
        )
    };
    let moved = edit(
        "1",
        json!({"a": {"time": "2025-10-25T00:00:00Z", "price": 130.0}}),
    );
    assert_eq!(moved["isError"], json!(false), "{moved}");
    assert_eq!(
        moved["structuredContent"]["drawings"][0]["anchors"][0]["price"],
        130.0
    );

    let recolored = edit(
        "2",
        json!({"color": "#123456", "b": {"time": "2025-11-10T00:00:00Z", "price": 90.0}}),
    );
    assert_eq!(
        recolored["structuredContent"]["drawings"][0]["style"]["color"],
        "#123456FF"
    );
    assert_eq!(
        recolored["structuredContent"]["drawings"][0]["anchors"][1]["price"],
        90.0
    );

    let relabeled = edit("3", json!({"text": "Two"}));
    assert_eq!(relabeled["structuredContent"]["drawings"][0]["text"], "Two");

    // A one-point drawing has no `b`, and a label cannot lose its text.
    let second_point = edit(
        "1",
        json!({"b": {"time": "2025-11-10T00:00:00Z", "price": 90.0}}),
    );
    assert_eq!(error_code(&second_point), "INVALID_ARGUMENT");
    let emptied = edit("3", json!({"text": ""}));
    assert_eq!(error_code(&emptied), "INVALID_ARGUMENT");
    let filled = edit("2", json!({"fill_color": "#FF0000"}));
    assert_eq!(error_code(&filled), "INVALID_ARGUMENT");
    let extended = edit("2", json!({"extend_left": true}));
    assert_eq!(error_code(&extended), "INVALID_ARGUMENT");
}

#[test]
fn a_tool_gets_only_the_points_and_styles_it_takes() {
    let harness = Harness::start(&[("c", "tok", DRAW)]);
    let url = harness.url();
    let a = json!({"time": "2025-10-25T00:00:00Z", "price": 104.0});
    let b = json!({"time": "2025-11-05T00:00:00Z", "price": 112.0});
    let one = |drawing: Value| with(chart(), json!({"drawings": [drawing]}));
    for drawing in [
        // A second point where there is one, and none where there are two.
        json!({"tool": "horizontal_line", "a": a, "b": b}),
        json!({"tool": "text", "a": a, "b": b, "text": "x"}),
        json!({"tool": "ray", "a": a}),
        json!({"tool": "fib_retracement", "a": a}),
        json!({"tool": "arrow", "a": a, "b": a}),
        // A label without text, or with only blanks.
        json!({"tool": "text", "a": a}),
        json!({"tool": "text", "a": a, "text": "   "}),
        // Styles the tool does not have.
        json!({"tool": "ray", "a": a, "b": b, "fill_color": "#FF0000"}),
        json!({"tool": "ray", "a": a, "b": b, "text": "x"}),
        json!({"tool": "arrow", "a": a, "b": b, "extend_right": true}),
        json!({"tool": "horizontal_line", "a": a, "text": "x"}),
        json!({"tool": "text", "a": a, "text": "x", "fill_color": "#FF0000"}),
        json!({"tool": "ellipse", "a": a, "b": b}),
        json!({"tool": "fib_extension", "a": a, "b": b}),
    ] {
        assert!(
            rejected(&url, "degenview_add_drawings", one(drawing.clone())),
            "{drawing}"
        );
    }
    assert!(listed(&url).is_empty(), "no refused call drew anything");
    // The old pair still needs two points and keeps its own styles.
    let ok = add(
        &url,
        json!([{"tool": "rectangle", "a": a, "b": {"time": "2025-11-05T00:00:00Z", "price": 99.0}, "text": "Zone", "fill_color": "#00FF00"}]),
    );
    assert_eq!(ok["isError"], json!(false), "{ok}");
}

#[test]
fn drawings_of_other_tools_are_still_listed_read_only() {
    let harness = Harness::start(&[("c", "tok", DRAW)]);
    {
        let mut chart = harness.stub_chart();
        let a = crate::model::Anchor {
            time_ms: 1_761_000_000_000,
            price: 100.0,
        };
        let b = crate::model::Anchor {
            time_ms: 1_762_000_000_000,
            price: 110.0,
        };
        chart.drawings.push(
            crate::model::Drawing::build(
                crate::model::Tool::Ellipse,
                7,
                &[a, b],
                crate::model::Tool::Ellipse.default_style(),
            )
            .unwrap(),
        );
        chart.next_id = 8;
    }
    let url = harness.url();
    let drawings = listed(&url);
    assert_eq!(drawings[0]["tool"], "ellipse");
    assert_eq!(drawings[0]["editable"], json!(false));
    let edit = call(
        &url,
        "tok",
        "degenview_update_drawing",
        with(chart(), json!({"drawing_id": "7", "color": "#FF0000"})),
    );
    assert_eq!(error_code(&edit), "UNSUPPORTED");
    // Any drawing can still be deleted.
    let deleted = call(
        &url,
        "tok",
        "degenview_delete_drawings",
        with(chart(), json!({"drawing_ids": ["7"]})),
    );
    assert_eq!(deleted["isError"], json!(false), "{deleted}");
}
