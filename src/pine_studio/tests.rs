use super::migrate_swift::parse_swift_index;
use super::*;
use crate::store::Store;
use crate::ui::render_tests::{Screen, gallery_dir};
use eframe::egui::{Event, pos2, vec2};
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

/// Frames of the whole window with the script picker up, so the popup is seen under its button.
fn picker_frames(
    manager: &mut PineStudio,
    screen: &mut Screen,
    events: Vec<Event>,
) -> egui::FullOutput {
    let mut last = None;
    for _ in 0..4 {
        crate::ui::theme::sync(&screen.ctx);
        last = Some(screen.run(events.clone(), |ui| {
            manager.ui(ui, None);
        }));
    }
    last.expect("a frame")
}

/// With `DEGENVIEW_GALLERY_DIR` set, the script picker is written as pictures in both themes: the
/// list with the open script lit, a row under the pointer, every group at once, a search that
/// finds nothing, and the short form of an empty library.
#[test]
#[ignore = "writes images for a person to look at"]
fn the_picker_gallery() {
    use crate::ui::theme::{self, AppTheme};
    if gallery_dir().is_none() {
        return;
    }
    let _theme = theme::RestoreDefaultTheme;
    let (mut manager, root) = manager("picker-gallery");
    manager.first_load();
    let first = manager.scripts[2].id.clone();
    let favorite = manager.scripts[0].id.clone();
    manager.with_store(|store| {
        store.set_favorite(&favorite, true)?;
        store.create("Shared Maths", ScriptType::Library, None, false)?;
        Ok(())
    });
    manager.refresh();
    manager.open_script(&first);
    for (tag, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let mut screen = Screen::new(vec2(1000.0, 760.0));
        theme::apply(&screen.ctx, appearance);
        manager.picker.open();
        let output = picker_frames(&mut manager, &mut screen, Vec::new());
        screen.save(&format!("picker-{tag}-list"), output);
        let hover = Event::PointerMoved(pos2(520.0, 250.0));
        let output = picker_frames(&mut manager, &mut screen, vec![hover]);
        screen.save(&format!("picker-{tag}-hover"), output);
        // A right-click on a row: its menu hangs beside the popup, which stays.
        let click = |pressed| Event::PointerButton {
            pos: pos2(520.0, 250.0),
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        picker_frames(&mut manager, &mut screen, vec![click(true)]);
        let output = picker_frames(&mut manager, &mut screen, vec![click(false)]);
        screen.save(&format!("picker-{tag}-row-menu"), output);
        assert!(manager.picker.is_open());
        manager.picker.open();
        picker_frames(&mut manager, &mut screen, Vec::new());
        let typed = Event::Text("zzzz".to_owned());
        let output = picker_frames(&mut manager, &mut screen, vec![typed]);
        screen.save(&format!("picker-{tag}-no-matches"), output);
    }
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
        manager.name_problem("MINE", None).as_deref(),
        Some("A script with that name already exists.")
    );
    assert_eq!(
        manager.name_problem("  ", None),
        None,
        "empty shows no error until it is submitted"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// A manager with "Mine" open and a second script, "Other", beside it.
fn with_two_named(tag: &str) -> (PineStudio, PathBuf, ScriptId) {
    let (mut manager, root) = manager(tag);
    manager.first_load();
    manager.create_script("Mine").unwrap();
    manager.create_script("Other").unwrap();
    manager.refresh();
    let id = manager
        .scripts
        .iter()
        .find(|script| script.name == "Mine")
        .map(|script| script.id.clone())
        .unwrap();
    manager.cmds.clear();
    manager.open_script(&id);
    (manager, root, id)
}

#[test]
fn renaming_a_script_changes_its_name_and_its_title() {
    let (mut manager, root, id) = with_two_named("rename-valid");
    manager.rename_script(&id, "Renamed").unwrap();
    assert_eq!(manager.open.as_ref().unwrap().name, "Renamed");
    assert_eq!(manager.title(), "Renamed");
    assert!(
        manager
            .script_named(&id)
            .is_some_and(|s| s.name == "Renamed")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_name_another_script_has_cannot_be_taken_but_the_scripts_own_can() {
    let (mut manager, root, id) = with_two_named("rename-conflict");
    let taken = crate::store::scripts::ScriptStoreError::NameConflict.to_string();
    assert_eq!(manager.name_problem("other", Some(&id)), Some(taken));
    assert!(manager.rename_script(&id, "Other").is_err());
    assert_eq!(manager.open.as_ref().unwrap().name, "Mine");
    // Its own name, even in other letters, is not in the way.
    assert_eq!(manager.name_problem("Mine", Some(&id)), None);
    assert_eq!(manager.name_problem("MINE", Some(&id)), None);
    manager.rename_script(&id, "Mine").unwrap();
    assert_eq!(manager.open.as_ref().unwrap().name, "Mine");
    // A name that is not valid is explained.
    assert!(manager.rename_script(&id, "a/b").is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_rename_sheet_starts_on_the_current_name_and_enter_takes_the_new_one() {
    let (mut manager, root, id) = with_two_named("rename-sheet");
    manager.run(Cmd::BeginRename(id.clone()), &egui::Context::default());
    assert!(matches!(
        &manager.sheet,
        Some(Sheet::Rename { name, error: None, .. }) if name == "Mine"
    ));
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let frame = |manager: &mut PineStudio, events: Vec<Event>| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                manager.ui(ui, None);
            },
        );
        output.textures_delta.clear();
    };
    frame(&mut manager, Vec::new());
    assert!(
        manager.sheet.is_some(),
        "the sheet stays up until it is answered"
    );
    if let Some(Sheet::Rename { name, .. }) = manager.sheet.as_mut() {
        *name = "Renamed".to_owned();
    }
    let enter = Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut manager, vec![enter]);
    assert!(manager.sheet.is_none());
    assert_eq!(manager.open.as_ref().unwrap().name, "Renamed");
    // Escape puts it away without renaming.
    manager.run(Cmd::BeginRename(id), &ctx);
    let escape = Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut manager, vec![escape]);
    assert!(manager.sheet.is_none());
    assert_eq!(manager.open.as_ref().unwrap().name, "Renamed");
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
    manager.cmds.push(Cmd::Select(first.id));
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
    manager.cmds.push(Cmd::OpenScriptPicker);
    frame(&mut manager);
    frame(&mut manager);
    assert!(manager.picker.is_open());
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

/// One click at `at` in the whole window: the pointer arrives, goes down and comes up.
fn click_window(manager: &mut PineStudio, screen: &mut Screen, at: egui::Pos2) {
    let press = |pressed| Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for events in [
        vec![],
        vec![Event::PointerMoved(at)],
        vec![press(true)],
        vec![press(false)],
        vec![],
    ] {
        crate::ui::theme::sync(&screen.ctx);
        screen.run(events, |ui| {
            manager.ui(ui, None);
        });
    }
}

#[test]
fn the_rail_moves_and_hides_the_preview_chart_and_opens_the_panels_in_the_whole_window() {
    let (mut manager, root, _) = edited("rail-window");
    let mut screen = Screen::new(vec2(1360.0, 860.0));
    click_window(&mut manager, &mut screen, pos2(0.0, 0.0));
    let button = |n| rail::button_centre(1360.0, crate::ui::kit::bar::HEIGHT, n);

    // Assistant, profiler and debugger are the first three buttons.
    click_window(&mut manager, &mut screen, button(0));
    assert!(manager.prefs.ai_open);
    click_window(&mut manager, &mut screen, button(2));
    assert!(manager.prefs.debug_open);

    // The position menu hangs left of the rail; its third row is "Below Code".
    assert_eq!(manager.prefs.chart_position, ChartPosition::Left);
    click_window(&mut manager, &mut screen, button(3));
    let row = pos2(1360.0 - rail::WIDTH - 30.0, button(3).y - 15.0 + 75.0);
    click_window(&mut manager, &mut screen, row);
    assert_eq!(manager.prefs.chart_position, ChartPosition::Bottom);

    assert!(manager.prefs.chart_visible);
    click_window(&mut manager, &mut screen, button(4));
    assert!(!manager.prefs.chart_visible);
    click_window(&mut manager, &mut screen, button(4));
    assert!(manager.prefs.chart_visible);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn choosing_a_theme_in_the_theme_menu_makes_it_the_editors_through_the_host() {
    use crate::model::editor_theme::{BuiltInTheme, ThemeSelection};
    let _published = crate::ui::editor_appearance::RestorePublished;
    let (mut manager, root) = manager("theme-menu-choose");
    let ctx = egui::Context::default();
    assert_eq!(manager.themes.choices().selected, ThemeSelection::Auto);
    manager.run(
        Cmd::SetEditorTheme(ThemeSelection::BuiltIn(BuiltInTheme::Dracula)),
        &ctx,
    );
    assert_eq!(
        manager.themes.choices().selected,
        ThemeSelection::BuiltIn(BuiltInTheme::Dracula)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn configure_themes_asks_the_app_for_the_editor_theme_page_of_settings() {
    let (mut manager, root) = manager("theme-menu-configure");
    let ctx = egui::Context::default();
    manager.run(Cmd::OpenThemeSettings, &ctx);
    assert_eq!(manager.actions, [PineStudioAction::OpenEditorThemeSettings]);
    let _ = std::fs::remove_dir_all(root);
}

/// With `DEGENVIEW_GALLERY_DIR` set, the whole window is written as pictures in both themes at the
/// default and the narrowest size, with the preview chart at each place, with the assistant and
/// debugger panels open under the code, and with the position menu open.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_rail() {
    use crate::ui::theme::{self, AppTheme};
    use debug::studio_tests::Rig;
    if gallery_dir().is_none() {
        return;
    }
    let _theme = theme::RestoreDefaultTheme;
    let shoot = |rig: &mut Rig, screen: &mut Screen, name: &str| {
        let mut last = None;
        for _ in 0..4 {
            theme::sync(&screen.ctx);
            last = Some(screen.run(Vec::new(), |ui| {
                rig.manager.ui(ui, None);
            }));
        }
        screen.save(name, last.expect("a frame"));
    };
    for (theme_name, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (size_name, size) in [
            ("wide", vec2(1360.0, 860.0)),
            ("narrow", vec2(960.0, 640.0)),
        ] {
            let mut rig = Rig::new(&format!("rail-{size_name}-{theme_name}"));
            let mut screen = Screen::new(size);
            theme::apply(&screen.ctx, appearance);
            for position in ChartPosition::ALL {
                rig.manager.prefs.chart_position = position;
                shoot(
                    &mut rig,
                    &mut screen,
                    &format!(
                        "rail-{size_name}-{theme_name}-{}",
                        position.title().replace(' ', "-")
                    ),
                );
            }
            rig.manager.prefs.chart_position = ChartPosition::Left;
            rig.manager.prefs.ai_open = true;
            rig.manager.prefs.debug_open = true;
            shoot(
                &mut rig,
                &mut screen,
                &format!("rail-{size_name}-{theme_name}-panels"),
            );
            rig.manager.prefs.chart_position = ChartPosition::Bottom;
            shoot(
                &mut rig,
                &mut screen,
                &format!("rail-{size_name}-{theme_name}-panels-below"),
            );
            rig.manager.prefs.ai_open = false;
            rig.manager.prefs.debug_open = false;
            let button = rail::button_centre(size.x, crate::ui::kit::bar::HEIGHT, 3);
            click_window(&mut rig.manager, &mut screen, button);
            shoot(
                &mut rig,
                &mut screen,
                &format!("rail-{size_name}-{theme_name}-menu"),
            );
        }
    }
}

/// Where the menu strip starts in the window: under the bar, at the editor's left edge, which is
/// half way across with the preview chart at its left.
fn strip_corner(size: egui::Vec2, position: ChartPosition) -> egui::Pos2 {
    let left = if position == ChartPosition::Left {
        size.x / 2.0
    } else {
        0.0
    };
    pos2(left, crate::ui::kit::bar::HEIGHT)
}

/// Frames of the whole window, the pointer left where it is.
fn settle(manager: &mut PineStudio, screen: &mut Screen) -> egui::FullOutput {
    let mut last = None;
    for _ in 0..4 {
        crate::ui::theme::sync(&screen.ctx);
        last = Some(screen.run(Vec::new(), |ui| {
            manager.ui(ui, None);
        }));
    }
    last.expect("a frame")
}

fn hover_window(manager: &mut PineStudio, screen: &mut Screen, at: egui::Pos2) {
    for _ in 0..2 {
        crate::ui::theme::sync(&screen.ctx);
        screen.run(vec![Event::PointerMoved(at)], |ui| {
            manager.ui(ui, None);
        });
    }
}

#[test]
fn opening_a_menu_keeps_the_editors_selection_so_cut_and_copy_stay_on() {
    use menu_bar::Menu;
    let (mut manager, root, _) = edited("menu-keeps-selection");
    manager.prefs.chart_visible = false;
    let size = vec2(1360.0, 860.0);
    let mut screen = Screen::new(size);
    settle(&mut manager, &mut screen);
    manager.edit_command(editor::EditAction::SelectAll);
    settle(&mut manager, &mut screen);
    let ctx = screen.ctx.clone();
    let selected = |manager: &mut PineStudio| {
        manager
            .open
            .as_mut()
            .map(|open| open.editor.menu_state(&ctx).selection)
    };
    assert_eq!(selected(&mut manager), Some(true));

    let corner = strip_corner(size, ChartPosition::Bottom);
    click_window(
        &mut manager,
        &mut screen,
        menu_bar::title_centre(Menu::Edit, corner),
    );
    settle(&mut manager, &mut screen);
    assert_eq!(
        selected(&mut manager),
        Some(true),
        "the click on the strip must not collapse the selection"
    );
    let _ = std::fs::remove_dir_all(root);
}

fn key_event(key: egui::Key, command: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command,
            mac_cmd: command && crate::ui::shortcut::is_mac(),
            ctrl: command && !crate::ui::shortcut::is_mac(),
            ..egui::Modifiers::NONE
        },
    }
}

/// A window with the editor alone under the bar (no preview chart), its text all selected so it
/// has the keyboard, and the click and frame helpers for it.
struct EditorWindow {
    manager: PineStudio,
    screen: Screen,
    corner: egui::Pos2,
    root: PathBuf,
}

impl EditorWindow {
    fn new(tag: &str) -> Self {
        let (mut manager, root, _) = edited(tag);
        manager.prefs.chart_visible = false;
        let size = vec2(1360.0, 860.0);
        let mut screen = Screen::new(size);
        settle(&mut manager, &mut screen);
        manager.edit_command(editor::EditAction::SelectAll);
        settle(&mut manager, &mut screen);
        Self {
            manager,
            screen,
            corner: strip_corner(size, ChartPosition::Bottom),
            root,
        }
    }

    fn click_title(&mut self, menu: menu_bar::Menu) -> egui::Pos2 {
        let at = menu_bar::title_centre(menu, self.corner);
        click_window(&mut self.manager, &mut self.screen, at);
        at
    }

    /// Clicks the `row`th row (rows only) of the menu opened from the title at `title`.
    fn click_row(&mut self, menu: menu_bar::Menu, title: egui::Pos2, row: usize) {
        let at = pos2(title.x + 20.0, menu_row_y(self.corner, menu, row));
        click_window(&mut self.manager, &mut self.screen, at);
    }

    fn frames(&mut self, events: Vec<Event>) {
        for (index, _) in (0..3).enumerate() {
            crate::ui::theme::sync(&self.screen.ctx);
            let events = if index == 0 {
                events.clone()
            } else {
                Vec::new()
            };
            self.screen.run(events, |ui| {
                self.manager.ui(ui, None);
            });
        }
    }

    fn focused(&self) -> Option<egui::Id> {
        self.screen.ctx.memory(|memory| memory.focused())
    }

    fn editor_id(&self) -> egui::Id {
        self.manager
            .open
            .as_ref()
            .expect("a script is open")
            .editor
            .focus_id()
    }

    fn find_open(&self) -> bool {
        self.manager
            .open
            .as_ref()
            .is_some_and(|open| open.editor.find.open)
    }
}

impl Drop for EditorWindow {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn open_script_from_the_file_menu_leaves_the_keyboard_in_the_pickers_search() {
    use menu_bar::Menu;
    let mut window = EditorWindow::new("menu-open-script");
    let before = window.manager.open.as_ref().unwrap().editor.source.clone();
    let title = window.click_title(Menu::File);
    window.click_row(Menu::File, title, 1);
    window.frames(Vec::new());
    assert!(window.manager.picker.is_open());
    let focused = window.focused();
    assert!(focused.is_some() && focused != Some(window.editor_id()));
    window.frames(vec![Event::Text("zz".to_owned())]);
    assert_eq!(
        window.manager.open.as_ref().unwrap().editor.source,
        before,
        "what is typed goes to the search, not into the code"
    );
}

#[test]
fn find_with_the_edit_menu_up_leaves_the_keyboard_in_the_find_field() {
    use menu_bar::Menu;
    let mut window = EditorWindow::new("menu-find");
    window.click_title(Menu::Edit);
    assert_eq!(
        window.focused(),
        Some(window.editor_id()),
        "held for the menu"
    );
    window.frames(vec![key_event(egui::Key::F, true)]);
    window.frames(Vec::new());
    assert!(window.find_open());
    assert_eq!(window.focused(), Some(egui::Id::new("script-find-field")));
}

#[test]
fn escape_that_closes_a_menu_or_the_picker_leaves_the_find_bar_open() {
    use menu_bar::Menu;
    let mut window = EditorWindow::new("menu-escape");
    window.manager.edit_command(editor::EditAction::Find);
    window.frames(Vec::new());
    assert!(window.find_open());

    window.click_title(Menu::Edit);
    assert!(egui::Popup::is_any_open(&window.screen.ctx));
    window.frames(vec![key_event(egui::Key::Escape, false)]);
    assert!(
        !egui::Popup::is_any_open(&window.screen.ctx),
        "the menu closed"
    );
    assert!(window.find_open(), "and the find bar stayed");

    window.manager.picker.open();
    window.frames(Vec::new());
    window.frames(vec![key_event(egui::Key::Escape, false)]);
    assert!(!window.manager.picker.is_open(), "the picker closed");
    assert!(window.find_open(), "and the find bar stayed");
}

/// With `DEGENVIEW_GALLERY_DIR` set, the whole window is written as pictures in both themes at the
/// default and the narrowest size with each of the strip's menus open, the Edit menu with a
/// selection and with the caret in a change, a submenu of View and one of Edit open, and the Theme
/// menu with a saved theme of the person's own.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_menu_bar() {
    use crate::model::editor_theme::{BuiltInTheme, ThemeSelection, ThemeSource};
    use crate::ui::theme::{self, AppTheme};
    use debug::studio_tests::Rig;
    use menu_bar::Menu;
    if gallery_dir().is_none() {
        return;
    }
    let _theme = theme::RestoreDefaultTheme;
    let _published = crate::ui::editor_appearance::RestorePublished;
    for (theme_name, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (size_name, size) in [
            ("wide", vec2(1360.0, 860.0)),
            ("narrow", vec2(960.0, 640.0)),
        ] {
            // (picture, menu, a row to point at, whether to select everything first)
            let shots: [(&str, Menu, Option<usize>, bool); 7] = [
                ("file", Menu::File, None, false),
                ("edit-selection", Menu::Edit, None, true),
                ("edit-change", Menu::Edit, Some(EDIT_CHANGE_ROW), false),
                (
                    "edit-breakpoint",
                    Menu::Edit,
                    Some(EDIT_CHANGE_ROW - 1),
                    false,
                ),
                ("view", Menu::View, None, false),
                ("view-position", Menu::View, Some(VIEW_POSITION_ROW), false),
                ("theme", Menu::Theme, None, false),
            ];
            for (name, menu, pointed, select) in shots {
                let tag = format!("menu-bar-{size_name}-{theme_name}-{name}");
                let mut rig = Rig::new(&tag);
                let mut screen = Screen::new(size);
                theme::apply(&screen.ctx, appearance);
                let manager = &mut rig.manager;
                manager.prefs.drawer_open = true;
                if menu == Menu::Theme {
                    // One theme worn and one with a name as long as a name may be.
                    let source = ThemeSource::BuiltIn(BuiltInTheme::Dracula);
                    let long = "A custom theme named as long as a name may be";
                    manager.themes.create(long, source).expect("a theme");
                    let worn = manager.themes.create("My Dracula", source);
                    manager
                        .themes
                        .select(ThemeSelection::Custom(worn.expect("a theme")));
                }
                if let Some(open) = manager.open.as_mut() {
                    // A saved script with one line changed, the caret on it.
                    let source = open.editor.source.clone();
                    open.editor.replace_source(&source, true);
                    open.editor.source =
                        source.replace("value = calculate(close)", "value = calculate(open)");
                }
                settle(manager, &mut screen);
                if select {
                    manager.edit_command(editor::EditAction::SelectAll);
                } else if let Some(open) = manager.open.as_mut() {
                    let at = open
                        .editor
                        .source
                        .find("value = calculate(open)")
                        .unwrap_or(0);
                    open.editor.place_caret(&screen.ctx, at);
                    // Hands the keyboard to the editor, as a click in the text would.
                    manager.edit_command(editor::EditAction::FindNext);
                }
                settle(manager, &mut screen);
                let corner = strip_corner(size, ChartPosition::Left);
                let title = menu_bar::title_centre(menu, corner);
                click_window(manager, &mut screen, title);
                if let Some(row) = pointed {
                    let x = title.x + 20.0;
                    hover_window(manager, &mut screen, pos2(x, menu_row_y(corner, menu, row)));
                }
                let output = settle(manager, &mut screen);
                screen.save(&tag, output);
            }
        }
    }
}

/// The Edit menu's row that opens the change submenu, and the one above it (Breakpoint), counted
/// as rows without the separators.
const EDIT_CHANGE_ROW: usize = 18;
/// The View menu's Preview Chart row.
const VIEW_POSITION_ROW: usize = 3;

/// The middle of the `row`th row of `menu` (counting rows, not separators), for a menu opened from
/// the strip at `corner`: a gap under the strip, the menu's margin, 24 points a row and 9 a
/// separator.
fn menu_row_y(corner: egui::Pos2, menu: menu_bar::Menu, row: usize) -> f32 {
    use menu_bar::Menu;
    // How many separators come before each row, by menu.
    let separators = match menu {
        Menu::File => |row: usize| [3, 6, 8, 9].iter().filter(|at| row >= **at).count(),
        Menu::Edit => |row: usize| [2, 6, 9, 14, 17].iter().filter(|at| row >= **at).count(),
        Menu::View => |row: usize| [3, 5].iter().filter(|at| row >= **at).count(),
        Menu::Theme => |row: usize| [4, 5].iter().filter(|at| row >= **at).count(),
    };
    corner.y
        + menu_bar::HEIGHT
        + 4.0
        + 5.0
        + row as f32 * 24.0
        + separators(row) as f32 * 9.0
        + 12.0
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

/// Every piece of text a frame drew, in drawing order.
fn drawn_texts(output: &egui::FullOutput) -> Vec<String> {
    fn collect(shape: &egui::Shape, found: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => found.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut found);
    }
    found
}

/// The Studio's own words in the pseudo-locale, at the narrowest window, in both themes: the menu
/// strip, every drawer tab, and the assistant and debugger panels. Each word of the interface is
/// put through `tr!`, so none shows without accents and brackets, and the longer text fits (the
/// frames are also written as pictures when `DEGENVIEW_GALLERY_DIR` is set).
#[test]
fn the_studio_draws_in_the_pseudo_locale() {
    use crate::i18n::{Locale, with_locale};
    use crate::ui::theme::{self, AppTheme};
    use debug::studio_tests::Rig;
    let _theme = theme::RestoreDefaultTheme;
    let size = vec2(960.0, 640.0);
    // The English words that must never be drawn as they are: the strip's titles, the drawer's
    // tab names and the panels' headings.
    let bare = [
        "File", "Edit", "View", "Theme", "Inputs", "Report", "Problems", "Logs", "Profile",
    ];
    with_locale(Locale::PSEUDO, || {
        for (theme_name, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            // (picture, drawer tab, whether the panels are open, the window's width)
            let shots: [(&str, Option<DrawerTab>, bool, f32); 8] = [
                ("inputs", Some(DrawerTab::Inputs), false, 960.0),
                ("report", Some(DrawerTab::Report), false, 960.0),
                ("problems", Some(DrawerTab::Problems), false, 960.0),
                ("logs", Some(DrawerTab::Logs), false, 960.0),
                ("profile", Some(DrawerTab::Profile), false, 960.0),
                ("panels", Some(DrawerTab::Inputs), true, 960.0),
                ("panels-narrow", Some(DrawerTab::Inputs), true, 760.0),
                ("no-drawer", None, false, 960.0),
            ];
            for (name, tab, panels, width) in shots {
                let mut rig = Rig::new(&format!("pseudo-{theme_name}-{name}"));
                let mut screen = Screen::new(vec2(width, size.y));
                theme::apply(&screen.ctx, appearance);
                rig.manager.prefs.drawer_open = tab.is_some();
                if let Some(tab) = tab {
                    rig.manager.prefs.drawer_tab = tab;
                }
                rig.manager.prefs.ai_open = panels;
                rig.manager.prefs.debug_open = panels;
                let output = settle(&mut rig.manager, &mut screen);
                let texts = drawn_texts(&output);
                for word in bare {
                    assert!(
                        !texts.iter().any(|text| text == word),
                        "{word:?} was drawn in English ({theme_name}, {name})"
                    );
                }
                assert!(
                    texts.iter().any(|text| text.starts_with('[')),
                    "no bracketed text in the {name} frame"
                );
                if gallery_dir().is_some() {
                    screen.save(&format!("pseudo-{name}-{theme_name}"), output);
                }
            }
        }
    });
}

/// The strip's menus in the pseudo-locale: each is as wide as its longest row in the longer
/// text, and none of its rows is drawn in English (the pictures are written when
/// `DEGENVIEW_GALLERY_DIR` is set).
#[test]
fn the_studio_menus_draw_in_the_pseudo_locale() {
    use crate::i18n::{Locale, with_locale};
    use crate::ui::theme::{self, AppTheme};
    use debug::studio_tests::Rig;
    use menu_bar::Menu;
    let _theme = theme::RestoreDefaultTheme;
    let size = vec2(960.0, 640.0);
    let bare = [
        "Undo",
        "Redo",
        "Cut",
        "Copy",
        "Paste",
        "Select All",
        "Find",
        "Rename",
        "New Script",
        "Preview Chart",
    ];
    with_locale(Locale::PSEUDO, || {
        for (theme_name, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            for (name, menu) in [
                ("file", Menu::File),
                ("edit", Menu::Edit),
                ("view", Menu::View),
                ("theme", Menu::Theme),
            ] {
                let mut rig = Rig::new(&format!("pseudo-menu-{theme_name}-{name}"));
                let mut screen = Screen::new(size);
                theme::apply(&screen.ctx, appearance);
                rig.manager.prefs.chart_visible = false;
                settle(&mut rig.manager, &mut screen);
                let corner = strip_corner(size, ChartPosition::Bottom);
                click_window(
                    &mut rig.manager,
                    &mut screen,
                    menu_bar::title_centre(menu, corner),
                );
                let output = settle(&mut rig.manager, &mut screen);
                let texts = drawn_texts(&output);
                for word in bare {
                    assert!(
                        !texts.iter().any(|text| text == word),
                        "{word:?} was drawn in English ({theme_name}, {name} menu)"
                    );
                }
                if gallery_dir().is_some() {
                    screen.save(&format!("pseudo-menu-{name}-{theme_name}"), output);
                }
            }
        }
    });
}

/// The script picker and the Inputs tab of a script that has inputs, in the pseudo-locale: the
/// panel's own words are put through `tr!`, the inputs the script declared stay as written (the
/// pictures are written when `DEGENVIEW_GALLERY_DIR` is set).
#[test]
fn the_picker_and_the_inputs_tab_draw_in_the_pseudo_locale() {
    use crate::i18n::{Locale, with_locale};
    use crate::ui::theme::{self, AppTheme};
    use debug::studio_tests::Rig;
    let _theme = theme::RestoreDefaultTheme;
    let size = vec2(960.0, 640.0);
    let source = "//@version=6
indicator(\"Inputs demo\")
length = input.int(14, \"Length\", minval = 1)
src = input.source(close, \"Source\")
frame = input.timeframe(\"D\", \"Timeframe\")
show = input.bool(true, \"Show the line\")
plot(show ? ta.sma(src, length) : na)
";
    with_locale(Locale::PSEUDO, || {
        for (theme_name, appearance) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let mut rig = Rig::new(&format!("pseudo-inputs-{theme_name}"));
            let mut screen = Screen::new(size);
            theme::apply(&screen.ctx, appearance);
            rig.set_source(source);
            rig.manager.prefs.drawer_open = true;
            rig.manager.prefs.drawer_tab = DrawerTab::Inputs;
            let output = settle(&mut rig.manager, &mut screen);
            let texts = drawn_texts(&output);
            assert!(
                !texts.iter().any(|text| text == "Inputs"),
                "the tab was drawn in English ({theme_name})"
            );
            if gallery_dir().is_some() {
                screen.save(&format!("pseudo-inputs-tab-{theme_name}"), output);
            }
            rig.manager.picker.open();
            let output = picker_frames(&mut rig.manager, &mut screen, Vec::new());
            let texts = drawn_texts(&output);
            assert!(
                !texts
                    .iter()
                    .any(|text| text == "Search" || text == "Favorites"),
                "the picker was drawn in English ({theme_name})"
            );
            if gallery_dir().is_some() {
                screen.save(&format!("pseudo-picker-{theme_name}"), output);
            }
        }
    });
}
