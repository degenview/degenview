use super::super::RecordName;
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

#[test]
fn a_setting_kept_on_this_mac_is_refused_and_a_portable_one_accepted() {
    for key in ["mcp.settings", "content.prefs", "something.new"] {
        let reason = rejected(&record(EntityKind::Setting, key, json!({"a": 1})));
        assert!(reason.contains("not synced"), "{key}: {reason}");
    }
    for key in ["appTheme", "editor.prefs", "watchlist.flags"] {
        assert!(matches!(
            validate(&record(EntityKind::Setting, key, json!({"a": 1}))),
            Verdict::Accept(_)
        ));
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
    let mut big = record(EntityKind::Setting, "appTheme", json!(1));
    big.payload = vec![b' '; MAX_RAW_PAYLOAD_BYTES + 1];
    assert!(rejected(&big).contains("bytes"));
    let mut junk = record(EntityKind::Setting, "appTheme", json!(1));
    junk.payload = b"{not json".to_vec();
    assert!(rejected(&junk).contains("JSON"));
}

#[test]
fn deep_nesting_is_rejected() {
    let mut value = json!(1);
    for _ in 0..(MAX_JSON_DEPTH + 5) {
        value = json!([value]);
    }
    let deep = record(EntityKind::Setting, "appTheme", value);
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
    let huge = record(EntityKind::Setting, "appTheme", json!({"price": 1e300}));
    assert!(rejected(&huge).contains("range"));
    let fine = record(EntityKind::Setting, "appTheme", json!({"price": 12345.5}));
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
    let image = image::RgbaImage::from_fn(20, 20, |x, y| image::Rgba([x as u8, y as u8, 9, 255]));
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        image.as_raw(),
        20,
        20,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    let kept = crate::service::annotation::ingest::ingest(&png).unwrap();
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
fn a_watchlist_row_this_version_does_not_know_is_not_taken() {
    let ok = record(
        EntityKind::Watchlist,
        "7",
        json!({"id": 7, "name": "x", "entries": [
            {"Instrument": {"id": 1}}, {"Section": {"id": 2, "title": "S"}}]}),
    );
    assert!(matches!(validate(&ok), Verdict::Accept(_)));
    for entry in [
        json!({"Futures": {"id": 3}}),
        json!({"Instrument": {"id": 1}, "Extra": {}}),
        json!("a string"),
    ] {
        let list = record(
            EntityKind::Watchlist,
            "7",
            json!({"id": 7, "name": "x", "entries": [entry]}),
        );
        assert!(rejected(&list).contains("does not know"), "{list:?}");
    }
    let long = record(
        EntityKind::Watchlist,
        "7",
        json!({"id": 7, "name": "x", "entries": [
            {"Section": {"id": 2, "title": "t".repeat(MAX_NAME_CHARS + 1)}}]}),
    );
    assert!(rejected(&long).contains("too long"));
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
