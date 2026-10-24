//! The debugger through the whole Script Manager window: starting on the open script and the
//! preview chart's bars, the gutter, the toolbar's commands and the shortcuts, persistence, and the
//! session's end with the script, the tab and the window.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use super::debug::{PanelAction, StartMode};
use super::debug_marks::{GutterEvent, MarkKind};
use super::*;
use crate::model::Kline;
use crate::pine::debug::{DebugCommand, DebugState, Outcome};
use crate::store::Store;

/// Line 3 is `calculate`; the body is lines 4-6, `value = …` is 9, `plot` is 11.
const SOURCE: &str = "//@version=6
indicator(\"Debug demo\")
calculate(x) =>
    doubled = x * 2
    result = doubled + 10
    result

var levels = array.new_float(0)
value = calculate(close)
array.push(levels, value)
plot(value)
";

fn bars(count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let price = 100.0 + index as f64;
            Kline {
                open_time: 1_700_000_000_000 + index as i64 * 3_600_000,
                close_time: 1_700_000_000_000 + (index as i64 + 1) * 3_600_000,
                open: price,
                high: price + 2.0,
                low: price - 2.0,
                close: price + 1.0,
                volume: 10.0,
            }
        })
        .collect()
}

/// Removes the scripts folder when the last rig over it goes.
pub(super) struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) struct Rig {
    pub manager: ScriptManager,
    pub shared: SharedStore,
    folder: Folder,
    pub ctx: egui::Context,
}

impl Rig {
    pub fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("degenview-dbg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        Self::over(Folder(root), shared)
    }

    fn over(folder: Folder, shared: SharedStore) -> Self {
        let root = folder.0.clone();
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        let mut manager = ScriptManager::with_script_store(1, shared.clone(), Some(store));
        manager.first_load();
        let id = manager.scripts[0].id.clone();
        manager.open_script(&id);
        let mut rig = Self {
            manager,
            shared,
            folder,
            ctx: egui::Context::default(),
        };
        rig.set_source(SOURCE);
        rig.manager.preview.set_candles_for_test(bars(40));
        rig.frame();
        rig
    }

    /// A second manager over the same store and folders: the app restarted.
    pub fn restarted(self) -> Self {
        let Self {
            manager,
            shared,
            folder,
            ..
        } = self;
        drop(manager);
        Self::over(folder, shared)
    }

    pub fn set_source(&mut self, source: &str) {
        let open = self.manager.open.as_mut().expect("a script is open");
        open.editor.replace_source(source, false);
        self.manager.editor_changed();
    }

    pub fn frame(&mut self) {
        self.frame_with(Vec::new());
    }

    pub fn frame_with(&mut self, events: Vec<egui::Event>) {
        let manager = &mut self.manager;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                events,
                ..egui::RawInput::default()
            },
            |ui| {
                manager.ui(ui, None);
            },
        );
        output.textures_delta.clear();
    }

    pub fn press(&mut self, key: egui::Key) {
        self.frame_with(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]);
    }

    pub fn run(&mut self, command: DebugCommand) {
        self.manager.cmds.push(Cmd::Debug(command));
        self.frame();
    }

    pub fn gutter(&mut self, event: GutterEvent) {
        self.manager.cmds.push(Cmd::Gutter(event));
        self.frame();
    }

    /// Frames until `done`, waiting a moment between them for the session's thread.
    pub fn until(&mut self, what: &str, done: impl Fn(&ScriptManager) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.frame();
            if done(&self.manager) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the session is {:?}",
                self.manager.debug.state_of_session()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn paused(&mut self) {
        self.until("a stop", |m| m.debug.is_paused());
    }

    pub fn stopped_line(&self) -> Option<usize> {
        let shown = self.manager.debug.shown.as_ref()?;
        shown
            .snapshot
            .stack
            .iter()
            .find(|frame| frame.index == shown.frame)
            .and_then(|frame| frame.location.map(|l| l.line))
    }
}

fn breakpoint_lines(manager: &ScriptManager) -> Vec<usize> {
    manager
        .debug
        .state
        .breakpoints
        .iter()
        .map(|b| b.spec.line)
        .collect()
}

#[test]
fn a_click_in_the_gutter_sets_and_clears_a_breakpoint_that_the_editor_draws() {
    let mut rig = Rig::new("gutter");
    rig.gutter(GutterEvent::Toggle { line: 9 });
    assert_eq!(breakpoint_lines(&rig.manager), vec![9]);
    let marks = rig.manager.debug.marks();
    assert_eq!(marks.breakpoints.len(), 1);
    assert_eq!(marks.breakpoints[0].kind, MarkKind::Line);
    assert!(marks.breakpoints[0].enabled && !marks.breakpoints[0].unverified);
    rig.gutter(GutterEvent::Toggle { line: 9 });
    assert!(breakpoint_lines(&rig.manager).is_empty());
}

#[test]
fn a_line_with_no_code_is_refused_with_the_reason() {
    let mut rig = Rig::new("refuse");
    for (line, expected) in [
        (3, "stop inside its body"),
        (7, "no code"),
        (2, "runs nothing"),
    ] {
        rig.gutter(GutterEvent::Toggle { line });
        assert!(breakpoint_lines(&rig.manager).is_empty(), "line {line}");
        let notice = rig.manager.debug.notice.clone().expect("a notice");
        assert!(
            notice.text.contains(expected),
            "line {line}: {}",
            notice.text
        );
    }
}

#[test]
fn start_runs_to_a_breakpoint_and_shows_where_it_stopped() {
    let mut rig = Rig::new("start");
    rig.gutter(GutterEvent::Toggle { line: 5 });
    rig.run(DebugCommand::Continue);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(5));
    assert!(rig.manager.prefs.debug_open, "starting shows the panel");
    let stack: Vec<String> = rig
        .manager
        .debug
        .stack()
        .iter()
        .map(|f| f.label.clone())
        .collect();
    assert_eq!(stack, ["calculate()", "Main Script"]);
    let marks = rig.manager.debug.marks();
    assert_eq!(marks.stop.map(|s| (s.line, s.innermost)), Some((5, true)));
    assert!(
        marks.values.contains_key("doubled"),
        "{:?}",
        marks.values.keys()
    );
    let (value, type_name) = &marks.values["doubled"];
    assert_eq!((value.as_str(), type_name.as_str()), ("202", "float"));
}

#[test]
fn stepping_with_the_toolbar_commands_and_the_call_stack() {
    let mut rig = Rig::new("step");
    rig.gutter(GutterEvent::Toggle { line: 9 });
    rig.run(DebugCommand::Continue);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(9));
    rig.run(DebugCommand::StepInto);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(4));
    rig.run(DebugCommand::StepOver);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(5));
    rig.run(DebugCommand::StepOut);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(10));
    // Step Bar runs on to the first statement of the next bar: the `var` line.
    rig.run(DebugCommand::StepBar);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(8));
    assert_eq!(
        rig.manager
            .debug
            .shown
            .as_ref()
            .unwrap()
            .snapshot
            .stop
            .bar_index,
        1
    );
}

#[test]
fn selecting_a_caller_frame_marks_its_line_and_shows_its_variables() {
    let mut rig = Rig::new("frame");
    rig.gutter(GutterEvent::Toggle { line: 5 });
    rig.run(DebugCommand::Continue);
    rig.paused();
    let caller = rig.manager.debug.stack().last().unwrap().index;
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::SelectFrame(caller)));
    rig.frame();
    rig.until("the caller's variables", |m| {
        m.debug.shown.as_ref().is_some_and(|s| {
            s.frame == caller
                && s.variables
                    .iter()
                    .any(|g| g.variables.iter().any(|v| v.name == "levels"))
        })
    });
    let marks = rig.manager.debug.marks();
    assert_eq!(marks.stop.map(|s| (s.line, s.innermost)), Some((9, false)));
    assert!(
        !marks.values.contains_key("doubled"),
        "a callee's local is not the caller's"
    );
}

#[test]
fn a_conditional_breakpoint_stops_only_when_it_holds() {
    let mut rig = Rig::new("condition");
    rig.gutter(GutterEvent::Toggle { line: 9 });
    let id = rig.manager.debug.state.breakpoints[0].spec.id;
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::EditBreakpoint {
            id,
            edit: super::debug::BreakpointEdit::Condition("bar_index == 7".to_owned()),
        }));
    rig.frame();
    assert_eq!(
        rig.manager.debug.marks().breakpoints[0].kind,
        MarkKind::Conditional
    );
    rig.run(DebugCommand::Continue);
    rig.paused();
    assert_eq!(
        rig.manager
            .debug
            .shown
            .as_ref()
            .unwrap()
            .snapshot
            .stop
            .bar_index,
        7
    );
}

#[test]
fn a_logpoint_writes_to_the_logs_tab_and_does_not_stop() {
    let mut rig = Rig::new("logpoint");
    rig.gutter(GutterEvent::AddLogpoint { line: 9 });
    let id = rig.manager.debug.state.breakpoints[0].spec.id;
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::EditBreakpoint {
            id,
            edit: super::debug::BreakpointEdit::LogMessage(Some(
                "bar {bar_index} value".to_owned(),
            )),
        }));
    rig.frame();
    assert_eq!(
        rig.manager.debug.marks().breakpoints[0].kind,
        MarkKind::Logpoint
    );
    rig.run(DebugCommand::Continue);
    rig.until("the run to end", |m| {
        m.debug.outcome == Some(Outcome::Completed)
    });
    assert_eq!(rig.manager.debug.logs.len(), 40);
    assert!(rig.manager.debug.has_logs());
    let first = rig.manager.debug.logs.buffer().iter().next().unwrap();
    assert_eq!(first.message, "bar 0 value");
    assert_eq!(first.range.start.line, 9);
}

#[test]
fn stepping_into_a_start_stops_on_the_first_statement() {
    let mut rig = Rig::new("entry");
    rig.run(DebugCommand::StepInto);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(8));
    assert_eq!(
        rig.manager
            .debug
            .shown
            .as_ref()
            .unwrap()
            .snapshot
            .stop
            .bar_index,
        0
    );
}

#[test]
fn watches_follow_the_stops_and_are_saved() {
    let mut rig = Rig::new("watches");
    rig.manager.cmds.push(Cmd::DebugPanel(PanelAction::AddWatch(
        "close - open".into(),
    )));
    rig.manager.cmds.push(Cmd::DebugPanel(PanelAction::AddWatch(
        "bar_index * 2".into(),
    )));
    rig.manager.cmds.push(Cmd::DebugPanel(PanelAction::AddWatch(
        "ta.sma(close, 3)".into(),
    )));
    rig.frame();
    rig.gutter(GutterEvent::Toggle { line: 9 });
    rig.run(DebugCommand::Continue);
    rig.paused();
    let rows = rig.manager.debug.shown.as_ref().unwrap().watches.clone();
    assert_eq!(rows.len(), 3);
    assert!(matches!(&rows[0].result, crate::pine::debug::Watched::Value(v) if v.value == "1"));
    assert!(matches!(&rows[1].result, crate::pine::debug::Watched::Value(v) if v.value == "0"));
    assert!(
        matches!(&rows[2].result, crate::pine::debug::Watched::Error(m) if m.contains("keeps state"))
    );
    // Removing one while stopped refreshes the rows.
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::RemoveWatch(0)));
    rig.frame();
    rig.until("the shorter list", |m| {
        m.debug.shown.as_ref().is_some_and(|s| s.watches.len() == 2)
    });
}

#[test]
fn breakpoints_and_watches_come_back_after_a_restart_of_the_app() {
    let mut rig = Rig::new("persist");
    rig.gutter(GutterEvent::Toggle { line: 5 });
    rig.gutter(GutterEvent::AddLogpoint { line: 10 });
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::AddWatch("close".into())));
    rig.frame();
    let id = rig.manager.open.as_ref().unwrap().id.clone();
    let saved = rig.manager.debug.state.clone();
    let rig = rig.restarted();
    assert_eq!(rig.manager.open.as_ref().unwrap().id, id);
    assert_eq!(rig.manager.debug.state.breakpoints, saved.breakpoints);
    assert_eq!(rig.manager.debug.state.watches, vec!["close".to_owned()]);
}

#[test]
fn breakpoints_follow_the_text_when_lines_are_added_above() {
    let mut rig = Rig::new("follow");
    rig.gutter(GutterEvent::Toggle { line: 9 });
    let edited = SOURCE.replacen("var levels", "// a note\n// another\nvar levels", 1);
    rig.set_source(&edited);
    rig.frame();
    assert_eq!(breakpoint_lines(&rig.manager), vec![11]);
    // Deleting the line leaves the breakpoint unresolved, not on the next statement.
    let without = edited.replacen("value = calculate(close)\n", "", 1);
    rig.set_source(&without);
    rig.frame();
    let stored = &rig.manager.debug.state.breakpoints[0];
    assert!(stored.unresolved);
    assert!(rig.manager.debug.marks().breakpoints.is_empty());
}

#[test]
fn editing_during_a_session_marks_it_out_of_date_until_restart() {
    let mut rig = Rig::new("stale");
    rig.gutter(GutterEvent::Toggle { line: 5 });
    rig.run(DebugCommand::Continue);
    rig.paused();
    let source = rig.manager.open.as_ref().unwrap().editor.source.clone();
    assert!(!rig.manager.debug.is_out_of_date(&source));
    let edited = SOURCE.replace("x * 2", "x * 3");
    rig.set_source(&edited);
    rig.frame();
    assert!(rig.manager.debug.is_out_of_date(&edited));
    assert_eq!(
        rig.stopped_line(),
        Some(5),
        "the session keeps the old revision"
    );
    rig.run(DebugCommand::Restart);
    rig.paused();
    assert!(!rig.manager.debug.is_out_of_date(&edited));
    let marks = rig.manager.debug.marks();
    let (value, _) = &marks.values["doubled"];
    assert_eq!(value, "303", "the restart runs the new code");
}

#[test]
fn a_script_with_errors_or_a_library_is_not_started_and_says_why() {
    let mut rig = Rig::new("refused");
    rig.set_source("//@version=6\nindicator(\"T\")\nx = (\n");
    rig.run(DebugCommand::Continue);
    let notice = rig.manager.debug.notice.clone().expect("a notice");
    assert!(notice.error && notice.text.contains("errors"), "{notice:?}");
    assert!(!rig.manager.debug.has_session());
}

#[test]
fn a_chart_with_no_bars_cannot_be_debugged_yet() {
    let mut rig = Rig::new("nobars");
    rig.manager.preview.set_candles_for_test(Vec::new());
    rig.run(DebugCommand::Continue);
    let notice = rig.manager.debug.notice.clone().expect("a notice");
    assert!(notice.text.contains("no bars"), "{notice:?}");
}

#[test]
fn a_runtime_error_stops_with_its_location_and_cannot_be_continued() {
    let mut rig = Rig::new("error");
    rig.set_source("//@version=6\nindicator(\"T\")\narr = array.new_float(1)\nx = array.get(arr, 5)\nplot(x)\n");
    rig.run(DebugCommand::Continue);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(4));
    assert!(rig.manager.debug.paused_on_error());
    assert!(!rig.manager.debug.allows(DebugCommand::Continue));
    assert!(!rig.manager.debug.allows(DebugCommand::StepOver));
    assert!(rig.manager.debug.allows(DebugCommand::Stop));
    assert!(rig.manager.debug.allows(DebugCommand::Restart));
    let marks = rig.manager.debug.marks();
    assert!(marks.stop.is_some_and(|stop| stop.error));
    assert!(rig.manager.debug.failure().is_some());
    rig.run(DebugCommand::Stop);
    rig.until("the failure", |m| {
        matches!(m.debug.outcome, Some(Outcome::Failed(_)))
    });
}

#[test]
fn the_shortcuts_drive_the_session() {
    let mut rig = Rig::new("keys");
    rig.gutter(GutterEvent::Toggle { line: 9 });
    let (over, into) = if shortcut::is_mac() {
        (egui::Key::F6, egui::Key::F7)
    } else {
        (egui::Key::F10, egui::Key::F11)
    };
    // Step Into with no session starts one on the first statement.
    rig.press(into);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(8));
    rig.press(over);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(9));
    rig.press(into);
    rig.paused();
    assert_eq!(rig.stopped_line(), Some(4));
}

#[test]
fn switching_scripts_ends_the_session_and_deleting_one_forgets_its_state() {
    let mut rig = Rig::new("switch");
    rig.gutter(GutterEvent::Toggle { line: 5 });
    rig.run(DebugCommand::Continue);
    rig.paused();
    let alive = rig.manager.debug.state_of_session();
    assert_eq!(alive, DebugState::Paused);
    let first = rig.manager.open.as_ref().unwrap().id.clone();
    let other = rig.manager.scripts[1].id.clone();
    rig.manager.open_script(&other);
    rig.frame();
    assert!(!rig.manager.debug.has_session());
    assert!(
        rig.manager.debug.state.breakpoints.is_empty(),
        "each script has its own"
    );
    rig.manager.open_script(&first);
    rig.frame();
    assert_eq!(breakpoint_lines(&rig.manager), vec![5]);
    rig.manager.delete_script(&first);
    let reloaded = super::debug::DebugPanel::load(&rig.shared);
    assert!(reloaded.state.breakpoints.is_empty());
    let states: Option<super::debug_state::DebugStates> =
        rig.shared.lock().unwrap().setting(super::debug_state::KEY);
    assert!(states.is_none_or(|s| s.get(&first).is_empty()));
}

#[test]
fn closing_the_window_ends_the_session_thread() {
    let mut rig = Rig::new("close");
    rig.gutter(GutterEvent::Toggle { line: 5 });
    rig.run(DebugCommand::Continue);
    rig.paused();
    let alive = rig.manager.debug.liveness().expect("a session");
    assert!(alive.strong_count() > 0);
    let Rig { manager, .. } = &mut rig;
    manager.debug.end_session();
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive.strong_count() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(alive.strong_count(), 0, "the thread outlived its session");
}

#[test]
fn bar_breaks_take_an_index_or_a_time_and_report_a_bad_one() {
    let mut rig = Rig::new("barbreak");
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::BreakOnBar("nonsense".into())));
    rig.frame();
    assert!(rig.manager.debug.notice.as_ref().is_some_and(|n| n.error));
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::BreakOnBar("12".into())));
    rig.frame();
    assert_eq!(rig.manager.debug.bar_break.as_deref(), Some("12"));
    rig.run(DebugCommand::Continue);
    rig.paused();
    assert_eq!(
        rig.manager
            .debug
            .shown
            .as_ref()
            .unwrap()
            .snapshot
            .stop
            .bar_index,
        12
    );
}

#[test]
fn the_arrow_keys_walk_the_variables_and_open_and_close_rows() {
    let mut rig = Rig::new("arrows");
    rig.manager.prefs.debug_open = true;
    rig.gutter(GutterEvent::Toggle { line: 10 });
    // Stop once the array has something in it.
    let id = rig.manager.debug.state.breakpoints[0].spec.id;
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::EditBreakpoint {
            id,
            edit: super::debug::BreakpointEdit::Condition("bar_index == 3".into()),
        }));
    rig.frame();
    rig.run(DebugCommand::Continue);
    rig.paused();
    rig.frame();
    let list = egui::Id::new("script-debug-variables-list");
    let cursor = |rig: &Rig| rig.manager.debug.ui.cursor.clone();
    // Picking a row gives the list the keyboard.
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::Cursor("levels".into())));
    rig.frame();
    rig.ctx.memory_mut(|memory| memory.request_focus(list));
    rig.frame();
    assert_eq!(cursor(&rig).as_deref(), Some("levels"));
    // Right opens the array; its rows load and Right again steps into them.
    rig.press(egui::Key::ArrowRight);
    rig.until("the array's rows", |m| {
        m.debug.shown.as_ref().is_some_and(|s| {
            s.open.contains("levels") && s.loaded.values().any(|l| !l.items.is_empty())
        })
    });
    rig.press(egui::Key::ArrowRight);
    assert_eq!(cursor(&rig).as_deref(), Some("levels/[0]"));
    // Left from a row with no parts goes to its parent; Left again closes the parent.
    rig.press(egui::Key::ArrowLeft);
    assert_eq!(cursor(&rig).as_deref(), Some("levels"));
    rig.press(egui::Key::ArrowLeft);
    assert!(
        !rig.manager
            .debug
            .shown
            .as_ref()
            .unwrap()
            .open
            .contains("levels")
    );
    // Down moves on to the next variable, Up back, Home and End go to the ends.
    rig.press(egui::Key::ArrowDown);
    let next = cursor(&rig);
    assert_ne!(next.as_deref(), Some("levels"));
    rig.press(egui::Key::ArrowUp);
    assert_eq!(cursor(&rig).as_deref(), Some("levels"));
    rig.press(egui::Key::End);
    let last = cursor(&rig);
    rig.press(egui::Key::Home);
    assert_ne!(cursor(&rig), last);
}

#[test]
fn the_variables_list_scrolls_over_exactly_its_rows() {
    let mut rig = Rig::new("extent");
    rig.manager.prefs.debug_open = true;
    rig.gutter(GutterEvent::Toggle { line: 10 });
    rig.run(DebugCommand::Continue);
    rig.paused();
    rig.frame();
    // Rows are laid out edge to edge; a list that counted spacing between them would reach below
    // its last row, and a scroll bar would end on blank space.
    let extent = rig.manager.debug.ui.list_extent;
    let rows = extent / 22.0; // a variables row is 22 points
    assert!(rows >= 1.0, "the list measured {extent}");
    assert!(
        (rows - rows.round()).abs() < 0.01,
        "{extent} is not a whole number of rows"
    );
}

#[test]
fn select_bar_waits_for_a_candle_and_arms_the_stop_at_it() {
    let mut rig = Rig::new("pick");
    rig.manager.prefs.chart_visible = false;
    rig.manager.cmds.push(Cmd::DebugPanel(PanelAction::PickBar));
    rig.frame();
    assert!(rig.manager.preview.is_picking_bar());
    assert!(rig.manager.debug.picking_bar, "the panel is told");
    assert!(
        rig.manager.prefs.chart_visible,
        "the chart has to be there to click"
    );
    // The same button again cancels.
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::CancelPickBar));
    rig.frame();
    assert!(!rig.manager.preview.is_picking_bar() && !rig.manager.debug.picking_bar);
    assert!(rig.manager.debug.bar_break.is_none());

    // A click on the sixth candle of the preview.
    let sixth = 1_700_000_000_000 + 5 * 3_600_000;
    rig.manager.cmds.push(Cmd::DebugPanel(PanelAction::PickBar));
    rig.frame();
    rig.manager.preview.pick_for_test(sixth);
    rig.frame();
    assert!(!rig.manager.preview.is_picking_bar());
    assert_eq!(
        rig.manager.debug.bar_break.as_deref(),
        Some(debug_panel::bar_time_text(sixth).as_str())
    );
    rig.run(DebugCommand::Continue);
    rig.paused();
    let stop = &rig.manager.debug.shown.as_ref().unwrap().snapshot.stop;
    assert_eq!(stop.bar_index, 5);
    assert_eq!(stop.reason, crate::pine::debug::StopReason::Bar);
}

#[test]
fn the_debug_panel_draws_in_every_state_without_panicking() {
    let mut rig = Rig::new("draw");
    rig.manager.prefs.debug_open = true;
    rig.frame();
    rig.gutter(GutterEvent::Toggle { line: 5 });
    rig.gutter(GutterEvent::AddLogpoint { line: 10 });
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::AddWatch("close".into())));
    rig.frame();
    for position in prefs::ChartPosition::ALL {
        rig.manager.prefs.chart_position = position;
        rig.frame();
    }
    rig.run(DebugCommand::Continue);
    rig.paused();
    rig.manager
        .cmds
        .push(Cmd::DebugPanel(PanelAction::Evaluate("close[1]".into())));
    rig.frame();
    rig.until("the console answer", |m| {
        m.debug
            .console
            .last()
            .is_some_and(|line| line.result.is_some())
    });
    rig.run(DebugCommand::Stop);
    rig.until("the end", |m| {
        m.debug.state_of_session() == DebugState::Completed
    });
    rig.frame();
    let _ = StartMode::Run;
}
