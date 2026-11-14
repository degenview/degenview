//! Remote records are untrusted input even though they come from the user's own iCloud: another
//! Mac may run a different version, a bug may have written garbage, and a malformed record must
//! never crash the app or reach a store. Everything here is structural and bounded; semantic
//! repair (broken chart references, duplicate ids) belongs to the merge and apply layers.

use super::policy::EntityKind;
use super::{FAVORITES_ID, MAX_RAW_PAYLOAD_BYTES, SCHEMA_VERSION, SyncRecord};
use crate::store::asset_store::StoredAsset;
use serde_json::Value;

/// Same bound the Pine lexer enforces on a source (`Limits::source_characters`).
pub const MAX_SCRIPT_SOURCE_CHARS: usize = 500_000;
pub const MAX_SCRIPT_NAME_CHARS: usize = 200;
/// Drawings keep their own anchor and text bounds (`model::drawings`).
pub const MAX_DRAWING_ARRAY: usize = crate::model::drawings::MAX_POINTS;
pub const MAX_DRAWING_STRING: usize = 10_000;
pub const MAX_CHARTS_PER_LAYOUT: usize = 64;
pub const MAX_WATCHLIST_ENTRIES: usize = crate::model::watchlist::MAX_ENTRIES_PER_LIST;
pub const MAX_NAME_CHARS: usize = crate::model::watchlist::MAX_NAME_CHARS;
/// A paper account's history is bounded by the record's size, this only stops absurd shapes.
pub const MAX_PAPER_ROWS: usize = 20_000;
pub const MAX_DEVICE_ID_CHARS: usize = 64;
pub const MAX_SETTING_BYTES: usize = 100_000;
pub const MAX_JSON_DEPTH: usize = 64;
/// Prices and timestamps in millisecond epochs stay far below this.
pub const MAX_ABS_NUMBER: f64 = 1e18;

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Safe to hand to the merge layer.
    Accept(Value),
    /// Written by a newer app: keep the raw record, apply after an update.
    NewerSchema,
    /// Unusable; keep the raw record for diagnostics, never apply.
    Reject(String),
}

pub fn validate(record: &SyncRecord) -> Verdict {
    if record.schema_version == 0 {
        return Verdict::Reject("schema version 0".into());
    }
    if record.schema_version > SCHEMA_VERSION {
        return Verdict::NewerSchema;
    }
    if record.payload.len() > MAX_RAW_PAYLOAD_BYTES {
        return reject(format!("payload is {} bytes", record.payload.len()));
    }
    let Ok(value) = serde_json::from_slice::<Value>(&record.payload) else {
        return reject("payload is not valid JSON".into());
    };
    match check(record, &value) {
        Ok(()) => Verdict::Accept(value),
        Err(reason) => Verdict::Reject(reason),
    }
}

fn reject(reason: String) -> Verdict {
    Verdict::Reject(reason)
}

fn check(record: &SyncRecord, value: &Value) -> Result<(), String> {
    if depth(value) > MAX_JSON_DEPTH {
        return Err("payload is nested too deeply".into());
    }
    check_numbers(value)?;
    let kind = record.name.kind;
    let id = record.name.id.as_str();
    match kind {
        EntityKind::Setting => {
            // A key this version keeps on the Mac (MCP, consent, AI preferences, the update channel,
            // anything it does not know) is never written from iCloud, whoever put the record there.
            if !super::policy::setting_is_portable(id) {
                return Err("setting is not synced".into());
            }
            if record.payload.len() > MAX_SETTING_BYTES {
                return Err("setting is too large".into());
            }
            Ok(())
        }
        EntityKind::Layout => {
            let object = object(value)?;
            id_matches_number(object, id, kind)?;
            name_ok(object, "name")?;
            let charts = array(object, "charts")?;
            if charts.len() > MAX_CHARTS_PER_LAYOUT {
                return Err(format!("layout has {} charts", charts.len()));
            }
            for chart in charts {
                let chart = self::object(chart)?;
                chart
                    .get("id")
                    .and_then(Value::as_u64)
                    .ok_or("chart has no id")?;
                check_pine_sources(chart)?;
            }
            Ok(())
        }
        EntityKind::LayoutDrawing => {
            let object = object(value)?;
            if record.name.layout_drawing_parts().is_none() {
                return Err("drawing record name is not layout.chart.drawing".into());
            }
            bound_collections(object.get("drawing").ok_or("drawing is missing")?)
        }
        EntityKind::DrawingAsset => {
            // Held to what a picture is before anything else looks at it: the bytes are decoded
            // and hashed here, so a record whose name is not the hash of its bytes, or which is
            // not a PNG or JPEG of the size it says, never reaches the store.
            let asset: StoredAsset = serde_json::from_value(value.clone())
                .map_err(|_| "a picture record is malformed".to_string())?;
            if asset.id != id {
                return Err("a picture's id is not its record's".into());
            }
            asset
                .verified_bytes()
                .map(|_| ())
                .ok_or_else(|| "a picture is not what its name says".into())
        }
        EntityKind::Watchlist => {
            let object = object(value)?;
            if id != FAVORITES_ID {
                id_matches_number(object, id, kind)?;
            }
            name_ok(object, "name")?;
            let entries = array(object, "entries")?;
            if entries.len() > MAX_WATCHLIST_ENTRIES {
                return Err(format!("watchlist has {} entries", entries.len()));
            }
            // A row this build does not know (a newer shape) keeps the whole list in the
            // inbox unchanged, instead of being stripped when the list is merged.
            for entry in entries {
                let known = entry.as_object().is_some_and(|variant| {
                    variant.len() == 1
                        && (variant.contains_key("Instrument") || variant.contains_key("Section"))
                });
                if !known {
                    return Err("watchlist has a row this version does not know".to_owned());
                }
                if let Some(title) = entry
                    .get("Section")
                    .and_then(|section| section.get("title"))
                    .and_then(Value::as_str)
                    && title.chars().count() > MAX_NAME_CHARS
                {
                    return Err("a section title is too long".to_owned());
                }
            }
            Ok(())
        }
        EntityKind::ChartTemplate | EntityKind::DrawingTemplate | EntityKind::EditorTheme => {
            let object = object(value)?;
            id_matches_number(object, id, kind)?;
            name_ok(object, "name")
        }
        EntityKind::Alert => {
            let object = object(value)?;
            id_matches_number(object, id, kind)
        }
        EntityKind::Portfolio => {
            let object = object(value)?;
            id_matches_number(object, id, kind)?;
            name_ok(object, "name")
        }
        EntityKind::PortfolioTransaction => {
            let object = object(value)?;
            id_matches_number(object, id, kind)?;
            object
                .get("portfolio_id")
                .and_then(Value::as_u64)
                .ok_or("transaction has no portfolio")?;
            Ok(())
        }
        EntityKind::PaperAccount => {
            let object = object(value)?;
            let account = self::object(object.get("account").ok_or("bundle has no account")?)?;
            id_matches_number(account, id, kind)?;
            name_ok(account, "name")?;
            let owner = object
                .get("owner")
                .and_then(Value::as_str)
                .ok_or("bundle has no owner")?;
            if owner.is_empty() || owner.chars().count() > MAX_DEVICE_ID_CHARS {
                return Err("bundle owner is not a device id".into());
            }
            for (key, _) in super::paper::COLLECTIONS {
                let rows = array(object, key)?;
                if rows.len() > MAX_PAPER_ROWS {
                    return Err(format!("`{key}` has {} rows", rows.len()));
                }
                if rows.iter().any(|row| !row.is_object()) {
                    return Err(format!("`{key}` has a row that is not an object"));
                }
            }
            Ok(())
        }
        EntityKind::Script => {
            let object = object(value)?;
            let stored = object
                .get("id")
                .and_then(Value::as_str)
                .ok_or("script has no id")?;
            if stored != id {
                return Err("script id does not match its record name".into());
            }
            if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("script id is not 32 hex digits".into());
            }
            script_name_ok(object.get("name").and_then(Value::as_str))?;
            let source = object
                .get("source")
                .and_then(Value::as_str)
                .ok_or("script has no source")?;
            if source.chars().count() > MAX_SCRIPT_SOURCE_CHARS {
                return Err("script source is too long".into());
            }
            Ok(())
        }
    }
}

fn object(value: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| "payload is not an object".into())
}

fn array<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a Vec<Value>, String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("`{key}` is missing or not a list"))
}

fn id_matches_number(
    object: &serde_json::Map<String, Value>,
    id: &str,
    kind: EntityKind,
) -> Result<(), String> {
    let stored = object
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{} has no numeric id", kind.prefix()))?;
    if stored.to_string() == id {
        Ok(())
    } else {
        Err(format!(
            "{} id does not match its record name",
            kind.prefix()
        ))
    }
}

fn name_ok(object: &serde_json::Map<String, Value>, key: &str) -> Result<(), String> {
    let name = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` is missing"))?;
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("`{key}` is too long"));
    }
    Ok(())
}

/// A script name becomes a file name, so it must not escape the Scripts folder.
fn script_name_ok(name: Option<&str>) -> Result<(), String> {
    let name = name.ok_or("script has no name")?;
    let trimmed = name.trim();
    if trimmed.is_empty()
        || name.chars().count() > MAX_SCRIPT_NAME_CHARS
        || trimmed.starts_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
    {
        return Err("script name is not a legal file name".into());
    }
    Ok(())
}

fn check_pine_sources(chart: &serde_json::Map<String, Value>) -> Result<(), String> {
    let Some(instances) = chart.get("pine_instances").and_then(Value::as_array) else {
        return Ok(());
    };
    for instance in instances {
        let source = instance.get("source").and_then(Value::as_str).unwrap_or("");
        if source.chars().count() > MAX_SCRIPT_SOURCE_CHARS {
            return Err("an applied Pine source is too long".into());
        }
    }
    Ok(())
}

/// Drawing geometry is bounded by array length and string length, whatever the tool.
fn bound_collections(value: &Value) -> Result<(), String> {
    match value {
        Value::Array(items) => {
            if items.len() > MAX_DRAWING_ARRAY {
                return Err("a drawing has too many points".into());
            }
            items.iter().try_for_each(bound_collections)
        }
        Value::Object(map) => map.values().try_for_each(bound_collections),
        Value::String(text) if text.chars().count() > MAX_DRAWING_STRING => {
            Err("a drawing has text that is too long".into())
        }
        _ => Ok(()),
    }
}

fn check_numbers(value: &Value) -> Result<(), String> {
    match value {
        // Integers are always fine (entity ids are nanosecond timestamps near 1.7e18); only
        // floating-point prices and coordinates need a sanity bound.
        Value::Number(number) if number.is_f64() => match number.as_f64() {
            Some(float) if float.is_finite() && float.abs() <= MAX_ABS_NUMBER => Ok(()),
            _ => Err("a number is out of range".into()),
        },
        Value::Array(items) => items.iter().try_for_each(check_numbers),
        Value::Object(map) => map.values().try_for_each(check_numbers),
        _ => Ok(()),
    }
}

fn depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(depth).max().unwrap_or(0),
        Value::Object(map) => 1 + map.values().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
