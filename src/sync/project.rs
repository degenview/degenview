//! Turns a stored row into the entities that actually sync. Projection is where per-device and
//! runtime fields are dropped, so two Macs with the same user-visible data produce the same
//! bytes and hash. That equality is what lets the outbox ignore the alert engine's once-a-second
//! heartbeat and ignore its own remote applies.
//!
//! Projections work on JSON values, not typed models: the store hook sees raw payload strings
//! and must not depend on every model's Rust type.

use super::policy::{EntityKind, SettingPolicy, TablePolicy, setting_policy, table_policy};
use super::record::{FAVORITES_ID, sha256_hex};
use serde_json::{Map, Value};

/// One synced entity derived from a stored row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projected {
    pub kind: EntityKind,
    pub sync_id: String,
    /// The id the local store knows it by; differs from `sync_id` only for the Favorites list.
    pub local_id: String,
    pub payload: Vec<u8>,
    pub hash: String,
}

impl Projected {
    fn new(kind: EntityKind, sync_id: String, local_id: String, value: &Value) -> Self {
        let payload = canonical_json(value);
        let hash = sha256_hex(&payload);
        Self {
            kind,
            sync_id,
            local_id,
            payload,
            hash,
        }
    }
}

/// Fields that describe this Mac's use of a layout, not the layout.
const LAYOUT_LOCAL_FIELDS: [&str; 2] = ["last_opened_at_ms", "autosave"];

/// The alert definition. Everything else on a stored alert is runtime state the engine rewrites
/// on every quote or heartbeat, and must never sync.
const ALERT_DEFINITION_FIELDS: [&str; 9] = [
    "id",
    "asset",
    "condition",
    "currency",
    "frequency",
    "note",
    "created_at_ms",
    "webhook_endpoint_ids",
    "webhook_message",
];

/// The entities one row of a document table syncs as. Empty for local tables and for rows that
/// are not valid JSON objects.
pub fn project_row(table: &str, id: &str, payload: &str) -> Vec<Projected> {
    let TablePolicy::Synced(kind) = table_policy(table) else {
        return Vec::new();
    };
    let Ok(Value::Object(row)) = serde_json::from_str::<Value>(payload) else {
        return Vec::new();
    };
    match kind {
        EntityKind::Layout => project_layout(id, row),
        EntityKind::Watchlist => {
            let favorites = row.get("is_favorites").and_then(Value::as_bool) == Some(true);
            let mut row = row;
            let sync_id = if favorites {
                // Every Mac has its own Favorites row id. Leaving it in the payload would make
                // each Mac see the other's version as different and re-upload it forever.
                row.insert("id".into(), 0.into());
                FAVORITES_ID
            } else {
                id
            };
            vec![Projected::new(
                kind,
                sync_id.to_string(),
                id.to_string(),
                &Value::Object(row),
            )]
        }
        EntityKind::Alert => vec![Projected::new(
            kind,
            id.to_string(),
            id.to_string(),
            &project_alert(&row),
        )],
        _ => vec![Projected::new(
            kind,
            id.to_string(),
            id.to_string(),
            &Value::Object(row),
        )],
    }
}

/// The entity a `setting` value syncs as, if the key is portable.
pub fn project_setting(key: &str, value: &str) -> Option<Projected> {
    if !matches!(setting_policy(key), SettingPolicy::Portable(_)) {
        return None;
    }
    let mut value = serde_json::from_str::<Value>(value).ok()?;
    if key == "snapshot.prefs" {
        // Where to upload and how, but never which hosts this Mac consented to send images to.
        value = keep_fields(&value, &["host", "include_trading", "scale"]);
    }
    Some(Projected::new(
        EntityKind::Setting,
        key.to_string(),
        key.to_string(),
        &value,
    ))
}

fn keep_fields(value: &Value, fields: &[&str]) -> Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    Value::Object(
        fields
            .iter()
            .filter_map(|field| object.get(*field).map(|v| (field.to_string(), v.clone())))
            .collect(),
    )
}

fn project_alert(row: &Map<String, Value>) -> Value {
    let mut definition: Map<String, Value> = ALERT_DEFINITION_FIELDS
        .iter()
        .filter_map(|field| row.get(*field).map(|v| (field.to_string(), v.clone())))
        .collect();
    // A paused alert stays paused on other Macs; every other state is runtime progress.
    let paused = row.get("state").and_then(Value::as_str) == Some("Paused");
    definition.insert(
        "intent".into(),
        Value::String(if paused { "paused" } else { "active" }.into()),
    );
    Value::Object(definition)
}

/// A layout without its drawings, plus one entity per drawing, keyed by layout, chart and
/// drawing id so two Macs editing different drawings both survive.
fn project_layout(id: &str, mut row: Map<String, Value>) -> Vec<Projected> {
    for field in LAYOUT_LOCAL_FIELDS {
        row.remove(field);
    }
    let mut drawings = Vec::new();
    if let Some(Value::Array(charts)) = row.get_mut("charts") {
        for chart in charts {
            let Some(chart) = chart.as_object_mut() else {
                continue;
            };
            let chart_id = chart.get("id").and_then(Value::as_u64);
            let taken = chart.remove("drawings");
            let (Some(chart_id), Some(Value::Array(list))) = (chart_id, taken) else {
                continue;
            };
            let mut seen = std::collections::HashSet::new();
            for (order, drawing) in list.into_iter().enumerate() {
                let Some(drawing_id) = drawing_id(&drawing) else {
                    continue;
                };
                // Ids are unique per chart; a duplicate would collide on one record name.
                if !seen.insert(drawing_id) {
                    continue;
                }
                drawings.push((chart_id, drawing_id, order, drawing));
            }
        }
    }
    let mut projected = vec![Projected::new(
        EntityKind::Layout,
        id.to_string(),
        id.to_string(),
        &Value::Object(row),
    )];
    for (chart_id, drawing_id, order, drawing) in drawings {
        let sync_id = format!("{id}.{chart_id}.{drawing_id}");
        let body = serde_json::json!({"ord": order, "drawing": drawing});
        projected.push(Projected::new(
            EntityKind::LayoutDrawing,
            sync_id.clone(),
            sync_id,
            &body,
        ));
    }
    projected
}

/// A stored drawing is `{"<Variant>": {"id": n, ...}}`; an unreadable one keeps the same shape.
fn drawing_id(drawing: &Value) -> Option<u64> {
    drawing.as_object()?.values().next()?.get("id")?.as_u64()
}

/// A user script as the scanner sees it. Scripts live in files, not SQLite, so the caller builds
/// this from the script library. Drafts and revisions never sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSnapshot {
    pub id: String,
    pub name: String,
    pub source: String,
    pub script_type: String,
    pub is_favorite: bool,
    pub created_ms: i64,
    pub modified_ms: i64,
}

pub fn project_script(script: &ScriptSnapshot) -> Projected {
    let body = serde_json::json!({
        "id": script.id,
        "name": script.name,
        "source": script.source,
        "script_type": script.script_type,
        "is_favorite": script.is_favorite,
        "created_ms": script.created_ms,
        "modified_ms": script.modified_ms,
        "sha256": sha256_hex(script.source.as_bytes()),
    });
    Projected::new(
        EntityKind::Script,
        script.id.clone(),
        script.id.clone(),
        &body,
    )
}

/// JSON with object keys sorted at every level, so equal data always hashes equal regardless of
/// serde_json's map ordering features.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out.into_bytes()
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort_unstable();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(drawing_body["ord"], 0);
    }

    #[test]
    fn per_device_layout_fields_do_not_change_the_layout() {
        let a = project_row("saved_view", "10", &layout(1, false, vec![]));
        let b = project_row("saved_view", "10", &layout(999, true, vec![]));
        assert_eq!(a[0].hash, b[0].hash);
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
            script_type: "indicator".into(),
            is_favorite: false,
            created_ms: 1,
            modified_ms: 2,
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
            "host": "snaps.example", "include_trading": true, "scale": "X2",
            "consented": ["evil.example"]
        })
        .to_string();
        let projected = project_setting("snapshot.prefs", &value).unwrap();
        let body: Value = serde_json::from_slice(&projected.payload).unwrap();
        assert!(body.get("consented").is_none());
        assert_eq!(body["host"], "snaps.example");
        let other = json!({
            "host": "snaps.example", "include_trading": true, "scale": "X2", "consented": []
        })
        .to_string();
        assert_eq!(
            project_setting("snapshot.prefs", &other).unwrap().hash,
            projected.hash
        );
    }
}
