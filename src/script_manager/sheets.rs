//! The modal sheets: New Script, the delete confirmation and the error alert. Ported from
//! `ScriptNameSheet` and the `ScriptManagerView` dialogs.

use super::{AlertSheet, ScriptManager, ScriptManagerAction, Sheet, unsaved};
use crate::model::pine_alert::ArmingFacts;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::pine_alert_editor::{self, PineAlertEditorInput, PineAlertEditorOutcome};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Key};

/// Every Script Manager sheet is this wide, as the layout sheets are.
const SHEET_WIDTH: f32 = 420.0;

impl ScriptManager {
    pub(super) fn sheets_ui(&mut self, ctx: &egui::Context) {
        if self.error.is_some() {
            self.error_alert(ctx);
        } else if let Some(prompt) = self.exit_prompt.clone() {
            if let Some(choice) = unsaved::show(ctx, &prompt) {
                self.resolve_exit(&prompt, choice);
            }
        } else if self.ai.review.is_some() {
            self.ai_review_sheet(ctx);
        } else if let Some(sheet) = self.sheet.take() {
            self.sheet = match sheet {
                Sheet::New {
                    name,
                    error,
                    needs_focus,
                } => self.new_script_sheet(ctx, name, error, needs_focus),
                Sheet::ConfirmDelete { id, name } => self.delete_sheet(ctx, id, name),
                Sheet::Alert(sheet) => self.alert_sheet(ctx, sheet),
            };
        }
    }

    fn error_alert(&mut self, ctx: &egui::Context) {
        let message = self.error.clone().unwrap_or_default();
        let mut dismiss = false;
        sheet::show(
            ctx,
            egui::Id::new("script-manager-alert"),
            SHEET_WIDTH,
            |ui| {
                ui.add_space(24.0);
                sheet::inset(ui, |ui| {
                    sheet::header(
                        ui,
                        Icon::TriangleFill,
                        "Script Manager",
                        &message,
                        kit::ORANGE,
                    );
                    ui.add_space(22.0);
                });
                sheet::footer(ui, |ui| {
                    dismiss = kit::sheet_button(ui, "OK", true, true).clicked();
                });
            },
        );
        if dismiss || ctx.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape)) {
            self.error = None;
        }
    }

    fn new_script_sheet(
        &mut self,
        ctx: &egui::Context,
        mut name: String,
        error: Option<String>,
        mut needs_focus: bool,
    ) -> Option<Sheet> {
        // A failed create explains itself until the name is edited.
        let entered = name.clone();
        let problem = self.name_problem(&name).or_else(|| error.clone());
        let can_create =
            !name.trim().is_empty() && problem.is_none() && super::name::validate(&name).is_ok();
        let mut cancel = false;
        let mut create = false;
        let field_id = egui::Id::new("script-manager-new-name");
        sheet::show(
            ctx,
            egui::Id::new("script-manager-new"),
            SHEET_WIDTH,
            |ui| {
                ui.add_space(24.0);
                sheet::inset(ui, |ui| {
                    sheet::header(
                        ui,
                        Icon::Curlybraces,
                        "New Script",
                        "Create a script file in your library.",
                        theme::ACCENT,
                    );
                    ui.add_space(20.0);
                    let focused = ui.memory(|memory| memory.has_focus(field_id));
                    kit::field_frame(focused).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            icons::inline(ui, Icon::Curlybraces, 13.0, theme::text_secondary());
                            let width = ui.available_width();
                            let response = kit::plain_field(
                                ui,
                                field_id,
                                &mut name,
                                "Script name",
                                kit::size::BODY,
                                kit::REGULAR,
                                width,
                            );
                            if std::mem::take(&mut needs_focus) {
                                response.request_focus();
                            }
                        });
                    });
                    // The problem line keeps its height with or without a message, so the buttons
                    // don't jump as the name becomes valid.
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.set_min_height(16.0);
                        if let Some(problem) = &problem {
                            icons::inline(ui, Icon::TriangleFill, 11.0, kit::ORANGE);
                            ui.label(kit::text(
                                problem,
                                kit::size::SUBHEADLINE,
                                kit::REGULAR,
                                theme::text_secondary(),
                            ));
                        }
                    });
                    ui.add_space(14.0);
                });
                sheet::footer(ui, |ui| {
                    if kit::sheet_button(ui, "Create", true, can_create).clicked() {
                        create = true;
                    }
                    if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                        cancel = true;
                    }
                });
            },
        );
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        } else if can_create && ctx.input(|i| i.key_pressed(Key::Enter)) {
            create = true;
        }
        if cancel {
            return None;
        }
        if create {
            return match self.create_script(&name) {
                Ok(()) => None,
                Err(message) => Some(Sheet::New {
                    name,
                    error: Some(message),
                    needs_focus: true,
                }),
            };
        }
        Some(Sheet::New {
            error: error.filter(|_| name == entered),
            name,
            needs_focus,
        })
    }

    /// The Create Script Alert sheet: arms the script's `alert()` calls on the focused chart.
    fn alert_sheet(&mut self, ctx: &egui::Context, mut sheet: Box<AlertSheet>) -> Option<Sheet> {
        let chart = sheet.chart.clone();
        let facts = ArmingFacts {
            loaded: true,
            instance_exists: true,
            source_loaded: true,
            call_sites: sheet.call_sites.len(),
            duplicate_exists: sheet.duplicate_exists,
        };
        let input = PineAlertEditorInput {
            chart_title: &chart.title,
            source_name: &chart.source_name,
            timeframe: &chart.timeframe,
            script_name: &sheet.name,
            facts,
            call_sites: &sheet.call_sites,
            endpoints: &sheet.endpoints,
        };
        match pine_alert_editor::show(ctx, &mut sheet.state, &input) {
            Some(PineAlertEditorOutcome::Create {
                note,
                webhook_endpoint_ids,
            }) => {
                self.actions.push(ScriptManagerAction::CreateAlert {
                    chart_id: chart.chart_id,
                    instance_id: sheet.instance.id,
                    script_id: sheet.instance.script_id.clone(),
                    script_name: sheet.name.clone(),
                    source: sheet.source.clone(),
                    inputs: sheet.instance.inputs.clone(),
                    symbol_key: chart.symbol_key.clone(),
                    timeframe: chart.timeframe.clone(),
                    note,
                    webhook_endpoint_ids,
                });
                None
            }
            Some(PineAlertEditorOutcome::Cancel) => None,
            None => Some(Sheet::Alert(sheet)),
        }
    }

    fn delete_sheet(&mut self, ctx: &egui::Context, id: String, name: String) -> Option<Sheet> {
        let title = format!("Delete \"{name}\"?");
        let result = kit::confirm(
            ctx,
            egui::Id::new("script-manager-delete"),
            &Confirm {
                icon: Icon::Trash,
                title: &title,
                message: "The script file will be moved to the Trash.",
                destructive: "Delete Script",
            },
        );
        if result == Some(ConfirmOutcome::Confirmed) {
            self.delete_script(&id);
            return None;
        }
        if result == Some(ConfirmOutcome::Cancelled) {
            return None;
        }
        Some(Sheet::ConfirmDelete { id, name })
    }
}
