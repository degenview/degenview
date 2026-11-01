//! The themes the app ships. They live in code, never in storage, so an update can only improve
//! them and a user can never edit, overwrite or lose one.

use super::{Palette, Rgba, SurfaceColor, SyntaxRole};

/// A theme that ships with the app.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BuiltInTheme {
    /// What the editor has always looked like in a dark app: macOS's system colours on a deep
    /// blue-grey. Every colour here equals the one the editor drew before themes existed
    /// (`dark_matches_the_editor_before_themes` in `ui::editor_appearance`).
    Dark,
    /// The same, for a light app.
    Light,
    /// The Dracula palette, mapped as its specification and editor ports do.
    Dracula,
}

impl BuiltInTheme {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::Dracula];

    /// Stable for good: a saved selection and an exported theme's `base` name it.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Dark => "builtin.dark",
            Self::Light => "builtin.light",
            Self::Dracula => "builtin.dracula",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.id() == id)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Dracula => "Dracula",
        }
    }

    pub const fn syntax(self, role: SyntaxRole) -> Rgba {
        match self {
            Self::Dark => dark_syntax(role),
            Self::Light => light_syntax(role),
            Self::Dracula => dracula_syntax(role),
        }
    }

    pub const fn surface(self, surface: SurfaceColor) -> Rgba {
        match self {
            Self::Dark => dark_surface(surface),
            Self::Light => light_surface(surface),
            Self::Dracula => dracula_surface(surface),
        }
    }

    /// Every colour of the theme.
    pub fn palette(self) -> Palette {
        Palette {
            syntax: SyntaxRole::ALL.map(|role| self.syntax(role)),
            surfaces: SurfaceColor::ALL.map(|surface| self.surface(surface)),
        }
    }
}

// MARK: Dark and Light

/// Names a user declares, and punctuation, were the default text colour before roles existed.
const fn dark_syntax(role: SyntaxRole) -> Rgba {
    use SyntaxRole as R;
    Rgba::hex(match role {
        R::Default | R::UserVariable | R::Parameter | R::UserFunction | R::Punctuation => 0xEBEBF0,
        R::Comment => 0x98989D,
        R::String | R::Escape => 0xFF453A,
        R::Number => 0xFF9F0A,
        R::Literal | R::BuiltinConstant => 0x63E6E2,
        R::BuiltinVariable => 0x30D158,
        R::Namespace => 0x40C8E0,
        R::BuiltinFunction => 0x0A84FF,
        R::Keyword | R::ControlKeyword => 0xBF5AF2,
        R::Type => 0x5E5CE6,
        R::Operator => 0x8E8E93,
        R::Directive => 0xAC8E68,
        R::ColorLiteral => 0xFF375F,
    })
}

const fn light_syntax(role: SyntaxRole) -> Rgba {
    use SyntaxRole as R;
    Rgba::hex(match role {
        R::Default | R::UserVariable | R::Parameter | R::UserFunction | R::Punctuation => 0x1D1D1F,
        R::Comment => 0x6E6E73,
        R::String | R::Escape => 0xFF3B30,
        R::Number => 0xE88000,
        R::Literal | R::BuiltinConstant => 0x00A8A0,
        R::BuiltinVariable => 0x28A745,
        R::Namespace => 0x30B0C7,
        R::BuiltinFunction => 0x007AFF,
        R::Keyword | R::ControlKeyword => 0xAF52DE,
        R::Type => 0x5856D6,
        R::Operator => 0x8E8E93,
        R::Directive => 0xA2845E,
        R::ColorLiteral => 0xFF2D55,
    })
}

const fn dark_surface(surface: SurfaceColor) -> Rgba {
    use SurfaceColor as S;
    match surface {
        S::Background => Rgba::hex(0x181F22),
        S::CurrentLine => Rgba::with_alpha(0xFFFFFF, 15),
        S::Selection => Rgba::hex(0xBF52CB),
        S::SelectionText => Rgba::hex(0xFFFFFF),
        S::Caret => Rgba::hex(0xC0DEFF),
        S::BracketMatch => Rgba::with_alpha(0xFFFFFF, 56),
        S::BracketMatchBorder | S::GutterSeparator => Rgba::CLEAR,
        S::WordHighlight => Rgba::with_alpha(0x3F6394, 178),
        S::FindMatch => Rgba::with_alpha(0xFFD60A, 71),
        S::FindMatchActive => Rgba::with_alpha(0xFFD60A, 153),
        S::IndentGuide => Rgba::with_alpha(0xFFFFFF, 22),
        S::IndentGuideActive => Rgba::with_alpha(0x98989D, 153),
        S::GutterBackground => Rgba::hex(0x1C2428),
        S::LineNumber | S::LineNumberActive => Rgba::hex(0x98989D),
        S::DiagnosticError => Rgba::hex(0xFF453A),
        S::DiagnosticWarning => Rgba::hex(0xFFD60A),
        S::PopupBackground => Rgba::hex(0x252D31),
        S::PopupSelection => Rgba::hex(0x586269),
    }
}

const fn light_surface(surface: SurfaceColor) -> Rgba {
    use SurfaceColor as S;
    match surface {
        S::Background => Rgba::hex(0xFFFFFF),
        S::CurrentLine => Rgba::with_alpha(0x000000, 15),
        S::Selection => Rgba::with_alpha(0xBF52CB, 89),
        S::SelectionText => Rgba::hex(0xBF52CB),
        S::Caret => Rgba::hex(0x00537D),
        S::BracketMatch => Rgba::with_alpha(0x000000, 56),
        S::BracketMatchBorder | S::GutterSeparator => Rgba::CLEAR,
        S::WordHighlight => Rgba::with_alpha(0x3F6394, 178),
        S::FindMatch => Rgba::with_alpha(0xFFD60A, 71),
        S::FindMatchActive => Rgba::with_alpha(0xFFD60A, 153),
        S::IndentGuide => Rgba::with_alpha(0x000000, 22),
        S::IndentGuideActive => Rgba::with_alpha(0x6E6E73, 153),
        S::GutterBackground => Rgba::hex(0xF6F6F7),
        S::LineNumber | S::LineNumberActive => Rgba::hex(0x6E6E73),
        S::DiagnosticError => Rgba::hex(0xFF3B30),
        S::DiagnosticWarning => Rgba::hex(0xFFD60A),
        S::PopupBackground => Rgba::hex(0xECECEE),
        S::PopupSelection => Rgba::hex(0x6E6E73),
    }
}

// MARK: Dracula

/// Dracula's specification: keywords and operators pink, functions green, parameters orange,
/// strings yellow, constants purple, escapes pink, comments `#6272A4`, types and built-ins cyan,
/// variables and brackets the foreground. It names no colour for numbers (the editor ports use
/// purple, as for constants), namespaces, directives or colour literals; those are our choices.
const fn dracula_syntax(role: SyntaxRole) -> Rgba {
    use SyntaxRole as R;
    Rgba::hex(match role {
        R::Default | R::UserVariable | R::Punctuation => 0xF8F8F2,
        R::Comment => 0x6272A4,
        R::String => 0xF1FA8C,
        R::Escape | R::Keyword | R::ControlKeyword | R::Operator => 0xFF79C6,
        R::Number | R::Literal | R::BuiltinConstant | R::Directive => 0xBD93F9,
        R::Parameter | R::ColorLiteral => 0xFFB86C,
        R::BuiltinVariable | R::Namespace | R::BuiltinFunction | R::Type => 0x8BE9FD,
        R::UserFunction => 0x50FA7B,
    })
}

/// The palette gives the selection (`#44475A`) and the foreground; the other surfaces follow the
/// way the editor ports use them (the darker background for the gutter, the comment colour for
/// line numbers, orange for matches).
const fn dracula_surface(surface: SurfaceColor) -> Rgba {
    use SurfaceColor as S;
    match surface {
        S::Background => Rgba::hex(0x282A36),
        S::CurrentLine => Rgba::with_alpha(0x44475A, 120),
        S::Selection => Rgba::hex(0x44475A),
        S::SelectionText => Rgba::hex(0xF8F8F2),
        S::Caret => Rgba::hex(0xF8F8F2),
        S::BracketMatch => Rgba::with_alpha(0xBD93F9, 64),
        S::BracketMatchBorder => Rgba::with_alpha(0xBD93F9, 160),
        S::WordHighlight => Rgba::with_alpha(0x8BE9FD, 48),
        S::FindMatch => Rgba::with_alpha(0xFFB86C, 80),
        S::FindMatchActive => Rgba::with_alpha(0xFFB86C, 160),
        S::IndentGuide => Rgba::hex(0x3A3C4E),
        S::IndentGuideActive => Rgba::hex(0x6272A4),
        S::GutterBackground => Rgba::hex(0x21222C),
        S::GutterSeparator => Rgba::hex(0x3A3C4E),
        S::LineNumber => Rgba::hex(0x6272A4),
        S::LineNumberActive => Rgba::hex(0xF8F8F2),
        S::DiagnosticError => Rgba::hex(0xFF5555),
        S::DiagnosticWarning => Rgba::hex(0xFFB86C),
        S::PopupBackground => Rgba::hex(0x21222C),
        S::PopupSelection => Rgba::hex(0x44475A),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_stable_unique_and_round_trip() {
        let ids: HashSet<_> = BuiltInTheme::ALL.iter().map(|t| t.id()).collect();
        assert_eq!(ids.len(), BuiltInTheme::ALL.len());
        for theme in BuiltInTheme::ALL {
            assert_eq!(BuiltInTheme::from_id(theme.id()), Some(theme));
        }
        assert_eq!(BuiltInTheme::from_id("builtin.nope"), None);
        // Saved selections and exported files name these: they never change.
        assert_eq!(BuiltInTheme::Dark.id(), "builtin.dark");
        assert_eq!(BuiltInTheme::Light.id(), "builtin.light");
        assert_eq!(BuiltInTheme::Dracula.id(), "builtin.dracula");
    }

    #[test]
    fn dracula_follows_its_specification() {
        let d = BuiltInTheme::Dracula;
        assert_eq!(d.surface(SurfaceColor::Background), Rgba::hex(0x282A36));
        assert_eq!(d.surface(SurfaceColor::Selection), Rgba::hex(0x44475A));
        assert_eq!(d.syntax(SyntaxRole::Default), Rgba::hex(0xF8F8F2));
        assert_eq!(d.syntax(SyntaxRole::Comment), Rgba::hex(0x6272A4));
        assert_eq!(d.syntax(SyntaxRole::Keyword), Rgba::hex(0xFF79C6));
        assert_eq!(d.syntax(SyntaxRole::UserFunction), Rgba::hex(0x50FA7B));
        assert_eq!(d.syntax(SyntaxRole::Parameter), Rgba::hex(0xFFB86C));
        assert_eq!(d.syntax(SyntaxRole::String), Rgba::hex(0xF1FA8C));
        assert_eq!(d.syntax(SyntaxRole::BuiltinFunction), Rgba::hex(0x8BE9FD));
        assert_eq!(d.syntax(SyntaxRole::BuiltinConstant), Rgba::hex(0xBD93F9));
        assert_eq!(
            d.surface(SurfaceColor::DiagnosticError),
            Rgba::hex(0xFF5555)
        );
    }

    #[test]
    fn dracula_tells_user_names_from_builtins() {
        let d = BuiltInTheme::Dracula;
        assert_ne!(
            d.syntax(SyntaxRole::UserFunction),
            d.syntax(SyntaxRole::BuiltinFunction)
        );
        assert_ne!(
            d.syntax(SyntaxRole::Parameter),
            d.syntax(SyntaxRole::UserVariable)
        );
    }

    #[test]
    fn text_is_readable_on_every_background() {
        // A coarse check that no theme shipped dark text on a dark editor or the reverse.
        let luminance =
            |c: Rgba| 0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b);
        for theme in BuiltInTheme::ALL {
            let background = luminance(theme.surface(SurfaceColor::Background));
            let text = luminance(theme.syntax(SyntaxRole::Default));
            assert!((text - background).abs() > 120.0, "{theme:?}");
        }
    }

    #[test]
    fn the_light_selection_lets_the_code_show_through() {
        assert!(BuiltInTheme::Light.surface(SurfaceColor::Selection).a < 128);
    }

    #[test]
    fn the_palette_holds_every_colour() {
        let palette = BuiltInTheme::Dracula.palette();
        for role in SyntaxRole::ALL {
            assert_eq!(
                palette.syntax[role.index()],
                BuiltInTheme::Dracula.syntax(role)
            );
        }
        for surface in SurfaceColor::ALL {
            assert_eq!(
                palette.surfaces[surface.index()],
                BuiltInTheme::Dracula.surface(surface)
            );
        }
    }
}
