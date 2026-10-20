//! Script sync: two Macs with their own script libraries, plus the real `ScriptStore`.

use super::core_tests::{Mac, mac};
use super::mock::MockCloud;
use super::scripts::{ScriptSink, StoreScriptSink};
use crate::script_manager::store::ScriptStore;
use std::collections::HashSet;

fn pair() -> (MockCloud, Mac, Mac) {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "Mac A", "acct", Some("acct"));
    let mut b = mac(&cloud, "Mac B", "acct", Some("acct"));
    a.apply = true;
    b.apply = true;
    a.core.start();
    b.core.start();
    (cloud, a, b)
}

fn settle(a: &mut Mac, b: &mut Mac) {
    for _ in 0..6 {
        a.scan_scripts();
        a.core.request_sync();
        a.run();
        b.scan_scripts();
        b.core.request_sync();
        b.run();
    }
}

fn outbox(mac: &Mac) -> usize {
    mac.outbox().len()
}

const V1: &str = "//@version=6\nindicator(\"V1\")\nplot(close)\n";
const V2: &str = "//@version=6\nindicator(\"V2\")\nplot(open)\n";
const V3: &str = "//@version=6\nindicator(\"V3\")\nplot(high)\n";

#[test]
fn a_script_made_on_one_mac_appears_on_the_other_without_echoing_back() {
    let (cloud, mut a, mut b) = pair();
    let id = a.library.add("My Strategy", V1);
    settle(&mut a, &mut b);
    assert_eq!(cloud.names(), [format!("script.{id}")]);
    assert_eq!(
        b.library.sorted(),
        [("My Strategy".to_string(), V1.to_string())]
    );
    assert_eq!(outbox(&a) + outbox(&b), 0);
    // The same script, same id, on both.
    assert!(b.library.scripts.contains_key(&id));
}

#[test]
fn edits_renames_and_favorites_travel() {
    let (_cloud, mut a, mut b) = pair();
    let id = a.library.add("Original", V1);
    settle(&mut a, &mut b);
    a.library.set_source(&id, V2);
    a.library.scripts.get_mut(&id).unwrap().name = "Renamed".into();
    a.library.scripts.get_mut(&id).unwrap().is_favorite = true;
    settle(&mut a, &mut b);
    let on_b = &b.library.scripts[&id];
    assert_eq!(on_b.name, "Renamed");
    assert_eq!(on_b.source, V2);
    assert!(on_b.is_favorite);
    assert_eq!(outbox(&a) + outbox(&b), 0);
}

#[test]
fn two_edits_to_the_same_script_never_lose_either_version() {
    let (_cloud, mut a, mut b) = pair();
    let id = a.library.add("My Strategy", V1);
    settle(&mut a, &mut b);
    a.library.set_source(&id, V2);
    b.library.set_source(&id, V3);
    settle(&mut a, &mut b);

    let (on_a, on_b) = (a.library.sorted(), b.library.sorted());
    assert_eq!(on_a, on_b, "both Macs end with the same scripts");
    assert_eq!(on_a.len(), 2, "the script and one conflict copy");
    let sources: HashSet<&str> = on_a.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(sources, [V2, V3].into(), "neither edit was lost");
    assert!(
        on_a.iter().any(|(name, _)| name.contains("conflict from")),
        "{on_a:?}"
    );
    assert!(
        a.store.lock().unwrap().conflicts().unwrap().len()
            + b.store.lock().unwrap().conflicts().unwrap().len()
            >= 1
    );
    assert_eq!(outbox(&a) + outbox(&b), 0);
}

#[test]
fn a_rename_on_one_mac_and_an_edit_on_the_other_is_not_a_conflict_copy_when_sources_match() {
    let (_cloud, mut a, mut b) = pair();
    let id = a.library.add("Old", V1);
    settle(&mut a, &mut b);
    a.library.scripts.get_mut(&id).unwrap().name = "New".into();
    b.library.scripts.get_mut(&id).unwrap().is_favorite = true;
    settle(&mut a, &mut b);
    assert_eq!(a.library.scripts.len(), 1);
    assert_eq!(b.library.scripts.len(), 1);
    assert_eq!(a.library.sorted(), b.library.sorted());
}

#[test]
fn text_being_typed_is_never_overwritten_and_the_update_applies_once_it_is_saved() {
    let (_cloud, mut a, mut b) = pair();
    let id = a.library.add("Typing", V1);
    settle(&mut a, &mut b);
    b.library.unsaved.insert(id.clone());
    a.library.set_source(&id, V2);
    settle(&mut a, &mut b);
    assert_eq!(
        b.library.source(&id),
        V1,
        "the open editor's script is left alone"
    );
    let waiting = b.store.lock().unwrap().inbox().unwrap();
    assert!(
        waiting
            .iter()
            .any(|r| r.record_name == format!("script.{id}") && r.reason == "busy")
    );

    // The user saves and closes; the next pass applies what was waiting.
    b.library.unsaved.clear();
    let empty = super::core::RemoteBatch {
        id: 900,
        records: vec![],
        deletions: vec![],
    };
    b.apply_to_store(&empty);
    assert_eq!(b.library.source(&id), V2);
    assert!(
        b.store
            .lock()
            .unwrap()
            .inbox()
            .unwrap()
            .iter()
            .all(|r| r.reason != "busy")
    );
}

#[test]
fn a_deletion_propagates_to_a_mac_that_did_not_change_the_script() {
    let (cloud, mut a, mut b) = pair();
    let id = a.library.add("Doomed", V1);
    let keep = a.library.add("Kept", V2);
    settle(&mut a, &mut b);
    a.library.scripts.remove(&id);
    settle(&mut a, &mut b);
    assert!(!b.library.scripts.contains_key(&id));
    assert!(b.library.scripts.contains_key(&keep));
    assert_eq!(cloud.names(), [format!("script.{keep}")]);
    assert_eq!(outbox(&a) + outbox(&b), 0);
}

#[test]
fn a_script_edited_elsewhere_beats_its_deletion() {
    let (cloud, mut a, mut b) = pair();
    let id = a.library.add("Contested", V1);
    settle(&mut a, &mut b);
    a.library.scripts.remove(&id);
    a.scan_scripts();
    a.core.request_sync();
    a.run();
    b.library.set_source(&id, V2);
    settle(&mut a, &mut b);
    assert_eq!(a.library.source(&id), V2, "it came back on A");
    assert!(cloud.names().contains(&format!("script.{id}")));
}

#[test]
fn unmodified_bundled_examples_are_never_uploaded() {
    let (cloud, mut a, mut b) = pair();
    let demo = a.library.add("EMA Cross", V1);
    a.library.pristine_demos.insert(demo);
    let own = a.library.add("Mine", V2);
    settle(&mut a, &mut b);
    assert_eq!(cloud.names(), [format!("script.{own}")]);
}

#[test]
fn an_untouched_example_gives_way_to_an_incoming_script_of_the_same_name() {
    let (_cloud, mut a, mut b) = pair();
    let mine = a.library.add("EMA Cross", V2);
    let demo = b.library.add("EMA Cross", V1);
    b.library.pristine_demos.insert(demo.clone());
    settle(&mut a, &mut b);
    assert!(
        !b.library.scripts.contains_key(&demo),
        "the untouched example was replaced"
    );
    assert_eq!(b.library.source(&mine), V2);
    assert_eq!(b.library.scripts.len(), 1);
    assert_eq!(
        b.library.scripts[&mine].name, "EMA Cross",
        "no pointless suffix"
    );
}

#[test]
fn a_name_already_taken_here_is_suffixed_and_the_suffix_is_not_uploaded() {
    let (cloud, mut a, mut b) = pair();
    let theirs = a.library.add("Same Name", V1);
    let mine = b.library.add("Same Name", V2);
    settle(&mut a, &mut b);
    assert_eq!(b.library.scripts[&mine].name, "Same Name");
    assert_eq!(b.library.scripts[&theirs].name, "Same Name 2");
    assert_eq!(
        a.library.scripts[&theirs].name, "Same Name",
        "A's script was not renamed"
    );
    assert_eq!(cloud.names().len(), 2);
    let on_a = a.library.sorted();
    assert!(on_a.iter().any(|(n, _)| n == "Same Name"));
    assert_eq!(outbox(&a) + outbox(&b), 0);
}

#[test]
fn an_unreadable_library_never_reads_as_everything_deleted() {
    let (cloud, mut a, mut b) = pair();
    for n in 0..8 {
        a.library.add(&format!("Script {n}"), V1);
    }
    settle(&mut a, &mut b);
    assert_eq!(cloud.names().len(), 8);
    a.library.fail_listing = true;
    settle(&mut a, &mut b);
    assert_eq!(cloud.names().len(), 8);
    assert_eq!(outbox(&a), 0);
    assert_eq!(b.library.scripts.len(), 8);
}

#[test]
fn applying_a_script_does_not_make_it_look_edited() {
    let (cloud, mut a, mut b) = pair();
    let id = a.library.add("Quiet", V1);
    settle(&mut a, &mut b);
    let saves = cloud.save_attempts();
    for _ in 0..3 {
        a.library.set_source(&id, V1);
        settle(&mut a, &mut b);
    }
    assert_eq!(cloud.save_attempts(), saves);
}

#[test]
fn a_script_with_an_illegal_file_name_is_kept_not_applied() {
    let (cloud, mut a, mut b) = pair();
    let id = "0123456789abcdef0123456789abcdef";
    let bad =
        serde_json::json!({"id": id, "name": "../escape", "source": V1, "is_favorite": false});
    cloud.put_from_elsewhere(
        super::core_tests::raw(&format!("script.{id}"), &bad, 1, 1, "Z"),
        "Z",
    );
    settle(&mut a, &mut b);
    assert!(b.library.scripts.is_empty());
    let kept = b.store.lock().unwrap().inbox().unwrap();
    assert!(kept.iter().any(|r| r.reason == "invalid"));
}

// MARK: The real script store

fn temp_store(tag: &str) -> (ScriptStore, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "degenview-sync-scripts-{tag}-{}-{}",
        std::process::id(),
        ScriptStore::fresh_id()
    ));
    (
        ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata")),
        root,
    )
}

#[test]
fn the_real_store_adopts_a_script_with_the_given_id_and_compiles_it() {
    let (mut store, root) = temp_store("adopt");
    let id = ScriptStore::fresh_id();
    let mut sink = StoreScriptSink {
        store: &mut store,
        unsaved: HashSet::new(),
    };
    sink.adopt(&id, "From Elsewhere", V1, true).unwrap();
    let got = sink.get(&id).unwrap().unwrap();
    assert_eq!(got.name, "From Elsewhere");
    assert_eq!(got.source, V1);
    assert!(got.is_favorite);
    let saved = store.script(&id).unwrap().unwrap();
    assert!(
        saved.compile_record.is_some(),
        "saved through the normal path"
    );
    assert!(saved.latest_revision_id.is_some());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_real_store_updates_renames_and_disambiguates() {
    let (mut store, root) = temp_store("update");
    let first = ScriptStore::fresh_id();
    let second = ScriptStore::fresh_id();
    {
        let mut sink = StoreScriptSink {
            store: &mut store,
            unsaved: HashSet::new(),
        };
        sink.adopt(&first, "One", V1, false).unwrap();
        sink.adopt(&second, "One", V2, false).unwrap();
        assert_eq!(sink.get(&second).unwrap().unwrap().name, "One 2");
        sink.update(&first, "Renamed", V3, true).unwrap();
        let got = sink.get(&first).unwrap().unwrap();
        assert_eq!(
            (got.name.as_str(), got.source.as_str(), got.is_favorite),
            ("Renamed", V3, true)
        );
        // Renaming onto another script's name suffixes rather than failing.
        sink.update(&first, "One 2", V3, true).unwrap();
        assert_eq!(sink.get(&first).unwrap().unwrap().name, "One 2 2");
        sink.delete(&second).unwrap();
        assert!(sink.get(&second).unwrap().is_none());
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_real_sink_hides_unmodified_examples_and_knows_about_drafts() {
    let (mut store, root) = temp_store("demos");
    crate::script_manager::demo::add_missing(&mut store).unwrap();
    let own = ScriptStore::fresh_id();
    let mut sink = StoreScriptSink {
        store: &mut store,
        unsaved: HashSet::new(),
    };
    sink.adopt(&own, "Mine", V1, false).unwrap();
    let synced: Vec<_> = sink.all().unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(synced, ["Mine"], "the nine examples are not uploaded");
    let found = sink.find_by_name("ema cross").unwrap().unwrap();
    assert!(found.pristine_demo);
    assert!(!sink.has_unsaved_work(&own));
    sink.unsaved.insert(own.clone());
    assert!(sink.has_unsaved_work(&own));
    let _ = std::fs::remove_dir_all(root);
}
