//! Two Macs, one cloud, the real core and the real apply layer: what the user would see.

use super::core::Phase;
use super::core_tests::{Mac, Raw, mac, raw};
use super::mock::MockCloud;
use super::policy::EntityKind;
use crate::model::chart_template::ChartAppearanceTemplate;
use crate::model::watchlist::{Watchlist, WatchlistEntry, WatchlistInstrument};
use crate::model::{ChartAppearance, DataSource, InstrumentId};
use serde_json::{Value, json};

pub(super) fn watchlist(id: u64, name: &str, markets: &[(u64, &str)], favorites: bool) -> Raw {
    let list = Watchlist {
        id,
        name: name.into(),
        entries: markets
            .iter()
            .map(|(entry, symbol)| {
                WatchlistEntry::Instrument(WatchlistInstrument {
                    id: *entry,
                    instrument: InstrumentId::new(DataSource::Binance, *symbol),
                    name: (*symbol).into(),
                    label: (*symbol).into(),
                    display_name: None,
                    image_url: None,
                    card: None,
                })
            })
            .collect(),
        display: Default::default(),
        is_favorites: favorites,
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    Raw(serde_json::to_value(list).unwrap())
}

pub(super) fn put(mac: &Mac, rows: &[Raw]) {
    mac.store
        .lock()
        .unwrap()
        .replace_documents("watchlist", rows)
        .unwrap();
}

pub(super) fn rows(mac: &Mac, table: &str) -> Vec<Value> {
    mac.store.lock().unwrap().raw_rows(table).unwrap()
}

pub(super) fn symbols(row: &Value) -> Vec<String> {
    row["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            entry["Instrument"]["instrument"]["symbol"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn pair() -> (MockCloud, Mac, Mac) {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "Mac A", "acct", Some("acct"));
    let mut b = mac(&cloud, "Mac B", "acct", Some("acct"));
    a.apply = true;
    b.apply = true;
    (cloud, a, b)
}

/// Lets both Macs upload and fetch until nothing moves.
fn settle(a: &mut Mac, b: &mut Mac) {
    for _ in 0..6 {
        a.core.request_sync();
        a.run();
        b.core.request_sync();
        b.run();
    }
}

pub(super) fn outbox_len(mac: &Mac) -> usize {
    mac.outbox().len()
}

#[test]
fn a_list_made_on_one_mac_appears_on_the_other_without_echoing_back() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[watchlist(10, "Tech", &[(1, "AAA")], false)]);
    settle(&mut a, &mut b);
    let on_b = rows(&b, "watchlist");
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0]["name"], "Tech");
    assert_eq!(symbols(&on_b[0]), ["AAA"]);
    assert_eq!(outbox_len(&b), 0, "applying must not queue an upload");
    assert_eq!(outbox_len(&a), 0);
    assert_eq!(cloud.names(), ["watchlist.10"]);
    assert_eq!(a.phase(), Phase::UpToDate);
    assert_eq!(b.phase(), Phase::UpToDate);
}

#[test]
fn an_edit_travels_back_and_forth() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[watchlist(10, "Tech", &[(1, "AAA")], false)]);
    settle(&mut a, &mut b);
    put(
        &b,
        &[watchlist(10, "Tech", &[(1, "AAA"), (2, "BBB")], false)],
    );
    settle(&mut a, &mut b);
    assert_eq!(symbols(&rows(&a, "watchlist")[0]), ["AAA", "BBB"]);
    put(
        &a,
        &[watchlist(10, "Renamed", &[(1, "AAA"), (2, "BBB")], false)],
    );
    settle(&mut a, &mut b);
    assert_eq!(rows(&b, "watchlist")[0]["name"], "Renamed");
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn concurrent_additions_on_both_macs_are_both_kept() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[watchlist(10, "Tech", &[(1, "AAA")], false)]);
    settle(&mut a, &mut b);
    // Both edit while neither has seen the other's change.
    put(
        &a,
        &[watchlist(10, "Tech", &[(1, "AAA"), (2, "XXX")], false)],
    );
    put(
        &b,
        &[watchlist(10, "Tech", &[(1, "AAA"), (3, "YYY")], false)],
    );
    settle(&mut a, &mut b);
    let (la, lb) = (rows(&a, "watchlist"), rows(&b, "watchlist"));
    assert_eq!(symbols(&la[0]), symbols(&lb[0]));
    let mut merged = symbols(&la[0]);
    merged.sort();
    assert_eq!(merged, ["AAA", "XXX", "YYY"]);
    assert_eq!(la, lb, "both Macs hold byte-identical rows");
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn additions_uploaded_before_either_mac_fetches_are_both_kept() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[watchlist(10, "Tech", &[(1, "AAA")], false)]);
    settle(&mut a, &mut b);
    // Both add a market and upload, neither having fetched the other's change: the second upload
    // is refused by the cloud and has to be merged, not saved over the first.
    put(
        &a,
        &[watchlist(10, "Tech", &[(1, "AAA"), (2, "XXX")], false)],
    );
    put(
        &b,
        &[watchlist(10, "Tech", &[(1, "AAA"), (3, "YYY")], false)],
    );
    a.run();
    b.run();
    settle(&mut a, &mut b);
    for mac in [&a, &b] {
        let mut kept = symbols(&rows(mac, "watchlist")[0]);
        kept.sort();
        assert_eq!(kept, ["AAA", "XXX", "YYY"]);
    }
    assert_eq!(rows(&a, "watchlist"), rows(&b, "watchlist"));
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn the_same_market_added_on_both_macs_appears_once() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[watchlist(10, "Tech", &[], false)]);
    settle(&mut a, &mut b);
    put(&a, &[watchlist(10, "Tech", &[(111, "BTC")], false)]);
    put(&b, &[watchlist(10, "Tech", &[(222, "BTC")], false)]);
    settle(&mut a, &mut b);
    assert_eq!(symbols(&rows(&a, "watchlist")[0]), ["BTC"]);
    assert_eq!(rows(&a, "watchlist"), rows(&b, "watchlist"));
}

#[test]
fn a_deletion_propagates_and_a_stale_mac_does_not_resurrect_the_list() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(
        &a,
        &[
            watchlist(10, "Tech", &[(1, "AAA")], false),
            watchlist(11, "Keep", &[], false),
        ],
    );
    settle(&mut a, &mut b);
    assert_eq!(rows(&b, "watchlist").len(), 2);

    // B goes away; A deletes the list and syncs.
    put(&a, &[watchlist(11, "Keep", &[], false)]);
    a.core.request_sync();
    a.run();
    assert_eq!(cloud.names(), ["watchlist.11"]);

    // B comes back, unchanged: the deletion reaches it and it does not re-upload the list.
    settle(&mut a, &mut b);
    let on_b = rows(&b, "watchlist");
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0]["name"], "Keep");
    assert_eq!(cloud.names(), ["watchlist.11"]);
    assert_eq!(outbox_len(&b), 0);
    let meta = b
        .store
        .lock()
        .unwrap()
        .entity_meta(EntityKind::Watchlist, "10")
        .unwrap()
        .unwrap();
    assert!(meta.deleted);
}

#[test]
fn an_edit_made_offline_beats_a_deletion_made_elsewhere() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(
        &a,
        &[
            watchlist(10, "Tech", &[(1, "AAA")], false),
            watchlist(11, "Keep", &[], false),
        ],
    );
    settle(&mut a, &mut b);

    put(&a, &[watchlist(11, "Keep", &[], false)]);
    a.core.request_sync();
    a.run();
    // B edited the list while offline.
    put(
        &b,
        &[
            watchlist(10, "Tech", &[(1, "AAA"), (2, "BBB")], false),
            watchlist(11, "Keep", &[], false),
        ],
    );
    settle(&mut a, &mut b);
    let on_a = rows(&a, "watchlist");
    assert_eq!(on_a.len(), 2, "the list came back on A");
    assert!(
        on_a.iter()
            .any(|row| row["name"] == "Tech" && symbols(row) == ["AAA", "BBB"])
    );
    assert_eq!(rows(&b, "watchlist").len(), 2);
    assert!(cloud.names().contains(&"watchlist.10".to_string()));
}

#[test]
fn each_macs_favorites_list_merges_without_ping_pong() {
    let (cloud, mut a, mut b) = pair();
    put(&a, &[watchlist(1111, "Favorites", &[(1, "AAA")], true)]);
    put(&b, &[watchlist(2222, "Favorites", &[(2, "BBB")], true)]);
    a.core.start();
    b.core.start();
    settle(&mut a, &mut b);
    let (la, lb) = (rows(&a, "watchlist"), rows(&b, "watchlist"));
    assert_eq!(la.len(), 1);
    assert_eq!(lb.len(), 1);
    assert_eq!(la[0]["id"], 1111, "each Mac keeps its own row id");
    assert_eq!(lb[0]["id"], 2222);
    let mut both = symbols(&la[0]);
    both.sort();
    assert_eq!(both, ["AAA", "BBB"]);
    assert_eq!(symbols(&la[0]), symbols(&lb[0]));
    assert!(la[0]["is_favorites"].as_bool().unwrap());
    assert_eq!(cloud.names(), ["watchlist.favorites"]);
    // Nothing keeps moving once they agree.
    let saves = cloud.save_attempts();
    settle(&mut a, &mut b);
    assert_eq!(cloud.save_attempts(), saves);
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

pub(super) fn template(id: u64, name: &str) -> Raw {
    let t = ChartAppearanceTemplate {
        id,
        name: name.into(),
        description: None,
        appearance: ChartAppearance::default(),
        created_at: 1,
        updated_at: 2,
    };
    Raw(serde_json::to_value(t).unwrap())
}

#[test]
fn templates_sync_and_built_in_ids_are_refused() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    a.store
        .lock()
        .unwrap()
        .replace_documents("chart_template", &[template(1500, "Mine")])
        .unwrap();
    settle(&mut a, &mut b);
    let on_b = rows(&b, "chart_template");
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0]["name"], "Mine");
    assert_eq!(outbox_len(&b), 0);

    // A record claiming a built-in preset's id is not applied, only kept.
    let forged = serde_json::to_value(template(5, "Pretends to be built in")).unwrap();
    cloud.put_from_elsewhere(raw("chart_template.5", &forged, 1, 1, "Z"), "Z");
    settle(&mut a, &mut b);
    assert_eq!(rows(&b, "chart_template").len(), 1);
    let kept = b.store.lock().unwrap().inbox().unwrap();
    assert!(
        kept.iter()
            .any(|r| r.record_name == "chart_template.5" && r.reason == "invalid")
    );
}

#[test]
fn a_portable_setting_travels_and_local_ones_do_not() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    {
        let store = a.store.lock().unwrap();
        store.set_setting("appTheme", &"dark").unwrap();
        store
            .set_setting("alert.health", &json!({"heartbeat_ms": 5}))
            .unwrap();
        store
            .set_setting("mcp.settings", &json!({"port": 4242}))
            .unwrap();
    }
    settle(&mut a, &mut b);
    let store = b.store.lock().unwrap();
    assert_eq!(store.raw_setting("appTheme").unwrap(), Some(json!("dark")));
    assert_eq!(store.raw_setting("alert.health").unwrap(), None);
    assert_eq!(store.raw_setting("mcp.settings").unwrap(), None);
    drop(store);
    assert_eq!(outbox_len(&b), 0);
}

#[test]
fn snapshot_preferences_never_overwrite_this_macs_consented_hosts() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    b.store
        .lock()
        .unwrap()
        .set_setting(
            "snapshot.prefs",
            &json!({"host": null, "include_trading": false, "scale": "X1", "consented": ["trusted.example"]}),
        )
        .unwrap();
    // The later edit wins; two edits in one millisecond would be decided by random device ids.
    std::thread::sleep(std::time::Duration::from_millis(3));
    a.store
        .lock()
        .unwrap()
        .set_setting(
            "snapshot.prefs",
            &json!({"host": "snaps.example", "include_trading": true, "scale": "X2", "consented": ["evil.example"]}),
        )
        .unwrap();
    settle(&mut a, &mut b);
    let prefs = b
        .store
        .lock()
        .unwrap()
        .raw_setting("snapshot.prefs")
        .unwrap()
        .unwrap();
    assert_eq!(prefs["host"], "snaps.example");
    assert_eq!(prefs["include_trading"], true);
    assert_eq!(prefs["consented"], json!(["trusted.example"]));
}

#[test]
fn a_record_this_version_cannot_decode_is_kept_not_applied() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    // Valid envelope, but the entries are not something the app's model can read.
    let broken = json!({"id": 77, "name": "Odd", "entries": [{"Instrument": {"nope": true}}]});
    cloud.put_from_elsewhere(raw("watchlist.77", &broken, 1, 1, "Z"), "Z");
    settle(&mut a, &mut b);
    assert!(rows(&b, "watchlist").is_empty());
    let kept = b.store.lock().unwrap().inbox().unwrap();
    assert!(
        kept.iter()
            .any(|r| r.record_name == "watchlist.77" && r.reason == "invalid")
    );
}

#[test]
fn a_table_holding_a_row_from_a_newer_version_is_never_rewritten() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    // A row this version cannot read (say, written by a newer app).
    b.store
        .lock()
        .unwrap()
        .execute_raw(
            "INSERT INTO watchlist (id, position, payload) VALUES ('999', 0, '{\"future\": true}')",
        )
        .unwrap();
    put_remote(&cloud, 10, "Tech");
    settle(&mut a, &mut b);
    let kept = b.store.lock().unwrap().inbox().unwrap();
    assert!(
        kept.iter()
            .any(|r| r.record_name == "watchlist.10" && r.reason == "local_unreadable")
    );
    let on_b = b.store.lock().unwrap().raw_rows("watchlist").unwrap();
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0], json!({"future": true}));
}

fn put_remote(cloud: &MockCloud, id: u64, name: &str) {
    let payload: Value = serde_json::to_value(watchlist(id, name, &[], false).0).unwrap();
    cloud.put_from_elsewhere(raw(&format!("watchlist.{id}"), &payload, 1, 1, "Z"), "Z");
}

#[test]
fn delivering_the_same_changes_twice_is_harmless() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[watchlist(10, "Tech", &[(1, "AAA")], false)]);
    settle(&mut a, &mut b);
    let batch = b.batches.last().cloned().expect("B received a batch");
    let before = rows(&b, "watchlist");
    let report = b.apply_to_store(&batch);
    assert_eq!(rows(&b, "watchlist"), before);
    assert_eq!(outbox_len(&b), 0);
    assert_eq!(report.inboxed, 0);
}

#[test]
fn an_older_version_arriving_late_does_not_overwrite_a_newer_one() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[watchlist(10, "v1", &[], false)]);
    settle(&mut a, &mut b);
    put(&a, &[watchlist(10, "v2", &[], false)]);
    put(&a, &[watchlist(10, "v3", &[], false)]);
    settle(&mut a, &mut b);
    assert_eq!(rows(&b, "watchlist")[0]["name"], "v3");
    // The first delivery is replayed out of order.
    let first = b.batches.first().cloned().unwrap();
    let report = b.apply_to_store(&first);
    assert_eq!(rows(&b, "watchlist")[0]["name"], "v3");
    assert_eq!(report.kept_local, 1);
}

#[test]
fn a_remote_delete_of_a_never_synced_list_just_records_the_tombstone() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let record = super::core::RemoteBatch {
        id: 99,
        records: vec![],
        deletions: vec![super::record::RecordName::new(EntityKind::Watchlist, "12345").unwrap()],
    };
    let report = b.apply_to_store(&record);
    assert_eq!(report.applied, 0);
    let meta = b
        .store
        .lock()
        .unwrap()
        .entity_meta(EntityKind::Watchlist, "12345")
        .unwrap()
        .unwrap();
    assert!(meta.deleted);
}

#[test]
fn a_failed_apply_changes_nothing() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&b, &[watchlist(10, "Mine", &[], false)]);
    settle(&mut a, &mut b);
    let before = rows(&b, "watchlist");
    // Make the next write fail partway, after the entity was linked.
    b.store
        .lock()
        .unwrap()
        .execute_raw(
            "CREATE TRIGGER fail_write BEFORE INSERT ON watchlist BEGIN SELECT RAISE(ABORT, 'disk full'); END",
        )
        .unwrap();
    let payload = serde_json::to_value(watchlist(10, "Mine", &[(1, "NEW")], false).0).unwrap();
    let batch = super::core::RemoteBatch {
        id: 5,
        records: vec![super::core::RemoteRecord {
            record: super::record::SyncRecord {
                name: super::record::RecordName::new(EntityKind::Watchlist, "10").unwrap(),
                schema_version: 1,
                revision: 50,
                origin_device: "Z".into(),
                modified_ms: 9,
                payload: serde_json::to_vec(&payload).unwrap(),
            },
            value: payload,
            system_fields: vec![1],
        }],
        deletions: vec![],
    };
    let state = b.store.lock().unwrap().sync_state().unwrap().unwrap();
    let ctx = super::apply::ApplyCtx {
        device_id: state.device_id,
        device_name: state.device_name,
        now_ms: 5,
        date: "2026-10-20".into(),
    };
    let store = b.store.lock().unwrap();
    assert!(
        super::apply::apply_batch(&store, &mut super::scripts::NoScripts, &batch, &ctx).is_err()
    );
    let meta = store
        .entity_meta(EntityKind::Watchlist, "10")
        .unwrap()
        .unwrap();
    drop(store);
    assert_eq!(rows(&b, "watchlist"), before);
    assert_ne!(
        meta.revision, 50,
        "the link was rolled back with the failed write"
    );
}

// MARK: Layouts and drawings

fn trend(id: u64, price: f64) -> Value {
    json!({"TrendLine": {"id": id, "a": {"time_ms": 1, "price": price}, "b": {"time_ms": 2, "price": price}}})
}

/// A saved layout as stored. `charts` is `(chart id, symbol, drawings as (id, price))`.
/// A chart in a test layout: its id, symbol, and drawings as `(id, price)`.
type TestChart<'a> = (u64, &'a str, &'a [(u64, f64)]);

fn layout(id: u64, name: &str, charts: &[TestChart]) -> Raw {
    let indicators = serde_json::to_value(crate::model::IndicatorConfig::default()).unwrap();
    let source = serde_json::to_value(DataSource::Binance).unwrap();
    let charts: Vec<Value> = charts
        .iter()
        .map(|(chart, symbol, drawings)| {
            json!({
                "id": chart, "symbol": symbol, "source": source, "log_scale": false,
                "y_zoom": 1.0, "indicators": indicators,
                "drawings": drawings.iter().map(|(d, p)| trend(*d, *p)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let value = json!({
        "id": id, "name": name, "timeframe": "Day", "created_at_ms": 1, "charts": charts,
        "candle_count": 0, "last_opened_at_ms": 5, "autosave": true,
    });
    assert!(
        crate::store::sync::row_decodes("saved_view", &value.to_string()),
        "test layout must decode"
    );
    Raw(value)
}

fn put_layouts(mac: &Mac, layouts: &[Raw]) {
    mac.store
        .lock()
        .unwrap()
        .replace_documents("saved_view", layouts)
        .unwrap();
}

fn drawings_of(row: &Value, chart: u64) -> Vec<(u64, f64)> {
    row["charts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == chart)
        .unwrap()["drawings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let body = d.as_object().unwrap().values().next().unwrap();
            (
                body["id"].as_u64().unwrap(),
                body["a"]["price"].as_f64().unwrap(),
            )
        })
        .collect()
}

fn layout_named<'a>(layouts: &'a [Value], name: &str) -> &'a Value {
    layouts
        .iter()
        .find(|l| l["name"] == name)
        .unwrap_or_else(|| panic!("no layout {name}"))
}

#[test]
fn a_layout_and_its_drawings_arrive_as_separate_records() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[(1, 1.0), (2, 2.0)])])],
    );
    settle(&mut a, &mut b);
    assert_eq!(
        cloud.names(),
        ["layout.10", "layoutdrawing.10.1.1", "layoutdrawing.10.1.2"]
    );
    let on_b = rows(&b, "saved_view");
    assert_eq!(on_b.len(), 1);
    assert_eq!(drawings_of(&on_b[0], 1), [(1, 1.0), (2, 2.0)]);
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn per_device_layout_fields_stay_per_device() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(&a, &[layout(10, "Main", &[(1, "BTC", &[])])]);
    settle(&mut a, &mut b);
    let on_b = rows(&b, "saved_view");
    assert!(
        on_b[0].get("last_opened_at_ms").is_none_or(Value::is_null),
        "not A's open time"
    );
    assert!(
        on_b[0].get("autosave").is_none_or(Value::is_null),
        "not A's autosave choice"
    );
    // B turns autosave on and later receives a structural change from A: it keeps its choice.
    let mut row = on_b[0].clone();
    row["autosave"] = json!(true);
    row["last_opened_at_ms"] = json!(777);
    b.store
        .lock()
        .unwrap()
        .replace_documents("saved_view", &[Raw(row)])
        .unwrap();
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[]), (2, "ETH", &[])])],
    );
    settle(&mut a, &mut b);
    let on_b = rows(&b, "saved_view");
    assert_eq!(on_b[0]["charts"].as_array().unwrap().len(), 2);
    assert_eq!(on_b[0]["autosave"], true);
    assert_eq!(on_b[0]["last_opened_at_ms"], 777);
}

#[test]
fn moving_one_drawing_uploads_only_that_drawing() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[(1, 1.0), (2, 2.0)])])],
    );
    settle(&mut a, &mut b);
    let before = cloud.save_attempts();
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[(1, 1.0), (2, 9.5)])])],
    );
    settle(&mut a, &mut b);
    assert_eq!(cloud.save_attempts(), before + 1);
    assert_eq!(
        drawings_of(&rows(&b, "saved_view")[0], 1),
        [(1, 1.0), (2, 9.5)]
    );
}

#[test]
fn edits_to_different_drawings_on_two_macs_both_survive() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[(1, 1.0), (2, 2.0)])])],
    );
    settle(&mut a, &mut b);
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[(1, 5.0), (2, 2.0)])])],
    );
    put_layouts(
        &b,
        &[layout(10, "Main", &[(1, "BTC", &[(1, 1.0), (2, 7.0)])])],
    );
    settle(&mut a, &mut b);
    let (ra, rb) = (rows(&a, "saved_view"), rows(&b, "saved_view"));
    assert_eq!(drawings_of(&ra[0], 1), [(1, 5.0), (2, 7.0)]);
    assert_eq!(ra, rb);
}

#[test]
fn both_macs_editing_the_same_drawing_converge_on_one_version() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(&a, &[layout(10, "Main", &[(1, "BTC", &[(1, 1.0)])])]);
    settle(&mut a, &mut b);
    put_layouts(&a, &[layout(10, "Main", &[(1, "BTC", &[(1, 3.0)])])]);
    put_layouts(&b, &[layout(10, "Main", &[(1, "BTC", &[(1, 4.0)])])]);
    settle(&mut a, &mut b);
    let (ra, rb) = (rows(&a, "saved_view"), rows(&b, "saved_view"));
    assert_eq!(ra, rb);
    assert_eq!(drawings_of(&ra[0], 1).len(), 1);
}

#[test]
fn two_new_drawings_with_the_same_id_are_both_kept() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[(1, 1.0), (2, 2.0)])])],
    );
    settle(&mut a, &mut b);
    // Each Mac draws a new line; both numbered it 3.
    put_layouts(
        &a,
        &[layout(
            10,
            "Main",
            &[(1, "BTC", &[(1, 1.0), (2, 2.0), (3, 10.0)])],
        )],
    );
    put_layouts(
        &b,
        &[layout(
            10,
            "Main",
            &[(1, "BTC", &[(1, 1.0), (2, 2.0), (3, 99.0)])],
        )],
    );
    settle(&mut a, &mut b);
    let (ra, rb) = (rows(&a, "saved_view"), rows(&b, "saved_view"));
    let mut prices: Vec<f64> = drawings_of(&ra[0], 1).into_iter().map(|(_, p)| p).collect();
    prices.sort_by(f64::total_cmp);
    assert_eq!(
        prices,
        [1.0, 2.0, 10.0, 99.0],
        "neither new drawing was lost"
    );
    let ids: std::collections::HashSet<_> = drawings_of(&ra[0], 1)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids.len(), 4, "ids are unique again");
    // Both Macs hold the same drawings; their stacking order is each Mac's own.
    let sorted = |row: &Value| {
        let mut drawings = drawings_of(row, 1);
        drawings.sort_by_key(|x| x.0);
        drawings
    };
    assert_eq!(sorted(&ra[0]), sorted(&rb[0]));
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn deleting_a_drawing_propagates() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[(1, 1.0), (2, 2.0)])])],
    );
    settle(&mut a, &mut b);
    put_layouts(&a, &[layout(10, "Main", &[(1, "BTC", &[(2, 2.0)])])]);
    settle(&mut a, &mut b);
    assert_eq!(drawings_of(&rows(&b, "saved_view")[0], 1), [(2, 2.0)]);
    assert!(!cloud.names().contains(&"layoutdrawing.10.1.1".to_string()));
}

#[test]
fn deleting_the_first_drawing_does_not_touch_the_others() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(
        &a,
        &[layout(
            10,
            "Main",
            &[(1, "BTC", &[(1, 1.0), (2, 2.0), (3, 3.0)])],
        )],
    );
    settle(&mut a, &mut b);
    let before = cloud.save_attempts();
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[(2, 2.0), (3, 3.0)])])],
    );
    settle(&mut a, &mut b);
    assert_eq!(cloud.save_attempts(), before, "only a delete is sent");
    assert_eq!(cloud.delete_attempts(), 1);
    assert_eq!(
        drawings_of(&rows(&b, "saved_view")[0], 1),
        [(2, 2.0), (3, 3.0)]
    );
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn deleting_a_layout_removes_it_and_its_drawings_for_a_stale_mac() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(
        &a,
        &[
            layout(10, "Gone", &[(1, "BTC", &[(1, 1.0), (2, 2.0)])]),
            layout(11, "Kept", &[(1, "ETH", &[])]),
        ],
    );
    settle(&mut a, &mut b);
    put_layouts(&a, &[layout(11, "Kept", &[(1, "ETH", &[])])]);
    settle(&mut a, &mut b);
    let on_b = rows(&b, "saved_view");
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0]["name"], "Kept");
    assert_eq!(cloud.names(), ["layout.11"]);
    assert_eq!(outbox_len(&b), 0);
}

#[test]
fn a_layout_edited_offline_survives_its_deletion_elsewhere_with_its_drawings() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(
        &a,
        &[
            layout(10, "Doomed", &[(1, "BTC", &[(1, 1.0)])]),
            layout(11, "Other", &[(1, "ETH", &[])]),
        ],
    );
    settle(&mut a, &mut b);
    put_layouts(&a, &[layout(11, "Other", &[(1, "ETH", &[])])]);
    a.core.request_sync();
    a.run();
    // B adds a second chart to the layout while offline.
    put_layouts(
        &b,
        &[
            layout(10, "Doomed", &[(1, "BTC", &[(1, 1.0)]), (2, "SOL", &[])]),
            layout(11, "Other", &[(1, "ETH", &[])]),
        ],
    );
    settle(&mut a, &mut b);
    let (ra, rb) = (rows(&a, "saved_view"), rows(&b, "saved_view"));
    assert_eq!(ra.len(), 2);
    let doomed = layout_named(&ra, "Doomed");
    assert_eq!(doomed["charts"].as_array().unwrap().len(), 2);
    assert_eq!(
        drawings_of(doomed, 1),
        [(1, 1.0)],
        "its drawings came back too"
    );
    assert_eq!(ra.len(), rb.len());
    assert!(cloud.names().contains(&"layoutdrawing.10.1.1".to_string()));
}

#[test]
fn conflicting_layout_structures_keep_both_as_a_recoverable_copy() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(&a, &[layout(10, "Main", &[(1, "BTC", &[])])]);
    settle(&mut a, &mut b);
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[]), (2, "ETH", &[])])],
    );
    put_layouts(
        &b,
        &[layout(10, "Main", &[(1, "BTC", &[]), (3, "SOL", &[])])],
    );
    settle(&mut a, &mut b);
    let (ra, rb) = (rows(&a, "saved_view"), rows(&b, "saved_view"));
    assert_eq!(ra.len(), 2, "the original and one conflict copy");
    let symbols_in = |layouts: &[Value]| -> Vec<String> {
        let mut all: Vec<String> = layouts
            .iter()
            .flat_map(|l| {
                l["charts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c["symbol"].as_str().unwrap().to_string())
            })
            .collect();
        all.sort();
        all.dedup();
        all
    };
    assert_eq!(symbols_in(&ra), ["BTC", "ETH", "SOL"], "no chart was lost");
    assert!(
        ra.iter()
            .any(|l| l["name"].as_str().unwrap().contains("conflict from"))
    );
    let names = |layouts: &[Value]| -> Vec<String> {
        let mut n: Vec<String> = layouts
            .iter()
            .map(|l| l["name"].as_str().unwrap().to_string())
            .collect();
        n.sort();
        n
    };
    assert_eq!(
        names(&ra),
        names(&rb),
        "both Macs end with the same layouts"
    );
    let conflicts = a.store.lock().unwrap().conflicts().unwrap().len()
        + b.store.lock().unwrap().conflicts().unwrap().len();
    assert!(conflicts >= 1);
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn layouts_uploaded_before_either_mac_fetches_keep_both_versions() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(&a, &[layout(10, "Main", &[(1, "BTC", &[])])]);
    settle(&mut a, &mut b);
    put_layouts(
        &a,
        &[layout(10, "Main", &[(1, "BTC", &[]), (2, "ETH", &[])])],
    );
    put_layouts(
        &b,
        &[layout(10, "Main", &[(1, "BTC", &[]), (3, "SOL", &[])])],
    );
    // Both upload before either fetches: the second upload is refused by the cloud.
    a.run();
    b.run();
    settle(&mut a, &mut b);
    let (ra, rb) = (rows(&a, "saved_view"), rows(&b, "saved_view"));
    assert_eq!(ra.len(), 2, "the original and one conflict copy");
    assert_eq!(ra, rb);
    let mut symbols: Vec<String> = ra
        .iter()
        .flat_map(|l| {
            l["charts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["symbol"].as_str().unwrap().to_string())
        })
        .collect();
    symbols.sort();
    symbols.dedup();
    assert_eq!(symbols, ["BTC", "ETH", "SOL"], "neither edit was lost");
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn a_layout_renamed_on_one_mac_is_not_a_conflict() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put_layouts(&a, &[layout(10, "Main", &[(1, "BTC", &[])])]);
    settle(&mut a, &mut b);
    put_layouts(&a, &[layout(10, "Renamed", &[(1, "BTC", &[])])]);
    put_layouts(&b, &[layout(10, "Main", &[(1, "BTC", &[])])]);
    settle(&mut a, &mut b);
    assert_eq!(rows(&a, "saved_view").len(), 1);
    assert_eq!(rows(&b, "saved_view").len(), 1);
    assert_eq!(rows(&b, "saved_view")[0]["name"], "Renamed");
}

#[test]
fn a_drawing_that_arrives_before_its_layout_waits_and_is_applied_later() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    a.run();
    b.run();
    // The drawing reaches B first.
    let drawing = json!({"drawing": trend(1, 4.5)});
    cloud.put_from_elsewhere(raw("layoutdrawing.10.1.1", &drawing, 1, 1, "Z"), "Z");
    b.core.request_sync();
    b.run();
    let kept = b.store.lock().unwrap().inbox().unwrap();
    assert!(
        kept.iter()
            .any(|r| r.record_name == "layoutdrawing.10.1.1" && r.reason == "orphan")
    );
    assert!(rows(&b, "saved_view").is_empty());

    // Then its layout arrives.
    let structure = serde_json::to_value(layout(10, "Main", &[(1, "BTC", &[])]).0).unwrap();
    let mut structure = structure;
    structure["charts"][0]
        .as_object_mut()
        .unwrap()
        .remove("drawings");
    structure
        .as_object_mut()
        .unwrap()
        .remove("last_opened_at_ms");
    structure.as_object_mut().unwrap().remove("autosave");
    cloud.put_from_elsewhere(raw("layout.10", &structure, 1, 1, "Z"), "Z");
    b.core.request_sync();
    b.run();
    let on_b = rows(&b, "saved_view");
    assert_eq!(on_b.len(), 1);
    assert_eq!(drawings_of(&on_b[0], 1), [(1, 4.5)]);
    assert!(
        b.store
            .lock()
            .unwrap()
            .inbox()
            .unwrap()
            .iter()
            .all(|r| r.reason != "orphan")
    );
}

#[test]
fn an_unreadable_future_drawing_travels_untouched() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let mut row = layout(10, "Main", &[(1, "BTC", &[(1, 1.0)])]).0;
    let future = json!({"HologramTool": {"id": 2, "anchors": [1, 2, 3], "newer": {"x": true}}});
    row["charts"][0]["drawings"]
        .as_array_mut()
        .unwrap()
        .push(future.clone());
    a.store
        .lock()
        .unwrap()
        .replace_documents("saved_view", &[Raw(row)])
        .unwrap();
    settle(&mut a, &mut b);
    let on_b = rows(&b, "saved_view");
    let drawings = on_b[0]["charts"][0]["drawings"].as_array().unwrap();
    assert!(
        drawings.contains(&future),
        "the future drawing is preserved exactly"
    );
}

// MARK: Alerts

use crate::model::alert::{
    AlertAsset, AlertCondition, AlertRuntimeCommand, AlertRuntimeCommandPayload, AlertState,
    MarketQuote, PriceAlert,
};
use crate::model::alert_engine::AlertEngine;

const T0: i64 = 1_700_000_000_000;

fn btc() -> AlertAsset {
    AlertAsset::new(
        InstrumentId::new(DataSource::Binance, "btcusdt"),
        "BTC",
        "Bitcoin",
    )
}

fn quote(price: f64, at: i64, fingerprint: &str) -> MarketQuote {
    MarketQuote {
        asset: btc(),
        price,
        currency: crate::model::alert::AlertCurrency::default(),
        source_timestamp_ms: at,
        received_at_ms: at,
        maximum_age_secs: 60.0,
        fingerprint: fingerprint.into(),
        candle: None,
    }
}

/// What the process that owns alert evaluation does each second: apply queued commands, honour
/// the per-Mac "not evaluated here" flags, and save the snapshot. Returns the engine.
fn owner_tick(mac: &Mac, now: i64) -> AlertEngine {
    let store = mac.store.lock().unwrap();
    let mut engine = AlertEngine::new(store.load_alert_snapshot().unwrap());
    engine.set_muted(store.muted_alerts());
    let commands = store.pending_alert_commands();
    for command in &commands {
        engine.apply(command, now);
    }
    let ids: Vec<u64> = commands.iter().map(|c| c.id).collect();
    store
        .save_alert_snapshot_and_acknowledge(engine.snapshot(), &ids)
        .unwrap();
    engine
}

fn send_command(mac: &Mac, payload: AlertRuntimeCommandPayload) {
    let command = AlertRuntimeCommand::new(payload, T0);
    mac.store
        .lock()
        .unwrap()
        .enqueue_alert_command(&command)
        .unwrap();
}

fn create_alert(mac: &Mac, target: f64) -> u64 {
    let alert = PriceAlert::new(btc(), AlertCondition::CrossesAbove { target }, T0);
    let id = alert.id;
    send_command(
        mac,
        AlertRuntimeCommandPayload::Save {
            alert: Box::new(alert),
            baseline: Some(90.0),
        },
    );
    owner_tick(mac, T0);
    id
}

/// Lets both Macs sync, with each running its alert owner between rounds.
fn settle_with_alerts(a: &mut Mac, b: &mut Mac) {
    for _ in 0..4 {
        settle(a, b);
        owner_tick(a, T0 + 1);
        owner_tick(b, T0 + 1);
    }
    settle(a, b);
}

fn alerts_on(mac: &Mac) -> Vec<PriceAlert> {
    mac.store
        .lock()
        .unwrap()
        .load_alert_snapshot()
        .unwrap()
        .map(|s| s.alerts)
        .unwrap_or_default()
}

#[test]
fn an_alert_arrives_on_the_other_mac_defined_but_switched_off() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let id = create_alert(&a, 100.0);
    settle_with_alerts(&mut a, &mut b);

    let on_b = alerts_on(&b);
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0].id, id);
    assert_eq!(
        on_b[0].state,
        AlertState::Active,
        "the definition says active"
    );
    let store = b.store.lock().unwrap();
    assert_eq!(
        store.alert_local(&id.to_string()).unwrap(),
        Some((false, "remote".into()))
    );
    assert!(store.muted_alerts().contains(&id));
    drop(store);
    // Applying must not echo back, and the alert owner's rewrite must not either.
    assert_eq!(outbox_len(&b), 0);
    assert_eq!(outbox_len(&a), 0);
    // A still evaluates its own alert.
    assert!(
        a.store
            .lock()
            .unwrap()
            .alert_local(&id.to_string())
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_switched_off_alert_does_not_fire_on_the_receiving_mac_and_can_be_switched_on() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let id = create_alert(&a, 100.0);
    settle_with_alerts(&mut a, &mut b);

    let mut engine = owner_tick(&b, T0 + 2);
    assert!(
        engine
            .process(&quote(90.0, T0 + 3, "q1"), None, None)
            .is_empty()
    );
    assert!(
        engine
            .process(&quote(120.0, T0 + 4, "q2"), None, None)
            .is_empty(),
        "muted: no event"
    );
    assert!(engine.active_assets().is_empty());

    // The user switches it on here: a per-Mac choice that never syncs.
    b.store
        .lock()
        .unwrap()
        .set_alert_local(&id.to_string(), true, "remote")
        .unwrap();
    let mut engine = owner_tick(&b, T0 + 5);
    assert_eq!(engine.active_assets().len(), 1);
    assert!(
        engine
            .process(&quote(150.0, T0 + 6, "q3"), None, None)
            .is_empty(),
        "first quote sets the side"
    );
    assert_eq!(outbox_len(&b), 0, "switching on is not an edit");
}

#[test]
fn alert_runtime_progress_never_uploads() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    create_alert(&a, 100.0);
    settle_with_alerts(&mut a, &mut b);
    let saves = cloud.save_attempts();
    // A's alert crosses, fires, and is spent; the engine rewrites its runtime fields.
    let mut engine = owner_tick(&a, T0 + 10);
    engine.process(&quote(90.0, T0 + 11, "r1"), None, None);
    let fired = engine.process(&quote(101.0, T0 + 12, "r2"), None, None);
    assert_eq!(fired.len(), 1);
    a.store
        .lock()
        .unwrap()
        .save_alert_snapshot_and_acknowledge(engine.snapshot(), &[])
        .unwrap();
    settle_with_alerts(&mut a, &mut b);
    assert_eq!(
        cloud.save_attempts(),
        saves,
        "firing is local progress, not an edit"
    );
    assert_eq!(alerts_on(&a)[0].state, AlertState::Triggered);
    assert_eq!(alerts_on(&b)[0].state, AlertState::Active);
}

#[test]
fn pausing_an_alert_pauses_it_on_the_other_mac() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let id = create_alert(&a, 100.0);
    settle_with_alerts(&mut a, &mut b);
    send_command(
        &a,
        AlertRuntimeCommandPayload::SetState {
            id,
            state: AlertState::Paused,
            baseline: None,
        },
    );
    settle_with_alerts(&mut a, &mut b);
    assert_eq!(alerts_on(&b)[0].state, AlertState::Paused);
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn editing_an_alert_target_updates_the_other_mac() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let id = create_alert(&a, 100.0);
    settle_with_alerts(&mut a, &mut b);
    let mut edited = alerts_on(&a).remove(0);
    edited.condition = AlertCondition::CrossesAbove { target: 250.0 };
    send_command(
        &a,
        AlertRuntimeCommandPayload::Save {
            alert: Box::new(edited),
            baseline: None,
        },
    );
    settle_with_alerts(&mut a, &mut b);
    let on_b = alerts_on(&b);
    assert_eq!(on_b[0].id, id);
    assert_eq!(
        on_b[0].condition,
        AlertCondition::CrossesAbove { target: 250.0 }
    );
    assert!(
        b.store.lock().unwrap().muted_alerts().contains(&id),
        "still off on B"
    );
}

#[test]
fn deleting_an_alert_deletes_it_on_the_other_mac() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let id = create_alert(&a, 100.0);
    settle_with_alerts(&mut a, &mut b);
    send_command(&a, AlertRuntimeCommandPayload::Delete(id));
    settle_with_alerts(&mut a, &mut b);
    assert!(alerts_on(&a).is_empty());
    assert!(alerts_on(&b).is_empty());
    assert!(
        b.store
            .lock()
            .unwrap()
            .alert_local(&id.to_string())
            .unwrap()
            .is_none()
    );
    assert!(!cloud.names().iter().any(|n| n.starts_with("alert.")));
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn an_alert_made_on_each_mac_stays_active_where_it_was_made() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let on_a = create_alert(&a, 100.0);
    let on_b = create_alert(&b, 200.0);
    settle_with_alerts(&mut a, &mut b);
    assert_eq!(alerts_on(&a).len(), 2);
    assert_eq!(alerts_on(&b).len(), 2);
    let muted_a = a.store.lock().unwrap().muted_alerts();
    let muted_b = b.store.lock().unwrap().muted_alerts();
    assert_eq!(muted_a, [on_b].into(), "A evaluates its own, not B's");
    assert_eq!(muted_b, [on_a].into());
}

#[test]
fn a_remote_alert_that_cannot_be_read_is_kept_not_applied() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    // Valid envelope, but the asset is not something this version understands.
    let broken = json!({"id": 4242, "asset": "not an asset", "condition": 5, "intent": "active"});
    cloud.put_from_elsewhere(raw("alert.4242", &broken, 1, 1, "Z"), "Z");
    settle_with_alerts(&mut a, &mut b);
    assert!(alerts_on(&b).is_empty());
    let kept = b.store.lock().unwrap().inbox().unwrap();
    assert!(
        kept.iter()
            .any(|r| r.record_name == "alert.4242" && r.reason == "invalid")
    );
}

// MARK: Category switches

use super::policy::Category;

#[test]
fn a_category_switched_off_neither_uploads_nor_applies() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    a.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Watchlists, false)
        .unwrap();
    b.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Watchlists, false)
        .unwrap();
    put(&a, &[watchlist(10, "Private to A", &[], false)]);
    a.store
        .lock()
        .unwrap()
        .set_setting("appTheme", &"dark")
        .unwrap();
    settle(&mut a, &mut b);
    assert_eq!(
        cloud.names(),
        ["setting.appTheme"],
        "other categories still sync"
    );
    assert!(rows(&b, "watchlist").is_empty());
    assert_eq!(
        b.store.lock().unwrap().raw_setting("appTheme").unwrap(),
        Some(json!("dark"))
    );
}

#[test]
fn incoming_records_of_a_category_that_is_off_wait_and_apply_when_it_returns() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    b.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Watchlists, false)
        .unwrap();
    put(&a, &[watchlist(10, "Waits", &[(1, "AAA")], false)]);
    settle(&mut a, &mut b);
    assert!(rows(&b, "watchlist").is_empty());
    let held = b.store.lock().unwrap().inbox().unwrap();
    assert!(
        held.iter()
            .any(|r| r.record_name == "watchlist.10" && r.reason == "category_off")
    );

    b.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Watchlists, true)
        .unwrap();
    let empty = super::core::RemoteBatch {
        id: 901,
        records: vec![],
        deletions: vec![],
    };
    b.apply_to_store(&empty);
    assert_eq!(rows(&b, "watchlist").len(), 1);
    assert!(
        b.store
            .lock()
            .unwrap()
            .inbox()
            .unwrap()
            .iter()
            .all(|r| r.reason != "category_off")
    );
    assert_eq!(outbox_len(&b), 0);
}

#[test]
fn switching_a_category_back_on_uploads_what_changed_meanwhile() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    a.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Watchlists, false)
        .unwrap();
    put(&a, &[watchlist(10, "Made while off", &[], false)]);
    settle(&mut a, &mut b);
    assert!(cloud.names().is_empty());
    a.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Watchlists, true)
        .unwrap();
    a.store.lock().unwrap().reconcile_stored().unwrap();
    settle(&mut a, &mut b);
    assert_eq!(cloud.names(), ["watchlist.10"]);
    assert_eq!(rows(&b, "watchlist").len(), 1);
}

#[test]
fn categories_default_to_on_and_round_trip() {
    let (_cloud, a, _b) = pair();
    let store = a.store.lock().unwrap();
    assert!(store.disabled_categories().is_empty());
    store
        .set_category_enabled(Category::Scripts, false)
        .unwrap();
    store.set_category_enabled(Category::Alerts, false).unwrap();
    store.set_category_enabled(Category::Alerts, true).unwrap();
    assert_eq!(store.disabled_categories(), [Category::Scripts].into());
}

// MARK: Catching up after being away

use super::transport::{ErrorKind, TransportEvent};

fn names_on(mac: &Mac) -> Vec<String> {
    let mut names: Vec<String> = rows(mac, "watchlist")
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn a_mac_whose_checkpoint_expired_learns_what_was_deleted_while_it_was_away() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(
        &a,
        &[
            watchlist(1, "One", &[], false),
            watchlist(2, "Two", &[], false),
            watchlist(3, "Three", &[], false),
        ],
    );
    settle(&mut a, &mut b);
    assert_eq!(names_on(&b), ["One", "Three", "Two"]);

    // A deletes two lists; iCloud later forgets the deletions, so B can never be told about them.
    put(&a, &[watchlist(3, "Three", &[], false)]);
    a.core.request_sync();
    a.run();
    cloud.compact_deletions();
    // B changed one of them while away: that edit must win over the deletion.
    put(
        &b,
        &[
            watchlist(1, "One", &[], false),
            watchlist(2, "Two, edited", &[], false),
            watchlist(3, "Three", &[], false),
        ],
    );
    b.deliver(TransportEvent::Error {
        kind: ErrorKind::ChangeTokenExpired,
        message: "expired".into(),
    });
    b.run();
    settle(&mut a, &mut b);

    assert_eq!(
        names_on(&b),
        ["Three", "Two, edited"],
        "the untouched list is gone, the edited one stays"
    );
    assert_eq!(
        names_on(&a),
        ["Three", "Two, edited"],
        "and the edit came back to A"
    );
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn a_mac_that_has_been_away_for_months_does_not_trust_its_checkpoint() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(
        &a,
        &[
            watchlist(1, "One", &[], false),
            watchlist(2, "Two", &[], false),
        ],
    );
    settle(&mut a, &mut b);
    put(&a, &[watchlist(2, "Two", &[], false)]);
    a.core.request_sync();
    a.run();
    cloud.compact_deletions();

    // B is relaunched after 61 days.
    b.advance(61 * 24 * 60 * 60 * 1_000);
    let now = b.now.clone();
    let mut core = super::core::SyncCore::new(
        b.store.clone(),
        super::mock::MockTransport::new(&cloud, "Mac B", "acct"),
        Box::new(move || now.load(std::sync::atomic::Ordering::Relaxed)),
    );
    core.start();
    let started_fresh = core.transport_mut().commands.iter().any(|c| {
        matches!(
            c,
            super::transport::TransportCommand::Start {
                engine_state: None,
                ..
            }
        )
    });
    assert!(started_fresh, "the old checkpoint was not used");
    b.core = core;
    b.run();
    assert_eq!(names_on(&b), ["Two"]);
}

#[test]
fn a_normal_incremental_fetch_never_deletes_anything() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[watchlist(1, "One", &[], false)]);
    settle(&mut a, &mut b);
    for _ in 0..3 {
        b.core.request_sync();
        b.run();
    }
    assert_eq!(names_on(&b), ["One"]);
}

#[test]
fn a_catch_up_that_would_remove_most_of_the_library_removes_nothing() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let lists: Vec<_> = (1..=30u64)
        .map(|id| watchlist(id, "x", &[], false))
        .collect();
    put(&a, &lists);
    settle(&mut a, &mut b);
    assert_eq!(rows(&b, "watchlist").len(), 30);
    // iCloud suddenly has none of them (and no deletions to replay).
    cloud.delete_zone_contents();
    b.deliver(TransportEvent::Error {
        kind: ErrorKind::ChangeTokenExpired,
        message: "expired".into(),
    });
    b.run();
    assert_eq!(
        rows(&b, "watchlist").len(),
        30,
        "nothing removed on a suspicious catch-up"
    );
    assert!(b.core.status().last_error.unwrap().contains("were kept"));
}

// MARK: Watchlist flags

fn set_flags(mac: &Mac, flags: Value) {
    mac.store
        .lock()
        .unwrap()
        .set_setting("watchlist.flags", &flags)
        .unwrap();
}

fn flags_of(mac: &Mac) -> Value {
    mac.store
        .lock()
        .unwrap()
        .raw_setting("watchlist.flags")
        .unwrap()
        .unwrap_or(Value::Null)
}

#[test]
fn a_flag_set_on_one_mac_appears_on_the_other() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    set_flags(&a, json!({"binance:BTCUSDT": "Red"}));
    settle(&mut a, &mut b);
    assert_eq!(cloud.names(), ["setting.watchlist.flags"]);
    assert_eq!(flags_of(&b), json!({"binance:BTCUSDT": "Red"}));
    set_flags(&b, json!({"binance:BTCUSDT": "Green"}));
    settle(&mut a, &mut b);
    assert_eq!(flags_of(&a), json!({"binance:BTCUSDT": "Green"}));
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn flags_on_different_markets_from_two_macs_are_both_kept() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    set_flags(&a, json!({"binance:BTCUSDT": "Red"}));
    settle(&mut a, &mut b);
    set_flags(
        &a,
        json!({"binance:BTCUSDT": "Red", "binance:ETHUSDT": "Blue"}),
    );
    set_flags(
        &b,
        json!({"binance:BTCUSDT": "Red", "binance:SOLUSDT": "Green"}),
    );
    a.run();
    b.run();
    settle(&mut a, &mut b);
    let expected = json!({
        "binance:BTCUSDT": "Red",
        "binance:ETHUSDT": "Blue",
        "binance:SOLUSDT": "Green",
    });
    assert_eq!(flags_of(&a), expected);
    assert_eq!(flags_of(&b), expected);
}

#[test]
fn a_flag_removed_on_one_mac_is_not_resurrected_by_the_other() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    set_flags(
        &a,
        json!({"binance:BTCUSDT": "Red", "binance:ETHUSDT": "Blue"}),
    );
    settle(&mut a, &mut b);
    // A clears ETH while B flags SOL, neither having fetched the other's change.
    set_flags(&a, json!({"binance:BTCUSDT": "Red"}));
    set_flags(
        &b,
        json!({
            "binance:BTCUSDT": "Red",
            "binance:ETHUSDT": "Blue",
            "binance:SOLUSDT": "Green",
        }),
    );
    a.run();
    b.run();
    settle(&mut a, &mut b);
    let expected = json!({"binance:BTCUSDT": "Red", "binance:SOLUSDT": "Green"});
    assert_eq!(flags_of(&a), expected);
    assert_eq!(flags_of(&b), expected);
}

#[test]
fn flags_follow_the_watchlists_switch_not_preferences() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    a.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Preferences, false)
        .unwrap();
    set_flags(&a, json!({"binance:BTCUSDT": "Red"}));
    a.store
        .lock()
        .unwrap()
        .set_setting("appTheme", &"dark")
        .unwrap();
    settle(&mut a, &mut b);
    assert_eq!(cloud.names(), ["setting.watchlist.flags"]);
    assert_eq!(flags_of(&b), json!({"binance:BTCUSDT": "Red"}));

    a.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Preferences, true)
        .unwrap();
    a.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::Watchlists, false)
        .unwrap();
    set_flags(&a, json!({"binance:BTCUSDT": "Blue"}));
    settle(&mut a, &mut b);
    assert_eq!(flags_of(&b), json!({"binance:BTCUSDT": "Red"}));
}
