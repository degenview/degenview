//! The replay tools over a real loopback socket. The stub app drives the real replay clock over
//! the stub chart's thirty daily bars (2025-10-20 onwards).

use super::{Harness, call, error_code, rpc, tool_names};
use crate::model::mcp::Permission;
use serde_json::{Value, json};

const READ: &[Permission] = &[Permission::ChartsRead];
const WRITE: &[Permission] = &[Permission::ChartsReplayWrite];

fn control(url: &str, token: &str, arguments: Value) -> Value {
    let mut args = json!({"workspace_id": "1"});
    args.as_object_mut()
        .unwrap()
        .extend(arguments.as_object().unwrap().clone());
    call(url, token, "degenview_control_replay", args)
}

fn rejected(url: &str, token: &str, arguments: Value) -> bool {
    let mut args = json!({"workspace_id": "1"});
    args.as_object_mut()
        .unwrap()
        .extend(arguments.as_object().unwrap().clone());
    let reply = rpc(
        url,
        Some(token),
        "tools/call",
        json!({"name": "degenview_control_replay", "arguments": args}),
    );
    let json = reply.json();
    json.get("error").is_some() || json["result"]["isError"] == json!(true)
}

#[test]
fn reading_needs_charts_read_and_driving_needs_its_own_permission() {
    let harness = Harness::start(&[("read", "r-tok", READ), ("write", "w-tok", WRITE)]);
    let url = harness.url();
    assert!(tool_names(&url, "r-tok").contains(&"degenview_get_replay".to_owned()));
    assert!(!tool_names(&url, "r-tok").contains(&"degenview_control_replay".to_owned()));
    assert_eq!(tool_names(&url, "w-tok"), ["degenview_control_replay"]);
    let denied = control(&url, "r-tok", json!({"action": "start"}));
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    let denied = call(
        &url,
        "w-tok",
        "degenview_get_replay",
        json!({"workspace_id": "1"}),
    );
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
}

#[test]
fn a_replay_is_started_stepped_sought_and_stopped() {
    let harness = Harness::start(&[("read", "r-tok", READ), ("write", "w-tok", WRITE)]);
    let url = harness.url();
    let idle = call(
        &url,
        "r-tok",
        "degenview_get_replay",
        json!({"workspace_id": "1"}),
    );
    assert_eq!(idle["structuredContent"]["status"], "inactive");
    assert_eq!(idle["structuredContent"]["active"], false);

    let started = control(
        &url,
        "w-tok",
        json!({"action": "start", "time": "2025-10-25T12:00:00Z", "chart_id": "2"}),
    );
    assert_eq!(started["isError"], json!(false), "{started}");
    let state = &started["structuredContent"];
    assert_eq!(state["status"], "paused");
    assert_eq!(state["active"], true);
    assert_eq!(
        state["current_time"], "2025-10-25T00:00:00Z",
        "the bar at or before the time"
    );
    assert_eq!(state["bar_index"], 5);
    assert_eq!(state["bars_total"], 30);
    assert_eq!(state["bars_remaining"], 24);
    assert_eq!(state["symbol"], "BTCUSDT");

    let stepped = control(&url, "w-tok", json!({"action": "step_forward", "steps": 3}));
    assert_eq!(stepped["structuredContent"]["bar_index"], 8);
    let back = control(&url, "w-tok", json!({"action": "step_back"}));
    assert_eq!(back["structuredContent"]["bar_index"], 7);
    let sought = control(
        &url,
        "w-tok",
        json!({"action": "seek", "time": "2025-11-10T00:00:00Z"}),
    );
    assert_eq!(
        sought["structuredContent"]["current_time"],
        "2025-11-10T00:00:00Z"
    );
    let end = control(
        &url,
        "w-tok",
        json!({"action": "step_forward", "steps": 500}),
    );
    assert_eq!(end["structuredContent"]["status"], "completed");
    assert_eq!(end["structuredContent"]["bars_remaining"], 0);
    let restarted = control(&url, "w-tok", json!({"action": "restart"}));
    assert_eq!(restarted["structuredContent"]["bar_index"], 5);
    let fast = control(
        &url,
        "w-tok",
        json!({"action": "set_speed", "speed": "10x"}),
    );
    assert_eq!(fast["structuredContent"]["speed"], "10x");
    let playing = control(&url, "w-tok", json!({"action": "play"}));
    assert_eq!(playing["structuredContent"]["status"], "playing");
    let paused = control(&url, "w-tok", json!({"action": "pause"}));
    assert_eq!(paused["structuredContent"]["status"], "paused");

    let live = control(&url, "w-tok", json!({"action": "stop"}));
    assert_eq!(live["structuredContent"]["status"], "inactive");
    assert_eq!(live["structuredContent"]["current_time"], Value::Null);
    let read = call(
        &url,
        "r-tok",
        "degenview_get_replay",
        json!({"workspace_id": "1"}),
    );
    assert_eq!(read["structuredContent"]["active"], false);
}

#[test]
fn acting_on_a_replay_that_is_not_running_or_a_chart_that_is_not_there_is_refused() {
    let harness = Harness::start(&[("write", "tok", WRITE)]);
    let url = harness.url();
    for action in [
        "play",
        "pause",
        "step_forward",
        "step_back",
        "restart",
        "stop",
    ] {
        let result = control(&url, "tok", json!({"action": action}));
        assert_eq!(error_code(&result), "CONFLICT", "{action}");
    }
    let missing = control(&url, "tok", json!({"action": "start", "chart_id": "9"}));
    assert_eq!(error_code(&missing), "NOT_FOUND");
    let missing = call(
        &url,
        "tok",
        "degenview_control_replay",
        json!({"workspace_id": "7", "action": "start"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");
}

#[test]
fn an_argument_that_does_not_belong_to_the_action_is_refused() {
    let harness = Harness::start(&[("write", "tok", WRITE)]);
    let url = harness.url();
    for arguments in [
        json!({"action": "play", "steps": 2}),
        json!({"action": "pause", "time": "2025-10-25T00:00:00Z"}),
        json!({"action": "step_forward", "chart_id": "2"}),
        json!({"action": "step_forward", "steps": 0}),
        json!({"action": "step_forward", "steps": 501}),
        json!({"action": "seek"}),
        json!({"action": "seek", "time": "yesterday"}),
        json!({"action": "set_speed"}),
        json!({"action": "set_speed", "speed": "3x"}),
        json!({"action": "start", "speed": "2x"}),
        json!({"action": "rewind"}),
        json!({"action": "start", "chart_id": "x"}),
    ] {
        assert!(rejected(&url, "tok", arguments.clone()), "{arguments}");
    }
}
