//! The assistant's panel under the editor: the conversation, the change it proposes as a diff, and
//! the box to ask for the next one. A view only: it reads the [`Assistant`] and reports what was
//! pressed; `ai_host` acts on it.

use super::ai::{Assistant, Notice, Proposal};
use super::ai_prompt::{self, Mode, Turn};
use super::diff_view;
use crate::ui::icons::{self, Icon};
use crate::ui::markdown_view;
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align, Color32, FontId, Id, Key, Layout, Margin, Modifiers, Sense, Ui, vec2,
};

const HEADER_HEIGHT: f32 = 34.0;
const INPUT_HEIGHT: f32 = 62.0;
const FOOTER_HEIGHT: f32 = 66.0;

/// Starting points for an empty conversation, for each mode.
const PLAN_EXAMPLES: [&str; 3] = [
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    let bottom = INPUT_HEIGHT + FOOTER_HEIGHT;
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
    input_box(&mut input_ui, ai, setup, &mut actions);
    actions
}

fn header_bar(ui: &mut Ui, ai: &Assistant, setup: &Setup, actions: &mut Vec<PanelAction>) {
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(12.0);
        icons::inline(ui, Icon::Sparkles, 13.0, kit::ACCENT);
        ui.label(kit::text(
            "Assistant",
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.label(kit::text(
            format!("{} · {}", setup.provider, setup.model),
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
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
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add_space(8.0);
            if let Some(problem) = &setup.problem
                && setup_card(ui, problem)
            {
                actions.push(PanelAction::OpenSettings);
            }
            if ai.turns.is_empty() && !ai.is_running() && ai.proposal.is_none() {
                empty_state(ui, ai, script_name);
            }
            let last = ai.turns.len().saturating_sub(1);
            let can_implement = ai.can_implement();
            for (index, turn) in ai.turns.iter().enumerate() {
                match turn {
                    Turn::User(text) => user_message(ui, text),
                    Turn::Assistant { text, mode } => {
                        let id = Id::new(("ai-turn", index));
                        assistant_message(ui, id, text, *mode, true);
                        if can_implement && index == last && implement_button(ui) {
                            actions.push(PanelAction::Implement);
                        }
                    }
                }
            }
            if let Some(request) = ai.pending_request() {
                user_message(ui, request);
            }
            if ai.is_running() {
                assistant_message(ui, Id::new("ai-streaming"), &ai.streaming, ai.mode, false);
            }
            if let Some(notice) = &ai.notice {
                notice_row(ui, notice);
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
        Mode::Plan => (
            format!("Ask about {script_name}, or plan a change. Nothing is edited in Plan mode."),
            PLAN_EXAMPLES,
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
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for example in examples {
            if kit::value_chip(ui, example, false).clicked() {
                ai.input = example.to_owned();
                ai.focus_input = true;
            }
        }
    });
}

fn caption(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        theme::text_tertiary(),
    ));
}

fn user_message(ui: &mut Ui, text: &str) {
    caption(ui, "You");
    egui::Frame::new()
        .fill(theme::with_alpha(kit::ACCENT, 0.12))
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
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

/// A reply. A Plan reply is shown whole, code blocks and all. An Edit reply's script is not shown
/// in the transcript (the proposal below it is where it is read), only a line saying it was written.
fn assistant_message(ui: &mut Ui, id: Id, text: &str, mode: Mode, done: bool) {
    caption(ui, "Assistant");
    if text.trim().is_empty() {
        thinking(ui);
        return;
    }
    if mode == Mode::Plan {
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

fn notice_row(ui: &mut Ui, notice: &Notice) {
    let (icon, color) = if notice.error {
        (Icon::TriangleFill, kit::RED)
    } else {
        (Icon::InfoCircle, theme::text_secondary())
    };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icons::inline(ui, icon, 12.0, color);
        ui.add(
            egui::Label::new(kit::text(
                &notice.text,
                kit::size::SUBHEADLINE + 0.5,
                kit::REGULAR,
                color,
            ))
            .selectable(true),
        );
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

fn input_box(ui: &mut Ui, ai: &mut Assistant, setup: &Setup, actions: &mut Vec<PanelAction>) {
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
    egui::Frame::new()
        .fill(theme::with_alpha(theme::editor_background(), 0.7))
        .corner_radius(8.0)
        .stroke(egui::Stroke::new(
            1.0,
            if ui.memory(|m| m.has_focus(id)) {
                theme::with_alpha(kit::ACCENT, 0.7)
            } else {
                kit::primary(0.15)
            },
        ))
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(INPUT_HEIGHT - 20.0);
            egui::ScrollArea::vertical()
                .id_salt("ai-input-scroll")
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
                                    Mode::Plan => "Ask a question or plan a change. Enter to send.",
                                    Mode::Edit => "Describe a change. Enter to send, Shift-Enter for a new line.",
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
        });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // The mode cannot change while a request runs or a change waits for an answer.
        let locked = ai.is_running() || ai.proposal.is_some();
        let mut mode = ai.mode;
        ui.add_enabled_ui(!locked, |ui| {
            kit::segmented(
                ui,
                Id::new("ai-mode"),
                &[
                    (Mode::Plan, Mode::Plan.title()),
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
            } else if kit::sheet_button(ui, "Send", true, can_send).clicked() {
                actions.push(PanelAction::Send);
            }
        });
    });
    ui.add(
        egui::Label::new(kit::text(
            match ai.mode {
                Mode::Plan => format!(
                    "Plan mode never changes your script. Sent to {}.",
                    setup.provider
                ),
                Mode::Edit => format!("Sent to {}: this script and your message.", setup.provider),
            },
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
        ))
        .truncate(),
    );
}
