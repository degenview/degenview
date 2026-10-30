//! What the Pine editor looks like right now: a theme resolved into the colours egui paints with,
//! and the font the code is set in. One [`Appearance`] is built per theme and shared (`Arc`), so
//! painting a token is an array lookup, and a change reaches every code surface by replacing the
//! `Arc`.
//!
//! The editor reads [`current`] and [`editor_font`] once per frame. Surfaces that sit on the app's
//! own background (the change popover, chat code blocks, the AI review) read [`chrome`] instead:
//! Dark or Light by the app's appearance whatever theme the editor wears, since Dracula's yellow on
//! a white popover would be unreadable. They do use the editor's font family.
//!
//! Until something publishes ([`publish`]) the editor wears Dark or Light by the app's appearance
//! at 13 points in the system monospace font, as it did before themes existed.

use crate::model::editor_theme::{
    BuiltInTheme, DEFAULT_FONT_SIZE, LineHeight, Palette, Rgba, SurfaceColor, SyntaxRole,
};
use crate::ui::theme;
use eframe::egui::{self, Color32, FontFamily, FontId};
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
        Self::from_palette(&theme.palette())
    }

    pub fn from_palette(palette: &Palette) -> Self {
        Self {
            roles: palette.syntax.map(color),
            surfaces: palette.surfaces.map(color),
        }
    }

    pub fn role(&self, role: SyntaxRole) -> Color32 {
        self.roles[role.index()]
    }

    pub fn surface(&self, surface: SurfaceColor) -> Color32 {
        self.surfaces[surface.index()]
    }
}

/// The name egui knows the editor's chosen font by.
pub const EDITOR_FAMILY: &str = "editor-code";

/// The font the code is set in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorFont {
    /// Whether a font the person chose is loaded (as the [`EDITOR_FAMILY`] family). Otherwise the
    /// system's monospace font is used.
    pub custom: bool,
    pub size: f32,
    pub line_height: LineHeight,
}

impl Default for EditorFont {
    fn default() -> Self {
        Self {
            custom: false,
            size: DEFAULT_FONT_SIZE,
            line_height: LineHeight::Default,
        }
    }
}

impl EditorFont {
    /// How much bigger or smaller than the 13 points the editor's chrome was laid out for. The
    /// popups and the gutter scale by this (within [`Self::chrome_scale`]'s limits).
    pub fn scale(&self) -> f32 {
        self.size / DEFAULT_FONT_SIZE
    }

    /// [`Self::scale`] for the editor's furniture (suggestion rows, the signature panel): it
    /// follows the text, but a 32-point font does not make a 54-point row.
    pub fn chrome_scale(&self) -> f32 {
        self.scale().clamp(0.9, 1.5)
    }

    /// The code font at `size` points, as `ctx` can draw it: the chosen family where `ctx` has
    /// loaded it, the monospace font otherwise (a context that never installed it, a test).
    pub fn at(&self, ctx: &egui::Context, size: f32) -> FontId {
        let named = FontFamily::Name(EDITOR_FAMILY.into());
        let family = if self.custom && ctx.fonts(|fonts| fonts.families().contains(&named)) {
            named
        } else {
            FontFamily::Monospace
        };
        FontId::new(size, family)
    }

    /// The font of the code itself.
    pub fn code(&self, ctx: &egui::Context) -> FontId {
        self.at(ctx, self.size)
    }

    /// The height of a line in points, or `None` for the font's own.
    pub fn line_height_points(&self) -> Option<f32> {
        self.line_height.factor().map(|factor| factor * self.size)
    }
}

/// What has been published: the theme the editor wears (`None` follows the app) and its font.
#[derive(Clone)]
struct Published {
    appearance: Option<Arc<Appearance>>,
    font: Arc<EditorFont>,
}

/// In the app the choice is process-wide. Under `cfg(test)` every test thread has its own, so a
/// test that publishes a theme can never be seen by another that paints or compares colours.
#[cfg(not(test))]
mod state {
    use super::Published;
    use std::sync::RwLock;

    static PUBLISHED: RwLock<Option<Published>> = RwLock::new(None);

    pub fn get() -> Option<Published> {
        PUBLISHED.read().expect("appearance lock poisoned").clone()
    }
    pub fn set(published: Option<Published>) {
        *PUBLISHED.write().expect("appearance lock poisoned") = published;
    }
}

#[cfg(test)]
mod state {
    use super::Published;
    use std::cell::RefCell;

    thread_local! {
        static PUBLISHED: RefCell<Option<Published>> = const { RefCell::new(None) };
    }

    pub fn get() -> Option<Published> {
        PUBLISHED.with(|cell| cell.borrow().clone())
    }
    pub fn set(published: Option<Published>) {
        PUBLISHED.with(|cell| *cell.borrow_mut() = published);
    }
}

/// Puts the editor in `appearance` (`None`: Dark or Light by the app) and `font`. Everything that
/// draws code picks it up on its next frame; the caller asks for that repaint.
pub fn publish(appearance: Option<Arc<Appearance>>, font: EditorFont) {
    state::set(Some(Published {
        appearance,
        font: Arc::new(font),
    }));
}

/// Forgets what was published: the editor is as it was before themes. For a test that publishes.
#[cfg(test)]
pub(crate) struct RestorePublished;

#[cfg(test)]
impl Drop for RestorePublished {
    fn drop(&mut self) {
        state::set(None);
    }
}

fn built_in(theme: BuiltInTheme) -> Arc<Appearance> {
    static DARK: OnceLock<Arc<Appearance>> = OnceLock::new();
    static LIGHT: OnceLock<Arc<Appearance>> = OnceLock::new();
    static DRACULA: OnceLock<Arc<Appearance>> = OnceLock::new();
    let cell = match theme {
        BuiltInTheme::Dark => &DARK,
        BuiltInTheme::Light => &LIGHT,
        BuiltInTheme::Dracula => &DRACULA,
    };
    cell.get_or_init(|| Arc::new(Appearance::of(theme))).clone()
}

/// The shared appearance of a built-in theme.
pub fn of_built_in(theme: BuiltInTheme) -> Arc<Appearance> {
    built_in(theme)
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
    state::get()
        .and_then(|published| published.appearance)
        .unwrap_or_else(|| built_in(automatic()))
}

/// The font of the Pine editor.
pub fn editor_font() -> Arc<EditorFont> {
    static DEFAULT: OnceLock<Arc<EditorFont>> = OnceLock::new();
    state::get().map_or_else(
        || {
            DEFAULT
                .get_or_init(|| Arc::new(EditorFont::default()))
                .clone()
        },
        |published| published.font,
    )
}

/// The appearance for code shown on the app's own background.
pub fn chrome() -> Arc<Appearance> {
    built_in(automatic())
}

#[cfg(test)]
mod tests;
