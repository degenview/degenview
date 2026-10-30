//! How readable a theme's colours are on its own backgrounds. A warning, never a rule: a person
//! may choose any colours, and a deliberately faint comment is a style, so this only says which
//! colours are hard to read and leaves the choice alone.

use super::{ColorKey, Palette, Rgba, SurfaceColor, SyntaxRole};

/// Below this a colour is "unusually low" against its background. WCAG asks 4.5 for body text and 3
/// for large text; code that is deliberately dim (comments) sits around 3, so this only catches
/// colours that are close to the background.
pub const LOW_CONTRAST: f32 = 2.0;

fn linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.0;
    if value <= 0.03928 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of an opaque colour.
fn luminance(color: Rgba) -> f32 {
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

/// `foreground` laid over an opaque `background`.
fn over(foreground: Rgba, background: Rgba) -> Rgba {
    let alpha = f32::from(foreground.a) / 255.0;
    let mix = |top: u8, bottom: u8| {
        (f32::from(top) * alpha + f32::from(bottom) * (1.0 - alpha)).round() as u8
    };
    Rgba::new(
        mix(foreground.r, background.r),
        mix(foreground.g, background.g),
        mix(foreground.b, background.b),
        255,
    )
}

/// The WCAG contrast ratio, 1 (none) to 21 (black on white), of `foreground` over `background`. A
/// translucent foreground is laid over the background first.
pub fn contrast_ratio(foreground: Rgba, background: Rgba) -> f32 {
    let background = over(background, Rgba::hex(0x808080));
    let a = luminance(over(foreground, background));
    let b = luminance(background);
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

/// The colours of `palette` that are hard to read where they are drawn: syntax colours and the
/// caret on the editor's background, line numbers on the gutter, selected text on the selection.
pub fn low_contrast(palette: &Palette) -> Vec<ColorKey> {
    let surface = |surface: SurfaceColor| palette.surfaces[surface.index()];
    let background = surface(SurfaceColor::Background);
    let selection = over(surface(SurfaceColor::Selection), background);
    let mut found: Vec<ColorKey> = SyntaxRole::ALL
        .iter()
        .filter(|role| contrast_ratio(palette.syntax[role.index()], background) < LOW_CONTRAST)
        .map(|role| ColorKey::Syntax(*role))
        .collect();
    for (color, behind) in [
        (SurfaceColor::Caret, background),
        (
            SurfaceColor::LineNumber,
            surface(SurfaceColor::GutterBackground),
        ),
        (
            SurfaceColor::LineNumberActive,
            surface(SurfaceColor::GutterBackground),
        ),
        (SurfaceColor::SelectionText, selection),
    ] {
        if contrast_ratio(surface(color), behind) < LOW_CONTRAST {
            found.push(ColorKey::Surface(color));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::editor_theme::BuiltInTheme;

    #[test]
    fn the_ratio_matches_the_wcag_figures() {
        let black = Rgba::hex(0x000000);
        let white = Rgba::hex(0xFFFFFF);
        assert!((contrast_ratio(black, white) - 21.0).abs() < 0.01);
        assert!(
            (contrast_ratio(white, black) - 21.0).abs() < 0.01,
            "symmetric"
        );
        assert!((contrast_ratio(white, white) - 1.0).abs() < 0.001);
        // #767676 on white is the classic 4.54:1.
        assert!((contrast_ratio(Rgba::hex(0x767676), white) - 4.54).abs() < 0.02);
    }

    #[test]
    fn a_translucent_foreground_is_judged_where_it_lands() {
        let white = Rgba::hex(0xFFFFFF);
        let faint = Rgba::with_alpha(0x000000, 10);
        assert!(contrast_ratio(faint, white) < 1.2);
        assert!(contrast_ratio(Rgba::with_alpha(0x000000, 255), white) > 20.0);
    }

    #[test]
    fn the_built_in_themes_are_comfortable() {
        for theme in BuiltInTheme::ALL {
            assert_eq!(low_contrast(&theme.palette()), Vec::new(), "{theme:?}");
        }
    }

    #[test]
    fn colours_near_the_background_are_named() {
        let mut palette = BuiltInTheme::Dracula.palette();
        let keyword = ColorKey::Syntax(SyntaxRole::Keyword);
        // The background itself, and a slightly different grey.
        palette.set(
            keyword,
            palette.get(ColorKey::Surface(SurfaceColor::Background)),
        );
        palette.set(ColorKey::Syntax(SyntaxRole::Comment), Rgba::hex(0x2D2F3B));
        palette.set(
            ColorKey::Surface(SurfaceColor::LineNumber),
            Rgba::hex(0x21222D),
        );
        let found = low_contrast(&palette);
        assert!(found.contains(&keyword));
        assert!(found.contains(&ColorKey::Syntax(SyntaxRole::Comment)));
        assert!(found.contains(&ColorKey::Surface(SurfaceColor::LineNumber)));
        assert!(!found.contains(&ColorKey::Syntax(SyntaxRole::String)));
    }

    #[test]
    fn selected_text_is_judged_on_the_selection() {
        let mut palette = BuiltInTheme::Light.palette();
        // White text on a nearly white selection.
        palette.set(
            ColorKey::Surface(SurfaceColor::SelectionText),
            Rgba::hex(0xFFFFFF),
        );
        assert!(low_contrast(&palette).contains(&ColorKey::Surface(SurfaceColor::SelectionText)));
    }
}
