//! The assistant's state: the conversation about the open script, the request in flight, and the
//! change it proposed. No egui and no store; `ai_host` connects it to the Script Manager and
//! `ai_panel` draws it.
//!
//! A reply streams on its own thread and arrives over a channel, so the UI never waits on the
//! network. Nothing here changes the script: a finished reply becomes a [`Proposal`] that the user
//! accepts or discards, and accepting goes through the editor as one undo step.

use super::ai_agent::{self, AgentSetup, Ended, Progress};
use super::ai_models::{self, ModelList};
use super::ai_plan::Plan;
use super::ai_prompt::{self, Mode, Turn};
use super::ai_review::Review;
use super::ai_tools::Draft;
use super::diff::DiffSnapshot;
use super::model::ScriptId;
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use crate::service::llm::prefs::AiPrefs;
use crate::service::llm::report::ErrorReport;
use crate::service::llm::{ChatRequest, LlmError, LlmProvider, ProviderKind};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

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
        self.models_rx = Some((kind, ai_models::fetch(provider, wake)));
    }

    /// Takes in a model list that has arrived.
    pub fn poll_models(&mut self) {
        let Some((kind, receiver)) = &self.models_rx else {
            return;
        };
        let kind = *kind;
        let list = match receiver.try_recv() {
            Ok(Ok(ids)) => ModelList::Ready(ai_models::usable_chat_models(kind, &ids)),
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
                Some(Turn::Assistant { text, mode: Mode::Ask }) if ai_prompt::looks_like_plan(text)
            )
    }

    /// Starts a request on its own thread. `wake` repaints the UI when text arrives.
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
                let result =
                    ai_agent::run(&*provider, request, setup.as_ref(), &flag, &mut |piece| {
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
                let reply = ai_prompt::split_reply(&text);
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
mod tests {
    use super::*;
    use crate::service::llm::ChatMessage;
    use crate::service::llm::mock::MockProvider;
    use std::time::{Duration, Instant};

    fn request() -> ChatRequest {
        ChatRequest {
            model: "m".to_owned(),
            system: "s".to_owned(),
            messages: vec![ChatMessage::user("hi")],
            max_tokens: 10,
            tools: Vec::new(),
        }
    }

    fn wake() -> Arc<dyn Fn() + Send + Sync> {
        Arc::new(|| {})
    }

    /// Runs a scripted reply to its end and finishes it against `base`.
    fn run(replies: Vec<Result<Vec<String>, LlmError>>, base: &str) -> Assistant {
        let mut assistant = Assistant {
            input: "typed".to_owned(),
            ..Assistant::default()
        };
        assistant.start(
            Box::new(MockProvider::replying(replies)),
            request(),
            "make it red".to_owned(),
            base.to_owned(),
            Mode::Edit,
            wake(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let finished = loop {
            if let Some(finished) = assistant.poll() {
                break finished;
            }
            assert!(Instant::now() < deadline, "the reply never finished");
            std::thread::sleep(Duration::from_millis(2));
        };
        assistant.finish(finished, &|_| Vec::new());
        assistant
    }

    const BASE: &str = "//@version=6\nindicator(\"x\")\nplot(close)\n";

    fn reply_with(script: &str) -> Result<Vec<String>, LlmError> {
        MockProvider::text(&["Made it red.\n```pine\n", script, "```\n"])
    }

    #[test]
    fn a_reply_with_a_new_script_becomes_a_proposal_and_changes_nothing() {
        let new = "//@version=6\nindicator(\"x\")\nplot(close, color = color.red)\n";
        let assistant = run(vec![reply_with(new)], BASE);
        let proposal = assistant.proposal.clone().expect("a proposal");
        assert_eq!(proposal.base, BASE);
        assert_eq!(proposal.script, new);
        assert_eq!(proposal.changes.hunks().len(), 1);
        assert_eq!(proposal.errors, 0);
        assert_eq!(assistant.turns.len(), 2);
        assert!(!assistant.is_running());
        assert_eq!(assistant.streaming, "");
        // Sending cleared the box.
        assert_eq!(assistant.input, "");
    }

    #[test]
    fn a_proposal_carries_the_diagnostics_count_of_its_script() {
        let mut assistant = Assistant::default();
        assistant.start(
            Box::new(MockProvider::replying(vec![reply_with(
                "//@version=6\nplot(clse)\n",
            )])),
            request(),
            "fix".to_owned(),
            BASE.to_owned(),
            Mode::Edit,
            wake(),
        );
        let finished = loop {
            if let Some(finished) = assistant.poll() {
                break finished;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let diagnostic = |severity| Diagnostic {
            code: "X".into(),
            severity,
            category: crate::pine::token::DiagnosticCategory::Semantic,
            message: "m".into(),
            range: Default::default(),
        };
        assistant.finish(finished, &|_| {
            vec![
                diagnostic(DiagnosticSeverity::Error),
                diagnostic(DiagnosticSeverity::Warning),
                diagnostic(DiagnosticSeverity::Warning),
            ]
        });
        let proposal = assistant.proposal.unwrap();
        assert_eq!((proposal.errors, proposal.warnings), (1, 2));
    }

    #[test]
    fn a_prose_only_reply_proposes_nothing() {
        let assistant = run(
            vec![MockProvider::text(&["The RSI measures momentum."])],
            BASE,
        );
        assert!(assistant.proposal.is_none());
        assert!(assistant.notice.is_none());
        assert_eq!(assistant.turns.len(), 2);
    }

    #[test]
    fn a_script_equal_to_the_current_one_is_not_a_change() {
        // Differs from BASE only by a missing final newline and CRLF endings.
        let assistant = run(vec![reply_with(&BASE.replace('\n', "\r\n"))], BASE);
        assert!(assistant.proposal.is_none());
        assert!(
            assistant
                .notice
                .as_ref()
                .unwrap()
                .text
                .contains("Nothing to change")
        );
    }

    #[test]
    fn a_cut_off_script_is_refused_not_offered() {
        let assistant = run(
            vec![MockProvider::text(&["Here:\n```pine\n//@version=6\npl"])],
            BASE,
        );
        assert!(assistant.proposal.is_none());
        assert!(assistant.notice.as_ref().unwrap().error);
    }

    #[test]
    fn a_failed_request_puts_the_message_back_for_another_try() {
        let mut assistant = Assistant::default();
        assistant.start(
            Box::new(MockProvider::replying(vec![Err(LlmError::rate_limited(
                "slow down",
            ))])),
            request(),
            "make it red".to_owned(),
            BASE.to_owned(),
            Mode::Edit,
            wake(),
        );
        let finished = loop {
            if let Some(finished) = assistant.poll() {
                break finished;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        assistant.finish(finished, &|_| Vec::new());
        assert_eq!(assistant.input, "make it red");
        assert!(
            assistant.turns.is_empty(),
            "an unanswered message is not history"
        );
        let notice = assistant.notice.unwrap();
        assert!(notice.error && notice.error_report().detail.contains("slow down"));
    }

    #[test]
    fn stop_ends_the_request_at_once_and_restores_the_message() {
        let mut assistant = Assistant::default();
        assistant.start(
            Box::new(MockProvider::replying(vec![reply_with("x")])),
            request(),
            "make it red".to_owned(),
            BASE.to_owned(),
            Mode::Edit,
            wake(),
        );
        assistant.stop(&|_| Vec::new());
        assert!(!assistant.is_running());
        assert_eq!(assistant.input, "make it red");
        assert_eq!(assistant.notice, Some(Notice::info("Stopped.")));
        assert!(assistant.poll().is_none(), "late text goes nowhere");
        assert!(assistant.turns.is_empty());
    }

    #[test]
    fn a_new_request_clears_the_old_proposal_and_notice() {
        let mut assistant = run(vec![reply_with("//@version=6\nplot(close)\n")], BASE);
        assert!(assistant.proposal.is_some());
        assistant.notice = Some(Notice::error("old"));
        assistant.start(
            Box::new(MockProvider::replying(vec![])),
            request(),
            "again".to_owned(),
            BASE.to_owned(),
            Mode::Edit,
            wake(),
        );
        assert!(assistant.proposal.is_none());
        assert!(assistant.notice.is_none());
    }

    #[test]
    fn reset_forgets_the_conversation_and_keeps_the_typed_text() {
        let mut assistant = run(vec![reply_with("//@version=6\nplot(close)\n")], BASE);
        assistant.input = "half typed".to_owned();
        assistant.reset(Some("other".into()));
        assert!(assistant.turns.is_empty());
        assert!(assistant.proposal.is_none());
        assert_eq!(assistant.script_id.as_deref(), Some("other"));
        assert_eq!(assistant.input, "half typed");
    }

    #[test]
    fn the_models_line_endings_follow_the_users_file() {
        assert_eq!(match_line_endings("a\nb", "x\ny\n"), "a\nb\n");
        assert_eq!(match_line_endings("a\nb\n\n", "x\ny"), "a\nb");
        assert_eq!(match_line_endings("a\nb", "x\r\ny\r\n"), "a\r\nb\r\n");
        assert!(same_text("a\r\nb\n", "a\nb"));
        assert!(!same_text("a", "b"));
    }

    /// Runs a scripted reply in `mode` to its end.
    fn run_in(mode: Mode, replies: Vec<Result<Vec<String>, LlmError>>, base: &str) -> Assistant {
        let mut assistant = Assistant::default();
        assistant.start(
            Box::new(MockProvider::replying(replies)),
            request(),
            "a question".to_owned(),
            base.to_owned(),
            mode,
            wake(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let finished = loop {
            if let Some(finished) = assistant.poll() {
                break finished;
            }
            assert!(Instant::now() < deadline, "the reply never finished");
            std::thread::sleep(Duration::from_millis(2));
        };
        assistant.finish(finished, &|_| Vec::new());
        assistant
    }

    #[test]
    fn a_new_conversation_starts_in_ask() {
        assert_eq!(Assistant::default().mode, Mode::Ask);
    }

    #[test]
    fn ask_never_proposes_even_when_the_model_writes_a_whole_script() {
        let new = "//@version=6\nindicator(\"x\")\nplot(close, color = color.red)\n";
        let assistant = run_in(Mode::Ask, vec![reply_with(new)], BASE);
        assert!(assistant.proposal.is_none(), "Ask must never offer Apply");
        assert!(assistant.notice.is_none());
        // The reply is still kept, with the mode it was made in, so it shows as code.
        assert_eq!(
            assistant.turns.last(),
            Some(&Turn::Assistant {
                text: format!("Made it red.\n```pine\n{new}```\n"),
                mode: Mode::Ask
            })
        );
    }

    #[test]
    fn edit_still_proposes() {
        let new = "//@version=6\nindicator(\"x\")\nplot(close, color = color.red)\n";
        let assistant = run_in(Mode::Edit, vec![reply_with(new)], BASE);
        assert!(assistant.proposal.is_some());
    }

    #[test]
    fn the_mode_cannot_change_while_a_request_runs_or_a_change_waits() {
        let mut assistant = Assistant::default();
        assert!(assistant.set_mode(Mode::Edit));
        assert_eq!(assistant.mode, Mode::Edit);
        assistant.start(
            Box::new(MockProvider::replying(vec![reply_with("x")])),
            request(),
            "q".to_owned(),
            BASE.to_owned(),
            Mode::Edit,
            wake(),
        );
        assert!(!assistant.set_mode(Mode::Ask), "refused while running");
        assistant.stop(&|_| Vec::new());
        assert!(assistant.set_mode(Mode::Ask));

        let mut with_proposal = run_in(
            Mode::Edit,
            vec![reply_with("//@version=6\nplot(close)\n")],
            BASE,
        );
        assert!(with_proposal.proposal.is_some());
        with_proposal.mode = Mode::Edit;
        assert!(
            !with_proposal.set_mode(Mode::Ask),
            "refused while a change waits"
        );
        assert_eq!(with_proposal.mode, Mode::Edit);
    }

    #[test]
    fn a_plan_reply_with_numbered_steps_can_be_implemented() {
        let plan =
            MockProvider::text(&["Here is the plan:\n1. Add an input\n2. Use it in the average\n"]);
        let assistant = run_in(Mode::Ask, vec![plan], BASE);
        assert!(assistant.can_implement());

        let answer = MockProvider::text(&["The RSI measures momentum."]);
        assert!(!run_in(Mode::Ask, vec![answer], BASE).can_implement());

        // Steps in an edit reply, or while a request is running, do not offer it.
        let steps = MockProvider::text(&["1. a\n2. b\n"]);
        assert!(!run_in(Mode::Edit, vec![steps], BASE).can_implement());
        let mut running = run_in(Mode::Ask, vec![MockProvider::text(&["1. a\n2. b\n"])], BASE);
        running.start(
            Box::new(MockProvider::replying(vec![])),
            request(),
            "again".to_owned(),
            BASE.to_owned(),
            Mode::Ask,
            wake(),
        );
        assert!(!running.can_implement());
    }

    #[test]
    fn a_new_script_goes_back_to_ask_but_start_over_keeps_the_mode() {
        let mut assistant = run_in(
            Mode::Edit,
            vec![reply_with("//@version=6\nplot(close)\n")],
            BASE,
        );
        assistant.mode = Mode::Edit;
        assistant.clear();
        assert_eq!(assistant.mode, Mode::Edit, "Start over keeps the mode");
        assert!(assistant.turns.is_empty());
        assistant.reset(Some("other".into()));
        assert_eq!(assistant.mode, Mode::Ask);
    }

    #[test]
    fn the_request_is_visible_while_it_runs_and_scrolls_the_transcript() {
        let mut assistant = Assistant::default();
        assert_eq!(assistant.pending_request(), None);
        assistant.start(
            Box::new(MockProvider::replying(vec![reply_with("x")])),
            request(),
            "make it red".to_owned(),
            BASE.to_owned(),
            Mode::Edit,
            wake(),
        );
        assert_eq!(assistant.pending_request(), Some("make it red"));
        assert!(assistant.scroll_to_end);
        assistant.stop(&|_| Vec::new());
        assert_eq!(assistant.pending_request(), None);
    }

    #[test]
    fn rewinding_takes_back_the_last_exchange_and_its_mode() {
        let mut assistant = run_in(
            Mode::Edit,
            vec![reply_with("//@version=6\nplot(close)\n")],
            BASE,
        );
        assistant.mode = Mode::Ask;
        assert!(assistant.proposal.is_some());
        let text = assistant.rewind_last();
        assert_eq!(text.as_deref(), Some("a question"));
        assert!(assistant.turns.is_empty());
        assert!(assistant.proposal.is_none() && assistant.notice.is_none());
        assert_eq!(assistant.mode, Mode::Edit, "the mode it was asked in");
    }

    #[test]
    fn rewinding_only_undoes_the_last_exchange() {
        let mut assistant = run_in(Mode::Ask, vec![MockProvider::text(&["one"])], BASE);
        assistant.turns.push(Turn::User("second".into()));
        assistant.turns.push(Turn::Assistant {
            text: "two".into(),
            mode: Mode::Ask,
        });
        assert_eq!(assistant.rewind_last().as_deref(), Some("second"));
        assert_eq!(assistant.turns.len(), 2, "the first exchange stays");
        assert_eq!(assistant.rewind_last().as_deref(), Some("a question"));
        assert_eq!(assistant.rewind_last(), None, "nothing left");
    }

    #[test]
    fn nothing_rewinds_while_a_request_runs() {
        let mut assistant = run_in(Mode::Ask, vec![MockProvider::text(&["one"])], BASE);
        assistant.start(
            Box::new(MockProvider::replying(vec![])),
            request(),
            "again".to_owned(),
            BASE.to_owned(),
            Mode::Ask,
            wake(),
        );
        assert_eq!(assistant.rewind_last(), None);
        assert_eq!(assistant.turns.len(), 2);
    }

    #[test]
    fn a_model_list_arrives_filtered_or_as_an_error() {
        let wait = |assistant: &mut Assistant, kind: ProviderKind| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while matches!(assistant.models.get(&kind), Some(ModelList::Loading)) {
                assert!(Instant::now() < deadline, "no answer");
                assistant.poll_models();
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let mut assistant = Assistant::default();
        let mut provider = MockProvider::replying(vec![]);
        provider.models = Ok(vec![
            "gpt-5".to_owned(),
            "text-embedding-3-small".to_owned(),
            "o3".to_owned(),
        ]);
        assistant.begin_model_fetch(ProviderKind::OpenAi, Box::new(provider), wake());
        assert_eq!(
            assistant.models.get(&ProviderKind::OpenAi),
            Some(&ModelList::Loading)
        );
        wait(&mut assistant, ProviderKind::OpenAi);
        assert_eq!(
            assistant.models.get(&ProviderKind::OpenAi),
            Some(&ModelList::Ready(vec!["gpt-5".to_owned(), "o3".to_owned()]))
        );

        let mut failing = MockProvider::replying(vec![]);
        failing.models = Err(LlmError::auth("bad key"));
        assistant.begin_model_fetch(ProviderKind::Anthropic, Box::new(failing), wake());
        wait(&mut assistant, ProviderKind::Anthropic);
        assert!(matches!(
            assistant.models.get(&ProviderKind::Anthropic),
            Some(ModelList::Failed(report)) if report.detail.contains("bad key")
        ));
    }

    // MARK: Tools

    use crate::script_manager::ai_agent::MAX_ROUNDS;
    use crate::script_manager::ai_tools::ToolBox;
    use crate::service::llm::mock::Scripted;
    use serde_json::json;

    fn no_problems(_: &str) -> Vec<Diagnostic> {
        Vec::new()
    }

    fn agent_request(tools: &ToolBox) -> ChatRequest {
        ChatRequest {
            tools: tools.specs(),
            ..request()
        }
    }

    fn setup(mode: Mode, base: &str) -> AgentSetup {
        AgentSetup {
            tools: ToolBox::new(mode, base, no_problems),
            fallback_system: "plain".to_owned(),
        }
    }

    fn start_agent(assistant: &mut Assistant, mode: Mode, base: &str, provider: MockProvider) {
        let setup = setup(mode, base);
        let request = agent_request(&setup.tools);
        assistant.start_agent(
            Box::new(provider),
            request,
            Some(setup),
            "make it red".to_owned(),
            base.to_owned(),
            mode,
            wake(),
        );
    }

    fn wait_for_end(assistant: &mut Assistant) -> Finished {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(finished) = assistant.poll() {
                return finished;
            }
            assert!(Instant::now() < deadline, "the request never finished");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn run_agent(mode: Mode, base: &str, turns: Vec<Result<Scripted, LlmError>>) -> Assistant {
        let mut assistant = Assistant::default();
        start_agent(&mut assistant, mode, base, MockProvider::turns(turns));
        let finished = wait_for_end(&mut assistant);
        assistant.finish(finished, &|_| Vec::new());
        assistant
    }

    fn red_edit(id: &str) -> Scripted {
        Scripted::call(
            id,
            "edit_script",
            json!({"old": "plot(close)", "new": "plot(close, color = color.red)"}),
        )
    }

    #[test]
    fn an_edit_made_with_tools_becomes_a_proposal_from_the_draft() {
        let assistant = run_agent(
            Mode::Edit,
            BASE,
            vec![Ok(red_edit("t1")), Ok(Scripted::say("Made the line red."))],
        );
        let proposal = assistant.proposal.clone().expect("a proposal");
        assert_eq!(proposal.base, BASE);
        assert_eq!(
            proposal.script,
            "//@version=6\nindicator(\"x\")\nplot(close, color = color.red)\n"
        );
        assert_eq!(proposal.note, None);
        assert!(assistant.notice.is_none());
        // The words are kept in history; the trail is shown but not stored.
        assert!(matches!(
            assistant.turns.last(),
            Some(Turn::Assistant { text, mode: Mode::Edit }) if text == "Made the line red."
        ));
        assert_eq!(assistant.steps.len(), 1);
        let Entry::Tool(step) = &assistant.steps[0] else {
            panic!("a tool step");
        };
        assert!(step.outcome.as_ref().unwrap().ok);
    }

    #[test]
    fn the_files_line_endings_come_back_in_the_proposal() {
        let crlf = BASE.replace('\n', "\r\n");
        let assistant = run_agent(
            Mode::Edit,
            &crlf,
            vec![Ok(red_edit("t1")), Ok(Scripted::say("Done."))],
        );
        let proposal = assistant.proposal.expect("a proposal");
        assert!(proposal.script.contains("color = color.red)\r\n"));
        assert!(!proposal.script.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn a_question_answered_without_edits_proposes_nothing() {
        let assistant = run_agent(
            Mode::Edit,
            BASE,
            vec![
                Ok(Scripted::call("t1", "read_script", json!({}))),
                Ok(Scripted::say("It plots the close.")),
            ],
        );
        assert!(assistant.proposal.is_none());
        assert!(assistant.notice.is_none());
        assert_eq!(assistant.turns.len(), 2);
    }

    #[test]
    fn ask_mode_with_tools_never_proposes_even_if_the_model_tries_to_edit() {
        let assistant = run_agent(
            Mode::Ask,
            BASE,
            vec![
                Ok(red_edit("t1")),
                Ok(Scripted::say("I cannot edit, but change line 3.")),
            ],
        );
        assert!(assistant.proposal.is_none());
        let Entry::Tool(step) = &assistant.steps[0] else {
            panic!("a tool step");
        };
        assert!(!step.outcome.as_ref().unwrap().ok);
    }

    #[test]
    fn running_out_of_steps_still_offers_what_was_done_labelled_incomplete() {
        let mut turns = vec![Ok(red_edit("t0"))];
        turns.extend((1..=MAX_ROUNDS + 2).map(|n| {
            Ok(Scripted::call(
                &format!("t{n}"),
                "read_script",
                json!({"start": n}),
            ))
        }));
        let assistant = run_agent(Mode::Edit, BASE, turns);
        let proposal = assistant.proposal.expect("the work so far");
        assert!(proposal.note.as_deref().unwrap().starts_with("Incomplete."));
        assert!(proposal.script.contains("color.red"));
    }

    #[test]
    fn a_failure_after_an_edit_keeps_the_edit_as_an_incomplete_proposal() {
        let assistant = run_agent(
            Mode::Edit,
            BASE,
            vec![Ok(red_edit("t1")), Err(LlmError::rate_limited("slow down"))],
        );
        let notice = assistant.notice.clone().unwrap();
        assert!(notice.error && notice.error_report().detail.contains("slow down"));
        let proposal = assistant.proposal.expect("the edit is not lost");
        assert!(proposal.note.as_deref().unwrap().starts_with("Incomplete"));
        // The message is back in the box to send again.
        assert_eq!(assistant.input, "make it red");
        assert!(assistant.turns.is_empty());
    }

    /// Edits once, then waits for the request to be stopped.
    struct EditsThenWaits {
        turns: std::sync::Mutex<usize>,
    }

    impl LlmProvider for EditsThenWaits {
        fn stream_chat(
            &self,
            _: &ChatRequest,
            _: &AtomicBool,
            _: &mut dyn FnMut(&str),
        ) -> Result<crate::service::llm::Done, LlmError> {
            unreachable!("only turns are used")
        }

        fn stream_turn(
            &self,
            _: &ChatRequest,
            cancel: &AtomicBool,
            _: &mut dyn FnMut(&str),
        ) -> Result<crate::service::llm::TurnReply, LlmError> {
            let mut turns = self.turns.lock().unwrap();
            *turns += 1;
            if *turns == 1 {
                return Ok(crate::service::llm::TurnReply {
                    text: String::new(),
                    calls: vec![crate::service::llm::ToolCall {
                        id: "t1".to_owned(),
                        name: "edit_script".to_owned(),
                        input: json!({"old": "plot(close)", "new": "plot(open)"}),
                        malformed: None,
                    }],
                    stop: crate::service::llm::Stop::ToolUse,
                });
            }
            while !cancel.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(LlmError::Cancelled)
        }

        fn list_models(&self) -> Result<Vec<String>, LlmError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn stopping_a_request_keeps_the_draft_as_a_proposal() {
        let mut assistant = Assistant::default();
        let setup = setup(Mode::Edit, BASE);
        let request = agent_request(&setup.tools);
        assistant.start_agent(
            Box::new(EditsThenWaits {
                turns: std::sync::Mutex::new(0),
            }),
            request,
            Some(setup),
            "use the open".to_owned(),
            BASE.to_owned(),
            Mode::Edit,
            wake(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !assistant
            .steps
            .iter()
            .any(|entry| matches!(entry, Entry::Tool(step) if step.outcome.is_some()))
        {
            assert!(assistant.poll().is_none());
            assert!(Instant::now() < deadline, "the edit never happened");
            std::thread::sleep(Duration::from_millis(2));
        }
        assistant.stop(&no_problems);
        let proposal = assistant.proposal.clone().expect("the edit is kept");
        assert_eq!(
            proposal.note.as_deref(),
            Some("Stopped before it finished.")
        );
        assert!(proposal.script.contains("plot(open)"));
        assert_eq!(assistant.input, "use the open");
        assert!(!assistant.is_running());
    }

    #[test]
    fn stopping_with_nothing_changed_proposes_nothing() {
        let mut assistant = Assistant::default();
        start_agent(
            &mut assistant,
            Mode::Edit,
            BASE,
            MockProvider::turns(vec![Ok(Scripted::say("thinking"))]),
        );
        assistant.stop(&no_problems);
        assert!(assistant.proposal.is_none());
    }

    #[test]
    fn a_server_that_refuses_tools_is_told_to_the_user_and_the_old_path_runs() {
        let new = "//@version=6\nindicator(\"x\")\nplot(close, color = color.red)\n";
        let assistant = run_agent(
            Mode::Edit,
            BASE,
            vec![
                Err(LlmError::http(400, "tools are not supported")),
                Ok(Scripted::say(&format!("Done.\n```pine\n{new}```\n"))),
            ],
        );
        assert!(
            assistant
                .notice
                .as_ref()
                .unwrap()
                .text
                .contains("does not accept tools")
        );
        assert_eq!(
            assistant.proposal.expect("from the fenced script").script,
            new
        );
    }

    #[test]
    fn the_trail_starts_over_with_each_request() {
        let mut assistant = run_agent(
            Mode::Edit,
            BASE,
            vec![Ok(red_edit("t1")), Ok(Scripted::say("Done."))],
        );
        assert_eq!(assistant.steps.len(), 1);
        start_agent(
            &mut assistant,
            Mode::Edit,
            BASE,
            MockProvider::turns(vec![Ok(Scripted::say("later"))]),
        );
        assert!(assistant.steps.is_empty());
    }

    fn plan_turn(id: &str) -> Scripted {
        Scripted::call(
            id,
            "submit_plan",
            json!({"summary": "Make the line red.", "steps": [
                {"title": "Colour it", "detail": "Pass color.red.", "lines": "4"},
                {"title": "Check", "detail": "Compile."}]}),
        )
    }

    #[test]
    fn a_submitted_plan_waits_for_the_user_and_changes_nothing() {
        let assistant = run_agent(Mode::Plan, BASE, vec![Ok(plan_turn("p1"))]);
        let plan = assistant.plan.clone().expect("a plan");
        assert_eq!(plan.steps.len(), 2);
        assert!(assistant.proposal.is_none());
        // Its words are the reply, and so the history.
        assert!(matches!(
            assistant.turns.last(),
            Some(Turn::Assistant { text, mode: Mode::Plan }) if text.starts_with("Make the line red.\n\n1. **Colour it**")
        ));
    }

    #[test]
    fn the_mode_is_locked_while_a_plan_waits_and_a_new_request_or_start_over_drops_it() {
        let mut assistant = run_agent(Mode::Plan, BASE, vec![Ok(plan_turn("p1"))]);
        assert!(!assistant.set_mode(Mode::Edit));
        assert_eq!(
            assistant.mode,
            Mode::Ask,
            "the mode is the one the test never set"
        );
        let mut kept = run_agent(Mode::Plan, BASE, vec![Ok(plan_turn("p1"))]);
        kept.clear();
        assert!(kept.plan.is_none());
        start_agent(
            &mut assistant,
            Mode::Plan,
            BASE,
            MockProvider::turns(vec![Ok(Scripted::say("later"))]),
        );
        assert!(assistant.plan.is_none());
    }

    #[test]
    fn plan_mode_never_proposes_even_if_the_model_writes_a_script() {
        let new = "//@version=6\nindicator(\"x\")\nplot(close, color = color.red)\n";
        let assistant = run_agent(
            Mode::Plan,
            BASE,
            vec![Ok(Scripted::say(&format!("```pine\n{new}```\n")))],
        );
        assert!(assistant.proposal.is_none());
        assert!(assistant.plan.is_none());
    }

    fn proposal_for_check(errors: usize, warnings: usize, note: Option<&str>) -> Proposal {
        Proposal {
            base: BASE.to_owned(),
            script: format!("{BASE}// x\n"),
            changes: DiffSnapshot::compute(BASE, &format!("{BASE}// x\n")),
            errors,
            warnings,
            note: note.map(str::to_owned),
        }
    }

    #[test]
    fn auto_applies_a_clean_result_made_from_the_text_in_the_editor() {
        assert_eq!(
            auto_apply_check(&proposal_for_check(0, 0, None), BASE),
            Ok(())
        );
        // Warnings do not hold it.
        assert_eq!(
            auto_apply_check(&proposal_for_check(0, 3, None), BASE),
            Ok(())
        );
    }

    #[test]
    fn auto_holds_what_is_not_clean_and_says_why() {
        assert_eq!(
            auto_apply_check(&proposal_for_check(1, 0, None), BASE),
            Err("it has 1 compile error.".to_owned())
        );
        assert_eq!(
            auto_apply_check(&proposal_for_check(2, 0, None), BASE),
            Err("it has 2 compile errors.".to_owned())
        );
        assert_eq!(
            auto_apply_check(
                &proposal_for_check(0, 0, Some("Stopped before it finished.")),
                BASE
            ),
            Err("it did not finish.".to_owned())
        );
        assert_eq!(
            auto_apply_check(&proposal_for_check(0, 0, None), "typed since"),
            Err("the script changed while the assistant worked.".to_owned())
        );
    }

    #[test]
    fn a_request_remembers_whether_auto_was_on_when_it_was_sent() {
        let mut assistant = Assistant::default();
        assistant.prefs.auto_apply = true;
        start_agent(
            &mut assistant,
            Mode::Edit,
            BASE,
            MockProvider::turns(vec![Ok(red_edit("t1")), Ok(Scripted::say("Done."))]),
        );
        // Switched off while it runs: this request still counts as sent with Auto on.
        assistant.prefs.auto_apply = false;
        let finished = wait_for_end(&mut assistant);
        assistant.finish(finished, &|_| Vec::new());
        assert!(assistant.auto_requested);
        assert!(assistant.proposal.is_some());
        // The next request, sent with it off, is not.
        let manual = run_agent(
            Mode::Edit,
            BASE,
            vec![Ok(red_edit("t1")), Ok(Scripted::say("Done."))],
        );
        assert!(!manual.auto_requested);
    }

    #[test]
    fn only_edit_requests_can_be_auto_applied() {
        let mut assistant = Assistant::default();
        assistant.prefs.auto_apply = true;
        start_agent(
            &mut assistant,
            Mode::Plan,
            BASE,
            MockProvider::turns(vec![Ok(plan_turn("p1"))]),
        );
        let finished = wait_for_end(&mut assistant);
        assistant.finish(finished, &|_| Vec::new());
        assert!(!assistant.auto_requested);
    }
}
