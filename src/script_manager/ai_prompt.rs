//! What the assistant says to the model and how it reads the answer. Pure text in, text out: no
//! I/O and no egui, so every rule here is a plain test.
//!
//! The contract with the model is "explain briefly, then give the whole script in one fenced
//! block". Whole script, never a patch: a model asked for a diff gets line numbers and context
//! wrong often enough that applying one blindly would corrupt the user's code, whereas a whole
//! script can be compiled and diffed against the buffer before anything changes.

use super::demo::BUNDLED;
use super::drawer;
use crate::pine::symbol_catalog;
use crate::pine::token::Diagnostic;
use crate::service::llm::ChatMessage;
use serde::{Deserialize, Serialize};

/// Same cap as the MCP server's script edits: a bigger script is a runaway, not a script.
pub const MAX_SOURCE_CHARS: usize = 200_000;
/// The exchanges kept in a request. Older ones are dropped: the script itself is always resent.
const HISTORY_TURNS: usize = 6;
const MAX_DIAGNOSTIC_LINES: usize = 20;
const OMITTED: &str =
    "[The script that was proposed here is omitted. The current script is in the last message.]";

/// What a request may do. In Ask the assistant only talks: the script is never offered for
/// applying, whatever the model writes. In Edit it may propose a whole new script.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    #[serde(alias = "plan")]
    Ask,
    Edit,
}

impl Mode {
    pub fn title(self) -> &'static str {
        match self {
            Self::Ask => "Ask",
            Self::Edit => "Edit",
        }
    }
}

/// What the Implement button sends after a plan.
pub const IMPLEMENT_MESSAGE: &str =
    "Implement the plan above. Make exactly those changes and return the complete updated script.";

/// One line of the conversation as the panel keeps it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Turn {
    /// What the user typed.
    User(String),
    /// The model's whole reply, code block included, and the mode the request was made in.
    Assistant { text: String, mode: Mode },
}

pub fn system_prompt(mode: Mode) -> String {
    let mut prompt = match mode {
        Mode::Edit => String::from(
            "You edit Pine Script v6 for DegenView, a desktop charting app with its own Pine \
interpreter. The user's current script and their request are in the last message.\n\n\
How to answer:\n\
- If the user asks for a change, reply with one or two sentences saying what you changed, then the \
COMPLETE updated script in a single ```pine fenced block. Never a fragment, a patch or a diff.\n\
- Keep everything the user did not ask you to change: names, inputs, comments, formatting.\n\
- Start the script with //@version=6 and keep its existing indicator(), strategy() or library() \
declaration unless asked to change it.\n\
- If the user asks a question, answer in prose with no code block.\n\
- If the request cannot be done in this Pine implementation, say so and why instead of guessing.\n\
- If the message lists compiler errors, fix them.\n\
- Write plain Markdown: short paragraphs and `code` for names.\n\n",
        ),
        Mode::Ask => String::from(
            "You help with Pine Script v6 for DegenView, a desktop charting app with its own Pine \
interpreter. The user's current script and their question are in the last message. You are in ASK \
mode: you cannot change the script, and nothing you write is applied to it.\n\n\
How to answer:\n\
- Answer the question directly and concisely, referring to names and lines in their script.\n\
- If they want a change, give a short numbered plan: what to change and why, one step per item. Do \
not write the code for it.\n\
- Never write the complete script. A short snippet (under about 15 lines) in a ```pine fenced block is \
fine when it makes the point clearer.\n\
- If something cannot be done in this Pine implementation, say so and why instead of guessing.\n\
- Write plain Markdown: short paragraphs, lists, and `code` for names.\n\n",
        ),
    };
    prompt.push_str(
        "This interpreter is not TradingView, and some Pine is not implemented. Do not use these:\n",
    );
    // Every Pine function is implemented now, so the list may be empty: no line, not a blank one.
    if !symbol_catalog::RESERVED_FUNCTIONS.is_empty() {
        prompt.push_str(&format!(
            "- functions: {}\n",
            symbol_catalog::RESERVED_FUNCTIONS.join(", ")
        ));
    }
    prompt.push_str(&format!(
        "- variables (they evaluate to na): {}\n",
        symbol_catalog::UNIMPLEMENTED_VARIABLES.join(", "),
    ));
    prompt.push_str(
        "\nPrefer plain, well-known Pine over clever Pine.\n\nTwo scripts that run correctly here, for style:\n",
    );
    for name in ["RSI Zones", "EMA Cross Strategy"] {
        if let Some(demo) = BUNDLED.iter().find(|demo| demo.name == name) {
            prompt.push_str(&format!("\n```pine\n{}\n```\n", demo.source.trim_end()));
        }
    }
    prompt
}

/// A fence longer than any run of backticks inside `text`, so code can never close it early.
fn fence_for(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

/// The last message: the script as it is now, what the compiler says about it, and the request.
pub fn user_turn(source: &str, request: &str, diagnostics: &[Diagnostic]) -> String {
    let fence = fence_for(source);
    let mut text = format!("Current script:\n{fence}pine\n{source}\n{fence}\n\n");
    let problems = drawer::copy_text(diagnostics);
    if problems.is_empty() {
        text.push_str("Compiler diagnostics for it: none.\n\n");
    } else {
        let mut lines: Vec<&str> = problems.lines().collect();
        let more = lines.len().saturating_sub(MAX_DIAGNOSTIC_LINES);
        lines.truncate(MAX_DIAGNOSTIC_LINES);
        text.push_str("Compiler diagnostics for it (code · line:column  message):\n");
        text.push_str(&lines.join("\n"));
        if more > 0 {
            text.push_str(&format!("\n… and {more} more."));
        }
        text.push_str("\n\n");
    }
    text.push_str("Request: ");
    text.push_str(request.trim());
    text
}

/// The messages for the next request: recent turns as typed, with earlier scripts left out, then
/// the current script and the new request.
pub fn build_messages(
    history: &[Turn],
    source: &str,
    request: &str,
    diagnostics: &[Diagnostic],
) -> Vec<ChatMessage> {
    let start = history.len().saturating_sub(HISTORY_TURNS);
    let mut messages: Vec<ChatMessage> = history[start..]
        .iter()
        .map(|turn| match turn {
            Turn::User(text) => ChatMessage::user(text.clone()),
            // An edit's script is dropped from history (the current one is resent); a plan's
            // snippets are part of what was said, so they stay.
            Turn::Assistant {
                text,
                mode: Mode::Edit,
            } => ChatMessage::assistant(without_script(text)),
            Turn::Assistant {
                text,
                mode: Mode::Ask,
            } => ChatMessage::assistant(text.clone()),
        })
        .collect();
    // Providers want the conversation to open with the user and to alternate.
    while messages
        .first()
        .is_some_and(|m| m.role == crate::service::llm::Role::Assistant)
    {
        messages.remove(0);
    }
    messages.push(ChatMessage::user(user_turn(source, request, diagnostics)));
    messages
}

/// A reply split into what the model said and the script it gave.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Reply {
    /// The text outside the script, trimmed.
    pub prose: String,
    /// The script, if the reply has one.
    pub script: Option<String>,
    /// The script's closing fence has arrived. While a reply streams this is false.
    pub complete: bool,
}

struct Block {
    start: usize,
    end: usize,
    code: String,
    closed: bool,
}

/// Every fenced block, in order. A fence is a line starting with three or more backticks; it
/// closes on a line of at least as many backticks and nothing else.
fn blocks(text: &str) -> Vec<Block> {
    let mut found = Vec::new();
    let mut open: Option<(usize, usize, usize)> = None; // (fence start, code start, fence width)
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let ticks = trimmed.chars().take_while(|c| *c == '`').count();
        match open {
            None if ticks >= 3 => open = Some((offset, offset + line.len(), ticks)),
            Some((start, code_start, width))
                if ticks >= width && trimmed.trim() == "`".repeat(ticks) =>
            {
                found.push(Block {
                    start,
                    end: offset + line.len(),
                    code: text[code_start..offset].to_owned(),
                    closed: true,
                });
                open = None;
            }
            _ => {}
        }
        offset += line.len();
    }
    if let Some((start, code_start, _)) = open {
        found.push(Block {
            start,
            end: text.len(),
            code: text[code_start..].to_owned(),
            closed: false,
        });
    }
    found
}

fn looks_like_a_script(code: &str) -> bool {
    code.contains("//@version")
        || ["indicator(", "strategy(", "library("]
            .iter()
            .any(|declaration| {
                code.lines()
                    .any(|line| line.trim_start().starts_with(declaration))
            })
}

pub fn split_reply(text: &str) -> Reply {
    let blocks = blocks(text);
    let chosen = blocks
        .iter()
        .position(|block| looks_like_a_script(&block.code))
        .or_else(|| (blocks.len() == 1).then_some(0));
    let Some(index) = chosen else {
        return Reply {
            prose: text.trim().to_owned(),
            script: None,
            complete: true,
        };
    };
    let block = &blocks[index];
    let prose = format!("{}{}", &text[..block.start], &text[block.end..]);
    let script = block.code.trim_end().to_owned();
    Reply {
        prose: prose.trim().to_owned(),
        script: (!script.trim().is_empty()).then_some(script),
        complete: block.closed,
    }
}

/// Whether a reply reads as a plan: two or more numbered steps outside any code. Decides whether
/// the panel offers to implement it.
pub fn looks_like_plan(text: &str) -> bool {
    let mut open: Option<usize> = None;
    let mut steps = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        let ticks = trimmed.chars().take_while(|c| *c == '`').count();
        match open {
            Some(width) if ticks >= width && trimmed.trim_matches('`').is_empty() => open = None,
            Some(_) => {}
            None if ticks >= 3 => open = Some(ticks),
            None => {
                let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
                let after = &trimmed[digits..];
                if (1..=2).contains(&digits) && (after.starts_with(". ") || after.starts_with(") "))
                {
                    steps += 1;
                }
            }
        }
    }
    steps >= 2
}

/// An earlier reply with its script replaced by a marker, so history stays short.
fn without_script(text: &str) -> String {
    let blocks = blocks(text);
    let Some(block) = blocks
        .iter()
        .find(|block| looks_like_a_script(&block.code))
        .or_else(|| (blocks.len() == 1).then(|| &blocks[0]))
    else {
        return text.to_owned();
    };
    format!("{}{OMITTED}{}", &text[..block.start], &text[block.end..])
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::token::{Diagnostic, DiagnosticCategory, SourceRange};
    use crate::service::llm::Role;

    fn error(message: &str) -> Diagnostic {
        Diagnostic::error(
            "PINE1001",
            DiagnosticCategory::Semantic,
            message,
            SourceRange::ZERO,
        )
    }

    #[test]
    fn the_system_prompt_states_the_contract_and_the_unsupported_names() {
        let prompt = system_prompt(Mode::Edit);
        assert!(prompt.contains("COMPLETE updated script"));
        assert!(
            !prompt.contains("- functions:"),
            "no function is unimplemented"
        );
        assert!(prompt.contains("syminfo.sector"));
        // Both examples are bundled scripts that compile with no diagnostics (a repo rule).
        assert!(prompt.contains("indicator(\"RSI Zones\""));
        assert!(prompt.contains("strategy("));
    }

    #[test]
    fn the_last_message_carries_the_script_the_diagnostics_and_the_request() {
        let text = user_turn("plot(close)", "  make it red ", &[]);
        assert!(text.starts_with("Current script:\n```pine\nplot(close)\n```"));
        assert!(text.contains("Compiler diagnostics for it: none."));
        assert!(text.ends_with("Request: make it red"));

        let text = user_turn("plot(clse)", "fix", &[error("Unknown identifier clse")]);
        assert!(text.contains("PINE1001 · "));
        assert!(text.contains("  Unknown identifier clse"));
    }

    #[test]
    fn code_containing_backticks_cannot_close_the_fence_early() {
        let source = "// ```\nplot(close)";
        assert_eq!(fence_for(source), "````");
        let text = user_turn(source, "x", &[]);
        assert!(text.contains("````pine\n// ```\nplot(close)\n````"));
        assert_eq!(fence_for("plain"), "```");
    }

    #[test]
    fn a_long_diagnostic_list_is_capped() {
        let many: Vec<_> = (0..30).map(|i| error(&format!("problem {i}"))).collect();
        let text = user_turn("x", "fix", &many);
        assert!(text.contains("problem 19"));
        assert!(!text.contains("problem 20"));
        assert!(text.contains("… and 10 more."));
    }

    #[test]
    fn history_drops_old_scripts_and_old_turns_and_starts_with_the_user() {
        let reply = "Done.\n```pine\n//@version=6\nplot(close)\n```\n";
        let history = vec![
            Turn::User("first".into()),
            Turn::Assistant {
                text: reply.into(),
                mode: Mode::Edit,
            },
            Turn::User("second".into()),
            Turn::Assistant {
                text: "Sure, a question needs no code.".into(),
                mode: Mode::Edit,
            },
        ];
        let messages = build_messages(&history, "src", "third", &[]);
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0], ChatMessage::user("first"));
        assert!(
            messages[1]
                .content
                .starts_with("Done.\n[The script that was proposed")
        );
        assert!(!messages[1].content.contains("plot(close)"));
        assert_eq!(messages[3].content, "Sure, a question needs no code.");
        assert_eq!(messages[4].role, Role::User);
        assert!(messages[4].content.contains("Request: third"));

        // Only the most recent turns go out, and the first one sent is always the user's.
        let long: Vec<Turn> = (0..10)
            .flat_map(|i| {
                [
                    Turn::User(format!("u{i}")),
                    Turn::Assistant {
                        text: format!("a{i}"),
                        mode: Mode::Edit,
                    },
                ]
            })
            .collect();
        let messages = build_messages(&long, "src", "now", &[]);
        assert_eq!(messages.len(), HISTORY_TURNS + 1);
        assert_eq!(messages[0].role, Role::User);
    }

    #[test]
    fn a_reply_splits_into_prose_and_script() {
        let reply = split_reply(
            "I made it red.\n\n```pine\n//@version=6\nplot(close, color=color.red)\n```\n\nEnjoy.",
        );
        assert_eq!(reply.prose, "I made it red.\n\n\nEnjoy.");
        assert_eq!(
            reply.script.as_deref(),
            Some("//@version=6\nplot(close, color=color.red)")
        );
        assert!(reply.complete);
    }

    #[test]
    fn a_reply_with_no_code_is_all_prose() {
        let reply = split_reply("The RSI is a momentum oscillator.");
        assert_eq!(reply.script, None);
        assert_eq!(reply.prose, "The RSI is a momentum oscillator.");
        assert!(reply.complete);
    }

    #[test]
    fn a_streaming_reply_reports_the_script_so_far_as_incomplete() {
        let reply = split_reply("Changing it.\n```pine\n//@version=6\nplo");
        assert_eq!(reply.script.as_deref(), Some("//@version=6\nplo"));
        assert!(!reply.complete);
        assert_eq!(reply.prose, "Changing it.");
    }

    #[test]
    fn the_script_block_wins_over_other_snippets() {
        let text = "Use `ta.rsi`:\n```\nta.rsi(close, 14)\n```\nFull script:\n```pine\n//@version=6\nindicator(\"x\")\n```\n";
        let reply = split_reply(text);
        assert_eq!(
            reply.script.as_deref(),
            Some("//@version=6\nindicator(\"x\")")
        );
        assert!(reply.prose.contains("ta.rsi(close, 14)"));
    }

    #[test]
    fn several_unmarked_snippets_are_not_guessed_at() {
        let text = "Either:\n```\na = 1\n```\nor:\n```\nb = 2\n```\n";
        assert_eq!(split_reply(text).script, None);
        // But one unmarked block is the script.
        let one = split_reply("Here:\n```\nplot(close)\n```\n");
        assert_eq!(one.script.as_deref(), Some("plot(close)"));
    }

    #[test]
    fn crlf_and_a_longer_fence_are_handled() {
        let reply = split_reply("ok\r\n````pine\r\n// ```\r\nplot(close)\r\n````\r\n");
        assert_eq!(reply.script.as_deref(), Some("// ```\r\nplot(close)"));
        assert!(reply.complete);
    }

    #[test]
    fn an_empty_block_is_no_script() {
        assert_eq!(split_reply("```pine\n```\n").script, None);
    }

    #[test]
    fn non_ascii_prose_around_a_block_survives() {
        let reply = split_reply("日本語 🙂\n```pine\n//@version=6\n```\nあとで");
        assert_eq!(reply.script.as_deref(), Some("//@version=6"));
        assert_eq!(reply.prose, "日本語 🙂\nあとで");
    }

    #[test]
    fn ask_mode_says_it_cannot_change_the_script_and_edit_mode_asks_for_the_whole_one() {
        let plan = system_prompt(Mode::Ask);
        assert!(plan.contains("ASK"));
        assert!(plan.contains("Never write the complete script"));
        assert!(!plan.contains("COMPLETE updated script"));
        let edit = system_prompt(Mode::Edit);
        assert!(edit.contains("COMPLETE updated script"));
        assert!(!edit.contains("ASK"));
        // Both carry the unsupported-name list and the examples.
        for prompt in [&plan, &edit] {
            assert!(prompt.contains("syminfo.sector"));
            assert!(prompt.contains("indicator(\"RSI Zones\""));
        }
    }

    #[test]
    fn the_default_mode_is_plan() {
        assert_eq!(Mode::default(), Mode::Ask);
        assert_eq!(Mode::Ask.title(), "Ask");
    }

    #[test]
    fn a_plans_snippets_stay_in_history_but_an_edits_script_does_not() {
        let snippet = "Use this:\n```pine\nlength = input.int(14)\n```\n1. Add it\n2. Use it";
        let history = vec![
            Turn::User("how".into()),
            Turn::Assistant {
                text: snippet.into(),
                mode: Mode::Ask,
            },
            Turn::User("do it".into()),
            Turn::Assistant {
                text: "Done.\n```pine\n//@version=6\nplot(close)\n```".into(),
                mode: Mode::Edit,
            },
        ];
        let messages = build_messages(&history, "src", "next", &[]);
        assert_eq!(messages[1].content, snippet);
        assert!(!messages[3].content.contains("plot(close)"));
        assert!(messages[3].content.contains("omitted"));
    }

    #[test]
    fn numbered_steps_outside_code_make_a_plan() {
        assert!(looks_like_plan(
            "Here is how:\n1. Add an input\n2. Use it\n"
        ));
        assert!(looks_like_plan("1) one\n2) two"));
        assert!(looks_like_plan("  1. indented\n  2. also"));
        // One step, a year, or steps inside a code block are not a plan.
        assert!(!looks_like_plan("1. only one"));
        assert!(!looks_like_plan("In 2024. Then 2025. It rose."));
        assert!(!looks_like_plan("```\n1. a\n2. b\n```"));
        assert!(!looks_like_plan("Just an answer."));
        assert!(!looks_like_plan(""));
        // Steps after a closed block count.
        assert!(looks_like_plan("```\nx\n```\n1. a\n2. b"));
    }

    #[test]
    fn the_implement_message_asks_for_the_whole_script() {
        assert!(IMPLEMENT_MESSAGE.contains("complete updated script"));
    }
}
