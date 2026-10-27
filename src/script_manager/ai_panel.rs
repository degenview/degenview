//! The assistant's panel under the editor: the conversation, the change it proposes as a diff, and
//! the box to ask for the next one. A view only: it reads the [`Assistant`] and reports what was
//! pressed; `ai_host` acts on it.

use super::ai::{Assistant, Notice, Proposal};
use super::ai_models::{self, ModelList};
use super::ai_prompt::{self, Mode, Turn};
use super::diff_view;
use crate::service::llm::ProviderKind;
use crate::ui::error_notice::{self, ErrorEvent, ErrorNotice};
use crate::ui::icons::{self, Icon};
use crate::ui::markdown_view;
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Key, Layout, Margin, Modifiers, Rect, Sense, Ui,
    pos2, vec2,
};
use std::time::{Duration, Instant};

const HEADER_HEIGHT: f32 = 38.0;
/// The composer (the box and its controls) and the line under it.
const BOTTOM_HEIGHT: f32 = 128.0;
/// The model menu's width. A text field inside a menu must not ask for more, or the menu grows to
/// fill the window.
const MENU_WIDTH: f32 = 300.0;
/// Space between the menu's edge and its fields and cards.
const MENU_INSET: f32 = 8.0;
/// Where a menu's text starts: the headings' and rows' own inset.
const MENU_TEXT_INSET: f32 = 14.0;
/// How long a Copy button says "Copied".
const COPIED_FOR: Duration = Duration::from_millis(1400);

/// Starting points for an empty conversation, for each mode.
const ASK_EXAMPLES: [&str; 3] = [
    "Explain what this script does",
    "What could go wrong with this logic?",
    "How would I add an alert to this?",
];
const EDIT_EXAMPLES: [&str; 3] = [
    "Add an input for the length",
    "Plot a marker when the signal crosses",
    "Tidy up the names and add comments",
];

/// What the panel needs to know about the provider, worked out by the host.
pub struct Setup {
    pub provider: String,
    pub model: String,
    /// Why a request cannot be made yet ("Add your OpenAI API key"), if it cannot.
    pub problem: Option<String>,
    /// What a request sends, in a line: "Bollinger Squeeze · 42 lines · No problems".
    pub context: String,
    /// Each provider, and whether it can be used now (it has a key, or needs none).
    pub providers: Vec<(ProviderKind, bool)>,
    pub current: ProviderKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PanelAction {
    Send,
    Stop,
    Accept,
    Discard,
    Clear,
    OpenSettings,
    Close,
    SetMode(Mode),
    /// Carry out the plan in the last reply: switch to Edit and ask for it.
    Implement,
    /// Open the review sheet over the proposed change.
    Review,
    /// The model menu opened: fetch the current provider's models.
    LoadModels,
    SetModel(String),
    SetProvider(ProviderKind),
    /// Ask the last message again.
    Retry,
    /// Put the last message back in the box to change it.
    EditLast,
    DismissNotice,
}

pub fn panel_button(ui: &mut Ui, panel_open: bool) -> bool {
    let tip = if panel_open {
        "Hide the assistant"
    } else {
        "Ask an AI assistant to change this script"
    };
    let color = if panel_open {
        kit::ACCENT
    } else {
        theme::text_primary()
    };
    icons::button(ui, Icon::Sparkles, 28.0, color, tip).clicked()
}

pub fn show(ui: &mut Ui, ai: &mut Assistant, setup: &Setup, script_name: &str) -> Vec<PanelAction> {
    let mut actions = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    ui.painter().rect_filled(full, 0.0, theme::background());
    let header = egui::Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
    let bottom = BOTTOM_HEIGHT;
    let body = egui::Rect::from_min_max(
        header.left_bottom(),
        egui::pos2(full.right(), (full.bottom() - bottom).max(header.bottom())),
    );
    let input = egui::Rect::from_min_max(body.left_bottom(), full.right_bottom());

    header_bar(
        &mut ui.new_child(egui::UiBuilder::new().max_rect(header)),
        ai,
        setup,
        &mut actions,
    );
    ui.painter().hline(
        full.x_range(),
        header.bottom(),
        egui::Stroke::new(1.0, kit::hairline()),
    );

    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink2(vec2(12.0, 0.0))));
    body_ui.set_clip_rect(body.intersect(ui.clip_rect()));
    transcript(&mut body_ui, ai, setup, script_name, &mut actions);

    let mut input_ui =
        ui.new_child(egui::UiBuilder::new().max_rect(input.shrink2(vec2(12.0, 6.0))));
    composer(&mut input_ui, ai, setup, &mut actions);
    actions
}

fn header_bar(ui: &mut Ui, ai: &mut Assistant, setup: &Setup, actions: &mut Vec<PanelAction>) {
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(12.0);
        icons::inline(ui, Icon::Sparkles, 13.0, kit::ACCENT);
        ui.label(kit::text(
            "Assistant",
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.add_space(2.0);
        let chip = model_chip(ui, &format!("{} · {}", setup.provider, setup.model));
        if chip.clicked() {
            actions.push(PanelAction::LoadModels);
        }
        kit::menu_popup(&chip)
            .width(MENU_WIDTH)
            .show(|ui| model_menu(ui, ai, setup, actions));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(6.0);
            if icons::button(
                ui,
                Icon::Xmark,
                24.0,
                theme::text_secondary(),
                "Hide the assistant",
            )
            .clicked()
            {
                actions.push(PanelAction::Close);
            }
            if !ai.turns.is_empty()
                && !ai.is_running()
                && icons::button(ui, Icon::Trash, 24.0, theme::text_secondary(), "Start over")
                    .clicked()
            {
                actions.push(PanelAction::Clear);
            }
        });
    });
}

/// The provider and model, as a pill that opens the model menu.
fn model_chip(ui: &mut Ui, label: &str) -> egui::Response {
    let galley = kit::galley(
        ui,
        label,
        kit::size::SUBHEADLINE,
        kit::MEDIUM,
        theme::text_secondary(),
        240.0,
    );
    let size = vec2(galley.size().x + 30.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    // Where the chip is, for the gallery to click it.
    ui.data_mut(|data| data.insert_temp(Id::new("ai-model-chip"), rect));
    let response = response
        .on_hover_text("Choose the model")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        11.0,
        theme::ink(if response.hovered() { 16 } else { 9 }),
    );
    painter.galley(
        pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_secondary(),
    );
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.right() - 11.0, rect.center().y), vec2(9.0, 9.0)),
        Icon::ChevronDown,
        theme::text_tertiary(),
    );
    response
}

/// The menu behind the chip: which provider, and which of its models.
fn model_menu(ui: &mut Ui, ai: &mut Assistant, setup: &Setup, actions: &mut Vec<PanelAction>) {
    kit::menu_scope(ui, MENU_WIDTH);
    // However long a provider's error is, the menu is this wide and no wider.
    ui.set_max_width(MENU_WIDTH);
    let inner = MENU_WIDTH - 2.0 * MENU_INSET;
    kit::menu_heading(ui, "Provider");
    for (kind, ready) in &setup.providers {
        let mut row = kit::MenuRow::new(kind.title());
        row.current = *kind == setup.current;
        row.reserve_icon = false;
        row.shortcut = (!ready).then_some("needs a key");
        if kit::menu_row(ui, row).clicked() {
            actions.push(if *ready {
                PanelAction::SetProvider(*kind)
            } else {
                PanelAction::OpenSettings
            });
            ui.close();
        }
    }
    kit::menu_separator(ui);
    kit::menu_heading(ui, "Model");
    egui::Frame::new()
        .inner_margin(Margin::symmetric(MENU_INSET as i8, 4))
        .show(ui, |ui| {
            let field = kit::search_field(
                ui,
                Id::new("ai-model-filter"),
                &mut ai.model_filter,
                "Filter, or type a model name",
                inner,
            );
            let typed = ai.model_filter.trim().to_owned();
            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !typed.is_empty() {
                actions.push(PanelAction::SetModel(typed));
                ai.model_filter.clear();
                ui.close();
            }
        });
    let note = |ui: &mut Ui, text: &str, color: Color32| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(MENU_TEXT_INSET);
            ui.add(
                egui::Label::new(kit::text(text, kit::size::SUBHEADLINE, kit::REGULAR, color))
                    .wrap(),
            );
        });
        ui.add_space(4.0);
    };
    let typed = ai.model_filter.trim().to_owned();
    let mut listed = false;
    match ai.models.get(&setup.current).cloned().unwrap_or_default() {
        ModelList::Unloaded | ModelList::Loading => {
            ui.horizontal(|ui| {
                ui.add_space(MENU_TEXT_INSET);
                ui.add(
                    egui::Spinner::new()
                        .size(12.0)
                        .color(theme::text_secondary()),
                );
                ui.label(kit::text(
                    "  Loading models…",
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            });
            ui.add_space(6.0);
        }
        ModelList::Failed(message) => {
            ui.add_space(4.0);
            egui::Frame::new()
                .inner_margin(Margin::symmetric(MENU_INSET as i8, 0))
                .show(ui, |ui| {
                    ui.set_width(inner);
                    let notice = ErrorNotice::new("models", &message).retry("Try again");
                    if error_notice::show(ui, &notice) == Some(ErrorEvent::Retry) {
                        actions.push(PanelAction::LoadModels);
                    }
                });
            ui.add_space(6.0);
        }
        ModelList::Ready(ids) => {
            listed = ids.contains(&typed);
            let shown = ai_models::matching(&ids, &ai.model_filter);
            if shown.is_empty() {
                note(ui, "No model matches.", theme::text_tertiary());
            }
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .auto_shrink([true, true])
                .show(ui, |ui| {
                    for id in shown {
                        let mut row = kit::MenuRow::new(id);
                        row.current = *id == setup.model;
                        row.reserve_icon = false;
                        if kit::menu_row(ui, row).clicked() {
                            actions.push(PanelAction::SetModel(id.clone()));
                            ui.close();
                        }
                    }
                });
        }
    }
    // A name the provider's list does not show: a new model, or a compatible server's own.
    if !typed.is_empty() && !listed {
        let label = format!("Use \"{typed}\"");
        let mut row = kit::MenuRow::new(&label);
        row.reserve_icon = false;
        if kit::menu_row(ui, row).clicked() {
            actions.push(PanelAction::SetModel(typed));
            ai.model_filter.clear();
            ui.close();
        }
    }
}
fn transcript(
    ui: &mut Ui,
    ai: &mut Assistant,
    setup: &Setup,
    script_name: &str,
    actions: &mut Vec<PanelAction>,
) {
    egui::ScrollArea::vertical()
        .id_salt("ai-transcript")
        .auto_shrink([false, false])
        // Follows a growing reply, but a card or hint on an empty panel stays at the top.
        .stick_to_bottom(!ai.turns.is_empty() || ai.is_running())
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.add_space(8.0);
            if let Some(problem) = &setup.problem
                && setup_card(ui, problem)
            {
                actions.push(PanelAction::OpenSettings);
            }
            if ai.turns.is_empty() && !ai.is_running() && ai.proposal.is_none() {
                empty_state(ui, ai, script_name);
            }
            let count = ai.turns.len();
            let busy = ai.is_running();
            let can_implement = ai.can_implement();
            for (index, turn) in ai.turns.iter().enumerate() {
                match turn {
                    Turn::User(text) => {
                        // The last message can be changed and sent again once it has an answer.
                        let editable = !busy && index + 2 == count;
                        if user_message(ui, text, editable) {
                            actions.push(PanelAction::EditLast);
                        }
                    }
                    Turn::Assistant { text, mode } => {
                        let id = Id::new(("ai-turn", index));
                        assistant_message(ui, id, text, *mode, true);
                        if index + 1 == count && !busy {
                            if reply_actions(ui, id, text) {
                                actions.push(PanelAction::Retry);
                            }
                            if can_implement && implement_button(ui) {
                                actions.push(PanelAction::Implement);
                            }
                        }
                    }
                }
            }
            if let Some(request) = ai.pending_request() {
                user_message(ui, request, false);
            }
            if ai.is_running() {
                assistant_message(ui, Id::new("ai-streaming"), &ai.streaming, ai.mode, false);
            }
            if let Some(notice) = &ai.notice {
                notice_banner(ui, notice, !ai.input.trim().is_empty(), actions);
            }
            if let Some(proposal) = &ai.proposal {
                proposal_card(ui, proposal, actions);
            }
            ui.add_space(8.0);
            if std::mem::take(&mut ai.scroll_to_end) {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
        });
}
/// What to do before the first request, with the button under the text so it never crowds it.
fn setup_card(ui: &mut Ui, problem: &str) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(theme::with_alpha(kit::ACCENT, 0.08))
        .corner_radius(10.0)
        .stroke(egui::Stroke::new(1.0, theme::with_alpha(kit::ACCENT, 0.25)))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::Key, 14.0, kit::ACCENT);
                ui.label(kit::text(
                    problem,
                    kit::size::SUBHEADLINE + 1.0,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
            });
            ui.add(
                egui::Label::new(kit::text(
                    "The key stays in this Mac's Keychain. The script is sent to the provider you pick.",
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ))
                .wrap(),
            );
            ui.add_space(4.0);
            clicked = kit::sheet_button(ui, "Open Settings", true, true).clicked();
        });
    clicked
}

fn empty_state(ui: &mut Ui, ai: &mut Assistant, script_name: &str) {
    ui.add_space(6.0);
    let (line, examples) = match ai.mode {
        Mode::Ask => (
            format!("Ask about {script_name}, or plan a change. Nothing is edited in Ask mode."),
            ASK_EXAMPLES,
        ),
        Mode::Edit => (
            format!("Describe a change to {script_name}."),
            EDIT_EXAMPLES,
        ),
    };
    ui.add(
        egui::Label::new(kit::text(
            line,
            kit::size::BODY,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
    ui.spacing_mut().item_spacing.y = 6.0;
    for example in examples {
        if suggestion(ui, example) {
            ai.input = example.to_owned();
            ai.focus_input = true;
        }
    }
}

/// One suggested question, as a full-width row.
fn suggestion(ui: &mut Ui, text: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let painter = ui.painter();
    painter.rect(
        rect,
        8.0,
        theme::ink(if response.hovered() { 14 } else { 7 }),
        egui::Stroke::new(1.0, kit::hairline()),
        egui::StrokeKind::Inside,
    );
    painter.text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(kit::size::BODY),
        theme::text_primary(),
    );
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(10.0, 10.0)),
        Icon::ChevronRight,
        theme::text_tertiary(),
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}
/// What the user sent, as a bubble on the right. Returns whether Edit was pressed.
fn user_message(ui: &mut Ui, text: &str, editable: bool) -> bool {
    let mut edit = false;
    ui.with_layout(Layout::top_down(Align::Max), |ui| {
        let widest = ui.available_width() * 0.9;
        egui::Frame::new()
            .fill(theme::with_alpha(kit::ACCENT, 0.14))
            .corner_radius(12.0)
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_max_width((widest - 24.0).max(40.0));
                ui.add(
                    egui::Label::new(kit::text(
                        text,
                        kit::size::BODY,
                        kit::REGULAR,
                        theme::text_primary(),
                    ))
                    .selectable(true),
                );
            });
        if editable
            && icons::button(
                ui,
                Icon::Pencil,
                20.0,
                theme::text_tertiary(),
                "Edit and send again",
            )
            .clicked()
        {
            edit = true;
        }
    });
    edit
}

/// Copy and ask-again under the last reply. Returns whether ask-again was pressed.
fn reply_actions(ui: &mut Ui, id: Id, text: &str) -> bool {
    let copied_id = id.with("reply-copied");
    let copied: Option<Instant> = ui.data(|data| data.get_temp(copied_id));
    let just_copied = copied.is_some_and(|at| at.elapsed() < COPIED_FOR);
    let mut again = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let (icon, tip, color) = if just_copied {
            (Icon::Check, "Copied", kit::GREEN)
        } else {
            (Icon::CopyDoc, "Copy reply", theme::text_tertiary())
        };
        if icons::button(ui, icon, 22.0, color, tip).clicked() {
            ui.ctx().copy_text(text.to_owned());
            ui.data_mut(|data| data.insert_temp(copied_id, Instant::now()));
            ui.ctx().request_repaint_after(COPIED_FOR);
        }
        if icons::button(ui, Icon::Repeat, 22.0, theme::text_tertiary(), "Ask again").clicked() {
            again = true;
        }
    });
    again
}
fn thinking(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.add(
            egui::Spinner::new()
                .size(12.0)
                .color(theme::text_secondary()),
        );
        ui.label(kit::text(
            "Thinking…",
            kit::size::BODY,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    });
}

/// A reply. An Ask reply is shown whole, code blocks and all. An Edit reply's script is not shown
/// in the transcript (the proposal below it is where it is read), only a line saying it was written.
fn assistant_message(ui: &mut Ui, id: Id, text: &str, mode: Mode, done: bool) {
    if text.trim().is_empty() {
        thinking(ui);
        return;
    }
    if mode == Mode::Ask {
        markdown_view::show(ui, id, &markdown_view::parse(text));
        return;
    }
    let reply = ai_prompt::split_reply(text);
    if reply.prose.is_empty() && reply.script.is_none() {
        thinking(ui);
        return;
    }
    if !reply.prose.is_empty() {
        markdown_view::show(ui, id, &markdown_view::parse(&reply.prose));
    }
    if let Some(script) = &reply.script {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if reply.complete || done {
                icons::inline(ui, Icon::Code, 12.0, theme::text_secondary());
                let lines = script.lines().count();
                let noun = if lines == 1 { "line" } else { "lines" };
                ui.label(kit::text(
                    format!("Wrote a script · {lines} {noun}"),
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            } else {
                ui.add(
                    egui::Spinner::new()
                        .size(12.0)
                        .color(theme::text_secondary()),
                );
                ui.label(kit::text(
                    format!(
                        "Writing the script… {} lines so far",
                        script.lines().count()
                    ),
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            }
        });
    }
}

fn implement_button(ui: &mut Ui) -> bool {
    ui.add_space(2.0);
    kit::push_button(ui, Some(Icon::PlayFill), "Implement this plan", true)
        .on_hover_text("Switches to Edit mode and asks for these changes.")
        .clicked()
}

/// A message about the last request: an error (with Try again) or a note. Dismissable.
fn notice_banner(ui: &mut Ui, notice: &Notice, can_retry: bool, actions: &mut Vec<PanelAction>) {
    if notice.error {
        let mut card = ErrorNotice::new("panel", &notice.text).dismissable();
        if can_retry {
            card = card.retry("Try again");
        }
        match error_notice::show(ui, &card) {
            Some(ErrorEvent::Retry) => actions.push(PanelAction::Send),
            Some(ErrorEvent::Dismiss) => actions.push(PanelAction::DismissNotice),
            None => {}
        }
        return;
    }
    let tint = theme::text_secondary();
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.1))
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::InfoCircle, 13.0, tint);
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 30.0).max(40.0));
                    ui.add(
                        egui::Label::new(kit::text(
                            &notice.text,
                            kit::size::SUBHEADLINE + 0.5,
                            kit::REGULAR,
                            theme::text_primary(),
                        ))
                        .selectable(true),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    if icons::button(ui, Icon::Xmark, 18.0, theme::text_tertiary(), "Dismiss")
                        .clicked()
                    {
                        actions.push(PanelAction::DismissNotice);
                    }
                });
            });
        });
}
fn proposal_card(ui: &mut Ui, proposal: &Proposal, actions: &mut Vec<PanelAction>) {
    let hunks = proposal.changes.hunks();
    let added = proposal.changes.lines_added();
    let removed = proposal.changes.lines_removed();
    egui::Frame::new()
        .fill(theme::with_alpha(theme::text_primary(), 0.04))
        .stroke(egui::Stroke::new(1.0, kit::hairline()))
        .corner_radius(10.0)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(kit::text(
                    "Proposed change",
                    kit::size::BODY,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                let (text, color) = compile_summary(proposal);
                ui.label(kit::text(text, kit::size::SUBHEADLINE, kit::MEDIUM, color));
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let noun = if hunks.len() == 1 {
                    "change"
                } else {
                    "changes"
                };
                ui.label(kit::text(
                    format!("{} {noun}", hunks.len()),
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.label(kit::text(
                    format!("+{added}"),
                    kit::size::SUBHEADLINE,
                    kit::MEDIUM,
                    kit::GREEN,
                ));
                ui.label(kit::text(
                    format!("−{removed}"),
                    kit::size::SUBHEADLINE,
                    kit::MEDIUM,
                    kit::RED,
                ));
            });
            // A glance at what changes; the sheet is where it is read.
            for hunk in hunks.iter().take(3) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let color = match hunk.kind {
                        super::diff::ChangeKind::Added => theme::editor_diff_added(),
                        super::diff::ChangeKind::Deleted => theme::editor_diff_deleted(),
                        super::diff::ChangeKind::Modified => theme::editor_diff_modified(),
                    };
                    let (dot, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.0, color);
                    ui.label(kit::text(
                        diff_view::title(hunk),
                        kit::size::SUBHEADLINE,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                });
            }
            if hunks.len() > 3 {
                ui.label(kit::text(
                    format!("and {} more", hunks.len() - 3),
                    kit::size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_tertiary(),
                ));
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                if kit::sheet_button(ui, "Review changes", true, true).clicked() {
                    actions.push(PanelAction::Review);
                }
                let label = if proposal.errors > 0 {
                    "Apply anyway"
                } else {
                    "Apply all"
                };
                if kit::sheet_button(ui, label, false, true)
                    .on_hover_text("Applies every change as one undo step")
                    .clicked()
                {
                    actions.push(PanelAction::Accept);
                }
                if kit::sheet_button(ui, "Discard", false, true).clicked() {
                    actions.push(PanelAction::Discard);
                }
            });
        });
}

fn compile_summary(proposal: &Proposal) -> (String, Color32) {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    if proposal.errors > 0 {
        (plural(proposal.errors, "compile error"), kit::RED)
    } else if proposal.warnings > 0 {
        (
            format!("Compiles · {}", plural(proposal.warnings, "warning")),
            Color32::from_rgb(255, 159, 10),
        )
    } else {
        ("Compiles".to_owned(), kit::GREEN)
    }
}

fn composer(ui: &mut Ui, ai: &mut Assistant, setup: &Setup, actions: &mut Vec<PanelAction>) {
    let id = Id::new("ai-input");
    let can_send = !ai.is_running() && setup.problem.is_none() && !ai.input.trim().is_empty();
    // Enter sends; Shift-Enter adds a line. Taken before the field sees it, and not while an
    // input method is composing text, where Enter confirms the composition instead.
    let composing = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Ime(_))));
    let enter = !composing
        && ui.memory(|m| m.has_focus(id))
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    if enter && can_send {
        actions.push(PanelAction::Send);
    }
    let focused = ui.memory(|m| m.has_focus(id));
    egui::Frame::new()
        .fill(theme::with_alpha(theme::editor_background(), 0.7))
        .corner_radius(12.0)
        .stroke(egui::Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(kit::ACCENT, 0.7)
            } else {
                kit::primary(0.15)
            },
        ))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("ai-input-scroll")
                .max_height(44.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let response = ui.add_enabled(
                        !ai.is_running(),
                        egui::TextEdit::multiline(&mut ai.input)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .font(FontId::proportional(kit::size::BODY))
                            .text_color(theme::text_primary())
                            .hint_text(kit::text(
                                match ai.mode {
                                    Mode::Ask => "Ask a question or plan a change",
                                    Mode::Edit => "Describe a change",
                                },
                                kit::size::BODY,
                                kit::REGULAR,
                                theme::text_tertiary(),
                            ))
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    if ai.focus_input {
                        ai.focus_input = false;
                        response.request_focus();
                    }
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // The mode cannot change while a request runs or a change waits for an answer.
                let locked = ai.is_running() || ai.proposal.is_some();
                let mut mode = ai.mode;
                ui.add_enabled_ui(!locked, |ui| {
                    kit::segmented(
                        ui,
                        Id::new("ai-mode"),
                        &[
                            (Mode::Ask, Mode::Ask.title()),
                            (Mode::Edit, Mode::Edit.title()),
                        ],
                        &mut mode,
                    );
                })
                .response
                .on_disabled_hover_text("Answer or discard the proposed change first.");
                if mode != ai.mode {
                    actions.push(PanelAction::SetMode(mode));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ai.is_running() {
                        if kit::sheet_button(ui, "Stop", false, true).clicked() {
                            actions.push(PanelAction::Stop);
                        }
                    } else if ui
                        .add_enabled_ui(can_send, |ui| {
                            // Gray, like Stop and Discard; the dimming says there is nothing to send.
                            kit::sheet_button(ui, "Send", false, true)
                                .on_hover_text("Enter")
                                .clicked()
                        })
                        .inner
                    {
                        actions.push(PanelAction::Send);
                    }
                });
            });
        });
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(kit::text(
            // What leaves this Mac comes first: a narrow panel cuts the end of the line.
            match ai.mode {
                Mode::Ask => format!(
                    "Sent to {}  ·  Ask never edits  ·  {}",
                    setup.provider, setup.context
                ),
                Mode::Edit => format!("Sent to {}  ·  {}", setup.provider, setup.context),
            },
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
        ))
        .truncate(),
    );
}
