//! The chart picture tool over a real loopback socket. The stub app makes the job pending once
//! and then hands out a small fixed PNG.

use super::{Harness, STUB_PNG, call, error_code, rpc, tool_names};
use crate::mcp::snapshot::base64;
use crate::model::mcp::Permission;
use serde_json::{Value, json};

const SNAPSHOT: &[Permission] = &[Permission::ChartsSnapshotRead];

fn chart() -> Value {
    json!({"workspace_id": "1", "chart_id": "2"})
}

#[test]
fn a_picture_comes_back_as_an_image_block_with_its_size() {
    let harness = Harness::start(&[("s", "tok", SNAPSHOT)]);
    let url = harness.url();
    assert_eq!(tool_names(&url, "tok"), ["degenview_get_chart_snapshot"]);

    let result = call(&url, "tok", "degenview_get_chart_snapshot", chart());
    assert_eq!(result["isError"], json!(false), "{result}");
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "{result}");
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["mimeType"], "image/png");
    assert_eq!(content[0]["data"], base64(STUB_PNG));
    assert_eq!(content[1]["type"], "text");
    let meta: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(meta["width"], 640);
    assert_eq!(meta["height"], 400);
    assert_eq!(meta["scale"], "1x");
    assert_eq!(meta["bytes"], STUB_PNG.len());
    assert_eq!(harness.draw_calls(), ["snapshot:X1"]);

    let sharp = call(
        &url,
        "tok",
        "degenview_get_chart_snapshot",
        json!({"workspace_id": "1", "chart_id": "2", "scale": "2x"}),
    );
    assert_eq!(sharp["isError"], json!(false), "{sharp}");
    assert_eq!(harness.draw_calls(), ["snapshot:X1", "snapshot:X2"]);
}

#[test]
fn a_picture_needs_its_own_permission_and_a_known_chart() {
    let harness = Harness::start(&[
        ("s", "tok", SNAPSHOT),
        (
            "c",
            "c-tok",
            &[Permission::ChartsRead, Permission::ChartsCandlesRead],
        ),
    ]);
    let url = harness.url();
    let denied = call(&url, "c-tok", "degenview_get_chart_snapshot", chart());
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    let missing = call(
        &url,
        "tok",
        "degenview_get_chart_snapshot",
        json!({"workspace_id": "1", "chart_id": "99"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");
    assert!(
        harness.draw_calls().is_empty(),
        "no refused call started a picture"
    );
}

#[test]
fn bad_picture_arguments_never_reach_the_app() {
    let harness = Harness::start(&[("s", "tok", SNAPSHOT)]);
    let url = harness.url();
    for arguments in [
        json!({"workspace_id": "1"}),
        json!({"workspace_id": "x", "chart_id": "2"}),
        json!({"workspace_id": "1", "chart_id": "-1"}),
        json!({"workspace_id": "1", "chart_id": "2", "scale": "3x"}),
        json!({"workspace_id": "1", "chart_id": "2", "scale": 2}),
        json!({"workspace_id": "1", "chart_id": "2", "format": "jpeg"}),
    ] {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": "degenview_get_chart_snapshot", "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "{arguments}");
    }
    assert!(harness.draw_calls().is_empty());
}
