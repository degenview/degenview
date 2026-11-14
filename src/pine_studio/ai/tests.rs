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
        category: crate::pine::syntax::token::DiagnosticCategory::Semantic,
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

use crate::pine_studio::ai::agent::MAX_ROUNDS;
use crate::pine_studio::ai::tools::ToolBox;
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
