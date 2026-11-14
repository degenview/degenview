//! The assistant's state: the conversation about the open script, the request in flight, and the
//! change it proposed. No egui and no store; `ai::host` connects it to the Pine Studio and
//! `ai::panel` draws it.
//!
//! A reply streams on its own thread and arrives over a channel, so the UI never waits on the
//! network. Nothing here changes the script: a finished reply becomes a [`Proposal`] that the user
//! accepts or discards, and accepting goes through the editor as one undo step.

pub(in crate::pine_studio) mod agent;
#[cfg(test)]
pub(in crate::pine_studio) mod gallery;
pub mod history;
pub(in crate::pine_studio) mod host;
pub(in crate::pine_studio) mod models;
pub(in crate::pine_studio) mod panel;
pub(in crate::pine_studio) mod plan;
pub(in crate::pine_studio) mod profile;
pub(in crate::pine_studio) mod prompt;
pub(in crate::pine_studio) mod review;
pub(in crate::pine_studio) mod review_view;
#[cfg(test)]
pub(in crate::pine_studio) mod studio_tests;
pub(in crate::pine_studio) mod tools;

use super::editor::diff::DiffSnapshot;
use crate::model::pine::script::ScriptId;
use crate::pine::syntax::token::{Diagnostic, DiagnosticSeverity};
use crate::service::llm::prefs::AiPrefs;
use crate::service::llm::report::ErrorReport;
use crate::service::llm::{ChatRequest, LlmError, LlmProvider, ProviderKind};
use agent::{AgentSetup, Ended, Progress};
use models::ModelList;
use plan::Plan;
use prompt::{Mode, Turn};
use review::Review;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;
use tools::Draft;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
    /// What the notice says; for a provider's failure, its headline.
    pub text: String,
    pub error: bool,
    /// A provider's failure, with its detail and request id.
    pub report: Option<ErrorReport>,
}

impl Notice {
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: true,
            report: None,
        }
    }

    /// An error from a provider, whole.
    pub fn failure(report: ErrorReport) -> Self {
        Self {
            text: report.headline.clone(),
            error: true,
            report: Some(report),
        }
    }

    /// The error as a report: the provider's own, or a plain message of ours.
    pub fn error_report(&self) -> ErrorReport {
        self.report
            .clone()
            .unwrap_or_else(|| ErrorReport::message(self.text.clone()))
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
            report: None,
        }
    }
}

/// A whole script the model wrote, waiting for the user.
#[derive(Clone, Debug)]
pub struct Proposal {
    /// The script as it was when the request was made. Accepting is refused if the editor no
    /// longer holds exactly this, so a change is never applied over typing it knew nothing of.
    pub base: String,
    pub script: String,
    pub changes: DiffSnapshot,
    pub errors: usize,
    pub warnings: usize,
    /// Set when the work is not all of what was asked: the assistant stopped, ran out of steps or
    /// failed part-way. The changes are still reviewed before anything is applied.
    pub note: Option<String>,
}

/// One tool call in the transcript: what it is doing, and what it came to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolStep {
    pub id: String,
    pub label: String,
    pub outcome: Option<StepOutcome>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepOutcome {
    pub summary: String,
    pub ok: bool,
    /// What the model was told.
    pub detail: String,
}

/// What happened between a message and its reply, in order. Shown for the latest exchange only and
/// never saved: history keeps the words.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Entry {
    /// What the model said before calling tools.
    Said(String),
    Tool(ToolStep),
}

enum Event {
    Progress(Progress),
    Done(Result<Ended, LlmError>),
}

/// The answer to a model-list request: the ids, or why they could not be had.
type ModelsReceiver = Receiver<Result<Vec<String>, ErrorReport>>;

/// A request in flight. Dropping it asks the thread to stop.
struct Run {
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    base: String,
    typed: String,
    mode: Mode,
    /// The assistant's working copy, if it has tools; read when the request is stopped.
    draft: Option<Draft>,
    /// Auto was on when the request was sent. Flipping the switch while it runs changes nothing.
    auto: bool,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// How a request ended.
pub struct Finished {
    result: Result<Ended, LlmError>,
    base: String,
    typed: String,
    mode: Mode,
    draft: Option<Draft>,
    auto: bool,
}

#[derive(Default)]
pub struct Assistant {
    /// The script this conversation is about; a different open script starts a new one.
    pub script_id: Option<ScriptId>,
    pub turns: Vec<Turn>,
    /// Whether a request may propose a change. Ask, until the user switches to Edit.
    pub mode: Mode,
    /// The message being typed.
    pub input: String,
    /// The reply so far, while one streams.
    pub streaming: String,
    /// The tools the latest request called, and what the model said between them.
    pub steps: Vec<Entry>,
    pub proposal: Option<Proposal>,
    /// The request that made `proposal` was sent with Auto on, so the host may apply it.
    pub auto_requested: bool,
    /// The plan waiting for the user to approve, revise or discard it. Like a proposal it is kept in
    /// memory only; the plan's words are in the conversation either way.
    pub plan: Option<Plan>,
    /// The review sheet over `proposal`, while it is open.
    pub review: Option<Review>,
    pub notice: Option<Notice>,
    /// The input box takes the keyboard on the next frame.
    pub focus_input: bool,
    /// The transcript scrolls to its end on the next frame (a request started or ended).
    pub scroll_to_end: bool,
    /// The provider and model, read from the store while the panel is open.
    pub prefs: AiPrefs,
    pub prefs_loaded_at: Option<Instant>,
    /// The models each provider's key can use, filled when the model menu opens.
    pub models: HashMap<ProviderKind, ModelList>,
    /// What is typed in the model menu's filter.
    pub model_filter: String,
    models_rx: Option<(ProviderKind, ModelsReceiver)>,
    run: Option<Run>,
}

impl Assistant {
    pub fn is_running(&self) -> bool {
        self.run.is_some()
    }

    /// What the user sent, while its reply is still coming. It joins the history only when the
    /// reply ends, but it has to be on screen from the moment it is sent.
    pub fn pending_request(&self) -> Option<&str> {
        self.run.as_ref().map(|run| run.typed.as_str())
    }

    /// Starts a new conversation about another script: the old one is forgotten, any request
    /// stops, and the mode goes back to Ask. The typed message stays.
    pub fn reset(&mut self, script_id: Option<ScriptId>) {
        self.script_id = script_id;
        self.mode = Mode::Ask;
        self.clear();
    }

    /// Forgets this conversation and stops any request, keeping the script and the mode.
    pub fn clear(&mut self) {
        self.run = None;
        self.turns.clear();
        self.streaming.clear();
        self.steps.clear();
        self.proposal = None;
        self.plan = None;
        self.review = None;
        self.notice = None;
    }

    /// Switches mode. Refused while a request is running or a proposal or plan waits, so Ask can
    /// never sit beside an Apply button.
    pub fn set_mode(&mut self, mode: Mode) -> bool {
        if self.run.is_some() || self.proposal.is_some() || self.plan.is_some() {
            return false;
        }
        self.mode = mode;
        true
    }

    /// Takes back the last exchange so it can be sent again or edited: its message is returned, the
    /// reply and any proposal over it are dropped, and the mode returns to the one it was made in.
    /// Only the last exchange, and never while a request runs.
    pub fn rewind_last(&mut self) -> Option<String> {
        if self.run.is_some() {
            return None;
        }
        let at = match self.turns.as_slice() {
            [.., Turn::User(_), Turn::Assistant { .. }] => self.turns.len() - 2,
            [.., Turn::User(_)] => self.turns.len() - 1,
            _ => return None,
        };
        let mode = match self.turns.last() {
            Some(Turn::Assistant { mode, .. }) => *mode,
            _ => self.mode,
        };
        let Turn::User(text) = self.turns[at].clone() else {
            return None;
        };
        self.turns.truncate(at);
        self.steps.clear();
        self.proposal = None;
        self.plan = None;
        self.review = None;
        self.notice = None;
        self.mode = mode;
        Some(text)
    }

    /// Starts asking `provider` for the models its key can use.
    pub fn begin_model_fetch(
        &mut self,
        kind: ProviderKind,
        provider: Box<dyn LlmProvider>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        self.models.insert(kind, ModelList::Loading);
        self.models_rx = Some((kind, models::fetch(provider, wake)));
    }

    /// Takes in a model list that has arrived.
    pub fn poll_models(&mut self) {
        let Some((kind, receiver)) = &self.models_rx else {
            return;
        };
        let kind = *kind;
        let list = match receiver.try_recv() {
            Ok(Ok(ids)) => ModelList::Ready(models::usable_chat_models(kind, &ids)),
            Ok(Err(report)) => ModelList::Failed(report),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                ModelList::Failed(ErrorReport::message("The model list stopped unexpectedly."))
            }
        };
        self.models.insert(kind, list);
        self.models_rx = None;
    }

    /// Whether the last reply is a plan that the Implement button can carry out.
    pub fn can_implement(&self) -> bool {
        self.run.is_none()
            && matches!(
                self.turns.last(),
                Some(Turn::Assistant { text, mode: Mode::Ask }) if prompt::looks_like_plan(text)
            )
    }

    /// Starts a request on its own thread. `wake` repaints the UI when text arrives.
    #[cfg(test)]
    pub fn start(
        &mut self,
        provider: Box<dyn LlmProvider>,
        request: ChatRequest,
        typed: String,
        base: String,
        mode: Mode,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        self.start_agent(provider, request, None, typed, base, mode, wake);
    }

    /// Starts a request that may call tools (`setup`), or an ordinary one (`None`).
    #[allow(clippy::too_many_arguments)]
    pub fn start_agent(
        &mut self,
        provider: Box<dyn LlmProvider>,
        request: ChatRequest,
        setup: Option<AgentSetup>,
        typed: String,
        base: String,
        mode: Mode,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        let (sender, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let draft = setup.as_ref().map(|setup| setup.tools.draft().clone());
        let spawned = std::thread::Builder::new()
            .name("ai-assistant".to_owned())
            .spawn(move || {
                let progress = sender.clone();
                let progress_wake = wake.clone();
                let result = agent::run(&*provider, request, setup.as_ref(), &flag, &mut |piece| {
                    let _ = progress.send(Event::Progress(piece));
                    progress_wake();
                });
                let _ = sender.send(Event::Done(result));
                wake();
            });
        if spawned.is_err() {
            self.notice = Some(Notice::error("Could not start the request."));
            return;
        }
        self.proposal = None;
        self.auto_requested = false;
        self.plan = None;
        self.review = None;
        self.notice = None;
        self.streaming.clear();
        self.steps.clear();
        self.input.clear();
        self.scroll_to_end = true;
        self.run = Some(Run {
            rx,
            cancel,
            base,
            typed,
            mode,
            draft,
            auto: self.prefs.auto_apply,
        });
    }

    /// Asks the request to stop. The panel shows it stopped at once; the thread follows when its
    /// connection next yields, and its late text goes nowhere. What the assistant had already
    /// changed in its draft is kept as a proposal, like any other, so stopping loses no work.
    pub fn stop(&mut self, compile: &dyn Fn(&str) -> Vec<Diagnostic>) {
        if let Some(run) = self.run.take() {
            run.cancel.store(true, Ordering::Relaxed);
            self.input = run.typed.clone();
            self.streaming.clear();
            self.notice = Some(Notice::info("Stopped."));
            if let Some(script) = changed_draft(run.draft.as_ref(), &run.base) {
                self.propose(
                    run.base.clone(),
                    script,
                    Some("Stopped before it finished.".to_owned()),
                    compile,
                );
            }
        }
    }

    /// Takes what has arrived. Returns the outcome when the request ended.
    pub fn poll(&mut self) -> Option<Finished> {
        let run = self.run.as_ref()?;
        loop {
            match run.rx.try_recv() {
                Ok(Event::Progress(progress)) => match progress {
                    Progress::Text(text) => self.streaming.push_str(&text),
                    // What came before a tool call is part of the trail, not of the reply.
                    Progress::Said(text) => {
                        self.streaming.clear();
                        self.steps.push(Entry::Said(text));
                    }
                    Progress::ToolStarted { id, label, .. } => {
                        self.streaming.clear();
                        self.steps.push(Entry::Tool(ToolStep {
                            id,
                            label,
                            outcome: None,
                        }));
                    }
                    Progress::ToolDone {
                        id,
                        summary,
                        ok,
                        content,
                    } => {
                        let step = self.steps.iter_mut().rev().find_map(|entry| match entry {
                            Entry::Tool(step) if step.id == id => Some(step),
                            _ => None,
                        });
                        if let Some(step) = step {
                            step.outcome = Some(StepOutcome {
                                summary,
                                ok,
                                detail: content,
                            });
                        }
                    }
                },
                Ok(Event::Done(result)) => return Some(self.end(result)),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    return Some(self.end(Err(LlmError::Protocol(
                        "The assistant stopped unexpectedly.".to_owned(),
                    ))));
                }
            }
        }
    }

    fn end(&mut self, result: Result<Ended, LlmError>) -> Finished {
        let run = self.run.take().expect("a request was running");
        self.streaming.clear();
        Finished {
            result,
            base: run.base.clone(),
            typed: run.typed.clone(),
            mode: run.mode,
            draft: run.draft.clone(),
            auto: run.auto,
        }
    }

    /// Records how a request ended. `compile` reports the diagnostics a script would have, so a
    /// proposal can say whether it compiles before the user accepts it.
    pub fn finish(&mut self, finished: Finished, compile: &dyn Fn(&str) -> Vec<Diagnostic>) {
        let Finished {
            result,
            base,
            typed,
            mode,
            draft,
            auto,
        } = finished;
        self.scroll_to_end = true;
        self.auto_requested = auto && mode == Mode::Edit;
        // What the assistant changed in its draft, if anything. Only Edit has tools that write.
        let drafted = changed_draft(draft.as_ref(), &base);
        match result {
            Err(LlmError::Cancelled) => {
                self.input = typed;
                self.notice = Some(Notice::info("Stopped."));
                if let Some(script) = drafted {
                    let note = "Stopped before it finished.".to_owned();
                    self.propose(base, script, Some(note), compile);
                }
            }
            Err(error) => {
                // The message goes back in the box so the user can send it again.
                if self.input.is_empty() {
                    self.input = typed;
                }
                self.notice = Some(Notice::failure(error.report()));
                if let Some(script) = drafted {
                    let note = "Incomplete: the request failed before it finished.".to_owned();
                    self.propose(base, script, Some(note), compile);
                }
            }
            Ok(ended) => {
                let mut text = ended.text.clone();
                if text.trim().is_empty() {
                    text = match &ended.limit {
                        Some(limit) => limit.message(),
                        None => "(No reply.)".to_owned(),
                    };
                }
                self.turns.push(Turn::User(typed));
                self.turns.push(Turn::Assistant {
                    text: text.clone(),
                    mode,
                });
                if ended.tools_refused {
                    self.notice = Some(Notice::info(
                        "This server does not accept tools, so the assistant answered without them.",
                    ));
                }
                if mode == Mode::Plan {
                    self.plan = ended.plan.clone();
                }
                // Ask and Plan only talk. Whatever the model wrote, no proposal comes of it.
                if mode != Mode::Edit {
                    return;
                }
                if let Some(script) = drafted {
                    let note = ended
                        .limit
                        .as_ref()
                        .map(|limit| format!("Incomplete. {}", limit.message()));
                    self.propose(base, script, note, compile);
                    return;
                }
                if let Some(limit) = &ended.limit {
                    self.notice = Some(Notice::error(limit.message()));
                    return;
                }
                let reply = prompt::split_reply(&text);
                let Some(script) = reply.script else {
                    return;
                };
                if ended.truncated || !reply.complete {
                    self.notice = Some(Notice::error(
                        "The reply hit the model's length limit, so the script is cut off. Ask for a smaller change.",
                    ));
                    return;
                }
                let script = match_line_endings(&script, &base);
                if same_text(&script, &base) {
                    self.notice = Some(Notice::info(
                        "The script already does this. Nothing to change.",
                    ));
                    return;
                }
                self.propose(base, script, None, compile);
            }
        }
    }

    /// Makes `script` the change waiting for the user.
    fn propose(
        &mut self,
        base: String,
        script: String,
        note: Option<String>,
        compile: &dyn Fn(&str) -> Vec<Diagnostic>,
    ) {
        let diagnostics = compile(&script);
        let errors = diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count();
        self.proposal = Some(Proposal {
            changes: DiffSnapshot::compute(&base, &script),
            base,
            script,
            errors,
            warnings: diagnostics.len() - errors,
            note,
        });
    }
}

/// Whether Auto may apply `proposal` to an editor holding `editor_source`, or why it must wait for
/// the user. Auto applies only what is clean: the request finished normally, the result has no
/// compile errors (warnings are fine), and the editor still holds the text it was made from (a
/// manual Apply would be refused otherwise, so there is nothing to apply). The reason finishes the
/// sentence "Not applied automatically: …".
pub fn auto_apply_check(proposal: &Proposal, editor_source: &str) -> Result<(), String> {
    if proposal.note.is_some() {
        return Err("it did not finish.".to_owned());
    }
    if proposal.errors > 0 {
        let n = proposal.errors;
        return Err(format!(
            "it has {n} compile error{}.",
            if n == 1 { "" } else { "s" }
        ));
    }
    if editor_source != proposal.base {
        return Err("the script changed while the assistant worked.".to_owned());
    }
    Ok(())
}

/// The draft as a script in the file's own line endings, if it differs from the script the request
/// started from.
fn changed_draft(draft: Option<&Draft>, base: &str) -> Option<String> {
    let script = match_line_endings(&draft?.text(), base);
    (!same_text(&script, base)).then_some(script)
}

/// Equal apart from line endings and trailing blank space, which a model does not reproduce.
fn same_text(a: &str, b: &str) -> bool {
    a.replace("\r\n", "\n").trim_end() == b.replace("\r\n", "\n").trim_end()
}

/// The model writes `\n` and ends the script with or without a newline; the user's file may do
/// otherwise, and a change should not rewrite every line ending.
fn match_line_endings(script: &str, base: &str) -> String {
    let mut text = script.replace("\r\n", "\n");
    if base.ends_with('\n') && !text.ends_with('\n') {
        text.push('\n');
    } else if !base.ends_with('\n') {
        text.truncate(text.trim_end_matches('\n').len());
    }
    if base.contains("\r\n") {
        text = text.replace('\n', "\r\n");
    }
    text
}

#[cfg(test)]
mod tests;
