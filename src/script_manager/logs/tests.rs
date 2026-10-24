use super::*;
use crate::store::Store;
use std::sync::{Arc, Mutex};

fn manager(tag: &str) -> (ScriptManager, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("degenview-sm-logs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let shared: crate::store::SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let store = super::super::ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
    (
        ScriptManager::with_script_store(1, shared, Some(store)),
        root,
    )
}

#[test]
fn the_more_menu_command_opens_and_closes_the_pane() {
    let (mut manager, root) = manager("toggle");
    assert!(!manager.logs.open);
    manager.toggle_logs();
    assert!(manager.logs.open);
    manager.apply_logs_action(LogsAction::Close);
    assert!(!manager.logs.open);
    manager.toggle_logs();
    manager.toggle_logs();
    assert!(!manager.logs.open);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_pane_draws_beside_the_editor_and_its_switch_reaches_the_preview() {
    let (mut manager, root) = manager("draw");
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    manager.toggle_logs();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            manager.ui(ui, None);
        });
        output.textures_delta.clear();
    }
    assert!(manager.logs.open);
    assert!(manager.preview.logging());
    manager.apply_logs_action(LogsAction::SetLogging(PREVIEW_INSTANCE, false));
    assert!(!manager.preview.logging());
    // A message that is not there leaves things as they are.
    manager.apply_logs_action(LogsAction::Source(99));
    manager.apply_logs_action(LogsAction::ScrollToBar(99));
    assert!(manager.error.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_panes_options_are_kept_locally_under_their_own_key() {
    let (mut manager, root) = manager("prefs");
    manager.toggle_logs();
    manager.logs.view.filter.options.regex = true;
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    for _ in 0..2 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            manager.ui(ui, None);
        });
        output.textures_delta.clear();
    }
    let saved: Option<LogsPrefs> = manager
        .shared
        .lock()
        .unwrap()
        .setting::<LogsPrefs>(PREFS_KEY);
    assert!(saved.is_some_and(|prefs| prefs.search.regex));
    let _ = std::fs::remove_dir_all(root);
}
