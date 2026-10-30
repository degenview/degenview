//! What the Pine editor looks like: the colours of its syntax and surfaces, in the built-in
//! themes. Pure: no egui types and no storage. The UI resolves a theme into an
//! [`Appearance`](crate::ui::editor_appearance::Appearance) once per change, never per token.
//!
//! Both enums are matched exhaustively by every built-in theme, so adding a role is a compile
//! error until each theme says what it looks like.

mod builtin;
mod color;
mod roles;

pub use builtin::BuiltInTheme;
pub use color::Rgba;
pub use roles::{SurfaceColor, SyntaxRole};
