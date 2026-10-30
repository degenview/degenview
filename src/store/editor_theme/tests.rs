use super::*;
use crate::model::editor_theme::{
    BuiltInTheme, ColorKey, Rgba, SyntaxRole, ThemeSelection, ThemeSource, TypographyOverride,
};

fn book() -> ThemeBook {
    let mut book = ThemeBook::default();
    let first = book
        .create(
            2_000,
            "First",
            ThemeSource::BuiltIn(BuiltInTheme::Dracula),
            1,
        )
        .unwrap();
    book.set_color(
        first,
        ColorKey::Syntax(SyntaxRole::Keyword),
        Rgba::hex(0x123456),
        2,
    )
    .unwrap();
    book.set_typography(
        first,
        TypographyOverride {
            font_family: Some("Menlo".into()),
            ..Default::default()
        },
        2,
    )
    .unwrap();
    book.create(
        2_001,
        "Second 🌙",
        ThemeSource::BuiltIn(BuiltInTheme::Light),
        3,
    )
    .unwrap();
    book
}

#[test]
fn themes_and_choices_save_and_load() {
    let store = Store::open_in_memory().unwrap();
    let book = book();
    store.replace_editor_themes(book.saved()).unwrap();
    let prefs = EditorPrefs {
        selected: ThemeSelection::Custom(2_000),
        typography: crate::model::editor_theme::Typography {
            font_size: 16.0,
            font_family: Some("JetBrains Mono".into()),
            ..Default::default()
        },
        ..EditorPrefs::default()
    };
    store.set_editor_prefs(&prefs).unwrap();

    let loaded = store.load_editor_themes();
    assert_eq!(loaded.book, book, "the same themes in the same order");
    assert_eq!(loaded.prefs, prefs);
    assert!(loaded.themes_unwritable.is_none());
    assert_eq!(
        loaded.book.get(2_000).unwrap().palette(),
        book.get(2_000).unwrap().palette()
    );
}

#[test]
fn a_database_that_never_heard_of_themes_is_the_old_editor() {
    let store = Store::open_in_memory().unwrap();
    let loaded = store.load_editor_themes();
    assert!(loaded.book.saved().is_empty());
    assert_eq!(loaded.prefs, EditorPrefs::default());
    assert_eq!(loaded.prefs.selected, ThemeSelection::Auto);
    assert!(loaded.themes_unwritable.is_none());
}

#[test]
fn a_row_that_cannot_be_read_blocks_writing_and_hides_nothing_else() {
    let store = Store::open_in_memory().unwrap();
    store.replace_editor_themes(book().saved()).unwrap();
    // A row with no name: not a theme this version can read.
    store
        .connection
        .execute(
            "INSERT INTO editor_theme (id, position, payload) VALUES ('9', 9, '{\"id\":9}')",
            [],
        )
        .unwrap();
    let loaded = store.load_editor_themes();
    assert_eq!(
        loaded.book.saved().len(),
        2,
        "the readable themes still load"
    );
    assert!(loaded.themes_unwritable.is_some());
    // The store itself refuses to rewrite a table it cannot fully read.
    assert!(store.replace_editor_themes(loaded.book.saved()).is_err());
    let rows: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM editor_theme", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 3, "the unreadable row is still there");
}

#[test]
fn unreadable_choices_fall_back_without_stopping_anything() {
    let store = Store::open_in_memory().unwrap();
    store.set_setting(KEY, &"sideways").unwrap();
    assert!(store.editor_prefs().is_err());
    let loaded = store.load_editor_themes();
    assert_eq!(loaded.prefs, EditorPrefs::default());
    // A readable value with one bad field keeps the rest.
    store
        .set_setting(
            KEY,
            &serde_json::json!({"selected": "builtin.dracula", "typography": {"fontSize": 99}}),
        )
        .unwrap();
    let prefs = store.editor_prefs().unwrap();
    assert_eq!(
        prefs.selected,
        ThemeSelection::BuiltIn(BuiltInTheme::Dracula)
    );
    assert_eq!(
        prefs.typography.font_size,
        crate::model::editor_theme::MAX_FONT_SIZE
    );
}

#[test]
fn choices_from_a_newer_version_survive_a_save() {
    let store = Store::open_in_memory().unwrap();
    store
        .set_setting(
            KEY,
            &serde_json::json!({"version": 2, "selected": "auto", "wrapLines": true}),
        )
        .unwrap();
    let mut prefs = store.editor_prefs().unwrap();
    prefs.selected = ThemeSelection::BuiltIn(BuiltInTheme::Light);
    store.set_editor_prefs(&prefs).unwrap();
    let raw: serde_json::Value = store.setting(KEY).unwrap();
    assert_eq!(raw["wrapLines"], true);
    assert_eq!(raw["version"], 2);
    assert_eq!(raw["selected"], "builtin.light");
}

#[test]
fn themes_are_not_synced_in_this_version() {
    use crate::sync::policy::{SettingPolicy, TablePolicy, setting_policy, table_policy};
    assert!(
        matches!(table_policy("editor_theme"), TablePolicy::Local(reason) if reason != "unclassified")
    );
    assert_eq!(setting_policy(KEY), SettingPolicy::Local);
}
