//! Turns a stored row into the entities that actually sync. Projection is where per-device and
//! runtime fields are dropped, so two Macs with the same user-visible data produce the same
//! bytes and hash. That equality is what lets the outbox ignore the alert engine's once-a-second
//! heartbeat and ignore its own remote applies.
//!
//! Projections work on JSON values, not typed models: the store hook sees raw payload strings
//! and must not depend on every model's Rust type.

use super::ids::{Direction, IdSpace, map_card, map_fields};
use super::policy::{EntityKind, TablePolicy, setting_is_portable, table_policy};
use super::{FAVORITES_ID, sha256_hex};
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

/// The database tag the tests project with.
#[cfg(test)]
pub const TEST_TAG: u32 = 0x0abc_def1;

/// `project_row_in` for a database whose id tag does not matter to the test.
#[cfg(test)]
pub fn project_row(table: &str, id: &str, payload: &str) -> Vec<Projected> {
    project_row_in(IdSpace::new(TEST_TAG), table, id, payload)
}

/// The entities one row of a document table syncs as. Empty for local tables and for rows that
/// are not valid JSON objects. `ids` turns this database's counter ids into ones that mean the
/// same on every Mac.
pub fn project_row_in(ids: IdSpace, table: &str, id: &str, payload: &str) -> Vec<Projected> {
    let TablePolicy::Synced(kind) = table_policy(table) else {
        return Vec::new();
    };
    let Ok(Value::Object(row)) = serde_json::from_str::<Value>(payload) else {
        return Vec::new();
    };
    match kind {
        EntityKind::Layout => project_layout(ids, id, row),
        EntityKind::Watchlist => {
            let favorites = row.get("is_favorites").and_then(Value::as_bool) == Some(true);
            let mut row = row;
            map_entry_cards(&mut row, ids, Direction::ToGlobal);
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
        EntityKind::Portfolio => {
            let mut value = Value::Object(row);
            map_fields(&mut value, &["id"], ids, Direction::ToGlobal);
            vec![Projected::new(
                kind,
                ids.to_global_string(id),
                id.to_string(),
                &value,
            )]
        }
        _ => vec![Projected::new(
            kind,
            id.to_string(),
            id.to_string(),
            &Value::Object(row),
        )],
    }
}

/// A bundle of several tables' rows as one entity (see `sync::record::paper`).
pub(crate) fn project_bundle(sync_id: String, local_id: String, bundle: &Value) -> Projected {
    Projected::new(EntityKind::PaperAccount, sync_id, local_id, bundle)
}

/// A portfolio transaction, whose table is not a document table, as the entity it syncs as.
pub fn project_transaction(ids: IdSpace, id: &str, payload: &str) -> Option<Projected> {
    let mut value = serde_json::from_str::<Value>(payload).ok()?;
    if !value.is_object() {
        return None;
    }
    map_fields(
        &mut value,
        &["id", "portfolio_id"],
        ids,
        Direction::ToGlobal,
    );
    Some(Projected::new(
        EntityKind::PortfolioTransaction,
        ids.to_global_string(id),
        id.to_string(),
        &value,
    ))
}

/// The entity a `setting` value syncs as, if the key is portable.
pub fn project_setting(key: &str, value: &str) -> Option<Projected> {
    if !setting_is_portable(key) {
        return None;
    }
    let mut value = serde_json::from_str::<Value>(value).ok()?;
    if key == "snapshot.prefs" {
        // Which provider to upload to and how it is set up (never a secret: those stay in each
        // Mac's secure storage), but never which destinations this Mac consented to send images to.
        // A value that has not been written since this version came (no `provider` yet) is still
        // in the shape an older version wrote, a top-level `host`, and goes out as it is stored:
        // reading it as this version does would make a Mac that merely received it send it back
        // as a new edit, over what another Mac chose since. A value that names its provider (or
        // none, as an explicit `null`) is the new shape, and a leftover `host` in it is not sent.
        let fields: &[&str] = if value.get("provider").is_some() {
            &["provider", "providers", "include_trading", "scale"]
        } else {
            &["host", "include_trading", "scale"]
        };
        value = keep_fields(&value, fields);
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
fn project_layout(ids: IdSpace, id: &str, mut row: Map<String, Value>) -> Vec<Projected> {
    for field in LAYOUT_LOCAL_FIELDS {
        row.remove(field);
    }
    map_chart_cards(&mut row, ids, Direction::ToGlobal);
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
            for drawing in list {
                let Some(drawing_id) = drawing_id(&drawing) else {
                    continue;
                };
                // Ids are unique per chart; a duplicate would collide on one record name.
                if !seen.insert(drawing_id) {
                    continue;
                }
                drawings.push((chart_id, drawing_id, drawing));
            }
        }
    }
    let mut projected = vec![Projected::new(
        EntityKind::Layout,
        id.to_string(),
        id.to_string(),
        &Value::Object(row),
    )];
    for (chart_id, drawing_id, drawing) in drawings {
        let sync_id = format!("{id}.{chart_id}.{drawing_id}");
        // Stacking order is deliberately not part of a drawing: one Mac deleting or inserting a
        // drawing would otherwise shift every later drawing and look like an edit of each.
        let body = serde_json::json!({ "drawing": drawing });
        projected.push(Projected::new(
            EntityKind::LayoutDrawing,
            sync_id.clone(),
            sync_id,
            &body,
        ));
    }
    projected
}

/// A layout's portfolio cards point at portfolios by id, and carry it in their key too.
pub(crate) fn map_chart_cards(row: &mut Map<String, Value>, ids: IdSpace, direction: Direction) {
    if let Some(Value::Array(charts)) = row.get_mut("charts") {
        for chart in charts {
            map_card(chart, ids, direction);
        }
    }
}

/// The same for the rows of a watchlist.
pub(crate) fn map_entry_cards(row: &mut Map<String, Value>, ids: IdSpace, direction: Direction) {
    if let Some(Value::Array(entries)) = row.get_mut("entries") {
        for entry in entries {
            // A row is an enum, stored as `{"Instrument": {...}}`.
            match entry.as_object_mut() {
                Some(variant) if variant.len() == 1 => {
                    if let Some(inner) = variant.values_mut().next() {
                        map_card(inner, ids, direction);
                    }
                }
                _ => map_card(entry, ids, direction),
            }
        }
    }
}

/// A stored drawing is `{"<Variant>": {"id": n, ...}}`; an unreadable one keeps the same shape.
pub(crate) fn drawing_id(drawing: &Value) -> Option<u64> {
    drawing.as_object()?.values().next()?.get("id")?.as_u64()
}

/// A user script as the scanner sees it. Scripts live in files, not SQLite, so the caller builds
/// this from the script library. Drafts and revisions never sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSnapshot {
    pub id: String,
    pub name: String,
    pub source: String,
    pub is_favorite: bool,
}

pub fn project_script(script: &ScriptSnapshot) -> Projected {
    // Only what makes the script the same script everywhere. Edit times and the declared type are
    // deliberately left out: saving stamps a new time and recomputes the type, so including them
    // would make every apply look like a local edit and bounce between Macs.
    let body = serde_json::json!({
        "id": script.id,
        "name": script.name,
        "source": script.source,
        "is_favorite": script.is_favorite,
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
mod tests;
