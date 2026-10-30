use super::*;
use crate::model::editor_theme::{LineHeight, SyntaxRole};
use crate::service::font_catalog::FontEntry;
use crate::ui::editor_appearance::{RestorePublished, current, editor_font, of_built_in};
use crate::ui::theme::RestoreDefaultTheme;

const KEYWORD: ColorKey = ColorKey::Syntax(SyntaxRole::Keyword);

fn host_over(store: &Store, catalog: FontCatalog) -> EditorThemeHost {
    EditorThemeHost::new(store.load_editor_themes(), catalog)
}

fn host() -> (EditorThemeHost, Store) {
    let store = Store::open_in_memory().unwrap();
    (
        host_over(&store, FontCatalog::with_entries(Vec::new())),
        store,
    )
}

fn soon() -> Instant {
    Instant::now() + SAVE_DELAY + Duration::from_millis(50)
}

/// A real, loadable font on disk: egui's own monospace font, under a family name of the test's.
fn font_on_disk(dir: &str, family: &str) -> (std::path::PathBuf, FontEntry) {
    let root = std::env::temp_dir().join(format!("degenview-host-{dir}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let bytes = egui::FontDefinitions::default().font_data["Hack"]
        .font
        .to_vec();
    let path = root.join("font.ttf");
    std::fs::write(&path, bytes).unwrap();
    (
        root,
        FontEntry {
            family: family.to_owned(),
            path,
            index: 0,
        },
    )
}

fn wait_for(host: &EditorThemeHost, store: &Store, done: impl Fn(&View) -> bool) -> View {
    let started = Instant::now();
    loop {
        host.tick(store, Instant::now());
        let view = host.view();
        if done(&view) {
            return view;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{view:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_fresh_install_follows_the_app_as_the_editor_always_did() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    let view = host.view();
    assert_eq!(view.selected, ThemeSelection::Auto);
    assert!(view.customs.is_empty());
    assert_eq!(view.effective, Typography::default());
    assert_eq!(view.font, FontStatus::Using("JetBrains Mono".into()));
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dark));
    assert_eq!(*editor_font(), EditorFont::default());
    assert!(editor_font().custom, "the built-in font is the default");
}

#[test]
fn choosing_a_built_in_theme_reaches_the_editors_at_once() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    host.select(ThemeSelection::BuiltIn(BuiltInTheme::Dracula));
    assert!(Arc::ptr_eq(&current(), &of_built_in(BuiltInTheme::Dracula)));
    host.select(ThemeSelection::Auto);
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dark));
}

#[test]
fn a_custom_theme_is_worn_when_made_and_follows_every_edit() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    let id = host
        .create("Mine", ThemeSource::BuiltIn(BuiltInTheme::Dracula))
        .unwrap();
    assert_eq!(host.view().selected, ThemeSelection::Custom(id));
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dracula));
    host.set_color(id, KEYWORD, Rgba::hex(0x123456)).unwrap();
    assert_eq!(
        current().role(SyntaxRole::Keyword),
        egui::Color32::from_rgb(0x12, 0x34, 0x56)
    );
    assert_eq!(
        current().role(SyntaxRole::String),
        of_built_in(BuiltInTheme::Dracula).role(SyntaxRole::String),
        "the rest is inherited"
    );
    host.reset_colors(id, &[KEYWORD]).unwrap();
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dracula));
}

#[test]
fn deleting_the_worn_theme_goes_back_to_following_the_app() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    let keep = host
        .create("Keep", ThemeSource::BuiltIn(BuiltInTheme::Light))
        .unwrap();
    let worn = host
        .create("Worn", ThemeSource::BuiltIn(BuiltInTheme::Dracula))
        .unwrap();
    host.delete(keep).unwrap();
    assert_eq!(
        host.view().selected,
        ThemeSelection::Custom(worn),
        "another theme's delete changes nothing"
    );
    host.delete(worn).unwrap();
    assert_eq!(host.view().selected, ThemeSelection::Auto);
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dark));
    assert!(host.view().customs.is_empty());
    assert!(host.delete(worn).is_err(), "it is already gone");
}

#[test]
fn a_saved_selection_of_a_theme_that_is_gone_is_automatic() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let store = Store::open_in_memory().unwrap();
    let prefs = EditorPrefs {
        selected: ThemeSelection::Custom(777),
        ..EditorPrefs::default()
    };
    store.set_editor_prefs(&prefs).unwrap();
    let host = host_over(&store, FontCatalog::with_entries(Vec::new()));
    assert_eq!(host.view().selected, ThemeSelection::Auto);
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dark));
    // It is not written back as Automatic: the theme may arrive from another Mac after the
    // choice does, and rewriting the choice meanwhile would undo it there.
    host.flush(&store);
    assert_eq!(
        store.editor_prefs().unwrap().selected,
        ThemeSelection::Custom(777)
    );
}

#[test]
fn saving_waits_for_a_pause_so_dragging_a_colour_writes_once() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, store) = host();
    let id = host
        .create("Mine", ThemeSource::BuiltIn(BuiltInTheme::Dark))
        .unwrap();
    // Dozens of changes in a row, as a colour picker makes while it is dragged.
    for step in 0..60u8 {
        host.set_color(id, KEYWORD, Rgba::new(step, 10, 20, 255))
            .unwrap();
        host.tick(&store, Instant::now());
    }
    assert!(
        store.editor_themes_strict().unwrap().is_empty(),
        "nothing written mid-drag"
    );
    assert_eq!(store.editor_prefs().unwrap().selected, ThemeSelection::Auto);
    host.tick(&store, soon());
    let saved = store.editor_themes_strict().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved[0].overrides["keyword"],
        Rgba::new(59, 10, 20, 255).to_hex(),
        "the last colour"
    );
    assert_eq!(
        store.editor_prefs().unwrap().selected,
        ThemeSelection::Custom(id)
    );
    // Idle: nothing more is written.
    store.replace_editor_themes(&[]).unwrap();
    host.tick(&store, soon());
    assert!(store.editor_themes_strict().unwrap().is_empty());
}

#[test]
fn flush_saves_at_once_for_quitting() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, store) = host();
    host.create("Mine", ThemeSource::BuiltIn(BuiltInTheme::Light))
        .unwrap();
    host.set_typography(Typography {
        font_size: 17.0,
        ..Typography::default()
    });
    host.flush(&store);
    let reloaded = host_over(&store, FontCatalog::with_entries(Vec::new()));
    let view = reloaded.view();
    assert_eq!(view.customs.len(), 1);
    assert_eq!(view.customs[0].theme.name, "Mine");
    assert_eq!(view.typography.font_size, 17.0);
    assert!(matches!(view.selected, ThemeSelection::Custom(_)));
}

#[test]
fn themes_survive_a_restart_with_every_override() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, store) = host();
    let id = host
        .create("Night", ThemeSource::BuiltIn(BuiltInTheme::Dracula))
        .unwrap();
    host.set_color(id, KEYWORD, Rgba::hex(0xABCDEF)).unwrap();
    host.set_color(
        id,
        ColorKey::Surface(crate::model::editor_theme::SurfaceColor::Selection),
        Rgba::with_alpha(0x112233, 99),
    )
    .unwrap();
    host.flush(&store);
    let before = host.theme(id).unwrap();
    let after = host_over(&store, FontCatalog::with_entries(Vec::new()))
        .theme(id)
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(before.palette(), after.palette());
}

#[test]
fn a_table_that_cannot_be_read_is_never_written_but_everything_else_works() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let store = Store::open_in_memory().unwrap();
    store
        .raw_connection()
        .execute(
            "INSERT INTO editor_theme (id, position, payload) VALUES ('9', 0, 'not json')",
            [],
        )
        .unwrap();
    let host = host_over(&store, FontCatalog::with_entries(Vec::new()));
    assert!(host.view().unwrap_or_default_for_test());
    let refused = host.create("New", ThemeSource::BuiltIn(BuiltInTheme::Dark));
    assert!(refused.is_err());
    assert!(
        host.view().customs.is_empty(),
        "nothing changed in memory either"
    );
    // Choosing a built-in theme and the typography still work and are saved.
    host.select(ThemeSelection::BuiltIn(BuiltInTheme::Dracula));
    host.flush(&store);
    assert_eq!(
        store.editor_prefs().unwrap().selected,
        ThemeSelection::BuiltIn(BuiltInTheme::Dracula)
    );
    let rows: i64 = store
        .raw_connection()
        .query_row("SELECT COUNT(*) FROM editor_theme", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "the unreadable row is untouched");
}

trait ViewExt {
    fn unwrap_or_default_for_test(&self) -> bool;
}

impl ViewExt for View {
    fn unwrap_or_default_for_test(&self) -> bool {
        self.unwritable.is_some()
    }
}

#[test]
fn typography_reaches_the_editor_and_a_theme_may_override_it() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    host.set_typography(Typography {
        font_size: 18.0,
        line_height: LineHeight::Relaxed,
        ..Typography::default()
    });
    assert_eq!(editor_font().size, 18.0);
    assert_eq!(editor_font().line_height, LineHeight::Relaxed);
    let id = host
        .create("Big", ThemeSource::BuiltIn(BuiltInTheme::Dark))
        .unwrap();
    assert_eq!(
        editor_font().size,
        18.0,
        "a theme that names no size leaves it alone"
    );
    host.set_theme_typography(
        id,
        TypographyOverride {
            font_size: Some(24.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(editor_font().size, 24.0);
    assert_eq!(editor_font().line_height, LineHeight::Relaxed);
    assert_eq!(
        host.view().typography.font_size,
        18.0,
        "the person's own setting is untouched"
    );
    host.select(ThemeSelection::Auto);
    assert_eq!(editor_font().size, 18.0, "and it is what returns");
    host.set_typography(Typography {
        font_size: 400.0,
        ..Typography::default()
    });
    assert_eq!(
        editor_font().size,
        crate::model::editor_theme::MAX_FONT_SIZE
    );
}

#[test]
fn a_chosen_font_is_read_off_thread_and_installed() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (root, entry) = font_on_disk("install", "Test Mono");
    let store = Store::open_in_memory().unwrap();
    let host = host_over(&store, FontCatalog::with_entries(vec![entry]));
    let ctx = egui::Context::default();
    host.attach(&ctx);
    host.set_typography(Typography {
        font_family: Some("test mono".into()),
        ..Typography::default()
    });
    assert!(
        matches!(host.view().font, FontStatus::Loading(_)),
        "reading, the old font still drawn"
    );
    assert!(
        editor_font().custom,
        "the built-in font draws while the other is read"
    );
    let view = wait_for(&host, &store, |view| {
        matches!(view.font, FontStatus::Using(_))
    });
    assert_eq!(view.font, FontStatus::Using("test mono".into()));
    assert!(editor_font().custom);
    // egui builds the fonts on its next pass; the family is then there to draw with.
    ctx.run_ui(egui::RawInput::default(), |_| {})
        .textures_delta
        .clear();
    let font = editor_font().code(&ctx);
    assert_eq!(
        font.family,
        egui::FontFamily::Name(crate::ui::editor_appearance::EDITOR_FAMILY.into())
    );
    // Putting the default back installs the system font again.
    host.set_typography(Typography::default());
    assert_eq!(host.view().font, FontStatus::Using("JetBrains Mono".into()));
    assert!(editor_font().custom);
    // And the system's own font is a choice too.
    host.set_typography(Typography {
        font_family: None,
        ..Typography::default()
    });
    assert_eq!(host.view().font, FontStatus::System);
    assert!(!editor_font().custom);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_built_in_font_needs_neither_the_scan_nor_a_file() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let store = Store::open_in_memory().unwrap();
    // A catalog that has not found anything yet, and a context that has just arrived.
    let host = host_over(&store, FontCatalog::default());
    let ctx = egui::Context::default();
    host.attach(&ctx);
    let view = host.view();
    assert_eq!(view.font, FontStatus::Using("JetBrains Mono".into()));
    assert!(!view.fonts_ready);
    assert_eq!(
        *view.fonts,
        ["JetBrains Mono"],
        "offered before anything is found"
    );
    assert!(editor_font().custom);
    ctx.run_ui(egui::RawInput::default(), |_| {})
        .textures_delta
        .clear();
    assert_eq!(
        editor_font().code(&ctx).family,
        egui::FontFamily::Name(crate::ui::editor_appearance::EDITOR_FAMILY.into())
    );
    // Choosing it by hand, in another case, is the same font: no thread, no waiting.
    host.set_typography(Typography {
        font_family: Some("jetbrains mono".into()),
        ..Typography::default()
    });
    assert_eq!(host.view().font, FontStatus::Using("JetBrains Mono".into()));
}

#[test]
fn the_system_font_is_a_choice_for_the_editor_and_for_a_theme() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    host.attach(&egui::Context::default());
    host.set_typography(Typography {
        font_family: None,
        ..Typography::default()
    });
    assert_eq!(host.view().font, FontStatus::System);
    assert!(
        !editor_font().custom,
        "the system's own monospace font draws"
    );
    // A theme can ask for the built-in font over the person's system choice, and for the system's
    // over the built-in one.
    let id = host
        .create("Mine", ThemeSource::BuiltIn(BuiltInTheme::Dark))
        .unwrap();
    host.set_theme_typography(
        id,
        TypographyOverride {
            font_family: Some("JetBrains Mono".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(host.view().font, FontStatus::Using("JetBrains Mono".into()));
    assert!(editor_font().custom);
    host.set_typography(Typography::default());
    host.set_theme_typography(
        id,
        TypographyOverride {
            font_family: Some("System Mono".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(host.view().font, FontStatus::System);
    assert!(!editor_font().custom);
    // Take the theme's choice away and the person's (the built-in font) is back.
    host.reset_all(id).unwrap();
    assert_eq!(host.view().font, FontStatus::Using("JetBrains Mono".into()));
}

#[test]
fn the_picker_lists_the_built_in_font_first_and_an_installed_namesake_is_that_font() {
    let list = family_list(
        [
            "Menlo",
            "jetbrains mono",
            "Fira Code",
            "System Mono",
            "menlo",
            "JetBrains Mono",
        ]
        .into_iter()
        .map(String::from),
    );
    assert_eq!(list, ["JetBrains Mono", "Fira Code", "Menlo"]);
    assert_eq!(family_list(std::iter::empty()), ["JetBrains Mono"]);
}

#[test]
fn a_font_that_is_not_installed_keeps_its_name_and_the_default_draws() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let store = Store::open_in_memory().unwrap();
    let host = host_over(&store, FontCatalog::with_entries(Vec::new()));
    host.attach(&egui::Context::default());
    host.set_typography(Typography {
        font_family: Some("Gone Mono".into()),
        ..Typography::default()
    });
    assert_eq!(host.view().font, FontStatus::Missing("Gone Mono".into()));
    assert!(editor_font().custom, "the built-in font draws in its place");
    host.flush(&store);
    // The name is saved, so the font returns when it is installed again.
    assert_eq!(
        store
            .editor_prefs()
            .unwrap()
            .typography
            .font_family
            .as_deref(),
        Some("Gone Mono")
    );
    let (root, entry) = font_on_disk("return", "Gone Mono");
    let back = host_over(&store, FontCatalog::with_entries(vec![entry]));
    back.attach(&egui::Context::default());
    wait_for(&back, &store, |view| {
        matches!(view.font, FontStatus::Using(_))
    });
    assert!(editor_font().custom);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_saved_font_waits_for_the_font_list_instead_of_being_called_missing() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let store = Store::open_in_memory().unwrap();
    let mut prefs = EditorPrefs::default();
    prefs.typography.font_family = Some("Late Mono".into());
    store.set_editor_prefs(&prefs).unwrap();
    // A catalog that has not finished scanning.
    let host = host_over(&store, FontCatalog::default());
    host.attach(&egui::Context::default());
    assert_eq!(host.view().font, FontStatus::Loading("Late Mono".into()));
    assert!(!host.view().fonts_ready);
    assert!(
        editor_font().custom,
        "waiting for the list: the built-in font draws"
    );
}

#[test]
fn a_font_that_cannot_be_read_is_reported_and_the_default_keeps_drawing() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (root, entry) = font_on_disk("unreadable", "Broken Mono");
    let store = Store::open_in_memory().unwrap();
    let host = host_over(&store, FontCatalog::with_entries(vec![entry.clone()]));
    host.attach(&egui::Context::default());
    std::fs::remove_file(&entry.path).unwrap();
    host.set_typography(Typography {
        font_family: Some("Broken Mono".into()),
        ..Typography::default()
    });
    let view = wait_for(&host, &store, |view| {
        matches!(view.font, FontStatus::Failed(..))
    });
    assert!(matches!(view.font, FontStatus::Failed(ref name, _) if name == "Broken Mono"));
    assert!(
        editor_font().custom,
        "an unreadable font: the built-in one draws"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_font_with_nothing_to_draw_is_refused_and_the_built_in_font_draws() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    // A "font" that reads fine but has no letters in it (a bitmap font, a broken file): the editor
    // would be blank in it.
    let (root, entry) = font_on_disk("blank", "Blank Mono");
    let bytes = std::fs::read(&entry.path).unwrap();
    std::fs::write(&entry.path, &bytes[..bytes.len() / 3]).unwrap();
    let store = Store::open_in_memory().unwrap();
    let host = host_over(&store, FontCatalog::with_entries(vec![entry]));
    let ctx = egui::Context::default();
    host.attach(&ctx);
    host.set_typography(Typography {
        font_family: Some("Blank Mono".into()),
        ..Typography::default()
    });
    let view = wait_for(&host, &store, |view| {
        matches!(view.font, FontStatus::Failed(..))
    });
    match &view.font {
        FontStatus::Failed(name, why) => {
            assert_eq!(name, "Blank Mono");
            assert!(why.contains("font"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(editor_font().custom, "the built-in font draws instead");
    // The name stays chosen, so the font is not forgotten.
    assert_eq!(
        host.view().typography.font_family.as_deref(),
        Some("Blank Mono")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_read_for_a_font_nobody_wants_any_more_is_dropped() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (root, entry) = font_on_disk("stale", "Old Mono");
    let store = Store::open_in_memory().unwrap();
    let host = host_over(&store, FontCatalog::with_entries(vec![entry]));
    host.attach(&egui::Context::default());
    host.set_typography(Typography {
        font_family: Some("Old Mono".into()),
        ..Typography::default()
    });
    // Changed their mind before the file was read.
    host.set_typography(Typography::default());
    std::thread::sleep(Duration::from_millis(200));
    host.tick(&store, Instant::now());
    assert_eq!(host.view().font, FontStatus::Using("JetBrains Mono".into()));
    assert!(editor_font().custom);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn export_and_import_go_through_the_host_without_replacing_anything() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    let id = host
        .create(
            "Night / Day: *1*",
            ThemeSource::BuiltIn(BuiltInTheme::Dracula),
        )
        .unwrap();
    host.set_color(id, KEYWORD, Rgba::hex(0x445566)).unwrap();
    let (stem, text) = host.export(ThemeSource::Custom(id)).unwrap();
    assert_eq!(stem, "Night - Day- -1", "a name safe for a file");
    let imported = host.import(&text).unwrap();
    assert_ne!(imported.id, id);
    assert_eq!(imported.name, "Night / Day: *1* 2");
    assert!(imported.skipped.is_empty());
    assert_eq!(
        host.theme(imported.id).unwrap().palette(),
        host.theme(id).unwrap().palette()
    );
    assert_eq!(
        host.view().selected,
        ThemeSelection::Custom(imported.id),
        "the imported theme is worn"
    );
    assert_eq!(host.view().customs.len(), 2);
    // A built-in exports too.
    let (stem, text) = host
        .export(ThemeSource::BuiltIn(BuiltInTheme::Light))
        .unwrap();
    assert_eq!(stem, "Light");
    assert!(host.import(&text).is_ok());
    assert!(host.import("{ nope").is_err());
    assert!(host.import("{}").is_err());
    assert_eq!(host.view().customs.len(), 3, "failed imports add nothing");
}

#[test]
fn file_stems_are_safe() {
    assert_eq!(file_stem("My Theme"), "My Theme");
    assert_eq!(file_stem("a/b\\c:d"), "a-b-c-d");
    assert_eq!(file_stem("Ночь 🌙"), "Ночь");
    assert_eq!(file_stem("///"), "theme");
    assert_eq!(file_stem(".."), "theme");
}

#[test]
fn restore_undoes_a_session_of_edits() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    let id = host
        .create("Mine", ThemeSource::BuiltIn(BuiltInTheme::Dark))
        .unwrap();
    let before = host.theme(id).unwrap();
    host.set_color(id, KEYWORD, Rgba::hex(0x010203)).unwrap();
    host.restore(before.clone()).unwrap();
    assert_eq!(host.theme(id).unwrap(), before);
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dark));
}

#[test]
fn many_themes_and_rapid_switching_stay_cheap() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (host, _store) = host();
    for number in 0..150 {
        host.create(
            &format!("Theme {number}"),
            ThemeSource::BuiltIn(BuiltInTheme::Dracula),
        )
        .unwrap();
    }
    let ids: Vec<_> = host.view().customs.iter().map(|c| c.theme.id).collect();
    let started = Instant::now();
    for round in 0..40 {
        for id in &ids {
            host.select(ThemeSelection::Custom(*id));
        }
        host.select(ThemeSelection::BuiltIn(BuiltInTheme::ALL[round % 3]));
    }
    // 6,000 switches, each resolving a palette and publishing it. A frame is 16 ms.
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}
