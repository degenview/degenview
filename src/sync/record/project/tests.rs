use super::*;
use serde_json::json;

fn row(value: Value) -> String {
    value.to_string()
}

fn alert(state: &str, armed: bool, previous: f64, updated: i64) -> Value {
    json!({
        "id": 5, "asset": {"symbol": "BTC"}, "condition": {"Above": 100.0},
        "currency": "USD", "frequency": "Once", "state": state, "note": "n",
        "armed": armed, "previous_value": previous, "last_fingerprint": "f",
        "created_at_ms": 1, "updated_at_ms": updated,
        "webhook_endpoint_ids": [1], "webhook_message": "m"
    })
}

#[test]
fn canonical_json_ignores_key_order() {
    let a = json!({"b": 1, "a": {"y": [1, {"d": 1, "c": 2}], "x": null}});
    let b = json!({"a": {"x": null, "y": [1, {"c": 2, "d": 1}]}, "b": 1});
    assert_eq!(canonical_json(&a), canonical_json(&b));
    assert_ne!(canonical_json(&a), canonical_json(&json!({"b": 2})));
}

#[test]
fn local_tables_and_garbage_project_to_nothing() {
    assert!(project_row("tab", "1", "{}").is_empty());
    assert!(project_row("mcp_client", "x", "{}").is_empty());
    assert!(project_row("mcp_audit", "x", "{}").is_empty());
    assert!(project_row("watchlist", "1", "not json").is_empty());
    assert!(project_row("watchlist", "1", "[1,2]").is_empty());
    assert!(project_row("some_future_table", "1", "{}").is_empty());
}

#[test]
fn alert_runtime_churn_does_not_change_the_projection() {
    let quiet = project_row("price_alert", "5", &row(alert("Active", true, 90.0, 10)));
    let churned = project_row(
        "price_alert",
        "5",
        &row(alert("Active", false, 99.5, 99999)),
    );
    assert_eq!(quiet, churned);
    assert_eq!(quiet.len(), 1);
}

#[test]
fn a_triggered_alert_still_projects_as_active() {
    let active = project_row("price_alert", "5", &row(alert("Active", true, 1.0, 1)));
    let triggered = project_row("price_alert", "5", &row(alert("Triggered", true, 1.0, 1)));
    assert_eq!(active[0].hash, triggered[0].hash);
}

#[test]
fn pausing_an_alert_changes_the_projection() {
    let active = project_row("price_alert", "5", &row(alert("Active", true, 1.0, 1)));
    let paused = project_row("price_alert", "5", &row(alert("Paused", true, 1.0, 1)));
    assert_ne!(active[0].hash, paused[0].hash);
    let body: Value = serde_json::from_slice(&paused[0].payload).unwrap();
    assert_eq!(body["intent"], "paused");
}

#[test]
fn editing_an_alert_definition_changes_the_projection() {
    let mut changed = alert("Active", true, 1.0, 1);
    changed["condition"] = json!({"Above": 200.0});
    let before = project_row("price_alert", "5", &row(alert("Active", true, 1.0, 1)));
    let after = project_row("price_alert", "5", &row(changed));
    assert_ne!(before[0].hash, after[0].hash);
}

#[test]
fn runtime_fields_never_appear_in_the_alert_payload() {
    let projected = project_row("price_alert", "5", &row(alert("Active", true, 1.0, 1)));
    let body: Value = serde_json::from_slice(&projected[0].payload).unwrap();
    for field in [
        "armed",
        "previous_value",
        "last_fingerprint",
        "updated_at_ms",
        "state",
    ] {
        assert!(body.get(field).is_none(), "{field}");
    }
}

#[test]
fn the_favorites_list_has_a_singleton_sync_id() {
    let favorites = project_row(
        "watchlist",
        "1739",
        &row(json!({"id": 1739, "name": "Favorites", "is_favorites": true})),
    );
    assert_eq!(favorites[0].sync_id, "favorites");
    assert_eq!(favorites[0].local_id, "1739");
    // Two Macs' Favorites with different row ids project to identical bytes.
    let other_mac = project_row(
        "watchlist",
        "5555",
        &row(json!({"id": 5555, "name": "Favorites", "is_favorites": true})),
    );
    assert_eq!(favorites[0].hash, other_mac[0].hash);
    let other = project_row(
        "watchlist",
        "8",
        &row(json!({"id": 8, "name": "Tech", "is_favorites": false})),
    );
    assert_eq!(other[0].sync_id, "8");
}

fn layout(opened: i64, autosave: bool, drawings: Vec<Value>) -> String {
    row(json!({
        "id": 10, "name": "Main", "last_opened_at_ms": opened, "autosave": autosave,
        "charts": [{"id": 2, "symbol": "BTC", "drawings": drawings}]
    }))
}

fn trend(id: u64, price: f64) -> Value {
    json!({"TrendLine": {"id": id, "a": {"time_ms": 1, "price": price}}})
}

#[test]
fn a_layout_splits_into_a_layout_and_one_entity_per_drawing() {
    let parts = project_row(
        "saved_view",
        "10",
        &layout(1, false, vec![trend(1, 1.0), trend(2, 2.0)]),
    );
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].kind, EntityKind::Layout);
    let ids: Vec<_> = parts[1..].iter().map(|p| p.sync_id.as_str()).collect();
    assert_eq!(ids, ["10.2.1", "10.2.2"]);
    let layout_body: Value = serde_json::from_slice(&parts[0].payload).unwrap();
    assert!(layout_body["charts"][0].get("drawings").is_none());
    let drawing_body: Value = serde_json::from_slice(&parts[1].payload).unwrap();
    assert!(drawing_body.get("ord").is_none());
    assert!(drawing_body["drawing"]["TrendLine"].is_object());
}

#[test]
fn per_device_layout_fields_do_not_change_the_layout() {
    let a = project_row("saved_view", "10", &layout(1, false, vec![]));
    let b = project_row("saved_view", "10", &layout(999, true, vec![]));
    assert_eq!(a[0].hash, b[0].hash);
}

#[test]
fn inserting_or_removing_a_drawing_does_not_disturb_the_others() {
    let three = project_row(
        "saved_view",
        "10",
        &layout(1, false, vec![trend(1, 1.0), trend(2, 2.0), trend(3, 3.0)]),
    );
    let without_first = project_row(
        "saved_view",
        "10",
        &layout(1, false, vec![trend(2, 2.0), trend(3, 3.0)]),
    );
    let by_id = |parts: &[Projected], id: &str| {
        parts.iter().find(|p| p.sync_id == id).unwrap().hash.clone()
    };
    assert_eq!(by_id(&three, "10.2.2"), by_id(&without_first, "10.2.2"));
    assert_eq!(by_id(&three, "10.2.3"), by_id(&without_first, "10.2.3"));
}

#[test]
fn moving_one_drawing_changes_only_that_drawing() {
    let before = project_row(
        "saved_view",
        "10",
        &layout(1, false, vec![trend(1, 1.0), trend(2, 2.0)]),
    );
    let after = project_row(
        "saved_view",
        "10",
        &layout(1, false, vec![trend(1, 1.0), trend(2, 9.0)]),
    );
    assert_eq!(before[0].hash, after[0].hash);
    assert_eq!(before[1].hash, after[1].hash);
    assert_ne!(before[2].hash, after[2].hash);
}

#[test]
fn unreadable_drawings_keep_their_id_and_duplicates_collapse() {
    let unreadable = json!({"FutureTool": {"id": 7, "blob": [1, 2]}});
    let parts = project_row(
        "saved_view",
        "10",
        &layout(
            1,
            false,
            vec![unreadable, trend(7, 1.0), json!({"Weird": {}}), json!(5)],
        ),
    );
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[1].sync_id, "10.2.7");
}

#[test]
fn a_layout_with_a_chart_that_has_no_drawings_field_projects() {
    let parts = project_row(
        "saved_view",
        "10",
        &row(json!({"id": 10, "name": "x", "charts": [{"id": 1}, "junk"]})),
    );
    assert_eq!(parts.len(), 1);
}

#[test]
fn a_script_projects_with_a_content_hash() {
    let script = ScriptSnapshot {
        id: "0123456789abcdef0123456789abcdef".into(),
        name: "My Strategy".into(),
        source: "//@version=6\n".into(),
        is_favorite: false,
    };
    let a = project_script(&script);
    assert_eq!(a.kind, EntityKind::Script);
    assert_eq!(a.sync_id, script.id);
    let body: Value = serde_json::from_slice(&a.payload).unwrap();
    assert_eq!(body["sha256"], sha256_hex(b"//@version=6\n"));
    let renamed = ScriptSnapshot {
        name: "Renamed".into(),
        ..script.clone()
    };
    assert_ne!(a.hash, project_script(&renamed).hash);
    assert_eq!(a.hash, project_script(&script).hash);
}

#[test]
fn only_portable_settings_project() {
    assert!(project_setting("alert.health", "{}").is_none());
    assert!(project_setting("mcp.settings", "{}").is_none());
    assert!(project_setting("unknownKey", "1").is_none());
    let theme = project_setting("appTheme", "\"dark\"").unwrap();
    assert_eq!(theme.kind, EntityKind::Setting);
    assert_eq!(theme.sync_id, "appTheme");
    assert!(project_setting("appTheme", "not json").is_none());
}

#[test]
fn snapshot_prefs_never_carry_consented_hosts() {
    let value = json!({
        "provider": "self_hosted",
        "providers": {"self_hosted": {"host": "https://snaps.example/up"}},
        "include_trading": true, "scale": "X2",
        "consented": ["evil.example"]
    })
    .to_string();
    let projected = project_setting("snapshot.prefs", &value).unwrap();
    let body: Value = serde_json::from_slice(&projected.payload).unwrap();
    assert!(body.get("consented").is_none());
    assert_eq!(body["provider"], "self_hosted");
    assert_eq!(
        body["providers"]["self_hosted"]["host"],
        "https://snaps.example/up"
    );
    let other = json!({
        "provider": "self_hosted",
        "providers": {"self_hosted": {"host": "https://snaps.example/up"}},
        "include_trading": true, "scale": "X2", "consented": []
    })
    .to_string();
    assert_eq!(
        project_setting("snapshot.prefs", &other).unwrap().hash,
        projected.hash
    );
}

#[test]
fn snapshot_prefs_carry_a_provider_that_is_off_and_never_a_stray_host() {
    // Off is an explicit `null`, and syncs as one.
    let off = json!({"provider": null, "providers": {}, "scale": "X2", "include_trading": false})
        .to_string();
    let body: Value =
        serde_json::from_slice(&project_setting("snapshot.prefs", &off).unwrap().payload).unwrap();
    assert!(body["provider"].is_null());
    assert!(body.as_object().unwrap().contains_key("provider"));
    // A leftover `host` beside a provider that was chosen (or turned off) is not sent on.
    let stray = json!({"host": "https://old.example/up", "provider": null}).to_string();
    let body: Value =
        serde_json::from_slice(&project_setting("snapshot.prefs", &stray).unwrap().payload)
            .unwrap();
    assert!(body.get("host").is_none());
}

#[test]
fn a_snapshot_host_nobody_has_rewritten_since_the_update_still_goes_out_as_it_is_stored() {
    // Nothing has changed the setting since the update, so it is still a top-level `host`.
    // Another Mac must still learn where to upload, and a Mac that only received it must not
    // read it back as a new edit (the echo would be the same record, byte for byte).
    let legacy = json!({
        "host": "https://snaps.example/up", "include_trading": false, "scale": "X1",
        "consented": ["snaps.example"]
    })
    .to_string();
    let sent = project_setting("snapshot.prefs", &legacy).unwrap();
    let body: Value = serde_json::from_slice(&sent.payload).unwrap();
    assert_eq!(body["host"], "https://snaps.example/up", "{body}");
    assert!(body.get("provider").is_none() && body.get("consented").is_none());
    // What an older Mac sent is what this Mac stores, and projects to the same record.
    let received = json!({
        "host": "https://snaps.example/up", "include_trading": false, "scale": "X1",
        "consented": []
    })
    .to_string();
    assert_eq!(
        project_setting("snapshot.prefs", &received).unwrap().hash,
        sent.hash
    );
}
