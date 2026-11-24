//! Timeframe, chart type and chart removal over a real loopback socket.

use super::{Harness, call, error_code, rpc, tool_names};
use crate::model::mcp::Permission;
use serde_json::{Value, json};

const VIEW: &[Permission] = &[Permission::ChartsViewWrite];
const DELETE: &[Permission] = &[Permission::ChartsDelete];

fn chart() -> Value {
    json!({"workspace_id": "1", "chart_id": "2"})
}

fn with(extra: Value) -> Value {
    let mut args = chart();
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    args
}

fn rejected(url: &str, token: &str, name: &str, arguments: Value) -> bool {
    let reply = rpc(
        url,
        Some(token),
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    );
    let json = reply.json();
    json.get("error").is_some() || json["result"]["isError"] == json!(true)
}

#[test]
fn each_chart_control_tool_needs_its_own_permission() {
    let harness = Harness::start(&[("view", "v-tok", VIEW), ("delete", "d-tok", DELETE)]);
    let url = harness.url();
    assert_eq!(
        tool_names(&url, "v-tok"),
        ["degenview_set_chart_type", "degenview_set_timeframe"]
    );
    assert_eq!(tool_names(&url, "d-tok"), ["degenview_remove_chart"]);
    for (tool, arguments, other) in [
        (
            "degenview_set_timeframe",
            json!({"workspace_id": "1", "timeframe": "1W"}),
            "d-tok",
        ),
        (
            "degenview_set_chart_type",
            with(json!({"chart_type": "renko"})),
            "d-tok",
        ),
        ("degenview_remove_chart", chart(), "v-tok"),
    ] {
        let result = call(&url, other, tool, arguments);
        assert_eq!(error_code(&result), "PERMISSION_DENIED", "{tool}");
    }
    assert!(
        harness.draw_calls().is_empty(),
        "a denied call reached the app"
    );
}

#[test]
fn the_timeframe_and_chart_type_reach_the_app_as_the_toolbar_names_them() {
    let harness = Harness::start(&[("view", "tok", VIEW)]);
    let url = harness.url();
    let result = call(
        &url,
        "tok",
        "degenview_set_timeframe",
        json!({"workspace_id": "1", "timeframe": "3M"}),
    );
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(result["structuredContent"]["timeframe"], "3M");
    let result = call(
        &url,
        "tok",
        "degenview_set_chart_type",
        with(json!({"chart_type": "heikin_ashi"})),
    );
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(result["structuredContent"]["status"], "changed");
    assert_eq!(
        harness.draw_calls(),
        ["timeframe:3M", "chart_type:heikin_ashi"]
    );
}

#[test]
fn any_count_and_unit_is_a_timeframe_and_the_old_names_still_are() {
    let harness = Harness::start(&[("view", "tok", VIEW)]);
    let url = harness.url();
    for sent in ["4h", "22m", "2D", "1H"] {
        let result = call(
            &url,
            "tok",
            "degenview_set_timeframe",
            json!({"workspace_id": "1", "timeframe": sent}),
        );
        assert_eq!(result["isError"], json!(false), "{sent}: {result}");
    }
    assert_eq!(
        harness.draw_calls(),
        [
            "timeframe:4h",
            "timeframe:22m",
            "timeframe:2D",
            "timeframe:1h"
        ]
    );
}

#[test]
fn a_refused_timeframe_says_why_and_what_is_accepted() {
    let harness = Harness::start(&[("view", "tok", VIEW)]);
    let url = harness.url();
    let result = call(
        &url,
        "tok",
        "degenview_set_timeframe",
        json!({"workspace_id": "1", "timeframe": "60s"}),
    );
    assert_eq!(error_code(&result), "INVALID_ARGUMENT");
    let message = result["structuredContent"]["message"].as_str().unwrap();
    assert!(message.contains("Seconds go from 1 to 59"), "{message}");
    assert!(
        message.contains("s seconds, m minutes, h hours"),
        "{message}"
    );
}

#[test]
fn bad_arguments_never_reach_the_app() {
    let harness = Harness::start(&[("view", "v-tok", VIEW), ("delete", "d-tok", DELETE)]);
    let url = harness.url();
    for (tool, arguments) in [
        (
            "degenview_set_timeframe",
            json!({"workspace_id": "1", "timeframe": "60s"}),
        ),
        (
            "degenview_set_timeframe",
            json!({"workspace_id": "1", "timeframe": "0m"}),
        ),
        (
            "degenview_set_timeframe",
            json!({"workspace_id": "1", "timeframe": "2000m"}),
        ),
        (
            "degenview_set_timeframe",
            json!({"workspace_id": "1", "timeframe": "2Y"}),
        ),
        (
            "degenview_set_timeframe",
            json!({"workspace_id": "one", "timeframe": "1D"}),
        ),
        ("degenview_set_timeframe", json!({"workspace_id": "1"})),
        (
            "degenview_set_chart_type",
            with(json!({"chart_type": "Candles"})),
        ),
        (
            "degenview_set_chart_type",
            with(json!({"chart_type": "macd"})),
        ),
        ("degenview_set_chart_type", with(json!({"chart_type": ""}))),
        (
            "degenview_set_chart_type",
            json!({"workspace_id": "1", "chart_id": "x", "chart_type": "bars"}),
        ),
    ] {
        assert!(
            rejected(&url, "v-tok", tool, arguments.clone()),
            "{tool} {arguments}"
        );
    }
    for arguments in [
        json!({"workspace_id": "1"}),
        json!({"workspace_id": "1", "chart_id": "-1"}),
    ] {
        assert!(
            rejected(&url, "d-tok", "degenview_remove_chart", arguments.clone()),
            "{arguments}"
        );
    }
    assert!(harness.draw_calls().is_empty());
}

#[test]
fn removing_a_chart_reaches_the_app_and_unknown_ones_are_not_found() {
    let harness = Harness::start(&[("delete", "tok", DELETE)]);
    let url = harness.url();
    let result = call(&url, "tok", "degenview_remove_chart", chart());
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(result["structuredContent"]["status"], "removed");
    assert_eq!(result["structuredContent"]["chart"], Value::Null);
    assert_eq!(harness.draw_calls(), ["remove_chart"]);
    let missing = call(
        &url,
        "tok",
        "degenview_remove_chart",
        json!({"workspace_id": "1", "chart_id": "9"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");
}
