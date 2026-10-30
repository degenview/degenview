//! Chart appearance templates in the chart settings: the Templates menu in the sheet's footer,
//! the save, manage and confirmation dialogs it opens, and the line of feedback after each
//! action. A template restyles the chart (type, colors, canvas) and nothing else; this module only
//! asks the card to apply one, so symbols, indicators, scripts, drawings and alerts never move.
//!
//! The saved templates are read when the sheet opens and again before every change (see
//! [`ChartTemplateStore`]); there is no database access in the paint path.

use super::chart_card::ChartCard;
use super::icons::{self, Icon};
use super::kit::{self, REGULAR, SEMIBOLD, size};
use super::sheet;
use super::theme;
use crate::model::ChartAppearance;
use crate::model::chart_template::{
    ApplyScope, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS, TemplateEntry, TemplateId, clean_name,
};
use crate::store::SharedStore;
use crate::store::chart_template::ChartTemplateStore;
use eframe::egui::{self, Align, Id, Layout, Rect, Sense, Ui, vec2};
use std::time::{Duration, Instant};

const DIALOG_WIDTH: f32 = 460.0;
const MANAGE_WIDTH: f32 = 600.0;
const NOTICE_LIFETIME: Duration = Duration::from_secs(7);

/// What the template dialogs ask the workspace to do beyond the one chart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TemplateAction {
    /// Give every chart of the workspace the look of the chart whose sheet this is, taking from
    /// it what the scope says.
    ApplyToAll(ApplyScope),
}

#[derive(Clone, Debug)]
enum Dialog {
    Save {
        name: String,
        description: String,
        focus: bool,
        error: Option<String>,
    },
    Manage(Manage),
    Update(TemplateId),
    Delete(TemplateId),
    ApplyToAll,
    ResetAppearance,
}

#[derive(Clone, Debug, Default)]
struct Manage {
    renaming: Option<Renaming>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct Renaming {
    id: TemplateId,
    name: String,
    description: String,
    focus: bool,
}

struct Notice {
    text: String,
    error: bool,
    at: Instant,
}

/// The template side of one open settings sheet.
pub struct TemplateState {
    store: Option<ChartTemplateStore>,
    /// What applying a template takes from it.
    pub scope: ApplyScope,
    dialog: Option<Dialog>,
    notice: Option<Notice>,
}

impl Default for TemplateState {
    fn default() -> Self {
        Self {
            store: None,
            scope: ApplyScope::AppearanceAndCanvas,
            dialog: None,
            notice: None,
        }
    }
}

impl TemplateState {
    /// Whether a template dialog is up, so Escape closes it before it closes the sheet.
    pub fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }

    /// Opens the dialog that saves the chart's current look as a template.
    pub fn open_save(&mut self) {
        self.dialog = Some(Dialog::Save {
            name: String::new(),
            description: String::new(),
            focus: true,
            error: None,
        });
    }

    /// Opens the dialog that renames, updates and deletes saved templates.
    pub fn open_manage(&mut self) {
        self.dialog = Some(Dialog::Manage(Manage::default()));
    }

    /// Opens the confirmation that puts the chart's look back to the defaults.
    pub fn open_reset(&mut self) {
        self.dialog = Some(Dialog::ResetAppearance);
    }

    /// The presets and saved templates, presets first, as the chart menu lists them.
    pub fn entries(&mut self, store: &SharedStore) -> Vec<TemplateEntry> {
        self.templates(store).book().entries()
    }

    /// Gives `card` the template `id`, taking from it what the scope says. Returns its name, or
    /// why it could not be applied.
    pub fn apply(
        &mut self,
        store: &SharedStore,
        id: TemplateId,
        card: &mut ChartCard,
    ) -> Result<String, String> {
        let scope = self.scope;
        let entry = self
            .templates(store)
            .book()
            .entry(id)
            .ok_or_else(|| "That template no longer exists.".to_owned())?;
        card.apply_template(&entry.appearance, scope);
        Ok(entry.name)
    }

    /// The feedback the last dialog left, once: `(text, is_error)`.
    pub fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take().map(|notice| (notice.text, notice.error))
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    fn complain(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    fn templates(&mut self, store: &SharedStore) -> &mut ChartTemplateStore {
        self.store
            .get_or_insert_with(|| ChartTemplateStore::new(store.clone()))
    }
}

// MARK: Footer

/// The feedback line, centred in the footer's free space.
pub fn notice(ui: &mut Ui, state: &mut TemplateState) {
    if state
        .notice
        .as_ref()
        .is_some_and(|notice| notice.at.elapsed() > NOTICE_LIFETIME)
    {
        state.notice = None;
    }
    let Some(notice) = &state.notice else {
        return;
    };
    ui.ctx().request_repaint_after(Duration::from_secs(1));
    let color = if notice.error {
        kit::ORANGE
    } else {
        theme::text_secondary()
    };
    let icon = if notice.error {
        Icon::TriangleFill
    } else {
        Icon::Check
    };
    icons::inline(ui, icon, 12.0, color);
    let text = kit::text(&notice.text, size::SUBHEADLINE, REGULAR, color);
    ui.add(egui::Label::new(text).truncate());
}

/// The "Templates ▾" button and its menu.
pub fn menu(ui: &mut Ui, state: &mut TemplateState, store: &SharedStore, card: &mut ChartCard) {
    let response = kit::push_button(ui, Some(Icon::Palette), "Templates", true);
    let response = super::accessibility::label(response, "Templates")
        .on_hover_text("Save this chart's look, or give it another.");
    let scope = state.scope;
    // Read before the menu opens: the list is the sheet's, not each frame's.
    let entries: Vec<TemplateEntry> = state.templates(store).book().entries();
    let load_error = state.templates(store).load_error().map(str::to_owned);
    let mut next_dialog = None;
    let mut next_scope = None;
    let mut applied: Option<TemplateEntry> = None;
    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        .width(290.0)
        .show(|ui| {
            ui.set_min_width(280.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if kit::menu_item(
                ui,
                Icon::Plus,
                "Save current settings as template…",
                true,
                false,
            ) {
                next_dialog = Some(Dialog::Save {
                    name: String::new(),
                    description: String::new(),
                    focus: true,
                    error: None,
                });
                ui.close();
            }
            kit::menu_separator(ui);
            heading(ui, "When applying");
            for option in ApplyScope::ALL {
                if kit::menu_item_marked(ui, Icon::Check, option.label(), option == scope) {
                    next_scope = Some(option);
                }
            }
            kit::menu_separator(ui);
            let saved: Vec<&TemplateEntry> = entries.iter().filter(|e| !e.built_in).collect();
            heading(ui, "My templates");
            if load_error.is_some() {
                disabled(ui, "Saved templates could not be read");
            } else if saved.is_empty() {
                disabled(ui, "None saved yet");
            }
            for entry in saved {
                if template_item(ui, entry) {
                    applied = Some(entry.clone());
                    ui.close();
                }
            }
            kit::menu_separator(ui);
            heading(ui, "Presets");
            for entry in entries.iter().filter(|e| e.built_in) {
                if template_item(ui, entry) {
                    applied = Some(entry.clone());
                    ui.close();
                }
            }
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::Sliders, "Manage Templates…", true, false) {
                next_dialog = Some(Dialog::Manage(Manage::default()));
                ui.close();
            }
            kit::menu_separator(ui);
            if kit::menu_item(
                ui,
                Icon::CopyDoc,
                "Apply current settings to all charts…",
                true,
                false,
            ) {
                next_dialog = Some(Dialog::ApplyToAll);
                ui.close();
            }
            if kit::menu_item(
                ui,
                Icon::Undo,
                "Reset chart appearance to defaults…",
                true,
                false,
            ) {
                next_dialog = Some(Dialog::ResetAppearance);
                ui.close();
            }
        });
    if let Some(next) = next_scope {
        state.scope = next;
    }
    if let Some(entry) = applied {
        card.apply_template(&entry.appearance, state.scope);
        state.say(format!(
            "Applied “{}”{}.",
            entry.name,
            if state.scope == ApplyScope::ColorsAndCanvas {
                " (colors and canvas only)"
            } else {
                ""
            }
        ));
    }
    if let Some(dialog) = next_dialog {
        state.dialog = Some(dialog);
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(3.0);
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(9.5)
            .strong()
            .color(theme::text_tertiary()),
    );
    ui.add_space(2.0);
}

fn disabled(ui: &mut Ui, text: &str) {
    kit::menu_item(ui, Icon::Circle, text, false, false);
}

fn template_item(ui: &mut Ui, entry: &TemplateEntry) -> bool {
    kit::menu_item(ui, Icon::Palette, &entry.name, true, false)
}

// MARK: Dialogs

/// Draws the open dialog, if any. Returns what the workspace has to do.
pub fn dialogs(
    ctx: &egui::Context,
    state: &mut TemplateState,
    store: &SharedStore,
    card: &mut ChartCard,
) -> Option<TemplateAction> {
    let dialog = state.dialog.take()?;
    let mut keep = None;
    let mut action = None;
    match dialog {
        Dialog::Save {
            mut name,
            mut description,
            mut focus,
            mut error,
        } => {
            if name.is_empty() && focus {
                name = state.templates(store).book().free_name("My Template");
            }
            match save_dialog(
                ctx,
                &mut name,
                &mut description,
                &mut focus,
                error.as_deref(),
            ) {
                Some(Outcome::Confirm) => {
                    let result =
                        state
                            .templates(store)
                            .save(&name, &description, &card.appearance());
                    match result {
                        Ok(_) => state.say(format!("Saved the template “{}”.", name.trim())),
                        Err(message) => {
                            error = Some(message);
                            keep = Some(Dialog::Save {
                                name,
                                description,
                                focus,
                                error,
                            });
                        }
                    }
                }
                Some(Outcome::Cancel) => {}
                None => {
                    keep = Some(Dialog::Save {
                        name,
                        description,
                        focus,
                        error,
                    });
                }
            }
        }
        Dialog::Manage(mut manage) => match manage_dialog(ctx, state, store, card, &mut manage) {
            ManageOutcome::Close => {}
            ManageOutcome::Stay => keep = Some(Dialog::Manage(manage)),
            ManageOutcome::Open(next) => keep = Some(next),
        },
        Dialog::Update(id) => {
            let name = state
                .templates(store)
                .book()
                .entry(id)
                .map(|entry| entry.name);
            match name {
                None => {}
                Some(name) => match confirm(
                    ctx,
                    "chart-template-update",
                    Icon::Download,
                    &format!("Update “{name}”?"),
                    "Replaces the template's look with this chart's current look. Charts that already use it keep theirs.",
                    "Update Template",
                ) {
                    Some(Outcome::Confirm) => {
                        match state
                            .templates(store)
                            .update_appearance(id, &card.appearance())
                        {
                            Ok(()) => state.say(format!("Updated “{name}” from this chart.")),
                            Err(message) => state.complain(message),
                        }
                        keep = Some(Dialog::Manage(Manage::default()));
                    }
                    Some(Outcome::Cancel) => keep = Some(Dialog::Manage(Manage::default())),
                    None => keep = Some(Dialog::Update(id)),
                },
            }
        }
        Dialog::Delete(id) => {
            let name = state
                .templates(store)
                .book()
                .entry(id)
                .map(|entry| entry.name);
            if let Some(name) = name {
                let title = format!("Delete “{name}”?");
                match kit::confirm(
                    ctx,
                    Id::new("chart-template-delete"),
                    &kit::Confirm {
                        icon: Icon::Trash,
                        title: &title,
                        message: "Charts that already use it keep their look.",
                        destructive: "Delete Template",
                    },
                ) {
                    Some(kit::ConfirmOutcome::Confirmed) => {
                        match state.templates(store).delete(id) {
                            Ok(()) => state.say(format!("Deleted “{name}”.")),
                            Err(message) => state.complain(message),
                        }
                        keep = Some(Dialog::Manage(Manage::default()));
                    }
                    Some(kit::ConfirmOutcome::Cancelled) => {
                        keep = Some(Dialog::Manage(Manage::default()));
                    }
                    None => keep = Some(Dialog::Delete(id)),
                }
            }
        }
        Dialog::ApplyToAll => {
            let scope = state.scope;
            let message = match scope {
                ApplyScope::AppearanceAndCanvas => {
                    "Gives every chart in this workspace this chart's type, colors and canvas. Symbols, timeframes, indicators, scripts, alerts and drawings stay as they are."
                }
                ApplyScope::ColorsAndCanvas => {
                    "Gives every chart in this workspace this chart's colors and canvas. Each keeps its own chart type, symbol, timeframe, indicators, scripts, alerts and drawings."
                }
            };
            match confirm(
                ctx,
                "chart-template-apply-all",
                Icon::CopyDoc,
                "Apply to all charts in this workspace?",
                message,
                "Apply to All Charts",
            ) {
                Some(Outcome::Confirm) => {
                    action = Some(TemplateAction::ApplyToAll(scope));
                }
                Some(Outcome::Cancel) => {}
                None => keep = Some(Dialog::ApplyToAll),
            }
        }
        Dialog::ResetAppearance => match confirm(
            ctx,
            "chart-template-reset",
            Icon::Undo,
            "Reset chart appearance?",
            "Puts the chart type and its settings, the colors and the whole canvas back to the defaults. Indicators, scripts, drawings and alerts stay. The price axis' decimals stay too.",
            "Reset Appearance",
        ) {
            Some(Outcome::Confirm) => {
                card.apply_template(&ChartAppearance::default(), ApplyScope::AppearanceAndCanvas);
                state.say("Reset the chart's appearance to the defaults.");
            }
            Some(Outcome::Cancel) => {}
            None => keep = Some(Dialog::ResetAppearance),
        },
    }
    state.dialog = keep;
    action
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    Confirm,
    Cancel,
}

/// A neutral confirmation: a badge, the question, Cancel and the action.
fn confirm(
    ctx: &egui::Context,
    id: &'static str,
    icon: Icon,
    title: &str,
    message: &str,
    action: &str,
) -> Option<Outcome> {
    let mut outcome = None;
    sheet::show(ctx, Id::new(id), DIALOG_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(ui, icon, title, message, theme::ACCENT);
            ui.add_space(22.0);
        });
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, action, true, true).clicked() {
                outcome = Some(Outcome::Confirm);
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Some(Outcome::Cancel);
            }
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(Outcome::Cancel);
    } else if outcome.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        // Return answers a macOS alert with its default button.
        outcome = Some(Outcome::Confirm);
    }
    outcome
}

pub(super) fn text_field(
    ui: &mut Ui,
    id: Id,
    icon: Icon,
    value: &mut String,
    hint: &str,
    focus: &mut bool,
    limit: usize,
) {
    let focused = ui.memory(|memory| memory.has_focus(id));
    kit::field_frame(focused).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icons::inline(ui, icon, 13.0, theme::text_secondary());
            let width = ui.available_width();
            let response = kit::plain_field(ui, id, value, hint, size::BODY, REGULAR, width);
            if response.changed() && value.chars().count() > limit {
                *value = value.chars().take(limit).collect();
            }
            if std::mem::take(focus) {
                response.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(value.chars().count()),
                        )));
                    state.store(ui.ctx(), id);
                }
            }
        });
    });
}

fn save_dialog(
    ctx: &egui::Context,
    name: &mut String,
    description: &mut String,
    focus: &mut bool,
    error: Option<&str>,
) -> Option<Outcome> {
    let mut outcome = None;
    let valid = clean_name(name).is_ok();
    sheet::show(ctx, Id::new("chart-template-save"), DIALOG_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Palette,
                "Save as Template",
                "Saves this chart's type, colors and canvas, so any chart can wear them. Nothing about its market, indicators or drawings is saved.",
                theme::ACCENT,
            );
            ui.add_space(20.0);
            text_field(
                ui,
                Id::new("chart-template-name"),
                Icon::Tag,
                name,
                "Template name",
                focus,
                MAX_NAME_CHARS,
            );
            ui.add_space(8.0);
            let mut no_focus = false;
            text_field(
                ui,
                Id::new("chart-template-description"),
                Icon::DocText,
                description,
                "Description (optional)",
                &mut no_focus,
                MAX_DESCRIPTION_CHARS,
            );
            if let Some(error) = error {
                ui.add_space(8.0);
                ui.label(kit::text(error, size::SUBHEADLINE, REGULAR, kit::ORANGE));
            }
            ui.add_space(22.0);
        });
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, "Save", true, valid).clicked() {
                outcome = Some(Outcome::Confirm);
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Some(Outcome::Cancel);
            }
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(Outcome::Cancel);
    } else if outcome.is_none() && valid && ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        outcome = Some(Outcome::Confirm);
    }
    outcome
}

// MARK: Manage

enum ManageOutcome {
    Close,
    Stay,
    Open(Dialog),
}

/// A small square icon button with a tooltip, always drawn.
fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str, enabled: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        vec2(26.0, 26.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = super::accessibility::label(response, tooltip).on_hover_text(tooltip);
    if enabled && response.hovered() {
        ui.painter().rect_filled(rect, 6.0, theme::ink(26));
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(13.0, 13.0)),
        icon,
        if enabled {
            theme::text_secondary()
        } else {
            theme::text_tertiary()
        },
    );
    enabled
        && response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
}

fn manage_dialog(
    ctx: &egui::Context,
    state: &mut TemplateState,
    store: &SharedStore,
    card: &mut ChartCard,
    manage: &mut Manage,
) -> ManageOutcome {
    let entries = state.templates(store).book().entries();
    let load_error = state.templates(store).load_error().map(str::to_owned);
    let scope = state.scope;
    let mut outcome = ManageOutcome::Stay;
    let mut apply: Option<TemplateEntry> = None;
    let mut duplicate: Option<TemplateId> = None;
    let mut rename_commit: Option<Renaming> = None;
    sheet::show(ctx, Id::new("chart-template-manage"), MANAGE_WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Sliders,
                "Manage Templates",
                "Saved looks for charts. Presets are fixed; duplicate one to make it your own.",
                theme::ACCENT,
            );
            ui.add_space(16.0);
            if let Some(error) = load_error.as_deref() {
                ui.label(kit::text(
                    format!("Saved templates could not be read ({error}). They have been left as they are; presets still work."),
                    size::SUBHEADLINE,
                    REGULAR,
                    kit::ORANGE,
                ));
                ui.add_space(8.0);
            }
            if let Some(error) = &manage.error {
                ui.label(kit::text(error, size::SUBHEADLINE, REGULAR, kit::ORANGE));
                ui.add_space(8.0);
            }
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for entry in &entries {
                        template_row(
                            ui,
                            entry,
                            manage,
                            &mut apply,
                            &mut duplicate,
                            &mut rename_commit,
                            &mut outcome,
                        );
                    }
                });
            ui.add_space(14.0);
            ui.label(kit::text(
                format!("{} · {}", scope.label(), scope.hint()),
                size::CAPTION + 0.5,
                REGULAR,
                theme::text_tertiary(),
            ));
            ui.add_space(14.0);
        });
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, "Done", true, true).clicked() {
                outcome = ManageOutcome::Close;
            }
        });
    });
    if let Some(entry) = apply {
        card.apply_template(&entry.appearance, scope);
        state.say(format!("Applied “{}”.", entry.name));
    }
    if let Some(id) = duplicate {
        match state.templates(store).duplicate(id) {
            Ok(_) => manage.error = None,
            Err(message) => manage.error = Some(message),
        }
    }
    if let Some(done) = rename_commit {
        match state
            .templates(store)
            .rename(done.id, &done.name, &done.description)
        {
            Ok(()) => {
                manage.renaming = None;
                manage.error = None;
            }
            Err(message) => manage.error = Some(message),
        }
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        if manage.renaming.is_some() {
            manage.renaming = None;
        } else {
            outcome = ManageOutcome::Close;
        }
    }
    outcome
}

fn template_row(
    ui: &mut Ui,
    entry: &TemplateEntry,
    manage: &mut Manage,
    apply: &mut Option<TemplateEntry>,
    duplicate: &mut Option<TemplateId>,
    rename_commit: &mut Option<Renaming>,
    outcome: &mut ManageOutcome,
) {
    let width = ui.available_width();
    kit::card_frame(false)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(width - 24.0 - 2.0);
            let renaming = manage
                .renaming
                .as_mut()
                .filter(|renaming| renaming.id == entry.id);
            if let Some(renaming) = renaming {
                text_field(
                    ui,
                    Id::new(("template-rename", entry.id)),
                    Icon::Tag,
                    &mut renaming.name,
                    "Template name",
                    &mut renaming.focus,
                    MAX_NAME_CHARS,
                );
                ui.add_space(6.0);
                let mut no_focus = false;
                text_field(
                    ui,
                    Id::new(("template-rename-description", entry.id)),
                    Icon::DocText,
                    &mut renaming.description,
                    "Description (optional)",
                    &mut no_focus,
                    MAX_DESCRIPTION_CHARS,
                );
                ui.add_space(8.0);
                let candidate = renaming.clone();
                let valid = clean_name(&candidate.name).is_ok();
                let mut cancelled = false;
                ui.horizontal(|ui| {
                    if kit::push_button(ui, None, "Save", valid).clicked()
                        || (valid && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                    {
                        *rename_commit = Some(candidate.clone());
                    }
                    cancelled = kit::push_button(ui, None, "Cancel", true).clicked();
                });
                if cancelled {
                    manage.renaming = None;
                }
                return;
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 190.0).max(120.0));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add(
                            egui::Label::new(kit::text(
                                &entry.name,
                                size::BODY,
                                SEMIBOLD,
                                theme::text_primary(),
                            ))
                            .truncate(),
                        );
                        if entry.built_in {
                            kit::status_badge(ui, "Built-in", kit::Tone::Neutral);
                        }
                    });
                    let detail = match (&entry.description, entry.built_in) {
                        (Some(text), _) => text.clone(),
                        (None, _) => String::new(),
                    };
                    if !detail.is_empty() {
                        ui.add(
                            egui::Label::new(kit::text(
                                detail,
                                size::CAPTION + 0.5,
                                REGULAR,
                                theme::text_secondary(),
                            ))
                            .truncate(),
                        );
                    }
                    if !entry.built_in {
                        let when = if entry.updated_at > entry.created_at {
                            format!("Updated {}", crate::format::timestamp(entry.updated_at))
                        } else {
                            format!("Created {}", crate::format::timestamp(entry.created_at))
                        };
                        ui.label(kit::text(
                            when,
                            size::CAPTION,
                            REGULAR,
                            theme::text_tertiary(),
                        ));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let saved = !entry.built_in;
                    if icon_button(ui, Icon::Trash, "Delete template", saved) {
                        *outcome = ManageOutcome::Open(Dialog::Delete(entry.id));
                    }
                    if icon_button(ui, Icon::Download, "Update from this chart", saved) {
                        *outcome = ManageOutcome::Open(Dialog::Update(entry.id));
                    }
                    if icon_button(ui, Icon::Pencil, "Rename template", saved) {
                        manage.renaming = Some(Renaming {
                            id: entry.id,
                            name: entry.name.clone(),
                            description: entry.description.clone().unwrap_or_default(),
                            focus: true,
                        });
                    }
                    if icon_button(ui, Icon::CopyDoc, "Duplicate template", true) {
                        *duplicate = Some(entry.id);
                    }
                    if icon_button(ui, Icon::Check, "Apply to this chart", true) {
                        *apply = Some(entry.clone());
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests;
