//! The transport buttons that sit in the Script Manager's toolbar.

use eframe::egui::{Color32, Sense, Ui, vec2};

use crate::pine::debug::DebugCommand;
use crate::script_manager::debug::DebugPanel;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::shortcut;
use crate::ui::theme;

/// One transport button. A command that does not apply is drawn faint and still names itself.
fn button(ui: &mut Ui, icon: Icon, color: Color32, enabled: bool, tooltip: &str) -> bool {
    if enabled {
        return icons::button(ui, icon, 28.0, color, tooltip).clicked();
    }
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
    response.on_hover_text(tooltip);
    icons::paint(
        ui.painter(),
        eframe::egui::Rect::from_center_size(rect.center(), vec2(28.0 * 0.62, 28.0 * 0.62)),
        icon,
        theme::text_tertiary().gamma_multiply(0.55),
    );
    false
}

/// The debugger button: shows or hides the panel under the editor.
pub fn panel_button(ui: &mut Ui, panel_open: bool) -> bool {
    let tip = if panel_open {
        "Hide the debugger"
    } else {
        "Show the debugger: controls, breakpoints, variables, watches"
    };
    let color = if panel_open {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    icons::button(ui, Icon::Bug, 28.0, color, tip).clicked()
}

/// The transport buttons and what the session is doing. Returns the commands pressed.
pub fn transport(ui: &mut Ui, panel: &DebugPanel) -> Vec<DebugCommand> {
    let mut commands = Vec::new();
    let (continue_tip, continue_icon) = if panel.would_start() {
        (
            shortcut::DEBUG_CONTINUE.hint("Start debugging"),
            Icon::PlayFill,
        )
    } else {
        (shortcut::DEBUG_CONTINUE.hint("Continue"), Icon::PlayFill)
    };
    let steps = [
        (
            DebugCommand::Continue,
            continue_icon,
            kit::GREEN,
            continue_tip,
        ),
        (
            DebugCommand::Pause,
            Icon::PauseFill,
            theme::text_primary(),
            shortcut::DEBUG_PAUSE.hint("Pause at the next statement"),
        ),
        (
            DebugCommand::StepOver,
            Icon::StepOver,
            theme::text_primary(),
            shortcut::DEBUG_STEP_OVER
                .hint("Step over: run this statement and stop at the next one"),
        ),
        (
            DebugCommand::StepInto,
            Icon::StepInto,
            theme::text_primary(),
            shortcut::DEBUG_STEP_INTO.hint("Step into a function call"),
        ),
        (
            DebugCommand::StepOut,
            Icon::StepOut,
            theme::text_primary(),
            shortcut::DEBUG_STEP_OUT.hint("Step out: run until the function returns"),
        ),
        (
            DebugCommand::StepBar,
            Icon::StepBar,
            theme::text_primary(),
            shortcut::DEBUG_STEP_BAR.hint("Step bar: run to the first statement of the next bar"),
        ),
        (
            DebugCommand::Restart,
            Icon::Restart,
            theme::text_primary(),
            shortcut::DEBUG_RESTART.hint("Restart on the current code"),
        ),
        (
            DebugCommand::Stop,
            Icon::StopFill,
            kit::RED,
            shortcut::DEBUG_STOP.hint("Stop debugging"),
        ),
    ];
    for (command, icon, color, tip) in steps {
        let enabled = panel.allows(command);
        if button(ui, icon, color, enabled, &tip) {
            commands.push(command);
        }
        if command == DebugCommand::Pause {
            ui.add_space(4.0);
        }
    }
    commands
}
