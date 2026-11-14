use super::inputs::{merge_input_changes, parse_input_args};
use super::*;
use crate::mcp::types::{EditAction, EditStatus, ScriptKind, Severity};
use crate::model::pine::script::ScriptType;
use crate::model::watchlist::{new_id, now_ms};
use crate::pine::compiler::program::InputValue;
use crate::store::scripts::hash_source;
use std::collections::HashMap;
use std::path::PathBuf;

fn temp_store(tag: &str) -> (ScriptStore, PathBuf) {
    let root = std::env::temp_dir().join(format!("degenview-mcp-pine-{tag}-{}", new_id()));
    let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
    (store, root)
}

const SHAPED_INPUTS: &str = "//@version=6\nindicator(\"Shaped\", overlay=true)\n\
len = input.int(20, \"Length\", minval=1, maxval=500)\n\
mult = input.float(2.5, \"Multiplier\")\n\
show = input.bool(true, \"Show\")\n\
col = input.color(color.red, \"Colour\")\n\
src = input.source(close, \"Source\")\n\
mode = input.string(\"fast\", \"Mode\", options=[\"fast\", \"slow\"])\n\
plot(ta.sma(src, len) * mult)\n";

fn args(json: serde_json::Value) -> InputArgs {
    serde_json::from_value(json).unwrap()
}

fn parsed(json: serde_json::Value) -> Result<HashMap<String, InputValue>, McpError> {
    let program = compile_source(SHAPED_INPUTS);
    assert!(program.is_valid(), "{:?}", program.diagnostics);
    let changes = parse_input_args(&program.input_schema, &args(json))?;
    let mut inputs = HashMap::new();
    merge_input_changes(&program.input_schema, &mut inputs, changes);
    Ok(inputs)
}

#[test]
fn input_arguments_are_checked_against_what_the_script_declares() {
    let ok = parsed(serde_json::json!({
        "len": 50, "mult": 3, "show": false, "col": "#00FF00",
        "src": "High", "mode": "slow"
    }))
    .unwrap();
    assert_eq!(ok["len"], InputValue::Int(50));
    assert_eq!(ok["mult"], InputValue::Float(3.0));
    assert_eq!(ok["show"], InputValue::Bool(false));
    assert_eq!(ok["col"], InputValue::Color(0x00FF_00FF));
    assert_eq!(ok["src"], InputValue::Source("high".into()));
    assert_eq!(ok["mode"], InputValue::String("slow".into()));
    assert_eq!(
        parsed(serde_json::json!({"col": "#11223344"})).unwrap()["col"],
        InputValue::Color(0x1122_3344)
    );

    for bad in [
        serde_json::json!({"nope": 1}),
        serde_json::json!({"len": 0}),
        serde_json::json!({"len": 501}),
        serde_json::json!({"len": 1.5}),
        serde_json::json!({"len": "20"}),
        serde_json::json!({"show": 1}),
        serde_json::json!({"col": "red"}),
        serde_json::json!({"col": "#12345"}),
        serde_json::json!({"src": "vwap"}),
        serde_json::json!({"mode": "medium"}),
        serde_json::json!({"mode": 1}),
    ] {
        assert!(
            matches!(parsed(bad.clone()), Err(McpError::InvalidArgument(_))),
            "{bad}"
        );
    }
    let unknown = parsed(serde_json::json!({"nope": 1})).unwrap_err();
    assert!(unknown.message().contains("len"), "{}", unknown.message());
}

#[test]
fn a_default_or_null_is_no_override() {
    let program = compile_source(SHAPED_INPUTS);
    let mut current = HashMap::from([
        ("len".to_owned(), InputValue::Int(50)),
        ("show".to_owned(), InputValue::Bool(false)),
    ]);
    let changes = parse_input_args(
        &program.input_schema,
        &args(serde_json::json!({"len": 20, "show": null, "mult": 4.5})),
    )
    .unwrap();
    merge_input_changes(&program.input_schema, &mut current, changes);
    assert_eq!(
        current,
        HashMap::from([("mult".to_owned(), InputValue::Float(4.5))])
    );
}

#[test]
fn only_a_compiling_indicator_or_strategy_can_be_applied() {
    let (mut store, root) = temp_store("apply");
    let good = create(&mut store, "Good", ScriptType::Indicator, SHAPED_INPUTS);
    let strategy = create(
        &mut store,
        "Strat",
        ScriptType::Strategy,
        "//@version=6\nstrategy(\"S\")\n",
    );
    let broken = create(
        &mut store,
        "Broken",
        ScriptType::Indicator,
        "//@version=6\nindicator(\"B\")\nplot(close\n",
    );
    let library = create(
        &mut store,
        "Lib",
        ScriptType::Library,
        "//@version=6\nlibrary(\"Lib\")\n",
    );
    let none = InputArgs::new();

    let applied =
        script_to_apply(&mut store, &good.id, &args(serde_json::json!({"len": 9}))).unwrap();
    assert_eq!(applied.source, SHAPED_INPUTS);
    assert_eq!(applied.inputs["len"], InputValue::Int(9));
    assert!(script_to_apply(&mut store, &strategy.id, &none).is_ok());
    assert!(matches!(
        script_to_apply(&mut store, &broken.id, &none),
        Err(McpError::CompilationFailed(_))
    ));
    assert!(matches!(
        script_to_apply(&mut store, &library.id, &none),
        Err(McpError::Unsupported(_))
    ));
    assert!(matches!(
        script_to_apply(&mut store, "missing", &none),
        Err(McpError::NotFound(_))
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_instance_is_checked_against_the_source_it_was_applied_with() {
    let current = HashMap::from([("len".to_owned(), InputValue::Int(50))]);
    let updated = instance_inputs(
        SHAPED_INPUTS,
        current.clone(),
        &args(serde_json::json!({"mult": 4.0})),
    )
    .unwrap();
    assert_eq!(updated["len"], InputValue::Int(50));
    assert_eq!(updated["mult"], InputValue::Float(4.0));
    assert!(matches!(
        instance_inputs("plot(", current, &args(serde_json::json!({"len": 3}))),
        Err(McpError::CompilationFailed(_))
    ));
}

const WITH_INPUTS: &str = "//@version=6\nindicator(\"Inputs\", overlay=true)\n\
len = input.int(20, \"Length\", minval=1, maxval=500, step=1, group=\"Main\", tooltip=\"Bars\")\n\
mult = input.float(2.5, \"Multiplier\")\n\
show = input.bool(true, \"Show\")\n\
col = input.color(color.red, \"Colour\")\n\
plot(ta.sma(close, len) * mult)\n";

fn create(store: &mut ScriptStore, name: &str, kind: ScriptType, source: &str) -> LocalScript {
    store.create(name, kind, Some(source), true).unwrap()
}

#[test]
fn lists_scripts_with_type_draft_and_compile_state_and_no_paths() {
    let (mut store, root) = temp_store("list");
    let plain = create(
        &mut store,
        "Plain",
        ScriptType::Indicator,
        "//@version=6\nindicator(\"P\")\nplot(close)\n",
    );
    let strat = create(
        &mut store,
        "Strat",
        ScriptType::Strategy,
        "//@version=6\nstrategy(\"S\")\n",
    );
    store
        .save_draft(&ScriptDraft {
            script_id: strat.id.clone(),
            source: "//@version=6\nstrategy(\"S2\")\n".into(),
            modified_at_ms: now_ms(),
            based_on_revision_id: None,
        })
        .unwrap();
    // A draft equal to the saved text is not a draft.
    store
        .save_draft(&ScriptDraft {
            script_id: plain.id.clone(),
            source: plain.source.clone(),
            modified_at_ms: now_ms(),
            based_on_revision_id: None,
        })
        .unwrap();

    let listed = list_scripts(&mut store).unwrap();
    assert_eq!(listed.total, 2);
    let by_name = |name: &str| listed.scripts.iter().find(|s| s.name == name).unwrap();
    assert_eq!(by_name("Strat").script_type, ScriptKind::Strategy);
    assert!(by_name("Strat").has_draft);
    assert!(!by_name("Plain").has_draft);
    assert_eq!(by_name("Plain").source_hash, hash_source(&plain.source));
    let json = serde_json::to_string(&listed).unwrap();
    assert!(
        !json.contains(root.to_str().unwrap()),
        "paths must not leak: {json}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saved_and_draft_are_never_substituted_for_each_other() {
    let (mut store, root) = temp_store("versions");
    let script = create(
        &mut store,
        "V",
        ScriptType::Indicator,
        "//@version=6\nindicator(\"V\")\nplot(close)\n",
    );

    let saved = get_script(&mut store, &script.id, Version::Saved).unwrap();
    assert_eq!(saved.version, SourceVersion::Saved);
    assert_eq!(saved.source, script.source);
    assert_eq!(saved.source_hash, hash_source(&script.source));
    assert!(!saved.has_draft);
    // No draft yet: asking for it is an error, not the saved text.
    assert!(matches!(
        get_script(&mut store, &script.id, Version::Draft),
        Err(McpError::NotFound(_))
    ));

    let draft_text = "//@version=6\nindicator(\"V\")\nplot(open)\n";
    store
        .save_draft(&ScriptDraft {
            script_id: script.id.clone(),
            source: draft_text.into(),
            modified_at_ms: now_ms(),
            based_on_revision_id: None,
        })
        .unwrap();
    let draft = get_script(&mut store, &script.id, Version::Draft).unwrap();
    assert_eq!(
        (draft.version, draft.source.as_str()),
        (SourceVersion::Draft, draft_text)
    );
    assert_eq!(draft.source_hash, hash_source(draft_text));
    assert!(draft.has_draft && draft.draft_modified_at.is_some());
    assert_eq!(
        get_script(&mut store, &script.id, Version::Saved)
            .unwrap()
            .source,
        script.source
    );
    assert!(matches!(
        get_script(&mut store, "nope", Version::Saved),
        Err(McpError::NotFound(_))
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn compiling_reports_real_diagnostics_with_positions() {
    let (mut store, root) = temp_store("compile");
    let ok = create(
        &mut store,
        "Ok",
        ScriptType::Indicator,
        "//@version=6\nindicator(\"Ok\")\nplot(close)\n",
    );
    let result = compile_script(&mut store, Target::Script(ok.id.clone(), Version::Saved)).unwrap();
    assert!(result.success);
    assert_eq!(result.script_type, Some(ScriptKind::Indicator));
    assert_eq!(result.title.as_deref(), Some("Ok"));
    assert_eq!(result.pine_version, Some(6));
    assert_eq!(
        (result.error_count, result.version),
        (0, SourceVersion::Saved)
    );

    let bad = compile_script(
        &mut store,
        Target::Inline("//@version=6\nindicator(\"B\")\nplot(\n".into()),
    )
    .unwrap();
    assert!(!bad.success);
    assert_eq!(bad.version, SourceVersion::Inline);
    assert!(bad.script_id.is_none());
    assert!(bad.error_count > 0);
    let first = &bad.diagnostics[0];
    assert_eq!(first.severity, Severity::Error);
    assert!(first.code.starts_with("PINE"), "{}", first.code);
    assert!(first.line >= 1 && first.column >= 1);
    assert!(!first.message.is_empty());

    // The wrong version header is its own coded error.
    let old = compile_script(
        &mut store,
        Target::Inline("//@version=4\nindicator(\"X\")\n".into()),
    )
    .unwrap();
    assert!(!old.success);
    assert_eq!(old.diagnostics[0].code, "PINE0002");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn compiling_changes_nothing_on_disk() {
    let (mut store, root) = temp_store("pure");
    let script = create(
        &mut store,
        "Pure",
        ScriptType::Indicator,
        "//@version=6\nindicator(\"P\")\nplot(close)\n",
    );
    let before = std::fs::read_to_string(store.file_path(&script.id).unwrap().unwrap()).unwrap();
    let record_before = store.script(&script.id).unwrap().unwrap().compile_record;
    compile_script(
        &mut store,
        Target::Script(script.id.clone(), Version::Saved),
    )
    .unwrap();
    compile_script(&mut store, Target::Inline("plot(close)".into())).unwrap();
    let after = std::fs::read_to_string(store.file_path(&script.id).unwrap().unwrap()).unwrap();
    assert_eq!(before, after);
    assert_eq!(
        store.script(&script.id).unwrap().unwrap().compile_record,
        record_before
    );
    assert!(store.draft(&script.id).unwrap().is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn oversized_source_is_refused() {
    let (mut store, root) = temp_store("big");
    let huge = "x".repeat(MAX_SOURCE_CHARS + 1);
    assert!(matches!(
        compile_script(&mut store, Target::Inline(huge)),
        Err(McpError::ResourceLimit(_))
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn inputs_come_from_the_compilers_own_schema() {
    let (mut store, root) = temp_store("inputs");
    let script = create(&mut store, "Inputs", ScriptType::Indicator, WITH_INPUTS);
    let result = script_inputs(&mut store, &script.id, Version::Saved).unwrap();
    assert!(result.compiled);
    let ids: Vec<_> = result.inputs.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["len", "mult", "show", "col"]);

    let len = &result.inputs[0];
    assert_eq!(len.value_type, "integer");
    assert_eq!(len.default, serde_json::json!(20));
    assert_eq!(
        (len.min, len.max, len.step),
        (Some(1.0), Some(500.0), Some(1.0))
    );
    assert_eq!(len.title.as_deref(), Some("Length"));
    assert_eq!(len.group.as_deref(), Some("Main"));
    assert_eq!(len.tooltip.as_deref(), Some("Bars"));
    assert_eq!(result.inputs[1].value_type, "float");
    assert_eq!(result.inputs[1].default, serde_json::json!(2.5));
    assert_eq!(result.inputs[2].default, serde_json::json!(true));
    assert_eq!(result.inputs[3].value_type, "color");
    assert!(
        result.inputs[3]
            .default
            .as_str()
            .is_some_and(|c| c.starts_with('#') && c.len() == 9)
    );

    // A draft's inputs are the draft's, not the saved script's.
    store
        .save_draft(&ScriptDraft {
            script_id: script.id.clone(),
            source: "//@version=6\nindicator(\"D\")\nn = input.int(5, \"N\")\nplot(close)\n".into(),
            modified_at_ms: now_ms(),
            based_on_revision_id: None,
        })
        .unwrap();
    let draft = script_inputs(&mut store, &script.id, Version::Draft).unwrap();
    assert_eq!(draft.inputs.len(), 1);
    assert_eq!(draft.inputs[0].id, "n");
    let _ = std::fs::remove_dir_all(root);
}

// MARK: Editing

const V1: &str = "//@version=6\nindicator(\"E\")\nplot(close)\n";
const V2: &str = "//@version=6\nindicator(\"E\")\nplot(open)\n";
const V3: &str = "//@version=6\nindicator(\"E\")\nplot(high)\n";
const BROKEN: &str = "//@version=6\nindicator(\"E\")\nplot(\n";

fn edit_store(tag: &str) -> (ScriptStore, LocalScript, PathBuf) {
    let (mut store, root) = temp_store(tag);
    let script = create(&mut store, "E", ScriptType::Indicator, V1);
    (store, script, root)
}

#[test]
fn a_draft_edit_changes_only_the_draft() {
    let (mut store, script, root) = edit_store("draft");
    let revision = hash_source(V1);
    let result = update_draft(&mut store, &script.id, &revision, V2, None).unwrap();
    assert_eq!(
        (result.action, result.status),
        (EditAction::UpdateDraft, EditStatus::Updated)
    );
    assert_eq!(result.previous_revision, revision);
    assert_eq!(result.revision, hash_source(V2));
    assert!(result.has_draft && result.saved_revision_id.is_none());

    // Saved text, its history and its compile state are exactly as they were.
    let after = store.script(&script.id).unwrap().unwrap();
    assert_eq!(after.source, V1);
    assert_eq!(after.latest_revision_id, script.latest_revision_id);
    assert_eq!(
        get_script(&mut store, &script.id, Version::Draft)
            .unwrap()
            .source,
        V2
    );
    assert_eq!(
        get_script(&mut store, &script.id, Version::Saved)
            .unwrap()
            .source,
        V1
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_stale_or_malformed_revision_is_a_conflict_and_nothing_is_written() {
    let (mut store, script, root) = edit_store("conflict");
    update_draft(&mut store, &script.id, &hash_source(V1), V2, None).unwrap();

    // The client still holds the saved revision, but a draft now exists: its edit loses.
    let stale = update_draft(&mut store, &script.id, &hash_source(V1), V3, None);
    let Err(McpError::Conflict(message)) = stale else {
        panic!("expected a conflict");
    };
    assert!(
        message.contains(&hash_source(V2)),
        "tells the caller the current revision"
    );
    assert_eq!(
        get_script(&mut store, &script.id, Version::Draft)
            .unwrap()
            .source,
        V2
    );

    for bad in ["", "abc", &"G".repeat(64), &hash_source(V2).to_uppercase()] {
        assert!(
            matches!(
                update_draft(&mut store, &script.id, bad, V3, None),
                Err(McpError::InvalidArgument(_))
            ),
            "{bad:?}"
        );
    }
    assert_eq!(
        get_script(&mut store, &script.id, Version::Draft)
            .unwrap()
            .source,
        V2
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn two_clients_editing_the_same_revision_cannot_both_win() {
    let (mut store, script, root) = edit_store("race");
    let revision = hash_source(V1);
    assert!(update_draft(&mut store, &script.id, &revision, V2, None).is_ok());
    assert!(matches!(
        update_draft(&mut store, &script.id, &revision, V3, None),
        Err(McpError::Conflict(_))
    ));
    assert_eq!(
        get_script(&mut store, &script.id, Version::Draft)
            .unwrap()
            .source,
        V2
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_open_editors_text_outranks_what_is_on_disk() {
    let (mut store, script, root) = edit_store("live");
    // The user has typed V3 into the open editor; nothing has reached disk yet.
    let live = Some(V3);
    // A client that read the saved text is told it is behind, and nothing is written.
    assert!(matches!(
        update_draft(&mut store, &script.id, &hash_source(V1), V2, live),
        Err(McpError::Conflict(_))
    ));
    assert!(store.draft(&script.id).unwrap().is_none());
    // One that read the editor's text may proceed.
    let result = update_draft(&mut store, &script.id, &hash_source(V3), V2, live).unwrap();
    assert_eq!(result.previous_revision, hash_source(V3));
    assert_eq!(store.draft(&script.id).unwrap().unwrap().source, V2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repeating_an_edit_is_a_no_op_and_sources_are_checked() {
    let (mut store, script, root) = edit_store("noop");
    let same = update_draft(&mut store, &script.id, &hash_source(V1), V1, None).unwrap();
    assert_eq!(same.status, EditStatus::Unchanged);
    assert!(!same.has_draft);
    assert!(
        store.draft(&script.id).unwrap().is_none(),
        "no file for a no-op"
    );

    assert!(matches!(
        update_draft(&mut store, &script.id, &hash_source(V1), "a\u{0}b", None),
        Err(McpError::InvalidArgument(_))
    ));
    assert!(matches!(
        update_draft(
            &mut store,
            &script.id,
            &hash_source(V1),
            &"x".repeat(MAX_EDIT_CHARS + 1),
            None
        ),
        Err(McpError::ResourceLimit(_))
    ));
    assert!(matches!(
        update_draft(&mut store, "missing", &hash_source(V1), V2, None),
        Err(McpError::NotFound(_))
    ));
    // Tabs, newlines and carriage returns are ordinary source text.
    assert!(
        update_draft(
            &mut store,
            &script.id,
            &hash_source(V1),
            "a\tb\r\nc\n",
            None
        )
        .is_ok()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saving_promotes_the_reviewed_draft_and_records_a_revision() {
    let (mut store, script, root) = edit_store("save");
    update_draft(&mut store, &script.id, &hash_source(V1), V2, None).unwrap();

    // A wrong revision saves nothing.
    assert!(matches!(
        save_script(&mut store, &script.id, &hash_source(V1), None, None),
        Err(McpError::Conflict(_))
    ));
    assert_eq!(store.script(&script.id).unwrap().unwrap().source, V1);

    let saved = save_script(&mut store, &script.id, &hash_source(V2), None, None).unwrap();
    assert_eq!(
        (saved.action, saved.status),
        (EditAction::Save, EditStatus::Saved)
    );
    assert_eq!(saved.revision, hash_source(V2));
    assert!(saved.saved_revision_id.is_some() && !saved.has_draft);
    let after = store.script(&script.id).unwrap().unwrap();
    assert_eq!(after.source, V2);
    assert_ne!(after.latest_revision_id, script.latest_revision_id);
    assert!(
        store.draft(&script.id).unwrap().is_none(),
        "the draft is consumed"
    );
    // A script has no history until its first save; this save is that first revision.
    assert_eq!(store.revisions(&script.id).unwrap().len(), 1);
    assert_eq!(store.revisions(&script.id).unwrap()[0].source, V2);

    // Nothing left to save.
    let again = save_script(&mut store, &script.id, &hash_source(V2), None, None).unwrap();
    assert_eq!(again.status, EditStatus::Unchanged);
    assert_eq!(store.revisions(&script.id).unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_draft_with_errors_is_never_saved_over_a_working_script() {
    let (mut store, script, root) = edit_store("broken");
    update_draft(&mut store, &script.id, &hash_source(V1), BROKEN, None).unwrap();
    let result = save_script(&mut store, &script.id, &hash_source(BROKEN), None, None);
    let Err(McpError::CompilationFailed(message)) = result else {
        panic!("expected COMPILATION_FAILED");
    };
    assert!(message.contains("PINE"), "{message}");
    let after = store.script(&script.id).unwrap().unwrap();
    assert_eq!(after.source, V1, "the working script is untouched");
    assert_eq!(
        store.draft(&script.id).unwrap().unwrap().source,
        BROKEN,
        "the draft is kept for fixing"
    );
    assert!(
        store.revisions(&script.id).unwrap().is_empty(),
        "no revision was recorded"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saving_what_an_open_editor_shows_works_without_a_draft_file() {
    let (mut store, script, root) = edit_store("save-live");
    let saved = save_script(&mut store, &script.id, &hash_source(V2), Some(V2), None).unwrap();
    assert_eq!(saved.status, EditStatus::Saved);
    assert_eq!(store.script(&script.id).unwrap().unwrap().source, V2);
    let _ = std::fs::remove_dir_all(root);
}

fn open_editor(saved: &str, conflict: bool) -> crate::pine_studio::OpenDiskState {
    crate::pine_studio::OpenDiskState {
        saved_source: saved.to_owned(),
        conflict,
    }
}

/// Another program rewrites the script's file behind the app.
fn edit_outside(store: &mut ScriptStore, id: &str, text: &str) {
    let path = store.file_path(id).unwrap().expect("script has a file");
    std::fs::write(path, text).unwrap();
}

#[test]
fn an_outside_edit_is_not_overwritten_when_the_open_editor_is_clean() {
    let (mut store, script, root) = edit_store("outside-clean");
    // The editor shows V1 (what it read), the file became V3 before it noticed.
    edit_outside(&mut store, &script.id, V3);
    let result = save_script(
        &mut store,
        &script.id,
        &hash_source(V1),
        Some(V1),
        Some(&open_editor(V1, false)),
    );
    // The editor's V1 is stale: writing it would silently undo the outside edit.
    assert!(matches!(result, Err(McpError::Conflict(_))), "{result:?}");
    let path = store.file_path(&script.id).unwrap().unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), V3);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_outside_edit_is_not_overwritten_when_the_open_editor_is_dirty() {
    let (mut store, script, root) = edit_store("outside-dirty");
    // Editor: opened V1, typed V2 (unsaved). Another program writes V3 to the file.
    edit_outside(&mut store, &script.id, V3);
    let result = save_script(
        &mut store,
        &script.id,
        &hash_source(V2),
        Some(V2),
        Some(&open_editor(V1, false)),
    );
    assert!(matches!(result, Err(McpError::Conflict(_))), "{result:?}");
    let path = store.file_path(&script.id).unwrap().unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), V3);
    assert!(
        store.revisions(&script.id).unwrap().is_empty(),
        "no revision was recorded"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_unresolved_conflict_in_the_editor_blocks_the_save() {
    let (mut store, script, root) = edit_store("outside-conflict");
    let result = save_script(
        &mut store,
        &script.id,
        &hash_source(V2),
        Some(V2),
        Some(&open_editor(V1, true)),
    );
    assert!(matches!(result, Err(McpError::Conflict(_))), "{result:?}");
    assert_eq!(store.script(&script.id).unwrap().unwrap().source, V1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_open_editor_that_has_seen_the_file_can_save() {
    let (mut store, script, root) = edit_store("outside-seen");
    let saved = save_script(
        &mut store,
        &script.id,
        &hash_source(V2),
        Some(V2),
        Some(&open_editor(V1, false)),
    )
    .unwrap();
    assert_eq!(saved.status, EditStatus::Saved);
    assert_eq!(store.script(&script.id).unwrap().unwrap().source, V2);
    let _ = std::fs::remove_dir_all(root);
}
