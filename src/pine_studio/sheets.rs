//! The modal sheets: New Script, Rename, the delete confirmation and the error alert. Ported from
//! `ScriptNameSheet` and the `PineStudioView` dialogs.

use super::{AlertSheet, PineStudio, PineStudioAction, Sheet, library::unsaved};
use crate::model::alert::pine::ArmingFacts;
use crate::model::pine::script::ScriptId;
use crate::ui::alert::pine_editor::{self, PineAlertEditorInput, PineAlertEditorOutcome};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::sheet;
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::theme;
use eframe::egui::{self, Key};

/// Every Pine Studio sheet is this wide, as the layout sheets are.
const SHEET_WIDTH: f32 = 420.0;

impl PineStudio {
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
                Sheet::Rename {
                    id,
                    name,
                    error,
                    needs_focus,
                } => self.rename_sheet(ctx, id, name, error, needs_focus),
                Sheet::ConfirmDelete { id, name } => self.delete_sheet(ctx, id, name),
                Sheet::Alert(sheet) => self.alert_sheet(ctx, sheet),
            };
        }
    }

    fn error_alert(&mut self, ctx: &egui::Context) {
        let message = self.error.clone().unwrap_or_default();
        let mut dismiss = false;
        sheet::show(ctx, egui::Id::new("pine-studio-alert"), SHEET_WIDTH, |ui| {
            ui.add_space(24.0);
            sheet::inset(ui, |ui| {
                sheet::header(ui, Icon::TriangleFill, "Pine Studio", &message, kit::ORANGE);
                ui.add_space(22.0);
            });
            sheet::footer(ui, |ui| {
                dismiss = kit::sheet_button(ui, "OK", true, true).clicked();
            });
        });
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
        let problem = self.name_problem(&name, None).or_else(|| error.clone());
        let can_create = name_is_usable(&name, problem.as_deref());
        let event = name_sheet(
            ctx,
            &NameSheet {
                salt: "new",
                icon: Icon::AngleBrackets,
                title: "New Script",
                subtitle: "Create a script file in your library.",
                action: "Create",
            },
            &mut name,
            &mut needs_focus,
            problem.as_deref(),
            can_create,
        );
        match event {
            NameEvent::Cancel => None,
            NameEvent::Submit => match self.create_script(&name) {
                Ok(()) => None,
                Err(message) => Some(Sheet::New {
                    name,
                    error: Some(message),
                    needs_focus: true,
                }),
            },
            NameEvent::Editing => Some(Sheet::New {
                error: error.filter(|_| name == entered),
                name,
                needs_focus,
            }),
        }
    }

    fn rename_sheet(
        &mut self,
        ctx: &egui::Context,
        id: ScriptId,
        mut name: String,
        error: Option<String>,
        mut needs_focus: bool,
    ) -> Option<Sheet> {
        let Some(current) = self.script_named(&id).map(|script| script.name.clone()) else {
            // Deleted in the Finder while the sheet was up.
            return None;
        };
        let entered = name.clone();
        let problem = self
            .name_problem(&name, Some(&id))
            .or_else(|| error.clone());
        let can_rename = name_is_usable(&name, problem.as_deref());
        let subtitle = format!("Choose a new name for \u{201c}{current}\u{201d}.");
        let event = name_sheet(
            ctx,
            &NameSheet {
                salt: "rename",
                icon: Icon::Pencil,
                title: "Rename Script",
                subtitle: &subtitle,
                action: "Rename",
            },
            &mut name,
            &mut needs_focus,
            problem.as_deref(),
            can_rename,
        );
        match event {
            NameEvent::Cancel => None,
            NameEvent::Submit => match self.rename_script(&id, &name) {
                Ok(()) => None,
                Err(message) => Some(Sheet::Rename {
                    id,
                    name,
                    error: Some(message),
                    needs_focus: true,
                }),
            },
            NameEvent::Editing => Some(Sheet::Rename {
                error: error.filter(|_| name == entered),
                id,
                name,
                needs_focus,
            }),
        }
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
        match pine_editor::show(ctx, &mut sheet.state, &input) {
            Some(PineAlertEditorOutcome::Create {
                note,
                webhook_endpoint_ids,
            }) => {
                self.actions.push(PineStudioAction::CreateAlert {
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
            Some(PineAlertEditorOutcome::OpenWebhookSettings) => {
                self.actions.push(PineStudioAction::OpenWebhookSettings);
                Some(Sheet::Alert(sheet))
            }
            None => Some(Sheet::Alert(sheet)),
        }
    }

    fn delete_sheet(&mut self, ctx: &egui::Context, id: String, name: String) -> Option<Sheet> {
        let title = format!("Delete \"{name}\"?");
        let result = kit::confirm(
            ctx,
            egui::Id::new("pine-studio-delete"),
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

/// What the sheet that asks for a script's name says.
struct NameSheet<'a> {
    /// Tells the sheets apart in egui's memory.
    salt: &'static str,
    icon: Icon,
    title: &'a str,
    subtitle: &'a str,
    /// The button that takes the name.
    action: &'a str,
}

enum NameEvent {
    Submit,
    Cancel,
    Editing,
}

/// Whether `name` can be given to a script: something is typed and nothing is wrong with it.
fn name_is_usable(name: &str, problem: Option<&str>) -> bool {
    !name.trim().is_empty() && problem.is_none() && super::library::name::validate(name).is_ok()
}

/// The name field sheet New Script and Rename share: the field, a line for what is wrong with the
/// name, the action and Cancel. Enter takes the name when `can_submit`, Escape cancels.
fn name_sheet(
    ctx: &egui::Context,
    spec: &NameSheet,
    name: &mut String,
    needs_focus: &mut bool,
    problem: Option<&str>,
    can_submit: bool,
) -> NameEvent {
    let mut cancel = false;
    let mut submit = false;
    let field_id = egui::Id::new(("pine-studio-name-field", spec.salt));
    sheet::show(
        ctx,
        egui::Id::new(("pine-studio-name", spec.salt)),
        SHEET_WIDTH,
        |ui| {
            ui.add_space(24.0);
            sheet::inset(ui, |ui| {
                sheet::header(ui, spec.icon, spec.title, spec.subtitle, theme::ACCENT);
                ui.add_space(20.0);
                let focused = ui.memory(|memory| memory.has_focus(field_id));
                kit::field_frame(focused).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        icons::inline(ui, Icon::AngleBrackets, 13.0, theme::text_secondary());
                        let width = ui.available_width();
                        let response = kit::plain_field(
                            ui,
                            field_id,
                            name,
                            "Script name",
                            kit::size::BODY,
                            kit::REGULAR,
                            width,
                        );
                        if std::mem::take(needs_focus) {
                            response.request_focus();
                        }
                    });
                });
                // The problem line keeps its height with or without a message, so the buttons
                // don't jump as the name becomes valid.
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.set_min_height(16.0);
                    if let Some(problem) = problem {
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
                if kit::sheet_button(ui, spec.action, true, can_submit).clicked() {
                    submit = true;
                }
                if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                    cancel = true;
                }
            });
        },
    );
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        cancel = true;
    } else if can_submit && ctx.input(|i| i.key_pressed(Key::Enter)) {
        submit = true;
    }
    if cancel {
        NameEvent::Cancel
    } else if submit {
        NameEvent::Submit
    } else {
        NameEvent::Editing
    }
}
