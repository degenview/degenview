//! The themes the app ships. They live in code, never in storage, so an update can only improve
//! them and a user can never edit, overwrite or lose one.

use super::{Rgba, SurfaceColor, SyntaxRole};

/// A theme that ships with the app.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BuiltInTheme {
    /// What the editor has always looked like in a dark app: macOS's system colours on a deep
    /// blue-grey. Every colour here equals the one the editor drew before themes existed
    /// (`dark_and_light_match_the_editor_before_themes` in `ui::editor_appearance`).
    Dark,
    /// The same, for a light app.
    Light,
}

impl BuiltInTheme {
    pub const fn syntax(self, role: SyntaxRole) -> Rgba {
        match self {
            Self::Dark => dark_syntax(role),
            Self::Light => light_syntax(role),
        }
    }

    pub const fn surface(self, surface: SurfaceColor) -> Rgba {
        match self {
            Self::Dark => dark_surface(surface),
            Self::Light => light_surface(surface),
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
        S::PopupSelection => Rgba::hex(0x0A84FF),
    }
}

const fn light_surface(surface: SurfaceColor) -> Rgba {
    use SurfaceColor as S;
    match surface {
        S::Background => Rgba::hex(0xFFFFFF),
        S::CurrentLine => Rgba::with_alpha(0x000000, 15),
        S::Selection => Rgba::with_alpha(0xBF52CB, 89),
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
        S::PopupSelection => Rgba::hex(0x0A84FF),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_readable_on_every_background() {
        // A coarse check that no theme shipped dark text on a dark editor or the reverse.
        let luminance =
            |c: Rgba| 0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b);
        for theme in [BuiltInTheme::Dark, BuiltInTheme::Light] {
            let background = luminance(theme.surface(SurfaceColor::Background));
            let text = luminance(theme.syntax(SyntaxRole::Default));
            assert!((text - background).abs() > 120.0, "{theme:?}");
        }
    }

    #[test]
    fn the_light_selection_lets_the_code_show_through() {
        assert!(BuiltInTheme::Light.surface(SurfaceColor::Selection).a < 128);
    }
}
