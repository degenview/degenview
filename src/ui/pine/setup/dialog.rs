//! "Confirm Inputs": the inputs a script marks `confirm`, asked before it is added. The points
//! already clicked are there too, to adjust.

use crate::i18n::tr;
use crate::pine::compiler::program::InputSchema;
use crate::pine::inputs::values::{InputEdit, InputValues};
use crate::pine_studio::inputs;
use crate::ui::kit::dialog::close_button;
use crate::ui::kit::{self, sheet};
use crate::ui::theme;
use eframe::egui::{self, Id, Key, ScrollArea};

const WIDTH: f32 = 440.0;
const MAX_LIST_HEIGHT: f32 = 360.0;

/// What the user did in the sheet this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub edits: Vec<InputEdit>,
    pub answer: Option<Answer>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Answer {
    /// Add the script with these values.
    Apply,
    /// Do not add the script.
    Cancel,
}

/// A click beside the sheet does nothing: cancelling takes the script off, so it is only ever
/// asked for.
pub fn show(
    ctx: &egui::Context,
    chart_id: u64,
    schema: &InputSchema,
    values: &InputValues,
) -> Outcome {
    let mut outcome = Outcome::default();
    let id = Id::new(("confirm-inputs", chart_id));
    sheet::show(ctx, id, WIDTH, |ui| {
        sheet::inset(ui, |ui| {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.label(kit::text(
                    tr!("Confirm Inputs"),
                    kit::size::TITLE2,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(24.0), egui::Sense::hover());
                    if close_button(ui, id.with("close"), rect).clicked() {
                        outcome.answer = Some(Answer::Cancel);
                    }
                });
            });
            ui.add_space(14.0);
            ScrollArea::vertical()
                .max_height(MAX_LIST_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    outcome.edits = inputs::cards(ui, schema, values, |input| input.confirm);
                });
            ui.add_space(14.0);
        });
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, tr!("Apply"), true, true).clicked() {
                outcome.answer = Some(Answer::Apply);
            }
            if kit::sheet_button(ui, tr!("Cancel"), false, true).clicked() {
                outcome.answer = Some(Answer::Cancel);
            }
        });
    });
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        outcome.answer = Some(Answer::Cancel);
    }
    outcome
}
