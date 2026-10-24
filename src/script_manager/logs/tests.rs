use super::*;
use crate::script_manager::prefs::DrawerTab;
use crate::store::Store;
use eframe::egui;
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

/// The text drawn by `frames` frames of the manager.
fn drawn_text(manager: &mut ScriptManager, ctx: &egui::Context, frames: usize) -> String {
    let mut text = String::new();
    for _ in 0..frames {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            manager.ui(ui, None);
        });
        output.textures_delta.clear();
        text.clear();
        for clipped in output.shapes {
            if let egui::Shape::Text(shape) = clipped.shape {
                text.push_str(&shape.galley.job.text);
                text.push('\n');
            }
        }
    }
    text
}

/// A manager with a script open and the drawer showing the Logs tab.
fn on_the_logs_tab(tag: &str) -> (ScriptManager, std::path::PathBuf, egui::Context) {
    let (mut manager, root) = manager(tag);
    manager.first_load();
    let id = manager.scripts[0].id.clone();
    manager.open_script(&id);
    manager.prefs.drawer_open = true;
    manager.prefs.chart_visible = true;
    manager.prefs.drawer_tab = DrawerTab::Logs;
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    (manager, root, ctx)
}

#[test]
fn the_drawer_has_a_logs_tab_and_shows_the_pane_without_its_own_title() {
    let (mut manager, root, ctx) = on_the_logs_tab("tab");
    let text = drawn_text(&mut manager, &ctx, 3);
    assert!(DrawerTab::ALL.contains(&DrawerTab::Logs));
    assert!(text.contains("Logs"), "{text}");
    assert!(text.contains("How to work with logs"), "{text}");
    // The tab is the title: the pane's own "Pine logs" header and close button are left out.
    assert!(!text.contains("Pine logs"), "{text}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn there_is_no_more_menu_or_side_pane_any_longer() {
    let (mut manager, root, ctx) = on_the_logs_tab("nomore");
    manager.prefs.drawer_tab = DrawerTab::Inputs;
    let text = drawn_text(&mut manager, &ctx, 3);
    assert!(!text.contains("How to work with logs"), "{text}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_switch_in_the_tab_reaches_the_preview() {
    let (mut manager, root, ctx) = on_the_logs_tab("switch");
    drawn_text(&mut manager, &ctx, 2);
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
fn the_tabs_options_are_kept_locally_under_their_own_key() {
    let (mut manager, root, ctx) = on_the_logs_tab("prefs");
    manager.logs.view.filter.options.regex = true;
    drawn_text(&mut manager, &ctx, 2);
    let saved: Option<LogsPrefs> = manager
        .shared
        .lock()
        .unwrap()
        .setting::<LogsPrefs>(PREFS_KEY);
    assert!(saved.is_some_and(|prefs| prefs.search.regex));
    // A manager made afterwards starts from them.
    let fresh = LogsPane::load(&manager.shared);
    assert!(fresh.view.filter.options.regex && fresh.view.embedded);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_logs_tab_is_remembered_with_the_layout() {
    let prefs = super::super::prefs::LayoutPrefs {
        drawer_tab: DrawerTab::Logs,
        ..Default::default()
    };
    let json = serde_json::to_string(&prefs).unwrap();
    assert!(json.contains("\"logs\""), "{json}");
    let back: super::super::prefs::LayoutPrefs = serde_json::from_str(&json).unwrap();
    assert_eq!(back.drawer_tab, DrawerTab::Logs);
}
