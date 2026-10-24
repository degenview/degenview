//! What debugging a script leaves behind between sessions: its breakpoints and watch expressions.
//!
//! Saved per script, on this Mac only (`scriptDebug.state` is not an iCloud setting): a breakpoint
//! belongs to a person's session at a desk, not to the script they share. Lines follow edits through
//! the same line diff the editor's change gutter uses, so a breakpoint stays on its statement when
//! lines are added above it; one whose line was deleted is kept, marked unresolved, and never
//! rebinds itself to whatever code ended up there.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::diff::{DiffHunk, DiffSnapshot};
use crate::pine::debug::BreakpointSpec;

pub const KEY: &str = "scriptDebug.state";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoredBreakpoint {
    #[serde(flatten)]
    pub spec: BreakpointSpec,
    /// Its line was deleted by an edit; it waits to be removed or set again.
    pub unresolved: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScriptDebugState {
    pub breakpoints: Vec<StoredBreakpoint>,
    pub watches: Vec<String>,
    next_id: u64,
}

/// What a click on a line did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Toggled {
    Added(u64),
    Removed(u64),
}

impl ScriptDebugState {
    pub fn is_empty(&self) -> bool {
        self.breakpoints.is_empty() && self.watches.is_empty()
    }

    pub fn breakpoint_at(&self, line: usize) -> Option<&StoredBreakpoint> {
        self.breakpoints
            .iter()
            .find(|stored| !stored.unresolved && stored.spec.line == line)
    }

    pub fn breakpoint(&self, id: u64) -> Option<&StoredBreakpoint> {
        self.breakpoints.iter().find(|stored| stored.spec.id == id)
    }

    pub fn breakpoint_mut(&mut self, id: u64) -> Option<&mut StoredBreakpoint> {
        self.breakpoints
            .iter_mut()
            .find(|stored| stored.spec.id == id)
    }

    fn mint(&mut self) -> u64 {
        self.next_id = self.next_id.max(
            self.breakpoints
                .iter()
                .map(|b| b.spec.id)
                .max()
                .unwrap_or(0),
        ) + 1;
        self.next_id
    }

    /// Adds a breakpoint on `line`, or removes the one that is there.
    pub fn toggle(&mut self, line: usize) -> Toggled {
        if let Some(existing) = self.breakpoint_at(line).map(|stored| stored.spec.id) {
            self.remove(existing);
            return Toggled::Removed(existing);
        }
        let id = self.mint();
        self.breakpoints.push(StoredBreakpoint {
            spec: BreakpointSpec::line(id, line),
            unresolved: false,
        });
        self.breakpoints.sort_by_key(|stored| stored.spec.line);
        Toggled::Added(id)
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.breakpoints.len();
        self.breakpoints.retain(|stored| stored.spec.id != id);
        self.breakpoints.len() != before
    }

    pub fn remove_all(&mut self) {
        self.breakpoints.clear();
    }

    /// Turns every breakpoint on or off together.
    pub fn enable_all(&mut self, enabled: bool) {
        for stored in &mut self.breakpoints {
            stored.spec.enabled = enabled;
        }
    }

    /// The breakpoints a session should install: the ones whose line still exists.
    pub fn specs(&self) -> Vec<BreakpointSpec> {
        self.breakpoints
            .iter()
            .filter(|stored| !stored.unresolved)
            .map(|stored| stored.spec.clone())
            .collect()
    }

    pub fn add_watch(&mut self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.watches.iter().any(|existing| existing == text) {
            return false;
        }
        self.watches.push(text.to_owned());
        true
    }

    pub fn edit_watch(&mut self, index: usize, text: &str) -> bool {
        let text = text.trim();
        match self.watches.get_mut(index) {
            Some(slot) if !text.is_empty() => {
                *slot = text.to_owned();
                true
            }
            _ => false,
        }
    }

    pub fn remove_watch(&mut self, index: usize) -> bool {
        if index < self.watches.len() {
            self.watches.remove(index);
            true
        } else {
            false
        }
    }

    /// Follows an edit from `old` to `new` text: each breakpoint moves with its line.
    pub fn follow_edit(&mut self, old: &str, new: &str) {
        if self.breakpoints.is_empty() || old == new {
            return;
        }
        let diff = DiffSnapshot::compute(old, new);
        for stored in &mut self.breakpoints {
            if stored.unresolved {
                continue;
            }
            match moved_line(diff.hunks(), stored.spec.line.saturating_sub(1)) {
                Some(line) => stored.spec.line = line + 1,
                None => stored.unresolved = true,
            }
        }
        self.breakpoints.sort_by_key(|stored| stored.spec.line);
    }
}

/// Where the zero-based `line` of the old text is in the new one; `None` if the edit removed it.
fn moved_line(hunks: &[DiffHunk], line: usize) -> Option<usize> {
    let mut delta: isize = 0;
    for hunk in hunks {
        if hunk.old_lines.end <= line {
            // The change is wholly above this line (an insertion right before it counts).
            delta += hunk.new_lines.len() as isize - hunk.old_lines.len() as isize;
        } else if hunk.old_lines.contains(&line) {
            // Inside a rewritten stretch the lines keep their order: the nth old line is the nth new
            // one, until the new stretch is shorter.
            let offset = line - hunk.old_lines.start;
            return (offset < hunk.new_lines.len()).then(|| hunk.new_lines.start + offset);
        } else {
            break;
        }
    }
    Some((line as isize + delta).max(0) as usize)
}

/// All scripts' state, as the one local setting holds it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DebugStates(HashMap<String, ScriptDebugState>);

impl DebugStates {
    pub fn get(&self, script_id: &str) -> ScriptDebugState {
        self.0.get(script_id).cloned().unwrap_or_default()
    }

    /// Records `state` for the script; an empty state is dropped rather than kept.
    pub fn set(&mut self, script_id: &str, state: ScriptDebugState) {
        if state.is_empty() {
            self.0.remove(script_id);
        } else {
            self.0.insert(script_id.to_owned(), state);
        }
    }

    pub fn forget(&mut self, script_id: &str) {
        self.0.remove(script_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::debug::HitRule;

    fn with_breakpoints(lines: &[usize]) -> ScriptDebugState {
        let mut state = ScriptDebugState::default();
        for line in lines {
            state.toggle(*line);
        }
        state
    }

    fn lines(state: &ScriptDebugState) -> Vec<(usize, bool)> {
        state
            .breakpoints
            .iter()
            .map(|stored| (stored.spec.line, stored.unresolved))
            .collect()
    }

    #[test]
    fn toggling_a_line_adds_and_removes_its_breakpoint() {
        let mut state = ScriptDebugState::default();
        let Toggled::Added(id) = state.toggle(7) else {
            panic!("added");
        };
        assert_eq!(state.breakpoint_at(7).map(|b| b.spec.id), Some(id));
        assert_eq!(state.toggle(7), Toggled::Removed(id));
        assert!(state.breakpoints.is_empty());
        // Ids are not reused, so a panel row keeps meaning one breakpoint.
        let Toggled::Added(next) = state.toggle(7) else {
            panic!("added");
        };
        assert!(next > id);
    }

    #[test]
    fn breakpoints_are_kept_in_line_order_and_enabled_in_bulk() {
        let mut state = with_breakpoints(&[9, 3, 6]);
        assert_eq!(lines(&state), [(3, false), (6, false), (9, false)]);
        state.enable_all(false);
        assert!(state.breakpoints.iter().all(|b| !b.spec.enabled));
        state.enable_all(true);
        assert!(state.breakpoints.iter().all(|b| b.spec.enabled));
        state.remove_all();
        assert!(state.is_empty());
    }

    #[test]
    fn watches_are_trimmed_unique_and_editable() {
        let mut state = ScriptDebugState::default();
        assert!(state.add_watch("  close - open "));
        assert!(!state.add_watch("close - open"), "no duplicates");
        assert!(!state.add_watch("   "));
        assert!(state.edit_watch(0, "high - low"));
        assert!(!state.edit_watch(0, ""));
        assert!(!state.edit_watch(5, "x"));
        assert_eq!(state.watches, ["high - low"]);
        assert!(state.remove_watch(0));
        assert!(!state.remove_watch(0));
    }

    #[test]
    fn lines_inserted_above_push_a_breakpoint_down_and_lines_removed_pull_it_up() {
        let old = "a\nb\nc\nd\ne\n";
        let mut state = with_breakpoints(&[4]);
        state.follow_edit(old, "x\ny\na\nb\nc\nd\ne\n");
        assert_eq!(lines(&state), [(6, false)]);
        state.follow_edit("x\ny\na\nb\nc\nd\ne\n", "a\nb\nc\nd\ne\n");
        assert_eq!(lines(&state), [(4, false)]);
    }

    #[test]
    fn editing_the_line_itself_keeps_the_breakpoint_on_it() {
        let mut state = with_breakpoints(&[2]);
        state.follow_edit("a\nb\nc\n", "a\nbb\nc\n");
        assert_eq!(lines(&state), [(2, false)]);
    }

    #[test]
    fn a_deleted_line_leaves_its_breakpoint_unresolved_not_on_the_code_that_took_its_place() {
        let mut state = with_breakpoints(&[2, 4]);
        state.follow_edit("a\nb\nc\nd\ne\n", "a\nc\nd\ne\n");
        // Line 2 was deleted; line 4 moved up to 3.
        assert_eq!(lines(&state), [(2, true), (3, false)]);
        assert_eq!(
            state.specs().len(),
            1,
            "an unresolved breakpoint is not installed"
        );
        assert!(state.breakpoint_at(2).is_none());
        assert_eq!(state.breakpoint_at(3).map(|b| b.spec.line), Some(3));
    }

    #[test]
    fn a_rewritten_stretch_keeps_the_order_of_its_lines() {
        let mut state = with_breakpoints(&[2, 3]);
        // Lines 2 and 3 are replaced by three new lines.
        state.follow_edit("a\nb\nc\nd\n", "a\nX\nY\nZ\nd\n");
        assert_eq!(lines(&state), [(2, false), (3, false)]);
        // Replaced by one line: the second is gone.
        let mut state = with_breakpoints(&[2, 3]);
        state.follow_edit("a\nb\nc\nd\n", "a\nX\nd\n");
        assert_eq!(lines(&state), [(2, false), (3, true)]);
    }

    #[test]
    fn an_unrelated_edit_leaves_everything_alone_and_an_identical_text_is_free() {
        let mut state = with_breakpoints(&[1, 3]);
        let before = state.clone();
        state.follow_edit("same", "same");
        assert_eq!(state, before);
        state.follow_edit("a\nb\nc\nd\n", "a\nb\nc\nd\ne\n");
        assert_eq!(lines(&state), [(1, false), (3, false)]);
    }

    #[test]
    fn state_round_trips_through_json_and_old_or_partial_rows_load() {
        let mut state = with_breakpoints(&[5]);
        state.breakpoints[0].spec.condition = Some("bar_index > 3".to_owned());
        state.breakpoints[0].spec.hit = HitRule::EveryNth(2);
        state.add_watch("close");
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(
            serde_json::from_str::<ScriptDebugState>(&json).unwrap(),
            state
        );
        let bare: ScriptDebugState = serde_json::from_str("{}").unwrap();
        assert!(bare.is_empty());
        let partial: ScriptDebugState =
            serde_json::from_str(r#"{"breakpoints":[{"id":3,"line":9}]}"#).unwrap();
        assert!(partial.breakpoints[0].spec.enabled && !partial.breakpoints[0].unresolved);
    }

    #[test]
    fn the_per_script_map_drops_empty_states_and_forgets_deleted_scripts() {
        let mut states = DebugStates::default();
        states.set("a", with_breakpoints(&[1]));
        states.set("b", with_breakpoints(&[2]));
        assert_eq!(states.get("a").breakpoints.len(), 1);
        states.set("a", ScriptDebugState::default());
        assert!(states.get("a").is_empty());
        states.forget("b");
        assert!(states.get("b").is_empty());
        assert_eq!(serde_json::to_string(&states).unwrap(), "{}");
    }
}
