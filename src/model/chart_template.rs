//! Reusable chart looks. A template is a named, frozen copy of one chart's
//! [`ChartAppearance`](super::ChartAppearance): its chart type and every type's settings, its
//! colours, and the whole canvas. It is not a layout: it holds no market, timeframe, indicators,
//! scripts, drawings, alerts, paper positions, replay position or chart id, so applying it to a
//! chart restyles that chart and nothing else.
//!
//! This follows TradingView's chart templates (styling only). Saving the chart type and its
//! settings, and the choice of applying a template without them ([`ApplyScope::ColorsAndCanvas`]),
//! are DegenView's own additions.
//!
//! Pure: no egui types and no storage. [`TemplateBook`] is the one place the rules live
//! (validation, unique names, rename, duplicate, update, delete); the store reads and writes it
//! whole.

use super::canvas::{
    BackgroundAppearance, BackgroundMode, ChartCanvas, ChartMargins, LineAppearance,
    ScaleAppearance, ScaleTextSize, WatermarkAppearance, WatermarkMode,
};
use super::chart_type::LineDash;
use super::{ChartAppearance, DataSource};
use serde::{Deserialize, Serialize};

pub type TemplateId = u64;

pub const MAX_NAME_CHARS: usize = 48;
pub const MAX_DESCRIPTION_CHARS: usize = 200;

/// What applying a template changes about the chart.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ApplyScope {
    /// The chart type and every setting of it, the colours and the canvas. The price axis'
    /// decimals stay the chart's own in both scopes.
    #[default]
    AppearanceAndCanvas,
    /// The colours and the canvas. The chart keeps its own type and what its bars are built
    /// from (a Renko brick size, a Kagi reversal…), so its data does not move.
    ColorsAndCanvas,
}

impl ApplyScope {
    pub const ALL: [Self; 2] = [Self::AppearanceAndCanvas, Self::ColorsAndCanvas];

    pub const fn label(self) -> &'static str {
        match self {
            Self::AppearanceAndCanvas => "Apply appearance and canvas",
            Self::ColorsAndCanvas => "Apply colors and canvas only",
        }
    }

    pub const fn hint(self) -> &'static str {
        match self {
            Self::AppearanceAndCanvas => {
                "Takes the chart type and its settings too. Another type redraws the chart."
            }
            Self::ColorsAndCanvas => "Keeps this chart's type and how its bars are built.",
        }
    }

    /// `template` laid over `target`.
    pub fn merge(self, target: &ChartAppearance, template: &ChartAppearance) -> ChartAppearance {
        match self {
            // The price axis' decimals follow the market (a satoshi price needs eight), not the
            // look, so they always stay the chart's own, and so does how a tiny price reads.
            Self::AppearanceAndCanvas => ChartAppearance {
                axis_decimals: target.axis_decimals,
                axis_small_prices: target.axis_small_prices,
                ..*template
            },
            Self::ColorsAndCanvas => {
                let (own, theirs) = (&target.style, &template.style);
                let mut style = *theirs;
                style.chart_type = own.chart_type;
                // What the price-based bars are built from stays the target's.
                style.renko.size = own.renko.size;
                style.renko.source = own.renko.source;
                style.kagi.size = own.kagi.size;
                style.kagi.source = own.kagi.source;
                style.line_break.lines = own.line_break.lines;
                style.point_figure.size = own.point_figure.size;
                style.point_figure.reversal = own.point_figure.reversal;
                style.point_figure.source = own.point_figure.source;
                style.range.size = own.range.size;
                // What a footprint is built from stays the target's too.
                style.footprint.keep_shape_of(&own.footprint);
                style.tpo.keep_shape_of(&own.tpo);
                ChartAppearance {
                    style,
                    axis_decimals: target.axis_decimals,
                    axis_small_prices: target.axis_small_prices,
                    ..*template
                }
            }
        }
    }
}

/// A saved look.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChartAppearanceTemplate {
    pub id: TemplateId,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub appearance: ChartAppearance,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// Why a name or description was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TemplateError {
    EmptyName,
    NameTooLong,
    DescriptionTooLong,
    ControlCharacters,
    /// Another template (or a built-in) already has the name, ignoring case.
    DuplicateName,
    /// A built-in preset cannot be changed; duplicate it first.
    BuiltIn,
    Missing,
}

impl TemplateError {
    pub fn message(self) -> String {
        match self {
            Self::EmptyName => "Give the template a name.".to_owned(),
            Self::NameTooLong => format!("Names are at most {MAX_NAME_CHARS} characters."),
            Self::DescriptionTooLong => {
                format!("Descriptions are at most {MAX_DESCRIPTION_CHARS} characters.")
            }
            Self::ControlCharacters => "Names cannot contain control characters.".to_owned(),
            Self::DuplicateName => "A template with that name already exists.".to_owned(),
            Self::BuiltIn => "Built-in templates cannot be changed. Duplicate it first.".to_owned(),
            Self::Missing => "That template no longer exists.".to_owned(),
        }
    }
}

/// `raw` trimmed and checked as a template name.
pub fn clean_name(raw: &str) -> Result<String, TemplateError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(TemplateError::EmptyName);
    }
    if name.chars().any(char::is_control) {
        return Err(TemplateError::ControlCharacters);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(TemplateError::NameTooLong);
    }
    Ok(name.to_owned())
}

/// `raw` trimmed; blank is no description. Line breaks are allowed, other control characters not.
pub fn clean_description(raw: &str) -> Result<Option<String>, TemplateError> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(TemplateError::ControlCharacters);
    }
    if text.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(TemplateError::DescriptionTooLong);
    }
    Ok(Some(text.to_owned()))
}

// MARK: Built-in presets

/// A preset shipped with the app. Presets are never stored: they are built from this list each
/// time, so they cannot be edited, duplicated into the table on every launch or lost.
#[derive(Clone, Copy, Debug)]
pub struct BuiltIn {
    pub id: TemplateId,
    pub name: &'static str,
    pub description: &'static str,
    build: fn() -> ChartAppearance,
}

impl PartialEq for BuiltIn {
    /// A preset is its id: the builder is a function, which has no useful equality.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for BuiltIn {}

impl BuiltIn {
    pub fn appearance(&self) -> ChartAppearance {
        (self.build)()
    }
}

const fn grey(value: u8, alpha: u8) -> Option<[u8; 4]> {
    Some([value, value, value, alpha])
}

fn default_dark() -> ChartAppearance {
    ChartAppearance {
        canvas: ChartCanvas {
            background: BackgroundAppearance {
                mode: BackgroundMode::Solid,
                solid: Some([20, 26, 28, 255]),
                ..BackgroundAppearance::default()
            },
            vertical_grid: LineAppearance {
                color: Some([90, 105, 112, 70]),
                ..LineAppearance::default()
            },
            horizontal_grid: LineAppearance {
                color: Some([90, 105, 112, 110]),
                ..LineAppearance::default()
            },
            scales: ScaleAppearance {
                text_color: Some([152, 152, 157, 255]),
                line_color: Some([57, 68, 73, 255]),
                ..ScaleAppearance::default()
            },
            ..ChartCanvas::default()
        },
        ..ChartAppearance::default()
    }
}

fn default_light() -> ChartAppearance {
    ChartAppearance {
        canvas: ChartCanvas {
            background: BackgroundAppearance {
                mode: BackgroundMode::Solid,
                solid: Some([255, 255, 255, 255]),
                ..BackgroundAppearance::default()
            },
            vertical_grid: LineAppearance {
                color: Some([200, 203, 210, 90]),
                ..LineAppearance::default()
            },
            horizontal_grid: LineAppearance {
                color: Some([200, 203, 210, 140]),
                ..LineAppearance::default()
            },
            crosshair: LineAppearance {
                color: Some([90, 90, 98, 200]),
                ..LineAppearance::new(LineDash::Dashed)
            },
            scales: ScaleAppearance {
                text_color: Some([110, 110, 115, 255]),
                line_color: Some([208, 209, 213, 255]),
                ..ScaleAppearance::default()
            },
            ..ChartCanvas::default()
        },
        ..ChartAppearance::default()
    }
}

fn minimal() -> ChartAppearance {
    ChartAppearance {
        canvas: ChartCanvas {
            vertical_grid: LineAppearance {
                visible: false,
                ..LineAppearance::default()
            },
            horizontal_grid: LineAppearance {
                visible: false,
                ..LineAppearance::default()
            },
            crosshair: LineAppearance {
                width: 1,
                ..LineAppearance::new(LineDash::Dotted)
            },
            watermark: WatermarkAppearance::default(),
            margins: ChartMargins {
                top: 12,
                bottom: 12,
                right: 6,
            },
            ..ChartCanvas::default()
        },
        ..ChartAppearance::default()
    }
}

fn high_contrast() -> ChartAppearance {
    ChartAppearance {
        bullish: [0, 230, 118],
        bearish: [255, 61, 87],
        canvas: ChartCanvas {
            background: BackgroundAppearance {
                mode: BackgroundMode::Solid,
                solid: Some([0, 0, 0, 255]),
                ..BackgroundAppearance::default()
            },
            vertical_grid: LineAppearance {
                color: grey(255, 36),
                ..LineAppearance::default()
            },
            horizontal_grid: LineAppearance {
                color: grey(255, 56),
                ..LineAppearance::default()
            },
            pane_separator: LineAppearance {
                color: grey(255, 140),
                width: 2,
                ..LineAppearance::default()
            },
            crosshair: LineAppearance {
                color: grey(255, 230),
                width: 2,
                ..LineAppearance::new(LineDash::Dashed)
            },
            scales: ScaleAppearance {
                text_color: grey(255, 255),
                text_size: ScaleTextSize::Pt12,
                line_color: grey(255, 160),
            },
            ..ChartCanvas::default()
        },
        ..ChartAppearance::default()
    }
}

fn presentation() -> ChartAppearance {
    ChartAppearance {
        canvas: ChartCanvas {
            background: BackgroundAppearance {
                mode: BackgroundMode::Gradient,
                top: Some([22, 28, 36, 255]),
                bottom: Some([30, 50, 92, 255]),
                ..BackgroundAppearance::default()
            },
            horizontal_grid: LineAppearance {
                color: grey(255, 22),
                ..LineAppearance::default()
            },
            vertical_grid: LineAppearance {
                visible: false,
                ..LineAppearance::default()
            },
            watermark: WatermarkAppearance {
                mode: WatermarkMode::Symbol,
                color: grey(255, 20),
            },
            scales: ScaleAppearance {
                text_size: ScaleTextSize::Pt14,
                ..ScaleAppearance::default()
            },
            ..ChartCanvas::default()
        },
        ..ChartAppearance::default()
    }
}

/// The presets, in the order the menu lists them. Their ids are small and fixed, below any id
/// [`new_user_id`] hands out.
pub const BUILT_INS: [BuiltIn; 5] = [
    BuiltIn {
        id: 1,
        name: "Default Dark",
        description: "The dark palette, with its colors set so they stay put in a light app.",
        build: default_dark,
    },
    BuiltIn {
        id: 2,
        name: "Default Light",
        description: "A white chart with soft grid lines, whatever the app theme.",
        build: default_light,
    },
    BuiltIn {
        id: 3,
        name: "Minimal",
        description: "No grid, a dotted crosshair and generous margins.",
        build: minimal,
    },
    BuiltIn {
        id: 4,
        name: "High Contrast",
        description: "Black, bright candles, strong lines and larger scale text.",
        build: high_contrast,
    },
    BuiltIn {
        id: 5,
        name: "Presentation",
        description: "A gradient backdrop, a symbol watermark and larger scale text.",
        build: presentation,
    },
];

/// Ids user templates get start above this, so a preset's id is never one of theirs.
pub const FIRST_USER_ID: TemplateId = 1_000;

pub fn built_in(id: TemplateId) -> Option<&'static BuiltIn> {
    BUILT_INS.iter().find(|preset| preset.id == id)
}

// MARK: The collection

/// One entry of the template menu: a preset or a saved template.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateEntry {
    pub id: TemplateId,
    pub name: String,
    pub description: Option<String>,
    pub appearance: ChartAppearance,
    pub built_in: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The user's saved templates, in the order they were made, and every rule about changing them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TemplateBook {
    templates: Vec<ChartAppearanceTemplate>,
}

impl TemplateBook {
    pub fn new(templates: Vec<ChartAppearanceTemplate>) -> Self {
        Self { templates }
    }

    pub fn saved(&self) -> &[ChartAppearanceTemplate] {
        &self.templates
    }

    /// The presets, then the saved templates.
    pub fn entries(&self) -> Vec<TemplateEntry> {
        BUILT_INS
            .iter()
            .map(|preset| TemplateEntry {
                id: preset.id,
                name: preset.name.to_owned(),
                description: Some(preset.description.to_owned()),
                appearance: preset.appearance(),
                built_in: true,
                created_at: 0,
                updated_at: 0,
            })
            .chain(self.templates.iter().map(|template| TemplateEntry {
                id: template.id,
                name: template.name.clone(),
                description: template.description.clone(),
                appearance: template.appearance,
                built_in: false,
                created_at: template.created_at,
                updated_at: template.updated_at,
            }))
            .collect()
    }

    pub fn entry(&self, id: TemplateId) -> Option<TemplateEntry> {
        self.entries().into_iter().find(|entry| entry.id == id)
    }

    fn name_taken(&self, name: &str, except: Option<TemplateId>) -> bool {
        let lowered = name.to_lowercase();
        BUILT_INS
            .iter()
            .any(|preset| preset.name.to_lowercase() == lowered)
            || self.templates.iter().any(|template| {
                Some(template.id) != except && template.name.to_lowercase() == lowered
            })
    }

    /// `base`, or `base 2`, `base 3`… the first that is free. A name already at the length limit
    /// loses its end to make room for the number.
    pub fn free_name(&self, base: &str) -> String {
        let base = base.trim();
        if !self.name_taken(base, None) {
            return base.to_owned();
        }
        (2..)
            .map(|number| {
                let suffix = format!(" {number}");
                let room = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
                let head: String = base.chars().take(room).collect();
                format!("{}{suffix}", head.trim_end())
            })
            .find(|name| !self.name_taken(name, None))
            .expect("some number is free")
    }

    /// Saves a frozen copy of `appearance`. `id` comes from [`new_user_id`].
    pub fn save(
        &mut self,
        id: TemplateId,
        name: &str,
        description: &str,
        appearance: &ChartAppearance,
        now_ms: i64,
    ) -> Result<TemplateId, TemplateError> {
        let name = clean_name(name)?;
        let description = clean_description(description)?;
        if self.name_taken(&name, None) {
            return Err(TemplateError::DuplicateName);
        }
        self.templates.push(ChartAppearanceTemplate {
            id,
            name,
            description,
            appearance: *appearance,
            created_at: now_ms,
            updated_at: now_ms,
        });
        Ok(id)
    }

    fn saved_mut(&mut self, id: TemplateId) -> Result<&mut ChartAppearanceTemplate, TemplateError> {
        if built_in(id).is_some() {
            return Err(TemplateError::BuiltIn);
        }
        self.templates
            .iter_mut()
            .find(|template| template.id == id)
            .ok_or(TemplateError::Missing)
    }

    pub fn rename(
        &mut self,
        id: TemplateId,
        name: &str,
        description: &str,
        now_ms: i64,
    ) -> Result<(), TemplateError> {
        let name = clean_name(name)?;
        let description = clean_description(description)?;
        self.saved_mut(id)?;
        if self.name_taken(&name, Some(id)) {
            return Err(TemplateError::DuplicateName);
        }
        let template = self.saved_mut(id)?;
        template.name = name;
        template.description = description;
        template.updated_at = now_ms;
        Ok(())
    }

    /// Replaces a template's look with `appearance`, keeping its name. Explicit: nothing calls
    /// this but the "Update from this chart" action.
    pub fn update_appearance(
        &mut self,
        id: TemplateId,
        appearance: &ChartAppearance,
        now_ms: i64,
    ) -> Result<(), TemplateError> {
        let template = self.saved_mut(id)?;
        template.appearance = *appearance;
        template.updated_at = now_ms;
        Ok(())
    }

    /// A copy of a saved template or a preset, named `<name> copy` (numbered when taken), placed
    /// after the others. The copy is editable.
    pub fn duplicate(
        &mut self,
        id: TemplateId,
        new_id: TemplateId,
        now_ms: i64,
    ) -> Result<TemplateId, TemplateError> {
        let source = self.entry(id).ok_or(TemplateError::Missing)?;
        let name = self.free_name(&format!("{} copy", source.name));
        self.templates.push(ChartAppearanceTemplate {
            id: new_id,
            name,
            description: source.description.filter(|_| !source.built_in),
            appearance: source.appearance,
            created_at: now_ms,
            updated_at: now_ms,
        });
        Ok(new_id)
    }

    pub fn delete(&mut self, id: TemplateId) -> Result<(), TemplateError> {
        self.saved_mut(id)?;
        self.templates.retain(|template| template.id != id);
        Ok(())
    }
}

// MARK: Apply to all

/// How a look is given to every other chart of a workspace, decided before anything changes so a
/// chart is never left half-updated: each target gets the scope its market allows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyPlan {
    /// The charts to restyle, with what each takes.
    pub steps: Vec<(u64, ApplyScope)>,
    /// How many of them keep their own chart type because their market cannot draw the look's.
    pub kept_type: usize,
}

impl ApplyPlan {
    /// One line for the sheet's footer.
    pub fn message(&self, chart_type_label: &str) -> String {
        let noun = |count: usize| if count == 1 { "chart" } else { "charts" };
        if self.steps.is_empty() {
            return "There are no other charts in this workspace.".to_owned();
        }
        let mut text = format!(
            "Applied the appearance to {} {}.",
            self.steps.len(),
            noun(self.steps.len())
        );
        if self.kept_type > 0 {
            text.push_str(&format!(
                " {} kept {} chart type: their market cannot draw {chart_type_label}.",
                self.kept_type,
                if self.kept_type == 1 { "its" } else { "their" },
            ));
        }
        text
    }
}

/// Plans giving `look` to every chart in `charts` (id and market) except `source`. A chart whose
/// market cannot draw the look's chart type, or is drawn as a line and has no types at all, takes
/// the colors and canvas only; the first kind is counted so it can be reported.
pub fn plan_apply_to_all(
    charts: &[(u64, DataSource)],
    look: &ChartAppearance,
    scope: ApplyScope,
    source: u64,
) -> ApplyPlan {
    let mut kept_type = 0;
    let steps = charts
        .iter()
        .filter(|(id, _)| *id != source)
        .map(|(id, market)| {
            let takes_type = scope == ApplyScope::AppearanceAndCanvas
                && !market.is_line_chart()
                && look.style.chart_type.supported_by(*market);
            if scope == ApplyScope::AppearanceAndCanvas && !takes_type && !market.is_line_chart() {
                kept_type += 1;
            }
            (
                *id,
                if takes_type {
                    ApplyScope::AppearanceAndCanvas
                } else {
                    ApplyScope::ColorsAndCanvas
                },
            )
        })
        .collect();
    ApplyPlan { steps, kept_type }
}

/// An id for a new saved template: wall-clock based like every document id, and never one of a
/// preset's.
pub fn new_user_id() -> TemplateId {
    super::watchlist::new_id().max(FIRST_USER_ID)
}

#[cfg(test)]
mod tests;
