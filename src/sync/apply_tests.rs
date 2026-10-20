//! Two Macs, one cloud, the real core and the real apply layer: what the user would see.

use super::core::Phase;
use super::core_tests::{Mac, Raw, mac, raw};
use super::mock::MockCloud;
use super::policy::EntityKind;
use crate::model::chart_template::ChartAppearanceTemplate;
use crate::model::watchlist::{Watchlist, WatchlistEntry, WatchlistInstrument};
use crate::model::{ChartAppearance, DataSource, InstrumentId};
use serde_json::{Value, json};

fn watchlist(id: u64, name: &str, markets: &[(u64, &str)], favorites: bool) -> Raw {
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

fn put(mac: &Mac, rows: &[Raw]) {
    mac.store
        .lock()
        .unwrap()
        .replace_documents("watchlist", rows)
        .unwrap();
}

fn rows(mac: &Mac, table: &str) -> Vec<Value> {
    mac.store.lock().unwrap().raw_rows(table).unwrap()
}

fn symbols(row: &Value) -> Vec<String> {
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

fn outbox_len(mac: &Mac) -> usize {
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

fn template(id: u64, name: &str) -> Raw {
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
    assert!(super::apply::apply_batch(&store, &batch, &ctx).is_err());
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
