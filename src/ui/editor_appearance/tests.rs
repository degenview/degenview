use super::*;
use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
use eframe::egui;

/// A context in `app_theme`, with the app's visuals installed (the editor's selection and caret
/// colours are the visuals').
fn context(app_theme: AppTheme) -> egui::Context {
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    theme::apply(&ctx, app_theme);
    ctx
}

/// A system colour as the editor's `theme::syntax` table gave it: one RGB per appearance.
fn system(dark: (u8, u8, u8), light: (u8, u8, u8)) -> Color32 {
    let (r, g, b) = if theme::is_light() { light } else { dark };
    Color32::from_rgb(r, g, b)
}

/// The colours the editor drew before themes existed: the `theme::syntax` table as it was (the
/// entries nothing else uses are written out here), and the `theme` and visuals values its
/// painting code called.
fn before_themes(ctx: &egui::Context) -> Appearance {
    use SyntaxRole as R;
    let syntax = theme::syntax::string();
    let constant = system((0x63, 0xE6, 0xE2), (0x00, 0xA8, 0xA0));
    let keyword = theme::syntax::keyword();
    let mut roles = [theme::text_primary(); SyntaxRole::COUNT];
    for (role, color) in [
        (R::Comment, system((152, 152, 157), (110, 110, 115))),
        (R::String, syntax),
        (R::Escape, syntax),
        (R::Number, theme::syntax::number()),
        (R::Literal, constant),
        (R::BuiltinVariable, theme::syntax::variable()),
        (R::BuiltinConstant, constant),
        (R::Namespace, system((0x40, 0xC8, 0xE0), (0x30, 0xB0, 0xC7))),
        (R::BuiltinFunction, theme::syntax::function()),
        (R::Keyword, keyword),
        (R::ControlKeyword, keyword),
        (R::Type, system((0x5E, 0x5C, 0xE6), (0x58, 0x56, 0xD6))),
        (R::Operator, system((0x8E, 0x8E, 0x93), (0x8E, 0x8E, 0x93))),
        (R::Directive, system((0xAC, 0x8E, 0x68), (0xA2, 0x84, 0x5E))),
        (
            R::ColorLiteral,
            system((0xFF, 0x37, 0x5F), (0xFF, 0x2D, 0x55)),
        ),
    ] {
        roles[role.index()] = color;
    }
    let visuals = ctx.global_style().visuals.clone();
    let mut surfaces = [Color32::TRANSPARENT; SurfaceColor::COUNT];
    for (surface, color) in [
        (SurfaceColor::Background, theme::editor_background()),
        (SurfaceColor::CurrentLine, theme::ink(15)),
        (SurfaceColor::Selection, visuals.selection.bg_fill),
        (SurfaceColor::Caret, visuals.text_cursor.stroke.color),
        (SurfaceColor::BracketMatch, theme::ink(56)),
        (
            SurfaceColor::WordHighlight,
            Color32::from_rgba_unmultiplied(63, 99, 148, 178),
        ),
        (
            SurfaceColor::FindMatch,
            theme::with_alpha(theme::YELLOW, 0.28),
        ),
        (
            SurfaceColor::FindMatchActive,
            theme::with_alpha(theme::YELLOW, 0.6),
        ),
        (SurfaceColor::IndentGuide, theme::ink(22)),
        (
            SurfaceColor::IndentGuideActive,
            theme::text_secondary().gamma_multiply(0.6),
        ),
        (SurfaceColor::GutterBackground, theme::gutter_background()),
        (SurfaceColor::LineNumber, theme::text_secondary()),
        (SurfaceColor::LineNumberActive, theme::text_secondary()),
        (SurfaceColor::DiagnosticError, syntax),
        (SurfaceColor::DiagnosticWarning, theme::YELLOW),
        (SurfaceColor::PopupBackground, theme::chrome()),
        (SurfaceColor::PopupSelection, theme::SYSTEM_ACCENT),
    ] {
        surfaces[surface.index()] = color;
    }
    Appearance { roles, surfaces }
}

#[test]
fn dark_and_light_match_the_editor_before_themes() {
    for (app_theme, built_in) in [
        (AppTheme::Dark, BuiltInTheme::Dark),
        (AppTheme::Light, BuiltInTheme::Light),
    ] {
        let _restore = RestoreDefaultTheme;
        let ctx = context(app_theme);
        let expected = before_themes(&ctx);
        let actual = Appearance::of(built_in);
        for role in SyntaxRole::ALL {
            assert_eq!(
                actual.role(role),
                expected.role(role),
                "{built_in:?} {role:?}"
            );
        }
        for surface in SurfaceColor::ALL {
            assert_eq!(
                actual.surface(surface),
                expected.surface(surface),
                "{built_in:?} {surface:?}"
            );
        }
    }
}

#[test]
fn the_current_appearance_follows_the_app() {
    let _restore = RestoreDefaultTheme;
    let ctx = context(AppTheme::Dark);
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Dark));
    theme::apply(&ctx, AppTheme::Light);
    assert_eq!(*current(), Appearance::of(BuiltInTheme::Light));
    assert_eq!(*chrome(), *current());
}

#[test]
fn built_ins_are_built_once_and_shared() {
    assert!(Arc::ptr_eq(
        &built_in(BuiltInTheme::Dark),
        &built_in(BuiltInTheme::Dark)
    ));
}
