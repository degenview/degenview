//! A plan the assistant hands the user in Plan mode: what it found and the steps it would take.
//!
//! The model gives it through the `submit_plan` tool as structured data, so the panel can show it as
//! a card with Approve, Revise and Discard, instead of guessing from a numbered list in prose whether
//! a reply is a plan. Pure: JSON in, text out.

use serde_json::Value;

/// Steps a plan may have. A longer one is a project, not a plan for one script.
pub const MAX_STEPS: usize = 12;
const MAX_SUMMARY_CHARS: usize = 600;
const MAX_TITLE_CHARS: usize = 120;
const MAX_DETAIL_CHARS: usize = 600;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanStep {
    pub title: String,
    pub detail: String,
    /// Where in the script it applies, as the model wrote it: "12-20", "line 7".
    pub lines: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    /// What it found and what the plan comes to.
    pub summary: String,
    pub steps: Vec<PlanStep>,
    /// What the user should expect once it is done, and how sure that is (measured, or a guess).
    pub expected: Option<String>,
}

fn text(input: &Value, name: &str) -> Option<String> {
    let value = input.get(name)?;
    let text = match value {
        Value::String(text) => text.trim().to_owned(),
        // `lines` is easily sent as a number.
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn cut(text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }
    let mut kept: String = text.chars().take(limit - 1).collect();
    kept.push('\u{2026}');
    kept
}

impl Plan {
    /// A plan from the arguments of `submit_plan`. What is wrong is said in words the model can act
    /// on; text that is too long is shortened rather than refused.
    pub fn parse(input: &Value) -> Result<Self, String> {
        let summary = text(input, "summary")
            .ok_or("`summary` is missing. Say what you found and what the plan comes to.")?;
        let steps = input.get("steps").and_then(Value::as_array).ok_or(
            "`steps` is missing. Give a list of steps, each with a `title` and a `detail`.",
        )?;
        if steps.is_empty() || steps.len() > MAX_STEPS {
            return Err(format!(
                "Give between 1 and {MAX_STEPS} steps; there were {}.",
                steps.len()
            ));
        }
        let mut parsed = Vec::with_capacity(steps.len());
        for (index, step) in steps.iter().enumerate() {
            let title =
                text(step, "title").ok_or_else(|| format!("Step {} has no `title`.", index + 1))?;
            parsed.push(PlanStep {
                title: cut(title, MAX_TITLE_CHARS),
                detail: cut(text(step, "detail").unwrap_or_default(), MAX_DETAIL_CHARS),
                lines: text(step, "lines").map(|lines| cut(lines, 40)),
            });
        }
        Ok(Self {
            summary: cut(summary, MAX_SUMMARY_CHARS),
            steps: parsed,
            expected: text(input, "expected_effect").map(|e| cut(e, MAX_SUMMARY_CHARS)),
        })
    }

    /// The plan as Markdown: what the transcript shows, what history keeps, and what the model reads
    /// back when the user approves it.
    pub fn markdown(&self) -> String {
        let mut out = format!("{}\n\n", self.summary);
        for (index, step) in self.steps.iter().enumerate() {
            out.push_str(&format!("{}. **{}**", index + 1, step.title));
            if let Some(lines) = &step.lines {
                out.push_str(&format!(" (lines {lines})"));
            }
            if !step.detail.is_empty() {
                out.push_str(&format!(": {}", step.detail));
            }
            out.push('\n');
        }
        if let Some(expected) = &self.expected {
            out.push_str(&format!("\n**Expected effect:** {expected}\n"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid() -> Value {
        json!({
            "summary": "The loop on line 5 does 90% of the work.",
            "steps": [
                {"title": "Hoist the sine", "detail": "Compute it once per bar.", "lines": "5-7"},
                {"title": "Use a for loop over fewer values", "detail": "Stop at 25."},
            ],
            "expected_effect": "About half the executions (measured: 1,500 per 30 bars)."
        })
    }

    #[test]
    fn a_plan_is_read_from_the_arguments() {
        let plan = Plan::parse(&valid()).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].lines.as_deref(), Some("5-7"));
        assert_eq!(plan.steps[1].lines, None);
        assert!(plan.expected.as_deref().unwrap().starts_with("About half"));
    }

    #[test]
    fn it_reads_as_markdown_with_numbered_steps() {
        let text = Plan::parse(&valid()).unwrap().markdown();
        assert!(text.starts_with("The loop on line 5 does 90% of the work.\n\n1. **Hoist the sine** (lines 5-7): Compute it once per bar.\n2. **Use a for loop over fewer values**: Stop at 25.\n"));
        assert!(text.ends_with(
            "**Expected effect:** About half the executions (measured: 1,500 per 30 bars).\n"
        ));
        // So a saved or fallback reply is recognised as a plan too.
        assert!(crate::script_manager::ai_prompt::looks_like_plan(&text));
    }

    #[test]
    fn what_is_missing_is_said_so_the_model_can_fix_it() {
        let no_summary = Plan::parse(&json!({"steps": [{"title": "a"}]}));
        assert!(no_summary.unwrap_err().contains("`summary`"));
        let no_steps = Plan::parse(&json!({"summary": "s"}));
        assert!(no_steps.unwrap_err().contains("`steps`"));
        let empty = Plan::parse(&json!({"summary": "s", "steps": []}));
        assert!(empty.unwrap_err().contains("between 1 and 12"));
        let many: Vec<Value> = (0..13).map(|n| json!({"title": format!("s{n}")})).collect();
        assert!(Plan::parse(&json!({"summary": "s", "steps": many})).is_err());
        let untitled =
            Plan::parse(&json!({"summary": "s", "steps": [{"title": "a"}, {"detail": "b"}]}));
        assert!(untitled.unwrap_err().contains("Step 2"));
    }

    #[test]
    fn long_text_is_shortened_not_refused_and_numbers_are_lines() {
        let long = "x".repeat(2000);
        let plan = Plan::parse(&json!({
            "summary": long,
            "steps": [{"title": "t", "detail": long, "lines": 12}],
        }))
        .unwrap();
        assert_eq!(plan.summary.chars().count(), MAX_SUMMARY_CHARS);
        assert!(plan.summary.ends_with('\u{2026}'));
        assert_eq!(plan.steps[0].detail.chars().count(), MAX_DETAIL_CHARS);
        assert_eq!(plan.steps[0].lines.as_deref(), Some("12"));
    }

    #[test]
    fn a_step_without_detail_or_an_expected_effect_is_fine() {
        let plan = Plan::parse(&json!({"summary": "s", "steps": [{"title": "Do it"}]})).unwrap();
        assert_eq!(plan.markdown(), "s\n\n1. **Do it**\n");
    }
}
