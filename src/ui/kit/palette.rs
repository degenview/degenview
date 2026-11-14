//! The system palette: the accent and status colours, the window and control plates, and the
//! tones a status reads in.

use crate::ui::theme;
use eframe::egui::Color32;

/// `Color.accentColor` with the system default (blue) accent.
pub const ACCENT: Color32 = theme::SYSTEM_ACCENT;
pub const GREEN: Color32 = Color32::from_rgb(48, 209, 88);
pub const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
pub const RED: Color32 = Color32::from_rgb(255, 69, 58);
/// `windowBackgroundColor`, behind cards and around sheets.
pub fn window() -> Color32 {
    theme::chrome()
}
/// `controlBackgroundColor`: the raised plate of a selected segment.
pub fn control() -> Color32 {
    theme::editor_background()
}
/// `.separator` at the half opacity cards use.
pub fn hairline() -> Color32 {
    theme::ink(20)
}

/// `Color.primary.opacity(x)`.
pub fn primary(opacity: f32) -> Color32 {
    theme::ink((255.0 * opacity.clamp(0.0, 1.0)).round() as u8)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Good,
    Warning,
    Bad,
    Neutral,
}

impl Tone {
    pub fn color(self) -> Color32 {
        match self {
            Self::Good => GREEN,
            Self::Warning => ORANGE,
            Self::Bad => RED,
            Self::Neutral => theme::text_secondary(),
        }
    }

    /// The tone as words on its own plate (a pill): a softened, lighter shade on dark windows and
    /// a deep one on light, so the label reads on the tint instead of glowing or washing out. The
    /// bare system colours are for dots and icons; text in them looks cheap.
    pub fn text(self) -> Color32 {
        match self {
            Self::Good => theme::pick(
                Color32::from_rgb(126, 214, 154),
                Color32::from_rgb(20, 108, 48),
            ),
            Self::Warning => theme::pick(
                Color32::from_rgb(244, 190, 112),
                Color32::from_rgb(140, 84, 0),
            ),
            Self::Bad => theme::pick(
                Color32::from_rgb(255, 142, 132),
                Color32::from_rgb(168, 30, 24),
            ),
            Self::Neutral => theme::text_secondary(),
        }
    }

    /// The tone as a small mark (a dot): a little calmer than the bare system colour, which is
    /// too hot for something that sits beside text all down a list.
    pub fn dot(self) -> Color32 {
        match self {
            Self::Good => theme::pick(
                Color32::from_rgb(98, 204, 132),
                Color32::from_rgb(40, 150, 76),
            ),
            Self::Warning => theme::pick(
                Color32::from_rgb(240, 172, 64),
                Color32::from_rgb(204, 132, 14),
            ),
            Self::Bad => theme::pick(
                Color32::from_rgb(255, 110, 100),
                Color32::from_rgb(210, 56, 48),
            ),
            Self::Neutral => theme::text_tertiary(),
        }
    }

    /// The plate behind a pill: a faint wash of the tone.
    pub fn fill(self) -> Color32 {
        let wash = theme::pick_value(0.13, 0.11);
        match self {
            Self::Neutral => theme::with_alpha(theme::text_secondary(), wash),
            tone => theme::with_alpha(tone.color(), wash),
        }
    }

    /// The hairline round a pill, so it keeps its shape on any card.
    pub fn edge(self) -> Color32 {
        theme::with_alpha(self.text(), theme::pick_value(0.26, 0.24))
    }
}
