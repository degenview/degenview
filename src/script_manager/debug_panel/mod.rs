//! The debug panel under the code editor: what the script is doing, its variables, and the lists a
//! debugger keeps (watches, call stack, breakpoints, console).
//!
//! It draws a [`DebugPanel`] and returns the [`PanelAction`]s the person asked for; it changes
//! nothing itself. On a wide window the variables take the left and the lists stack on the right;
//! on a narrow one everything stacks in one column.

mod lists;
mod toolbar;
mod tree;
mod widgets;

use std::cell::RefCell;

use eframe::egui::{
    self, Align, Layout, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};

use crate::pine::debug::{
    BarContext, BreakpointState, DebugCommand, DebugState, Outcome, StopReason,
};
use crate::script_manager::debug::{DebugPanel, PanelAction, PanelUi, Section};
use crate::script_manager::prefs::LayoutPrefs;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::split::{self, Axis, DefaultLength, SplitSpec, SplitState};
use crate::ui::theme;

pub use toolbar::{panel_button, transport};

use widgets::{ROW, icon_button, mono, placeholder, section_header, text_field};

const STATUS_HEIGHT: f32 = 30.0;
const STRIP_HEIGHT: f32 = 24.0;
/// Below this width the two columns stack.
const WIDE: f32 = 640.0;

/// What the sections need to draw.
struct Context<'a> {
    panel: &'a DebugPanel,
    state: &'a mut PanelUi,
    actions: &'a mut Vec<PanelAction>,
    sections: &'a mut u8,
    statuses: &'a [(u64, BreakpointState)],
    source: &'a str,
}

impl Context<'_> {
    fn is_open(&self, section: Section) -> bool {
        *self.sections & section as u8 != 0
    }

    fn toggle(&mut self, section: Section) {
        *self.sections ^= section as u8;
    }
}

/// Draws the panel into the space `ui` has left, and returns what was asked of it.
pub fn show(
    ui: &mut Ui,
    panel: &mut DebugPanel,
    prefs: &mut LayoutPrefs,
    source: &str,
    out_of_date: bool,
) -> Vec<PanelAction> {
    // The panel's own state (text being typed, a popover) is lent to the drawing, which reads the
    // rest of the panel.
    let mut state = std::mem::take(&mut panel.ui);
    let statuses: Vec<(u64, BreakpointState)> = panel
        .statuses()
        .iter()
        .map(|status| (status.id, status.state.clone()))
        .collect();
    let actions = draw(ui, panel, &mut state, &statuses, prefs, source, out_of_date);
    panel.ui = state;
    actions
}

fn draw(
    ui: &mut Ui,
    panel: &DebugPanel,
    state: &mut PanelUi,
    statuses: &[(u64, BreakpointState)],
    prefs: &mut LayoutPrefs,
    source: &str,
    out_of_date: bool,
) -> Vec<PanelAction> {
    let mut actions = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    ui.painter().rect_filled(full, 0.0, theme::background());
    let status = Rect::from_min_size(full.min, vec2(full.width(), STATUS_HEIGHT));
    status_bar(ui, status, panel, out_of_date, &mut actions);
    let mut top = status.bottom();
    if let Some(bar) = panel.bar() {
        let strip = Rect::from_min_max(
            pos2(full.left(), top),
            pos2(full.right(), top + STRIP_HEIGHT),
        );
        bar_strip(ui, strip, bar, &panel.label, panel.shown.is_some());
        top = strip.bottom();
    }
    let body = Rect::from_min_max(pos2(full.left(), top), full.max);
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body));
    body_ui.set_clip_rect(body.intersect(ui.clip_rect()));

    let mut context = Context {
        panel,
        state,
        actions: &mut actions,
        sections: &mut prefs.debug_sections,
        statuses,
        source,
    };
    if body.width() >= WIDE {
        let spec = SplitSpec {
            axis: Axis::Horizontal,
            secondary_first: false,
            min_primary: 260.0,
            min_secondary: 260.0,
            max_secondary: f32::MAX,
            default: DefaultLength::Fraction(0.46),
        };
        let mut split_state = SplitState {
            length: prefs.debug_side_width,
            visible: true,
        };
        let cell = RefCell::new(&mut context);
        split::show(
            &mut body_ui,
            egui::Id::new("script-debug-columns"),
            spec,
            &mut split_state,
            |ui| variables_pane(ui, &mut cell.borrow_mut(), true),
            |ui| sections_pane(ui, &mut cell.borrow_mut(), true),
        );
        prefs.debug_side_width = split_state.length;
    } else {
        stacked(&mut body_ui, &mut context);
    }
    actions
}

// MARK: Status

fn status_bar(
    ui: &mut Ui,
    rect: Rect,
    panel: &DebugPanel,
    out_of_date: bool,
    actions: &mut Vec<PanelAction>,
) {
    ui.painter().rect_filled(rect, 0.0, theme::chrome());
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, kit::hairline()),
    );
    let (color, text) = headline(panel);
    let dot = pos2(rect.left() + 16.0, rect.center().y);
    ui.painter().circle_filled(dot, 4.0, color);
    let mut content = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(rect.left() + 30.0, rect.top()),
                pos2(rect.right() - 6.0, rect.bottom()),
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    content.spacing_mut().item_spacing.x = 8.0;
    content.label(kit::text(
        text,
        kit::size::BODY - 0.5,
        kit::MEDIUM,
        theme::text_primary(),
    ));
    content.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if icons::button(
            ui,
            Icon::Xmark,
            22.0,
            theme::text_secondary(),
            "Close the debugger panel",
        )
        .clicked()
        {
            actions.push(PanelAction::Close);
        }
        if out_of_date && panel.has_session() {
            let response = kit::push_button(
                ui,
                Some(Icon::Restart),
                "Restart on the new code",
                true,
            )
            .on_hover_text(
                "The script was edited after this session began; it is still running the old text.",
            );
            if response.clicked() {
                actions.push(PanelAction::Command(DebugCommand::Restart));
            }
        }
        if let Some(notice) = &panel.notice {
            ui.label(kit::text(
                notice.text.clone(),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                if notice.error {
                    theme::syntax::string()
                } else {
                    theme::text_secondary()
                },
            ));
        }
    });
}

fn headline(panel: &DebugPanel) -> (eframe::egui::Color32, String) {
    let resting = theme::text_tertiary();
    match panel.state_of_session() {
        DebugState::Idle => (
            resting,
            "Not debugging — press ▶ to run to a breakpoint, or step to start at the first line"
                .to_owned(),
        ),
        DebugState::Preparing => (resting, "Starting…".to_owned()),
        DebugState::Running => (kit::GREEN, "Running".to_owned()),
        DebugState::Pausing => (theme::editor_debug_line(), "Pausing…".to_owned()),
        DebugState::Stopping => (resting, "Stopping…".to_owned()),
        DebugState::Paused => match panel.shown.as_ref().map(|shown| &shown.snapshot) {
            Some(snapshot) => {
                let line = snapshot
                    .stop
                    .location
                    .map(|location| format!(" at line {}", location.line))
                    .unwrap_or_default();
                match &snapshot.stop.reason {
                    StopReason::Breakpoint(_) => (
                        theme::editor_debug_line(),
                        format!("Paused on a breakpoint{line}"),
                    ),
                    StopReason::Step => (theme::editor_debug_line(), format!("Paused{line}")),
                    StopReason::Pause => (
                        theme::editor_debug_line(),
                        format!("Paused by request{line}"),
                    ),
                    StopReason::Bar => (
                        theme::editor_debug_line(),
                        format!("Paused at the start of bar {}", snapshot.stop.bar_index),
                    ),
                    StopReason::Exception(error) => (
                        theme::editor_breakpoint(),
                        format!("{}{line}: {}", error.code, error.message),
                    ),
                }
            }
            None => (theme::editor_debug_line(), "Paused".to_owned()),
        },
        DebugState::Completed => match &panel.outcome {
            Some(Outcome::Stopped) => (resting, "Stopped".to_owned()),
            _ => (
                kit::GREEN,
                "Finished — the script ran to the end of the bars".to_owned(),
            ),
        },
        DebugState::Failed => match panel.failure() {
            Some(error) => (
                theme::editor_breakpoint(),
                format!("{}: {}", error.code, error.message),
            ),
            None => (theme::editor_breakpoint(), "The script failed".to_owned()),
        },
    }
}

fn bar_strip(ui: &mut Ui, rect: Rect, bar: &BarContext, label: &str, stopped: bool) {
    ui.painter().rect_filled(rect, 0.0, theme::ink(4));
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, kit::hairline()),
    );
    let mut row = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 10.0;
    let dim = |text: String| {
        kit::text(
            text,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        )
    };
    let strong = |text: String| {
        kit::text(
            text,
            kit::size::SUBHEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        )
    };
    match &bar.request {
        Some(request) => {
            row.label(strong(format!("REQUEST BAR {}", bar.bar_index)));
            row.label(dim(format!(
                "{} / {}",
                request.symbol,
                crate::pine::debug::timeframe_label(request.interval_seconds)
            )));
            row.label(dim(format!("chart bar {}", request.chart_bar_index)));
        }
        None => {
            row.label(strong(format!("BAR {}", bar.bar_index)));
            if !label.is_empty() {
                row.label(dim(label.to_owned()));
            }
            row.label(dim(if bar.historical {
                "Historical"
            } else {
                "Realtime"
            }
            .to_owned()));
            row.label(dim(if bar.confirmed {
                "Confirmed"
            } else {
                "Unconfirmed"
            }
            .to_owned()));
        }
    }
    row.label(dim(format_time(bar.time_ms)));
    if row.available_width() > 300.0 {
        row.label(mono(
            format!(
                "O {}  H {}  L {}  C {}  V {}",
                number(bar.open),
                number(bar.high),
                number(bar.low),
                number(bar.close),
                number(bar.volume)
            ),
            10.5,
            theme::text_secondary(),
        ));
    }
    if !stopped {
        row.label(dim("last stop".to_owned()));
    }
}

fn number(value: f64) -> String {
    crate::pine::number_format::format(
        &crate::pine::runtime_value::RuntimeValue::Float(value),
        None,
        0.01,
    )
}

/// `2026-10-24 12:00` (UTC).
fn format_time(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// The calendar date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

// MARK: Variables

fn variables_pane(ui: &mut Ui, context: &mut Context<'_>, with_header: bool) {
    let panel = context.panel;
    if with_header {
        let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        ui.painter().rect_filled(header, 0.0, theme::ink(6));
        ui.painter().line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0, kit::hairline()),
        );
        let mut row = ui.new_child(
            UiBuilder::new()
                .max_rect(header.shrink2(vec2(12.0, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        row.label(kit::text(
            "VARIABLES",
            kit::size::CAPTION + 0.5,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        if let Some(shown) = panel.shown.as_ref()
            && let Some(frame) = shown
                .snapshot
                .stack
                .iter()
                .find(|frame| frame.index == shown.frame)
            && shown.frame != shown.snapshot.frame
        {
            row.label(kit::text(
                format!("in {}", frame.label),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        }
        row.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let id = egui::Id::new("script-debug-filter");
            kit::search_field(ui, id, &mut context.state.filter, "Filter", 150.0);
        });
    }
    let Some(shown) = panel.shown.as_ref() else {
        placeholder(
            ui,
            "Nothing to inspect",
            "Variables appear here when the script is stopped at a breakpoint or a step.",
        );
        return;
    };
    let lines = tree::variable_lines(shown, &context.state.filter);
    if lines.is_empty() {
        placeholder(
            ui,
            if context.state.filter.trim().is_empty() {
                "No variables in scope"
            } else {
                "No matches"
            },
            "",
        );
        return;
    }
    let stale = !panel.is_paused();
    // The list takes the keyboard once a row is picked: arrows move, Right and Left open and close.
    let list_id = egui::Id::new("script-debug-variables-list");
    let focused = ui.memory(|memory| memory.has_focus(list_id));
    if focused {
        // The arrows are this list's: egui would otherwise move the focus with them.
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                list_id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        let navigation = tree::navigate(
            ui,
            &lines,
            shown,
            context.state.cursor.as_deref(),
            context.actions,
        );
        context.state.cursor = navigation.cursor;
        if let Some(index) = navigation.reveal {
            context.state.reveal = Some(index);
        }
    }
    let mut area = ScrollArea::vertical()
        .id_salt("script-debug-variables")
        .auto_shrink([false, false]);
    if let Some(offset) = context.state.scroll_to.take() {
        area = area.vertical_scroll_offset(offset);
    }
    let before = context.actions.len();
    let cursor = context.state.cursor.clone();
    let output = ui
        .add_enabled_ui(!stale, |ui| {
            area.show_rows(ui, ROW, lines.len(), |ui, range| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                tree::show_rows(
                    ui,
                    &lines[range],
                    shown,
                    context.actions,
                    true,
                    cursor.as_deref(),
                );
            })
        })
        .inner;
    if context.actions[before..]
        .iter()
        .any(|action| matches!(action, PanelAction::Cursor(_)))
    {
        ui.memory_mut(|memory| memory.request_focus(list_id));
    }
    // The list is a stop on the Tab order, and says so while it has the keys.
    let response = ui.interact(
        output.inner_rect,
        list_id,
        Sense::focusable_noninteractive(),
    );
    if focused {
        ui.painter().rect_stroke(
            output.inner_rect.shrink(1.0),
            0.0,
            Stroke::new(1.0, theme::with_alpha(kit::ACCENT, 0.7)),
            egui::StrokeKind::Inside,
        );
    }
    let _ = response;
    if let Some(index) = context.state.reveal.take() {
        let top = index as f32 * ROW;
        let height = output.inner_rect.height();
        let offset = output.state.offset.y;
        if top < offset {
            context.state.scroll_to = Some(top);
        } else if top + ROW > offset + height {
            context.state.scroll_to = Some(top + ROW - height);
        }
        ui.ctx().request_repaint();
    }
}

// MARK: The other sections

fn sections_pane(ui: &mut Ui, context: &mut Context<'_>, scroll: bool) {
    let body = |ui: &mut Ui, context: &mut Context<'_>| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        section(ui, context, Section::Watches);
        section(ui, context, Section::Stack);
        section(ui, context, Section::Breakpoints);
        section(ui, context, Section::Console);
    };
    if scroll {
        ScrollArea::vertical()
            .id_salt("script-debug-sections")
            .auto_shrink([false, false])
            .show(ui, |ui| body(ui, context));
    } else {
        body(ui, context);
    }
}

fn stacked(ui: &mut Ui, context: &mut Context<'_>) {
    ScrollArea::vertical()
        .id_salt("script-debug-stacked")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let count = context
                .panel
                .shown
                .as_ref()
                .map(|shown| shown.variables.iter().map(|g| g.variables.len()).sum());
            section_header(ui, "Variables", count, true, |_| {});
            ui.allocate_ui(vec2(ui.available_width(), 190.0), |ui| {
                variables_pane(ui, context, false);
            });
            sections_pane(ui, context, false);
        });
}

fn section(ui: &mut Ui, context: &mut Context<'_>, which: Section) {
    let panel = context.panel;
    let open = context.is_open(which);
    let (title, count) = match which {
        Section::Watches => ("Watches", Some(panel.state.watches.len())),
        Section::Stack => ("Call Stack", Some(panel.stack().len())),
        Section::Breakpoints => ("Breakpoints", Some(panel.state.breakpoints.len())),
        Section::Console => ("Console", None),
    };
    let mut trailing_actions = Vec::new();
    let toggled = section_header(ui, title, count, open, |ui| match which {
        Section::Breakpoints if !panel.state.breakpoints.is_empty() => {
            if icon_button(ui, Icon::Trash, true, "Remove all breakpoints") {
                trailing_actions.push(PanelAction::RemoveAllBreakpoints);
            }
            let all_on = panel.state.breakpoints.iter().all(|b| b.spec.enabled);
            if icon_button(
                ui,
                if all_on { Icon::EyeSlash } else { Icon::Eye },
                true,
                if all_on {
                    "Disable all breakpoints"
                } else {
                    "Enable all breakpoints"
                },
            ) {
                trailing_actions.push(PanelAction::EnableAllBreakpoints(!all_on));
            }
        }
        Section::Console
            if !panel.console.is_empty()
                && icon_button(ui, Icon::Trash, true, "Clear the console") =>
        {
            trailing_actions.push(PanelAction::ClearConsole);
        }
        _ => {}
    });
    context.actions.append(&mut trailing_actions);
    if toggled {
        context.toggle(which);
    }
    if !open {
        return;
    }
    match which {
        Section::Watches => lists::watches(ui, panel, context.state, context.actions),
        Section::Stack => lists::call_stack(ui, panel, context.actions),
        Section::Breakpoints => {
            lists::breakpoints(
                ui,
                panel,
                context.statuses,
                context.source,
                context.state,
                context.actions,
            );
            bar_break(ui, context);
        }
        Section::Console => lists::console(ui, panel, context.state, context.actions),
    }
}

/// The Pine-aware breakpoint: stop at the start of a bar, whatever runs in it.
fn bar_break(ui: &mut Ui, context: &mut Context<'_>) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(kit::text(
            "Stop at bar",
            kit::size::SUBHEADLINE,
            kit::SEMIBOLD,
            theme::text_secondary(),
        ));
        let width = (ui.available_width() - 90.0).max(80.0);
        let id = ui.id().with("bar-break");
        let response = text_field(
            ui,
            id,
            &mut context.state.bar_input,
            "index or 2026-10-24 12:00",
            width,
            true,
        );
        let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter && !context.state.bar_input.trim().is_empty() {
            context.actions.push(PanelAction::BreakOnBar(std::mem::take(
                &mut context.state.bar_input,
            )));
        }
        if context.panel.bar_break.is_some()
            && icon_button(ui, Icon::Xmark, true, "Cancel the stop at a bar")
        {
            context.actions.push(PanelAction::ClearBarBreak);
        }
    });
    if let Some(armed) = &context.panel.bar_break {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(kit::text(
                format!("Armed for {armed}"),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::editor_debug_line(),
            ));
        });
    }
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_written_in_utc() {
        assert_eq!(format_time(0), "1970-01-01 00:00");
        assert_eq!(format_time(1_792_843_200_000), "2026-10-24 12:00");
        assert_eq!(
            format_time(951_868_800_000 + 3_600_000 * 5 + 60_000 * 7),
            "2000-03-01 05:07"
        );
        assert_eq!(format_time(-1000), "1969-12-31 23:59");
    }
}
