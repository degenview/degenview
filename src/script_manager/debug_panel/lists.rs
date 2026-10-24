//! The call stack, watches, breakpoints and console sections of the debug panel.

use eframe::egui::{self, Align, Align2, FontId, Id, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::pine::debug::{
    BreakpointState, FrameKind, Template, Variable, WatchRow, Watched, parse_expression,
};
use crate::script_manager::debug::{BreakpointEdit, DebugPanel, HitKind, PanelAction, PanelUi};
use crate::script_manager::debug_state::StoredBreakpoint;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;

use super::tree;
use super::widgets::{ROW, icon_button, mono, pill, placeholder, text_field};

// MARK: Call stack

pub fn call_stack(ui: &mut Ui, panel: &DebugPanel, actions: &mut Vec<PanelAction>) {
    let Some(shown) = panel.shown.as_ref() else {
        placeholder(
            ui,
            "No call stack",
            "It shows who called whom when the script is stopped.",
        );
        return;
    };
    for frame in &shown.snapshot.stack {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW + 2.0), Sense::click());
        let selected = frame.index == shown.frame;
        if selected {
            ui.painter()
                .rect_filled(rect, 0.0, theme::with_alpha(kit::ACCENT, 0.16));
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, theme::ink(8));
        }
        let innermost = frame.index == shown.snapshot.frame;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(11.0, 11.0)),
            match frame.kind {
                FrameKind::Security { .. } => Icon::Switch,
                _ if innermost => Icon::PlayFill,
                _ => Icon::ChevronRight,
            },
            if innermost {
                theme::editor_debug_line()
            } else {
                theme::text_tertiary()
            },
        );
        let library = matches!(frame.kind, FrameKind::Library { .. });
        let line = frame
            .location
            .map(|location| format!("line {}", location.line))
            .unwrap_or_default();
        let line_galley = kit::galley(
            ui,
            &line,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
            90.0,
        );
        let line_width = line_galley.size().x;
        ui.painter().galley(
            pos2(
                rect.right() - 10.0 - line_width,
                rect.center().y - line_galley.size().y / 2.0,
            ),
            line_galley,
            theme::text_tertiary(),
        );
        let label = kit::galley(
            ui,
            &frame.label,
            kit::size::BODY - 0.5,
            if innermost {
                kit::SEMIBOLD
            } else {
                kit::REGULAR
            },
            if library {
                theme::text_tertiary()
            } else {
                theme::text_primary()
            },
            rect.width() - 40.0 - line_width,
        );
        ui.painter().galley(
            pos2(rect.left() + 28.0, rect.center().y - label.size().y / 2.0),
            label,
            theme::text_primary(),
        );
        let response = response.on_hover_text(match frame.kind {
            FrameKind::Library { .. } => "A library function: it is stepped over, not into.",
            FrameKind::Security { .. } => {
                "The expression of a request.security() runs on its own bars, in its own state."
            }
            _ => "Show this frame's variables and watches",
        });
        if response.clicked() {
            actions.push(PanelAction::SelectFrame(frame.index));
        }
    }
}

// MARK: Watches

pub fn watches(
    ui: &mut Ui,
    panel: &DebugPanel,
    state: &mut PanelUi,
    actions: &mut Vec<PanelAction>,
) {
    let results: &[WatchRow] = panel
        .shown
        .as_ref()
        .map_or(&[], |shown| shown.watches.as_slice());
    for (index, text) in panel.state.watches.iter().enumerate() {
        if state
            .editing_watch
            .as_ref()
            .is_some_and(|(editing, _)| *editing == index)
        {
            watch_editor(ui, index, state, actions);
            continue;
        }
        let result = results.get(index).filter(|row| row.text == *text);
        watch_row(ui, panel, index, text, result, state, actions);
    }
    // The line that adds one: the field, and the button that adds what it holds (Enter does too).
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(8.0);
        let width = ui.available_width() - ADD_BUTTON - 6.0 - 8.0;
        let id = ui.id().with("add-watch");
        let response = text_field(
            ui,
            id,
            &mut state.watch_input,
            "Add expression, e.g. close - open",
            width,
            true,
        );
        let ready = !state.watch_input.trim().is_empty();
        let entered =
            response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) && ready;
        let clicked = kit::push_button(ui, None, "Add", ready)
            .on_hover_text("Watch this expression")
            .clicked();
        if (entered || clicked) && ready {
            actions.push(PanelAction::AddWatch(std::mem::take(
                &mut state.watch_input,
            )));
            response.request_focus();
        }
    });
    ui.add_space(6.0);
}

/// The width kept beside the add-expression field for its button.
const ADD_BUTTON: f32 = 50.0;

fn watch_editor(ui: &mut Ui, index: usize, state: &mut PanelUi, actions: &mut Vec<PanelAction>) {
    let Some((_, text)) = state.editing_watch.as_mut() else {
        return;
    };
    let mut finished = None;
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let width = ui.available_width() - 8.0;
        let id = ui.id().with(("edit-watch", index));
        let response = text_field(ui, id, text, "Expression", width, true);
        if !response.has_focus() && !response.lost_focus() {
            response.request_focus();
        }
        if response.lost_focus() {
            let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
            finished = Some(!escaped);
        }
    });
    if let Some(keep) = finished {
        if keep && let Some((_, text)) = state.editing_watch.take() {
            actions.push(PanelAction::EditWatch { index, text });
        } else {
            state.editing_watch = None;
        }
    }
}

/// The room at a watch row's right edge kept for its edit and remove buttons, so they sit beside
/// the type rather than over it, and the row does not shift when they appear.
const WATCH_ACTIONS: f32 = 50.0;

fn watch_row(
    ui: &mut Ui,
    panel: &DebugPanel,
    index: usize,
    text: &str,
    result: Option<&WatchRow>,
    state: &mut PanelUi,
    actions: &mut Vec<PanelAction>,
) {
    let key = format!("watch:{index}");
    let value = match result.map(|row| &row.result) {
        Some(Watched::Value(variable)) => Some(variable.clone()),
        Some(Watched::Error(message)) => {
            let mut error = Variable {
                name: text.to_owned(),
                value: message.clone(),
                full: message.clone(),
                type_name: String::new(),
                is_na: false,
                assigned: false,
                changed: false,
                declaration: None,
                expand: None,
                age: None,
            };
            error.assigned = false;
            Some(error)
        }
        None => None,
    };
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), ROW),
        if value.as_ref().is_some_and(|v| v.expand.is_some()) {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let failed = matches!(result.map(|row| &row.result), Some(Watched::Error(_)));
    match &value {
        Some(variable) => {
            let open = panel
                .shown
                .as_ref()
                .is_some_and(|shown| shown.open.contains(&key));
            let mut shown_variable = variable.clone();
            shown_variable.name = text.to_owned();
            tree::paint_variable(
                ui,
                rect,
                0,
                &shown_variable,
                open,
                response.hovered(),
                WATCH_ACTIONS,
            );
            if failed {
                // The reason is the value; draw it in the error colour over the faint one.
                let value_left = rect.left() + (rect.width() * 0.4).clamp(96.0, 240.0);
                ui.painter().rect_filled(
                    Rect::from_min_max(pos2(value_left - 2.0, rect.top()), rect.right_bottom()),
                    0.0,
                    ui.visuals().panel_fill,
                );
                ui.painter()
                    .with_clip_rect(Rect::from_min_max(
                        pos2(value_left, rect.top()),
                        pos2(rect.right() - WATCH_ACTIONS - 6.0, rect.bottom()),
                    ))
                    .text(
                        pos2(value_left, rect.center().y),
                        Align2::LEFT_CENTER,
                        &variable.value,
                        FontId::proportional(kit::size::SUBHEADLINE),
                        theme::syntax::string(),
                    );
            }
            if let Some(reference) = &variable.expand {
                if response.clicked() {
                    actions.push(PanelAction::ToggleRow {
                        key: key.clone(),
                        reference: reference.clone(),
                    });
                }
                if open
                    && panel
                        .shown
                        .as_ref()
                        .is_some_and(|shown| !shown.loaded.contains_key(reference))
                {
                    actions.push(PanelAction::LoadMore {
                        reference: reference.clone(),
                    });
                }
            }
        }
        None => {
            if response.hovered() {
                ui.painter().rect_filled(rect, 0.0, theme::ink(8));
            }
            ui.painter().with_clip_rect(rect).text(
                pos2(rect.left() + 26.0, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                FontId::monospace(11.5),
                theme::text_primary(),
            );
            ui.painter().text(
                pos2(rect.right() - WATCH_ACTIONS - 12.0, rect.center().y),
                Align2::RIGHT_CENTER,
                "—",
                FontId::proportional(kit::size::BODY),
                theme::text_tertiary(),
            );
        }
    }
    // Actions that appear on the row under the pointer.
    if response.hovered() || ui.rect_contains_pointer(rect) {
        let mut buttons = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(rect.right() - WATCH_ACTIONS, rect.top()),
                    pos2(rect.right() - 4.0, rect.bottom()),
                ))
                .layout(Layout::right_to_left(Align::Center)),
        );
        buttons.spacing_mut().item_spacing.x = 0.0;
        if icons::button(
            &mut buttons,
            Icon::Xmark,
            20.0,
            theme::text_secondary(),
            "Remove watch",
        )
        .clicked()
        {
            actions.push(PanelAction::RemoveWatch(index));
        }
        if icons::button(
            &mut buttons,
            Icon::Pencil,
            20.0,
            theme::text_secondary(),
            "Edit expression",
        )
        .clicked()
        {
            state.editing_watch = Some((index, text.to_owned()));
        }
    }
    egui::Popup::context_menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| {
            kit::menu_fit(
                ui,
                &[
                    "Edit Expression",
                    "Copy Expression",
                    "Copy Value",
                    "Remove Watch",
                ],
            );
            ui.spacing_mut().item_spacing.y = 0.0;
            if kit::menu_item(ui, Icon::Pencil, "Edit Expression", true, false) {
                state.editing_watch = Some((index, text.to_owned()));
                ui.close();
            }
            if kit::menu_item(ui, Icon::CopyDoc, "Copy Expression", true, false) {
                ui.ctx().copy_text(text.to_owned());
                ui.close();
            }
            if let Some(variable) = &value
                && !failed
                && kit::menu_item(ui, Icon::CopyDoc, "Copy Value", true, false)
            {
                ui.ctx().copy_text(variable.full.clone());
                ui.close();
            }
            if kit::menu_item(ui, Icon::Trash, "Remove Watch", true, true) {
                actions.push(PanelAction::RemoveWatch(index));
                ui.close();
            }
        });
    if let (Some(shown), Some(variable)) = (panel.shown.as_ref(), value.as_ref())
        && !failed
    {
        let lines = tree::watch_children(shown, index, variable);
        tree::show_rows(ui, &lines, shown, actions, false, None);
    }
}

// MARK: Breakpoints

pub fn breakpoints(
    ui: &mut Ui,
    panel: &DebugPanel,
    statuses: &[(u64, BreakpointState)],
    source: &str,
    state: &mut PanelUi,
    actions: &mut Vec<PanelAction>,
) {
    if panel.state.breakpoints.is_empty() {
        placeholder(
            ui,
            "No breakpoints",
            "Click a line number to stop there, or press the breakpoint key on the caret's line.",
        );
        return;
    }
    for stored in &panel.state.breakpoints {
        let status = statuses
            .iter()
            .find(|(id, _)| *id == stored.spec.id)
            .map(|(_, status)| status);
        breakpoint_row(ui, panel, stored, status, source, state, actions);
        if state
            .editor
            .as_ref()
            .is_some_and(|editor| editor.id == stored.spec.id)
        {
            breakpoint_editor(ui, state, actions);
        }
    }
}

fn hits_text(rule: crate::pine::debug::HitRule) -> Option<String> {
    use crate::pine::debug::HitRule;
    match rule {
        HitRule::Always => None,
        HitRule::Equals(n) => Some(format!("hit #{n}")),
        HitRule::EveryNth(n) => Some(format!("every {n}")),
        HitRule::GreaterThan(n) => Some(format!("after {n}")),
    }
}

fn breakpoint_row(
    ui: &mut Ui,
    panel: &DebugPanel,
    stored: &StoredBreakpoint,
    status: Option<&BreakpointState>,
    source: &str,
    state: &mut PanelUi,
    actions: &mut Vec<PanelAction>,
) {
    let spec = &stored.spec;
    let problem = match status {
        _ if stored.unresolved => {
            Some("Its line was deleted. Remove it, or set it again.".to_owned())
        }
        Some(BreakpointState::Unresolved(reason)) => Some(reason.clone()),
        Some(BreakpointState::Invalid(reason)) => Some(reason.clone()),
        _ => None,
    };
    let height = if problem.is_some() {
        ROW + 14.0
    } else {
        ROW + 2.0
    };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::ink(8));
    }
    // The switch: a dot that is on, off, or cannot be.
    let dot = pos2(rect.left() + 16.0, rect.top() + (ROW + 2.0) / 2.0);
    let dot_hit = Rect::from_center_size(dot, vec2(20.0, 20.0));
    let toggle = ui.interact(
        dot_hit,
        ui.id().with(("bp-toggle", spec.id)),
        Sense::click(),
    );
    let live = spec.enabled && problem.is_none();
    let color = if live {
        theme::editor_breakpoint()
    } else {
        theme::editor_breakpoint_muted()
    };
    if spec.log_message.is_some() {
        let r = 5.0;
        let points = vec![
            dot + vec2(0.0, -r),
            dot + vec2(r, 0.0),
            dot + vec2(0.0, r),
            dot + vec2(-r, 0.0),
        ];
        if live {
            ui.painter()
                .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
        } else {
            ui.painter()
                .add(egui::Shape::closed_line(points, Stroke::new(1.4, color)));
        }
    } else if live {
        ui.painter().circle_filled(dot, 4.6, color);
    } else {
        ui.painter()
            .circle_stroke(dot, 4.2, Stroke::new(1.4, color));
    }
    let toggle = toggle.on_hover_text(if spec.enabled {
        "Disable breakpoint"
    } else {
        "Enable breakpoint"
    });
    if toggle.clicked() {
        actions.push(PanelAction::SetBreakpointEnabled {
            id: spec.id,
            enabled: !spec.enabled,
        });
    }

    let center_y = rect.top() + (ROW + 2.0) / 2.0;
    let mut x = rect.left() + 32.0;
    let title = kit::galley(
        ui,
        &format!("Line {}", spec.line),
        kit::size::BODY - 0.5,
        kit::SEMIBOLD,
        if spec.enabled {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        },
        120.0,
    );
    let title_width = title.size().x;
    ui.painter().galley(
        pos2(x, center_y - title.size().y / 2.0),
        title,
        theme::text_primary(),
    );
    x += title_width + 8.0;
    if spec.log_message.is_some() {
        x = pill(ui, x, center_y, "log", theme::syntax::function()) + 4.0;
    }
    if spec.condition.is_some() {
        x = pill(ui, x, center_y, "if", theme::syntax::number()) + 4.0;
    }
    if let Some(text) = hits_text(spec.hit) {
        x = pill(ui, x, center_y, &text, theme::syntax::keyword()) + 4.0;
    }
    let hits = panel
        .hits
        .iter()
        .find(|(id, _)| *id == spec.id)
        .map_or(0, |(_, hits)| *hits);
    let right_clip = rect.right() - 58.0;
    if let Some(line) = source.lines().nth(spec.line.saturating_sub(1)) {
        let clip = Rect::from_min_max(pos2(x, rect.top()), pos2(right_clip - 40.0, rect.bottom()));
        if clip.width() > 20.0 {
            ui.painter().with_clip_rect(clip).text(
                pos2(x, center_y),
                Align2::LEFT_CENTER,
                line.trim(),
                FontId::monospace(11.0),
                theme::text_tertiary(),
            );
        }
    }
    if hits > 0 {
        ui.painter().text(
            pos2(right_clip, center_y),
            Align2::RIGHT_CENTER,
            format!("{hits}×"),
            FontId::proportional(kit::size::SUBHEADLINE),
            theme::text_secondary(),
        );
    }
    if let Some(problem) = &problem {
        ui.painter()
            .with_clip_rect(Rect::from_min_max(
                pos2(rect.left() + 32.0, rect.top()),
                pos2(rect.right() - 8.0, rect.bottom()),
            ))
            .text(
                pos2(rect.left() + 32.0, rect.bottom() - 9.0),
                Align2::LEFT_CENTER,
                problem,
                FontId::proportional(kit::size::SUBHEADLINE),
                theme::syntax::string(),
            );
    }
    if response.hovered() || ui.rect_contains_pointer(rect) {
        let mut buttons = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    rect.min,
                    pos2(rect.right() - 4.0, rect.top() + ROW + 2.0),
                ))
                .layout(Layout::right_to_left(Align::Center)),
        );
        buttons.spacing_mut().item_spacing.x = 0.0;
        if icons::button(
            &mut buttons,
            Icon::Xmark,
            20.0,
            theme::text_secondary(),
            "Remove breakpoint",
        )
        .clicked()
        {
            actions.push(PanelAction::RemoveBreakpoint(spec.id));
        }
        if icons::button(
            &mut buttons,
            Icon::Pencil,
            20.0,
            theme::text_secondary(),
            "Edit breakpoint",
        )
        .clicked()
        {
            state.toggle_editor(stored);
        }
    }
    let response = response.on_hover_text("Show this line");
    if response.clicked() && !stored.unresolved {
        actions.push(PanelAction::GotoLine(spec.line));
    }
    egui::Popup::context_menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| {
            kit::menu_fit(
                ui,
                &[
                    "Edit Breakpoint…",
                    "Disable Breakpoint",
                    "Remove Breakpoint",
                    "Remove All Breakpoints",
                ],
            );
            ui.spacing_mut().item_spacing.y = 0.0;
            if kit::menu_item(ui, Icon::Pencil, "Edit Breakpoint…", true, false) {
                state.toggle_editor(stored);
                ui.close();
            }
            let (icon, label) = if spec.enabled {
                (Icon::EyeSlash, "Disable Breakpoint")
            } else {
                (Icon::Eye, "Enable Breakpoint")
            };
            if kit::menu_item(ui, icon, label, true, false) {
                actions.push(PanelAction::SetBreakpointEnabled {
                    id: spec.id,
                    enabled: !spec.enabled,
                });
                ui.close();
            }
            if kit::menu_item(ui, Icon::Trash, "Remove Breakpoint", true, true) {
                actions.push(PanelAction::RemoveBreakpoint(spec.id));
                ui.close();
            }
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::Trash, "Remove All Breakpoints", true, true) {
                actions.push(PanelAction::RemoveAllBreakpoints);
                ui.close();
            }
        });
}

fn breakpoint_editor(ui: &mut Ui, state: &mut PanelUi, actions: &mut Vec<PanelAction>) {
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    let before = editor.clone();
    let id = editor.id;
    egui::Frame::new()
        .fill(theme::ink(6))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let width = ui.available_width();

            // Condition.
            label(ui, "Condition");
            hint(ui, "Stops only when this Pine expression is true.");
            text_field(
                ui,
                ui.id().with(("bp-condition", id)),
                &mut editor.condition,
                "e.g. bar_index > 100 and close > open",
                width,
                true,
            );
            if !editor.condition.trim().is_empty()
                && let Err(error) = parse_expression(&editor.condition)
            {
                problem(ui, &error.message);
            }

            // Hit count.
            ui.add_space(8.0);
            label(ui, "Hit count");
            hint(
                ui,
                "Hits are the times the condition held, counted across bars.",
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let titles: Vec<String> = HitKind::ALL
                    .iter()
                    .map(|kind| kind.title().to_owned())
                    .collect();
                let selected = HitKind::ALL
                    .iter()
                    .position(|kind| *kind == editor.hit)
                    .unwrap_or(0);
                if let Some(index) = kit::menu_picker(ui, 168.0, &titles, selected, true) {
                    editor.hit = HitKind::ALL[index];
                }
                if editor.hit != HitKind::Always {
                    text_field(
                        ui,
                        ui.id().with(("bp-count", id)),
                        &mut editor.count,
                        "n",
                        56.0,
                        true,
                    );
                }
            });
            if editor.hit != HitKind::Always {
                if editor.count.trim().parse::<u64>().is_ok_and(|n| n >= 1) {
                    hint(ui, &hit_sentence(editor.hit, editor.count.trim()));
                } else {
                    problem(ui, "Enter a whole number, 1 or more.");
                }
            }

            // Logpoint.
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                kit::switch(
                    ui,
                    &mut editor.is_logpoint,
                    "Log a message instead of stopping",
                );
                ui.label(kit::text(
                    "Log a message instead of stopping",
                    kit::size::BODY,
                    kit::REGULAR,
                    theme::text_primary(),
                ));
            });
            if editor.is_logpoint {
                hint(
                    ui,
                    "Goes to the Logs tab. {expression} shows its value; {{ and }} are braces.",
                );
                text_field(
                    ui,
                    ui.id().with(("bp-message", id)),
                    &mut editor.message,
                    "Bar {bar_index}: close={close}",
                    width,
                    true,
                );
                if !editor.message.is_empty()
                    && let Err(error) = Template::parse(&editor.message)
                {
                    problem(ui, &error);
                } else if editor.message.is_empty() {
                    hint(
                        ui,
                        "Type a message; until then the breakpoint stops as usual.",
                    );
                }
            }
        });
    if editor.condition != before.condition {
        actions.push(PanelAction::EditBreakpoint {
            id,
            edit: BreakpointEdit::Condition(editor.condition.clone()),
        });
    }
    if editor.hit != before.hit || editor.count != before.count {
        let count = editor.count.trim().parse::<u64>().unwrap_or(1).max(1);
        actions.push(PanelAction::EditBreakpoint {
            id,
            edit: BreakpointEdit::Hit(editor.hit.rule(count)),
        });
    }
    if editor.is_logpoint != before.is_logpoint || editor.message != before.message {
        actions.push(PanelAction::EditBreakpoint {
            id,
            edit: BreakpointEdit::LogMessage(
                (editor.is_logpoint && !editor.message.is_empty()).then(|| editor.message.clone()),
            ),
        });
    }
}

/// What a hit rule means with its number, as a sentence.
fn hit_sentence(kind: HitKind, n: &str) -> String {
    match kind {
        HitKind::Always => "Stops every time the condition holds.".to_owned(),
        HitKind::Equals => format!("Stops on hit number {n} only."),
        HitKind::EveryNth => format!("Stops on every {} hit.", ordinal(n)),
        HitKind::GreaterThan => format!("Stops on every hit after the {}.", ordinal(n)),
    }
}

/// "1st", "2nd", "3rd", "4th", "11th", "22nd" for a number typed as digits.
fn ordinal(n: &str) -> String {
    let suffix = match (n.chars().last(), n.chars().rev().nth(1)) {
        (_, Some('1')) => "th",
        (Some('1'), _) => "st",
        (Some('2'), _) => "nd",
        (Some('3'), _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn label(ui: &mut Ui, title: &str) {
    ui.label(kit::text(
        title,
        kit::size::SUBHEADLINE,
        kit::SEMIBOLD,
        theme::text_secondary(),
    ));
}

/// A line of explanation under a field.
fn hint(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_tertiary(),
    ));
}

/// What is wrong with what was typed.
fn problem(ui: &mut Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::syntax::string(),
    ));
}

// MARK: Console

pub fn console(
    ui: &mut Ui,
    panel: &DebugPanel,
    state: &mut PanelUi,
    actions: &mut Vec<PanelAction>,
) {
    if panel.console.is_empty() {
        placeholder(
            ui,
            "Evaluate Pine expressions",
            "Stop the script, then type one: close[1], bar_index % 5 == 0, myArray.size().",
        );
    }
    for line in &panel.console {
        ui.horizontal_wrapped(|ui| {
            ui.add_space(12.0);
            ui.label(mono(
                format!("› {}", line.input),
                11.5,
                theme::text_secondary(),
            ));
        });
        match &line.result {
            Some(Watched::Value(variable)) => {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(26.0);
                    let color = if variable.is_na {
                        theme::text_tertiary()
                    } else {
                        theme::text_primary()
                    };
                    let response = ui.label(mono(variable.full.clone(), 11.5, color));
                    if response.clicked() {
                        ui.ctx().copy_text(variable.full.clone());
                    }
                    ui.label(kit::text(
                        format!("  {}", variable.type_name),
                        kit::size::SUBHEADLINE,
                        kit::REGULAR,
                        theme::text_tertiary(),
                    ));
                });
            }
            Some(Watched::Error(message)) => {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(26.0);
                    ui.label(kit::text(
                        message.clone(),
                        kit::size::SUBHEADLINE,
                        kit::REGULAR,
                        theme::syntax::string(),
                    ));
                });
            }
            None => {
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    ui.label(kit::text(
                        "…",
                        kit::size::SUBHEADLINE,
                        kit::REGULAR,
                        theme::text_tertiary(),
                    ));
                });
            }
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let width = ui.available_width() - 8.0 - 24.0;
        let id = ui.id().with("console-input");
        let response = text_field(ui, id, &mut state.console_input, "Evaluate…", width, true);
        let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter && !state.console_input.trim().is_empty() {
            actions.push(PanelAction::Evaluate(std::mem::take(
                &mut state.console_input,
            )));
            response.request_focus();
        }
        if !panel.console.is_empty() && icon_button(ui, Icon::Trash, true, "Clear the console") {
            actions.push(PanelAction::ClearConsole);
        }
    });
    ui.add_space(6.0);
    let _ = Id::NULL;
}
