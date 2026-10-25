//! The Script Manager's side of the debugger: starting a session on the open script, routing the
//! toolbar, the shortcuts, the gutter and the panel to it, and keeping the breakpoints' map of the
//! script current.

use std::sync::Arc;

use eframe::egui;

use super::debug::{Notice, PanelAction, StartMode};
use super::debug_marks::GutterEvent;
use super::model::{CompileStatus, ScriptType};
use super::registry::LibraryRegistry;
use super::{Cmd, ScriptManager};
use crate::pine::compiler;
use crate::pine::debug::{DebugCommand, SourceMap};
use crate::pine::token::Limits;
use crate::ui::shortcut;
use crate::ui::theme;

impl ScriptManager {
    /// Once a frame, before anything is drawn: breakpoints follow the text, and the session's
    /// events are read.
    pub(super) fn debug_frame(&mut self, ctx: &egui::Context) {
        if let Some(open) = self.open.as_ref() {
            self.debug.observe_text(&open.editor.source);
        }
        if self.debug.poll() {
            ctx.request_repaint();
        }
        if let Some(line) = self.debug.take_stop_to_reveal()
            && let Some(open) = self.open.as_mut()
        {
            open.editor.reveal_stop_line(line);
        }
        self.debug.picking_bar = self.preview.is_picking_bar();
        let bars = self.preview.bar_key();
        if self.debug.bars_key != bars {
            self.debug.set_bars(self.preview.bar_open_times(), bars);
        }
        if let Some(picked) = self.preview.take_picked_bar() {
            self.debug
                .apply(PanelAction::BreakOnBar(super::debug_panel::bar_time_text(
                    picked,
                )));
        }
        if let Some(wait) = self.debug.wants_frames() {
            ctx.request_repaint_after(wait);
        }
    }

    /// Compiles the editor's text for the map that says which lines a breakpoint can stop at.
    pub(super) fn refresh_debug_map(&mut self) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let program = compiler::compile(&open.editor.source, Limits::default(), &LibraryRegistry);
        self.debug.set_map(
            program
                .is_valid()
                .then(|| SourceMap::build(&program.statements)),
        );
    }

    /// The debugger's shortcuts. Nothing here is typed text, so they work with the editor focused.
    pub(super) fn debug_shortcuts(&mut self, ctx: &egui::Context) {
        let pressed = ctx.input(|input| {
            [
                (shortcut::DEBUG_CONTINUE, DebugCommand::Continue),
                (shortcut::DEBUG_PAUSE, DebugCommand::Pause),
                (shortcut::DEBUG_STEP_OVER, DebugCommand::StepOver),
                (shortcut::DEBUG_STEP_INTO, DebugCommand::StepInto),
                (shortcut::DEBUG_STEP_OUT, DebugCommand::StepOut),
                (shortcut::DEBUG_STEP_BAR, DebugCommand::StepBar),
                (shortcut::DEBUG_STOP, DebugCommand::Stop),
                (shortcut::DEBUG_RESTART, DebugCommand::Restart),
            ]
            .into_iter()
            .filter(|(chord, _)| chord.peek(input))
            .map(|(_, command)| command)
            .collect::<Vec<_>>()
        });
        if self.open.is_none() {
            return;
        }
        for command in pressed {
            if self.debug.allows(command) {
                self.cmds.push(Cmd::Debug(command));
            }
        }
        if ctx.input(|input| shortcut::DEBUG_TOGGLE_BREAKPOINT.peek(input))
            && let Some(line) = self.open.as_ref().and_then(|open| open.editor.caret_line())
        {
            self.cmds.push(Cmd::Gutter(GutterEvent::Toggle { line }));
        }
    }

    pub(super) fn debug_command(&mut self, command: DebugCommand, ctx: &egui::Context) {
        match command {
            DebugCommand::Restart => self.debug_start(StartMode::Run, ctx),
            DebugCommand::Continue if self.debug.would_start() => {
                self.debug_start(StartMode::Run, ctx);
            }
            DebugCommand::StepInto | DebugCommand::StepOver if self.debug.would_start() => {
                self.debug_start(StartMode::Entry, ctx);
            }
            other => {
                self.debug.command(other);
            }
        }
    }

    /// Starts a session on the text in the editor and the preview chart's bars.
    pub(super) fn debug_start(&mut self, mode: StartMode, ctx: &egui::Context) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let refusal = if open.script_type == ScriptType::Library {
            Some("A library exports functions; there is nothing to run.")
        } else if open.editor.status == CompileStatus::Error {
            Some("The script has errors. Fix them to debug it.")
        } else {
            None
        };
        if let Some(text) = refusal {
            self.debug.notice = Some(Notice {
                text: text.to_owned(),
                error: true,
            });
            self.prefs.debug_open = true;
            return;
        }
        let source = open.editor.source.clone();
        let Some(feed) = self
            .preview
            .debug_feed(super::model::now_ms(), theme::is_light())
        else {
            self.debug.notice = Some(Notice {
                text: "The preview chart has no bars yet. Wait for it to load.".to_owned(),
                error: true,
            });
            self.prefs.debug_open = true;
            return;
        };
        self.refresh_debug_map();
        let ctx = ctx.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        self.debug
            .start(&source, feed, mode, Box::new(LibraryRegistry), wake);
        self.prefs.debug_open = true;
    }

    pub(super) fn debug_action(&mut self, action: PanelAction, ctx: &egui::Context) {
        match action {
            PanelAction::Command(command) => self.debug_command(command, ctx),
            PanelAction::Close => self.prefs.debug_open = false,
            PanelAction::PickBar => {
                // The chart has to be there to be clicked.
                self.prefs.chart_visible = true;
                self.preview.begin_bar_pick();
                self.debug.picking_bar = true;
                ctx.request_repaint();
            }
            PanelAction::CancelPickBar => {
                self.preview.cancel_bar_pick();
                self.debug.picking_bar = false;
            }
            other => {
                if let Some(line) = self.debug.apply(other)
                    && let Some(open) = self.open.as_mut()
                {
                    open.editor.show_line(line);
                    ctx.request_repaint();
                }
            }
        }
    }

    pub(super) fn debug_gutter(&mut self, event: GutterEvent) {
        if self.debug.map().is_none() {
            self.refresh_debug_map();
        }
        if self.debug.gutter(event) {
            self.prefs.debug_open = true;
            self.prefs.debug_sections |= super::debug::Section::Breakpoints as u8;
        }
    }
}
