//! Pictures of the debugger in the whole window, in both themes, for a person to look at. Run by
//! hand: `DEGENVIEW_GALLERY_DIR=… cargo test --lib script_manager::debug_gallery -- --ignored`.

use std::time::Duration;

use eframe::egui;

use super::debug::{BreakpointEdit, PanelAction};
use super::debug_marks::GutterEvent;
use super::debug_tests::Rig;
use super::*;
use crate::pine::debug::{DebugCommand, HitRule};
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::theme::AppTheme;

struct Scene<'a> {
    rig: &'a mut Rig,
    screen: Screen,
    last: Option<egui::FullOutput>,
    name: String,
}

impl Scene<'_> {
    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            theme::sync(&self.screen.ctx);
            let manager = &mut self.rig.manager;
            self.last = Some(self.screen.run(Vec::new(), |ui| {
                manager.ui(ui, None);
            }));
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn until_paused(&mut self) {
        for _ in 0..600 {
            if self.rig.manager.debug.is_paused() {
                break;
            }
            self.frames(1);
        }
        assert!(self.rig.manager.debug.is_paused(), "never stopped");
    }

    fn push(&mut self, cmd: Cmd) {
        self.rig.manager.cmds.push(cmd);
    }

    fn save(&mut self, label: &str) {
        let output = self.last.take().expect("a frame");
        self.screen
            .save(&format!("debugger-{label}-{}", self.name), output);
    }
}

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_debugger() {
    if gallery_dir().is_none() {
        return;
    }
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (layout, width) in [("wide", 1360.0), ("full", 1360.0), ("narrow", 760.0)] {
            let mut rig = Rig::new(&format!("gallery-{layout}-{theme_name}"));
            if layout == "full" {
                rig.manager.prefs.chart_position = prefs::ChartPosition::Bottom;
            }
            let screen = Screen::new(egui::vec2(width, 860.0));
            theme::apply(&screen.ctx, app_theme);
            let mut scene = Scene {
                rig: &mut rig,
                screen,
                last: None,
                name: format!("{layout}-{theme_name}"),
            };
            scene.frames(3);
            // Breakpoints of every kind, before any session.
            for line in [5, 9] {
                scene.push(Cmd::Gutter(GutterEvent::Toggle { line }));
            }
            scene.push(Cmd::Gutter(GutterEvent::AddLogpoint { line: 10 }));
            scene.frames(3);
            let ids: Vec<u64> = scene
                .rig
                .manager
                .debug
                .state
                .breakpoints
                .iter()
                .map(|b| b.spec.id)
                .collect();
            scene.push(Cmd::DebugPanel(PanelAction::EditBreakpoint {
                id: ids[1],
                edit: BreakpointEdit::Condition("bar_index > 20".into()),
            }));
            scene.push(Cmd::DebugPanel(PanelAction::EditBreakpoint {
                id: ids[2],
                edit: BreakpointEdit::LogMessage(Some("Bar {bar_index}: close={close}".into())),
            }));
            scene.push(Cmd::DebugPanel(PanelAction::SetBreakpointEnabled {
                id: ids[0],
                enabled: false,
            }));
            for watch in ["close - open", "levels", "ta.sma(close, 5)"] {
                scene.push(Cmd::DebugPanel(PanelAction::AddWatch(watch.into())));
            }
            scene.rig.manager.prefs.debug_open = true;
            scene.frames(4);
            scene.save("idle");

            // Stopped inside the function, an array open.
            scene.push(Cmd::DebugPanel(PanelAction::SetBreakpointEnabled {
                id: ids[0],
                enabled: true,
            }));
            scene.push(Cmd::DebugPanel(PanelAction::EditBreakpoint {
                id: ids[0],
                edit: BreakpointEdit::Hit(HitRule::GreaterThan(6)),
            }));
            scene.frames(2);
            scene.push(Cmd::Debug(DebugCommand::Continue));
            scene.until_paused();
            scene.frames(2);
            let levels = scene
                .rig
                .manager
                .debug
                .shown
                .as_ref()
                .and_then(|shown| {
                    shown
                        .variables
                        .iter()
                        .flat_map(|group| &group.variables)
                        .find(|variable| variable.name == "levels")
                })
                .and_then(|variable| variable.expand.clone());
            if let Some(reference) = levels {
                scene.push(Cmd::DebugPanel(PanelAction::ToggleRow {
                    key: "levels".into(),
                    reference,
                }));
            }
            scene.frames(6);
            scene.save("paused-function");

            // The caller's frame selected.
            let caller = scene
                .rig
                .manager
                .debug
                .stack()
                .last()
                .map_or(0, |frame| frame.index);
            scene.push(Cmd::DebugPanel(PanelAction::SelectFrame(caller)));
            scene.frames(8);
            scene.save("caller-frame");

            // A script that fails.
            scene.rig.manager.debug.end_session();
            scene.rig.set_source(
                "//@version=6\nindicator(\"Fails\")\nvalues = array.new_float(3)\nindex = bar_index + 2\npicked = array.get(values, index)\nplot(picked)\n",
            );
            scene.push(Cmd::Debug(DebugCommand::Continue));
            scene.until_paused();
            scene.frames(6);
            scene.save("error");
            scene.rig.manager.debug.end_session();
        }
    }
}
