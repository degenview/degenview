use super::*;

#[test]
fn source_hash_is_sha256() {
    assert_eq!(
        hash_source("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

fn temp_store(tag: &str) -> (ScriptStore, PathBuf) {
    let root = std::env::temp_dir().join(format!("degenview-store-{tag}-{}", new_id()));
    let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
    (store, root)
}

#[test]
fn create_uses_template_and_disambiguates() {
    let (mut store, root) = temp_store("create");
    let first = store
        .create("Alpha", ScriptType::Indicator, None, true)
        .unwrap();
    assert_eq!(
        first.source,
        "//@version=6\nindicator(\"Alpha\", overlay=true)\nplot(close)\n"
    );
    let second = store
        .create("alpha", ScriptType::Strategy, None, true)
        .unwrap();
    assert_eq!(second.name, "alpha 2");
    assert!(matches!(
        store.create("ALPHA", ScriptType::Indicator, None, false),
        Err(ScriptStoreError::NameConflict)
    ));
    assert_eq!(store.all_scripts().unwrap().len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn save_updates_type_from_declaration_and_records_revisions() {
    let (mut store, root) = temp_store("save");
    let script = store
        .create("S", ScriptType::Indicator, None, true)
        .unwrap();
    let saved = store
        .save(&script.id, "//@version=6\nstrategy(\"S\")\n")
        .unwrap();
    assert_eq!(saved.script_type, ScriptType::Strategy);
    assert_eq!(
        saved.compile_record.as_ref().unwrap().compiler_version,
        COMPILER_VERSION
    );
    // Mid-edit source with no declaration keeps the old type.
    let kept = store.save(&script.id, "plot(close)\n").unwrap();
    assert_eq!(kept.script_type, ScriptType::Strategy);
    assert_eq!(store.revisions(&script.id).unwrap().len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn save_ends_the_file_in_a_newline() {
    let (mut store, root) = temp_store("newline");
    let script = store
        .create("N", ScriptType::Indicator, None, true)
        .unwrap();
    let saved = store
        .save(&script.id, "//@version=6\nindicator(\"N\")\nplot(close)")
        .unwrap();
    assert!(saved.source.ends_with("plot(close)\n"));
    let path = store.file_path(&script.id).unwrap().unwrap();
    assert!(fs::read_to_string(path).unwrap().ends_with("plot(close)\n"));
    // Already terminated, extra blank lines and empty source are left alone.
    assert_eq!(with_trailing_newline("a\n"), "a\n");
    assert_eq!(with_trailing_newline("a\n\n"), "a\n\n");
    assert_eq!(with_trailing_newline(""), "");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rename_duplicate_favorite_and_draft_round_trip() {
    let (mut store, root) = temp_store("crud");
    let script = store
        .create("One", ScriptType::Indicator, None, true)
        .unwrap();
    let renamed = store.rename(&script.id, "Two").unwrap();
    assert_eq!(renamed.name, "Two");
    assert_eq!(renamed.id, script.id);
    assert!(root.join("Scripts/Two.pine").exists());
    assert!(!root.join("Scripts/One.pine").exists());
    // Case-only rename.
    assert_eq!(store.rename(&script.id, "TWO").unwrap().name, "TWO");

    let copy = store.duplicate(&script.id).unwrap();
    assert_eq!(copy.name, "TWO copy");
    assert!(!copy.is_favorite);
    assert_ne!(copy.id, script.id);

    store.set_favorite(&script.id, true).unwrap();
    let draft = ScriptDraft {
        script_id: script.id.clone(),
        source: "draft".into(),
        modified_at_ms: 5,
        based_on_revision_id: None,
    };
    store.save_draft(&draft).unwrap();
    assert_eq!(store.draft(&script.id).unwrap(), Some(draft));

    // A fresh store over the same folders sees the same ids and the favorite.
    let mut again = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
    let all = again.all_scripts().unwrap();
    let loaded = all.iter().find(|s| s.id == script.id).expect("same id");
    assert!(loaded.is_favorite);
    assert_eq!(loaded.name, "TWO");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_files_appear_and_stale_compile_records_drop() {
    let (mut store, root) = temp_store("external");
    let script = store
        .create("Ext", ScriptType::Indicator, None, true)
        .unwrap();
    store
        .save(
            &script.id,
            "//@version=6\nindicator(\"Ext\")\nplot(close)\n",
        )
        .unwrap();
    fs::write(
        root.join("Scripts/Ext.pine"),
        "//@version=6\nindicator(\"Ext\")\nplot(open)\n",
    )
    .unwrap();
    fs::write(
        root.join("Scripts/Outside.pine"),
        "//@version=6\nlibrary(\"Outside\")\n",
    )
    .unwrap();
    let all = store.all_scripts().unwrap();
    assert_eq!(all.len(), 2);
    assert!(
        all.iter()
            .find(|s| s.id == script.id)
            .unwrap()
            .compile_record
            .is_none()
    );
    let outside = all.iter().find(|s| s.name == "Outside").unwrap();
    assert_eq!(outside.script_type, ScriptType::Library);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_folder_imports_whole_and_leaves_the_originals() {
    let (mut store, root) = temp_store("folder");
    let source = root.join("swift");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("One.pine"),
        "//@version=6\nindicator(\"One\")\n",
    )
    .unwrap();
    fs::write(source.join("Two.pine"), "//@version=6\nlibrary(\"Two\")\n").unwrap();
    fs::write(source.join("notes.txt"), "ignored").unwrap();
    assert_eq!(store.import_folder(&source).unwrap(), 2);
    let all = store.all_scripts().unwrap();
    assert_eq!(all.len(), 2);
    assert!(source.join("One.pine").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn import_and_delete() {
    let (mut store, root) = temp_store("import");
    let outside = root.join("incoming.pine");
    fs::create_dir_all(&root).unwrap();
    fs::write(&outside, "//@version=6\nstrategy(\"x\")\n").unwrap();
    let imported = store.import_file(&outside).unwrap();
    assert_eq!(imported.name, "incoming");
    assert_eq!(imported.script_type, ScriptType::Strategy);
    let binary = root.join("bad.pine");
    fs::write(&binary, [0xff, 0xfe, 0x41]).unwrap();
    // Latin-1 fallback still reads it.
    assert!(store.import_file(&binary).is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_save_is_refused_when_the_file_changed_outside_since_it_was_read() {
    let (mut store, dir) = temp_store("checked-save");
    let script = store
        .create("Checked", ScriptType::Indicator, None, false)
        .unwrap();
    let read = store.script(&script.id).unwrap().unwrap().source;
    let path = store.file_path(&script.id).unwrap().unwrap();
    // Unchanged: saves.
    let saved = store
        .save_if_unchanged(&script.id, "//@version=6\nindicator(\"A\")\n", &read)
        .unwrap();
    // Another tool rewrites the file.
    fs::write(&path, "//@version=6\nindicator(\"Outside\")\n").unwrap();
    let refused = store.save_if_unchanged(&script.id, "mine\n", &saved.source);
    assert!(matches!(refused, Err(ScriptStoreError::ChangedOnDisk)));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "//@version=6\nindicator(\"Outside\")\n",
        "the outside edit is untouched"
    );
    // A plain save still overwrites, as when the person chose to keep their version.
    store.save(&script.id, "mine\n").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "mine\n");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_rewrite_changes_the_files_stamp_and_a_missing_file_has_none() {
    let (mut store, dir) = temp_store("stamp");
    let script = store
        .create("Stamped", ScriptType::Indicator, None, false)
        .unwrap();
    let before = store.file_stamp(&script.id).unwrap();
    assert_eq!(before, store.file_stamp(&script.id).unwrap());
    let path = store.file_path(&script.id).unwrap().unwrap();
    fs::write(
        &path,
        "a much longer text than the template had before\n".repeat(20),
    )
    .unwrap();
    assert_ne!(before, store.file_stamp(&script.id).unwrap());
    assert!(store.file_stamp("no-such-script").is_none());
    let _ = fs::remove_dir_all(dir);
}
