//! The activity log over real loopback requests: every way a call can end leaves one row, and
//! no row holds anything a client sent beyond a few ids.

use super::hardening::{add_chart_call, cancel_message, head, limits, start_call, wait_for_calls};
use super::{BOTH, Harness, READ, call, connect_local, error_code, raw_request, rpc};
use crate::model::mcp::{McpAuditEntry, McpAuditKind, McpAuditOutcome};
use serde_json::json;
use std::io::Write;
use std::time::{Duration, Instant};

/// Rows queued so far, oldest first. Waits briefly: the row is written as the call's response
/// is finished, which can be just after the client has the answer.
fn rows(harness: &Harness, wanted: usize) -> Vec<McpAuditEntry> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = Vec::new();
    loop {
        seen.extend(harness.shared.audit().take(0, false));
        if seen.len() >= wanted || Instant::now() > deadline {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_successful_call_leaves_one_row_with_who_what_and_how_long() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    let result = call(
        &url,
        "tok",
        "degenview_list_charts",
        json!({"workspace_id": "1"}),
    );
    assert_eq!(result["isError"], json!(false));
    let rows = rows(&harness, 1);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.kind, McpAuditKind::Call);
    assert_eq!(row.outcome, McpAuditOutcome::Succeeded);
    assert_eq!(row.tool, "degenview_list_charts");
    assert_eq!(
        (row.client_id.as_str(), row.client_name.as_str()),
        ("c1", "client c1")
    );
    assert!(row.code.is_empty());
    assert_eq!(row.request_id, "1");
    assert_eq!(row.targets.len(), 1);
    assert_eq!(
        (row.targets[0].kind.as_str(), row.targets[0].id.as_str()),
        ("workspace_id", "1")
    );
    assert!(row.at_ms > 0 && row.id != 0 && row.count == 1);
    assert!(row.duration_ms < 5_000);
}

#[test]
fn listing_tools_and_initializing_are_not_calls() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    let _ = rpc(&url, Some("tok"), "tools/list", json!({}));
    let _ = rpc(
        &url,
        Some("tok"),
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}),
    );
    assert!(rows(&harness, 1).is_empty());
}

#[test]
fn a_refused_call_is_recorded_as_denied_with_the_permission_code() {
    let harness = Harness::start(&[("c1", "tok", READ)]);
    let url = harness.url();
    // `degenview_add_chart` needs charts.create, which this client lacks.
    let result = call(
        &url,
        "tok",
        "degenview_add_chart",
        json!({"workspace_id": "1", "source": "binance", "symbol": "ETHUSDT"}),
    );
    assert_eq!(error_code(&result), "PERMISSION_DENIED");
    let rows = rows(&harness, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, McpAuditOutcome::Denied);
    assert_eq!(rows[0].code, "PERMISSION_DENIED");
    assert_eq!(rows[0].tool, "degenview_add_chart");
    assert!(harness.added.lock().unwrap().is_empty());
}

#[test]
fn a_call_that_fails_is_recorded_with_its_error_code() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    let result = call(
        &url,
        "tok",
        "degenview_list_charts",
        json!({"workspace_id": "99999"}),
    );
    assert_eq!(error_code(&result), "NOT_FOUND");
    let rows = rows(&harness, 1);
    assert_eq!(rows[0].outcome, McpAuditOutcome::Failed);
    assert_eq!(rows[0].code, "NOT_FOUND");
}

#[test]
fn a_tool_that_does_not_exist_is_recorded_as_denied() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    let reply = rpc(
        &url,
        Some("tok"),
        "tools/call",
        json!({"name": "degenview_no_such_tool\n\"quoted\"", "arguments": {}}),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    let rows = rows(&harness, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, McpAuditOutcome::Denied);
    assert_eq!(rows[0].code, "UNKNOWN_TOOL");
    // The name a client invented is kept only as id-like characters.
    assert_eq!(rows[0].tool, "degenview_no_such_toolquoted");
}

#[test]
fn a_call_the_client_hangs_up_on_is_recorded_as_cancelled() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let port = harness.port;
    harness.pause_app(true);
    {
        let body = add_chart_call(json!(9));
        let mut stream = connect_local(port);
        let request = format!(
            "POST /mcp HTTP/1.1\r\n{}MCP-Protocol-Version: 2025-06-18\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
            head(port, "tok"),
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
    }
    let rows = rows(&harness, 1);
    harness.pause_app(false);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].outcome, McpAuditOutcome::Cancelled);
    assert_eq!(rows[0].request_id, "9");
    assert!(rows[0].code.is_empty());
}

#[test]
fn a_call_cancelled_by_notification_is_recorded_as_cancelled() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    harness.pause_app(true);
    let waiting = start_call(&url, "tok", add_chart_call(json!(4)));
    wait_for_calls(&harness, 1);
    super::post(
        &url,
        Some("tok"),
        &[super::PROTOCOL],
        &cancel_message(json!(4)),
    );
    let _ = waiting.join().unwrap();
    harness.pause_app(false);
    let rows = rows(&harness, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, McpAuditOutcome::Cancelled);
    assert_eq!(rows[0].tool, "degenview_add_chart");
}

#[test]
fn calls_refused_for_rate_limits_are_counted_not_listed() {
    let harness = Harness::start(&[("c1", "tok", READ)]);
    harness.shared.set_rate_limits(limits(1000, 2));
    let port = harness.port;
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
    for _ in 0..8 {
        let _ = raw_request(port, &head(port, "tok"), list);
    }
    // Two got through (and are not calls); six were refused.
    assert!(
        harness.shared.audit().take(0, false).is_empty(),
        "still inside the minute"
    );
    let rows = harness.shared.audit().take(0, true);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].kind, McpAuditKind::RateLimited);
    assert_eq!(rows[0].client_id, "c1");
    assert_eq!(rows[0].count, 6);
    assert_eq!(rows[0].outcome, McpAuditOutcome::Denied);
}

#[test]
fn bad_tokens_leave_no_rows_however_many() {
    let harness = Harness::start(&[("c1", "tok", READ)]);
    let port = harness.port;
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
    for n in 0..30 {
        let _ = raw_request(port, &head(port, &format!("wrong-{n}")), list);
    }
    assert!(harness.shared.audit().take(0, true).is_empty());
}

#[test]
fn no_row_holds_a_token_a_script_or_an_argument_value() {
    let bearer = "dvmcp_this_is_the_bearer_secret_0123456789";
    let harness = Harness::start(&[("c1", bearer, BOTH)]);
    let url = harness.url();
    let pine = "//@version=6\nindicator(\"SECRET_SCRIPT_BODY\")\nplot(close)";
    let note = "a private note: SECRET_NOTE_TEXT";
    let leaked = "dvmcp_leaked_token_in_an_argument_9876543210";
    let calls = [
        (
            "degenview_compile_pine_script",
            json!({"source": pine, "script_id": leaked}),
        ),
        (
            "degenview_create_price_alert",
            json!({"source": "binance", "symbol": "BTCUSDT", "condition": "crosses_above",
                   "target_price": "1", "note": note, "workspace_id": leaked}),
        ),
        (
            "degenview_add_chart",
            json!({"workspace_id": "1", "source": "binance", "symbol": leaked}),
        ),
        (
            "degenview_list_charts",
            json!({"workspace_id": 1, "chart_id": note}),
        ),
    ];
    for (tool, arguments) in &calls {
        let _ = rpc(
            &url,
            Some(bearer),
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        );
    }
    let rows = rows(&harness, calls.len());
    assert_eq!(rows.len(), calls.len(), "{rows:?}");
    let stored = serde_json::to_string(&rows).unwrap();
    for secret in [
        bearer,
        "SECRET_SCRIPT_BODY",
        "SECRET_NOTE_TEXT",
        "a private note",
        "leaked_token",
        "@version",
        "BTCUSDT",
    ] {
        assert!(
            !stored.contains(secret),
            "{secret} reached the log: {stored}"
        );
    }
    // The ids that are kept are cut down to id characters.
    assert!(rows.iter().flat_map(|row| &row.targets).all(|target| {
        target
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && target.id.len() <= 32
    }));
}

#[test]
fn rows_keep_the_order_the_calls_ended_in() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    for workspace in ["1", "2", "3"] {
        let _ = call(
            &url,
            "tok",
            "degenview_list_charts",
            json!({"workspace_id": workspace}),
        );
    }
    let rows = rows(&harness, 3);
    let ids: Vec<_> = rows.iter().map(|row| row.targets[0].id.as_str()).collect();
    assert_eq!(ids, ["1", "2", "3"]);
    assert!(rows.windows(2).all(|pair| pair[0].at_ms <= pair[1].at_ms));
}
