//! Saved styles: the template bar and menu, the name prompt and a template's row in the menu.

use super::{DrawingProperties, Notice, PAGE_MARGIN, Prompt};
use crate::i18n::tr;
use crate::model::Tool;
use crate::model::chart::template::TemplateId;
use crate::model::drawings::template::style_for;
use crate::store::drawing_template::DrawingTemplateStore;
use crate::ui::icons::Icon;
use crate::ui::{
    kit::{self, sheet},
    theme,
};
use eframe::egui::{self, Id, Key, Sense, Ui, pos2, vec2};

/// The least width of the Template menu; a language whose rows are longer widens it.
const MENU_WIDTH: f32 = 250.0;

/// The words of the Template menu's fixed rows: they label the rows and size the menu.
struct MenuText {
    save: &'static str,
    none: &'static str,
    make_default: String,
    reset: &'static str,
    forget: &'static str,
}

impl MenuText {
    fn new(tool: Tool) -> Self {
        Self {
            save: tr!("Save as template…"),
            none: tr!("No saved templates yet"),
            make_default: tr!(
                "Make this the default {tool}",
                tool = tool.label().to_lowercase()
            ),
            reset: tr!("Reset to the default style"),
            forget: tr!("Forget my default"),
        }
    }

    /// The menu as wide as its longest row needs, and at least `MENU_WIDTH`.
    fn width(&self, ui: &Ui) -> f32 {
        let rows = [
            self.save,
            self.none,
            self.make_default.as_str(),
            self.reset,
            self.forget,
        ];
        kit::menu_width_for(ui, &rows, 0.0).max(MENU_WIDTH)
    }
}

impl DrawingProperties {
    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
        });
    }

    /// The row under the page: the Template menu, a name prompt when one is wanted, and what
    /// the last template action did.
    pub(super) fn template_bar(&mut self, ui: &mut Ui, templates: &mut DrawingTemplateStore) {
        // Only tools with a style have anything to save.
        if self.draft.style().is_none() {
            return;
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(sheet::INSET, 10))
            .show(ui, |ui| {
                ui.set_width(self.width - 2.0 * PAGE_MARGIN);
                if self.prompt.is_some() {
                    self.prompt_row(ui, templates);
                    return;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let button = kit::push_button(ui, Some(Icon::Layers), tr!("Template"), true);
                    let width = MenuText::new(self.tool).width(ui);
                    egui::Popup::menu(&button)
                        .frame(kit::menu_frame())
                        .width(width)
                        .show(|ui| self.template_menu(ui, templates));
                    if self.notice.is_none() && templates.load_error().is_some() {
                        ui.add(
                            egui::Label::new(kit::text(
                                tr!("Saved templates could not be read, so none are changed."),
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
        let text = MenuText::new(tool);
        if kit::menu_item(ui, Icon::Plus, text.save, true, false) {
            let name = templates
                .book()
                .free_name(tool, &tr!("My {tool}", tool = tool.label().to_lowercase()));
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
            kit::menu_item(ui, Icon::Layers, text.none, false, false);
        }
        for (id, name) in saved {
            match template_row(ui, &name) {
                Some(RowAction::Apply) => {
                    if let Some(template) = templates.book().get(id) {
                        let style = template.style;
                        self.set_style(style);
                        self.say(tr!("Applied “{name}”", name = name), false);
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
                        Ok(()) => self.say(tr!("Deleted “{name}”", name = name), false),
                        Err(error) => self.say(error, true),
                    }
                    ui.close();
                }
                None => {}
            }
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Check, &text.make_default, true, false) {
            match templates.set_default(tool, &self.style()) {
                Ok(()) => self.say(
                    tr!(
                        "New {tool} drawings start with this style",
                        tool = tool.label().to_lowercase()
                    ),
                    false,
                ),
                Err(error) => self.say(error, true),
            }
            ui.close();
        }
        if kit::menu_item(ui, Icon::Undo, text.reset, true, false) {
            let style = style_for(tool, templates.defaults());
            self.set_style(style);
            self.say(tr!("Back to the default style"), false);
            ui.close();
        }
        let has_default = templates.defaults().contains_key(&tool);
        if kit::menu_item(ui, Icon::Trash, text.forget, has_default, false) {
            match templates.reset_default(tool) {
                Ok(()) => self.say(
                    tr!("New drawings start with the built-in style again"),
                    false,
                ),
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
                                tr!("Template name"),
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
                tr!("Rename")
            } else {
                tr!("Save")
            };
            if kit::push_button(ui, None, label, true).clicked() || enter {
                done = Some(true);
            }
            if kit::push_button(ui, None, tr!("Cancel"), true).clicked() {
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
                        .map(|()| tr!("Renamed to “{name}”", name = name.trim())),
                    None => templates
                        .save(&name, tool, &style)
                        .map(|_| tr!("Saved “{name}”", name = name.trim())),
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
    let response =
        crate::ui::accessibility::label(response, tr!("Apply template {name}", name = name));
    let hovered = response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 5.0, theme::selected_tint(true));
    }
    let color = theme::text_primary();
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
                theme::text_primary(),
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
