//! What the Pine editor looks like right now, resolved from a theme into the colours egui paints
//! with. One [`Appearance`] is built per theme and shared (`Arc`), so painting a token is an array
//! lookup, and a colour change reaches every code surface by replacing the `Arc`.
//!
//! The editor reads [`current`] once per frame. Surfaces that sit on the app's own background
//! (the change popover, chat code blocks, the AI review) read [`chrome`] instead: Dark or Light
//! by the app's appearance whatever theme the editor wears, since Dracula's yellow on a white
//! popover would be unreadable.

use crate::model::editor_theme::{BuiltInTheme, Rgba, SurfaceColor, SyntaxRole};
use crate::ui::theme;
use eframe::egui::Color32;
use std::sync::{Arc, OnceLock};

/// A theme's colours, one per role and surface.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    roles: [Color32; SyntaxRole::COUNT],
    surfaces: [Color32; SurfaceColor::COUNT],
}

fn color(rgba: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(rgba.r, rgba.g, rgba.b, rgba.a)
}

impl Appearance {
    pub fn of(theme: BuiltInTheme) -> Self {
        Self {
            roles: SyntaxRole::ALL.map(|role| color(theme.syntax(role))),
            surfaces: SurfaceColor::ALL.map(|surface| color(theme.surface(surface))),
        }
    }

    pub fn role(&self, role: SyntaxRole) -> Color32 {
        self.roles[role.index()]
    }

    pub fn surface(&self, surface: SurfaceColor) -> Color32 {
        self.surfaces[surface.index()]
    }
}

fn built_in(theme: BuiltInTheme) -> Arc<Appearance> {
    static DARK: OnceLock<Arc<Appearance>> = OnceLock::new();
    static LIGHT: OnceLock<Arc<Appearance>> = OnceLock::new();
    let cell = match theme {
        BuiltInTheme::Dark => &DARK,
        BuiltInTheme::Light => &LIGHT,
    };
    cell.get_or_init(|| Arc::new(Appearance::of(theme))).clone()
}

/// The built-in theme that matches the app's appearance: what the editor wore before themes.
fn automatic() -> BuiltInTheme {
    if theme::is_light() {
        BuiltInTheme::Light
    } else {
        BuiltInTheme::Dark
    }
}

/// The appearance of the Pine editor.
pub fn current() -> Arc<Appearance> {
    built_in(automatic())
}

/// The appearance for code shown on the app's own background.
pub fn chrome() -> Arc<Appearance> {
    built_in(automatic())
}

#[cfg(test)]
mod tests;
