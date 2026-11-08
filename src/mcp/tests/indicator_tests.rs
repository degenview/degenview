//! Indicator values and removal over a real loopback socket. The stub app serves thirty daily
//! bars for workspace 1, chart 2.

use super::{Harness, call, error_code, rpc, tool_names};
use crate::model::mcp::Permission;
use serde_json::{Value, json};

const CANDLES: &[Permission] = &[Permission::ChartsCandlesRead];
const INDICATORS: &[Permission] = &[Permission::ChartsIndicatorsWrite];

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
fn the_indicator_tools_are_listed_only_for_their_permissions() {
    let harness = Harness::start(&[("read", "r-tok", CANDLES), ("write", "w-tok", INDICATORS)]);
    let url = harness.url();
    assert_eq!(
        tool_names(&url, "r-tok"),
        [
            "degenview_get_candles",
            "degenview_get_indicator_values",
            "degenview_get_session_stats"
        ]
    );
    let write = tool_names(&url, "w-tok");
    assert!(
        write.contains(&"degenview_remove_indicator".to_owned()),
        "{write:?}"
    );
    assert!(
        write.contains(&"degenview_add_indicator".to_owned()),
        "{write:?}"
    );
    assert!(!write.contains(&"degenview_get_indicator_values".to_owned()));
}

#[test]
fn values_come_back_aligned_with_the_bars_and_warm() {
    let harness = Harness::start(&[("c", "tok", CANDLES)]);
    let result = call(
        &harness.url(),
        "tok",
        "degenview_get_indicator_values",
        with(json!({"limit": 5, "indicators": ["rsi", "ema"], "ema_period": 9})),
    );
    assert_eq!(result["isError"], json!(false), "{result}");
    let out = &result["structuredContent"];
    assert_eq!(out["times"].as_array().unwrap().len(), 5);
    assert_eq!(out["closes"].as_array().unwrap().len(), 5);
    assert_eq!(out["instrument"], "BINANCE:BTCUSDT");
    let names: Vec<_> = out["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["rsi_14", "ema_9"]);
    // Thirty bars are held, so the newest five are past both warm-ups.
    for series in out["series"].as_array().unwrap() {
        let values = series["values"].as_array().unwrap();
        assert_eq!(values.len(), 5);
        assert!(values.iter().all(|v| v.is_number()), "{series}");
    }
    assert_eq!(out["loaded_bars"], 30);
    assert_eq!(out["has_older_loaded"], true);

    // Paging back from the first of those times.
    let first = out["times"][0].as_str().unwrap().to_owned();
    let page = call(
        &harness.url(),
        "tok",
        "degenview_get_indicator_values",
        with(json!({"limit": 3, "before": first, "indicators": ["bollinger"]})),
    );
    let page = &page["structuredContent"];
    assert_eq!(page["times"].as_array().unwrap().len(), 3);
    let names: Vec<_> = page["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["bb_upper", "bb_middle", "bb_lower"]);
}

#[test]
fn bad_value_requests_and_unknown_charts_are_refused() {
    let harness = Harness::start(&[("c", "tok", CANDLES)]);
    let url = harness.url();
    let tool = "degenview_get_indicator_values";
    for extra in [
        json!({"indicators": []}),
        json!({"indicators": ["volume"]}),
        json!({"ema_period": 1}),
        json!({"ema_period": 401}),
        json!({"limit": 0}),
        json!({"limit": 1001}),
        json!({"before": "yesterday"}),
        json!({"nope": 1}),
    ] {
        assert!(rejected(&url, "tok", tool, with(extra.clone())), "{extra}");
    }
    let result = call(
        &url,
        "tok",
        tool,
        json!({"workspace_id": "1", "chart_id": "99"}),
    );
    assert_eq!(error_code(&result), "NOT_FOUND");
}

#[test]
fn removing_an_indicator_needs_its_own_permission_and_reaches_the_app() {
    let harness = Harness::start(&[("write", "w-tok", INDICATORS), ("read", "r-tok", CANDLES)]);
    let url = harness.url();
    let tool = "degenview_remove_indicator";
    let denied = call(&url, "r-tok", tool, with(json!({"indicator": "rsi"})));
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    let result = call(&url, "w-tok", tool, with(json!({"indicator": "rsi"})));
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(result["structuredContent"]["status"], "removed");
    assert_eq!(result["structuredContent"]["indicator"], "rsi");
    assert!(rejected(
        &url,
        "w-tok",
        tool,
        with(json!({"indicator": "macd"}))
    ));
    assert!(rejected(&url, "w-tok", tool, chart()));
    let missing = call(
        &url,
        "w-tok",
        tool,
        json!({"workspace_id": "1", "chart_id": "9", "indicator": "rsi"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");
}
