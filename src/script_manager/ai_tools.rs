//! The tools the assistant may call, and the private copy of the script they work on.
//!
//! The assistant never touches the editor. A request starts with a [`Draft`], a copy of the open
//! script, and every tool reads or changes that copy. What comes out is compared with the script the
//! request started from and offered as a proposal, which the user reviews and applies as before. No
//! egui and no store here: a tool is text in, text out, so every rule is a plain test.
//!
//! The draft is kept with `\n` line endings whatever the file uses. A model writes `\n`, so an exact
//! match for text it copied would never succeed on a CRLF file; the file's own endings come back when
//! the proposal is built (`ai::match_line_endings`).

use super::ai_profile::ProfileTool;
use super::ai_prompt::Mode;
use super::debug::DebugFeed;
use super::drawer;
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use crate::service::llm::{ToolCall, ToolSpec};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// What a tool's answer may hold. The model gets a cut one with a note saying so.
const MAX_RESULT_CHARS: usize = 8_000;
/// Lines `read_script` returns for one call.
const MAX_READ_LINES: usize = 400;
const MAX_DIAGNOSTIC_LINES: usize = 20;
/// Problems listed after an edit; the full list is one `compile` away.
const MAX_PROBLEMS_AFTER_EDIT: usize = 5;
/// Places listed when `old` matches more than once.
const MAX_MATCHES_LISTED: usize = 6;

/// How a script is compiled for the diagnostics a tool reports.
pub type Compiler = fn(&str) -> Vec<Diagnostic>;

pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The assistant's working copy. Shared, so the panel can read how far it got when the user stops
/// a request that is still running.
#[derive(Clone, Default)]
pub struct Draft(Arc<Mutex<String>>);

impl Draft {
    pub fn new(source: &str) -> Self {
        Self(Arc::new(Mutex::new(normalize(source))))
    }

    pub fn text(&self) -> String {
        self.0.lock().map(|text| text.clone()).unwrap_or_default()
    }

    fn set(&self, text: String) {
        if let Ok(mut current) = self.0.lock() {
            *current = text;
        }
    }
}

/// What a tool call came to: what the model is told, and the line the panel shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outcome {
    pub content: String,
    pub is_error: bool,
    pub summary: String,
}

impl Outcome {
    fn ok(content: String, summary: String) -> Self {
        Self {
            content: cap(content),
            is_error: false,
            summary,
        }
    }

    pub(super) fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            summary: message.clone(),
            content: message,
            is_error: true,
        }
    }
}

struct Tool {
    name: &'static str,
    /// Changes the draft, so only Edit mode has it.
    writes: bool,
}

const TOOLS: [Tool; 5] = [
    Tool {
        name: "read_script",
        writes: false,
    },
    Tool {
        name: "compile",
        writes: false,
    },
    Tool {
        name: "profile",
        writes: false,
    },
    Tool {
        name: "edit_script",
        writes: true,
    },
    Tool {
        name: "write_script",
        writes: true,
    },
];

fn allowed(tool: &Tool, mode: Mode) -> bool {
    !tool.writes || mode == Mode::Edit
}

/// The tools of one request, and the draft they work on.
#[derive(Clone)]
pub struct ToolBox {
    mode: Mode,
    draft: Draft,
    compile: Compiler,
    profile: Arc<ProfileTool>,
}

impl ToolBox {
    pub fn new(mode: Mode, source: &str, compile: Compiler) -> Self {
        Self {
            mode,
            draft: Draft::new(source),
            compile,
            profile: Arc::new(ProfileTool::new(None)),
        }
    }

    /// Gives `profile` the preview chart's bars to run the draft on. Without them it answers that
    /// there is nothing to run on.
    pub fn with_bars(mut self, feed: Option<DebugFeed>) -> Self {
        self.profile = Arc::new(ProfileTool::new(feed));
        self
    }

    pub fn draft(&self) -> &Draft {
        &self.draft
    }

    /// What the model is offered in this mode.
    pub fn specs(&self) -> Vec<ToolSpec> {
        TOOLS
            .iter()
            .filter(|tool| allowed(tool, self.mode))
            .map(|tool| spec(tool.name))
            .collect()
    }

    /// What the panel says while a call runs.
    pub fn label(call: &ToolCall) -> String {
        match call.name.as_str() {
            "read_script" => match (line_arg(&call.input, "start"), line_arg(&call.input, "end")) {
                (Some(start), Some(end)) => format!("Reading lines {start}\u{2013}{end}"),
                (Some(start), None) => format!("Reading from line {start}"),
                _ => "Reading the script".to_owned(),
            },
            "compile" => "Compiling the draft".to_owned(),
            "profile" => "Profiling the draft".to_owned(),
            "edit_script" => "Editing the script".to_owned(),
            "write_script" => "Rewriting the script".to_owned(),
            other => format!("Calling {other}"),
        }
    }

    /// Runs one call. A call the model got wrong is an error result it can read and correct, never
    /// a failure of the request.
    pub fn run(&self, call: &ToolCall, cancel: &AtomicBool) -> Outcome {
        if let Some(reason) = &call.malformed {
            return Outcome::error(format!(
                "Could not read the arguments of `{}`. {reason} Call it again with a JSON object.",
                call.name
            ));
        }
        let Some(tool) = TOOLS.iter().find(|tool| tool.name == call.name) else {
            let available: Vec<&str> = TOOLS
                .iter()
                .filter(|tool| allowed(tool, self.mode))
                .map(|tool| tool.name)
                .collect();
            return Outcome::error(format!(
                "There is no tool named `{}`. Available: {}.",
                call.name,
                available.join(", ")
            ));
        };
        if !allowed(tool, self.mode) {
            return Outcome::error(format!(
                "`{}` is not available here: you are in {} mode and cannot change the script.",
                call.name,
                self.mode.title()
            ));
        }
        match tool.name {
            "read_script" => self.read(&call.input),
            "compile" => self.compile_draft(),
            "profile" => self.profile_draft(cancel),
            "edit_script" => self.edit(&call.input),
            _ => self.write(&call.input),
        }
    }

    fn read(&self, input: &Value) -> Outcome {
        let text = self.draft.text();
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        if total == 0 {
            return Outcome::ok(
                "The script is empty.".to_owned(),
                "Read the script".to_owned(),
            );
        }
        let start = line_arg(input, "start").unwrap_or(1);
        if start == 0 || start > total {
            return Outcome::error(format!(
                "Line {start} is outside the script, which has {total} lines."
            ));
        }
        let limit = start + MAX_READ_LINES - 1;
        let end = line_arg(input, "end")
            .unwrap_or(total)
            .min(total)
            .min(limit);
        if end < start {
            return Outcome::error(format!("`end` ({end}) is before `start` ({start})."));
        }
        let mut out = format!("Lines {start}\u{2013}{end} of {total} of the draft:\n");
        for (index, line) in lines[start - 1..end].iter().enumerate() {
            out.push_str(&format!("{}: {line}\n", start + index));
        }
        if end < total && line_arg(input, "end").is_none_or(|asked| asked > end) {
            out.push_str(&format!(
                "(Stopped at line {end}; call again with `start` {} for more.)\n",
                end + 1
            ));
        }
        let summary = if start == 1 && end == total {
            format!("Read the script \u{b7} {total} lines")
        } else {
            format!("Read lines {start}\u{2013}{end}")
        };
        Outcome::ok(out, summary)
    }

    fn compile_draft(&self) -> Outcome {
        let diagnostics = (self.compile)(&self.draft.text());
        let summary = drawer::problems_summary(&diagnostics);
        if diagnostics.is_empty() {
            return Outcome::ok(
                "No problems: the draft compiles with no errors or warnings.".to_owned(),
                format!("Compiled \u{b7} {summary}"),
            );
        }
        let all = drawer::copy_text(&diagnostics);
        let mut lines: Vec<&str> = all.lines().collect();
        let more = lines.len().saturating_sub(MAX_DIAGNOSTIC_LINES);
        lines.truncate(MAX_DIAGNOSTIC_LINES);
        let mut out = format!(
            "{summary} (code \u{b7} line:column  message):\n{}",
            lines.join("\n")
        );
        if more > 0 {
            out.push_str(&format!("\n\u{2026} and {more} more."));
        }
        Outcome::ok(out, format!("Compiled \u{b7} {summary}"))
    }

    /// Profiles the draft, which has to compile first: a script with errors has nothing to run.
    fn profile_draft(&self, cancel: &AtomicBool) -> Outcome {
        let text = self.draft.text();
        let diagnostics = (self.compile)(&text);
        if diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
        {
            let all = drawer::copy_text(&diagnostics);
            let shown: Vec<&str> = all.lines().take(MAX_PROBLEMS_AFTER_EDIT).collect();
            return Outcome::error(format!(
                "The draft has errors, so it cannot be profiled. Fix them first:\n{}",
                shown.join("\n")
            ));
        }
        let mut outcome = self.profile.run(&text, cancel);
        outcome.content = cap(outcome.content);
        outcome
    }

    fn edit(&self, input: &Value) -> Outcome {
        let (Some(old), Some(new)) = (string_arg(input, "old"), string_arg(input, "new")) else {
            return Outcome::error("`edit_script` needs `old` and `new`, both strings.");
        };
        let (old, new) = (normalize(old), normalize(new));
        if old.is_empty() {
            return Outcome::error(
                "`old` is empty. Give the exact text to replace, with enough lines around it to be unique.",
            );
        }
        if old == new {
            return Outcome::error("`old` and `new` are the same, so nothing would change.");
        }
        let text = self.draft.text();
        let places: Vec<usize> = text.match_indices(&old).map(|(at, _)| at).collect();
        match places.as_slice() {
            [] => Outcome::error(not_found(&text, &old)),
            [at] => {
                let start_line = text[..*at].matches('\n').count() + 1;
                let edited = format!("{}{new}{}", &text[..*at], &text[at + old.len()..]);
                self.draft.set(edited);
                let end_line = start_line + new.matches('\n').count();
                self.after_change("Edited", start_line, end_line)
            }
            many => {
                let lines: Vec<String> = many
                    .iter()
                    .take(MAX_MATCHES_LISTED)
                    .map(|at| (text[..*at].matches('\n').count() + 1).to_string())
                    .collect();
                Outcome::error(format!(
                    "`old` matches {} places (lines {}{}). Include more of the surrounding lines so it matches once.",
                    many.len(),
                    lines.join(", "),
                    if many.len() > MAX_MATCHES_LISTED {
                        ", \u{2026}"
                    } else {
                        ""
                    }
                ))
            }
        }
    }

    fn write(&self, input: &Value) -> Outcome {
        let Some(text) = string_arg(input, "text") else {
            return Outcome::error("`write_script` needs `text`, a string with the whole script.");
        };
        if text.trim().is_empty() {
            return Outcome::error("`text` is empty. Give the whole script.");
        }
        let text = normalize(text);
        let lines = text.lines().count().max(1);
        self.draft.set(text);
        self.after_change("Rewrote", 1, lines)
    }

    /// What a change did, without a second call: the model sees at once whether it still compiles.
    fn after_change(&self, verb: &str, first: usize, last: usize) -> Outcome {
        let text = self.draft.text();
        let diagnostics = (self.compile)(&text);
        let summary = drawer::problems_summary(&diagnostics);
        let total = text.lines().count();
        let place = if verb == "Rewrote" {
            format!("the whole script ({total} lines)")
        } else if first == last {
            format!("line {first}")
        } else {
            format!("lines {first}\u{2013}{last}")
        };
        let mut out = format!("{verb} {place}. The draft has {total} lines. Now: {summary}.");
        if diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
        {
            let all = drawer::copy_text(&diagnostics);
            let shown: Vec<&str> = all.lines().take(MAX_PROBLEMS_AFTER_EDIT).collect();
            out.push_str(&format!("\n{}", shown.join("\n")));
        }
        Outcome::ok(out, format!("{verb} {place} \u{b7} {summary}"))
    }
}

/// Why `old` was not found, with the most likely reason first.
fn not_found(text: &str, old: &str) -> String {
    let mut message = String::from(
        "`old` was not found in the draft. It must match exactly, spaces and indentation included.",
    );
    if text.contains(old.trim()) {
        message.push_str(" The text is there, but not with the spaces or blank lines around it that you gave: copy it from `read_script`.");
    } else {
        message.push_str(
            " Do not include the `12: ` line numbers `read_script` prints. Read the lines again, then copy from them.",
        );
    }
    message
}

fn string_arg<'a>(input: &'a Value, name: &str) -> Option<&'a str> {
    input.get(name).and_then(Value::as_str)
}

fn line_arg(input: &Value, name: &str) -> Option<usize> {
    let value = input.get(name)?;
    value
        .as_u64()
        .map(|n| n as usize)
        // Some models send numbers as text.
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

/// Cuts a long answer on a character boundary and says so.
fn cap(text: String) -> String {
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text;
    }
    let kept: String = text.chars().take(MAX_RESULT_CHARS).collect();
    let cut = text.chars().count() - MAX_RESULT_CHARS;
    format!("{kept}\n\u{2026} (cut: {cut} more characters)")
}

fn spec(name: &str) -> ToolSpec {
    let (description, schema) = match name {
        "read_script" => (
            "Reads lines of the draft, which is your working copy of the script, with line numbers. \
The numbers are only for reference: never include them in text you give to another tool. With no \
arguments it reads from line 1 (at most 400 lines at a time).",
            json!({"type": "object", "properties": {
                "start": {"type": "integer", "description": "First line, counting from 1."},
                "end": {"type": "integer", "description": "Last line, inclusive."},
            }}),
        ),
        "compile" => (
            "Compiles the draft and lists its errors and warnings. Takes no arguments.",
            json!({"type": "object", "properties": {}}),
        ),
        "profile" => (
            "Runs the draft on the preview chart's bars (at most 500) and says where the time goes: \
the hottest lines with their share, time and number of executions, the hottest block and function. \
Use it to find what is slow before changing anything, and again after a change to check it. The \
draft must compile. Takes no arguments.",
            json!({"type": "object", "properties": {}}),
        ),
        "edit_script" => (
            "Replaces one piece of the draft. `old` must match the draft exactly (spaces, indentation \
and all) and exactly once, so include enough surrounding lines to be unique. Prefer several small \
edits to rewriting. The answer says whether the draft still compiles.",
            json!({"type": "object", "properties": {
                "old": {"type": "string", "description": "The exact text to replace, without line numbers."},
                "new": {"type": "string", "description": "What to put there. May be empty to delete."},
            }, "required": ["old", "new"]}),
        ),
        _ => (
            "Replaces the whole draft. Use it only when most of the script changes; otherwise use \
`edit_script`. The answer says whether the draft still compiles.",
            json!({"type": "object", "properties": {
                "text": {"type": "string", "description": "The complete script."},
            }, "required": ["text"]}),
        ),
    };
    ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        schema,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::{DiagnosticCategory, SourceRange};

    fn clean(_: &str) -> Vec<Diagnostic> {
        Vec::new()
    }

    fn failing(source: &str) -> Vec<Diagnostic> {
        if source.contains("oops") {
            vec![Diagnostic::error(
                "PINE3001",
                DiagnosticCategory::Runtime,
                "Undeclared identifier 'oops'",
                SourceRange::ZERO,
            )]
        } else {
            Vec::new()
        }
    }

    const SCRIPT: &str = "//@version=6\nindicator(\"x\")\nlen = 14\nplot(close)\nplot(open)\n";

    impl ToolBox {
        fn run_for_test(&self, call: &ToolCall) -> Outcome {
            self.run(call, &AtomicBool::new(false))
        }
    }

    fn call(name: &str, input: Value) -> ToolCall {
        ToolCall {
            id: "t".to_owned(),
            name: name.to_owned(),
            input,
            malformed: None,
        }
    }

    fn edit_box() -> ToolBox {
        ToolBox::new(Mode::Edit, SCRIPT, clean)
    }

    #[test]
    fn ask_has_the_reading_tools_and_edit_adds_the_writing_ones() {
        let names = |mode| {
            ToolBox::new(mode, SCRIPT, clean)
                .specs()
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(Mode::Ask), ["read_script", "compile", "profile"]);
        assert_eq!(
            names(Mode::Edit),
            [
                "read_script",
                "compile",
                "profile",
                "edit_script",
                "write_script"
            ]
        );
    }

    #[test]
    fn every_spec_is_an_object_schema() {
        for spec in edit_box().specs() {
            assert_eq!(spec.schema["type"], "object", "{}", spec.name);
            assert!(!spec.description.is_empty());
        }
    }

    #[test]
    fn ask_cannot_write_even_if_the_model_calls_a_writing_tool() {
        let tools = ToolBox::new(Mode::Ask, SCRIPT, clean);
        let outcome = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "len = 14", "new": "len = 20"}),
        ));
        assert!(outcome.is_error);
        assert!(outcome.content.contains("cannot change the script"));
        let outcome = tools.run_for_test(&call("write_script", json!({"text": "x"})));
        assert!(outcome.is_error);
        assert_eq!(tools.draft().text(), SCRIPT);
    }

    #[test]
    fn reading_numbers_the_lines_and_clamps_the_range() {
        let tools = edit_box();
        let all = tools.run_for_test(&call("read_script", json!({})));
        assert!(!all.is_error);
        assert!(
            all.content
                .starts_with("Lines 1\u{2013}5 of 5 of the draft:\n1: //@version=6\n")
        );
        assert!(all.content.contains("3: len = 14\n"));
        assert_eq!(all.summary, "Read the script \u{b7} 5 lines");
        let part = tools.run_for_test(&call("read_script", json!({"start": 3, "end": 99})));
        assert!(
            part.content
                .contains("3: len = 14\n4: plot(close)\n5: plot(open)")
        );
        assert!(!part.content.contains("2: indicator"));
        assert_eq!(part.summary, "Read lines 3\u{2013}5");
        // Numbers sent as text are read as numbers.
        let text = tools.run_for_test(&call("read_script", json!({"start": "4", "end": "4"})));
        assert!(text.content.contains("4: plot(close)"));
    }

    #[test]
    fn reading_outside_the_script_is_an_error_the_model_can_read() {
        let tools = edit_box();
        let past = tools.run_for_test(&call("read_script", json!({"start": 40})));
        assert!(past.is_error);
        assert!(past.content.contains("5 lines"));
        let backwards = tools.run_for_test(&call("read_script", json!({"start": 4, "end": 2})));
        assert!(backwards.is_error);
        let zero = tools.run_for_test(&call("read_script", json!({"start": 0})));
        assert!(zero.is_error);
    }

    #[test]
    fn a_long_script_is_read_in_pages() {
        let long: String = (1..=1000).map(|n| format!("x{n} = {n}\n")).collect();
        let tools = ToolBox::new(Mode::Edit, &long, clean);
        let page = tools.run_for_test(&call("read_script", json!({})));
        assert!(page.content.contains("400: x400 = 400"));
        assert!(!page.content.contains("401: "));
        assert!(page.content.contains("`start` 401"));
    }

    #[test]
    fn an_edit_replaces_one_exact_match_and_reports_where() {
        let tools = edit_box();
        let outcome = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "len = 14", "new": "len = 20"}),
        ));
        assert!(!outcome.is_error, "{}", outcome.content);
        assert_eq!(outcome.summary, "Edited line 3 \u{b7} No problems");
        assert!(outcome.content.contains("Now: No problems."));
        assert_eq!(
            tools.draft().text(),
            "//@version=6\nindicator(\"x\")\nlen = 20\nplot(close)\nplot(open)\n"
        );
    }

    #[test]
    fn an_edit_spanning_lines_reports_the_new_range() {
        let tools = edit_box();
        let outcome = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "plot(close)\n", "new": "a = 1\nb = 2\nplot(close)\n"}),
        ));
        assert_eq!(
            outcome.summary,
            "Edited lines 4\u{2013}7 \u{b7} No problems"
        );
    }

    #[test]
    fn an_edit_that_matches_nothing_says_why_and_changes_nothing() {
        let tools = edit_box();
        let missing = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "len = 99", "new": "len = 1"}),
        ));
        assert!(missing.is_error);
        assert!(missing.content.contains("not found"));
        assert!(missing.content.contains("line numbers"));
        // Present, but not as written.
        let spaced = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "  len = 14  ", "new": "len = 1"}),
        ));
        assert!(spaced.is_error);
        assert!(spaced.content.contains("copy it from `read_script`"));
        assert_eq!(tools.draft().text(), SCRIPT);
    }

    #[test]
    fn an_edit_that_matches_twice_lists_the_places() {
        let tools = edit_box();
        let outcome = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "plot(", "new": "plot( "}),
        ));
        assert!(outcome.is_error);
        assert!(outcome.content.contains("matches 2 places (lines 4, 5)"));
        assert_eq!(tools.draft().text(), SCRIPT);
    }

    #[test]
    fn nonsense_edits_are_refused() {
        let tools = edit_box();
        assert!(
            tools
                .run_for_test(&call("edit_script", json!({"old": "", "new": "x"})))
                .is_error
        );
        assert!(
            tools
                .run_for_test(&call("edit_script", json!({"old": "len", "new": "len"})))
                .is_error
        );
        assert!(
            tools
                .run_for_test(&call("edit_script", json!({"old": "len"})))
                .is_error
        );
        assert!(
            tools
                .run_for_test(&call("edit_script", json!({"old": 3, "new": "x"})))
                .is_error
        );
        assert_eq!(tools.draft().text(), SCRIPT);
    }

    #[test]
    fn a_crlf_script_is_edited_with_unix_line_endings() {
        let crlf = SCRIPT.replace('\n', "\r\n");
        let tools = ToolBox::new(Mode::Edit, &crlf, clean);
        assert_eq!(tools.draft().text(), SCRIPT);
        let outcome = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "len = 14\nplot(close)", "new": "len = 9\nplot(close)"}),
        ));
        assert!(!outcome.is_error, "{}", outcome.content);
        // What the model writes with CRLF in it is normalised too.
        let again = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "len = 9\r\nplot(close)", "new": "len = 5\r\nplot(close)"}),
        ));
        assert!(!again.is_error);
        assert!(tools.draft().text().contains("len = 5\nplot(close)"));
        assert!(!tools.draft().text().contains('\r'));
    }

    #[test]
    fn a_change_reports_whether_the_draft_still_compiles() {
        let tools = ToolBox::new(Mode::Edit, SCRIPT, failing);
        let outcome = tools.run_for_test(&call(
            "edit_script",
            json!({"old": "len = 14", "new": "len = oops"}),
        ));
        assert!(!outcome.is_error);
        assert!(outcome.content.contains("Now: 1 error."));
        assert!(outcome.content.contains("PINE3001"));
        assert_eq!(outcome.summary, "Edited line 3 \u{b7} 1 error");
    }

    #[test]
    fn writing_replaces_the_whole_draft() {
        let tools = edit_box();
        let outcome = tools.run_for_test(&call(
            "write_script",
            json!({"text": "//@version=6\r\nindicator(\"y\")\r\n"}),
        ));
        assert!(!outcome.is_error);
        assert_eq!(
            outcome.summary,
            "Rewrote the whole script (2 lines) \u{b7} No problems"
        );
        assert_eq!(tools.draft().text(), "//@version=6\nindicator(\"y\")\n");
        assert!(
            tools
                .run_for_test(&call("write_script", json!({"text": "  \n"})))
                .is_error
        );
        assert!(
            tools
                .run_for_test(&call("write_script", json!({})))
                .is_error
        );
    }

    #[test]
    fn compile_lists_the_problems_and_says_so_when_there_are_none() {
        let tools = ToolBox::new(Mode::Ask, "plot(oops)", failing);
        let outcome = tools.run_for_test(&call("compile", json!({})));
        assert!(!outcome.is_error);
        assert!(outcome.content.starts_with("1 error"));
        assert!(outcome.content.contains("Undeclared identifier"));
        let ok = ToolBox::new(Mode::Ask, SCRIPT, clean).run_for_test(&call("compile", json!({})));
        assert!(ok.content.starts_with("No problems"));
        assert_eq!(ok.summary, "Compiled \u{b7} No problems");
    }

    #[test]
    fn a_call_the_model_got_wrong_is_an_error_result_not_a_failure() {
        let tools = edit_box();
        let mut bad = call("read_script", json!({}));
        bad.malformed = Some("The arguments were not valid JSON: eof.".to_owned());
        let outcome = tools.run_for_test(&bad);
        assert!(outcome.is_error);
        assert!(outcome.content.contains("not valid JSON"));
        let unknown = tools.run_for_test(&call("run_the_markets", json!({})));
        assert!(unknown.is_error);
        assert!(
            unknown
                .content
                .contains("read_script, compile, profile, edit_script, write_script")
        );
    }

    #[test]
    fn a_long_answer_is_cut_with_a_note() {
        let long = "x".repeat(MAX_RESULT_CHARS + 50);
        let cut = cap(long);
        assert!(cut.ends_with("(cut: 50 more characters)"));
        assert_eq!(cap("short".to_owned()), "short");
    }

    #[test]
    fn labels_say_what_a_call_is_doing() {
        let label = |name, input| ToolBox::label(&call(name, input));
        assert_eq!(label("read_script", json!({})), "Reading the script");
        assert_eq!(
            label("read_script", json!({"start": 3, "end": 9})),
            "Reading lines 3\u{2013}9"
        );
        assert_eq!(label("compile", json!({})), "Compiling the draft");
        assert_eq!(label("edit_script", json!({})), "Editing the script");
    }
}
