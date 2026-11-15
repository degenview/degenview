//! What the Screenshot Studio is asked to make: which part of the chart, in which look, at which
//! proportions, on which backdrop and holding what. Only values and the rules that tie them
//! together (Size means nothing in the Chart style; a Styled capture opens at 3:2); the geometry
//! lives in [`composition`](super::composition) and the view in [`framing`](super::framing).
//!
//! The names, labels and tooltips are the ones the Studio shows, so a view never spells a choice
//! out by hand. The look (style, size, background, include) is serde so it can be remembered;
//! the area and the framing belong to one capture and never are.

use super::framing::Framing;
use serde::{Deserialize, Serialize};

/// Which part of the workspace the image shows.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Area {
    /// The whole chart that was focused.
    #[default]
    FullChart,
    /// A rectangle the user drew over the chart or the layout.
    Portion,
    /// Every chart of the workspace, as arranged.
    EntireLayout,
}

impl Area {
    /// The words in the footer's summary line.
    pub const fn label(self) -> &'static str {
        match self {
            Self::FullChart => "Full chart",
            Self::Portion => "Selected portion",
            Self::EntireLayout => "Entire layout",
        }
    }
}

/// How the chart is dressed.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// The chart as it is on screen.
    #[default]
    Chart,
    /// Larger labels, a title block and the DegenView mark, on the proportions Size asks for.
    Styled,
}

impl Style {
    pub const ALL: [Self; 2] = [Self::Chart, Self::Styled];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Chart => "Chart",
            Self::Styled => "Styled",
        }
    }
}

/// The proportions of the finished image.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectRatio {
    /// The capture's own proportions.
    Original,
    Wide16x9,
    /// What a Styled capture opens at.
    Landscape3x2,
    Square,
    Portrait9x16,
}

impl Default for AspectRatio {
    /// The size a Styled capture opens at; the Chart style ignores it.
    fn default() -> Self {
        Self::Landscape3x2
    }
}

impl AspectRatio {
    pub const ALL: [Self; 5] = [
        Self::Original,
        Self::Wide16x9,
        Self::Landscape3x2,
        Self::Square,
        Self::Portrait9x16,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Wide16x9 => "16:9",
            Self::Landscape3x2 => "3:2",
            Self::Square => "1:1",
            Self::Portrait9x16 => "9:16",
        }
    }

    pub const fn tooltip(self) -> &'static str {
        match self {
            Self::Original => "The capture's own proportions",
            Self::Wide16x9 => "Wide landscape",
            Self::Landscape3x2 => "Landscape",
            Self::Square => "Square",
            Self::Portrait9x16 => "Tall portrait",
        }
    }

    /// The preset's proportions as the smallest whole numbers, width then height; `None` for
    /// [`Original`](Self::Original).
    pub const fn terms(self) -> Option<(u32, u32)> {
        match self {
            Self::Original => None,
            Self::Wide16x9 => Some((16, 9)),
            Self::Landscape3x2 => Some((3, 2)),
            Self::Square => Some((1, 1)),
            Self::Portrait9x16 => Some((9, 16)),
        }
    }

    /// Width over height; `None` for [`Original`](Self::Original), which is whatever the
    /// capture is.
    pub fn ratio(self) -> Option<f32> {
        self.terms()
            .map(|(width, height)| width as f32 / height as f32)
    }
}

/// What lies behind the chart.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    /// Nothing: the image is the chart edge to edge.
    #[default]
    None,
    Dots,
    Luminous,
    Radiant,
    Spectrum,
    Illuminated,
}

impl Background {
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::Dots,
        Self::Luminous,
        Self::Radiant,
        Self::Spectrum,
        Self::Illuminated,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Dots => "Dots",
            Self::Luminous => "Luminous",
            Self::Radiant => "Radiant",
            Self::Spectrum => "Spectrum",
            Self::Illuminated => "Illuminated",
        }
    }

    /// Whether the chart sits on a backdrop, inset from the image's edge with a shadow.
    pub const fn has_frame(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// What the image holds besides the candles. Each part is on until switched off.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Include {
    pub indicators: bool,
    pub drawings: bool,
    /// The DegenView mark; only a Styled image carries it.
    pub branding: bool,
}

impl Default for Include {
    fn default() -> Self {
        Self {
            indicators: true,
            drawings: true,
            branding: true,
        }
    }
}

/// Everything the Studio's controls decide about one capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StudioConfig {
    pub area: Area,
    pub style: Style,
    /// What the Size control holds. Read through [`effective_size`](Self::effective_size): in the
    /// Chart style the control is disabled and the image keeps the capture's proportions.
    pub size: AspectRatio,
    pub background: Background,
    pub include: Include,
    pub framing: Framing,
}

impl StudioConfig {
    /// A capture of `area` in the Chart style, as it opens.
    pub fn new(area: Area) -> Self {
        Self {
            area,
            style: Style::Chart,
            size: AspectRatio::default(),
            background: Background::None,
            include: Include::default(),
            framing: Framing::default(),
        }
    }

    /// Picks the look. Switching to Styled opens it at 3:2, whatever Size held before.
    pub fn set_style(&mut self, style: Style) {
        if style == Style::Styled && self.style != Style::Styled {
            self.size = AspectRatio::Landscape3x2;
        }
        self.style = style;
    }

    /// The proportions in force: always the capture's own in the Chart style.
    pub fn effective_size(&self) -> AspectRatio {
        match self.style {
            Style::Chart => AspectRatio::Original,
            Style::Styled => self.size,
        }
    }

    /// Whether the Size control does anything.
    pub fn size_enabled(&self) -> bool {
        self.style == Style::Styled
    }

    /// Whether the DegenView mark is drawn: Styled, and not switched off.
    pub fn shows_branding(&self) -> bool {
        self.style == Style::Styled && self.include.branding
    }
}

impl Default for StudioConfig {
    fn default() -> Self {
        Self::new(Area::default())
    }
}
