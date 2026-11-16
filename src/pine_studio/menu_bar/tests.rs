use super::entry::{self, Entry, Item};
use super::*;
use crate::host::editor_theme::Choices;
use crate::model::editor_theme::{AUTOMATIC_NAME, BuiltInTheme, ThemeSelection, ThemeSource};
use crate::model::pine::script::ScriptType;
use crate::pine_studio::debug::PanelAction;
use crate::pine_studio::debug::marks::{BreakpointMark, GutterEvent, MarkKind};
use crate::pine_studio::editor::diff::ChangeKind;
use crate::pine_studio::editor::status::ProblemsChip;
use crate::pine_studio::editor::{EditAction, breakpoint_rows, change_rows};
use crate::pine_studio::library::prefs::{ChartPosition, DrawerTab};
use crate::pine_studio::profiler::Status;
use crate::pine_studio::rail::RailState;
use crate::ui::shortcut;
use eframe::egui::{Event, PointerButton};

fn file_state() -> FileState {
    FileState {
        id: "script".to_owned(),
        favorite: false,
        script_type: ScriptType::Indicator,
        dirty: true,
    }
}

fn view_state() -> ViewState {
    ViewState {
        panels: RailState::idle(),
        drawer_open: true,
        drawer_tab: DrawerTab::Inputs,
    }
}

/// Everything on: a selection, history both ways, a name with a declaration and calls, a
/// breakpoint and a change on the caret's line.
fn busy_editor() -> EditorMenuState {
    EditorMenuState {
        can_undo: true,
        can_redo: true,
        selection: true,
        has_text: true,
        can_step_matches: true,
        can_toggle_comment: true,
        can_move_up: true,
        can_move_down: true,
        definition: true,
        usages: true,
        rename: true,
        line: 7,
        breakpoint: Some(BreakpointMark {
            line: 7,
            kind: MarkKind::Line,
            enabled: true,
            unverified: false,
        }),
        breakpoints: 2,
        change: Some(ChangeKind::Modified),
    }
}

/// The rows of `entries` and of the submenus in them, in order.
fn items(entries: &[Entry]) -> Vec<&Item> {
    entries
        .iter()
        .flat_map(|entry| match entry {
            Entry::Item(item) => vec![item],
            Entry::Submenu(submenu) => items(&submenu.entries),
            Entry::Scroll(rows) => items(rows),
            Entry::Separator => Vec::new(),
        })
        .collect()
}

fn item<'a>(entries: &'a [Entry], label: &str) -> &'a Item {
    items(entries)
        .into_iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("no row {label}"))
}

/// The menu as it reads: labels in order, `|` for a separator, `label >` for a submenu.
fn outline(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match entry {
            Entry::Item(item) => item.label.clone(),
            Entry::Separator => "|".to_owned(),
            Entry::Submenu(submenu) => format!("{} >", submenu.label),
            Entry::Scroll(rows) => format!("[{}]", outline(rows).join(", ")),
        })
        .collect()
}

fn submenu<'a>(entries: &'a [Entry], label: &str) -> &'a [Entry] {
    entries
        .iter()
        .find_map(|entry| match entry {
            Entry::Submenu(submenu) if submenu.label == label => Some(submenu.entries.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no submenu {label}"))
}

// MARK: File

#[test]
fn the_file_menu_lists_new_open_and_save_then_what_can_be_done_to_the_script() {
    let entries = file::entries(&file_state());
    let show_in = format!("Show in {}", crate::platform::file_manager());
    assert_eq!(
        outline(&entries),
        [
            "New Script…",
            "Open Script…",
            "Save",
            "|",
            "Favorite",
            "Rename…",
            "Duplicate",
            "|",
            &show_in,
            "Export…",
            "|",
            "Add to Chart",
            "Create Alert",
            "|",
            "Add Example Scripts",
            "|",
            "Delete…",
        ]
    );
}

#[test]
fn file_rows_ask_for_their_commands_on_the_open_script() {
    let entries = file::entries(&file_state());
    assert!(matches!(
        item(&entries, "New Script…").command,
        Cmd::NewScript
    ));
    assert!(matches!(
        item(&entries, "Open Script…").command,
        Cmd::OpenScriptPicker
    ));
    assert!(matches!(item(&entries, "Save").command, Cmd::Save));
    assert!(matches!(
        &item(&entries, "Favorite").command,
        Cmd::ToggleFavorite(id) if id == "script"
    ));
    assert!(matches!(
        &item(&entries, "Rename…").command,
        Cmd::BeginRename(id) if id == "script"
    ));
    assert!(matches!(
        item(&entries, "Duplicate").command,
        Cmd::Duplicate(_)
    ));
    assert!(matches!(item(&entries, "Export…").command, Cmd::Export(_)));
    assert!(matches!(
        item(&entries, "Add to Chart").command,
        Cmd::AddToChart(_)
    ));
    assert!(matches!(
        item(&entries, "Create Alert").command,
        Cmd::CreateAlert(_)
    ));
    assert!(matches!(
        item(&entries, "Add Example Scripts").command,
        Cmd::AddExamples
    ));
    let delete = item(&entries, "Delete…");
    assert!(matches!(delete.command, Cmd::AskDelete(_)));
    assert!(delete.destructive);
    let reveal = items(&entries)
        .into_iter()
        .find(|item| item.label.starts_with("Show in"))
        .expect("a row that shows the file");
    assert!(matches!(reveal.command, Cmd::RevealFile(_)));
}

#[test]
fn file_rows_show_the_chords_and_a_saved_script_has_nothing_to_save() {
    let entries = file::entries(&file_state());
    assert_eq!(
        item(&entries, "Save").shortcut.as_deref(),
        Some(shortcut::SAVE.label().as_str())
    );
    assert_eq!(
        item(&entries, "Open Script…").shortcut.as_deref(),
        Some(shortcut::OPEN_SCRIPT_PICKER.label().as_str())
    );
    assert!(item(&entries, "Save").enabled);
    let saved = file::entries(&FileState {
        dirty: false,
        ..file_state()
    });
    assert!(!item(&saved, "Save").enabled);
}

#[test]
fn a_favorite_can_be_removed_and_a_library_has_no_chart_or_alert() {
    let entries = file::entries(&FileState {
        favorite: true,
        script_type: ScriptType::Library,
        ..file_state()
    });
    let labels = outline(&entries);
    assert!(labels.contains(&"Remove Favorite".to_owned()));
    assert!(!labels.contains(&"Add to Chart".to_owned()));
    assert!(!labels.contains(&"Create Alert".to_owned()));
}

// MARK: Edit

#[test]
fn the_edit_menu_groups_history_clipboard_find_lines_names_and_the_carets_line() {
    let entries = edit::entries(&busy_editor());
    assert_eq!(
        outline(&entries),
        [
            "Undo",
            "Redo",
            "|",
            "Cut",
            "Copy",
            "Paste",
            "Select All",
            "|",
            "Find…",
            "Find Next",
            "Find Previous",
            "|",
            "Toggle Comment",
            "Duplicate Line Up",
            "Duplicate Line Down",
            "Move Line Up",
            "Move Line Down",
            "|",
            "Go to Definition",
            "Find Usages",
            "Rename Symbol…",
            "|",
            "Breakpoint >",
            "Change >",
        ]
    );
}

#[test]
fn every_edit_action_has_a_row_with_the_chord_of_its_shortcut() {
    let entries = edit::entries(&busy_editor());
    let rows = [
        ("Undo", EditAction::Undo, Some(shortcut::UNDO)),
        ("Redo", EditAction::Redo, Some(shortcut::REDO)),
        ("Cut", EditAction::Cut, Some(shortcut::CUT)),
        ("Copy", EditAction::Copy, Some(shortcut::COPY)),
        ("Paste", EditAction::Paste, Some(shortcut::PASTE)),
        (
            "Select All",
            EditAction::SelectAll,
            Some(shortcut::SELECT_ALL),
        ),
        ("Find…", EditAction::Find, Some(shortcut::FIND)),
        ("Find Next", EditAction::FindNext, Some(shortcut::FIND_NEXT)),
        (
            "Find Previous",
            EditAction::FindPrevious,
            Some(shortcut::FIND_PREVIOUS),
        ),
        (
            "Toggle Comment",
            EditAction::ToggleComment,
            Some(shortcut::TOGGLE_COMMENT),
        ),
        (
            "Duplicate Line Up",
            EditAction::DuplicateLineUp,
            Some(shortcut::DUPLICATE_LINE_UP),
        ),
        (
            "Duplicate Line Down",
            EditAction::DuplicateLineDown,
            Some(shortcut::DUPLICATE_LINE_DOWN),
        ),
        (
            "Move Line Up",
            EditAction::MoveLineUp,
            Some(shortcut::MOVE_LINE_UP),
        ),
        (
            "Move Line Down",
            EditAction::MoveLineDown,
            Some(shortcut::MOVE_LINE_DOWN),
        ),
        (
            "Go to Definition",
            EditAction::GoToDefinition,
            Some(shortcut::GO_TO_DEFINITION),
        ),
        ("Find Usages", EditAction::FindUsages, None),
        (
            "Rename Symbol…",
            EditAction::Rename,
            Some(shortcut::RENAME_SYMBOL),
        ),
    ];
    for (label, action, chord) in rows {
        let row = item(&entries, label);
        assert!(
            matches!(row.command, Cmd::Edit(asked) if asked == action),
            "{label}"
        );
        assert_eq!(row.shortcut, chord.map(|chord| chord.label()), "{label}");
        assert!(row.enabled, "{label} is on when everything applies");
    }
}

#[test]
fn with_nothing_to_act_on_only_what_needs_nothing_is_on() {
    let entries = edit::entries(&EditorMenuState {
        line: 1,
        ..EditorMenuState::default()
    });
    let on: Vec<_> = items(&entries)
        .into_iter()
        .filter(|item| item.enabled)
        .map(|item| item.label.as_str())
        .collect();
    assert_eq!(
        on,
        [
            "Paste",
            "Find…",
            "Add Breakpoint",
            "Add Conditional Breakpoint…",
            "Add Logpoint…"
        ]
    );
    let change = entries.iter().find_map(|entry| match entry {
        Entry::Submenu(submenu) if submenu.label == "Change" => Some(submenu),
        _ => None,
    });
    assert!(change.is_some_and(|change| !change.enabled && change.entries.is_empty()));
}

#[test]
fn the_breakpoint_submenu_is_the_gutters_menu_for_the_carets_line() {
    for mark in [
        None,
        Some(BreakpointMark {
            line: 7,
            kind: MarkKind::Line,
            enabled: false,
            unverified: false,
        }),
    ] {
        let state = EditorMenuState {
            line: 7,
            breakpoint: mark,
            ..EditorMenuState::default()
        };
        let entries = edit::entries(&state);
        let rows = items(submenu(&entries, "Breakpoint"));
        let wanted = breakpoint_rows(7, mark.as_ref());
        assert_eq!(rows.len(), wanted.len());
        for (row, wanted) in rows.iter().zip(&wanted) {
            assert_eq!(row.label, wanted.label);
            assert_eq!(row.destructive, wanted.destructive);
            assert!(matches!(&row.command, Cmd::Gutter(event) if *event == wanted.event));
        }
    }
}

#[test]
fn the_breakpoint_that_is_off_offers_to_enable_and_the_toggle_has_its_chord() {
    let state = EditorMenuState {
        line: 3,
        breakpoint: Some(BreakpointMark {
            line: 3,
            kind: MarkKind::Line,
            enabled: false,
            unverified: false,
        }),
        ..EditorMenuState::default()
    };
    let entries = edit::entries(&state);
    assert!(matches!(
        item(&entries, "Enable Breakpoint").command,
        Cmd::Gutter(GutterEvent::SetEnabled {
            line: 3,
            enabled: true
        })
    ));
    let remove = item(&entries, "Remove Breakpoint");
    assert_eq!(
        remove.shortcut,
        Some(shortcut::DEBUG_TOGGLE_BREAKPOINT.label())
    );
}

#[test]
fn remove_all_breakpoints_is_offered_only_when_there_are_some() {
    let none = edit::entries(&EditorMenuState {
        line: 1,
        ..EditorMenuState::default()
    });
    assert!(
        !items(&none)
            .iter()
            .any(|item| item.label == "Remove All Breakpoints")
    );
    let some = edit::entries(&busy_editor());
    let all = item(&some, "Remove All Breakpoints");
    assert!(matches!(
        all.command,
        Cmd::DebugPanel(PanelAction::RemoveAllBreakpoints)
    ));
    assert!(all.destructive);
}

#[test]
fn the_change_submenu_is_the_change_columns_menu_for_each_kind_of_change() {
    for kind in [ChangeKind::Added, ChangeKind::Deleted, ChangeKind::Modified] {
        let state = EditorMenuState {
            change: Some(kind),
            line: 1,
            ..EditorMenuState::default()
        };
        let entries = edit::entries(&state);
        let rows = items(submenu(&entries, "Change"));
        let wanted = change_rows(kind);
        assert_eq!(rows.len(), wanted.len(), "{kind:?}");
        for (row, wanted) in rows.iter().zip(&wanted) {
            assert_eq!(row.label, wanted.label);
            assert!(
                matches!(&row.command, Cmd::Edit(EditAction::Change(command)) if *command == wanted.command)
            );
        }
    }
}

// MARK: View

#[test]
fn the_view_menu_lists_the_panels_the_preview_chart_and_the_drawer() {
    let entries = view::entries(&view_state());
    assert_eq!(
        outline(&entries),
        [
            "Assistant",
            "Profiler",
            "Debugger",
            "|",
            "Preview Chart >",
            "Hide Preview Chart",
            "|",
            "Hide Drawer",
            "Drawer Tab >",
        ]
    );
    assert!(matches!(
        item(&entries, "Assistant").command,
        Cmd::ToggleAiPanel
    ));
    assert!(matches!(
        item(&entries, "Profiler").command,
        Cmd::ProfilerToggle
    ));
    assert!(matches!(
        item(&entries, "Debugger").command,
        Cmd::ToggleDebugPanel
    ));
    assert!(matches!(
        item(&entries, "Hide Preview Chart").command,
        Cmd::ToggleChart
    ));
    assert!(matches!(
        item(&entries, "Hide Drawer").command,
        Cmd::ToggleDrawer
    ));
    assert_eq!(
        item(&entries, "Profiler").shortcut,
        Some(shortcut::PROFILER_TOGGLE.label())
    );
    assert_eq!(
        item(&entries, "Hide Preview Chart").shortcut,
        Some(shortcut::TOGGLE_PREVIEW_CHART.label())
    );
}

#[test]
fn the_view_menu_marks_what_is_open_and_flips_its_show_and_hide_labels() {
    let mut state = view_state();
    let entries = view::entries(&state);
    for label in ["Assistant", "Profiler", "Debugger"] {
        assert!(!item(&entries, label).checked, "{label}");
    }
    state.panels.assistant_open = true;
    state.panels.debugger_open = true;
    state.panels.profiler = Status::Idle;
    state.panels.chart_visible = false;
    state.drawer_open = false;
    let entries = view::entries(&state);
    for label in ["Assistant", "Profiler", "Debugger"] {
        assert!(item(&entries, label).checked, "{label}");
    }
    let labels = outline(&entries);
    assert!(labels.contains(&"Show Preview Chart".to_owned()));
    assert!(labels.contains(&"Show Drawer".to_owned()));
}

#[test]
fn the_position_and_drawer_tab_submenus_check_the_current_choice_and_ask_to_change_it() {
    let mut state = view_state();
    state.panels.chart_position = ChartPosition::Bottom;
    state.drawer_tab = DrawerTab::Logs;
    let entries = view::entries(&state);
    let positions = items(submenu(&entries, "Preview Chart"));
    let titles: Vec<_> = positions.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(titles, ["Left of Code", "Above Code", "Below Code"]);
    let checked: Vec<_> = positions.iter().map(|row| row.checked).collect();
    assert_eq!(checked, [false, false, true]);
    assert!(matches!(
        positions[1].command,
        Cmd::SetChartPosition(ChartPosition::Top)
    ));

    let tabs = items(submenu(&entries, "Drawer Tab"));
    assert_eq!(tabs.len(), DrawerTab::ALL.len());
    for (row, tab) in tabs.iter().zip(DrawerTab::ALL) {
        assert_eq!(row.label, tab.title());
        assert_eq!(row.checked, tab == DrawerTab::Logs);
        assert!(matches!(row.command, Cmd::SetDrawerTab(asked) if asked == tab));
    }
}

// MARK: Theme

fn choices() -> Choices {
    Choices {
        selected: ThemeSelection::Auto,
        customs: vec![(1_730_000_000_001, "My Dracula".to_owned())],
    }
}

#[test]
fn the_theme_menu_lists_what_settings_lists_then_the_way_to_settings() {
    let entries = themes::entries(&choices());
    assert_eq!(
        outline(&entries),
        [
            "[Automatic, Dark, Light, Dracula, |, My Dracula]",
            "|",
            "Configure Themes…"
        ]
    );
    let names: Vec<_> = items(&entries)
        .into_iter()
        .map(|item| item.label.as_str())
        .collect();
    let mut settings = vec![AUTOMATIC_NAME];
    settings.extend(BuiltInTheme::ALL.map(BuiltInTheme::name));
    assert_eq!(&names[..4], settings);
}

#[test]
fn with_no_themes_of_ones_own_the_menu_has_no_second_group() {
    let entries = themes::entries(&Choices {
        selected: ThemeSelection::Auto,
        customs: Vec::new(),
    });
    assert_eq!(
        outline(&entries),
        [
            "[Automatic, Dark, Light, Dracula]",
            "|",
            "Configure Themes…"
        ]
    );
}

#[test]
fn the_theme_worn_is_the_only_one_checked_and_every_row_asks_to_wear_its_theme() {
    let worn = [
        ThemeSelection::Auto,
        ThemeSelection::BuiltIn(BuiltInTheme::Light),
        ThemeSelection::Custom(1_730_000_000_001),
    ];
    for selected in worn {
        let entries = themes::entries(&Choices {
            selected,
            ..choices()
        });
        let rows = items(&entries);
        let checked: Vec<_> = rows
            .iter()
            .filter(|row| row.checked)
            .map(|row| &row.command)
            .collect();
        assert!(
            matches!(checked.as_slice(), [Cmd::SetEditorTheme(chosen)] if *chosen == selected),
            "{selected:?}"
        );
    }
    let entries = themes::entries(&choices());
    assert!(matches!(
        item(&entries, "Dracula").command,
        Cmd::SetEditorTheme(ThemeSelection::BuiltIn(BuiltInTheme::Dracula))
    ));
    assert!(matches!(
        item(&entries, "My Dracula").command,
        Cmd::SetEditorTheme(ThemeSelection::Custom(1_730_000_000_001))
    ));
    assert!(matches!(
        item(&entries, "Configure Themes…").command,
        Cmd::OpenThemeSettings
    ));
    assert!(!item(&entries, "Configure Themes…").checked);
}

#[test]
fn a_long_list_of_themes_scrolls_instead_of_growing_past_the_menu_height() {
    let _published = crate::ui::editor_appearance::RestorePublished;
    let mut strip = Strip::new();
    for index in 0..60 {
        let name = format!("Theme {index} named as long as a name may be");
        strip
            .state
            .themes
            .create(&name, ThemeSource::BuiltIn(BuiltInTheme::Dark))
            .expect("a theme");
    }
    let theme = strip.at(Menu::Theme);
    strip.click(theme);
    let (_, popup) = *strip
        .popups
        .iter()
        .find(|(menu, _)| *menu == Menu::Theme)
        .unwrap();
    let rect = strip
        .ctx
        .memory(|memory| memory.area_rect(popup))
        .expect("the menu is up");
    assert!(
        rect.height() <= kit::MENU_MAX_HEIGHT + 60.0,
        "{} points tall",
        rect.height()
    );
    assert!(rect.width() <= 360.0 + 1.0, "{} points wide", rect.width());
    assert!(rect.height() > 200.0, "{} points tall", rect.height());
}

// MARK: Sizing

/// `entries` with the rows of every scrolling block in place of the block.
fn flat(entries: &[Entry]) -> Vec<&Entry> {
    entries
        .iter()
        .flat_map(|entry| match entry {
            Entry::Scroll(rows) => flat(rows),
            other => vec![other],
        })
        .collect()
}

/// What a row needs to be seen whole: the icon's column, the label, the shortcut and a gap before
/// it, and room at the end (a submenu's chevron takes more).
fn needed(ui: &Ui, label: &str, shortcut: Option<&str>, submenu: bool) -> f32 {
    let text = |text: &str| {
        kit::galley(
            ui,
            text,
            size::BODY,
            kit::REGULAR,
            theme::text_primary(),
            f32::INFINITY,
        )
        .size()
        .x
    };
    let end = if submenu { 8.0 + 22.0 } else { 10.0 };
    32.0 + text(label) + shortcut.map_or(0.0, |hint| text(hint) + 12.0) + end
}

#[test]
fn each_menu_is_as_wide_as_its_longest_row_and_no_wider() {
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let mut view = view_state();
    view.panels.profiler = Status::Idle;
    let menus = [
        ("file", file::entries(&file_state())),
        ("edit", edit::entries(&busy_editor())),
        ("view", view::entries(&view)),
        ("theme", themes::entries(&choices())),
    ];
    let mut checked = 0;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        for (name, entries) in &menus {
            let measured = entry::measured(entries);
            let width = kit::menu_width_for_rows(ui, &entry::as_rows(&measured), 0.0)
                - kit::MENU_FRAME_MARGIN;
            let mut widest: f32 = 0.0;
            for entry in flat(entries) {
                let need = match entry {
                    Entry::Item(item) => needed(ui, &item.label, item.shortcut.as_deref(), false),
                    Entry::Submenu(submenu) => needed(ui, &submenu.label, None, true),
                    Entry::Separator | Entry::Scroll(_) => continue,
                };
                assert!(
                    need <= width,
                    "{name}: a row needs {need}, the menu is {width}"
                );
                widest = widest.max(need);
                checked += 1;
            }
            assert!(
                width - widest < 6.0,
                "{name}: {} points of blank space after the longest row",
                width - widest
            );
        }
    });
    output.textures_delta.clear();
    assert!(checked > 30);
}

#[test]
fn a_submenus_rows_are_measured_with_their_own_shortcuts() {
    let entries = edit::entries(&busy_editor());
    let rows = entry::measured(submenu(&entries, "Breakpoint"));
    assert_eq!(rows[0].0, "Edit Breakpoint…");
    assert_eq!(rows.iter().filter(|(_, hint)| hint.is_some()).count(), 1);
}

// MARK: The strip

struct Strip {
    ctx: egui::Context,
    time: f64,
    state: MenuBarState,
    editor: EditorMenuState,
    cmds: RefCell<Vec<Cmd>>,
    /// egui's id for the menu of each title.
    popups: Vec<(Menu, Id)>,
    editor_asked: std::cell::Cell<u32>,
}

impl Strip {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let mut strip = Self {
            ctx,
            time: 0.0,
            state: MenuBarState {
                editor_id: Id::new("test-editor"),
                file: file_state(),
                view: view_state(),
                themes: EditorThemeHost::default(),
                problems: ProblemsChip::of(&crate::pine_studio::editor::EditorState::new(
                    "s", "", "",
                )),
            },
            editor: busy_editor(),
            cmds: RefCell::new(Vec::new()),
            popups: Vec::new(),
            editor_asked: std::cell::Cell::new(0),
        };
        strip.frame(Vec::new());
        strip
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.1;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (state, editor, asked) = (&self.state, &self.editor, &self.editor_asked);
        let mut popups = Vec::new();
        let mut chosen = Vec::new();
        let editor_id = state.editor_id;
        let mut text = String::from("text");
        let mut output = self.ctx.run_ui(raw, |ui| {
            egui::Panel::top("pine-studio-menu-bar")
                .exact_size(HEIGHT)
                .show(ui, |ui| {
                    for menu in Menu::ALL {
                        popups.push((menu, title_id(ui, menu).with("popup")));
                    }
                    chosen = titles_and_menus(ui, state, || {
                        asked.set(asked.get() + 1);
                        editor.clone()
                    });
                });
            // A field of the editor's id under the strip, which lets go of the keyboard when a
            // click lands elsewhere, as the editor's does.
            ui.add(egui::TextEdit::multiline(&mut text).id(editor_id));
        });
        output.textures_delta.clear();
        self.popups = popups;
        self.cmds.borrow_mut().extend(chosen);
    }

    fn at(&self, menu: Menu) -> egui::Pos2 {
        title_centre(menu, egui::Pos2::ZERO)
    }

    fn open(&self) -> Option<Menu> {
        self.popups
            .iter()
            .find(|(_, id)| Popup::is_id_open(&self.ctx, *id))
            .map(|(menu, _)| *menu)
    }

    fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }

    fn hover(&mut self, at: egui::Pos2) {
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(Vec::new());
    }
}

#[test]
fn a_click_on_a_title_opens_its_menu_and_a_second_click_closes_it() {
    let mut strip = Strip::new();
    assert_eq!(strip.open(), None);
    let file = strip.at(Menu::File);
    strip.click(file);
    assert_eq!(strip.open(), Some(Menu::File));
    strip.click(file);
    assert_eq!(strip.open(), None);
}

#[test]
fn pointing_at_another_title_while_a_menu_is_up_opens_that_one() {
    let mut strip = Strip::new();
    let edit = strip.at(Menu::Edit);
    strip.hover(edit);
    assert_eq!(
        strip.open(),
        None,
        "nothing is up, so pointing opens nothing"
    );
    let file = strip.at(Menu::File);
    strip.click(file);
    assert_eq!(strip.open(), Some(Menu::File));
    strip.hover(edit);
    assert_eq!(strip.open(), Some(Menu::Edit));
    let view = strip.at(Menu::View);
    strip.hover(view);
    assert_eq!(strip.open(), Some(Menu::View));
    let theme = strip.at(Menu::Theme);
    strip.hover(theme);
    assert_eq!(strip.open(), Some(Menu::Theme));
    strip.hover(file);
    assert_eq!(strip.open(), Some(Menu::File));
}

#[test]
fn escape_closes_the_menu() {
    let mut strip = Strip::new();
    let edit = strip.at(Menu::Edit);
    strip.click(edit);
    assert_eq!(strip.open(), Some(Menu::Edit));
    strip.frame(vec![Event::Key {
        key: egui::Key::Escape,
        physical_key: Some(egui::Key::Escape),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    strip.frame(Vec::new());
    assert_eq!(strip.open(), None);
}

#[test]
fn the_editors_state_is_asked_for_only_while_the_edit_menu_is_up() {
    let mut strip = Strip::new();
    let file = strip.at(Menu::File);
    strip.click(file);
    let view = strip.at(Menu::View);
    strip.hover(view);
    assert_eq!(
        strip.editor_asked.get(),
        0,
        "no copy of the undo history for File or View"
    );
    let edit = strip.at(Menu::Edit);
    strip.hover(edit);
    assert!(strip.editor_asked.get() > 0);
}

#[test]
fn choosing_a_row_asks_for_its_command_and_closes_the_menu() {
    let mut strip = Strip::new();
    let file = strip.at(Menu::File);
    strip.click(file);
    // The first row is New Script…, under the title and the menu's margin.
    strip.click(egui::pos2(file.x + 20.0, HEIGHT + 4.0 + 5.0 + 12.0));
    assert!(matches!(strip.cmds.borrow().as_slice(), [Cmd::NewScript]));
    assert_eq!(strip.open(), None);
}

#[test]
fn choosing_a_theme_or_configure_in_the_theme_menu_asks_for_it_and_closes_the_menu() {
    let mut strip = Strip::new();
    let theme = strip.at(Menu::Theme);
    strip.click(theme);
    assert_eq!(strip.open(), Some(Menu::Theme));
    // The second row is Dark: Automatic is first, under the title and the menu's margin.
    let first = HEIGHT + 4.0 + 5.0 + 12.0;
    strip.click(egui::pos2(theme.x, first + 24.0));
    assert!(matches!(
        strip.cmds.borrow().as_slice(),
        [Cmd::SetEditorTheme(ThemeSelection::BuiltIn(
            BuiltInTheme::Dark
        ))]
    ));
    assert_eq!(strip.open(), None);

    strip.cmds.borrow_mut().clear();
    strip.click(theme);
    // Configure Themes… comes after the four themes and a separator.
    strip.click(egui::pos2(theme.x, first + 4.0 * 24.0 + 9.0));
    assert!(matches!(
        strip.cmds.borrow().as_slice(),
        [Cmd::OpenThemeSettings]
    ));
    assert_eq!(strip.open(), None);
}

#[test]
fn a_disabled_row_asks_for_nothing() {
    let mut strip = Strip::new();
    strip.state.file.dirty = false;
    let file = strip.at(Menu::File);
    strip.click(file);
    // Save is the third row.
    strip.click(egui::pos2(
        file.x + 20.0,
        HEIGHT + 4.0 + 5.0 + 2.0 * 24.0 + 12.0,
    ));
    assert!(strip.cmds.borrow().is_empty());
}

#[test]
fn the_editor_keeps_the_keyboard_while_a_menu_is_up_if_it_had_it() {
    let mut strip = Strip::new();
    let editor = strip.state.editor_id;
    strip.ctx.memory_mut(|memory| memory.request_focus(editor));
    strip.frame(Vec::new());
    let edit = strip.at(Menu::Edit);
    strip.click(edit);
    assert_eq!(strip.open(), Some(Menu::Edit));
    assert!(strip.ctx.memory(|memory| memory.has_focus(editor)));
}

#[test]
fn a_menu_opened_while_something_else_has_the_keyboard_leaves_it_there() {
    let mut strip = Strip::new();
    let other = Id::new("the assistant's input");
    strip.ctx.memory_mut(|memory| memory.request_focus(other));
    strip.frame(Vec::new());
    let edit = strip.at(Menu::Edit);
    strip.click(edit);
    assert_eq!(strip.open(), Some(Menu::Edit));
    assert!(
        !strip
            .ctx
            .memory(|memory| memory.has_focus(strip.state.editor_id))
    );
}
