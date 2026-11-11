//! Inputs and Style for one applied Pine script, with explicit Apply and Cancel so the script does
//! not rerun on every keystroke. Ported from `PineInstanceSettingsPopover`; the chart legend's
//! chips and the chart settings sheet's script rows both open it, pointing at themselves.

use super::icons::Icon;
use super::kit::{self, SEMIBOLD};
use super::pine_style_view;
use super::popover;
use super::theme;
use crate::model::pine::instance::PineInstance;
use crate::pine::compiler;
use crate::pine::compiler::program::InputSchema;
use crate::pine::outputs::style_overrides::{StyleOverrides, StyleRow};
use crate::pine::syntax::token::Limits;
use crate::pine_studio::inputs::{self, InputValues};
use crate::pine_studio::library::registry::LibraryRegistry;
use eframe::egui::{Align, Context, Id, Layout, Rect, ScrollArea, Ui};
use std::collections::HashMap;

const WIDTH: f32 = 440.0;
const PADDING: i8 = 16;
/// Swift's `ScrollView.frame(maxHeight:)`.
const MAX_PAGE_HEIGHT: f32 = 320.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Inputs,
    Style,
}

/// What the popover wants after a frame.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Open,
    Cancel,
    Apply {
        inputs: InputValues,
        style: HashMap<String, String>,
    },
}

pub struct PineSettingsPopover {
    chart_id: u64,
    instance_id: u64,
    anchor: Rect,
    title: String,
    schema: InputSchema,
    rows: Vec<StyleRow>,
    /// The edits so far; nothing reaches the chart until Apply.
    inputs: InputValues,
    style: HashMap<String, String>,
    page: Page,
}

impl PineSettingsPopover {
    /// `rows` are the styleable outputs of the script's last run; with none (it has not run, or
    /// draws nothing) there is no Style page.
    pub fn open(chart_id: u64, instance: &PineInstance, rows: Vec<StyleRow>, anchor: Rect) -> Self {
        let program = compiler::compile(&instance.source, Limits::default(), &LibraryRegistry);
        let declared = program.declaration.title.trim();
        let title = if declared.is_empty() {
            "Script".to_owned()
        } else {
            declared.to_owned()
        };
        Self {
            chart_id,
            instance_id: instance.id,
            anchor,
            title,
            schema: program.input_schema,
            rows,
            inputs: instance.input_values(),
            style: instance.style_overrides.clone(),
            page: Page::Inputs,
        }
    }

    pub fn is_for(&self, chart_id: u64, instance_id: u64) -> bool {
        self.chart_id == chart_id && self.instance_id == instance_id
    }

    pub fn chart_id(&self) -> u64 {
        self.chart_id
    }

    pub fn instance_id(&self) -> u64 {
        self.instance_id
    }

    pub fn show(&mut self, ctx: &Context) -> Outcome {
        let id = Id::new(("pine-settings-popover", self.chart_id, self.instance_id));
        let mut outcome = Outcome::Open;
        let content_width = WIDTH - 2.0 * f32::from(PADDING) - 2.0;
        let dismissed = popover::show(ctx, id, self.anchor, WIDTH, PADDING, true, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(kit::text(
                &self.title,
                kit::size::BODY + 1.0,
                SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add_space(12.0);
            // Only a script with both kinds of setting gets the page switcher; one with a single
            // kind opens straight on it.
            self.page = self.visible_page();
            if self.has_inputs() && self.has_style() {
                // The app's icon tab bar: a tall track with a raised plate under the page in view,
                // and how many settings each page holds.
                kit::tab_bar(
                    ui,
                    id.with("page"),
                    &[
                        kit::TabItem {
                            value: Page::Inputs,
                            title: "Inputs",
                            icon: Icon::Sliders,
                            count: self.schema.inputs.len(),
                        },
                        kit::TabItem {
                            value: Page::Style,
                            title: "Style",
                            icon: Icon::Palette,
                            count: self.rows.len(),
                        },
                    ],
                    &mut self.page,
                );
                ui.add_space(12.0);
            }
            ScrollArea::vertical()
                .id_salt(("pine-settings-page", self.page as u8))
                .max_height(MAX_PAGE_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_width(content_width);
                    self.page_content(ui);
                });
            ui.add_space(12.0);
            outcome = self.footer(ui);
        });
        if dismissed {
            return Outcome::Cancel;
        }
        outcome
    }

    fn has_inputs(&self) -> bool {
        !self.schema.inputs.is_empty()
    }

    fn has_style(&self) -> bool {
        !self.rows.is_empty()
    }

    /// The page to show: the chosen one when the script has it, else the one it does have.
    /// A script with neither shows the (empty) inputs page.
    fn visible_page(&self) -> Page {
        match (self.has_inputs(), self.has_style()) {
            (false, true) => Page::Style,
            (true, false) | (false, false) => Page::Inputs,
            (true, true) => self.page,
        }
    }

    fn page_content(&mut self, ui: &mut Ui) {
        if !self.has_inputs() && !self.has_style() {
            ui.label(kit::text(
                "This script has no inputs or style options.",
                kit::size::BODY,
                kit::REGULAR,
                theme::text_secondary(),
            ));
            return;
        }
        match self.page {
            Page::Style => {
                if let Some(next) = pine_style_view::show(ui, &self.rows, &self.style) {
                    self.style = next;
                }
            }
            Page::Inputs => {
                for edit in inputs::cards(ui, &self.schema, &self.inputs) {
                    inputs::apply_edit(&mut self.inputs, &self.schema, edit);
                }
            }
        }
    }

    /// "Reset Style" on the Style page, then Cancel and Apply at the trailing edge.
    fn footer(&mut self, ui: &mut Ui) -> Outcome {
        let mut outcome = Outcome::Open;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if self.page == Page::Style
                && kit::push_button(ui, None, "Reset Style", !self.style.is_empty()).clicked()
            {
                self.style.clear();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::sheet_button(ui, "Apply", true, true).clicked() {
                    outcome = Outcome::Apply {
                        inputs: self.inputs.clone(),
                        style: self.applied_style(),
                    };
                }
                if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        });
        outcome
    }

    /// The style choices that still belong to an output the script draws.
    fn applied_style(&self) -> HashMap<String, String> {
        if self.rows.is_empty() {
            self.style.clone()
        } else {
            StyleOverrides::pruned(self.style.clone(), &self.rows)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::outputs::style_overrides::StyleKind;
    use eframe::egui::{self, pos2, vec2};

    fn instance(source: &str) -> PineInstance {
        PineInstance::new(1, None, "Test", source)
    }

    fn row() -> StyleRow {
        StyleRow {
            kind: StyleKind::Plot,
            output_id: 0,
            title: "Plot 1".into(),
            default_color: Some(0xff00_00ff),
            default_width: Some(2),
        }
    }

    fn anchor() -> Rect {
        Rect::from_min_size(pos2(300.0, 40.0), vec2(80.0, 20.0))
    }

    fn draw(popover: &mut PineSettingsPopover) -> Outcome {
        let ctx = Context::default();
        let mut outcome = Outcome::Open;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                outcome = popover.show(ui.ctx());
            });
            output.textures_delta.clear();
        }
        outcome
    }

    #[test]
    fn the_title_is_the_declaration_and_the_schema_comes_from_the_source() {
        let source =
            "//@version=5\nindicator(\"My RSI\")\nlen = input.int(14, \"Length\")\nplot(close)";
        let popover = PineSettingsPopover::open(3, &instance(source), Vec::new(), anchor());
        assert_eq!(popover.title, "My RSI");
        assert_eq!(popover.schema.inputs.len(), 1);
        assert!(popover.is_for(3, 1) && !popover.is_for(2, 1));
    }

    #[test]
    fn the_popover_draws_both_pages_and_waits_for_a_choice() {
        let source = "//@version=5\nindicator(\"X\")\nlen = input.int(14, \"Length\")\nplot(close)";
        let mut popover = PineSettingsPopover::open(1, &instance(source), vec![row()], anchor());
        assert_eq!(draw(&mut popover), Outcome::Open);
        popover.page = Page::Style;
        popover
            .style
            .insert("plot.0.visible".into(), "false".into());
        assert_eq!(draw(&mut popover), Outcome::Open);
    }

    #[test]
    fn a_script_with_nothing_to_style_stays_on_its_inputs() {
        let mut popover =
            PineSettingsPopover::open(1, &instance("plot(close)"), Vec::new(), anchor());
        popover.page = Page::Style;
        draw(&mut popover);
        assert_eq!(popover.page, Page::Inputs);
    }

    #[test]
    fn the_pages_follow_what_the_script_has() {
        let with_inputs =
            "//@version=5\nindicator(\"X\")\nlen = input.int(14, \"Length\")\nplot(close)";
        let page = |source: &str, rows: Vec<StyleRow>, chosen: Page| {
            let mut popover = PineSettingsPopover::open(1, &instance(source), rows, anchor());
            popover.page = chosen;
            draw(&mut popover);
            (popover.has_inputs(), popover.has_style(), popover.page)
        };
        // Both: the switcher is there and the choice is kept.
        assert_eq!(
            page(with_inputs, vec![row()], Page::Style),
            (true, true, Page::Style)
        );
        assert_eq!(
            page(with_inputs, vec![row()], Page::Inputs),
            (true, true, Page::Inputs)
        );
        // Only inputs: no Style page to be on.
        assert_eq!(
            page(with_inputs, Vec::new(), Page::Style),
            (true, false, Page::Inputs)
        );
        // Only style: straight to it, whatever page was remembered.
        assert_eq!(
            page("plot(close)", vec![row()], Page::Inputs),
            (false, true, Page::Style)
        );
        // Neither: the empty inputs page.
        assert_eq!(
            page("plot(close)", Vec::new(), Page::Style),
            (false, false, Page::Inputs)
        );
    }

    #[test]
    fn apply_drops_style_choices_for_outputs_that_are_gone() {
        let mut popover =
            PineSettingsPopover::open(1, &instance("plot(close)"), vec![row()], anchor());
        popover
            .style
            .insert("plot.0.visible".into(), "false".into());
        popover
            .style
            .insert("plot.9.visible".into(), "false".into());
        let kept = popover.applied_style();
        assert_eq!(kept.len(), 1);
        assert!(kept.contains_key("plot.0.visible"));
        // With no rows to judge by (the script has not run) nothing is dropped.
        popover.rows.clear();
        assert_eq!(popover.applied_style().len(), 2);
    }

    /// The popover as pictures. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_popover() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let source = "//@version=5\nindicator(\"Squeeze Momentum\")\nlen = input.int(20, \"Length\")\nmult = input.float(2.0, \"Multiplier\")\nplot(close)";
        for (name, page) in [
            ("popover-inputs", Page::Inputs),
            ("popover-style", Page::Style),
        ] {
            let mut popover =
                PineSettingsPopover::open(1, &instance(source), vec![row()], anchor());
            popover.page = page;
            let mut screen = Screen::new(vec2(560.0, 520.0));
            let mut last = None;
            for _ in 0..4 {
                last = Some(screen.run(Vec::new(), |ui| {
                    popover.show(ui.ctx());
                }));
            }
            screen.save(name, last.expect("a frame"));
        }
    }
}
