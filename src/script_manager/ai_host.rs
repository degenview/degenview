//! Connects the assistant to the Script Manager: builds a request from the open script, applies an
//! accepted proposal to the editor, and follows the open script around. The state and the rules
//! about replies live in `ai`; the words sent to the model in `ai_prompt`.

use super::ai::Notice;
use super::ai_panel::{PanelAction, Setup};
use super::ai_prompt::{self, IMPLEMENT_MESSAGE, MAX_SOURCE_CHARS, Mode};
use super::registry::LibraryRegistry;
use super::{ScriptManager, ScriptManagerAction, editor};
use crate::pine::compiler;
use crate::pine::token::{Diagnostic, Limits};
use crate::service::keychain_ai_keys::{self, AiKeyStore};
use crate::service::llm::prefs::AiPrefs;
use crate::service::llm::{ChatRequest, HttpProvider, ProviderKind};
use eframe::egui;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Room for a whole script and for a reasoning model's thinking, which counts against the limit.
const MAX_REPLY_TOKENS: u32 = 16_000;
/// How often the saved provider and model are read again while the panel is open, so a change
/// made in Settings shows without reopening it.
const PREFS_REFRESH: Duration = Duration::from_millis(1500);

fn diagnostics_of(script: &str) -> Vec<Diagnostic> {
    compiler::compile(script, Limits::default(), &LibraryRegistry).diagnostics
}

fn key_available(kind: ProviderKind) -> bool {
    AiKeyStore::shared().is_configured(kind)
        || std::env::var(kind.env_var()).is_ok_and(|value| !value.trim().is_empty())
}

impl ScriptManager {
    /// Once a frame: a different open script starts a new conversation, and whatever the model has
    /// written since the last frame is taken in.
    pub(super) fn ai_begin_frame(&mut self) {
        let open_id = self.open.as_ref().map(|open| open.id.clone());
        if self.ai.script_id != open_id {
            self.ai.reset(open_id);
        }
        if let Some(finished) = self.ai.poll() {
            self.ai.finish(finished, &diagnostics_of);
        }
        let stale = self
            .ai
            .prefs_loaded_at
            .is_none_or(|at| at.elapsed() > PREFS_REFRESH);
        if self.prefs.ai_open && stale {
            self.ai_reload_prefs();
        }
    }

    fn ai_reload_prefs(&mut self) {
        if let Ok(store) = self.shared.lock() {
            self.ai.prefs = AiPrefs::load(&store);
        }
        self.ai.prefs_loaded_at = Some(Instant::now());
    }

    /// What the panel shows about the provider, and why a request cannot be made yet.
    pub(super) fn ai_setup(&self) -> Setup {
        let prefs = &self.ai.prefs;
        let kind = prefs.provider;
        let model = prefs.model(kind);
        let problem = if kind.needs_key() && !key_available(kind) {
            Some(format!("Add your {} API key to start", kind.title()))
        } else if kind == ProviderKind::Compatible && prefs.base_url(kind).is_empty() {
            Some("Enter your server's address to start".to_owned())
        } else if model.is_empty() {
            Some("Choose a model to start".to_owned())
        } else {
            None
        };
        Setup {
            provider: kind.title().to_owned(),
            model,
            problem,
        }
    }

    pub(super) fn ai_toggle_panel(&mut self) {
        self.prefs.ai_open = !self.prefs.ai_open;
        if self.prefs.ai_open {
            self.ai_reload_prefs();
            self.ai.focus_input = true;
        }
    }

    pub(super) fn ai_action(&mut self, action: PanelAction, ctx: &egui::Context) {
        match action {
            PanelAction::Send => self.ai_send(ctx),
            PanelAction::Stop => self.ai.stop(),
            PanelAction::Accept => self.ai_accept(ctx),
            PanelAction::Discard => self.ai.proposal = None,
            PanelAction::Clear => self.ai.clear(),
            PanelAction::SetMode(mode) => {
                self.ai.set_mode(mode);
            }
            PanelAction::Implement => {
                if self.ai.can_implement() && self.ai.set_mode(Mode::Edit) {
                    self.ai.input = IMPLEMENT_MESSAGE.to_owned();
                    self.ai_send(ctx);
                }
            }
            PanelAction::OpenSettings => self.actions.push(ScriptManagerAction::OpenAiSettings),
            PanelAction::Close => self.prefs.ai_open = false,
        }
    }

    /// Sends the message in the box with the open script. Nothing in the editor changes.
    fn ai_send(&mut self, ctx: &egui::Context) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let typed = self.ai.input.trim().to_owned();
        if typed.is_empty() || self.ai.is_running() {
            return;
        }
        let source = open.editor.source.clone();
        if source.chars().count() > MAX_SOURCE_CHARS {
            self.ai.notice = Some(Notice::error(
                "This script is too long to send to the assistant.",
            ));
            return;
        }
        self.ai_reload_prefs();
        let kind = self.ai.prefs.provider;
        // Reading the key is where the Keychain may ask; the user just pressed Send.
        let provider = match HttpProvider::new(
            kind,
            self.ai.prefs.base_url(kind),
            keychain_ai_keys::key(kind),
        ) {
            Ok(provider) => provider,
            Err(error) => {
                self.ai.notice = Some(Notice::error(error.to_string()));
                return;
            }
        };
        // Compiled now, not read from the editor, which may still be waiting out its typing pause.
        let diagnostics = diagnostics_of(&source);
        let request = ChatRequest {
            model: self.ai.prefs.model(kind),
            system: ai_prompt::system_prompt(self.ai.mode),
            messages: ai_prompt::build_messages(&self.ai.turns, &source, &typed, &diagnostics),
            max_tokens: MAX_REPLY_TOKENS,
        };
        let repaint = ctx.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || repaint.request_repaint());
        let mode = self.ai.mode;
        self.ai
            .start(Box::new(provider), request, typed, source, mode, wake);
    }

    /// Puts the proposed script in the editor as one undo step, unless the editor has moved on
    /// from the script the model was shown.
    fn ai_accept(&mut self, ctx: &egui::Context) {
        // A proposal only exists after an Edit request, and the mode cannot change while one waits;
        // this guard makes "Plan never changes the script" true even if that ever slips.
        if self.ai.mode != Mode::Edit {
            return;
        }
        let (Some(proposal), Some(open)) = (self.ai.proposal.as_ref(), self.open.as_mut()) else {
            return;
        };
        if open.editor.source != proposal.base {
            self.ai.proposal = None;
            self.ai.notice = Some(Notice::error(
                "The script changed while the assistant was writing. Ask again to base the change on the current text.",
            ));
            return;
        }
        editor::apply_script(ctx, &mut open.editor, &proposal.script);
        open.editor.request_compile();
        self.ai.proposal = None;
        self.ai.notice = Some(Notice::info("Applied. Undo brings the old script back."));
    }
}
