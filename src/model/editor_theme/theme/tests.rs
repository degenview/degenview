use super::*;
use crate::model::editor_theme::{SurfaceColor, SyntaxRole, TypographyOverride};

const KEYWORD: ColorKey = ColorKey::Syntax(SyntaxRole::Keyword);
const BACKGROUND: ColorKey = ColorKey::Surface(SurfaceColor::Background);

fn book_with_one() -> (ThemeBook, ThemeId) {
    let mut book = ThemeBook::default();
    let id = book
        .create(
            2_000,
            "Mine",
            ThemeSource::BuiltIn(BuiltInTheme::Dracula),
            10,
        )
        .unwrap();
    (book, id)
}

#[test]
fn a_new_theme_starts_as_its_source_looks() {
    let (book, id) = book_with_one();
    let theme = book.get(id).unwrap();
    assert_eq!(theme.base_theme(), BuiltInTheme::Dracula);
    assert_eq!(theme.palette(), BuiltInTheme::Dracula.palette());
    assert!(theme.overrides.is_empty());
    assert_eq!(theme.override_count(), 0);
}

#[test]
fn an_override_is_laid_over_the_base_and_reset_gives_the_base_back() {
    let (mut book, id) = book_with_one();
    let pink = Rgba::hex(0x123456);
    book.set_color(id, KEYWORD, pink, 11).unwrap();
    let theme = book.get(id).unwrap();
    assert_eq!(theme.palette().get(KEYWORD), pink);
    assert!(theme.is_overridden(KEYWORD));
    // Everything else is inherited.
    assert_eq!(
        theme.palette().get(BACKGROUND),
        BuiltInTheme::Dracula.palette().get(BACKGROUND)
    );
    assert_eq!(theme.override_count(), 1);
    assert_eq!(theme.updated_at, 11);

    book.reset_colors(id, &[KEYWORD], 12).unwrap();
    let theme = book.get(id).unwrap();
    assert_eq!(
        theme.palette().get(KEYWORD),
        BuiltInTheme::Dracula.palette().get(KEYWORD),
        "back to the base's colour, not a fixed default"
    );
    assert!(!theme.is_overridden(KEYWORD));
}

#[test]
fn setting_the_base_colour_is_no_override() {
    let (mut book, id) = book_with_one();
    let base = BuiltInTheme::Dracula.palette().get(KEYWORD);
    book.set_color(id, KEYWORD, Rgba::hex(0x010203), 1).unwrap();
    book.set_color(id, KEYWORD, base, 2).unwrap();
    assert!(book.get(id).unwrap().overrides.is_empty());
}

#[test]
fn a_custom_source_is_flattened_so_nothing_points_at_another_custom_theme() {
    let (mut book, first) = book_with_one();
    book.set_color(first, KEYWORD, Rgba::hex(0x00FF00), 1)
        .unwrap();
    book.set_typography(
        first,
        TypographyOverride {
            font_size: Some(18.0),
            ..Default::default()
        },
        1,
    )
    .unwrap();
    let second = book
        .create(2_001, "Child", ThemeSource::Custom(first), 2)
        .unwrap();
    let child = book.get(second).unwrap().clone();
    assert_eq!(
        child.base,
        BuiltInTheme::Dracula.id(),
        "the base, not the parent"
    );
    assert_eq!(child.palette(), book.get(first).unwrap().palette());
    assert_eq!(child.typography.font_size, Some(18.0));
    // Deleting the parent leaves the child whole.
    book.delete(first).unwrap();
    assert_eq!(book.get(second).unwrap().palette(), child.palette());
}

#[test]
fn duplicating_names_the_copy_and_numbers_it_when_taken() {
    let (mut book, id) = book_with_one();
    let one = book.duplicate(2_001, ThemeSource::Custom(id), 3).unwrap();
    let two = book.duplicate(2_002, ThemeSource::Custom(id), 4).unwrap();
    assert_eq!(book.get(one).unwrap().name, "Mine copy");
    assert_eq!(book.get(two).unwrap().name, "Mine copy 2");
    let from_built_in = book
        .duplicate(2_003, ThemeSource::BuiltIn(BuiltInTheme::Light), 5)
        .unwrap();
    assert_eq!(book.get(from_built_in).unwrap().name, "Light copy");
    assert_eq!(
        book.get(from_built_in).unwrap().base_theme(),
        BuiltInTheme::Light
    );
}

#[test]
fn names_are_trimmed_unique_and_checked() {
    let mut book = ThemeBook::default();
    let make = |book: &mut ThemeBook, name: &str| {
        book.create(
            2_000 + book.saved().len() as u64,
            name,
            ThemeSource::BuiltIn(BuiltInTheme::Dark),
            0,
        )
    };
    assert_eq!(make(&mut book, ""), Err(ThemeError::EmptyName));
    assert_eq!(make(&mut book, "   \t "), Err(ThemeError::EmptyName));
    assert_eq!(
        make(&mut book, "a\u{7}b"),
        Err(ThemeError::ControlCharacters)
    );
    assert_eq!(make(&mut book, "a\nb"), Err(ThemeError::ControlCharacters));
    assert_eq!(
        make(&mut book, &"x".repeat(MAX_NAME_CHARS + 1)),
        Err(ThemeError::NameTooLong)
    );
    assert!(make(&mut book, &"x".repeat(MAX_NAME_CHARS)).is_ok());
    // Unicode is a name, and so are emoji; the limit counts characters, not bytes.
    assert!(make(&mut book, "  Nacht 🌙 ночь  ").is_ok());
    assert_eq!(book.saved()[1].name, "Nacht 🌙 ночь");
    assert!(make(&mut book, &"é".repeat(MAX_NAME_CHARS)).is_ok());
    // Names are unique whatever the case, built-ins' and the automatic tile's included.
    assert_eq!(
        make(&mut book, "nacht 🌙 НОЧЬ"),
        Err(ThemeError::DuplicateName)
    );
    for taken in ["dark", "LIGHT", "Dracula", "automatic"] {
        assert_eq!(
            make(&mut book, taken),
            Err(ThemeError::DuplicateName),
            "{taken}"
        );
    }
}

#[test]
fn a_name_can_be_checked_before_it_is_used() {
    let (book, id) = book_with_one();
    assert_eq!(book.check_name("  Fresh  ", None), Ok("Fresh".to_owned()));
    assert_eq!(
        book.check_name("mine", None),
        Err(ThemeError::DuplicateName)
    );
    assert_eq!(
        book.check_name("mine", Some(id)),
        Ok("mine".to_owned()),
        "its own name"
    );
    assert_eq!(book.check_name("", None), Err(ThemeError::EmptyName));
    assert_eq!(
        book.check_name("Dark", Some(id)),
        Err(ThemeError::DuplicateName)
    );
}

#[test]
fn free_names_count_up_and_respect_the_length_limit() {
    let (book, _) = book_with_one();
    assert_eq!(book.free_name("Mine"), "Mine 2");
    assert_eq!(book.free_name("Fresh"), "Fresh");
    assert_eq!(book.free_name("Dark"), "Dark 2");
    let long = "y".repeat(MAX_NAME_CHARS);
    let mut book = ThemeBook::default();
    book.create(2_000, &long, ThemeSource::BuiltIn(BuiltInTheme::Dark), 0)
        .unwrap();
    let next = book.free_name(&long);
    assert_eq!(next.chars().count(), MAX_NAME_CHARS);
    assert!(next.ends_with(" 2"));
}

#[test]
fn rename_keeps_its_own_name_and_refuses_another_s() {
    let (mut book, first) = book_with_one();
    let second = book
        .create(2_001, "Other", ThemeSource::BuiltIn(BuiltInTheme::Dark), 0)
        .unwrap();
    assert_eq!(
        book.rename(second, "mine", 1),
        Err(ThemeError::DuplicateName)
    );
    assert_eq!(
        book.rename(first, "MINE", 2),
        Ok(()),
        "its own name, in another case"
    );
    assert_eq!(book.get(first).unwrap().name, "MINE");
    assert_eq!(book.rename(second, "  Renamed ", 3), Ok(()));
    assert_eq!(book.get(second).unwrap().name, "Renamed");
    assert_eq!(book.rename(999, "x", 4), Err(ThemeError::Missing));
}

#[test]
fn built_ins_are_not_in_the_book_so_they_cannot_be_changed() {
    let (mut book, _) = book_with_one();
    for id in 0..FIRST_USER_ID {
        assert_eq!(book.delete(id), Err(ThemeError::Missing));
        assert_eq!(
            book.set_color(id, KEYWORD, Rgba::CLEAR, 0),
            Err(ThemeError::Missing)
        );
        assert_eq!(book.rename(id, "Nope", 0), Err(ThemeError::Missing));
    }
    assert!(new_user_id() >= FIRST_USER_ID);
    assert_eq!(book.saved().len(), 1);
}

#[test]
fn reset_all_clears_colours_and_typography() {
    let (mut book, id) = book_with_one();
    book.set_color(id, KEYWORD, Rgba::hex(0x010203), 1).unwrap();
    book.set_color(id, BACKGROUND, Rgba::hex(0x040506), 1)
        .unwrap();
    book.set_typography(
        id,
        TypographyOverride {
            font_family: Some("Menlo".into()),
            ..Default::default()
        },
        1,
    )
    .unwrap();
    book.reset_all(id, 2).unwrap();
    let theme = book.get(id).unwrap();
    assert!(theme.overrides.is_empty());
    assert!(theme.typography.is_empty());
    assert_eq!(theme.palette(), BuiltInTheme::Dracula.palette());
}

#[test]
fn restore_puts_a_theme_back_as_it_was() {
    let (mut book, id) = book_with_one();
    let before = book.get(id).unwrap().clone();
    book.set_color(id, KEYWORD, Rgba::hex(0x010203), 1).unwrap();
    book.restore(before.clone()).unwrap();
    assert_eq!(book.get(id).unwrap(), &before);
}

#[test]
fn there_is_a_ceiling_on_themes() {
    let mut book = ThemeBook::default();
    for number in 0..MAX_THEMES {
        book.create(
            2_000 + number as u64,
            &format!("T{number}"),
            ThemeSource::BuiltIn(BuiltInTheme::Dark),
            0,
        )
        .unwrap();
    }
    assert_eq!(
        book.create(
            9_999,
            "One more",
            ThemeSource::BuiltIn(BuiltInTheme::Dark),
            0
        ),
        Err(ThemeError::TooMany)
    );
    assert_eq!(
        book.duplicate(9_999, ThemeSource::Custom(2_000), 0),
        Err(ThemeError::TooMany)
    );
}

#[test]
fn unreadable_overrides_are_kept_but_never_used() {
    let text = r##"{"id":2000,"name":"Old","base":"builtin.light",
        "overrides":{"keyword":"#112233","comment":"not a colour","futureRole":"#445566"},
        "somethingNew":{"a":1}}"##;
    let theme: EditorTheme = serde_json::from_str(text).unwrap();
    let palette = theme.palette();
    assert_eq!(palette.get(KEYWORD), Rgba::hex(0x112233));
    assert_eq!(
        palette.get(ColorKey::Syntax(SyntaxRole::Comment)),
        BuiltInTheme::Light
            .palette()
            .get(ColorKey::Syntax(SyntaxRole::Comment)),
        "an unreadable colour inherits"
    );
    assert_eq!(theme.override_count(), 1);
    let written = serde_json::to_value(&theme).unwrap();
    assert_eq!(written["overrides"]["futureRole"], "#445566", "kept");
    assert_eq!(written["overrides"]["comment"], "not a colour", "kept");
    assert_eq!(written["somethingNew"]["a"], 1, "kept");
}

#[test]
fn a_theme_with_little_in_it_still_reads() {
    let theme: EditorTheme = serde_json::from_str(r#"{"id":2000,"name":"Bare"}"#).unwrap();
    assert_eq!(theme.base_theme(), BuiltInTheme::Dark);
    assert_eq!(theme.palette(), BuiltInTheme::Dark.palette());
    assert_eq!(theme.version, FORMAT_VERSION);
    let theme: EditorTheme = serde_json::from_str(
        r#"{"id":2000,"name":"Odd","base":"builtin.from-the-future","overrides":[1,2],"typography":"x"}"#,
    )
    .unwrap();
    assert_eq!(
        theme.base_theme(),
        BuiltInTheme::Dark,
        "an unknown base reads as Dark"
    );
    assert!(theme.overrides.is_empty());
    assert!(theme.typography.is_empty());
}
