//! `degenview_get_session_stats` over a real loopback socket. The stub app serves thirty daily
//! bars for workspace 1, chart 2.

use super::{Harness, call, error_code, rpc, tool_names};
use crate::model::mcp::Permission;
use serde_json::{Value, json};

const CANDLES: &[Permission] = &[Permission::ChartsCandlesRead];
const READ: &[Permission] = &[Permission::ChartsRead];
const TOOL: &str = "degenview_get_session_stats";

fn ask(extra: Value) -> Value {
    let mut args = json!({"workspace_id": "1", "chart_id": "2", "outcome": "day_up"});
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    args
}

fn rejected(url: &str, token: &str, arguments: Value) -> bool {
    let reply = rpc(
        url,
        Some(token),
        "tools/call",
        json!({"name": TOOL, "arguments": arguments}),
    );
    let json = reply.json();
    json.get("error").is_some() || json["result"]["isError"] == json!(true)
}

#[test]
fn the_tool_is_listed_with_the_candles_permission_only() {
    let harness = Harness::start(&[("c", "c-tok", CANDLES), ("r", "r-tok", READ)]);
    let url = harness.url();
    assert!(tool_names(&url, "c-tok").contains(&TOOL.to_owned()));
    assert!(!tool_names(&url, "r-tok").contains(&TOOL.to_owned()));
    let denied = call(&url, "r-tok", TOOL, ask(json!({})));
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
}

#[test]
fn an_answer_names_its_sample_and_carries_the_reminder() {
    let harness = Harness::start(&[("c", "tok", CANDLES)]);
    let result = call(&harness.url(), "tok", TOOL, ask(json!({})));
    assert_eq!(result["isError"], json!(false), "{result}");
    let out = &result["structuredContent"];
    assert_eq!(out["outcome"], "day_up");
    assert_eq!(out["instrument"], "BINANCE:BTCUSDT");
    assert_eq!(out["conditions"], "every day");
    assert_eq!(out["n"], 30);
    assert!(out["estimate"].is_number());
    assert!(out["ci_low"].as_f64().unwrap() <= out["ci_high"].as_f64().unwrap());
    assert!(
        out["disclaimer"]
            .as_str()
            .unwrap()
            .contains("not predictions")
    );
}

#[test]
fn conditions_narrow_the_days() {
    let harness = Harness::start(&[("c", "tok", CANDLES)]);
    let url = harness.url();
    let all = call(&url, "tok", TOOL, ask(json!({})));
    let mondays = call(&url, "tok", TOOL, ask(json!({"weekdays": ["Mon"]})));
    assert_eq!(mondays["isError"], json!(false), "{mondays}");
    let (all, mondays) = (&all["structuredContent"], &mondays["structuredContent"]);
    assert!(mondays["n"].as_u64().unwrap() < all["n"].as_u64().unwrap());
    assert_eq!(mondays["conditions"], "Mon");
}

#[test]
fn bad_arguments_and_unanswerable_questions_are_refused() {
    let harness = Harness::start(&[("c", "tok", CANDLES)]);
    let url = harness.url();
    for extra in [
        json!({"outcome": "moon"}),
        json!({"weekdays": []}),
        json!({"weekdays": ["Funday"]}),
        json!({"gap": "sideways"}),
        json!({"gap_min_pct": -1}),
        json!({"gap_max_pct": 5000}),
        json!({"gap_min_pct": 2, "gap_max_pct": 1}),
        json!({"nope": 1}),
    ] {
        assert!(rejected(&url, "tok", ask(extra.clone())), "{extra}");
    }
    // A first-bar question needs hourly bars and the stub's chart is daily.
    let result = call(
        &url,
        "tok",
        TOOL,
        ask(json!({"outcome": "first_bar_high_break"})),
    );
    assert_eq!(error_code(&result), "UNSUPPORTED");
    let unknown = call(
        &url,
        "tok",
        TOOL,
        json!({"workspace_id": "1", "chart_id": "99", "outcome": "day_up"}),
    );
    assert_eq!(error_code(&unknown), "NOT_FOUND");
}
