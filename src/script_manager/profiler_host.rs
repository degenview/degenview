//! The Script Manager's side of the profiler: the key a profile has to match, starting a run on the
//! open script and the preview chart's bars, and routing the toolbar, the drawer and the keyboard
//! to the [`Profiler`](super::profiler::Profiler).

use std::sync::Arc;

use eframe::egui;

use super::model::{CompileStatus, ScriptType};
use super::profile_ui::Action;
use super::profiler::ProfileKey;
use super::registry::LibraryRegistry;
use super::{Cmd, ScriptManager};
use crate::pine::profile::ProfileConfig;
use crate::ui::shortcut;
use crate::ui::theme;

impl From<Action> for Cmd {
    fn from(action: Action) -> Self {
        match action {
            Action::Toggle => Self::ProfilerToggle,
            Action::Run => Self::ProfilerRun,
            Action::Cancel => Self::ProfilerCancel,
            Action::Reveal(line) => Self::RevealLine(line),
        }
    }
}

impl ScriptManager {
    /// What a profile of the open script would have to be of to be current, now.
    fn profiler_key(&self) -> Option<ProfileKey> {
        let open = self.open.as_ref()?;
        let context = self.preview.run_context();
        Some(ProfileKey::new(
            &open.id,
            &open.editor.source,
            context.inputs,
            &context.chart,
            context.bars,
        ))
    }

    /// Once a frame, before anything is drawn: the key is brought up to date (only while the
    /// profiler is on, so an unused profiler hashes nothing), a finished run is read, and a run that
    /// should start by itself does.
    pub(super) fn profiler_frame(&mut self, ctx: &egui::Context) {
        if !self.profiler.is_on() {
            self.profiler_now = None;
            return;
        }
        self.profiler_now = self.profiler_key();
        if let Some(key) = &self.profiler_now {
            self.profiler.text_changed(key);
        }
        if self.profiler.poll() {
            ctx.request_repaint();
        }
        if self.profiler.wants_start() && self.open.is_some() {
            self.profiler_start(ctx);
        }
        if let Some(wait) = self.profiler.wants_frames() {
            ctx.request_repaint_after(wait);
        }
    }

    pub(super) fn profiler_shortcuts(&mut self, ctx: &egui::Context) {
        if self.open.is_some() && ctx.input(|input| shortcut::PROFILER_TOGGLE.peek(input)) {
            self.cmds.push(Cmd::ProfilerToggle);
        }
    }

    pub(super) fn profiler_toggle(&mut self) {
        if self.profiler.is_on() {
            self.profiler.disable();
            self.profiler_now = None;
        } else if self.open.is_some() {
            self.profiler.enable();
            // The drawer shows the result; the person asked to see it.
            self.prefs.drawer_tab = super::prefs::DrawerTab::Profile;
            self.prefs.drawer_open = true;
        }
    }

    /// Profiles the text in the editor on the preview chart's bars, ending a run in flight. The
    /// run is a thread of its own over a frozen copy of the bars: nothing it does reaches the
    /// preview, a chart, an alert or an order.
    pub(super) fn profiler_start(&mut self, ctx: &egui::Context) {
        if !self.profiler.is_on() {
            return;
        }
        let Some(key) = self.profiler_key() else {
            return;
        };
        let Some(open) = self.open.as_ref() else {
            return;
        };
        if open.script_type == ScriptType::Library {
            self.profiler.refuse(
                key,
                "A library exports functions; there is nothing to run.",
                true,
            );
            return;
        }
        if open.editor.compile_pending() {
            self.profiler.wait("Waiting for the script to compile.");
            return;
        }
        if open.editor.status == CompileStatus::Error {
            self.profiler
                .refuse(key, "The script has errors. Fix them to profile it.", true);
            return;
        }
        let source = open.editor.source.clone();
        let Some(feed) = self
            .preview
            .debug_feed(super::model::now_ms(), theme::is_light())
        else {
            self.profiler
                .wait("The preview chart has no bars yet. Wait for it to load.");
            return;
        };
        let mut config = ProfileConfig::new(source.clone(), feed.symbol, feed.bars, feed.now_ms);
        config.inputs = feed.inputs;
        config.theme = Some(feed.theme);
        config.libraries = Box::new(LibraryRegistry);
        config.security_data = Some(Box::new(crate::host::pine_host::KlineSecurityDataProvider));
        let ctx = ctx.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        // A refusal to start is kept by the profiler and shown in its place.
        let _ = self.profiler.begin(key, &source, feed.label, config, wake);
    }
}
