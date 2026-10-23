//! Remote records are untrusted input even though they come from the user's own iCloud: another
//! Mac may run a different version, a bug may have written garbage, and a malformed record must
//! never crash the app or reach a store. Everything here is structural and bounded; semantic
//! repair (broken chart references, duplicate ids) belongs to the merge and apply layers.

use super::policy::EntityKind;
use super::record::{FAVORITES_ID, MAX_RAW_PAYLOAD_BYTES, SCHEMA_VERSION, SyncRecord};
use crate::store::asset_store::StoredAsset;
use serde_json::Value;

/// Same bound the Pine lexer enforces on a source (`Limits::source_characters`).
pub const MAX_SCRIPT_SOURCE_CHARS: usize = 500_000;
pub const MAX_SCRIPT_NAME_CHARS: usize = 200;
/// Drawings keep their own anchor and text bounds (`model::drawings`).
pub const MAX_DRAWING_ARRAY: usize = crate::model::drawings::MAX_POINTS;
pub const MAX_DRAWING_STRING: usize = 10_000;
pub const MAX_CHARTS_PER_LAYOUT: usize = 64;
pub const MAX_WATCHLIST_ENTRIES: usize = 5_000;
pub const MAX_NAME_CHARS: usize = 200;
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
            Ok(())
        }
        EntityKind::ChartTemplate | EntityKind::DrawingTemplate => {
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
mod tests {
    use super::super::record::RecordName;
    use super::*;
    use serde_json::json;

    fn record(kind: EntityKind, id: &str, payload: Value) -> SyncRecord {
        SyncRecord {
            name: RecordName::new(kind, id).unwrap(),
            schema_version: SCHEMA_VERSION,
            revision: 1,
            origin_device: "d".into(),
            modified_ms: 0,
            payload: serde_json::to_vec(&payload).unwrap(),
        }
    }

    fn rejected(record: &SyncRecord) -> String {
        match validate(record) {
            Verdict::Reject(reason) => reason,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    fn paper_bundle() -> Value {
        json!({
            "account": {"id": 36028797018963970u64, "name": "Paper"},
            "owner": "device-a",
            "orders": [], "fills": [], "positions": [],
            "order_events": [], "closed_trades": [], "journal": [],
        })
    }

    #[test]
    fn malformed_portfolio_and_paper_records_are_rejected() {
        let mismatch = record(EntityKind::Portfolio, "5", json!({"id": 6, "name": "x"}));
        assert!(rejected(&mismatch).contains("does not match"));
        let orphan = record(EntityKind::PortfolioTransaction, "5", json!({"id": 5}));
        assert!(rejected(&orphan).contains("no portfolio"));
        for (key, bad) in [
            ("owner", json!("")),
            ("owner", json!(7)),
            ("orders", json!({})),
            ("fills", json!([1])),
            ("account", json!({"id": 9, "name": "other"})),
        ] {
            let mut bundle = paper_bundle();
            bundle[key] = bad;
            let case = record(EntityKind::PaperAccount, "36028797018963970", bundle);
            assert!(matches!(validate(&case), Verdict::Reject(_)), "{key}");
        }
        let mut missing = paper_bundle();
        missing.as_object_mut().unwrap().remove("journal");
        let case = record(EntityKind::PaperAccount, "36028797018963970", missing);
        assert!(matches!(validate(&case), Verdict::Reject(_)));
    }

    const HEX: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_well_formed_record_of_every_kind_is_accepted() {
        let cases = [
            record(EntityKind::Setting, "appTheme", json!("dark")),
            record(
                EntityKind::Layout,
                "5",
                json!({"id": 5, "name": "Main", "charts": [{"id": 1, "pine_instances": []}]}),
            ),
            record(
                EntityKind::Watchlist,
                "7",
                json!({"id": 7, "name": "Tech", "entries": []}),
            ),
            record(
                EntityKind::Watchlist,
                "favorites",
                json!({"id": 99, "name": "Favorites", "entries": []}),
            ),
            record(
                EntityKind::ChartTemplate,
                "1001",
                json!({"id": 1001, "name": "Dark"}),
            ),
            record(
                EntityKind::DrawingTemplate,
                "3",
                json!({"id": 3, "name": "Red"}),
            ),
            record(EntityKind::Alert, "8", json!({"id": 8})),
            record(
                EntityKind::Portfolio,
                "36028797018963969",
                json!({"id": 36028797018963969u64, "name": "Main"}),
            ),
            record(
                EntityKind::PortfolioTransaction,
                "1700000000000000000",
                json!({"id": 1700000000000000000u64, "portfolio_id": 36028797018963969u64}),
            ),
            record(
                EntityKind::PaperAccount,
                "36028797018963970",
                paper_bundle(),
            ),
            record(
                EntityKind::Script,
                HEX,
                json!({"id": HEX, "name": "My Strategy", "source": "//@version=6\n"}),
            ),
        ];
        for case in cases {
            assert!(
                matches!(validate(&case), Verdict::Accept(_)),
                "{:?}",
                case.name
            );
        }
    }

    #[test]
    fn a_newer_schema_is_preserved_not_rejected() {
        let mut newer = record(EntityKind::Alert, "8", json!({"id": 8}));
        newer.schema_version = SCHEMA_VERSION + 1;
        assert_eq!(validate(&newer), Verdict::NewerSchema);
    }

    #[test]
    fn schema_zero_is_rejected() {
        let mut zero = record(EntityKind::Alert, "8", json!({"id": 8}));
        zero.schema_version = 0;
        assert!(rejected(&zero).contains("schema"));
    }

    #[test]
    fn oversize_and_unparseable_payloads_are_rejected() {
        let mut big = record(EntityKind::Setting, "x", json!(1));
        big.payload = vec![b' '; MAX_RAW_PAYLOAD_BYTES + 1];
        assert!(rejected(&big).contains("bytes"));
        let mut junk = record(EntityKind::Setting, "x", json!(1));
        junk.payload = b"{not json".to_vec();
        assert!(rejected(&junk).contains("JSON"));
    }

    #[test]
    fn deep_nesting_is_rejected() {
        let mut value = json!(1);
        for _ in 0..(MAX_JSON_DEPTH + 5) {
            value = json!([value]);
        }
        let deep = record(EntityKind::Setting, "x", value);
        assert!(rejected(&deep).contains("nested"));
    }

    #[test]
    fn nanosecond_entity_ids_are_valid() {
        let id = 1_792_487_796_669_773_000_u64;
        let r = record(
            EntityKind::Watchlist,
            &id.to_string(),
            json!({"id": id, "name": "x", "entries": [{"Section": {"id": id, "title": "t"}}]}),
        );
        assert!(matches!(validate(&r), Verdict::Accept(_)));
        let layout = record(
            EntityKind::Layout,
            &id.to_string(),
            json!({"id": id, "name": "x", "charts": [{"id": 1, "created": id}]}),
        );
        assert!(matches!(validate(&layout), Verdict::Accept(_)));
    }

    #[test]
    fn numbers_outside_a_sane_range_are_rejected() {
        let huge = record(EntityKind::Setting, "x", json!({"price": 1e300}));
        assert!(rejected(&huge).contains("range"));
        let fine = record(EntityKind::Setting, "x", json!({"price": 12345.5}));
        assert!(matches!(validate(&fine), Verdict::Accept(_)));
    }

    #[test]
    fn ids_must_match_their_record_names() {
        assert!(rejected(&record(EntityKind::Alert, "8", json!({"id": 9}))).contains("match"));
        assert!(rejected(&record(EntityKind::Alert, "8", json!({}))).contains("numeric"));
        let wrong = record(
            EntityKind::Script,
            HEX,
            json!({"id": "ffffffffffffffffffffffffffffffff", "name": "a", "source": ""}),
        );
        assert!(rejected(&wrong).contains("match"));
    }

    #[test]
    fn a_script_id_must_be_32_hex_digits() {
        let bad = record(
            EntityKind::Script,
            "short",
            json!({"id": "short", "name": "a", "source": ""}),
        );
        assert!(rejected(&bad).contains("32 hex"));
    }

    #[test]
    fn script_names_cannot_escape_the_scripts_folder() {
        for name in [
            "../evil",
            "a/b",
            ".hidden",
            "",
            "   ",
            "a\\b",
            "bad\u{0}name",
        ] {
            let r = record(
                EntityKind::Script,
                HEX,
                json!({"id": HEX, "name": name, "source": ""}),
            );
            assert!(rejected(&r).contains("file name"), "{name:?}");
        }
    }

    #[test]
    fn a_script_source_over_the_pine_limit_is_rejected() {
        let long = "a".repeat(MAX_SCRIPT_SOURCE_CHARS + 1);
        let r = record(
            EntityKind::Script,
            HEX,
            json!({"id": HEX, "name": "a", "source": long}),
        );
        assert!(rejected(&r).contains("too long"));
    }

    #[test]
    fn a_layout_with_too_many_charts_or_a_huge_pine_source_is_rejected() {
        let charts: Vec<_> = (0..=MAX_CHARTS_PER_LAYOUT)
            .map(|id| json!({"id": id}))
            .collect();
        let many = record(
            EntityKind::Layout,
            "5",
            json!({"id": 5, "name": "x", "charts": charts}),
        );
        assert!(rejected(&many).contains("charts"));
        let long = "a".repeat(MAX_SCRIPT_SOURCE_CHARS + 1);
        let pine = record(
            EntityKind::Layout,
            "5",
            json!({"id": 5, "name": "x", "charts": [{"id": 1, "pine_instances": [{"source": long}]}]}),
        );
        assert!(rejected(&pine).contains("Pine"));
    }

    fn picture_record() -> (String, serde_json::Value) {
        let image =
            image::RgbaImage::from_fn(20, 20, |x, y| image::Rgba([x as u8, y as u8, 9, 255]));
        let mut png = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut png),
            image.as_raw(),
            20,
            20,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        let kept = crate::service::image_ingest::ingest(&png).unwrap();
        let asset = crate::store::asset_store::StoredAsset::from_ingested(&kept);
        (asset.id.clone(), serde_json::to_value(asset).unwrap())
    }

    #[test]
    fn a_picture_record_is_accepted_only_when_it_is_what_its_name_says() {
        let (id, good) = picture_record();
        let ok = record(EntityKind::DrawingAsset, &id, good.clone());
        assert!(matches!(validate(&ok), Verdict::Accept(_)));
        // Another name: the record is not the picture its name claims.
        let renamed = record(EntityKind::DrawingAsset, &"b".repeat(64), good.clone());
        assert!(rejected(&renamed).contains("id"));
        // Bytes that do not hash to the name.
        let mut tampered = good.clone();
        tampered["data"] = json!("AAAAAAAA");
        assert!(rejected(&record(EntityKind::DrawingAsset, &id, tampered)).contains("picture"));
        // A type or size it does not have.
        for (field, value) in [
            ("mime", json!("image/png")),
            ("width", json!(21)),
            ("height", json!(0)),
        ] {
            let mut wrong = good.clone();
            wrong[field] = value;
            assert!(
                matches!(
                    validate(&record(EntityKind::DrawingAsset, &id, wrong)),
                    Verdict::Reject(_)
                ),
                "{field}"
            );
        }
        // Not a picture record at all.
        let junk = record(EntityKind::DrawingAsset, &id, json!({"id": id}));
        assert!(matches!(validate(&junk), Verdict::Reject(_)));
        let oversized = {
            let mut big = good.clone();
            big["data"] = json!("A".repeat(2_000_000));
            record(EntityKind::DrawingAsset, &id, big)
        };
        assert!(matches!(validate(&oversized), Verdict::Reject(_)));
    }

    #[test]
    fn a_layout_chart_needs_an_id() {
        let r = record(
            EntityKind::Layout,
            "5",
            json!({"id": 5, "name": "x", "charts": [{}]}),
        );
        assert!(rejected(&r).contains("chart"));
    }

    #[test]
    fn drawing_geometry_is_bounded() {
        let points: Vec<_> = (0..=MAX_DRAWING_ARRAY)
            .map(|i| json!({"t": i, "p": 1.0}))
            .collect();
        let many = record(
            EntityKind::LayoutDrawing,
            "1.2.3",
            json!({"drawing": {"Brush": {"anchors": points}}}),
        );
        assert!(rejected(&many).contains("points"));
        let text = record(
            EntityKind::LayoutDrawing,
            "1.2.3",
            json!({"drawing": {"text": "a".repeat(MAX_DRAWING_STRING + 1)}}),
        );
        assert!(rejected(&text).contains("text"));
        let ok = record(
            EntityKind::LayoutDrawing,
            "1.2.3",
            json!({"drawing": {"Unreadable": {"future": true}}}),
        );
        assert!(matches!(validate(&ok), Verdict::Accept(_)));
    }

    #[test]
    fn a_drawing_record_needs_a_composite_name_and_an_order() {
        let bad_name = record(EntityKind::LayoutDrawing, "1.2", json!({"drawing": {}}));
        assert!(rejected(&bad_name).contains("layout.chart.drawing"));
        let no_drawing = record(EntityKind::LayoutDrawing, "1.2.3", json!({}));
        assert!(rejected(&no_drawing).contains("missing"));
    }

    #[test]
    fn oversize_watchlists_and_settings_are_rejected() {
        let entries: Vec<_> = (0..=MAX_WATCHLIST_ENTRIES)
            .map(|id| json!({"id": id}))
            .collect();
        let list = record(
            EntityKind::Watchlist,
            "7",
            json!({"id": 7, "name": "x", "entries": entries}),
        );
        assert!(rejected(&list).contains("entries"));
        let setting = record(
            EntityKind::Setting,
            "x",
            json!("a".repeat(MAX_SETTING_BYTES + 1)),
        );
        assert!(rejected(&setting).contains("setting"));
    }

    #[test]
    fn a_non_object_payload_is_rejected_where_an_object_is_required() {
        assert!(rejected(&record(EntityKind::Layout, "5", json!([1]))).contains("object"));
        assert!(rejected(&record(EntityKind::Watchlist, "7", json!("x"))).contains("object"));
    }

    #[test]
    fn long_names_are_rejected() {
        let r = record(
            EntityKind::ChartTemplate,
            "1001",
            json!({"id": 1001, "name": "a".repeat(MAX_NAME_CHARS + 1)}),
        );
        assert!(rejected(&r).contains("too long"));
    }
}
