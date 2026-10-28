//! The loop that lets the assistant use tools: ask the model, run what it calls, give it the answers,
//! and ask again until it has nothing more to call. Runs on the request's own thread; nothing here
//! touches the editor or egui, and every way out of the loop is bounded.
//!
//! What the loop reports goes out as [`Progress`], one piece at a time, so the panel can show the
//! reply as it streams and each tool call as it happens.

use super::ai_tools::{Outcome, ToolBox};
use crate::service::llm::{ChatMessage, ChatRequest, LlmError, LlmProvider, Stop, ToolResult};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Calls to the model in one request. Each resends the whole conversation, so this bounds cost too.
pub const MAX_ROUNDS: usize = 12;
/// The same call, with the same arguments, this many times is a loop the model is not leaving.
const MAX_REPEATS: usize = 3;

/// What the panel hears while a request runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Progress {
    /// A piece of the reply.
    Text(String),
    /// A round that ended in tool calls: what the model said before calling them.
    Said(String),
    ToolStarted {
        id: String,
        name: String,
        label: String,
    },
    ToolDone {
        id: String,
        summary: String,
        ok: bool,
        content: String,
    },
}

/// Why the loop stopped without the model finishing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Limit {
    Rounds,
    Repeating,
}

impl Limit {
    pub fn message(&self) -> String {
        match self {
            Self::Rounds => format!("It used all {MAX_ROUNDS} steps without finishing."),
            Self::Repeating => "It kept repeating the same step.".to_owned(),
        }
    }
}

/// How a request that ran to the end ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ended {
    /// The model's last reply (what it said after its last tool call).
    pub text: String,
    /// The last reply hit the token limit.
    pub truncated: bool,
    /// The loop gave up; the work so far is in the draft.
    pub limit: Option<Limit>,
    /// The server refused the tools, so this was answered without them.
    pub tools_refused: bool,
}

/// What a request needs beyond the conversation to use tools.
pub struct AgentSetup {
    pub tools: ToolBox,
    /// The prompt for a request without tools, used if the server refuses them.
    pub fallback_system: String,
}

/// A refusal of the request itself, which a server without tool support gives once it sees `tools`.
fn refused_tools(error: &LlmError) -> bool {
    matches!(
        error,
        LlmError::Http {
            status: 400 | 404 | 422,
            ..
        }
    )
}

pub fn run(
    provider: &dyn LlmProvider,
    mut request: ChatRequest,
    setup: Option<&AgentSetup>,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<Ended, LlmError> {
    let mut tools_refused = false;
    let mut seen: HashMap<String, usize> = HashMap::new();
    for _ in 0..MAX_ROUNDS {
        if cancel.load(Ordering::Relaxed) {
            return Err(LlmError::Cancelled);
        }
        let reply = match provider.stream_turn(&request, cancel, &mut |text| {
            emit(Progress::Text(text.to_owned()));
        }) {
            Ok(reply) => reply,
            Err(error) if !tools_refused && !request.tools.is_empty() && refused_tools(&error) => {
                tools_refused = true;
                request.tools.clear();
                if let Some(setup) = setup {
                    request.system = setup.fallback_system.clone();
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let truncated = reply.stop == Stop::MaxTokens;
        let (Some(setup), Stop::ToolUse) = (setup, reply.stop) else {
            return Ok(Ended {
                text: reply.text,
                truncated,
                limit: None,
                tools_refused,
            });
        };
        if !reply.text.trim().is_empty() {
            emit(Progress::Said(reply.text.clone()));
        }
        request.messages.push(ChatMessage::assistant_calling(
            reply.text.clone(),
            reply.calls.clone(),
        ));
        let mut results = Vec::with_capacity(reply.calls.len());
        for call in &reply.calls {
            if cancel.load(Ordering::Relaxed) {
                return Err(LlmError::Cancelled);
            }
            let key = format!("{}\u{0}{}", call.name, call.input);
            let count = seen.entry(key).or_insert(0);
            *count += 1;
            if *count >= MAX_REPEATS {
                return Ok(Ended {
                    text: reply.text,
                    truncated: false,
                    limit: Some(Limit::Repeating),
                    tools_refused,
                });
            }
            emit(Progress::ToolStarted {
                id: call.id.clone(),
                name: call.name.clone(),
                label: ToolBox::label(call),
            });
            let Outcome {
                content,
                is_error,
                summary,
            } = setup.tools.run(call, cancel);
            emit(Progress::ToolDone {
                id: call.id.clone(),
                summary,
                ok: !is_error,
                content: content.clone(),
            });
            results.push(ToolResult {
                id: call.id.clone(),
                content,
                is_error,
            });
        }
        request.messages.push(ChatMessage::tool_results(results));
    }
    Ok(Ended {
        text: String::new(),
        truncated: false,
        limit: Some(Limit::Rounds),
        tools_refused,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::Diagnostic;
    use crate::script_manager::ai_prompt::Mode;
    use crate::service::llm::mock::{MockProvider, Scripted};
    use serde_json::json;

    fn clean(_: &str) -> Vec<Diagnostic> {
        Vec::new()
    }

    const SCRIPT: &str = "//@version=6\nindicator(\"x\")\nlen = 14\nplot(close)\n";

    fn setup(mode: Mode) -> AgentSetup {
        AgentSetup {
            tools: ToolBox::new(mode, SCRIPT, clean),
            fallback_system: "plain".to_owned(),
        }
    }

    fn request(setup: &AgentSetup) -> ChatRequest {
        ChatRequest {
            model: "m".to_owned(),
            system: "agent".to_owned(),
            messages: vec![ChatMessage::user("make it 20")],
            max_tokens: 100,
            tools: setup.tools.specs(),
        }
    }

    fn go(
        provider: &MockProvider,
        setup: &AgentSetup,
        cancel: &AtomicBool,
    ) -> (Result<Ended, LlmError>, Vec<Progress>) {
        let mut seen = Vec::new();
        let result = run(provider, request(setup), Some(setup), cancel, &mut |p| {
            seen.push(p)
        });
        (result, seen)
    }

    fn edit_turn(id: &str, new: &str) -> Scripted {
        Scripted::call(id, "edit_script", json!({"old": "len = 14", "new": new}))
    }

    #[test]
    fn it_calls_tools_until_the_model_is_done() {
        let setup = setup(Mode::Edit);
        let provider = MockProvider::turns(vec![
            Ok(Scripted::say("Let me change it.").then_call(
                "t1",
                "edit_script",
                json!({"old": "len = 14", "new": "len = 20"}),
            )),
            Ok(Scripted::say("Done: the length is 20.")),
        ]);
        let (result, seen) = go(&provider, &setup, &AtomicBool::new(false));
        let ended = result.unwrap();
        assert_eq!(ended.text, "Done: the length is 20.");
        assert_eq!(ended.limit, None);
        assert!(setup.tools.draft().text().contains("len = 20"));
        // The panel hears the text, the tool and its answer, in order.
        assert!(matches!(&seen[0], Progress::Text(t) if t == "Let me change it."));
        assert!(matches!(&seen[1], Progress::Said(t) if t == "Let me change it."));
        assert!(matches!(&seen[2], Progress::ToolStarted { name, .. } if name == "edit_script"));
        assert!(
            matches!(&seen[3], Progress::ToolDone { ok: true, summary, .. } if summary.starts_with("Edited line 3"))
        );
        // The second request carries the call and its answer.
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let second = &requests[1].messages;
        assert_eq!(second.len(), 3);
        assert_eq!(second[1].tool_calls[0].id, "t1");
        assert_eq!(second[2].tool_results[0].id, "t1");
        assert!(!second[2].tool_results[0].is_error);
    }

    #[test]
    fn several_calls_in_one_turn_are_all_answered_in_order() {
        let setup = setup(Mode::Edit);
        let provider = MockProvider::turns(vec![
            Ok(Scripted::call("a", "read_script", json!({})).then_call("b", "compile", json!({}))),
            Ok(Scripted::say("Looks fine.")),
        ]);
        let (result, _) = go(&provider, &setup, &AtomicBool::new(false));
        assert!(result.is_ok());
        let requests = provider.requests.lock().unwrap();
        let results = &requests[1].messages[2].tool_results;
        assert_eq!(
            results.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn a_failed_call_is_an_answer_the_model_can_act_on() {
        let setup = setup(Mode::Edit);
        let provider = MockProvider::turns(vec![
            Ok(Scripted::call(
                "t1",
                "edit_script",
                json!({"old": "nope", "new": "x"}),
            )),
            Ok(edit_turn("t2", "len = 20")),
            Ok(Scripted::say("Fixed.")),
        ]);
        let (result, seen) = go(&provider, &setup, &AtomicBool::new(false));
        assert_eq!(result.unwrap().text, "Fixed.");
        assert!(
            seen.iter()
                .any(|p| matches!(p, Progress::ToolDone { ok: false, .. }))
        );
        let requests = provider.requests.lock().unwrap();
        assert!(requests[1].messages[2].tool_results[0].is_error);
        assert!(setup.tools.draft().text().contains("len = 20"));
    }

    #[test]
    fn ask_mode_cannot_change_the_draft_even_if_the_model_tries() {
        let setup = setup(Mode::Ask);
        let provider = MockProvider::turns(vec![
            Ok(edit_turn("t1", "len = 20")),
            Ok(Scripted::say("I cannot change it, but here is how.")),
        ]);
        let (result, _) = go(&provider, &setup, &AtomicBool::new(false));
        assert!(result.is_ok());
        assert_eq!(setup.tools.draft().text(), SCRIPT);
        let requests = provider.requests.lock().unwrap();
        assert!(requests[1].messages[2].tool_results[0].is_error);
        // And it was never offered the tool.
        assert!(requests[0].tools.iter().all(|t| t.name != "edit_script"));
    }

    #[test]
    fn it_stops_after_the_round_cap_and_keeps_the_work() {
        let setup = setup(Mode::Edit);
        // A different call every round, so only the cap can end it.
        let turns = (0..MAX_ROUNDS + 3)
            .map(|n| {
                Ok(Scripted::call(
                    &format!("t{n}"),
                    "read_script",
                    json!({"start": n + 1}),
                ))
            })
            .collect();
        let provider = MockProvider::turns(turns);
        let (result, _) = go(&provider, &setup, &AtomicBool::new(false));
        let ended = result.unwrap();
        assert_eq!(ended.limit, Some(Limit::Rounds));
        assert_eq!(provider.requests.lock().unwrap().len(), MAX_ROUNDS);
    }

    #[test]
    fn the_same_call_three_times_ends_the_request() {
        let setup = setup(Mode::Edit);
        let turns = (0..6)
            .map(|n| Ok(Scripted::call(&format!("t{n}"), "compile", json!({}))))
            .collect();
        let provider = MockProvider::turns(turns);
        let (result, _) = go(&provider, &setup, &AtomicBool::new(false));
        assert_eq!(result.unwrap().limit, Some(Limit::Repeating));
        assert_eq!(provider.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn cancelling_stops_before_the_next_call_and_keeps_what_was_done() {
        let setup = setup(Mode::Edit);
        let provider = MockProvider::turns(vec![
            Ok(edit_turn("t1", "len = 20")),
            Ok(Scripted::say("never reached")),
        ]);
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let result = run(
            &provider,
            request(&setup),
            Some(&setup),
            &cancel,
            &mut |p| {
                if matches!(p, Progress::ToolDone { .. }) {
                    calls += 1;
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert_eq!(result, Err(LlmError::Cancelled));
        assert_eq!(calls, 1);
        assert!(setup.tools.draft().text().contains("len = 20"));
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_server_that_refuses_tools_is_asked_again_without_them() {
        let setup = setup(Mode::Edit);
        let provider = MockProvider::turns(vec![
            Err(LlmError::http(400, "tools are not supported")),
            Ok(Scripted::say("```pine\n//@version=6\n```")),
        ]);
        let (result, _) = go(&provider, &setup, &AtomicBool::new(false));
        let ended = result.unwrap();
        assert!(ended.tools_refused);
        let requests = provider.requests.lock().unwrap();
        assert!(!requests[0].tools.is_empty());
        assert!(requests[1].tools.is_empty());
        assert_eq!(requests[1].system, "plain");
    }

    #[test]
    fn a_refusal_is_not_retried_forever_and_other_errors_pass_through() {
        let setup = setup(Mode::Edit);
        let provider = MockProvider::turns(vec![
            Err(LlmError::http(400, "tools are not supported")),
            Err(LlmError::http(400, "still no")),
        ]);
        let (result, _) = go(&provider, &setup, &AtomicBool::new(false));
        assert!(matches!(result, Err(LlmError::Http { status: 400, .. })));
        let provider = MockProvider::turns(vec![Err(LlmError::rate_limited("slow down"))]);
        let (result, _) = go(&provider, &setup, &AtomicBool::new(false));
        assert!(matches!(result, Err(LlmError::RateLimited { .. })));
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn without_tools_it_is_one_ordinary_request() {
        let provider = MockProvider::replying(vec![MockProvider::text(&["Hel", "lo"])]);
        let request = ChatRequest {
            model: "m".to_owned(),
            system: "s".to_owned(),
            messages: vec![ChatMessage::user("hi")],
            max_tokens: 10,
            tools: Vec::new(),
        };
        let mut seen = Vec::new();
        let ended = run(
            &provider,
            request,
            None,
            &AtomicBool::new(false),
            &mut |p| seen.push(p),
        )
        .unwrap();
        assert_eq!(ended.text, "Hello");
        assert_eq!(seen.len(), 2);
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_reply_cut_off_by_the_token_limit_says_so() {
        let setup = setup(Mode::Edit);
        let provider = MockProvider::turns(vec![Ok(Scripted {
            pieces: vec!["part".to_owned()],
            truncated: true,
            ..Scripted::default()
        })]);
        let (result, _) = go(&provider, &setup, &AtomicBool::new(false));
        assert!(result.unwrap().truncated);
    }
}
