use super::*;
use crate::model::editor_theme::{ColorKey, Rgba, SyntaxRole, ThemeSource};
use crate::ui::editor_appearance::{Appearance, RestorePublished, current, editor_font};
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::theme::{AppTheme, RestoreDefaultTheme};
use eframe::egui::{Event, Modifiers, PointerButton, Vec2};

/// The page's room in the Settings window: the window less its sidebar.
const CONTENT: Vec2 = Vec2::new(
    crate::ui::settings_window::WINDOW_SIZE.x - crate::ui::settings_window::SIDEBAR_WIDTH,
    crate::ui::settings_window::WINDOW_SIZE.y,
);

struct Rig {
    screen: Screen,
    state: ScriptManagerState,
    host: EditorThemeHost,
}

impl Rig {
    fn new(size: Vec2) -> Self {
        let screen = Screen::new(size);
        let host = EditorThemeHost::default();
        host.attach(&screen.ctx);
        Self {
            screen,
            state: ScriptManagerState::default(),
            host,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        let Self {
            screen,
            state,
            host,
        } = self;
        screen.run(events, |ui| {
            page(ui, SettingsSection::ScriptManager, state, host);
        })
    }

    fn settle(&mut self) -> egui::FullOutput {
        let mut last = self.frame(Vec::new());
        for _ in 0..3 {
            last = self.frame(Vec::new());
        }
        last
    }
}

fn press(key: Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

fn click_at(position: egui::Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(position),
        Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]
}

#[test]
fn the_page_is_in_the_sidebar_after_appearance() {
    use crate::ui::settings_window::SettingsSection as S;
    assert_eq!(S::ALL.len(), 12);
    let visible: Vec<_> = S::visible().collect();
    assert_eq!(visible[1], S::ScriptManager);
    assert_eq!(S::ScriptManager.title(), "Script Manager");
    assert_eq!(S::from_key(S::ScriptManager.key()), Some(S::ScriptManager));
    // The sidebar still fits the smallest window Settings can be (400 points tall).
    assert!(10.0 + 32.0 * visible.len() as f32 <= 400.0);
}

#[test]
fn the_page_draws_at_every_size_in_both_appearances() {
    for app_theme in [AppTheme::Dark, AppTheme::Light] {
        let _theme = RestoreDefaultTheme;
        let _published = RestorePublished;
        for size in [
            vec2(320.0, 400.0),
            CONTENT,
            vec2(700.0, 520.0),
            vec2(1100.0, 900.0),
        ] {
            let mut rig = Rig::new(size);
            theme::apply(&rig.screen.ctx, app_theme);
            rig.host
                .create(
                    "A theme with a rather long name indeed",
                    ThemeSource::BuiltIn(BuiltInTheme::Dracula),
                )
                .unwrap();
            rig.settle();
            // Everything is reachable: the page scrolls, so nothing is wider than the page.
            let mut width = 0.0f32;
            let Rig {
                screen,
                state,
                host,
            } = &mut rig;
            let _ = screen.run(Vec::new(), |ui| {
                let before = ui.max_rect();
                page(ui, SettingsSection::ScriptManager, state, host);
                width = ui.min_rect().width().max(before.width());
            });
            assert!(width <= size.x + 0.5, "{app_theme:?} at {size:?}: {width}");
        }
    }
}

#[test]
fn choosing_a_tile_wears_the_theme() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    rig.settle();
    let view = rig.host.view();
    apply(
        TileAction::Select(ThemeSelection::BuiltIn(BuiltInTheme::Dracula)),
        &mut rig.state,
        &rig.host,
        &view,
    );
    assert_eq!(
        rig.host.view().selected,
        ThemeSelection::BuiltIn(BuiltInTheme::Dracula)
    );
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dracula));
    rig.settle();
}

#[test]
fn the_tiles_are_automatic_then_the_built_ins_then_your_own() {
    let host = EditorThemeHost::default();
    host.create("Mine", ThemeSource::BuiltIn(BuiltInTheme::Light))
        .unwrap();
    let view = host.view();
    let names: Vec<_> = tiles(&view).iter().map(|tile| tile.name.clone()).collect();
    assert_eq!(names, ["Automatic", "Dark", "Light", "Dracula", "Mine"]);
    let all = tiles(&view);
    assert_eq!(all[0].palettes.len(), 2, "automatic shows both appearances");
    assert!(all[1..].iter().all(|tile| tile.palettes.len() == 1));
    assert_eq!(all[4].caption, "Custom");
    assert_eq!(all[1].caption, "Built-in");
    assert!(all[4].custom.is_some() && all[1].custom.is_none());
}

#[test]
fn duplicate_and_customize_opens_an_editable_copy_and_leaves_the_original_alone() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    let view = rig.host.view();
    apply(
        TileAction::Customize(ThemeSelection::BuiltIn(BuiltInTheme::Dracula)),
        &mut rig.state,
        &rig.host,
        &view,
    );
    let view = rig.host.view();
    assert_eq!(view.customs.len(), 1);
    assert_eq!(view.customs[0].theme.name, "Dracula copy");
    assert_eq!(view.customs[0].base, BuiltInTheme::Dracula);
    assert!(matches!(rig.state.mode, Mode::Editing(_)));
    assert_eq!(
        view.selected,
        ThemeSelection::Custom(view.customs[0].theme.id)
    );
    // The built-in itself is code: it is what it was.
    assert_eq!(
        BuiltInTheme::Dracula.palette(),
        BuiltInTheme::Dracula.palette()
    );
    rig.settle();
}

#[test]
fn customizing_automatic_copies_the_built_in_that_matches_the_app() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    theme::apply(&rig.screen.ctx, AppTheme::Light);
    rig.settle();
    let view = rig.host.view();
    apply(
        TileAction::Customize(ThemeSelection::Auto),
        &mut rig.state,
        &rig.host,
        &view,
    );
    assert_eq!(rig.host.view().customs[0].base, BuiltInTheme::Light);
    assert_eq!(rig.host.view().customs[0].theme.name, "Light copy");
}

#[test]
fn creating_a_theme_from_the_sheet_validates_and_opens_the_editor() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    rig.state.dialog = Some(Dialog::Create {
        name: String::new(),
        source: 2,
        focus: true,
        error: None,
    });
    rig.frame(Vec::new());
    // Return with no name does nothing.
    rig.frame(press(Key::Enter));
    assert!(rig.host.view().customs.is_empty());
    assert!(rig.state.dialog.is_some());
    // A name that is a built-in's is refused, with the reason on the sheet.
    rig.state.dialog = Some(Dialog::Create {
        name: "dracula".into(),
        source: 0,
        focus: false,
        error: None,
    });
    rig.frame(press(Key::Enter));
    assert!(rig.host.view().customs.is_empty());
    assert!(rig.host.name_problem("dracula", None).is_some());
    // A good one, based on the third choice (Dracula), makes the theme and opens it.
    rig.state.dialog = Some(Dialog::Create {
        name: "  My Night  ".into(),
        source: 2,
        focus: false,
        error: None,
    });
    rig.frame(press(Key::Enter));
    let view = rig.host.view();
    assert_eq!(view.customs.len(), 1);
    assert_eq!(view.customs[0].theme.name, "My Night");
    assert_eq!(view.customs[0].base, BuiltInTheme::Dracula);
    assert!(rig.state.dialog.is_none());
    assert!(matches!(rig.state.mode, Mode::Editing(_)));
    assert_eq!(
        view.selected,
        ThemeSelection::Custom(view.customs[0].theme.id)
    );
}

#[test]
fn escape_cancels_the_sheet_and_changes_nothing() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    rig.state.dialog = Some(Dialog::Create {
        name: "Typed but cancelled".into(),
        source: 0,
        focus: false,
        error: None,
    });
    rig.frame(Vec::new());
    rig.frame(press(Key::Escape));
    assert!(rig.state.dialog.is_none());
    assert!(rig.host.view().customs.is_empty());
    assert!(matches!(rig.state.mode, Mode::Overview));
}

#[test]
fn renaming_keeps_the_theme_and_refuses_a_taken_name() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    let one = rig
        .host
        .create("One", ThemeSource::BuiltIn(BuiltInTheme::Dark))
        .unwrap();
    rig.host
        .create("Two", ThemeSource::BuiltIn(BuiltInTheme::Dark))
        .unwrap();
    let view = rig.host.view();
    apply(TileAction::Rename(one), &mut rig.state, &rig.host, &view);
    let Some(Dialog::Rename { name, .. }) = &rig.state.dialog else {
        panic!("a rename sheet");
    };
    assert_eq!(name, "One", "it starts with the current name");
    rig.state.dialog = Some(Dialog::Rename {
        id: one,
        name: "two".into(),
        focus: false,
        error: None,
    });
    rig.frame(press(Key::Enter));
    assert_eq!(rig.host.theme(one).unwrap().name, "One");
    rig.state.dialog = Some(Dialog::Rename {
        id: one,
        name: "Uno 🌙".into(),
        focus: false,
        error: None,
    });
    rig.frame(press(Key::Enter));
    assert_eq!(rig.host.theme(one).unwrap().name, "Uno 🌙");
    assert!(rig.state.dialog.is_none());
}

#[test]
fn deleting_the_worn_theme_needs_a_confirmation_and_goes_back_to_the_app() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    let id = rig
        .host
        .create("Doomed", ThemeSource::BuiltIn(BuiltInTheme::Dracula))
        .unwrap();
    let view = rig.host.view();
    apply(TileAction::Delete(id), &mut rig.state, &rig.host, &view);
    assert!(matches!(rig.state.dialog, Some(Dialog::Delete(_))));
    // Looking at the question deletes nothing; Escape answers no.
    rig.frame(Vec::new());
    assert!(rig.host.theme(id).is_some());
    rig.frame(press(Key::Escape));
    assert!(rig.state.dialog.is_none());
    assert!(rig.host.theme(id).is_some(), "cancelled");
    assert_eq!(rig.host.view().selected, ThemeSelection::Custom(id));
    // The host does what Delete Theme asks.
    rig.host.delete(id).unwrap();
    assert_eq!(rig.host.view().selected, ThemeSelection::Auto);
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dark));
}

#[test]
fn a_theme_deleted_while_it_is_open_closes_the_editor() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    let id = rig
        .host
        .create("Gone", ThemeSource::BuiltIn(BuiltInTheme::Light))
        .unwrap();
    rig.state.mode = Mode::Editing(Box::new(Editing::new(rig.host.theme(id).unwrap())));
    rig.settle();
    assert!(matches!(rig.state.mode, Mode::Editing(_)));
    rig.host.delete(id).unwrap();
    rig.settle();
    assert!(matches!(rig.state.mode, Mode::Overview));
}

#[test]
fn leaving_the_page_ends_an_edit_and_closes_a_sheet() {
    let mut state = ScriptManagerState {
        dialog: Some(Dialog::Delete(1)),
        ..Default::default()
    };
    state.leave();
    assert!(state.dialog.is_none());
    assert!(matches!(state.mode, Mode::Overview));
}

#[test]
fn an_import_wears_the_theme_and_says_what_it_skipped() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let host = EditorThemeHost::default();
    let id = host
        .create("Source", ThemeSource::BuiltIn(BuiltInTheme::Dracula))
        .unwrap();
    host.set_color(
        id,
        ColorKey::Syntax(SyntaxRole::Keyword),
        Rgba::hex(0x123456),
    )
    .unwrap();
    let (_, text) = host.export(ThemeSource::Custom(id)).unwrap();
    let damaged = text.replace("\"#123456\"", "\"blue\"");

    let mut state = ScriptManagerState::default();
    let (sender, receiver) = channel();
    sender.send(Ok(Some(damaged))).unwrap();
    state.flow = Some(Flow::Import(receiver));
    poll(&mut state, &host);
    let (message, is_error) = state.status.clone().expect("a result");
    assert!(!is_error, "{message}");
    assert!(message.starts_with("Imported “Source 2”."), "{message}");
    assert!(message.contains("skipped"), "{message}");
    assert_eq!(host.view().customs.len(), 2);
    assert!(state.flow.is_none());

    // A dialog still open leaves the flow in place; a failure is an error, a cancel says nothing.
    let (sender, receiver) = channel::<Result<Option<String>, String>>();
    state.flow = Some(Flow::Import(receiver));
    poll(&mut state, &host);
    assert!(state.flow.is_some(), "still waiting");
    sender.send(Err("could not read it".into())).unwrap();
    poll(&mut state, &host);
    assert_eq!(state.status, Some(("could not read it".to_owned(), true)));
    state.status = None;
    let (sender, receiver) = channel();
    sender.send(Ok(None)).unwrap();
    state.flow = Some(Flow::Import(receiver));
    poll(&mut state, &host);
    assert!(state.status.is_none());
    // A file that is not a theme is an error and adds nothing.
    let (sender, receiver) = channel();
    sender.send(Ok(Some("{\"hello\": 1}".to_owned()))).unwrap();
    state.flow = Some(Flow::Import(receiver));
    poll(&mut state, &host);
    assert!(matches!(&state.status, Some((_, true))));
    assert_eq!(host.view().customs.len(), 2);
}

#[test]
fn an_export_says_where_it_went() {
    let host = EditorThemeHost::default();
    let mut state = ScriptManagerState::default();
    let (sender, receiver) = channel();
    sender
        .send(Ok(Some(PathBuf::from("/tmp/Night.degenview-theme.json"))))
        .unwrap();
    state.flow = Some(Flow::Export(receiver, "Night".into()));
    poll(&mut state, &host);
    assert_eq!(
        state.status,
        Some(("Exported “Night.degenview-theme.json”.".to_owned(), false))
    );
}

#[test]
fn sizes_are_typed_as_people_write_them() {
    assert_eq!(parse_size("14"), Some(14.0));
    assert_eq!(parse_size(" 14 pt "), Some(14.0));
    assert_eq!(parse_size("13,5"), Some(13.5));
    assert_eq!(parse_size("2"), Some(MIN_FONT_SIZE), "clamped up");
    assert_eq!(parse_size("500"), Some(MAX_FONT_SIZE), "clamped down");
    for bad in ["", "big", "1e999", "NaN", "inf", "14 px", "--3"] {
        assert_eq!(parse_size(bad), None, "{bad:?}");
    }
    assert_eq!(format_size(13.0), "13");
    assert_eq!(format_size(13.5), "13.5");
}

#[test]
fn typography_changes_reach_the_editor() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let host = EditorThemeHost::default();
    let mut typography = host.view().typography;
    typography.font_size = 20.0;
    typography.line_height = LineHeight::Loose;
    host.set_typography(typography);
    assert_eq!(editor_font().size, 20.0);
    assert_eq!(editor_font().line_height, LineHeight::Loose);
}

#[test]
fn arrow_keys_move_between_tiles_and_return_picks() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    rig.settle();
    let first = Id::new(("editor-theme-tile", ThemeSelection::Auto.as_text()));
    let second = Id::new((
        "editor-theme-tile",
        ThemeSelection::BuiltIn(BuiltInTheme::Dark).as_text(),
    ));
    let third = Id::new((
        "editor-theme-tile",
        ThemeSelection::BuiltIn(BuiltInTheme::Light).as_text(),
    ));
    rig.screen
        .ctx
        .memory_mut(|memory| memory.request_focus(first));
    rig.frame(Vec::new());
    assert!(
        rig.screen.ctx.memory(|memory| memory.has_focus(first)),
        "focus took"
    );
    rig.frame(press(Key::ArrowRight));
    rig.frame(Vec::new());
    assert!(
        rig.screen.ctx.memory(|memory| memory.has_focus(second)),
        "one step right"
    );
    rig.frame(press(Key::ArrowRight));
    rig.frame(Vec::new());
    assert!(rig.screen.ctx.memory(|memory| memory.has_focus(third)));
    rig.frame(press(Key::ArrowLeft));
    rig.frame(Vec::new());
    assert!(rig.screen.ctx.memory(|memory| memory.has_focus(second)));
    // Return activates the focused tile: Dark is worn.
    rig.frame(press(Key::Enter));
    rig.frame(Vec::new());
    assert_eq!(
        rig.host.view().selected,
        ThemeSelection::BuiltIn(BuiltInTheme::Dark),
        "Return picked the focused tile"
    );
}

#[test]
fn clicking_a_tile_wears_it() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    rig.settle();
    let view = rig.host.view();
    let all = tiles(&view);
    // Find the Dracula tile by laying the grid out the way the page does.
    let width = CONTENT.x - 2.0 * crate::ui::settings_window::INSET - 8.0;
    let columns = (((width + TILE_GAP) / (TILE_MIN_WIDTH + TILE_GAP)).floor() as usize).max(2);
    let tile_width = (width - TILE_GAP * (columns - 1) as f32) / columns as f32;
    let index = all
        .iter()
        .position(|tile| tile.selection == ThemeSelection::BuiltIn(BuiltInTheme::Dracula))
        .unwrap();
    // The tiles start below the header, the sections' first heading, and the page's inset: ask
    // the page where they are by trying the row's cells down the first screen.
    let x = crate::ui::settings_window::INSET
        + (index % columns) as f32 * (tile_width + TILE_GAP)
        + tile_width / 2.0;
    let mut y = 120.0;
    let mut picked = false;
    while y < CONTENT.y && !picked {
        rig.frame(click_at(egui::pos2(x, y)));
        rig.frame(Vec::new());
        picked = rig.host.view().selected == ThemeSelection::BuiltIn(BuiltInTheme::Dracula);
        y += 12.0;
    }
    assert!(picked, "a click in the Dracula tile's column found it");
}

#[test]
fn drawing_the_page_never_changes_the_theme_or_the_script() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(CONTENT);
    let before = rig.host.view();
    rig.settle();
    rig.settle();
    assert_eq!(rig.host.view(), before);
}

#[test]
#[ignore = "writes images for a person to look at: DEGENVIEW_GALLERY_DIR=… cargo test --lib settings_script_manager -- --ignored"]
fn gallery() {
    if gallery_dir().is_none() {
        return;
    }
    for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let _theme = RestoreDefaultTheme;
        let _published = RestorePublished;
        for (shape, size) in [
            ("window", CONTENT),
            ("small", vec2(320.0, 420.0)),
            ("wide", vec2(1000.0, 700.0)),
        ] {
            let mut rig = Rig::new(size);
            theme::apply(&rig.screen.ctx, app_theme);
            let id = rig
                .host
                .create("My Dracula", ThemeSource::BuiltIn(BuiltInTheme::Dracula))
                .unwrap();
            rig.host
                .set_color(
                    id,
                    ColorKey::Syntax(SyntaxRole::Keyword),
                    Rgba::hex(0x5AF78E),
                )
                .unwrap();
            rig.host
                .create("Quiet", ThemeSource::BuiltIn(BuiltInTheme::Light))
                .unwrap();
            rig.host.select(ThemeSelection::Custom(id));
            let overview = rig.settle();
            rig.screen
                .save(&format!("script-manager-{name}-{shape}-overview"), overview);
            if shape == "small" {
                continue;
            }
            rig.state.mode = Mode::Editing(Box::new(Editing::new(rig.host.theme(id).unwrap())));
            let editing = rig.settle();
            rig.screen.save(
                &format!("script-manager-{name}-{shape}-edit-syntax"),
                editing,
            );
        }
    }
}
