//! The pieces the symbol picker and the portfolio's asset picker share: a type scale for the
//! sheet-style text, market artwork, result rows, recent markets, suggestion chips and the
//! buttons, cards and search box both sheets are made of.

pub mod art;
pub mod artwork;
pub mod recents;
pub mod results;
pub mod suggestions;
pub(in crate::ui) mod text;
pub mod widgets;

use eframe::egui::{Vec2, vec2};

/// Fixed rather than content-hugging, so switching sections never changes a sheet's size.
pub(crate) const SHEET_SIZE: Vec2 = vec2(900.0, 620.0);
pub(crate) const PADDING: f32 = 24.0;
