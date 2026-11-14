use super::*;
use crate::store::Document;
use crate::sync::record::project::project_row;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct Raw(Value);

impl Document for Raw {
    fn document_id(&self) -> String {
        self.0["id"].to_string()
    }
}

fn store(enabled: bool) -> Store {
    let store = Store::open_in_memory().unwrap();
    let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
    state.enabled = enabled;
    store.save_sync_state(&state).unwrap();
    store
}

fn watchlist(id: u64, name: &str) -> Raw {
    Raw(json!({"id": id, "name": name, "entries": [], "is_favorites": false}))
}

fn put(store: &Store, table: &str, rows: &[Raw]) {
    store.replace_documents(table, rows).unwrap();
}

fn outbox(store: &Store) -> Vec<(String, String, OutboxOp, OutboxState)> {
    store
        .outbox()
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.kind.prefix().to_string(),
                row.sync_id,
                row.op,
                row.state,
            )
        })
        .collect()
}

fn mark_synced(store: &Store, kind: EntityKind, sync_id: &str, hash: &str) {
    store
        .connection
        .execute(
            "UPDATE sync_entity SET synced_hash = ?3, deleted = 0 WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id, hash],
        )
        .unwrap();
}

fn revision(store: &Store, kind: EntityKind, sync_id: &str) -> i64 {
    store
        .connection
        .query_row(
            "SELECT logical_revision FROM sync_entity WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id],
            |row| row.get(0),
        )
        .unwrap()
}

fn is_tombstone(store: &Store, kind: EntityKind, sync_id: &str) -> bool {
    store
        .connection
        .query_row(
            "SELECT deleted FROM sync_entity WHERE entity_type = ?1 AND sync_id = ?2",
            params![kind.prefix(), sync_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn nothing_is_tracked_while_sync_is_off() {
    let store = store(false);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    store.set_setting("appTheme", &"dark").unwrap();
    assert!(outbox(&store).is_empty());
    let entities: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM sync_entity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entities, 0);
}

#[test]
fn nothing_is_tracked_before_a_sync_identity_exists() {
    let store = Store::open_in_memory().unwrap();
    put(&store, "watchlist", &[watchlist(1, "A")]);
    assert!(outbox(&store).is_empty());
}

#[test]
fn a_new_row_queues_an_upload() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    assert_eq!(
        outbox(&store),
        [(
            "watchlist".into(),
            "1".into(),
            OutboxOp::Upsert,
            OutboxState::Pending
        )]
    );
    assert_eq!(revision(&store, EntityKind::Watchlist, "1"), 1);
}

#[test]
fn local_tables_are_never_tracked() {
    let store = store(true);
    put(&store, "tab", &[watchlist(1, "A")]);
    put(&store, "mcp_client", &[watchlist(2, "B")]);
    put(&store, "webhook_endpoint", &[watchlist(3, "C")]);
    assert!(outbox(&store).is_empty());
}

#[test]
fn only_the_row_that_changed_is_queued() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A"), watchlist(2, "B")]);
    let first = store.outbox().unwrap();
    let hashes: Vec<_> = first.iter().map(|row| row.sync_id.clone()).collect();
    for row in &first {
        let projected = project_row(
            "watchlist",
            &row.sync_id,
            &watchlist(
                row.sync_id.parse().unwrap(),
                if row.sync_id == "1" { "A" } else { "B" },
            )
            .0
            .to_string(),
        );
        mark_synced(&store, row.kind, &row.sync_id, &projected[0].hash);
    }
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(
        &store,
        "watchlist",
        &[watchlist(1, "A"), watchlist(2, "B2")],
    );
    let queued = outbox(&store);
    assert_eq!(hashes.len(), 2);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].1, "2");
}

#[test]
fn rewriting_identical_rows_queues_nothing() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "watchlist", &[watchlist(1, "A")]);
    assert!(outbox(&store).is_empty());
}

#[test]
fn repeated_edits_coalesce_into_one_entry_with_a_newer_seq() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    let first = store.outbox().unwrap()[0].seq;
    put(&store, "watchlist", &[watchlist(1, "B")]);
    put(&store, "watchlist", &[watchlist(1, "C")]);
    let rows = store.outbox().unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].seq > first);
    assert_eq!(revision(&store, EntityKind::Watchlist, "1"), 3);
}

#[test]
fn a_remote_apply_does_not_echo_back() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    // The apply layer records what the cloud holds, then writes the row.
    let remote = watchlist(1, "From other Mac");
    let hash = project_row("watchlist", "1", &remote.0.to_string())[0]
        .hash
        .clone();
    mark_synced(&store, EntityKind::Watchlist, "1", &hash);
    put(&store, "watchlist", &[remote]);
    assert!(outbox(&store).is_empty());
}

#[test]
fn undoing_an_edit_back_to_the_synced_state_queues_nothing() {
    let store = store(true);
    let original = watchlist(1, "A");
    put(&store, "watchlist", std::slice::from_ref(&original));
    let hash = project_row("watchlist", "1", &original.0.to_string())[0]
        .hash
        .clone();
    mark_synced(&store, EntityKind::Watchlist, "1", &hash);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "watchlist", &[watchlist(1, "B")]);
    assert_eq!(outbox(&store).len(), 1);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "watchlist", &[original]);
    assert!(outbox(&store).is_empty());
}

#[test]
fn a_failed_outbox_write_rolls_the_document_back() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER no_outbox BEFORE INSERT ON sync_outbox
             BEGIN SELECT RAISE(ABORT, 'outbox full'); END;",
        )
        .unwrap();
    let result = store.replace_documents("watchlist", &[watchlist(1, "B")]);
    assert!(result.is_err());
    let rows: Vec<Raw> = store.documents("watchlist");
    assert_eq!(rows[0].0["name"], "A");
}

#[test]
fn removing_a_row_queues_a_delete_and_leaves_a_tombstone() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A"), watchlist(2, "B")]);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "watchlist", &[watchlist(2, "B")]);
    assert_eq!(
        outbox(&store),
        [(
            "watchlist".into(),
            "1".into(),
            OutboxOp::Delete,
            OutboxState::Pending
        )]
    );
    assert!(is_tombstone(&store, EntityKind::Watchlist, "1"));
}

#[test]
fn a_deleted_row_that_returns_clears_its_tombstone() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    put(&store, "watchlist", &[]);
    assert!(is_tombstone(&store, EntityKind::Watchlist, "1"));
    put(&store, "watchlist", &[watchlist(1, "A")]);
    assert!(!is_tombstone(&store, EntityKind::Watchlist, "1"));
    assert_eq!(outbox(&store)[0].2, OutboxOp::Upsert);
}

#[test]
fn a_remote_delete_does_not_echo_back() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    store
        .connection
        .execute("UPDATE sync_entity SET deleted = 1, deleted_ms = 1", [])
        .unwrap();
    put(&store, "watchlist", &[]);
    assert!(outbox(&store).is_empty());
}

#[test]
fn deleting_most_of_a_collection_is_held_for_confirmation() {
    let store = store(true);
    let rows: Vec<_> = (1..=6).map(|id| watchlist(id, "x")).collect();
    put(&store, "watchlist", &rows);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "watchlist", &[]);
    let queued = outbox(&store);
    assert_eq!(queued.len(), 6);
    assert!(queued.iter().all(|row| row.3 == OutboxState::Held));
}

#[test]
fn deleting_a_few_rows_is_not_held() {
    let store = store(true);
    let rows: Vec<_> = (1..=10).map(|id| watchlist(id, "x")).collect();
    put(&store, "watchlist", &rows);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "watchlist", &rows[4..]);
    let queued = outbox(&store);
    assert_eq!(queued.len(), 4);
    assert!(queued.iter().all(|row| row.3 == OutboxState::Pending));
}

#[test]
fn deleting_a_small_collection_entirely_is_not_held() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "x"), watchlist(2, "y")]);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "watchlist", &[]);
    assert!(
        outbox(&store)
            .iter()
            .all(|row| row.3 == OutboxState::Pending)
    );
}

fn alert(state: &str, armed: bool, previous: f64, updated: i64) -> Raw {
    Raw(json!({
        "id": 5, "asset": {"symbol": "BTC"}, "condition": {"Above": 100.0},
        "currency": "USD", "frequency": "Once", "state": state, "note": "",
        "armed": armed, "previous_value": previous, "last_fingerprint": "f",
        "created_at_ms": 1, "updated_at_ms": updated,
        "webhook_endpoint_ids": [], "webhook_message": null
    }))
}

#[test]
fn the_alert_engine_heartbeat_queues_nothing() {
    let store = store(true);
    put(&store, "price_alert", &[alert("Active", true, 90.0, 1)]);
    assert_eq!(outbox(&store).len(), 1);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    for tick in 0..20 {
        put(
            &store,
            "price_alert",
            &[alert(
                "Active",
                tick % 2 == 0,
                90.0 + tick as f64,
                1000 + tick,
            )],
        );
    }
    put(
        &store,
        "price_alert",
        &[alert("Triggered", false, 120.0, 5000)],
    );
    assert!(outbox(&store).is_empty());
    put(&store, "price_alert", &[alert("Paused", true, 120.0, 6000)]);
    assert_eq!(outbox(&store).len(), 1);
}

fn layout(drawings: Vec<Value>, opened: i64) -> Raw {
    Raw(json!({
        "id": 10, "name": "Main", "last_opened_at_ms": opened,
        "charts": [{"id": 2, "symbol": "BTC", "drawings": drawings}]
    }))
}

fn trend(id: u64, price: f64) -> Value {
    json!({"TrendLine": {"id": id, "a": {"time_ms": 1, "price": price}}})
}

#[test]
fn a_drawing_edit_queues_only_that_drawing() {
    let store = store(true);
    put(
        &store,
        "saved_view",
        &[layout(vec![trend(1, 1.0), trend(2, 2.0)], 1)],
    );
    let kinds: Vec<_> = outbox(&store).into_iter().map(|row| row.1).collect();
    assert_eq!(kinds, ["10", "10.2.1", "10.2.2"]);
    for row in store.outbox().unwrap() {
        store
            .connection
            .execute(
                "UPDATE sync_entity SET synced_hash = 'x' WHERE sync_id = ?1",
                [&row.sync_id],
            )
            .unwrap();
    }
    // Re-sync the hashes exactly as an acknowledgement would.
    let stored: Vec<(String, String)> = {
        let mut statement = store
            .connection
            .prepare("SELECT id, payload FROM saved_view")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for (id, payload) in stored {
        for entity in project_row("saved_view", &id, &payload) {
            mark_synced(&store, entity.kind, &entity.sync_id, &entity.hash);
        }
    }
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(
        &store,
        "saved_view",
        &[layout(vec![trend(1, 1.0), trend(2, 9.0)], 50)],
    );
    let queued = outbox(&store);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].1, "10.2.2");
}

#[test]
fn removing_a_drawing_queues_its_delete() {
    let store = store(true);
    put(
        &store,
        "saved_view",
        &[layout(vec![trend(1, 1.0), trend(2, 2.0)], 1)],
    );
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "saved_view", &[layout(vec![trend(1, 1.0)], 1)]);
    let queued = outbox(&store);
    assert!(
        queued
            .iter()
            .any(|row| row.0 == "layoutdrawing" && row.1 == "10.2.2" && row.2 == OutboxOp::Delete)
    );
}

#[test]
fn deleting_a_big_layout_holds_its_drawings_with_it() {
    let store = store(true);
    let layouts: Vec<_> = (1..=6)
        .map(|id| {
            Raw(json!({"id": id, "name": "x",
                "charts": [{"id": 1, "drawings": [trend(1, 1.0)]}]}))
        })
        .collect();
    put(&store, "saved_view", &layouts);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "saved_view", &[]);
    let queued = outbox(&store);
    assert_eq!(queued.len(), 12);
    assert!(queued.iter().all(|row| row.3 == OutboxState::Held));
}

#[test]
fn deleting_one_layout_does_not_hold_its_many_drawings() {
    let store = store(true);
    let drawings: Vec<_> = (1..=8).map(|id| trend(id, 1.0)).collect();
    let layouts = [
        Raw(json!({"id": 1, "name": "a", "charts": [{"id": 1, "drawings": drawings}]})),
        Raw(json!({"id": 2, "name": "b", "charts": []})),
        Raw(json!({"id": 3, "name": "c", "charts": []})),
    ];
    put(&store, "saved_view", &layouts);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    put(&store, "saved_view", &layouts[1..]);
    let queued = outbox(&store);
    assert_eq!(queued.len(), 9);
    assert!(queued.iter().all(|row| row.3 == OutboxState::Pending));
}

#[test]
fn portable_settings_queue_and_local_settings_do_not() {
    let store = store(true);
    store.set_setting("appTheme", &"dark").unwrap();
    store
        .set_setting("alert.health", &json!({"heartbeat_ms": 1}))
        .unwrap();
    store
        .set_setting("mcp.settings", &json!({"port": 1}))
        .unwrap();
    assert_eq!(
        outbox(&store),
        [(
            "setting".into(),
            "appTheme".into(),
            OutboxOp::Upsert,
            OutboxState::Pending
        )]
    );
}

#[test]
fn clearing_a_portable_setting_queues_a_delete() {
    let store = store(true);
    store.set_setting("appTheme", &"dark").unwrap();
    store.remove_setting("appTheme").unwrap();
    assert_eq!(outbox(&store)[0].2, OutboxOp::Delete);
    assert!(store.setting::<String>("appTheme").is_none());
}

#[test]
fn removing_a_setting_that_never_existed_queues_nothing() {
    let store = store(true);
    store.remove_setting("appTheme").unwrap();
    assert!(outbox(&store).is_empty());
}

#[test]
fn a_failed_setting_outbox_write_rolls_the_setting_back() {
    let store = store(true);
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER no_outbox BEFORE INSERT ON sync_outbox
             BEGIN SELECT RAISE(ABORT, 'outbox full'); END;",
        )
        .unwrap();
    assert!(store.set_setting("appTheme", &"dark").is_err());
    assert!(store.setting::<String>("appTheme").is_none());
}

#[test]
fn consenting_to_a_snapshot_host_does_not_sync() {
    let store = store(true);
    let prefs = |consented: Vec<&str>| json!({"provider": "self_hosted", "providers": {"self_hosted": {"host": "https://h.example/up"}}, "include_trading": false, "scale": "X2", "consented": consented});
    store.set_setting("snapshot.prefs", &prefs(vec![])).unwrap();
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    store
        .set_setting("snapshot.prefs", &prefs(vec!["h.example"]))
        .unwrap();
    assert!(outbox(&store).is_empty());
}

#[test]
fn reconcile_finds_data_that_predates_sync_and_is_then_quiet() {
    let store = store(false);
    put(&store, "watchlist", &[watchlist(1, "A"), watchlist(2, "B")]);
    store.set_setting("appTheme", &"dark").unwrap();
    assert!(outbox(&store).is_empty());
    let mut state = store.sync_state().unwrap().unwrap();
    state.enabled = true;
    store.save_sync_state(&state).unwrap();

    let first = store.reconcile_stored().unwrap();
    assert_eq!(
        first,
        Reconciled {
            upserts: 3,
            deletes: 0
        }
    );
    let seqs: Vec<_> = store.outbox().unwrap().iter().map(|r| r.seq).collect();
    let second = store.reconcile_stored().unwrap();
    assert_eq!(second, Reconciled::default());
    let again: Vec<_> = store.outbox().unwrap().iter().map(|r| r.seq).collect();
    assert_eq!(seqs, again);
}

#[test]
fn reconcile_skips_entities_that_match_the_cloud() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    let hash = project_row("watchlist", "1", &watchlist(1, "A").0.to_string())[0]
        .hash
        .clone();
    mark_synced(&store, EntityKind::Watchlist, "1", &hash);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    assert_eq!(store.reconcile_stored().unwrap(), Reconciled::default());
}

#[test]
fn reconcile_catches_an_edit_an_older_build_made() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A")]);
    let hash = project_row("watchlist", "1", &watchlist(1, "A").0.to_string())[0]
        .hash
        .clone();
    mark_synced(&store, EntityKind::Watchlist, "1", &hash);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    // An older build writes without the hook.
    store
        .connection
        .execute(
            "UPDATE watchlist SET payload = ?1 WHERE id = '1'",
            [watchlist(1, "Edited").0.to_string()],
        )
        .unwrap();
    assert_eq!(store.reconcile_stored().unwrap().upserts, 1);
}

#[test]
fn reconcile_queues_deletes_for_rows_an_older_build_removed() {
    let store = store(true);
    put(&store, "watchlist", &[watchlist(1, "A"), watchlist(2, "B")]);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    store
        .connection
        .execute("DELETE FROM watchlist WHERE id = '1'", [])
        .unwrap();
    let done = store.reconcile_stored().unwrap();
    assert_eq!(done.deletes, 1);
    assert!(is_tombstone(&store, EntityKind::Watchlist, "1"));
    let deletes: Vec<_> = store
        .outbox()
        .unwrap()
        .into_iter()
        .filter(|r| r.op == OutboxOp::Delete)
        .collect();
    assert_eq!(deletes.len(), 1);
}

#[test]
fn reconcile_holds_a_mass_delete() {
    let store = store(true);
    let rows: Vec<_> = (1..=6).map(|id| watchlist(id, "x")).collect();
    put(&store, "watchlist", &rows);
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    store
        .connection
        .execute("DELETE FROM watchlist", [])
        .unwrap();
    store.reconcile_stored().unwrap();
    let held = store
        .outbox()
        .unwrap()
        .iter()
        .filter(|r| r.state == OutboxState::Held)
        .count();
    assert_eq!(held, 6);
}

#[test]
fn reconcile_never_touches_local_only_data() {
    let store = store(true);
    put(&store, "tab", &[watchlist(1, "A")]);
    store.set_setting("alert.health", &json!({})).unwrap();
    assert_eq!(store.reconcile_stored().unwrap(), Reconciled::default());
}

fn script(id: &str, source: &str) -> Projected {
    crate::sync::record::project::project_script(&crate::sync::record::project::ScriptSnapshot {
        id: id.into(),
        name: format!("Script {id}"),
        source: source.into(),
        is_favorite: false,
    })
}

#[test]
fn script_files_reconcile_like_rows() {
    let store = store(true);
    let current: HashMap<_, _> = [
        ("a".to_string(), script("a", "x")),
        ("b".to_string(), script("b", "y")),
    ]
    .into();
    assert_eq!(
        store
            .reconcile_kind(EntityKind::Script, &current)
            .unwrap()
            .upserts,
        2
    );
    assert_eq!(
        store.reconcile_kind(EntityKind::Script, &current).unwrap(),
        Reconciled::default()
    );

    for entity in current.values() {
        mark_synced(&store, EntityKind::Script, &entity.sync_id, &entity.hash);
    }
    store
        .connection
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    let edited: HashMap<_, _> = [
        ("a".to_string(), script("a", "x2")),
        ("b".to_string(), script("b", "y")),
    ]
    .into();
    assert_eq!(
        store.reconcile_kind(EntityKind::Script, &edited).unwrap(),
        Reconciled {
            upserts: 1,
            deletes: 0
        }
    );

    let only_b: HashMap<_, _> = [("b".to_string(), script("b", "y"))].into();
    let done = store.reconcile_kind(EntityKind::Script, &only_b).unwrap();
    assert_eq!(done.deletes, 1);
    assert!(is_tombstone(&store, EntityKind::Script, "a"));
}

#[test]
fn a_script_that_returns_after_deletion_is_uploaded_again() {
    let store = store(true);
    let one: HashMap<_, _> = [("a".to_string(), script("a", "x"))].into();
    store.reconcile_kind(EntityKind::Script, &one).unwrap();
    store
        .reconcile_kind(EntityKind::Script, &HashMap::new())
        .unwrap();
    assert!(is_tombstone(&store, EntityKind::Script, "a"));
    store.reconcile_kind(EntityKind::Script, &one).unwrap();
    assert!(!is_tombstone(&store, EntityKind::Script, "a"));
    let rows = store.outbox().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].op, OutboxOp::Upsert);
}

#[test]
fn writes_inside_an_outer_transaction_are_tracked_and_roll_back_together() {
    let store = store(true);
    let result: rusqlite::Result<()> = store.in_transaction(|| {
        store.replace_documents("watchlist", &[watchlist(1, "A")])?;
        Err(rusqlite::Error::QueryReturnedNoRows)
    });
    assert!(result.is_err());
    assert!(outbox(&store).is_empty());
    assert!(store.documents::<Raw>("watchlist").is_empty());
}
