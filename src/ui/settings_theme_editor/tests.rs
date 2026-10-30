use super::*;
use crate::host::editor_theme_host::EditorThemeHost;
use crate::model::editor_theme::{BuiltInTheme, LineHeight, ThemeSource, TypographyOverride};
use crate::ui::editor_appearance::RestorePublished;
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::settings_script_manager::Controls;
use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
use eframe::egui::Vec2;

struct Rig {
    screen: Screen,
    host: EditorThemeHost,
    editing: Editing,
    preview: ThemePreview,
    controls: Controls,
    dialog: Option<Dialog>,
    id: ThemeId,
}

impl Rig {
    fn new(size: Vec2) -> Self {
        let screen = Screen::new(size);
        let host = EditorThemeHost::default();
        host.attach(&screen.ctx);
        let id = host
            .create("Night shift", ThemeSource::BuiltIn(BuiltInTheme::Dracula))
            .unwrap();
        let editing = Editing::new(host.theme(id).unwrap());
        Self {
            screen,
            host,
            editing,
            preview: ThemePreview::new(),
            controls: Controls::default(),
            dialog: None,
            id,
        }
    }

    /// One frame; whether Done or Back was pressed.
    fn frame(&mut self) -> (egui::FullOutput, bool) {
        let Self {
            screen,
            host,
            editing,
            preview,
            controls,
            dialog,
            ..
        } = self;
        let view = host.view();
        let mut done = false;
        let output = screen.run(Vec::new(), |ui| {
            done = show(ui, editing, preview, controls, dialog, host, &view);
        });
        (output, done)
    }

    fn settle(&mut self) -> egui::FullOutput {
        let mut last = self.frame().0;
        for _ in 0..3 {
            last = self.frame().0;
        }
        last
    }
}

const KEYWORD: ColorKey = ColorKey::Syntax(SyntaxRole::Keyword);

#[test]
fn search_matches_a_colour_by_its_name_its_purpose_or_its_key() {
    assert!(matches("", KEYWORD));
    assert!(matches("  ", KEYWORD));
    assert!(matches("keyword", KEYWORD));
    assert!(matches("KEYWORDS", KEYWORD), "case does not matter");
    assert!(
        matches("import", KEYWORD),
        "what it is for: the hint names import"
    );
    assert!(matches(
        "controlkeyword",
        ColorKey::Syntax(SyntaxRole::ControlKeyword)
    ));
    assert!(!matches("selection", KEYWORD));
    assert!(matches(
        "selection",
        ColorKey::Surface(SurfaceColor::Selection)
    ));
    assert!(matches(
        "selected",
        ColorKey::Surface(SurfaceColor::SelectionText)
    ));
    assert!(!matches("zzzz", ColorKey::Surface(SurfaceColor::Caret)));
}

#[test]
fn every_colour_is_in_exactly_one_group() {
    let mut seen = std::collections::HashSet::new();
    for group in SyntaxGroup::ALL {
        for key in group_keys(Group::Syntax(group)) {
            assert!(seen.insert(key.key()), "{} twice", key.key());
        }
    }
    for group in SurfaceGroup::ALL {
        for key in group_keys(Group::Surface(group)) {
            assert!(seen.insert(key.key()), "{} twice", key.key());
        }
    }
    assert_eq!(seen.len(), SyntaxRole::COUNT + SurfaceColor::COUNT);
}

#[test]
fn reset_changes_only_applies_to_what_was_done_since_opening() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let rig = Rig::new(vec2(700.0, 560.0));
    let opened = rig.host.theme(rig.id).unwrap();
    assert!(!rig.editing.changed(&opened));
    rig.host
        .set_color(rig.id, KEYWORD, Rgba::hex(0x010203))
        .unwrap();
    let changed = rig.host.theme(rig.id).unwrap();
    assert!(rig.editing.changed(&changed));
    // Undo it the way the button does.
    rig.host.restore(rig.editing.entry.clone()).unwrap();
    assert!(!rig.editing.changed(&rig.host.theme(rig.id).unwrap()));
    assert_eq!(rig.host.theme(rig.id).unwrap().palette(), opened.palette());
}

#[test]
fn a_rename_is_not_a_change_to_undo() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(vec2(700.0, 560.0));
    rig.host.rename(rig.id, "Renamed").unwrap();
    rig.editing.renamed("Renamed");
    assert!(!rig.editing.changed(&rig.host.theme(rig.id).unwrap()));
    rig.host
        .set_color(rig.id, KEYWORD, Rgba::hex(0x040506))
        .unwrap();
    rig.host.restore(rig.editing.entry.clone()).unwrap();
    assert_eq!(
        rig.host.theme(rig.id).unwrap().name,
        "Renamed",
        "the name stays"
    );
    rig.settle();
}

#[test]
fn the_editor_draws_every_tab_at_every_size_in_both_appearances() {
    for app_theme in [AppTheme::Dark, AppTheme::Light] {
        let _theme = RestoreDefaultTheme;
        let _published = RestorePublished;
        for size in [
            vec2(320.0, 400.0),
            vec2(600.0, 580.0),
            vec2(760.0, 600.0),
            vec2(1100.0, 800.0),
        ] {
            for tab in [Tab::Syntax, Tab::Editor, Tab::Typography] {
                let mut rig = Rig::new(size);
                theme::apply(&rig.screen.ctx, app_theme);
                rig.host
                    .set_color(rig.id, KEYWORD, Rgba::hex(0x123456))
                    .unwrap();
                rig.editing.tab = tab;
                rig.settle();
                // Searching with nothing to find, and with something to find.
                rig.editing.query = "zzzz".into();
                rig.settle();
                rig.editing.query = "string".into();
                rig.settle();
            }
        }
    }
}

#[test]
fn done_and_back_leave_and_nothing_else_does() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(vec2(700.0, 560.0));
    let (_, done) = rig.frame();
    assert!(!done);
    let (_, done) = rig.frame();
    assert!(!done, "drawing is not leaving");
}

#[test]
fn a_theme_that_is_gone_ends_the_editor() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(vec2(700.0, 560.0));
    rig.host.delete(rig.id).unwrap();
    let (_, done) = rig.frame();
    assert!(done);
}

#[test]
fn the_typography_tab_sets_a_theme_s_own_and_resets_it() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(vec2(700.0, 560.0));
    rig.editing.tab = Tab::Typography;
    rig.host
        .set_theme_typography(
            rig.id,
            TypographyOverride {
                font_size: Some(18.0),
                line_height: Some(LineHeight::Relaxed),
                ..Default::default()
            },
        )
        .unwrap();
    rig.settle();
    let view = rig.host.view();
    assert_eq!(view.effective.font_size, 18.0);
    assert_eq!(
        view.typography.font_size, 13.0,
        "the person's own is untouched"
    );
    rig.host.reset_all(rig.id).unwrap();
    rig.settle();
    assert_eq!(rig.host.view().effective.font_size, 13.0);
}

#[test]
fn colours_that_vanish_into_the_background_are_warned_about_and_still_allowed() {
    let _theme = RestoreDefaultTheme;
    let _published = RestorePublished;
    let mut rig = Rig::new(vec2(700.0, 560.0));
    let background = BuiltInTheme::Dracula
        .palette()
        .get(ColorKey::Surface(SurfaceColor::Background));
    rig.host.set_color(rig.id, KEYWORD, background).unwrap();
    let palette = rig.host.theme(rig.id).unwrap().palette();
    assert!(low_contrast(&palette).contains(&KEYWORD));
    // The theme is saved as chosen: nothing blocked it.
    assert_eq!(
        rig.host.theme(rig.id).unwrap().palette().get(KEYWORD),
        background
    );
    rig.settle();
}

#[test]
#[ignore = "writes images for a person to look at: DEGENVIEW_GALLERY_DIR=… cargo test --lib settings_theme_editor -- --ignored"]
fn gallery() {
    if gallery_dir().is_none() {
        return;
    }
    for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let _theme = RestoreDefaultTheme;
        let _published = RestorePublished;
        for (shape, size) in [
            ("window", vec2(600.0, 580.0)),
            ("small", vec2(320.0, 420.0)),
            ("wide", vec2(1000.0, 700.0)),
        ] {
            for (tab_name, tab) in [
                ("syntax", Tab::Syntax),
                ("editor", Tab::Editor),
                ("typography", Tab::Typography),
            ] {
                let mut rig = Rig::new(size);
                theme::apply(&rig.screen.ctx, app_theme);
                rig.host
                    .set_color(rig.id, KEYWORD, Rgba::hex(0x5AF78E))
                    .unwrap();
                rig.host
                    .set_color(
                        rig.id,
                        ColorKey::Syntax(SyntaxRole::Comment),
                        BuiltInTheme::Dracula
                            .palette()
                            .get(ColorKey::Surface(SurfaceColor::Background)),
                    )
                    .unwrap();
                // A theme that sets all three, so each row shows its reset beside the control.
                rig.host
                    .set_theme_typography(
                        rig.id,
                        TypographyOverride {
                            font_family: Some("Menlo".into()),
                            font_size: Some(15.0),
                            line_height: Some(LineHeight::Relaxed),
                        },
                    )
                    .unwrap();
                rig.editing.tab = tab;
                let output = rig.settle();
                rig.screen
                    .save(&format!("theme-editor-{name}-{shape}-{tab_name}"), output);
            }
        }
    }
}

#[test]
fn the_tabs_take_the_whole_width_and_the_search_sits_under_them() {
    for width in [320.0, 600.0, 1000.0, 1300.0] {
        let _theme = RestoreDefaultTheme;
        let _published = RestorePublished;
        let mut rig = Rig::new(vec2(width, 700.0));
        rig.editing.tab = Tab::Syntax;
        rig.settle();
        let search = rig
            .screen
            .ctx
            .read_response(Id::new("editor-theme-search"))
            .unwrap_or_else(|| panic!("a search field at {width}"));
        // The editor's column is at most the page less its two insets; the search spans nearly
        // all of what there is, which it cannot while it shares a row with the tabs.
        let column = if width >= WIDE {
            (width - 2.0 * INSET - 16.0) / 2.0
        } else {
            width - 2.0 * INSET
        };
        assert!(
            search.rect.width() > column * 0.8,
            "{width}: the search is {} of a {column}-point column",
            search.rect.width()
        );
        // The Typography tab has no search and still shows its tabs.
        rig.editing.tab = Tab::Typography;
        rig.settle();
    }
}
