//! The assistant's state: the conversation about the open script, the request in flight, and the
//! change it proposed. No egui and no store; `ai_host` connects it to the Script Manager and
//! `ai_panel` draws it.
//!
//! A reply streams on its own thread and arrives over a channel, so the UI never waits on the
//! network. Nothing here changes the script: a finished reply becomes a [`Proposal`] that the user
//! accepts or discards, and accepting goes through the editor as one undo step.

use super::ai_models::{self, ModelList};
use super::ai_prompt::{self, Mode, Turn};
use super::ai_review::Review;
use super::diff::DiffSnapshot;
use super::model::ScriptId;
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use crate::service::llm::prefs::AiPrefs;
use crate::service::llm::report::ErrorReport;
use crate::service::llm::{ChatRequest, Done, LlmError, LlmProvider, ProviderKind};
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
}

enum Event {
    Text(String),
    Done(Result<Done, LlmError>),
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
}

impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// How a request ended.
pub struct Finished {
    result: Result<Done, LlmError>,
    text: String,
    base: String,
    typed: String,
    mode: Mode,
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
    pub proposal: Option<Proposal>,
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
        self.proposal = None;
        self.review = None;
        self.notice = None;
    }

    /// Switches mode. Refused while a request is running or a proposal waits, so Ask can never
    /// sit beside an Apply button.
    pub fn set_mode(&mut self, mode: Mode) -> bool {
        if self.run.is_some() || self.proposal.is_some() {
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
        self.proposal = None;
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
        let (sender, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("ai-assistant".to_owned())
            .spawn(move || {
                let result = provider.stream_chat(&request, &flag, &mut |text| {
                    let _ = sender.send(Event::Text(text.to_owned()));
                    wake();
                });
                let _ = sender.send(Event::Done(result));
                wake();
            });
        if spawned.is_err() {
            self.notice = Some(Notice::error("Could not start the request."));
            return;
        }
        self.proposal = None;
        self.review = None;
        self.notice = None;
        self.streaming.clear();
        self.input.clear();
        self.scroll_to_end = true;
        self.run = Some(Run {
            rx,
            cancel,
            base,
            typed,
            mode,
        });
    }

    /// Asks the request to stop. The panel shows it stopped at once; the thread follows when its
    /// connection next yields, and its late text goes nowhere.
    pub fn stop(&mut self) {
        if let Some(run) = self.run.take() {
            run.cancel.store(true, Ordering::Relaxed);
            self.input = run.typed.clone();
            self.streaming.clear();
            self.notice = Some(Notice::info("Stopped."));
        }
    }

    /// Takes what has arrived. Returns the outcome when the request ended.
    pub fn poll(&mut self) -> Option<Finished> {
        let run = self.run.as_ref()?;
        loop {
            match run.rx.try_recv() {
                Ok(Event::Text(text)) => {
                    self.streaming.push_str(&text);
                }
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

    fn end(&mut self, result: Result<Done, LlmError>) -> Finished {
        let run = self.run.take().expect("a request was running");
        Finished {
            result,
            text: std::mem::take(&mut self.streaming),
            base: run.base.clone(),
            typed: run.typed.clone(),
            mode: run.mode,
        }
    }

    /// Records how a request ended. `compile` reports the diagnostics a script would have, so a
    /// proposal can say whether it compiles before the user accepts it.
    pub fn finish(&mut self, finished: Finished, compile: &dyn Fn(&str) -> Vec<Diagnostic>) {
        let Finished {
            result,
            text,
            base,
            typed,
            mode,
        } = finished;
        self.scroll_to_end = true;
        match result {
            Err(LlmError::Cancelled) => {
                self.input = typed;
                self.notice = Some(Notice::info("Stopped."));
            }
            Err(error) => {
                // The message goes back in the box so the user can send it again.
                if self.input.is_empty() {
                    self.input = typed;
                }
                self.notice = Some(Notice::failure(error.report()));
            }
            Ok(done) => {
                self.turns.push(Turn::User(typed));
                self.turns.push(Turn::Assistant {
                    text: text.clone(),
                    mode,
                });
                // Ask only talks. Whatever the model wrote, no proposal comes of it.
                if mode == Mode::Ask {
                    return;
                }
                let reply = ai_prompt::split_reply(&text);
                let Some(script) = reply.script else {
                    return;
                };
                if done.truncated || !reply.complete {
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
                });
            }
        }
    }
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
        assistant.stop();
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
        assistant.stop();
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
        assistant.stop();
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
}
