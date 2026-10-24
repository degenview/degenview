//! Breakpoints: what the person sets, and what the running script checks.
//!
//! A [`BreakpointSpec`] is the person's side: a line, an on/off switch, an optional condition, a
//! hit-count rule and an optional logpoint message. It is plain data that is saved and edited.
//! [`compile`] resolves each to a statement and parses its expressions once, reporting what is
//! wrong with the ones that cannot work instead of leaving them silently inert.

use serde::{Deserialize, Serialize};

use super::expression::{self, DebugExpression};
use super::source_map::{LineResolution, SourceMap};

/// When a breakpoint whose condition holds actually acts. Hits are counted every time the
/// statement is reached with the condition true, across bars and calls, from the start of the
/// session; a restart counts from zero again.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "rule", content = "n")]
pub enum HitRule {
    #[default]
    Always,
    /// Only the nth hit.
    Equals(u64),
    /// Every nth hit: the nth, the 2nth, …
    EveryNth(u64),
    /// Every hit after the nth.
    GreaterThan(u64),
}

impl HitRule {
    pub fn accepts(self, hits: u64) -> bool {
        match self {
            Self::Always => true,
            Self::Equals(n) => hits == n,
            Self::EveryNth(n) => n > 0 && hits.is_multiple_of(n),
            Self::GreaterThan(n) => hits > n,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BreakpointSpec {
    pub id: u64,
    /// One-based.
    pub line: usize,
    pub enabled: bool,
    /// A Pine expression; the breakpoint acts only when it is true.
    pub condition: Option<String>,
    pub hit: HitRule,
    /// Makes this a logpoint: the message is written to the Pine Logs and the script does not stop.
    /// `{expression}` is replaced by the value of a Pine expression; `{{` and `}}` are braces.
    pub log_message: Option<String>,
}

impl Default for BreakpointSpec {
    fn default() -> Self {
        Self {
            id: 0,
            line: 1,
            enabled: true,
            condition: None,
            hit: HitRule::Always,
            log_message: None,
        }
    }
}

impl BreakpointSpec {
    pub fn line(id: u64, line: usize) -> Self {
        Self {
            id,
            line,
            ..Self::default()
        }
    }

    pub fn is_logpoint(&self) -> bool {
        self.log_message.is_some()
    }
}

/// What became of a breakpoint when it met the program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BreakpointState {
    /// Installed at a statement.
    Verified { offset: usize },
    /// The line holds no code; the reason says why.
    Unresolved(String),
    /// A condition or message that does not parse or is not allowed while debugging.
    Invalid(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BreakpointStatus {
    pub id: u64,
    pub state: BreakpointState,
}

/// A logpoint message with its expressions parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Text(String),
    Expression(DebugExpression),
}

impl Template {
    pub fn parse(message: &str) -> Result<Self, String> {
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut chars = message.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    text.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    text.push('}');
                }
                '{' => {
                    let mut inner = String::new();
                    let mut closed = false;
                    for c in chars.by_ref() {
                        if c == '}' {
                            closed = true;
                            break;
                        }
                        inner.push(c);
                    }
                    if !closed {
                        return Err("A { has no closing }.".to_owned());
                    }
                    let expression = expression::parse(&inner)
                        .map_err(|error| format!("{{{}}}: {}", inner.trim(), error.message))?;
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    parts.push(Part::Expression(expression));
                }
                '}' => return Err("A } has no opening {.".to_owned()),
                other => text.push(other),
            }
        }
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        Ok(Self { parts })
    }

    /// The message, with each expression replaced by what `evaluate` makes of it. An expression
    /// that fails reads `‹error›` in place, so a logpoint never stops the script.
    pub fn render(
        &self,
        mut evaluate: impl FnMut(&DebugExpression) -> Result<String, String>,
    ) -> String {
        let mut message = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => message.push_str(text),
                Part::Expression(expression) => match evaluate(expression) {
                    Ok(text) => message.push_str(&text),
                    Err(error) => {
                        message.push('‹');
                        message.push_str(&error);
                        message.push('›');
                    }
                },
            }
        }
        message
    }
}

/// A breakpoint ready to be checked at every visit to its statement.
#[derive(Clone, Debug)]
pub struct CompiledBreakpoint {
    pub id: u64,
    pub offset: usize,
    pub enabled: bool,
    pub condition: Option<DebugExpression>,
    pub hit: HitRule,
    pub template: Option<Template>,
    pub hits: u64,
}

/// Resolves and parses `specs` against the program. The compiled list is sorted by statement.
pub fn compile(
    specs: &[BreakpointSpec],
    map: &SourceMap,
) -> (Vec<CompiledBreakpoint>, Vec<BreakpointStatus>) {
    let mut compiled = Vec::new();
    let mut statuses = Vec::new();
    for spec in specs {
        let state = match build(spec, map) {
            Ok(breakpoint) => {
                let offset = breakpoint.offset;
                compiled.push(breakpoint);
                BreakpointState::Verified { offset }
            }
            Err(state) => state,
        };
        statuses.push(BreakpointStatus { id: spec.id, state });
    }
    compiled.sort_by_key(|breakpoint| breakpoint.offset);
    (compiled, statuses)
}

fn build(spec: &BreakpointSpec, map: &SourceMap) -> Result<CompiledBreakpoint, BreakpointState> {
    let offset = match map.resolve_line(spec.line) {
        LineResolution::Executable { offset } => offset,
        LineResolution::NotExecutable(reason) => {
            return Err(BreakpointState::Unresolved(reason.describe().to_owned()));
        }
    };
    let condition = match spec.condition.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(text) => Some(
            expression::parse(text)
                .map_err(|error| BreakpointState::Invalid(format!("Condition: {error}")))?,
        ),
    };
    let template = spec
        .log_message
        .as_deref()
        .map(Template::parse)
        .transpose()
        .map_err(|error| BreakpointState::Invalid(format!("Message: {error}")))?;
    if let HitRule::EveryNth(0) | HitRule::Equals(0) = spec.hit {
        return Err(BreakpointState::Invalid(
            "The hit count must be at least 1.".to_owned(),
        ));
    }
    Ok(CompiledBreakpoint {
        id: spec.id,
        offset,
        enabled: spec.enabled,
        condition,
        hit: spec.hit,
        template,
        hits: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::token::Limits;

    fn map(body: &str) -> SourceMap {
        let source = format!("//@version=6\nindicator(\"T\")\n{body}");
        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
        assert!(program.is_valid(), "{:?}", program.diagnostics);
        SourceMap::build(&program.statements)
    }

    #[test]
    fn hit_rules_count_the_way_they_are_documented() {
        let hits = |rule: HitRule| (1..=9).filter(|n| rule.accepts(*n)).collect::<Vec<_>>();
        assert_eq!(hits(HitRule::Always), (1..=9).collect::<Vec<_>>());
        assert_eq!(hits(HitRule::Equals(3)), vec![3]);
        assert_eq!(hits(HitRule::EveryNth(3)), vec![3, 6, 9]);
        assert_eq!(hits(HitRule::GreaterThan(6)), vec![7, 8, 9]);
        assert!(!HitRule::EveryNth(0).accepts(0));
    }

    #[test]
    fn templates_split_text_and_expressions_and_unescape_braces() {
        let template = Template::parse("Bar {bar_index}: close={close}, {{literal}}").unwrap();
        let text = template.render(|expression| Ok(format!("<{}>", expression.source())));
        assert_eq!(text, "Bar <bar_index>: close=<close>, {literal}");
    }

    #[test]
    fn a_failing_expression_reads_as_its_error_and_does_not_end_the_message() {
        let template = Template::parse("a={x} b={y}").unwrap();
        let text = template.render(|expression| match expression.source() {
            "x" => Err("undefined".to_owned()),
            other => Ok(other.to_owned()),
        });
        assert_eq!(text, "a=‹undefined› b=y");
    }

    #[test]
    fn malformed_templates_are_reported() {
        assert!(Template::parse("{close").unwrap_err().contains("closing"));
        assert!(Template::parse("close}").unwrap_err().contains("opening"));
        assert!(
            Template::parse("{ta.sma(close, 3)}")
                .unwrap_err()
                .contains("keeps state")
        );
        assert!(Template::parse("{}").unwrap_err().contains("Enter"));
    }

    #[test]
    fn compile_resolves_lines_and_reports_what_cannot_work() {
        let map = map("a = close\n// comment\nplot(a)\n");
        let specs = [
            BreakpointSpec::line(1, 3),
            BreakpointSpec::line(2, 4),
            BreakpointSpec {
                condition: Some("close >".to_owned()),
                ..BreakpointSpec::line(3, 5)
            },
            BreakpointSpec {
                condition: Some("close > open".to_owned()),
                log_message: Some("a={a}".to_owned()),
                hit: HitRule::EveryNth(2),
                ..BreakpointSpec::line(4, 5)
            },
            BreakpointSpec {
                hit: HitRule::Equals(0),
                ..BreakpointSpec::line(5, 3)
            },
            BreakpointSpec::line(6, 2),
        ];
        let (compiled, statuses) = compile(&specs, &map);
        assert_eq!(
            compiled.iter().map(|b| b.id).collect::<Vec<_>>(),
            vec![1, 4]
        );
        let state = |id: u64| statuses.iter().find(|s| s.id == id).unwrap().state.clone();
        assert!(matches!(state(1), BreakpointState::Verified { .. }));
        assert!(matches!(state(2), BreakpointState::Unresolved(_)));
        assert!(matches!(state(3), BreakpointState::Invalid(ref m) if m.starts_with("Condition")));
        assert!(matches!(state(5), BreakpointState::Invalid(ref m) if m.contains("at least 1")));
        assert!(
            matches!(state(6), BreakpointState::Unresolved(_)),
            "the indicator() line"
        );
        let logpoint = compiled.iter().find(|b| b.id == 4).unwrap();
        assert!(logpoint.template.is_some() && logpoint.condition.is_some());
    }

    #[test]
    fn specs_round_trip_through_json_and_old_rows_load() {
        let spec = BreakpointSpec {
            condition: Some("bar_index > 3".to_owned()),
            hit: HitRule::GreaterThan(2),
            ..BreakpointSpec::line(7, 12)
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(serde_json::from_str::<BreakpointSpec>(&json).unwrap(), spec);
        let bare: BreakpointSpec = serde_json::from_str(r#"{"id":1,"line":4}"#).unwrap();
        assert!(bare.enabled && bare.hit == HitRule::Always && bare.condition.is_none());
    }
}
