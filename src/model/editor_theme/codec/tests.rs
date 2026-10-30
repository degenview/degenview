use super::*;
use crate::model::editor_theme::{Rgba, ThemeBook, ThemeSource};

fn theme_with_everything() -> EditorTheme {
    let mut book = ThemeBook::default();
    let id = book
        .create(
            2_000,
            "My Dracula 🌙",
            ThemeSource::BuiltIn(BuiltInTheme::Dracula),
            5,
        )
        .unwrap();
    book.set_color(
        id,
        ColorKey::Syntax(SyntaxRole::Keyword),
        Rgba::hex(0x123456),
        6,
    )
    .unwrap();
    book.set_color(
        id,
        ColorKey::Surface(SurfaceColor::Selection),
        Rgba::with_alpha(0xAABBCC, 0x80),
        6,
    )
    .unwrap();
    book.set_typography(
        id,
        TypographyOverride {
            font_family: Some("JetBrains Mono".into()),
            font_size: Some(15.5),
            line_height: Some(LineHeight::Relaxed),
        },
        6,
    )
    .unwrap();
    book.get(id).unwrap().clone()
}

fn replace_in(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from} not in file");
    text.replacen(from, to, 1)
}

#[test]
fn an_export_round_trips_to_the_same_appearance() {
    let theme = theme_with_everything();
    let text = export(&theme);
    let imported = import(&text).unwrap();
    assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
    let mut book = ThemeBook::default();
    let id = book.import(3_000, imported.theme, 9).unwrap();
    let back = book.get(id).unwrap();
    assert_eq!(back.palette(), theme.palette(), "every colour is the same");
    assert_eq!(back.typography, theme.typography);
    assert_eq!(back.base_theme(), theme.base_theme());
    assert_eq!(back.name, theme.name);
    // And the import kept only what differs from the base.
    assert_eq!(back.override_count(), 2);
}

#[test]
fn an_export_is_complete_readable_and_portable() {
    let text = export(&theme_with_everything());
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["format"], FORMAT);
    assert_eq!(value["version"], VERSION);
    assert_eq!(value["base"], "builtin.dracula");
    assert_eq!(
        value["syntax"].as_object().unwrap().len(),
        SyntaxRole::COUNT
    );
    assert_eq!(
        value["editor"].as_object().unwrap().len(),
        SurfaceColor::COUNT
    );
    assert_eq!(value["syntax"]["keyword"], "#123456");
    assert_eq!(value["editor"]["selection"], "#AABBCC80");
    assert_eq!(value["typography"]["fontFamily"], "JetBrains Mono");
    assert!(text.contains("\n  "), "pretty-printed");
    assert!(text.ends_with("}\n"));
    // Nothing machine-specific: no paths, no ids, no timestamps.
    for forbidden in [
        "/Users",
        "\\\\",
        "\"id\"",
        "createdAt",
        "updatedAt",
        "overrides",
    ] {
        assert!(!text.contains(forbidden), "{forbidden}");
    }
    // The format and the name come first, in the order the settings read.
    assert!(text.find("\"format\"").unwrap() < text.find("\"syntax\"").unwrap());
    assert!(text.find("\"default\"").unwrap() < text.find("\"keyword\"").unwrap());
}

#[test]
fn a_theme_with_no_typography_writes_none() {
    let mut book = ThemeBook::default();
    let id = book
        .create(2_000, "Plain", ThemeSource::BuiltIn(BuiltInTheme::Light), 0)
        .unwrap();
    let value: Value = serde_json::from_str(&export(book.get(id).unwrap())).unwrap();
    assert!(value.get("typography").is_none());
}

#[test]
fn the_colours_that_match_the_base_are_not_overrides() {
    let imported = import(&export(&theme_with_everything())).unwrap();
    assert_eq!(imported.theme.overrides.len(), 2);
    assert_eq!(imported.theme.overrides["keyword"], "#123456");
}

#[test]
fn bad_json_and_other_files_are_refused() {
    assert_eq!(import(""), Err(ImportError::NotJson));
    assert_eq!(import("{"), Err(ImportError::NotJson));
    assert_eq!(import("not json at all"), Err(ImportError::NotJson));
    assert_eq!(import("[]"), Err(ImportError::NotAThemeFile));
    assert_eq!(import("12"), Err(ImportError::NotAThemeFile));
    assert_eq!(import("{}"), Err(ImportError::NotAThemeFile));
    assert_eq!(
        import(r#"{"format":"someone-elses","version":1}"#),
        Err(ImportError::NotAThemeFile)
    );
    assert_eq!(
        import(r#"{"format":"degenview-editor-theme"}"#),
        Err(ImportError::NotAThemeFile)
    );
    assert_eq!(
        import(r#"{"format":"degenview-editor-theme","version":"1"}"#),
        Err(ImportError::NotAThemeFile)
    );
}

#[test]
fn a_version_this_build_does_not_read_is_refused_with_advice() {
    let text = replace_in(
        &export(&theme_with_everything()),
        "\"version\": 1",
        "\"version\": 2",
    );
    let error = import(&text).unwrap_err();
    assert_eq!(error, ImportError::UnsupportedVersion(2));
    assert!(error.message().contains("Update DegenView"));
    let zero = replace_in(
        &export(&theme_with_everything()),
        "\"version\": 1",
        "\"version\": 0",
    );
    assert_eq!(import(&zero), Err(ImportError::UnsupportedVersion(0)));
}

#[test]
fn an_oversize_file_is_refused_unread() {
    let filler = "x".repeat(MAX_FILE_BYTES);
    let text = format!(
        r#"{{"format":"degenview-editor-theme","version":1,"name":"{filler}","base":"builtin.dark"}}"#
    );
    assert_eq!(import(&text), Err(ImportError::TooLarge));
    assert!(import(&"{".repeat(MAX_FILE_BYTES + 1)).unwrap_err() == ImportError::TooLarge);
}

#[test]
fn names_and_bases_are_checked() {
    let file = |name: &str, base: &str| {
        format!(r#"{{"format":"degenview-editor-theme","version":1,"name":{name},"base":{base}}}"#)
    };
    assert_eq!(
        import(&file("\"Ok\"", "\"builtin.dark\""))
            .unwrap()
            .theme
            .name,
        "Ok"
    );
    assert_eq!(
        import(r#"{"format":"degenview-editor-theme","version":1,"base":"builtin.dark"}"#),
        Err(ImportError::MissingName)
    );
    for name in ["\"\"", "\"   \"", "7", "\"a\\u0007b\""] {
        let error = import(&file(name, "\"builtin.dark\"")).unwrap_err();
        assert!(
            matches!(error, ImportError::BadName(_) | ImportError::MissingName),
            "{name}: {error:?}"
        );
    }
    let long = format!("\"{}\"", "n".repeat(500));
    assert!(matches!(
        import(&file(&long, "\"builtin.dark\"")),
        Err(ImportError::BadName(_))
    ));
    for base in ["\"builtin.nope\"", "\"custom.5\"", "\"\"", "5"] {
        let error = import(&file("\"Ok\"", base)).unwrap_err();
        assert!(
            matches!(error, ImportError::UnknownBase(_)),
            "{base}: {error:?}"
        );
    }
    // A hostile base name is cut, not echoed whole.
    let error = import(&file("\"Ok\"", &format!("\"{}\"", "b".repeat(5000)))).unwrap_err();
    assert!(error.message().len() < 300);
}

#[test]
fn colours_that_cannot_be_read_are_skipped_and_said() {
    let mut text = export(&theme_with_everything());
    text = replace_in(&text, "\"#123456\"", "\"blue\"");
    text = replace_in(&text, "\"#AABBCC80\"", "\"#ABC\"");
    let imported = import(&text).unwrap();
    // Both fell back to the base's colour: no override at all.
    assert!(imported.theme.overrides.is_empty());
    assert_eq!(imported.skipped.len(), 2, "{:?}", imported.skipped);
    assert!(imported.skipped.iter().any(|line| line.contains("keyword")));
    assert!(
        imported
            .skipped
            .iter()
            .any(|line| line.contains("selection"))
    );
}

#[test]
fn unknown_and_misplaced_colours_are_skipped() {
    let text = r##"{"format":"degenview-editor-theme","version":1,"name":"T","base":"builtin.dark",
        "syntax":{"keyword":"#010203","sparkle":"#040506","background":"#070809","number":5},
        "editor":{"selection":"#0A0B0C","keyword":"#0D0E0F"}}"##;
    let imported = import(text).unwrap();
    assert_eq!(imported.theme.overrides.len(), 2);
    assert!(imported.theme.overrides.contains_key("keyword"));
    assert!(imported.theme.overrides.contains_key("selection"));
    assert_eq!(imported.skipped.len(), 4, "{:?}", imported.skipped);
}

#[test]
fn missing_sections_are_a_theme_that_is_its_base() {
    let imported = import(
        r#"{"format":"degenview-editor-theme","version":1,"name":"Bare","base":"builtin.light"}"#,
    )
    .unwrap();
    assert_eq!(imported.theme.base, BuiltInTheme::Light);
    assert!(imported.theme.overrides.is_empty());
    assert!(imported.theme.typography.is_empty());
    assert!(imported.skipped.is_empty());
}

#[test]
fn sections_of_the_wrong_shape_are_skipped_whole() {
    let imported = import(
        r#"{"format":"degenview-editor-theme","version":1,"name":"T","base":"builtin.dark",
            "syntax":[1,2],"editor":"x","typography":7}"#,
    )
    .unwrap();
    assert_eq!(imported.skipped.len(), 3);
    assert!(imported.theme.overrides.is_empty());
}

#[test]
fn typography_is_validated_value_by_value() {
    let imported = import(
        r#"{"format":"degenview-editor-theme","version":1,"name":"T","base":"builtin.dark",
            "typography":{"fontFamily":"Menlo","fontSize":500,"lineHeight":"huge"}}"#,
    )
    .unwrap();
    assert_eq!(
        imported.theme.typography.font_family.as_deref(),
        Some("Menlo")
    );
    assert_eq!(
        imported.theme.typography.font_size,
        Some(crate::model::editor_theme::MAX_FONT_SIZE)
    );
    assert_eq!(imported.theme.typography.line_height, None);
    assert_eq!(imported.skipped.len(), 1);
    let imported = import(
        r#"{"format":"degenview-editor-theme","version":1,"name":"T","base":"builtin.dark",
            "typography":{"fontFamily":"a\nb","fontSize":"big"}}"#,
    )
    .unwrap();
    assert!(imported.theme.typography.is_empty());
    assert_eq!(imported.skipped.len(), 2);
}

#[test]
fn an_imported_theme_never_replaces_one_and_always_gets_a_new_id() {
    let theme = theme_with_everything();
    let text = export(&theme);
    let mut book = ThemeBook::new(vec![theme.clone()]);
    let first = book.import(3_001, import(&text).unwrap().theme, 1).unwrap();
    let second = book.import(3_002, import(&text).unwrap().theme, 2).unwrap();
    assert_eq!(book.saved().len(), 3, "nothing was overwritten");
    assert_eq!(book.get(2_000).unwrap(), &theme);
    assert_ne!(first, second);
    assert_ne!(first, theme.id);
    assert_eq!(book.get(first).unwrap().name, "My Dracula 🌙 2");
    assert_eq!(book.get(second).unwrap().name, "My Dracula 🌙 3");
}

#[test]
fn extra_fields_and_a_foreign_id_are_ignored() {
    let text = r##"{"format":"degenview-editor-theme","version":1,"id":42,"name":"T",
        "base":"builtin.dark","createdAt":5,"extra":{"a":1},"overrides":{"keyword":"#010203"}}"##;
    let imported = import(text).unwrap();
    assert!(
        imported.theme.overrides.is_empty(),
        "only syntax/editor carry colours"
    );
    assert!(imported.skipped.is_empty());
}
