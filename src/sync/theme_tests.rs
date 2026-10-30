//! Custom editor themes and the editor's choices across two Macs: the real core and apply layer.

use super::apply_tests::{outbox_len, pair, rows, settle};
use super::core_tests::{Raw, raw};
use crate::host::editor_theme_host::{EditorThemeHost, FontStatus};
use crate::model::editor_theme::prefs::KEY;
use crate::model::editor_theme::{
    BuiltInTheme, EditorPrefs, EditorTheme, ThemeSelection, TypographyOverride,
};
use crate::service::font_catalog::FontCatalog;
use crate::ui::editor_appearance::RestorePublished;
use crate::ui::theme::RestoreDefaultTheme;
use serde_json::json;
use std::collections::BTreeMap;

fn theme(id: u64, name: &str) -> EditorTheme {
    EditorTheme {
        id,
        name: name.into(),
        base: BuiltInTheme::Dracula.id().into(),
        overrides: BTreeMap::from([("keyword".to_owned(), "#123456".to_owned())]),
        typography: TypographyOverride {
            font_size: Some(15.0),
            ..Default::default()
        },
        version: 1,
        created_at: 1,
        updated_at: 2,
        extra: BTreeMap::new(),
    }
}

fn put(mac: &super::core_tests::Mac, themes: &[EditorTheme]) {
    let rows: Vec<Raw> = themes
        .iter()
        .map(|theme| Raw(serde_json::to_value(theme).unwrap()))
        .collect();
    mac.store
        .lock()
        .unwrap()
        .replace_documents("editor_theme", &rows)
        .unwrap();
}

#[test]
fn custom_themes_reach_the_other_mac_whole_and_do_not_echo_back() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[theme(2_000, "Night"), theme(2_001, "Day ☀️")]);
    settle(&mut a, &mut b);
    let on_b = rows(&b, "editor_theme");
    assert_eq!(on_b.len(), 2);
    assert_eq!(on_b[0]["name"], "Night");
    assert_eq!(on_b[1]["name"], "Day ☀️");
    assert_eq!(on_b[0]["overrides"]["keyword"], "#123456");
    assert_eq!(on_b[0]["typography"]["fontSize"], 15.0);
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0, "nothing echoes back");
    // The theme is a theme on the other Mac too: its palette is the same.
    let loaded = b.store.lock().unwrap().load_editor_themes();
    let original = theme(2_000, "Night");
    assert_eq!(
        loaded.book.get(2_000).unwrap().palette(),
        original.palette()
    );
}

#[test]
fn a_rename_a_change_and_a_delete_travel() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[theme(2_000, "Night"), theme(2_001, "Day")]);
    settle(&mut a, &mut b);

    // Mac B edits one and renames the other; Mac A sees both.
    let mut edited = theme(2_000, "Night");
    edited.overrides.insert("string".into(), "#ABCDEF".into());
    put(&b, &[edited, theme(2_001, "Dusk")]);
    settle(&mut a, &mut b);
    let on_a = rows(&a, "editor_theme");
    assert_eq!(on_a[0]["overrides"]["string"], "#ABCDEF");
    assert_eq!(on_a[1]["name"], "Dusk");

    // And a delete on A reaches B without touching the other theme.
    let mut edited = theme(2_000, "Night");
    edited.overrides.insert("string".into(), "#ABCDEF".into());
    put(&a, &[edited]);
    settle(&mut a, &mut b);
    let on_b = rows(&b, "editor_theme");
    assert_eq!(on_b.len(), 1, "the other theme was deleted");
    assert_eq!(on_b[0]["name"], "Night");
    assert_eq!(on_b[0]["overrides"]["string"], "#ABCDEF");
}

#[test]
fn the_later_edit_of_one_theme_wins_on_both_macs() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[theme(2_000, "Night")]);
    settle(&mut a, &mut b);
    let mut on_a = theme(2_000, "Night");
    on_a.overrides.insert("keyword".into(), "#111111".into());
    put(&a, &[on_a]);
    settle(&mut a, &mut b);
    let mut on_b = theme(2_000, "Night");
    on_b.overrides.insert("keyword".into(), "#222222".into());
    put(&b, &[on_b]);
    settle(&mut a, &mut b);
    assert_eq!(
        rows(&a, "editor_theme")[0]["overrides"]["keyword"],
        "#222222"
    );
    assert_eq!(
        rows(&b, "editor_theme")[0]["overrides"]["keyword"],
        "#222222"
    );
}

#[test]
fn a_record_that_is_not_a_theme_is_kept_not_applied() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    for (name, payload) in [
        // An id a built-in could never have had, claimed by a custom theme.
        ("editor_theme.5", json!({"id": 5, "name": "Pretends"})),
        // No name.
        ("editor_theme.2000", json!({"id": 2000})),
        // An id that is not the record's.
        ("editor_theme.2001", json!({"id": 9, "name": "Mismatch"})),
        // An id that is a string.
        (
            "editor_theme.2002",
            json!({"id": "2002", "name": "Stringy"}),
        ),
    ] {
        cloud.put_from_elsewhere(raw(name, &payload, 1, 1, "Z"), "Z");
    }
    settle(&mut a, &mut b);
    assert!(rows(&b, "editor_theme").is_empty());
    let kept = b.store.lock().unwrap().inbox().unwrap();
    assert_eq!(
        kept.iter()
            .filter(|record| record.record_name.starts_with("editor_theme."))
            .count(),
        4,
        "{kept:?}"
    );
}

#[test]
fn fields_a_newer_version_wrote_survive_the_trip() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let future = json!({
        "id": 2000, "name": "From the future", "base": "builtin.dracula",
        "overrides": {"keyword": "#123456", "someNewRole": "#654321"},
        "typography": {}, "version": 2, "createdAt": 1, "updatedAt": 2,
        "tabWidth": 8
    });
    cloud.put_from_elsewhere(raw("editor_theme.2000", &future, 1, 1, "Z"), "Z");
    settle(&mut a, &mut b);
    let row = &rows(&b, "editor_theme")[0];
    assert_eq!(row["tabWidth"], 8);
    assert_eq!(row["overrides"]["someNewRole"], "#654321");
    assert_eq!(row["version"], 2);
}

#[test]
fn the_choices_travel_and_a_missing_font_falls_back_on_the_other_mac() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    put(&a, &[theme(2_000, "Night")]);
    let mut prefs = EditorPrefs {
        selected: ThemeSelection::Custom(2_000),
        ..EditorPrefs::default()
    };
    prefs.typography.font_family = Some("Only On Mac A".into());
    prefs.typography.font_size = 17.0;
    a.store.lock().unwrap().set_setting(KEY, &prefs).unwrap();
    settle(&mut a, &mut b);

    let store = b.store.lock().unwrap();
    let host = EditorThemeHost::new(
        store.load_editor_themes(),
        FontCatalog::with_entries(Vec::new()),
    );
    host.attach(&eframe::egui::Context::default());
    let view = host.view();
    assert_eq!(view.selected, ThemeSelection::Custom(2_000));
    assert_eq!(
        view.effective.font_size, 15.0,
        "the theme's own size over the choice"
    );
    assert_eq!(view.typography.font_size, 17.0);
    // The font is not installed here: its name is kept and the default draws.
    assert_eq!(view.font, FontStatus::Missing("Only On Mac A".into()));
    assert_eq!(
        store
            .editor_prefs()
            .unwrap()
            .typography
            .font_family
            .as_deref(),
        Some("Only On Mac A")
    );
}

#[test]
fn a_choice_that_arrives_before_its_theme_is_not_rewritten() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    // Mac A chose a theme; its prefs reach Mac B while the theme has not yet.
    let prefs = EditorPrefs {
        selected: ThemeSelection::Custom(2_000),
        ..EditorPrefs::default()
    };
    a.store.lock().unwrap().set_setting(KEY, &prefs).unwrap();
    settle(&mut a, &mut b);
    let host = {
        let store = b.store.lock().unwrap();
        let host = EditorThemeHost::new(store.load_editor_themes(), FontCatalog::default());
        host.attach(&eframe::egui::Context::default());
        host
    };
    assert_eq!(
        host.view().selected,
        ThemeSelection::Auto,
        "worn as Automatic meanwhile"
    );
    // Mac B saves something (its own typography), which must not undo the choice.
    host.set_typography(crate::model::editor_theme::Typography {
        font_size: 16.0,
        ..Default::default()
    });
    {
        let store = b.store.lock().unwrap();
        host.flush(&store);
    }
    // The theme arrives; reading again, the choice is there.
    put(&a, &[theme(2_000, "Night")]);
    settle(&mut a, &mut b);
    let host = {
        let store = b.store.lock().unwrap();
        let loaded = store.load_editor_themes();
        assert_eq!(loaded.book.saved().len(), 1);
        EditorThemeHost::new(loaded, FontCatalog::default())
    };
    // B's own edit is the later one for the typography, but the choice was never rewritten, so
    // the theme is worn.
    assert_eq!(host.view().selected, ThemeSelection::Custom(2_000));
}

#[test]
fn reloading_picks_up_what_sync_wrote_unless_this_mac_has_unsaved_changes() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let host = EditorThemeHost::new(
        b.store.lock().unwrap().load_editor_themes(),
        FontCatalog::default(),
    );
    host.attach(&eframe::egui::Context::default());
    put(&a, &[theme(2_000, "Night")]);
    a.store
        .lock()
        .unwrap()
        .set_setting(
            KEY,
            &EditorPrefs {
                selected: ThemeSelection::Custom(2_000),
                ..EditorPrefs::default()
            },
        )
        .unwrap();
    settle(&mut a, &mut b);
    assert!(host.view().customs.is_empty(), "nothing reloaded yet");
    host.reload(&b.store.lock().unwrap());
    let view = host.view();
    assert_eq!(view.customs.len(), 1);
    assert_eq!(view.selected, ThemeSelection::Custom(2_000));

    // An unsaved change here is not thrown away by a reload.
    host.set_typography(crate::model::editor_theme::Typography {
        font_size: 20.0,
        ..Default::default()
    });
    put(&a, &[theme(2_000, "Night"), theme(2_001, "Another")]);
    settle(&mut a, &mut b);
    host.reload(&b.store.lock().unwrap());
    assert_eq!(host.view().typography.font_size, 20.0);
    assert_eq!(
        host.view().customs.len(),
        1,
        "it waits until this Mac has saved"
    );
    host.flush(&b.store.lock().unwrap());
    host.reload(&b.store.lock().unwrap());
    assert_eq!(host.view().customs.len(), 2);
}
