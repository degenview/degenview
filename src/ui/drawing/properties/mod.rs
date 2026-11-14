//! The properties dialog of a drawing: Style, Text, Coordinates and Visibility, each offering
//! only what the drawing's tool supports (`Tool::caps`). It edits a working copy that the chart
//! shows live; OK commits it as one change and Cancel (or Escape) throws it away, so a dialog
//! session is at most one undo step and a cancelled one is none.

use crate::format;
use crate::model::chart::template::TemplateId;
use crate::model::drawings::style::DrawingStyle;
use crate::model::{Caps, Drawing, Geometry, Tool};
use crate::store::drawing_template::DrawingTemplateStore;
use crate::ui::chart::settings::style::{number_field, picker_row};
use crate::ui::icons::Icon;
use crate::ui::kit::date_picker;
use crate::ui::workspace::tool_sidebar::tool_icon;
use crate::ui::{
    kit::{self, sheet},
    settings::card,
    theme,
};
use eframe::egui::{self, Color32, Context, Id, Key, Stroke, Ui, pos2};
use rows::rule;

const WIDTH: f32 = 440.0;
const PAGE_HEIGHT: f32 = 380.0;
/// Room kept either side of the tab bar.
const TAB_BAR_MARGIN: f32 = 12.0;
/// The page's inset on every side, as the other sheets.
const PAGE_MARGIN: f32 = sheet::INSET as f32;
/// The vertical rhythm between the cards of a page, as on the chart settings pages.
const CARD_SPACING: f32 = 8.0;
/// Extra room above every section heading after the first.
const SECTION_GAP: f32 = 10.0;
/// The outer width of a menu picker, which the other right-hand inputs match.
const CONTROL_WIDTH: f32 = 130.0;
/// How many anchors the Coordinates tab lists; a long path keeps the rest on the chart.
const MAX_LISTED: usize = 40;

mod content;
mod coordinates;
mod fib_pages;
mod forecast_pages;
mod levels;
mod rows;
mod style;
mod templates;
mod text;
mod visibility;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tab {
    Style,
    /// The regression trend's source, deviations and lines.
    Inputs,
    /// The extra lines of a channel or a pitchfork.
    Levels,
    Text,
    /// What a content tool shows: an image's picture, name and opacity.
    Content,
    /// A Post card's link, preview and size.
    Link,
    Coordinates,
    Visibility,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Self::Style => "Style",
            Self::Inputs => "Inputs",
            Self::Levels => "Levels",
            Self::Text => "Text",
            Self::Content => "Image",
            Self::Link => "Link",
            Self::Coordinates => "Coordinates",
            Self::Visibility => "Visibility",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Style => Icon::Palette,
            Self::Inputs => Icon::ChartLine,
            Self::Levels => Icon::Layers,
            Self::Text => Icon::TextFormat,
            Self::Content => Icon::ImageTool,
            Self::Link => Icon::Link,
            Self::Coordinates => Icon::Hash,
            Self::Visibility => Icon::Eye,
        }
    }
}

/// What the dialog wants done this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Open,
    /// OK: replace the drawing with this one.
    Apply(Box<Drawing>),
    Cancel,
}

/// The text a coordinate field is being edited as: the time as the date picker reads and
/// writes it, UTC `YYYY-MM-DD HH:MM`.
#[derive(Clone, Default)]
struct CoordinateText {
    time: String,
}

/// A name being asked for: a new template's, or a template's new name.
struct Prompt {
    rename: Option<TemplateId>,
    name: String,
    error: Option<String>,
    focus: bool,
}

/// What the template menu last did, shown beside it.
struct Notice {
    text: String,
    error: bool,
}

pub struct DrawingProperties {
    pub chart_id: u64,
    draft: Drawing,
    tool: Tool,
    tab: Tab,
    coordinates: Vec<CoordinateText>,
    /// No frame has been drawn yet, so a calendar an earlier dialog left open is still to close.
    fresh: bool,
    prompt: Option<Prompt>,
    notice: Option<Notice>,
    /// The tallest page content seen, so switching tabs does not resize the dialog.
    page_height: f32,
    /// The sheet's width: `WIDTH`, or wider when the tab bar needs more.
    width: f32,
    /// The picture the Image tab previews, with the texture it was uploaded as.
    preview: Option<(String, egui::TextureHandle)>,
    /// A new picture was asked for; the workspace takes the request and runs the picker.
    replace_requested: bool,
}

/// Whether a Fibonacci or Gann tool has a list of levels to edit (the spiral has none).
fn has_fib_levels(tool: Tool) -> bool {
    crate::model::fib::features_of(tool).is_some_and(|features| !features.lists[0].is_empty())
}

/// Whether the style page has anything to show for these capabilities.
fn has_style_page(caps: Caps) -> bool {
    [
        Caps::COLOR,
        Caps::WIDTH,
        Caps::DASH,
        Caps::FILL,
        Caps::EXTEND,
        Caps::MIDDLE_LINE,
        Caps::ENDS,
        Caps::STATS,
        Caps::PRICE_LABELS,
        Caps::TIME_LABEL,
        Caps::MIDDLE_POINT,
        Caps::BOX,
        Caps::EMOJI,
    ]
    .into_iter()
    .any(|cap| caps.has(cap))
}

/// Whether the tool has inputs of its own: a regression's, a pattern's, a wave's, a position's, a
/// forecast's, a bars pattern's, a ghost feed's, a VWAP's, a profile's or a measurer's.
fn has_inputs_page(caps: Caps) -> bool {
    [
        Caps::REGRESSION,
        Caps::PATTERN,
        Caps::WAVE,
        Caps::POSITION,
        Caps::FORECAST,
        Caps::BARS,
        Caps::GHOST,
        Caps::VWAP,
        Caps::PROFILE,
        Caps::MEASURE,
    ]
    .into_iter()
    .any(|cap| caps.has(cap))
}

impl DrawingProperties {
    /// A dialog for `drawing`; `None` for a ruler or a row this version cannot read, which
    /// have no properties.
    pub fn new(chart_id: u64, drawing: &Drawing) -> Option<Self> {
        let tool = drawing.tool()?;
        if drawing.is_ruler() {
            return None;
        }
        let mut dialog = Self {
            chart_id,
            draft: drawing.clone(),
            tool,
            tab: Tab::Style,
            coordinates: Vec::new(),
            fresh: true,
            prompt: None,
            notice: None,
            page_height: 0.0,
            width: WIDTH,
            preview: None,
            replace_requested: false,
        };
        dialog.tab = dialog.tabs()[0];
        dialog.reset_coordinate_text();
        Some(dialog)
    }

    #[cfg(test)]
    pub fn set_draft_text_for_test(&mut self, text: &str) {
        self.draft.set_text(text);
    }

    /// The drawing as edited so far; the chart shows it in place of the stored one.
    pub fn draft(&self) -> &Drawing {
        &self.draft
    }

    /// Whether the user asked for another picture since this was last called.
    pub fn take_replace_request(&mut self) -> bool {
        std::mem::take(&mut self.replace_requested)
    }

    /// Hands the dialog the picture a replacement stored, so the draft shows it and keeps the
    /// drawing's own size, opacity and name when the user has set one.
    pub fn replace_picture(
        &mut self,
        content: &crate::model::annotation::image_content::ImageContent,
    ) {
        if let Some(Geometry::Image(image)) = self.draft.geometry_mut() {
            image.swap_picture(content);
        }
    }

    fn tabs(&self) -> Vec<Tab> {
        let caps = self.tool.caps();
        let mut tabs = Vec::new();
        if has_style_page(caps) {
            tabs.push(Tab::Style);
        }
        if has_inputs_page(caps) {
            tabs.push(Tab::Inputs);
        }
        if caps.has(Caps::LEVELS) || has_fib_levels(self.tool) {
            tabs.push(Tab::Levels);
        }
        if caps.has(Caps::TEXT) || caps.has(Caps::AUTO_TEXT) {
            tabs.push(Tab::Text);
        }
        match self.draft.geometry() {
            Some(Geometry::Image(_)) => tabs.push(Tab::Content),
            Some(Geometry::Card { .. }) => tabs.push(Tab::Link),
            _ => {}
        }
        if !matches!(self.tool, Tool::Brush | Tool::Highlighter) {
            tabs.push(Tab::Coordinates);
        }
        tabs.push(Tab::Visibility);
        tabs
    }

    fn reset_coordinate_text(&mut self) {
        self.coordinates = self
            .draft
            .anchors()
            .iter()
            .map(|anchor| CoordinateText {
                time: format::timestamp(anchor.time_ms),
            })
            .collect();
    }

    /// Shows the dialog. Call every frame while it is open.
    pub fn show(&mut self, ctx: &Context, templates: &mut DrawingTemplateStore) -> Outcome {
        let mut outcome = Outcome::Open;
        let content = ctx.content_rect();
        let typing = ctx.egui_wants_keyboard_input();
        if self.fresh {
            self.close_calendars(ctx);
            self.fresh = false;
        }
        // Escape and Return belong to an open calendar before they reach the dialog.
        let calendar_open = self.calendar_open(ctx);
        let window = egui::Window::new("Drawing properties")
            .id(Id::new(("drawing-properties", self.draft.id())))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .movable(true)
            .constrain(true)
            .default_pos(pos2(
                (content.right() - WIDTH - 28.0).max(content.left() + 8.0),
                content.top() + 70.0,
            ))
            .frame(
                egui::Frame::new()
                    .fill(theme::chrome())
                    .corner_radius(12.0)
                    .stroke(Stroke::new(1.0, theme::ink(22)))
                    .shadow(egui::Shadow {
                        offset: [0, 12],
                        blur: 40,
                        spread: 0,
                        color: Color32::from_black_alpha(130),
                    }),
            );
        window.show(ctx, |ui| {
            self.width = self.sheet_width(ui);
            ui.set_width(self.width);
            ui.spacing_mut().item_spacing.y = 0.0;
            outcome = self.body(ui, templates);
        });
        if matches!(outcome, Outcome::Open) && !calendar_open {
            if ctx.input(|input| input.key_pressed(Key::Escape)) {
                outcome = Outcome::Cancel;
            } else if !typing && ctx.input(|input| input.key_pressed(Key::Enter)) {
                outcome = Outcome::Apply(Box::new(self.draft.clone()));
            }
        }
        if !matches!(outcome, Outcome::Open) {
            self.close_calendars(ctx);
        }
        outcome
    }

    /// The sheet is `WIDTH` wide, or wider when its tab bar (five tabs for most new tools) would
    /// not fit: the bar, the pages and the footer then share one width, so nothing is cut.
    fn sheet_width(&self, ui: &Ui) -> f32 {
        let tabs = self.tabs();
        if tabs.len() < 2 {
            return WIDTH;
        }
        let items: Vec<kit::TabItem<Tab>> = tabs.iter().map(|&tab| self.tab_item(tab)).collect();
        WIDTH.max(kit::tab_bar_regular_width(ui, &items) + 2.0 * TAB_BAR_MARGIN)
    }

    fn tab_item(&self, tab: Tab) -> kit::TabItem<Tab> {
        kit::TabItem {
            value: tab,
            title: tab.title(),
            icon: tab.icon(),
            count: 0,
        }
    }

    fn time_id(&self, index: usize) -> Id {
        Id::new(("drawing-time", self.draft.id(), index))
    }

    fn calendar_open(&self, ctx: &Context) -> bool {
        (0..self.coordinates.len().min(MAX_LISTED))
            .any(|index| date_picker::is_open(ctx, self.time_id(index)))
    }

    fn close_calendars(&self, ctx: &Context) {
        for index in 0..self.coordinates.len().min(MAX_LISTED) {
            date_picker::close(ctx, self.time_id(index));
        }
    }

    fn body(&mut self, ui: &mut Ui, templates: &mut DrawingTemplateStore) -> Outcome {
        ui.add_space(sheet::INSET as f32);
        let subtitle = "Changes show on the chart; OK keeps them";
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                tool_icon(self.tool),
                self.tool.label(),
                subtitle,
                kit::ACCENT,
            );
        });
        ui.add_space(16.0);
        let tabs = self.tabs();
        if !tabs.contains(&self.tab) {
            self.tab = tabs[0];
        }
        if tabs.len() > 1 {
            let items: Vec<kit::TabItem<Tab>> =
                tabs.iter().map(|&tab| self.tab_item(tab)).collect();
            ui.vertical_centered(|ui| {
                kit::tab_bar_regular(
                    ui,
                    Id::new(("drawing-properties-tabs", self.draft.id())),
                    &items,
                    &mut self.tab,
                );
            });
            ui.add_space(16.0);
        }
        rule(ui);
        let margin = PAGE_MARGIN as i8;
        let mut used = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("drawing-properties-page")
            .max_height(PAGE_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let page = egui::Frame::new()
                    .inner_margin(egui::Margin::same(margin))
                    .show(ui, |ui| {
                        ui.set_width(self.width - 2.0 * PAGE_MARGIN);
                        // The tallest page so far, so a tab switch does not resize the dialog.
                        ui.set_min_height((self.page_height - 2.0 * PAGE_MARGIN).max(0.0));
                        ui.spacing_mut().item_spacing.y = CARD_SPACING;
                        match self.tab {
                            Tab::Style => self.style_page(ui),
                            Tab::Inputs => self.inputs_page(ui),
                            Tab::Levels => self.levels_page(ui),
                            Tab::Text => self.text_page(ui),
                            Tab::Content => self.content_page(ui),
                            Tab::Link => self.link_page(ui),
                            Tab::Coordinates => self.coordinates_page(ui),
                            Tab::Visibility => self.visibility_page(ui),
                        }
                    });
                used = page.response.rect.height();
            });
        self.page_height = self.page_height.max(used.min(PAGE_HEIGHT));
        // The footer draws its own hairline; the template bar needs one of its own above it.
        if self.draft.style().is_some() {
            rule(ui);
            self.template_bar(ui, templates);
        }
        let mut outcome = Outcome::Open;
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, "OK", true, true).clicked() {
                outcome = Outcome::Apply(Box::new(self.draft.clone()));
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Outcome::Cancel;
            }
        });
        outcome
    }

    fn style(&self) -> DrawingStyle {
        self.draft.style().copied().unwrap_or_default()
    }

    fn set_style(&mut self, style: DrawingStyle) {
        self.draft.set_style(style);
    }
}

#[cfg(test)]
mod tests;
