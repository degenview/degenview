use super::migrate_swift::parse_swift_index;
use super::*;
use crate::store::Store;
use library::prefs::{ChartPosition, DrawerTab};
use std::sync::{Arc, Mutex};

fn manager(tag: &str) -> (PineStudio, PathBuf) {
    let root = std::env::temp_dir().join(format!("degenview-sm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
    (PineStudio::with_script_store(1, shared, Some(store)), root)
}

/// A manager with the example scripts loaded and the first one open and edited.
fn edited(tag: &str) -> (PineStudio, PathBuf, ScriptId) {
    let (mut manager, root) = manager(tag);
    manager.first_load();
    let id = manager.scripts[0].id.clone();
    manager.open_script(&id);
    let open = manager.open.as_mut().unwrap();
    open.editor.source.push_str("// edited\n");
    (manager, root, id)
}

/// With `DEGENVIEW_GALLERY_DIR` set, the sidebar is written as pictures: a row selected, the
/// button idle and under the pointer, in both themes. Without it the test does nothing, so it
/// never touches the shared theme.
#[test]
fn the_sidebar_gallery() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::{self, AppTheme};
    use eframe::egui::{Event, Pos2, vec2};
    if gallery_dir().is_none() {
        return;
    }
    let (mut manager, root) = manager("sidebar-gallery");
    manager.first_load();
    let script = manager.scripts[2].clone();
    manager.selected_row = Some(RowKey {
        group: group_of(&script),
        id: script.id.clone(),
    });
    for (tag, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let mut screen = Screen::new(vec2(240.0, 420.0));
        theme::apply(&screen.ctx, appearance);
        for (name, pointer) in [
            ("idle", None),
            ("hover-button", Some(Pos2::new(120.0, 420.0 - 28.0 - 24.0))),
            ("hover-row", Some(Pos2::new(120.0, 120.0))),
        ] {
            let events = pointer
                .map(|pos| vec![Event::PointerMoved(pos)])
                .unwrap_or_default();
            screen.run(events.clone(), |ui| manager.sidebar_ui(ui));
            let output = screen.run(events, |ui| manager.sidebar_ui(ui));
            screen.save(&format!("sidebar-{tag}-{name}"), output);
        }
    }
    // Every section at once, a favorite, and a search that finds nothing.
    manager.scripts[0].is_favorite = true;
    manager.scripts[1].script_type = ScriptType::Strategy;
    manager.scripts[3].script_type = ScriptType::Library;
    let mut screen = Screen::new(vec2(240.0, 640.0));
    theme::apply(&screen.ctx, AppTheme::Dark);
    screen.run(vec![], |ui| manager.sidebar_ui(ui));
    let output = screen.run(vec![], |ui| manager.sidebar_ui(ui));
    screen.save("sidebar-dark-sections", output);
    manager.search = "zzzz".into();
    screen.run(vec![], |ui| manager.sidebar_ui(ui));
    let output = screen.run(vec![], |ui| manager.sidebar_ui(ui));
    screen.save("sidebar-dark-no-matches", output);
    theme::apply(&egui::Context::default(), AppTheme::Dark);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn new_script_asks_for_a_name_unless_a_sheet_is_already_up() {
    let (mut manager, root) = manager("new-script");
    assert!(manager.can_create_script());
    manager.new_script();
    assert!(matches!(manager.cmds.as_slice(), [Cmd::NewScript]));
    manager.cmds.clear();
    manager.sheet = Some(Sheet::New {
        name: "kept".into(),
        error: None,
        needs_focus: false,
    });
    assert!(!manager.can_create_script());
    manager.new_script();
    assert!(manager.cmds.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn nothing_asks_while_every_script_is_saved() {
    let (mut manager, root) = manager("clean");
    manager.first_load();
    let id = manager.scripts[0].id.clone();
    manager.open_script(&id);
    assert!(!manager.begin_exit(ExitReason::CloseTab));
    assert!(manager.take_exit_decision().is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saving_from_the_question_writes_the_script_and_lets_the_close_go() {
    let (mut manager, root, id) = edited("exit-save");
    assert!(manager.begin_exit(ExitReason::CloseTab));
    let prompt = manager.exit_prompt.clone().unwrap();
    assert_eq!(prompt.scripts.len(), 1);
    assert_eq!((prompt.scripts[0].added, prompt.scripts[0].removed), (1, 0));
    manager.resolve_exit(&prompt, UnsavedChoice::Save);
    assert_eq!(
        manager.take_exit_decision(),
        Some(ExitDecision {
            reason: ExitReason::CloseTab,
            proceed: true
        })
    );
    assert!(manager.exit_prompt.is_none());
    assert!(!manager.open.as_ref().unwrap().editor.is_dirty());
    let saved = manager.with_store(|store| store.script(&id)).flatten();
    assert!(saved.unwrap().source.contains("// edited"));
    assert!(!manager.begin_exit(ExitReason::Quit), "nothing left to ask");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn dont_save_puts_the_script_back_and_forgets_its_draft() {
    let (mut manager, root, id) = edited("exit-discard");
    let before = manager
        .open
        .as_ref()
        .unwrap()
        .editor
        .saved_source()
        .to_owned();
    manager.flush_draft();
    assert!(manager.begin_exit(ExitReason::Quit));
    let prompt = manager.exit_prompt.clone().unwrap();
    manager.resolve_exit(&prompt, UnsavedChoice::Discard);
    assert_eq!(manager.take_exit_decision().map(|d| d.proceed), Some(true));
    assert_eq!(manager.open.as_ref().unwrap().editor.source, before);
    assert!(
        manager
            .with_store(|store| store.draft(&id))
            .flatten()
            .is_none()
    );
    assert!(!manager.begin_exit(ExitReason::Quit));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cancel_keeps_the_edits_and_the_window() {
    let (mut manager, root, _) = edited("exit-cancel");
    assert!(manager.begin_exit(ExitReason::CloseTab));
    // Asking again while the question is up keeps it, and a quit retitles it.
    assert!(manager.begin_exit(ExitReason::Quit));
    let prompt = manager.exit_prompt.clone().unwrap();
    assert_eq!(prompt.reason, ExitReason::Quit);
    manager.resolve_exit(&prompt, UnsavedChoice::Cancel);
    assert_eq!(
        manager.take_exit_decision(),
        Some(ExitDecision {
            reason: ExitReason::Quit,
            proceed: false
        })
    );
    assert!(manager.open.as_ref().unwrap().editor.is_dirty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_script_edited_earlier_is_asked_about_through_its_draft() {
    let (mut manager, root, first) = edited("exit-drafts");
    // Switching away keeps the edit as a draft (typing leaves one behind after a pause).
    let edited = manager.open.as_ref().unwrap().editor.source.clone();
    let draft = ScriptDraft {
        script_id: first.clone(),
        source: edited,
        modified_at_ms: now_ms(),
        based_on_revision_id: None,
    };
    manager.with_store(|store| store.save_draft(&draft));
    let second = manager.scripts[1].id.clone();
    manager.open_script(&second);
    manager
        .open
        .as_mut()
        .unwrap()
        .editor
        .source
        .push_str("// also\n");
    assert!(manager.begin_exit(ExitReason::Quit));
    let prompt = manager.exit_prompt.clone().unwrap();
    let ids: Vec<&str> = prompt.scripts.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&first.as_str()) && ids.contains(&second.as_str()));
    manager.resolve_exit(&prompt, UnsavedChoice::Save);
    assert_eq!(manager.take_exit_decision().map(|d| d.proceed), Some(true));
    let saved = manager.with_store(|store| store.script(&first)).flatten();
    assert!(saved.unwrap().source.contains("// edited"));
    assert!(manager.unsaved_scripts().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_swift_index_reads_both_dictionary_encodings() {
    let flat = parse_swift_index(br#"["A-1","Delta Tether.pine","B-2","RSI Zones.pine","odd"]"#);
    assert_eq!(flat["A-1"], "Delta Tether");
    assert_eq!(flat.len(), 2);
    let object = parse_swift_index(br#"{"A-1":"Delta Tether.pine"}"#);
    assert_eq!(object["A-1"], "Delta Tether");
    assert!(parse_swift_index(b"nope").is_empty());
}

#[test]
fn first_load_seeds_examples_once_and_never_for_existing_libraries() {
    let (mut manager, root) = manager("seed");
    manager.first_load();
    assert_eq!(manager.scripts.len(), 9);

    // A fresh settings store over the same folder: scripts exist, so nothing is added.
    let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
    let mut again = PineStudio::with_script_store(2, shared, Some(store));
    again.first_load();
    assert_eq!(again.scripts.len(), 9);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_select_rename_delete_flow() {
    let (mut manager, root) = manager("flow");
    manager.first_load();
    manager.create_script("Mine").unwrap();
    let cmds = std::mem::take(&mut manager.cmds);
    assert_eq!(cmds.len(), 1);
    for cmd in cmds {
        manager.run(cmd, &egui::Context::default());
    }
    assert_eq!(manager.open.as_ref().unwrap().name, "Mine");
    assert_eq!(manager.title(), "Mine");
    manager
        .open
        .as_mut()
        .unwrap()
        .editor
        .source
        .push_str("\n// edit");
    assert_eq!(manager.title(), "Mine — Edited");
    manager.save_open();
    assert_eq!(manager.title(), "Mine");

    assert!(
        manager.create_script("mine").is_err(),
        "case-insensitive conflict"
    );
    assert_eq!(
        manager.name_problem("MINE").as_deref(),
        Some("A script with that name already exists.")
    );
    assert_eq!(
        manager.name_problem("  "),
        None,
        "empty shows no error until it is submitted"
    );

    let id = manager.open.as_ref().unwrap().id.clone();
    manager.renaming = Some(Rename {
        row: RowKey {
            group: Group::Indicators,
            id: id.clone(),
        },
        text: "Renamed".into(),
        error: None,
        needs_focus: false,
    });
    manager.commit_rename();
    assert_eq!(manager.open.as_ref().unwrap().name, "Renamed");
    // A bad rename keeps editing and raises the alert.
    manager.renaming = Some(Rename {
        row: RowKey {
            group: Group::Indicators,
            id,
        },
        text: "a/b".into(),
        error: None,
        needs_focus: false,
    });
    manager.commit_rename();
    assert!(manager.renaming.is_some() && manager.error.is_some());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_window_draws_headlessly_through_selection_and_layout_changes() {
    let (mut manager, root) = manager("draw");
    let ctx = egui::Context::default();
    let frame = |manager: &mut PineStudio| {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            manager.ui(ui, None);
        });
        output.textures_delta.clear();
    };
    // Empty library and first load (seeds examples), then a selected script.
    frame(&mut manager);
    frame(&mut manager);
    assert_eq!(manager.scripts.len(), 9);
    let first = manager.scripts[0].clone();
    manager.cmds.push(Cmd::Select(RowKey {
        group: Group::Indicators,
        id: first.id,
    }));
    frame(&mut manager);
    frame(&mut manager);
    assert!(manager.open.is_some());
    for position in ChartPosition::ALL {
        manager.prefs.chart_position = position;
        frame(&mut manager);
    }
    for tab in DrawerTab::ALL {
        manager.cmds.push(Cmd::SetDrawerTab(tab));
        frame(&mut manager);
    }
    manager.cmds.push(Cmd::ToggleSidebar);
    manager.cmds.push(Cmd::ToggleChart);
    manager.cmds.push(Cmd::ToggleDrawer);
    manager.cmds.push(Cmd::NewScript);
    frame(&mut manager);
    frame(&mut manager);
    manager.sheet = None;
    manager.error = Some("boom".into());
    frame(&mut manager);
    let _ = std::fs::remove_dir_all(root);
}

fn target(
    script_id: Option<&str>,
    library: Option<&str>,
    ran_source: Option<&str>,
) -> crate::model::pine::logs_view::SourceTarget {
    crate::model::pine::logs_view::SourceTarget {
        script_id: script_id.map(str::to_owned),
        library: library.map(str::to_owned),
        offset: 17,
        ran_source: ran_source.map(str::to_owned),
    }
}

#[test]
fn an_unknown_log_function_fails_the_editors_compile_not_just_the_run() {
    let source = "//@version=6\r\nindicator(\"T\")\r\n\r\nlog.hi(\"hiii\")\r\n";
    let mut editor = EditorState::new("id", source, source);
    compile_into(&mut editor);
    assert_eq!(editor.status, CompileStatus::Error);
    let problem = editor
        .diagnostics
        .iter()
        .find(|d| d.code == "PINE3051")
        .expect("log.hi is not a function")
        .clone();
    assert_eq!(
        (problem.range.start.line, problem.range.start.column),
        (4, 1)
    );
    // Revealing it puts the caret on the call, though the editor's text holds the CRs.
    editor.reveal(&problem);
    let index = editor::syntax::compiled_utf16_to_char_index(
        &editor.source,
        editor.pending_jump().expect("a jump"),
    );
    assert!(
        editor
            .source
            .chars()
            .skip(index)
            .collect::<String>()
            .starts_with("log.hi")
    );
}

#[test]
fn a_logged_call_opens_its_script_with_the_caret_on_it() {
    let (mut manager, root) = manager("reveal");
    manager.first_load();
    let script = manager.scripts[1].clone();
    manager.reveal_source(&target(Some(&script.id), None, Some(&script.source)));
    let open = manager.open.as_ref().expect("the script is open");
    assert_eq!(open.id, script.id);
    assert_eq!(open.editor.pending_jump(), Some(17));
    assert!(manager.error.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_fresh_manager_survives_its_first_load_with_the_script_open_and_selected() {
    // The scripts are on disk from an earlier session; the Pine Studio tab does not exist
    // yet when "Source code" is chosen, so the app opens it and asks before its first frame.
    let (mut seeded, root) = manager("reveal-fresh");
    seeded.first_load();
    let script = seeded.scripts[1].clone();
    let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
    let mut fresh = PineStudio::with_script_store(2, shared, Some(store));
    assert!(!fresh.loaded);
    fresh.reveal_source(&target(Some(&script.id), None, Some(&script.source)));
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    for _ in 0..2 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            fresh.ui(ui, None);
        });
        output.textures_delta.clear();
    }
    assert!(fresh.loaded);
    assert_eq!(
        fresh.open.as_ref().map(|o| o.id.clone()),
        Some(script.id.clone())
    );
    assert_eq!(
        fresh.selected_row.as_ref().map(|row| row.id.clone()),
        Some(script.id)
    );
    assert!(fresh.error.is_none());
    // The editor took the caret request when it drew.
    assert_eq!(fresh.open.as_ref().unwrap().editor.pending_jump(), None);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_script_already_in_the_editor_keeps_its_unsaved_text_and_history() {
    let (mut manager, root, id) = edited("reveal-open");
    let before = manager.open.as_ref().unwrap().editor.source.clone();
    assert!(before.ends_with("// edited\n"));
    manager.reveal_source(&target(Some(&id), None, None));
    let open = manager.open.as_ref().unwrap();
    assert_eq!(open.editor.source, before);
    assert_eq!(open.editor.pending_jump(), Some(17));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_script_edited_since_the_message_still_opens_with_a_note() {
    let (mut manager, root) = manager("reveal-changed");
    manager.first_load();
    let script = manager.scripts[1].clone();
    manager.reveal_source(&target(Some(&script.id), None, Some("// what ran then\n")));
    assert_eq!(manager.open.as_ref().map(|o| o.id.clone()), Some(script.id));
    assert!(
        manager
            .error
            .as_deref()
            .is_some_and(|note| note.contains("edited"))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_script_that_is_gone_changes_nothing_and_says_so() {
    let (mut manager, root) = manager("reveal-gone");
    manager.first_load();
    let before = manager.open.as_ref().map(|o| o.id.clone());
    manager.reveal_source(&target(Some("no-such-script"), None, None));
    assert_eq!(manager.open.as_ref().map(|o| o.id.clone()), before);
    assert!(
        manager
            .error
            .as_deref()
            .is_some_and(|note| note.contains("no longer"))
    );
    manager.error = None;
    manager.reveal_source(&target(None, Some("Nope"), None));
    assert!(
        manager
            .error
            .as_deref()
            .is_some_and(|note| note.contains("library “Nope”"))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_library_call_opens_the_library_by_name() {
    let (mut manager, root) = manager("reveal-library");
    manager.first_load();
    let library = manager
        .with_store(|store| {
            store.create(
                "Dbg",
                ScriptType::Library,
                Some("//@version=6\nlibrary(\"Dbg\")\nexport f(float x) => x\n"),
                false,
            )
        })
        .expect("a library");
    manager.reveal_source(&target(None, Some("Dbg"), None));
    let open = manager.open.as_ref().expect("the library is open");
    assert_eq!(open.id, library.id);
    assert_eq!(open.editor.pending_jump(), Some(17));
    assert!(manager.error.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_alert_needs_a_chart_then_opens_the_sheet_for_the_script() {
    let (mut manager, root) = manager("alert");
    manager.first_load();
    let ctx = egui::Context::default();
    let demo = manager
        .scripts
        .iter()
        .find(|s| s.name == "Volume Spike Alert")
        .expect("a demo with alert() calls")
        .clone();

    manager.run(Cmd::CreateAlert(demo.id.clone()), &ctx);
    assert!(manager.sheet.is_none());
    assert!(
        manager
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Open a chart"))
    );
    manager.error = None;

    manager.chart = Some(ChartInfo {
        chart_id: 3,
        title: "BTC/USDT".into(),
        source_name: "Binance".into(),
        timeframe: "1h".into(),
        symbol_key: "BINANCE:BTCUSDT".into(),
        instances: vec![PineInstance::new(
            11,
            Some(demo.id.clone()),
            &demo.name,
            &demo.source,
        )],
    });
    manager.run(Cmd::CreateAlert(demo.id), &ctx);
    let Some(Sheet::Alert(sheet)) = &manager.sheet else {
        panic!("the Create Script Alert sheet opens");
    };
    assert_eq!(sheet.name, "Volume Spike Alert");
    assert_eq!(sheet.instance.id, 11);
    assert!(!sheet.call_sites.is_empty(), "the example raises an alert");
    assert!(!sheet.duplicate_exists);
    // And it draws.
    for _ in 0..2 {
        let chart = manager.chart.clone();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            manager.ui(ui, chart.clone());
        });
        output.textures_delta.clear();
    }
    assert!(
        matches!(manager.sheet, Some(Sheet::Alert(_))),
        "still open until chosen"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_outside_edit_reaches_the_open_editor_without_losing_or_duplicating_a_draft() {
    let (mut manager, root) = manager("external");
    manager.create_script("Ext").unwrap();
    for cmd in std::mem::take(&mut manager.cmds) {
        manager.run(cmd, &egui::Context::default());
    }
    let id = manager.open.as_ref().unwrap().id.clone();
    let saved_text = manager.open.as_ref().unwrap().editor.source.clone();
    assert_eq!(manager.open_text(&id).as_deref(), Some(saved_text.as_str()));
    assert_eq!(manager.open_text("other"), None);

    // A draft arrives: the editor shows it, dirty, with the saved text still the baseline.
    let draft = "//@version=6\nindicator(\"Ext\")\nplot(open)\n";
    assert!(manager.adopt_external_text(&id, draft, false));
    let open = manager.open.as_ref().unwrap();
    assert_eq!(open.editor.source, draft);
    assert!(open.editor.is_dirty());
    assert_eq!(open.editor.saved_source(), saved_text);
    assert!(
        !open.editor.has_pending_draft(),
        "the outside writer already stored it"
    );
    assert_eq!(open.editor.status, CompileStatus::Valid);

    // A bad draft shows its problems in the editor.
    assert!(manager.adopt_external_text(&id, "//@version=6\nindicator(\"Ext\")\nplot(\n", false));
    let open = manager.open.as_ref().unwrap();
    assert_eq!(open.editor.status, CompileStatus::Error);
    assert!(!open.editor.diagnostics.is_empty());

    // A save: the text is now the baseline, so the editor is clean and the list is current.
    assert!(manager.adopt_external_text(&id, draft, true));
    let open = manager.open.as_ref().unwrap();
    assert!(!open.editor.is_dirty());
    assert_eq!(manager.title(), "Ext");

    // A script that is not open is left alone.
    assert!(!manager.adopt_external_text("not-open", draft, true));
    assert_eq!(manager.open.as_ref().unwrap().id, id);
    let _ = std::fs::remove_dir_all(root);
}
