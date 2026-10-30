//! What the Pine editor looks like: the colours of its syntax and surfaces, in the built-in
//! themes. Pure: no egui types and no storage. The UI resolves a theme into an
//! [`Appearance`](crate::ui::editor_appearance::Appearance) once per change, never per token.
//!
//! Both enums are matched exhaustively by every built-in theme, so adding a role is a compile
//! error until each theme says what it looks like.

mod builtin;
pub mod codec;
mod color;
pub mod contrast;
mod palette;
pub mod prefs;
mod roles;
mod theme;
mod typography;

pub use builtin::BuiltInTheme;
pub use color::Rgba;
pub use palette::{ColorKey, Palette};
pub use prefs::{EditorPrefs, ThemeSelection};
pub use roles::{SurfaceColor, SurfaceGroup, SyntaxGroup, SyntaxRole};
pub use theme::{
    AUTOMATIC_NAME, EditorTheme, MAX_NAME_CHARS, NewTheme, ThemeBook, ThemeError, ThemeId,
    ThemeSource, clean_name, new_user_id,
};
pub use typography::{
    DEFAULT_FONT_SIZE, LineHeight, MAX_FONT_SIZE, MIN_FONT_SIZE, Typography, TypographyOverride,
    clamp_size, clean_family,
};
