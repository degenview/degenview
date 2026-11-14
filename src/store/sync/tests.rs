use super::*;
use rusqlite::Connection;

fn table_exists(store: &Store, name: &str) -> bool {
    store
        .connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .is_ok()
}

#[test]
fn schema_creates_every_sync_table() {
    let store = Store::open_in_memory().unwrap();
    for table in [
        "sync_state",
        "sync_entity",
        "sync_outbox",
        "sync_inbox",
        "sync_conflict",
        "sync_device",
        "price_alert_local",
    ] {
        assert!(table_exists(&store, table), "{table}");
    }
}

#[test]
fn dismissing_conflicts_by_kind_or_all() {
    let store = Store::open_in_memory().unwrap();
    store.ensure_sync_state("Mac", "hw").unwrap();
    for (id, kind) in [("1", "paper"), ("2", "paper"), ("3", "layout")] {
        store
            .record_conflict(
                &ConflictRecord {
                    entity: EntityKind::PaperAccount,
                    sync_id: id.into(),
                    kind: kind.into(),
                    local_payload: None,
                    remote_payload: None,
                    remote_device: None,
                    copy_entity: None,
                    copy_sync_id: None,
                },
                10,
            )
            .unwrap();
    }
    store.dismiss_conflicts(Some("paper"), 20).unwrap();
    let open = store.open_conflicts().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, "layout");
    store.dismiss_conflicts(None, 30).unwrap();
    assert!(store.open_conflicts().unwrap().is_empty());
}

#[test]
fn schema_is_idempotent() {
    let store = Store::open_in_memory().unwrap();
    store.ensure_sync_state("Mac", "hw").unwrap();
    store.create_schema().unwrap();
    store.create_schema().unwrap();
    assert!(store.sync_state().unwrap().is_some());
}

#[test]
fn a_database_from_before_sync_opens_and_keeps_its_data() {
    let path = std::env::temp_dir().join(format!("dv-sync-old-{}.sqlite", new_device_id()));
    {
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            "CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO setting VALUES ('appTheme', '\"dark\"');",
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert!(table_exists(&store, "sync_outbox"));
    assert_eq!(store.setting::<String>("appTheme").as_deref(), Some("dark"));
    assert!(!store.sync_enabled());
    drop(store);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sync_is_off_until_enabled() {
    let store = Store::open_in_memory().unwrap();
    assert!(!store.sync_enabled());
    assert!(store.sync_state().unwrap().is_none());
    let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
    assert!(!state.enabled);
    state.enabled = true;
    store.save_sync_state(&state).unwrap();
    assert!(store.sync_enabled());
}

#[test]
fn device_identity_is_stable_and_never_overwritten() {
    let store = Store::open_in_memory().unwrap();
    let first = store.ensure_sync_state("Mac A", "hw-a").unwrap();
    let again = store.ensure_sync_state("Mac B", "hw-b").unwrap();
    assert_eq!(first.device_id, again.device_id);
    assert_eq!(again.device_name, "Mac A");
    assert_eq!(first.device_id.len(), 32);
    assert_eq!(first.zone_name, "DegenViewZone");
}

#[test]
fn id_tag_is_assigned_once_and_kept() {
    let store = Store::open_in_memory().unwrap();
    let first = store.ensure_sync_state("Mac A", "hw-a").unwrap();
    assert!(first.id_tag > 0 && first.id_tag < 1 << 31);
    assert_eq!(
        store.ensure_sync_state("Mac B", "hw-b").unwrap().id_tag,
        first.id_tag
    );
    let mut saved = store.sync_state().unwrap().unwrap();
    saved.last_error = Some("x".into());
    store.save_sync_state(&saved).unwrap();
    assert_eq!(store.sync_state().unwrap().unwrap().id_tag, first.id_tag);
}

#[test]
fn a_database_that_enabled_sync_before_the_tag_existed_gets_one_when_it_opens() {
    let store = Store::open_in_memory().unwrap();
    store.ensure_sync_state("Mac", "hw").unwrap();
    let zero_tagged = crate::model::ids::GLOBAL_BASE | 1;
    let other_macs = crate::model::ids::GLOBAL_BASE | (7 << 24) | 1;
    for (kind, id) in [
        ("paperaccount", zero_tagged),
        ("paperaccount", other_macs),
        ("watchlist", 1),
    ] {
        store
            .connection
            .execute(
                "INSERT INTO sync_entity (entity_type, sync_id, local_id) VALUES (?1, ?2, '1')",
                params![kind, id.to_string()],
            )
            .unwrap();
    }
    store
        .connection
        .execute("UPDATE sync_state SET id_tag = 0", [])
        .unwrap();
    store.create_sync_schema().unwrap();
    assert!(store.sync_state().unwrap().unwrap().id_tag > 0);
    let left: Vec<(String, String)> = store
        .connection
        .prepare("SELECT entity_type, sync_id FROM sync_entity ORDER BY sync_id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        left,
        vec![
            ("watchlist".to_string(), "1".to_string()),
            ("paperaccount".to_string(), other_macs.to_string()),
        ],
        "only what was tracked under the zero tag is forgotten"
    );
    let tag = store.sync_state().unwrap().unwrap().id_tag;
    store.create_sync_schema().unwrap();
    assert_eq!(
        store.sync_state().unwrap().unwrap().id_tag,
        tag,
        "assigned once"
    );
}

#[test]
fn database_without_the_id_tag_column_is_upgraded() {
    let store = Store::open_in_memory().unwrap();
    store
        .connection
        .execute_batch("ALTER TABLE sync_state DROP COLUMN id_tag")
        .unwrap();
    store.create_sync_schema().unwrap();
    let state = store.ensure_sync_state("Mac", "hw").unwrap();
    assert!(state.id_tag > 0);
}

#[test]
fn state_round_trips() {
    let store = Store::open_in_memory().unwrap();
    let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
    state.enabled = true;
    state.account_hash = Some("acct".into());
    state.engine_state = Some(vec![1, 2, 3]);
    state.engine_state_account_hash = Some("acct".into());
    state.last_error = Some("offline".into());
    state.last_fetch_ms = Some(42);
    store.save_sync_state(&state).unwrap();
    assert_eq!(store.sync_state().unwrap().unwrap(), state);
}

#[test]
fn saving_without_a_row_is_an_error() {
    let store = Store::open_in_memory().unwrap();
    let mut state = store.ensure_sync_state("Mac", "hw").unwrap();
    store
        .connection
        .execute("DELETE FROM sync_state", [])
        .unwrap();
    state.enabled = true;
    assert!(store.save_sync_state(&state).is_err());
}

#[test]
fn a_cloned_database_gets_a_new_identity_and_loses_its_checkpoint() {
    let store = Store::open_in_memory().unwrap();
    let mut state = store.ensure_sync_state("Mac", "hw-a").unwrap();
    state.engine_state = Some(vec![9]);
    state.initial_sync_done = true;
    store.save_sync_state(&state).unwrap();
    store.reset_sync_device("hw-b").unwrap();
    let reset = store.sync_state().unwrap().unwrap();
    assert_ne!(reset.device_id, state.device_id);
    assert_eq!(reset.hardware_hash, "hw-b");
    assert!(reset.engine_state.is_none());
    assert!(!reset.initial_sync_done);
}

#[test]
fn outbox_rejects_an_unknown_operation() {
    let store = Store::open_in_memory().unwrap();
    let bad = store.connection.execute(
        "INSERT INTO sync_outbox (entity_type, sync_id, op, queued_ms) VALUES ('layout', '1', 'poke', 0)",
        [],
    );
    assert!(bad.is_err());
}
