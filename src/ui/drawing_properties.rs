//! The properties dialog of a drawing: Style, Text, Coordinates and Visibility, each offering
//! only what the drawing's tool supports (`Tool::caps`). It edits a working copy that the chart
//! shows live; OK commits it as one change and Cancel (or Escape) throws it away, so a dialog
//! session is at most one undo step and a cancelled one is none.

use crate::format;
use crate::model::Timeframe;
use crate::model::chart_template::TemplateId;
use crate::model::chart_type::{LineDash, Rgba};
use crate::model::drawing_style::{
    DrawingStyle, HorizontalAlign, Level, LineEnd, MiddleLine, StatsPosition, TextOrientation,
    VerticalAlign,
};
use crate::model::drawing_template::style_for;
use crate::model::{
    Anchor, Caps, Drawing, Geometry, MAX_TEXT_CHARS, PitchforkVariant, Tool, Visibility,
};
use crate::store::drawing_template::DrawingTemplateStore;
use crate::ui::chart_style_settings::{number_field, number_field_sized, picker_row};
use crate::ui::icons::Icon;
use crate::ui::tool_sidebar::tool_icon;
use crate::ui::{kit, series_render, settings_card, sheet, theme};
use eframe::egui::{
    self, Color32, Context, Id, Key, Sense, Stroke, Ui, color_picker::Alpha, pos2, vec2,
};

const WIDTH: f32 = 440.0;
const PAGE_HEIGHT: f32 = 380.0;
/// The page's inset on every side, as the other sheets.
const PAGE_MARGIN: f32 = sheet::INSET as f32;
/// The vertical rhythm between the cards of a page, as on the chart settings pages.
const CARD_SPACING: f32 = 8.0;
/// Extra room above every section heading after the first.
const SECTION_GAP: f32 = 10.0;
/// The outer width of a menu picker, which the other right-hand inputs match.
const CONTROL_WIDTH: f32 = 130.0;
/// A framed text area's inner width so its outer width is `CONTROL_WIDTH`: the 1 pt border and
/// 8 pt margin on each side are 18 pt.
const TIME_FIELD_WIDTH: f32 = CONTROL_WIDTH - 18.0;
/// The price field has the stepper beside it, and the card's 14 pt gap, so it is narrower.
const PRICE_FIELD_WIDTH: f32 = CONTROL_WIDTH - 18.0 - 16.0 - 14.0;
/// How many anchors the Coordinates tab lists; a long path keeps the rest on the chart.
const MAX_LISTED: usize = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tab {
    Style,
    /// The regression trend's source, deviations and lines.
    Inputs,
    /// The extra lines of a channel or a pitchfork.
    Levels,
    Text,
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

/// The text a coordinate field is being edited as.
#[derive(Clone, Default)]
struct CoordinateText {
    time: String,
    time_invalid: bool,
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
    prompt: Option<Prompt>,
    notice: Option<Notice>,
    /// The tallest page content seen, so switching tabs does not resize the dialog.
    page_height: f32,
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
    ]
    .into_iter()
    .any(|cap| caps.has(cap))
}

/// Whether a tool's text has a vertical alignment that means something: inside a shape (top,
/// middle or bottom of it), or on a line or channel (above it, on it or below it).
fn text_has_position(tool: Tool) -> bool {
    use crate::model::ToolGroup;
    matches!(tool, Tool::Rectangle | Tool::Circle | Tool::Ellipse)
        || matches!(
            tool.spec().group,
            ToolGroup::TrendLines | ToolGroup::Channels
        )
}

/// The label of anchor `index` of a drawing by `tool`.
fn anchor_label(tool: Tool, index: usize) -> String {
    let named: &[&str] = match tool {
        Tool::Rectangle
        | Tool::Ellipse
        | Tool::Arrow
        | Tool::ArrowMarker
        | Tool::TrendLine
        | Tool::Ray
        | Tool::InfoLine
        | Tool::ExtendedLine
        | Tool::TrendAngle
        | Tool::Fib => &["Point 1", "Point 2"],
        Tool::HorizontalLine => &["Price level"],
        Tool::HorizontalRay | Tool::Crossline => &["Anchor"],
        Tool::VerticalLine => &["Time"],
        Tool::ParallelChannel => &["Point 1", "Point 2", "Offset point"],
        Tool::FlatChannel => &["Point 1", "Point 2", "Level"],
        Tool::DisjointChannel => &["Point 1", "Point 2", "Point 3", "End of second edge"],
        Tool::RegressionTrend => &["Start bar", "End bar"],
        Tool::Pitchfork
        | Tool::SchiffPitchfork
        | Tool::ModifiedSchiffPitchfork
        | Tool::InsidePitchfork => &["Point 1", "Point 2", "Point 3"],
        Tool::RotatedRectangle => &["Point 1", "Point 2", "Width point"],
        Tool::Circle => &["Center", "Radius point"],
        Tool::Arc => &["Start", "End", "Curvature point"],
        Tool::Curve => &["Start", "End", "Control point"],
        Tool::DoubleCurve => &["Start", "End", "Control point 1", "Control point 2"],
        Tool::ArrowMarkUp | Tool::ArrowMarkDown => &["Anchor"],
        _ => &[],
    };
    named
        .get(index)
        .map_or_else(|| format!("Point {}", index + 1), |name| (*name).to_owned())
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
            prompt: None,
            notice: None,
            page_height: 0.0,
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

    fn tabs(&self) -> Vec<Tab> {
        let caps = self.tool.caps();
        let mut tabs = Vec::new();
        if has_style_page(caps) {
            tabs.push(Tab::Style);
        }
        if caps.has(Caps::REGRESSION) {
            tabs.push(Tab::Inputs);
        }
        if caps.has(Caps::LEVELS) {
            tabs.push(Tab::Levels);
        }
        if caps.has(Caps::TEXT) {
            tabs.push(Tab::Text);
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
                time_invalid: false,
            })
            .collect();
    }

    /// Shows the dialog. Call every frame while it is open.
    pub fn show(&mut self, ctx: &Context, templates: &mut DrawingTemplateStore) -> Outcome {
        let mut outcome = Outcome::Open;
        let content = ctx.content_rect();
        let typing = ctx.egui_wants_keyboard_input();
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
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 0.0;
            outcome = self.body(ui, templates);
        });
        if matches!(outcome, Outcome::Open) {
            if ctx.input(|input| input.key_pressed(Key::Escape)) {
                outcome = Outcome::Cancel;
            } else if !typing && ctx.input(|input| input.key_pressed(Key::Enter)) {
                outcome = Outcome::Apply(Box::new(self.draft.clone()));
            }
        }
        outcome
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
            let items: Vec<kit::TabItem<Tab>> = tabs
                .iter()
                .map(|&tab| kit::TabItem {
                    value: tab,
                    title: tab.title(),
                    icon: tab.icon(),
                    count: 0,
                })
                .collect();
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
                        ui.set_width(WIDTH - 2.0 * PAGE_MARGIN);
                        // The tallest page so far, so a tab switch does not resize the dialog.
                        ui.set_min_height((self.page_height - 2.0 * PAGE_MARGIN).max(0.0));
                        ui.spacing_mut().item_spacing.y = CARD_SPACING;
                        match self.tab {
                            Tab::Style => self.style_page(ui),
                            Tab::Inputs => self.inputs_page(ui),
                            Tab::Levels => self.levels_page(ui),
                            Tab::Text => self.text_page(ui),
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

    // MARK: Templates

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
        });
    }

    /// The row under the page: the Template menu, a name prompt when one is wanted, and what
    /// the last template action did.
    fn template_bar(&mut self, ui: &mut Ui, templates: &mut DrawingTemplateStore) {
        // Only tools with a style have anything to save.
        if self.draft.style().is_none() {
            return;
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(sheet::INSET, 10))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 2.0 * PAGE_MARGIN);
                if self.prompt.is_some() {
                    self.prompt_row(ui, templates);
                    return;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let button = kit::push_button(ui, Some(Icon::Layers), "Template", true);
                    egui::Popup::menu(&button)
                        .frame(kit::menu_frame())
                        .width(250.0)
                        .show(|ui| self.template_menu(ui, templates));
                    if self.notice.is_none() && templates.load_error().is_some() {
                        ui.add(
                            egui::Label::new(kit::text(
                                "Saved templates could not be read, so none are changed.",
                                kit::size::SUBHEADLINE,
                                kit::REGULAR,
                                kit::RED,
                            ))
                            .truncate(),
                        );
                    }
                    if let Some(notice) = &self.notice {
                        let color = if notice.error {
                            kit::RED
                        } else {
                            theme::text_secondary()
                        };
                        ui.add(
                            egui::Label::new(kit::text(
                                &notice.text,
                                kit::size::SUBHEADLINE,
                                kit::REGULAR,
                                color,
                            ))
                            .truncate(),
                        );
                    }
                });
            });
    }

    fn template_menu(&mut self, ui: &mut Ui, templates: &mut DrawingTemplateStore) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let tool = self.tool;
        if kit::menu_item(ui, Icon::Plus, "Save as template…", true, false) {
            let name = templates
                .book()
                .free_name(tool, &format!("My {}", tool.label().to_lowercase()));
            self.prompt = Some(Prompt {
                rename: None,
                name,
                error: None,
                focus: true,
            });
            ui.close();
        }
        kit::menu_separator(ui);
        let saved: Vec<(TemplateId, String)> = templates
            .book()
            .for_tool(tool)
            .iter()
            .map(|template| (template.id, template.name.clone()))
            .collect();
        if saved.is_empty() {
            kit::menu_item(ui, Icon::Layers, "No saved templates yet", false, false);
        }
        for (id, name) in saved {
            match template_row(ui, &name) {
                Some(RowAction::Apply) => {
                    if let Some(template) = templates.book().get(id) {
                        let style = template.style;
                        self.set_style(style);
                        self.say(format!("Applied “{name}”"), false);
                    }
                    ui.close();
                }
                Some(RowAction::Rename) => {
                    self.prompt = Some(Prompt {
                        rename: Some(id),
                        name: name.clone(),
                        error: None,
                        focus: true,
                    });
                    ui.close();
                }
                Some(RowAction::Delete) => {
                    match templates.delete(id) {
                        Ok(()) => self.say(format!("Deleted “{name}”"), false),
                        Err(error) => self.say(error, true),
                    }
                    ui.close();
                }
                None => {}
            }
        }
        kit::menu_separator(ui);
        let label = format!("Make this the default {}", tool.label().to_lowercase());
        if kit::menu_item(ui, Icon::Check, &label, true, false) {
            match templates.set_default(tool, &self.style()) {
                Ok(()) => self.say(
                    format!(
                        "New {} drawings start with this style",
                        tool.label().to_lowercase()
                    ),
                    false,
                ),
                Err(error) => self.say(error, true),
            }
            ui.close();
        }
        if kit::menu_item(ui, Icon::Undo, "Reset to the default style", true, false) {
            let style = style_for(tool, templates.defaults());
            self.set_style(style);
            self.say("Back to the default style", false);
            ui.close();
        }
        let has_default = templates.defaults().contains_key(&tool);
        if kit::menu_item(ui, Icon::Trash, "Forget my default", has_default, false) {
            match templates.reset_default(tool) {
                Ok(()) => self.say("New drawings start with the built-in style again", false),
                Err(error) => self.say(error, true),
            }
            ui.close();
        }
    }

    /// The name field of "Save as template" and "Rename".
    fn prompt_row(&mut self, ui: &mut Ui, templates: &mut DrawingTemplateStore) {
        let tool = self.tool;
        let style = self.style();
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        let id = Id::new("drawing-template-name");
        let mut done: Option<bool> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let focused = ui.memory(|memory| memory.has_focus(id));
            let response = kit::field_frame(focused)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(8, 0))
                .show(ui, |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(210.0, 22.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            kit::plain_field(
                                ui,
                                id,
                                &mut prompt.name,
                                "Template name",
                                12.0,
                                kit::REGULAR,
                                210.0,
                            )
                        },
                    )
                    .inner
                })
                .inner;
            if prompt.focus {
                response.request_focus();
                prompt.focus = false;
            }
            let enter = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            let label = if prompt.rename.is_some() {
                "Rename"
            } else {
                "Save"
            };
            if kit::push_button(ui, None, label, true).clicked() || enter {
                done = Some(true);
            }
            if kit::push_button(ui, None, "Cancel", true).clicked() {
                done = Some(false);
            }
        });
        if let Some(error) = &prompt.error {
            ui.label(kit::text(
                error,
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                kit::RED,
            ));
        }
        match done {
            Some(true) => {
                let name = prompt.name.clone();
                let result = match prompt.rename {
                    Some(id) => templates
                        .rename(id, &name)
                        .map(|()| format!("Renamed to “{}”", name.trim())),
                    None => templates
                        .save(&name, tool, &style)
                        .map(|_| format!("Saved “{}”", name.trim())),
                };
                match result {
                    Ok(text) => {
                        self.prompt = None;
                        self.say(text, false);
                    }
                    Err(error) => prompt.error = Some(error),
                }
            }
            Some(false) => self.prompt = None,
            None => {}
        }
    }

    fn style(&self) -> DrawingStyle {
        self.draft.style().copied().unwrap_or_default()
    }

    fn set_style(&mut self, style: DrawingStyle) {
        self.draft.set_style(style);
    }

    // MARK: Style

    fn style_page(&mut self, ui: &mut Ui) {
        let caps = self.tool.caps();
        let mut style = self.style();
        let before = style;
        let line = caps.has(Caps::COLOR) || caps.has(Caps::WIDTH) || caps.has(Caps::DASH);
        let shape = caps.has(Caps::FILL) || caps.has(Caps::MIDDLE_LINE);
        let details = caps.has(Caps::MIDDLE_POINT)
            || caps.has(Caps::PRICE_LABELS)
            || caps.has(Caps::TIME_LABEL);
        let variant = match self.draft.geometry() {
            Some(Geometry::Pitchfork { variant, .. }) => Some(*variant),
            _ => None,
        };
        let groups = [
            variant.is_some(),
            line,
            caps.has(Caps::ENDS),
            shape,
            caps.has(Caps::EXTEND),
            details,
            caps.has(Caps::STATS),
        ];
        // A page of one group needs no heading to say what it is.
        let mut page = Page::new(groups.into_iter().filter(|present| *present).count() > 1);

        // The four pitchforks are one drawing in four constructions: switching keeps the three
        // pivots, so a fork can be tried every way without redrawing it.
        if let Some(current) = variant {
            page.section(ui, "Pitchfork", None);
            let options: Vec<(PitchforkVariant, &str)> = PitchforkVariant::ALL
                .iter()
                .map(|option| (*option, option.label()))
                .collect();
            let mut chosen = current;
            picker_row(
                ui,
                Icon::ForkAndrews,
                "Style",
                Some("Where the median starts and ends."),
                &options,
                &mut chosen,
            );
            if chosen != current {
                self.switch_variant(chosen);
            }
        }
        if line {
            page.section(ui, if variant.is_some() { "Median" } else { "Line" }, None);
        }
        if caps.has(Caps::COLOR) {
            let title = if variant.is_some() {
                "Median color"
            } else if caps.has(Caps::WIDTH) {
                "Line color"
            } else {
                "Color"
            };
            color_card(ui, Icon::Palette, title, None, &mut style.stroke.color);
            // The marks and the marker carry their text in the same colour.
            if !caps.has(Caps::WIDTH) {
                style.text.color = style.stroke.color;
            }
        }
        if caps.has(Caps::WIDTH) {
            let max = crate::model::drawing_style::MAX_STROKE_WIDTH as f64;
            if let Some(width) = settings_card::card(ui, Icon::Sliders, "Thickness", None, |ui| {
                number_field(
                    ui,
                    Id::new("drawing-width"),
                    f64::from(style.stroke.width),
                    1.0,
                    1.0..=max,
                    0,
                )
            }) {
                style.stroke.width = width as f32;
            }
        }
        if caps.has(Caps::DASH) {
            dash_card(ui, "Line style", &mut style.stroke.dash);
        }
        if caps.has(Caps::ENDS) {
            page.section(ui, "Arrowheads", None);
            let options: Vec<(LineEnd, &str)> =
                LineEnd::ALL.iter().map(|end| (*end, end.label())).collect();
            picker_row(
                ui,
                Icon::ArrowUpRight,
                "Start",
                None,
                &options,
                &mut style.head_start,
            );
            picker_row(
                ui,
                Icon::ArrowUpRight,
                "End",
                None,
                &options,
                &mut style.head_end,
            );
        }
        if shape {
            page.section(ui, "Shape", None);
        }
        if caps.has(Caps::FILL) {
            // A channel or a fork fills the space between its lines: a background.
            let title = if self.tool.spec().group == crate::model::ToolGroup::Shapes {
                "Fill"
            } else {
                "Background"
            };
            settings_card::card(ui, Icon::RectFill, title, None, |ui| {
                color_well(ui, "Fill color", &mut style.fill.color);
                settings_card::switch(ui, &mut style.fill.enabled);
            });
        }
        if caps.has(Caps::MIDDLE_LINE) {
            settings_card::card(ui, Icon::Minus, "Middle line", None, |ui| {
                color_well(ui, "Middle line color", &mut style.middle_line.color);
                settings_card::switch(ui, &mut style.middle_line.enabled);
            });
            if style.middle_line.enabled {
                dash_card(ui, "Middle line style", &mut style.middle_line.dash);
            }
        }
        if caps.has(Caps::EXTEND) {
            page.section(ui, "Extend", None);
            toggle_row(
                ui,
                Icon::ArrowLeftRight,
                "Extend left",
                "Continue to the left edge of the chart.",
                &mut style.extend_left,
            );
            toggle_row(
                ui,
                Icon::ArrowLeftRight,
                "Extend right",
                "Continue to the right edge of the chart.",
                &mut style.extend_right,
            );
        }
        if details {
            page.section(ui, "Details", None);
        }
        if caps.has(Caps::MIDDLE_POINT) {
            toggle_row(
                ui,
                Icon::Circle,
                "Middle point",
                "A dot halfway along the line.",
                &mut style.middle_point,
            );
        }
        if caps.has(Caps::PRICE_LABELS) {
            // A line at a price is tagged on the price axis; the others print their end prices.
            let on_axis = matches!(
                self.tool,
                Tool::HorizontalLine | Tool::HorizontalRay | Tool::Crossline
            );
            toggle_row(
                ui,
                Icon::Tag,
                if on_axis {
                    "Price label"
                } else {
                    "Price labels"
                },
                if on_axis {
                    "A tag with the price on the price axis."
                } else {
                    "The price at each end."
                },
                &mut style.price_labels,
            );
        }
        if caps.has(Caps::TIME_LABEL) {
            toggle_row(
                ui,
                Icon::Clock,
                "Time label",
                "A tag with the time on the time axis.",
                &mut style.time_label,
            );
        }
        if caps.has(Caps::STATS) {
            self.stats_section(ui, &mut page, &mut style);
        }
        if style != before {
            self.set_style(style);
        }
    }

    fn stats_section(&self, ui: &mut Ui, page: &mut Page, style: &mut DrawingStyle) {
        // Always titled: its subtitle says what the switches below it print.
        let noun = if self.tool == Tool::Arrow {
            "arrow"
        } else {
            "line"
        };
        page.titled_section(
            ui,
            "Statistics",
            Some(&format!("Measurements printed beside the {noun}.")),
        );
        let stats = &mut style.stats;
        toggle_row(
            ui,
            Icon::Hash,
            "Price range",
            "The price change between the ends.",
            &mut stats.price_range,
        );
        toggle_row(
            ui,
            Icon::Percent,
            "Percent change",
            "The change as a percentage of the start.",
            &mut stats.percent,
        );
        toggle_row(
            ui,
            Icon::ChartLine,
            "Bars",
            "How many bars the arrow spans.",
            &mut stats.bars,
        );
        toggle_row(
            ui,
            Icon::Clock,
            "Time range",
            "How long the arrow spans.",
            &mut stats.time,
        );
        toggle_row(
            ui,
            Icon::ArrowLeftRight,
            "Distance",
            "The signed number of bars from start to end.",
            &mut stats.distance,
        );
        toggle_row(
            ui,
            Icon::ArrowUpRight,
            "Angle",
            "The arrow's angle as drawn on screen.",
            &mut stats.angle,
        );
        let options: Vec<(StatsPosition, &str)> = StatsPosition::ALL
            .iter()
            .map(|position| (*position, position.label()))
            .collect();
        picker_row(
            ui,
            Icon::Sliders,
            "Position",
            None,
            &options,
            &mut stats.position,
        );
        toggle_row(
            ui,
            Icon::Eye,
            "Always show",
            "Off shows the statistics only while the arrow is selected.",
            &mut stats.always_show,
        );
    }

    /// Draws the pitchfork as another variant: the same three pivots, another construction.
    fn switch_variant(&mut self, variant: PitchforkVariant) {
        if let Some(Geometry::Pitchfork {
            variant: current, ..
        }) = self.draft.geometry_mut()
        {
            *current = variant;
            self.tool = variant.tool();
        }
    }

    // MARK: Levels and inputs

    /// The extra lines of a pitchfork (each on both sides of the median) or of a parallel channel.
    fn levels_page(&mut self, ui: &mut Ui) {
        let mut style = self.style();
        let before = style;
        let fork = matches!(self.draft.geometry(), Some(Geometry::Pitchfork { .. }));
        let mut page = Page::new(true);
        if fork {
            page.titled_section(
                ui,
                "Colors",
                Some("Each level is drawn on both sides of the median."),
            );
            toggle_row(
                ui,
                Icon::Palette,
                "Use one color",
                "Draw every line in the median's color.",
                &mut style.use_one_color,
            );
            for (index, level) in style.pitchfork_levels.iter_mut().enumerate() {
                level_cards(ui, &mut page, ("fork", index), level);
            }
        } else {
            for (index, level) in style.channel_levels.iter_mut().enumerate() {
                level_cards(ui, &mut page, ("channel", index), level);
            }
        }
        if style != before {
            self.set_style(style);
        }
    }

    /// The regression trend's source, the deviations its bands sit at, and its three lines.
    fn inputs_page(&mut self, ui: &mut Ui) {
        use crate::model::chart_type::PriceSource;
        let mut style = self.style();
        let before = style;
        let mut page = Page::new(true);
        page.section(ui, "Inputs", None);
        let options: Vec<(PriceSource, &str)> = PriceSource::ALL
            .iter()
            .map(|source| (*source, source.label()))
            .collect();
        picker_row(
            ui,
            Icon::ChartLine,
            "Source",
            Some("The price the line is fitted to."),
            &options,
            &mut style.regression.source,
        );
        let max = f64::from(crate::model::drawing_style::MAX_DEVIATION);
        let regression = &mut style.regression;
        for (title, hint, id, value) in [
            (
                "Upper deviation",
                "Standard deviations above the line.",
                "regression-upper",
                &mut regression.upper_dev,
            ),
            (
                "Lower deviation",
                "Standard deviations from the line, negative for below.",
                "regression-lower",
                &mut regression.lower_dev,
            ),
        ] {
            if let Some(picked) = settings_card::card(ui, Icon::Hash, title, Some(hint), |ui| {
                number_field(ui, Id::new(id), f64::from(*value), 0.1, -max..=max, 2)
            }) {
                *value = picked as f32;
            }
        }
        toggle_row(
            ui,
            Icon::Percent,
            "Pearson's R",
            "Print how closely the price follows the line (-1 to 1).",
            &mut regression.pearson,
        );
        page.titled_section(ui, "Base line", None);
        line_cards(ui, "regression-base", &mut regression.base);
        page.titled_section(ui, "Upper line", None);
        line_cards(ui, "regression-upper-line", &mut regression.upper);
        page.titled_section(ui, "Lower line", None);
        line_cards(ui, "regression-lower-line", &mut regression.lower);
        if style != before {
            self.set_style(style);
        }
    }

    // MARK: Text

    fn text_page(&mut self, ui: &mut Ui) {
        let mut page = Page::new(true);
        page.section(ui, "Text", None);
        let mut text = self.draft.text().to_owned();
        let focused_id = Id::new("drawing-text");
        let focused = ui.memory(|memory| memory.has_focus(focused_id));
        kit::field_frame(focused).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .id(focused_id)
                    .frame(egui::Frame::NONE)
                    .hint_text("Add text to this drawing")
                    .char_limit(MAX_TEXT_CHARS)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
        });
        if text != self.draft.text() {
            self.draft.set_text(&text);
        }

        let mut style = self.style();
        let before = style;
        page.section(ui, "Font", None);
        color_card(ui, Icon::Palette, "Text color", None, &mut style.text.color);
        if let Some(size) = settings_card::card(ui, Icon::TextFormat, "Size", None, |ui| {
            number_field(
                ui,
                Id::new("drawing-text-size"),
                f64::from(style.text.size),
                1.0,
                f64::from(crate::model::drawing_style::TEXT_SIZE_RANGE.0)
                    ..=f64::from(crate::model::drawing_style::TEXT_SIZE_RANGE.1),
                0,
            )
        }) {
            style.text.size = size as f32;
        }
        toggle_row(ui, Icon::TextFormat, "Bold", "", &mut style.text.bold);
        toggle_row(ui, Icon::TextFormat, "Italic", "", &mut style.text.italic);
        page.section(ui, "Alignment", None);
        let horizontal: Vec<(HorizontalAlign, &str)> = HorizontalAlign::ALL
            .iter()
            .map(|align| (*align, align.label()))
            .collect();
        picker_row(
            ui,
            Icon::Sliders,
            "Alignment",
            None,
            &horizontal,
            &mut style.text.align,
        );
        if text_has_position(self.tool) {
            let vertical: Vec<(VerticalAlign, &str)> = VerticalAlign::ALL
                .iter()
                .map(|align| (*align, align.label()))
                .collect();
            picker_row(
                ui,
                Icon::Sliders,
                "Position",
                None,
                &vertical,
                &mut style.text.valign,
            );
        }
        if self.tool.caps().has(Caps::ORIENTATION) {
            // A vertical line's text reads up the line or across it.
            let options = [
                (TextOrientation::Vertical, "Vertical"),
                (TextOrientation::Horizontal, "Horizontal"),
            ];
            let mut shown = if style.text.orientation == TextOrientation::Horizontal {
                TextOrientation::Horizontal
            } else {
                TextOrientation::Vertical
            };
            picker_row(
                ui,
                Icon::TextFormat,
                "Orientation",
                None,
                &options,
                &mut shown,
            );
            style.text.orientation = shown;
        }
        let mut visible = !style.hide_text;
        toggle_row(
            ui,
            Icon::Eye,
            "Show text",
            "Off keeps the text but does not draw it.",
            &mut visible,
        );
        style.hide_text = !visible;
        if style != before {
            self.set_style(style);
        }
    }

    // MARK: Coordinates

    fn coordinates_page(&mut self, ui: &mut Ui) {
        let mut page = Page::new(true);
        let anchors = self.draft.anchors();
        let shown = anchors.len().min(MAX_LISTED);
        for (index, &anchor) in anchors.iter().enumerate().take(shown) {
            page.section(ui, &anchor_label(self.tool, index), None);
            let mut edited = anchor;
            // Some anchors are read for one of their two numbers only (a level has no time, a
            // regression no price): the other is where a handle was left and is not offered.
            let (reads_time, reads_price) = self
                .draft
                .geometry()
                .map_or((true, true), |geometry| geometry.reads(index));
            let decimals = price_decimals(anchor.price);
            let step = 10f64.powi(-(decimals as i32));
            let price_id = Id::new(("drawing-price", self.draft.id(), index));
            if reads_price
                && let Some(price) = settings_card::card(ui, Icon::Tag, "Price", None, |ui| {
                    number_field_sized(
                        ui,
                        price_id,
                        anchor.price,
                        step,
                        -1e12..=1e12,
                        decimals,
                        PRICE_FIELD_WIDTH,
                    )
                })
            {
                edited.price = price;
            }
            let state = &mut self.coordinates[index];
            if reads_time
                && let Some(time) = time_row(
                    ui,
                    Id::new(("drawing-time", self.draft.id(), index)),
                    state,
                    anchor.time_ms,
                )
            {
                edited.time_ms = time;
            }
            if edited.time_ms != anchor.time_ms || edited.price != anchor.price {
                self.move_anchor(index, edited);
            }
        }
        if anchors.len() > shown {
            ui.label(kit::text(
                format!(
                    "{} more points: drag them on the chart.",
                    anchors.len() - shown
                ),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        }
    }

    fn move_anchor(&mut self, index: usize, anchor: Anchor) {
        let mut candidate = self.draft.clone();
        if let Some(slot) = candidate.anchors_mut().into_iter().nth(index) {
            *slot = anchor;
        }
        self.draft = candidate;
        if let Some(text) = self.coordinates.get_mut(index) {
            text.time = format::timestamp(anchor.time_ms);
            text.time_invalid = false;
        }
    }

    // MARK: Visibility

    fn visibility_page(&mut self, ui: &mut Ui) {
        let mut visibility = self.draft.visibility();
        let before = visibility;
        let mut page = Page::new(true);
        page.section(
            ui,
            "Show on",
            Some("The drawing is kept when you switch to a timeframe it is not shown on."),
        );
        let mut all = visibility.is_all();
        let was_all = all;
        toggle_row(ui, Icon::Eye, "All timeframes", "", &mut all);
        if all && !was_all {
            visibility = Visibility::ALL;
        }
        for timeframe in Timeframe::ALL {
            let mut shown = visibility.shows(timeframe);
            let was = shown;
            toggle_row(ui, Icon::Clock, timeframe.description(), "", &mut shown);
            if shown != was {
                visibility = visibility.with(timeframe, shown);
            }
        }
        if visibility != before {
            self.draft.set_visibility(visibility);
        }
    }
}

// MARK: Rows

/// The headings of one page. Every card of a page sits `CARD_SPACING` apart; a heading adds
/// `SECTION_GAP` above itself (except the first) and hands the card spacing back to the cards
/// below it, because `settings_card::section_title` tightens it for its own subtitle.
struct Page {
    titled: bool,
    started: bool,
}

impl Page {
    /// `titled` is whether [`Page::section`] prints anything.
    fn new(titled: bool) -> Self {
        Self {
            titled,
            started: false,
        }
    }

    /// A heading, when the page has more than one group.
    fn section(&mut self, ui: &mut Ui, title: &str, subtitle: Option<&str>) {
        if self.titled {
            self.titled_section(ui, title, subtitle);
        }
    }

    /// A heading whatever the page.
    fn titled_section(&mut self, ui: &mut Ui, title: &str, subtitle: Option<&str>) {
        if self.started {
            ui.add_space(SECTION_GAP);
        }
        self.started = true;
        settings_card::section_title(ui, title, subtitle);
        ui.spacing_mut().item_spacing.y = CARD_SPACING;
    }
}

/// A ratio as a short number: `0.382`, `1`, `-0.25`.
fn ratio_text(ratio: f32) -> String {
    let text = format!("{ratio:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The cards of one level: whether and in what colour it is drawn, and, once it is, where it
/// sits and how it looks.
fn level_cards(ui: &mut Ui, page: &mut Page, id: (&'static str, usize), level: &mut Level) {
    page.titled_section(ui, &format!("Level {}", ratio_text(level.ratio)), None);
    line_cards_with_ratio(ui, id, level);
}

fn line_cards_with_ratio(ui: &mut Ui, id: (&'static str, usize), level: &mut Level) {
    settings_card::card(ui, Icon::Eye, "Show", None, |ui| {
        color_well(ui, "Level color", &mut level.line.color);
        settings_card::switch(ui, &mut level.line.enabled);
    });
    if !level.line.enabled {
        return;
    }
    let max = f64::from(crate::model::drawing_style::MAX_LEVEL_RATIO);
    if let Some(ratio) = settings_card::card(ui, Icon::Hash, "Ratio", None, |ui| {
        number_field(
            ui,
            Id::new(("drawing-level-ratio", id)),
            f64::from(level.ratio),
            0.05,
            -max..=max,
            3,
        )
    }) {
        level.ratio = ratio as f32;
    }
    thickness_and_style(ui, Id::new(("drawing-level-width", id)), &mut level.line);
}

/// The cards of a line with its own look: shown or not with its colour, then thickness and style.
fn line_cards(ui: &mut Ui, id: &'static str, line: &mut MiddleLine) {
    settings_card::card(ui, Icon::Eye, "Show", None, |ui| {
        color_well(ui, "Line color", &mut line.color);
        settings_card::switch(ui, &mut line.enabled);
    });
    if line.enabled {
        thickness_and_style(ui, Id::new(("drawing-line-width", id)), line);
    }
}

fn thickness_and_style(ui: &mut Ui, id: Id, line: &mut MiddleLine) {
    let max = f64::from(crate::model::drawing_style::MAX_STROKE_WIDTH);
    if let Some(width) = settings_card::card(ui, Icon::Sliders, "Thickness", None, |ui| {
        number_field(ui, id, f64::from(line.width), 1.0, 1.0..=max, 0)
    }) {
        line.width = width as f32;
    }
    dash_card(ui, "Line style", &mut line.dash);
}

/// A card with a switch; an empty `hint` is no hint at all, so the title centres on its badge.
fn toggle_row(ui: &mut Ui, icon: Icon, title: &str, hint: &str, value: &mut bool) {
    let hint = Some(hint).filter(|hint| !hint.is_empty());
    settings_card::card(ui, icon, title, hint, |ui| {
        settings_card::switch(ui, value);
    });
}

/// What a click on a template's row asks for.
enum RowAction {
    Apply,
    Rename,
    Delete,
}

/// One saved template in the menu: its name, and a pencil and a bin that show on hover.
fn template_row(ui: &mut Ui, name: &str) -> Option<RowAction> {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let response = crate::ui::accessibility::label(response, format!("Apply template {name}"));
    let hovered = response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 5.0, kit::ACCENT);
    }
    let color = if hovered {
        Color32::WHITE
    } else {
        theme::text_primary()
    };
    let galley = kit::galley(ui, name, kit::size::BODY, kit::REGULAR, color, width - 84.0);
    ui.painter().galley(
        pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    let bin =
        egui::Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(20.0, 20.0));
    let pencil = bin.translate(vec2(-24.0, 0.0));
    if hovered {
        for (spot, icon) in [(pencil, Icon::Pencil), (bin, Icon::Trash)] {
            crate::ui::icons::paint(
                ui.painter(),
                egui::Rect::from_center_size(spot.center(), vec2(12.0, 12.0)),
                icon,
                Color32::WHITE,
            );
        }
    }
    if !response.clicked() {
        return None;
    }
    let at = response.interact_pointer_pos()?;
    Some(if bin.contains(at) {
        RowAction::Delete
    } else if pencil.contains(at) {
        RowAction::Rename
    } else {
        RowAction::Apply
    })
}

fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::separator());
}

fn paint_color(color: Rgba) -> Color32 {
    series_render::color(color)
}

fn color_well(ui: &mut Ui, title: &str, color: &mut Rgba) {
    if let Some(picked) = kit::color_well(
        ui,
        vec2(44.0, 24.0),
        title,
        paint_color(*color),
        Alpha::OnlyBlend,
    ) {
        *color = picked.to_srgba_unmultiplied();
    }
}

fn color_card(ui: &mut Ui, icon: Icon, title: &str, hint: Option<&str>, color: &mut Rgba) {
    settings_card::card(ui, icon, title, hint, |ui| color_well(ui, title, color));
}

fn dash_card(ui: &mut Ui, title: &str, dash: &mut LineDash) {
    let options: Vec<(LineDash, &str)> = LineDash::ALL
        .iter()
        .map(|option| (*option, option.label()))
        .collect();
    picker_row(ui, Icon::Minus, title, None, &options, dash);
}

/// The decimals a price is edited with: enough that a typical tick is visible.
fn price_decimals(price: f64) -> usize {
    let magnitude = price.abs();
    if magnitude >= 1000.0 {
        2
    } else if magnitude >= 1.0 {
        4
    } else if magnitude >= 0.01 {
        6
    } else {
        8
    }
}

/// The time field of one anchor: UTC `YYYY-MM-DD HH:MM`, committed on Return or when it loses
/// focus, and marked invalid (and not applied) when it does not parse.
fn time_row(ui: &mut Ui, id: Id, state: &mut CoordinateText, original_ms: i64) -> Option<i64> {
    let mut result = None;
    settings_card::card(ui, Icon::Clock, "Time (UTC)", None, |ui| {
        let focused = ui.memory(|memory| memory.has_focus(id));
        let stroke = if state.time_invalid {
            Stroke::new(1.5, kit::RED)
        } else {
            kit::field_frame(focused).stroke
        };
        let response = kit::field_frame(focused)
            .stroke(stroke)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(8, 0))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    vec2(TIME_FIELD_WIDTH, 22.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        kit::plain_field(
                            ui,
                            id,
                            &mut state.time,
                            "YYYY-MM-DD HH:MM",
                            12.0,
                            kit::REGULAR,
                            TIME_FIELD_WIDTH,
                        )
                    },
                )
                .inner
            })
            .inner;
        // The field shows the time to the minute, so only text the user changed is a new time.
        if response.lost_focus() && state.time.trim() != format::timestamp(original_ms) {
            match format::parse_timestamp(&state.time) {
                Some(time) => {
                    state.time_invalid = false;
                    result = Some(time);
                }
                None => state.time_invalid = true,
            }
        }
    });
    if state.time_invalid {
        ui.label(kit::text(
            "Use a date and time like 2025-03-14 09:30.",
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            kit::RED,
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::drawing_style::MAX_STROKE_WIDTH;
    use crate::model::{Anchor, Placement};
    use std::sync::{Arc, Mutex};

    fn templates() -> DrawingTemplateStore {
        DrawingTemplateStore::new(Arc::new(Mutex::new(
            crate::store::Store::open_in_memory().expect("store"),
        )))
    }

    fn drawing(tool: Tool) -> Drawing {
        let anchors: Vec<Anchor> = (0..4)
            .map(|index| Anchor {
                time_ms: 1_700_000_000_000 + index * 3_600_000,
                price: 100.0 + index as f64 * 7.0,
            })
            .collect();
        let count = match tool.spec().placement {
            Placement::Points(count) => usize::from(count),
            Placement::Single => 1,
            _ => 3,
        };
        Drawing::build(tool, 1, &anchors[..count], tool.default_style()).expect("builds")
    }

    fn frame(
        ctx: &egui::Context,
        dialog: &mut DrawingProperties,
        store: &mut DrawingTemplateStore,
        events: Vec<egui::Event>,
    ) -> Outcome {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(1200.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let mut outcome = Outcome::Open;
        let mut output = ctx.run_ui(input, |ui| outcome = dialog.show(ui.ctx(), store));
        output.textures_delta.clear();
        outcome
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_ruler_and_an_unreadable_row_have_no_properties() {
        let ruler = Drawing::Ruler {
            id: 1,
            a: Anchor {
                time_ms: 0,
                price: 1.0,
            },
            b: Anchor {
                time_ms: 1,
                price: 2.0,
            },
        };
        assert!(DrawingProperties::new(1, &ruler).is_none());
        assert!(DrawingProperties::new(1, &Drawing::Unreadable(serde_json::Value::Null)).is_none());
    }

    #[test]
    fn a_dialog_offers_only_the_pages_its_tool_supports() {
        for tool in Tool::GROUPED {
            let dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
            let tabs = dialog.tabs();
            let caps = tool.caps();
            assert_eq!(
                tabs.contains(&Tab::Style),
                has_style_page(caps),
                "{tool:?} style"
            );
            assert_eq!(
                tabs.contains(&Tab::Text),
                caps.has(Caps::TEXT),
                "{tool:?} text"
            );
            assert_eq!(
                tabs.contains(&Tab::Levels),
                caps.has(Caps::LEVELS),
                "{tool:?} levels"
            );
            assert_eq!(
                tabs.contains(&Tab::Inputs),
                caps.has(Caps::REGRESSION),
                "{tool:?} inputs"
            );
            assert_eq!(
                tabs.contains(&Tab::Coordinates),
                !matches!(tool, Tool::Brush | Tool::Highlighter),
                "{tool:?} coordinates"
            );
            assert!(tabs.contains(&Tab::Visibility), "{tool:?}");
            assert_eq!(dialog.tab, tabs[0], "{tool:?} opens on its first page");
        }
        // The marks have a colour and text but no thickness, pattern or fill to offer.
        let mark = Tool::ArrowMarkUp.caps();
        assert!(mark.has(Caps::COLOR) && !mark.has(Caps::WIDTH) && !mark.has(Caps::FILL));
    }

    #[test]
    fn every_page_of_every_tool_draws() {
        let ctx = egui::Context::default();
        let mut store = templates();
        for tool in Tool::GROUPED {
            let mut dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
            for tab in dialog.tabs() {
                dialog.tab = tab;
                for _ in 0..2 {
                    assert_eq!(
                        frame(&ctx, &mut dialog, &mut store, Vec::new()),
                        Outcome::Open
                    );
                }
            }
        }
    }

    #[test]
    fn every_page_leaves_the_card_spacing_as_it_found_it() {
        let ctx = egui::Context::default();
        let tools = Tool::GROUPED
            .into_iter()
            .chain([Tool::TrendLine, Tool::Fib])
            .collect::<Vec<_>>();
        for tool in tools {
            let mut dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
            for tab in dialog.tabs() {
                let mut spacing = 0.0;
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.spacing_mut().item_spacing.y = CARD_SPACING;
                    match tab {
                        Tab::Style => dialog.style_page(ui),
                        Tab::Inputs => dialog.inputs_page(ui),
                        Tab::Levels => dialog.levels_page(ui),
                        Tab::Text => dialog.text_page(ui),
                        Tab::Coordinates => dialog.coordinates_page(ui),
                        Tab::Visibility => dialog.visibility_page(ui),
                    }
                    spacing = ui.spacing().item_spacing.y;
                });
                output.textures_delta.clear();
                assert_eq!(spacing, CARD_SPACING, "{tool:?} {tab:?}");
            }
        }
    }

    #[test]
    fn a_toggle_without_a_hint_is_as_tall_as_a_card_can_be_without_one() {
        let ctx = egui::Context::default();
        let mut with_empty = 0.0;
        let mut without = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut on = false;
            let top = ui.cursor().top();
            toggle_row(ui, Icon::Eye, "Title", "", &mut on);
            with_empty = ui.cursor().top() - top;
            let top = ui.cursor().top();
            settings_card::card(ui, Icon::Eye, "Title", None, |ui| {
                settings_card::switch(ui, &mut on);
            });
            without = ui.cursor().top() - top;
        });
        output.textures_delta.clear();
        assert_eq!(with_empty, without);
    }

    #[test]
    fn escape_cancels_and_return_applies_the_working_copy() {
        let ctx = egui::Context::default();
        let mut store = templates();
        let mut dialog = DrawingProperties::new(3, &drawing(Tool::Rectangle)).expect("dialog");
        frame(&ctx, &mut dialog, &mut store, Vec::new());
        assert_eq!(
            frame(&ctx, &mut dialog, &mut store, vec![key(Key::Escape)]),
            Outcome::Cancel
        );
        let mut dialog = DrawingProperties::new(3, &drawing(Tool::Rectangle)).expect("dialog");
        dialog.draft.set_text("edited");
        frame(&ctx, &mut dialog, &mut store, Vec::new());
        let outcome = frame(&ctx, &mut dialog, &mut store, vec![key(Key::Enter)]);
        let Outcome::Apply(applied) = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(applied.text(), "edited");
        assert_eq!(applied.id(), 1);
    }

    #[test]
    fn the_pitchfork_style_switches_the_construction_and_keeps_the_pivots_and_the_look() {
        let mut dialog = DrawingProperties::new(1, &drawing(Tool::Pitchfork)).expect("dialog");
        let pivots = dialog.draft().anchors();
        let mut style = dialog.style();
        style.pitchfork_levels[0].line.enabled = true;
        style.stroke.color = [1, 2, 3, 255];
        dialog.set_style(style);
        for variant in PitchforkVariant::ALL {
            dialog.switch_variant(variant);
            assert_eq!(dialog.tool, variant.tool(), "the dialog follows the tool");
            assert_eq!(dialog.draft().tool(), Some(variant.tool()));
            assert_eq!(dialog.draft().anchors(), pivots);
            assert_eq!(dialog.style(), style, "every level and colour is kept");
        }
        // The dialog still draws every page after the switch.
        let ctx = egui::Context::default();
        let mut store = templates();
        for tab in dialog.tabs() {
            dialog.tab = tab;
            frame(&ctx, &mut dialog, &mut store, Vec::new());
        }
    }

    #[test]
    fn the_levels_and_inputs_pages_edit_the_style_the_drawing_keeps() {
        let mut fork = DrawingProperties::new(1, &drawing(Tool::Pitchfork)).expect("dialog");
        let mut style = fork.style();
        style.pitchfork_levels[1].line.enabled = true;
        style.pitchfork_levels[1].ratio = 0.7;
        style.use_one_color = true;
        fork.set_style(style);
        let json = serde_json::to_string(fork.draft()).unwrap();
        let back: Drawing = serde_json::from_str(&json).unwrap();
        assert_eq!(back.style().unwrap().pitchfork_levels[1].ratio, 0.7);
        assert!(back.style().unwrap().use_one_color);
        // Absurd ratios and deviations are brought into range, never stored.
        let mut regression =
            DrawingProperties::new(1, &drawing(Tool::RegressionTrend)).expect("dialog");
        let mut style = regression.style();
        style.regression.upper_dev = f32::NAN;
        style.regression.lower_dev = -1.0e9;
        style.pitchfork_levels[0].ratio = f32::INFINITY;
        regression.set_style(style);
        let kept = regression.style();
        assert_eq!(kept.regression.upper_dev, 2.0);
        assert_eq!(
            kept.regression.lower_dev,
            -crate::model::drawing_style::MAX_DEVIATION
        );
        assert!(kept.pitchfork_levels[0].ratio.is_finite());
    }

    #[test]
    fn the_coordinates_tab_leaves_out_what_an_anchor_does_not_read() {
        let ctx = egui::Context::default();
        let mut store = templates();
        // A horizontal line has a price and no time; a vertical line the opposite.
        for (tool, price, time) in [
            (Tool::HorizontalLine, true, false),
            (Tool::VerticalLine, false, true),
            (Tool::HorizontalRay, true, true),
            (Tool::RegressionTrend, false, true),
        ] {
            let dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
            let geometry = dialog.draft().geometry().expect("a geometry");
            assert_eq!(geometry.reads(0), (time, price), "{tool:?}");
            let mut dialog = dialog;
            dialog.tab = Tab::Coordinates;
            assert_eq!(
                frame(&ctx, &mut dialog, &mut store, Vec::new()),
                Outcome::Open
            );
        }
        assert_eq!(anchor_label(Tool::HorizontalLine, 0), "Price level");
        assert_eq!(anchor_label(Tool::DisjointChannel, 3), "End of second edge");
        assert_eq!(anchor_label(Tool::ParallelChannel, 2), "Offset point");
        assert_eq!(anchor_label(Tool::SchiffPitchfork, 2), "Point 3");
    }

    #[test]
    fn a_trend_line_takes_text_in_the_dialog_like_any_other_line() {
        let mut dialog = DrawingProperties::new(1, &drawing(Tool::TrendLine)).expect("dialog");
        dialog.draft.set_text("support");
        assert_eq!(dialog.draft().text(), "support");
        assert_eq!(dialog.draft().tool(), Some(Tool::TrendLine));
        assert_eq!(dialog.tool, Tool::TrendLine);
        assert!(dialog.tabs().contains(&Tab::Text));
        dialog.draft.set_text("");
        assert_eq!(dialog.draft().text(), "");
    }

    #[test]
    fn editing_a_coordinate_moves_that_anchor_and_refreshes_its_text() {
        let mut dialog = DrawingProperties::new(1, &drawing(Tool::Triangle)).expect("dialog");
        let before = dialog.draft().anchors();
        let moved = Anchor {
            time_ms: 1_800_000_000_000,
            price: -12.5,
        };
        dialog.move_anchor(1, moved);
        let after = dialog.draft().anchors();
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1], moved, "negative prices are allowed");
        assert_eq!(after[2], before[2]);
        assert_eq!(dialog.coordinates[1].time, format::timestamp(moved.time_ms));
        assert!(!dialog.coordinates[1].time_invalid);
    }

    #[test]
    fn the_dialog_never_stores_an_out_of_range_style() {
        let mut dialog = DrawingProperties::new(1, &drawing(Tool::Rectangle)).expect("dialog");
        let mut style = dialog.style();
        style.stroke.width = 1.0e9;
        style.text.size = f32::NAN;
        dialog.set_style(style);
        let kept = dialog.style();
        assert_eq!(kept.stroke.width, MAX_STROKE_WIDTH);
        assert!(kept.text.size.is_finite());
        // Text is capped, and a drawing without a text field refuses it.
        dialog.draft.set_text(&"x".repeat(10_000));
        assert!(dialog.draft().text().chars().count() <= crate::model::MAX_TEXT_CHARS);
        let mut brush = DrawingProperties::new(1, &drawing(Tool::Brush)).expect("brush dialog");
        brush.draft.set_text("ignored");
        assert_eq!(brush.draft().text(), "");
    }

    #[test]
    fn visibility_edits_keep_at_least_one_timeframe() {
        let mut dialog = DrawingProperties::new(1, &drawing(Tool::Circle)).expect("dialog");
        let mut visibility = dialog.draft().visibility();
        for timeframe in Timeframe::ALL {
            visibility = visibility.with(timeframe, false);
        }
        dialog.draft.set_visibility(visibility);
        let shown = Timeframe::ALL
            .into_iter()
            .filter(|timeframe| dialog.draft().visibility().shows(*timeframe))
            .count();
        assert_eq!(shown, 1, "the last one cannot be switched off");
    }

    #[test]
    fn a_saved_style_applies_to_the_draft_and_can_become_the_default() {
        let mut store = templates();
        let mut dialog = DrawingProperties::new(1, &drawing(Tool::Rectangle)).expect("dialog");
        let mut thick = dialog.style();
        thick.stroke.width = 9.0;
        dialog.set_style(thick);
        // Save as a template, as the prompt does.
        let id = store
            .save("Thick zone", Tool::Rectangle, &dialog.style())
            .unwrap();
        // Back to the built-in look, then apply the template to the draft.
        dialog.set_style(Tool::Rectangle.default_style());
        assert!(dialog.style().stroke.width != 9.0);
        dialog.set_style(store.book().get(id).expect("saved").style);
        assert_eq!(dialog.style().stroke.width, 9.0);
        // Making it the default changes what a new rectangle starts with, nothing already drawn.
        store.set_default(Tool::Rectangle, &dialog.style()).unwrap();
        assert_eq!(
            crate::model::drawing_template::style_for(Tool::Rectangle, store.defaults())
                .stroke
                .width,
            9.0
        );
        assert_eq!(
            drawing(Tool::Rectangle).style().unwrap().stroke.width,
            Tool::Rectangle.default_style().stroke.width
        );
    }

    /// The new tools' pages as pictures. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_new_pages() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        for (name, tool, tab) in [
            ("dialog-pitchfork-style", Tool::SchiffPitchfork, Tab::Style),
            ("dialog-pitchfork-levels", Tool::Pitchfork, Tab::Levels),
            (
                "dialog-regression-inputs",
                Tool::RegressionTrend,
                Tab::Inputs,
            ),
            ("dialog-regression-style", Tool::RegressionTrend, Tab::Style),
            ("dialog-channel-style", Tool::ParallelChannel, Tab::Style),
            ("dialog-trend-line-text", Tool::TrendLine, Tab::Text),
            ("dialog-vertical-text", Tool::VerticalLine, Tab::Text),
            ("dialog-horizontal-style", Tool::HorizontalLine, Tab::Style),
            ("dialog-info-style", Tool::InfoLine, Tab::Style),
            (
                "dialog-flat-coordinates",
                Tool::FlatChannel,
                Tab::Coordinates,
            ),
        ] {
            let mut dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
            dialog.tab = tab;
            let mut store = templates();
            let mut screen = Screen::new(vec2(520.0, 760.0));
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(Vec::new(), |ui| {
                    dialog.show(ui.ctx(), &mut store);
                }));
            }
            screen.save(name, last.expect("a frame"));
        }
    }
}
