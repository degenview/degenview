//! What the Screenshot Studio is asked to make: which part of the chart, in which look, at which
//! proportions, on which backdrop and holding what. Only values and the rules that tie them
//! together (Size means nothing in the Chart style; a Styled capture opens at 3:2); the geometry
//! lives in [`composition`](super::composition) and the view in [`framing`](super::framing).
//!
//! The names, labels and tooltips are the ones the Studio shows, so a view never spells a choice
//! out by hand. The look (style, size, background, include) is [`StudioPrefs`], which is what is
//! remembered; the area and the framing belong to one capture and never are.

use super::framing::Framing;
use crate::i18n::tr;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// The word on the Area control.
    pub fn control_label(self) -> &'static str {
        match self {
            Self::FullChart => tr!("Full chart"),
            Self::Portion => tr!("Portion"),
            Self::EntireLayout => tr!("Entire layout"),
        }
    }

    /// What hovering the Area control's choice says.
    pub fn tooltip(self) -> &'static str {
        match self {
            Self::FullChart => tr!("The whole chart that has the focus"),
            Self::Portion => tr!("Choose a part of the charts; the window comes back with it"),
            Self::EntireLayout => tr!("Every chart, as the window arranges them"),
        }
    }

    /// The words in the footer's summary line.
    pub fn label(self) -> &'static str {
        match self {
            Self::FullChart => tr!("Full chart"),
            Self::Portion => tr!("Selected portion"),
            Self::EntireLayout => tr!("Entire layout"),
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

    pub fn label(self) -> &'static str {
        match self {
            Self::Chart => tr!("Chart"),
            Self::Styled => tr!("Styled"),
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Chart => tr!("The chart as it is on screen"),
            Self::Styled => tr!("Larger labels, a title block and the DegenView mark"),
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

    pub fn label(self) -> &'static str {
        match self {
            Self::Original => tr!("Original"),
            Self::Wide16x9 => "16:9",
            Self::Landscape3x2 => "3:2",
            Self::Square => "1:1",
            Self::Portrait9x16 => "9:16",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Original => tr!("The capture's own proportions"),
            Self::Wide16x9 => tr!("Wide landscape"),
            Self::Landscape3x2 => tr!("Landscape"),
            Self::Square => tr!("Square"),
            Self::Portrait9x16 => tr!("Tall portrait"),
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
    #[cfg(test)]
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

/// The look the Studio remembers between captures: how it is dressed, its proportions, what is
/// behind it and what it holds. Never the area, the framing or anything about one capture.
///
/// A part this version cannot read (a choice a newer version added) falls back to its default on
/// its own, so it never takes the other snapshot settings down with it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(from = "StoredStudio")]
pub struct StudioPrefs {
    pub style: Style,
    pub size: AspectRatio,
    pub background: Background,
    pub include: Include,
}

impl Default for StudioPrefs {
    /// What a first Studio opens with: Styled at 3:2, nothing behind the chart, everything in
    /// (the way Vela's documented images show it; a decision, Chart is one segment away).
    fn default() -> Self {
        Self {
            style: Style::Styled,
            size: AspectRatio::default(),
            background: Background::None,
            include: Include::default(),
        }
    }
}

/// How the look is read: every part separately, so one that is not understood is only itself lost.
#[derive(Default, Deserialize)]
#[serde(default)]
struct StoredStudio {
    style: Option<Value>,
    size: Option<Value>,
    background: Option<Value>,
    include: Option<Value>,
}

fn understood<T: DeserializeOwned>(value: Option<Value>) -> Option<T> {
    value.and_then(|value| serde_json::from_value(value).ok())
}

impl From<StoredStudio> for StudioPrefs {
    fn from(stored: StoredStudio) -> Self {
        let default = Self::default();
        Self {
            style: understood(stored.style).unwrap_or(default.style),
            size: understood(stored.size).unwrap_or(default.size),
            background: understood(stored.background).unwrap_or(default.background),
            include: understood(stored.include).unwrap_or(default.include),
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
    /// A capture of `area` as a first Studio opens it ([`StudioPrefs::default`]).
    pub fn new(area: Area) -> Self {
        Self::remembering(area, &StudioPrefs::default())
    }

    /// A capture of `area` in the remembered `look`, at the default framing.
    pub fn remembering(area: Area, look: &StudioPrefs) -> Self {
        Self {
            area,
            style: look.style,
            size: look.size,
            background: look.background,
            include: look.include,
            framing: Framing::default(),
        }
    }

    /// The part of this configuration that is remembered.
    pub fn look(&self) -> StudioPrefs {
        StudioPrefs {
            style: self.style,
            size: self.size,
            background: self.background,
            include: self.include,
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
