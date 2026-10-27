//! The state and the rows of the review sheet that opens over an assistant proposal. Pure: no
//! egui and no store. `ai_review_view` draws these rows; `ai_host` applies what was chosen.
//!
//! A proposal is a whole new script, but the user reviews it as separate changes: each can be
//! accepted or rejected, and only the accepted ones are applied (`diff::merge_selected`). The diff
//! is shown as one list of rows over the whole file, with long runs of unchanged lines folded away.

use super::diff::{self, ChangeKind, DiffHunk};
use std::collections::HashSet;
use std::ops::Range;

/// Unchanged lines kept on each side of a change; longer runs fold into one row.
pub const CONTEXT: usize = 3;
/// A fold must hide at least this many lines. Hiding two behind a row that costs one is no help.
pub const MIN_FOLD: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Row {
    /// The heading above one change: its number, its kind and its accept toggle.
    Header { hunk: usize },
    /// An unchanged line. Both numbers are zero-based.
    Context {
        old: usize,
        new: usize,
        text: String,
    },
    Removed {
        old: usize,
        hunk: usize,
        text: String,
        /// The words that changed, as byte ranges into `text`.
        words: Vec<Range<usize>>,
    },
    Added {
        new: usize,
        hunk: usize,
        text: String,
        words: Vec<Range<usize>>,
    },
    /// `count` unchanged lines left out. `key` is the first of them in the old text; expanding it
    /// puts the key in `Review::expanded`.
    Collapsed { count: usize, key: usize },
}

impl Row {
    /// The change this row belongs to, if it belongs to one.
    pub fn hunk(&self) -> Option<usize> {
        match self {
            Self::Header { hunk } | Self::Removed { hunk, .. } | Self::Added { hunk, .. } => {
                Some(*hunk)
            }
            Self::Context { .. } | Self::Collapsed { .. } => None,
        }
    }
}

/// A line as shown: no terminator.
fn display_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n')
        .map(|line| line.trim_end_matches(['\n', '\r']).to_owned())
        .collect()
}

struct Walk<'a> {
    old: &'a [String],
    expanded: &'a HashSet<usize>,
    rows: Vec<Row>,
}

impl Walk<'_> {
    /// `run` unchanged lines starting at `old_at` / `new_at`. Next to a change on a side, up to
    /// `CONTEXT` lines are kept on that side; the rest fold, unless the user opened the fold or
    /// it would hide fewer than `MIN_FOLD` lines.
    fn equal_run(
        &mut self,
        old_at: usize,
        new_at: usize,
        run: usize,
        has_prev: bool,
        has_next: bool,
    ) {
        if run == 0 {
            return;
        }
        let lead = if has_prev { CONTEXT } else { 0 };
        let trail = if has_next { CONTEXT } else { 0 };
        let line = |old: &[String], k: usize| Row::Context {
            old: old_at + k,
            new: new_at + k,
            text: old[old_at + k].clone(),
        };
        if self.expanded.contains(&old_at) || run.saturating_sub(lead + trail) < MIN_FOLD {
            for k in 0..run {
                self.rows.push(line(self.old, k));
            }
            return;
        }
        for k in 0..lead {
            self.rows.push(line(self.old, k));
        }
        self.rows.push(Row::Collapsed {
            count: run - lead - trail,
            key: old_at,
        });
        for k in run - trail..run {
            self.rows.push(line(self.old, k));
        }
    }
}

/// The rows of the whole-file diff from `base` to `script`, given the hunks between them.
pub fn rows(base: &str, script: &str, hunks: &[DiffHunk], expanded: &HashSet<usize>) -> Vec<Row> {
    if hunks.is_empty() {
        return Vec::new();
    }
    let old = display_lines(base);
    let new = display_lines(script);
    let mut walk = Walk {
        old: &old,
        expanded,
        rows: Vec::new(),
    };
    let (mut old_at, mut new_at) = (0, 0);
    for (index, hunk) in hunks.iter().enumerate() {
        walk.equal_run(
            old_at,
            new_at,
            hunk.old_lines.start - old_at,
            index > 0,
            true,
        );
        walk.rows.push(Row::Header { hunk: index });
        // Lines of a modified change are paired in order to show which words differ.
        let removed = &old[hunk.old_lines.clone()];
        let added = &new[hunk.new_lines.clone()];
        let pairs: Vec<diff::WordRanges> = (0..removed.len().min(added.len()))
            .map(|k| {
                if hunk.kind == ChangeKind::Modified {
                    diff::word_ranges(&removed[k], &added[k])
                } else {
                    diff::WordRanges::default()
                }
            })
            .collect();
        for (k, text) in removed.iter().enumerate() {
            walk.rows.push(Row::Removed {
                old: hunk.old_lines.start + k,
                hunk: index,
                text: text.clone(),
                words: pairs.get(k).map(|w| w.old.clone()).unwrap_or_default(),
            });
        }
        for (k, text) in added.iter().enumerate() {
            walk.rows.push(Row::Added {
                new: hunk.new_lines.start + k,
                hunk: index,
                text: text.clone(),
                words: pairs.get(k).map(|w| w.new.clone()).unwrap_or_default(),
            });
        }
        old_at = hunk.old_lines.end;
        new_at = hunk.new_lines.end;
    }
    let tail = old.len().saturating_sub(old_at);
    walk.equal_run(old_at, new_at, tail, !hunks.is_empty(), false);
    walk.rows
}

/// The index of the header row of change `hunk`, to scroll to.
pub fn header_row(rows: &[Row], hunk: usize) -> Option<usize> {
    rows.iter()
        .position(|row| matches!(row, Row::Header { hunk: h } if *h == hunk))
}

/// What the sheet knows beyond the proposal: which changes are accepted, where the user is, and
/// whether the selection compiles.
#[derive(Clone, Debug)]
pub struct Review {
    pub accepted: Vec<bool>,
    pub current: usize,
    /// Folded runs the user opened, by `Row::Collapsed::key`.
    pub expanded: HashSet<usize>,
    /// Compiler errors and warnings of the script the current selection would make.
    pub errors: usize,
    pub warnings: usize,
    /// The diff scrolls to `current` on the next frame.
    pub scroll_to_current: bool,
    /// The rows of the diff, built once and again when a fold opens.
    pub rows: Vec<Row>,
    /// The longest line in `rows`, in characters, to size the horizontal scroll.
    pub max_chars: usize,
}

impl Review {
    pub fn new(changes: usize, errors: usize, warnings: usize) -> Self {
        Self {
            accepted: vec![true; changes],
            current: 0,
            expanded: HashSet::new(),
            errors,
            warnings,
            scroll_to_current: true,
            rows: Vec::new(),
            max_chars: 0,
        }
    }

    /// A review of `hunks` (from `base` to `script`) with every change accepted.
    pub fn open(
        base: &str,
        script: &str,
        hunks: &[DiffHunk],
        errors: usize,
        warnings: usize,
    ) -> Self {
        let mut review = Self::new(hunks.len(), errors, warnings);
        review.refresh_rows(base, script, hunks);
        review
    }

    pub fn refresh_rows(&mut self, base: &str, script: &str, hunks: &[DiffHunk]) {
        self.rows = rows(base, script, hunks, &self.expanded);
        self.max_chars = self
            .rows
            .iter()
            .map(|row| match row {
                Row::Context { text, .. } | Row::Removed { text, .. } | Row::Added { text, .. } => {
                    text.chars().count()
                }
                Row::Header { .. } | Row::Collapsed { .. } => 0,
            })
            .max()
            .unwrap_or(0);
    }

    pub fn selected(&self) -> usize {
        self.accepted.iter().filter(|accepted| **accepted).count()
    }

    pub fn toggle(&mut self, change: usize) {
        if let Some(accepted) = self.accepted.get_mut(change) {
            *accepted = !*accepted;
        }
    }

    pub fn set_all(&mut self, accepted: bool) {
        self.accepted.iter_mut().for_each(|a| *a = accepted);
    }

    pub fn select(&mut self, change: usize) {
        if change < self.accepted.len() {
            self.current = change;
            self.scroll_to_current = true;
        }
    }

    /// The next (`forward`) or previous change, wrapping around.
    pub fn step(&mut self, forward: bool) {
        let count = self.accepted.len();
        if count > 0 {
            let next = if forward {
                (self.current + 1) % count
            } else {
                (self.current + count - 1) % count
            };
            self.select(next);
        }
    }

    /// The script the current selection makes.
    pub fn merged(&self, base: &str, hunks: &[DiffHunk]) -> String {
        diff::merge_selected(base, hunks, &self.accepted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_manager::diff::DiffSnapshot;

    fn numbered(count: usize) -> String {
        (1..=count).map(|i| format!("line {i}\n")).collect()
    }

    fn rows_between(base: &str, script: &str) -> Vec<Row> {
        let snapshot = DiffSnapshot::compute(base, script);
        rows(base, script, snapshot.hunks(), &HashSet::new())
    }

    fn describe(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Header { hunk } => format!("#{hunk}"),
                Row::Context { text, .. } => format!(" {text}"),
                Row::Removed { text, .. } => format!("-{text}"),
                Row::Added { text, .. } => format!("+{text}"),
                Row::Collapsed { count, .. } => format!("⋯{count}"),
            })
            .collect()
    }

    #[test]
    fn identical_texts_have_no_rows() {
        assert!(rows_between("a\nb\n", "a\nb\n").is_empty());
        assert!(rows_between("", "").is_empty());
    }

    #[test]
    fn a_change_in_the_middle_keeps_three_lines_each_side_and_folds_the_rest() {
        let base = numbered(20);
        let script = base.replace("line 10\n", "LINE 10\n");
        let rows = rows_between(&base, &script);
        assert_eq!(
            describe(&rows),
            vec![
                "⋯6", " line 7", " line 8", " line 9", "#0", "-line 10", "+LINE 10", " line 11",
                " line 12", " line 13", "⋯7"
            ]
        );
    }

    #[test]
    fn line_numbers_follow_the_old_and_new_text() {
        let base = "a\nb\nc\n";
        let script = "a\nx\ny\nc\n";
        let rows = rows_between(base, script);
        let numbers: Vec<(Option<usize>, Option<usize>)> = rows
            .iter()
            .map(|row| match row {
                Row::Context { old, new, .. } => (Some(*old), Some(*new)),
                Row::Removed { old, .. } => (Some(*old), None),
                Row::Added { new, .. } => (None, Some(*new)),
                _ => (None, None),
            })
            .collect();
        assert_eq!(
            numbers,
            vec![
                (Some(0), Some(0)),
                (None, None),
                (Some(1), None),
                (None, Some(1)),
                (None, Some(2)),
                (Some(2), Some(3))
            ]
        );
    }

    #[test]
    fn a_change_at_the_first_or_last_line_has_no_fold_on_that_side() {
        let base = numbered(12);
        let first = rows_between(&base, &base.replace("line 1\n", "ONE\n"));
        assert_eq!(describe(&first)[0], "#0", "nothing above the first line");
        let last = rows_between(&base, &base.replace("line 12\n", "TWELVE\n"));
        let last = describe(&last);
        assert_eq!(
            last.last().unwrap(),
            "+TWELVE",
            "nothing below the last line"
        );
        assert!(last[0].starts_with('⋯'));
    }

    #[test]
    fn two_changes_close_together_show_the_lines_between_them() {
        let base = numbered(12);
        let script = base
            .replace("line 3\n", "THREE\n")
            .replace("line 8\n", "EIGHT\n");
        let described = describe(&rows_between(&base, &script));
        // Four unchanged lines lie between them, and three are kept on each side, so nothing folds.
        assert!(
            described.iter().all(|r| !r.starts_with('⋯')),
            "{described:?}"
        );
        for line in ["line 4", "line 5", "line 6", "line 7"] {
            assert!(described.contains(&format!(" {line}")), "{line}");
        }
        // Far apart, the run between them folds.
        let base = numbered(30);
        let script = base
            .replace("line 3\n", "THREE\n")
            .replace("line 28\n", "TWENTY-EIGHT\n");
        let described = describe(&rows_between(&base, &script));
        assert_eq!(described.iter().filter(|r| r.starts_with('⋯')).count(), 1);
    }

    #[test]
    fn a_fold_of_a_single_line_is_not_worth_a_row() {
        // One unchanged line beyond the context on each side: shown, not folded.
        let base = numbered(9);
        let script = base.replace("line 5\n", "FIVE\n");
        let described = describe(&rows_between(&base, &script));
        assert!(
            described.iter().all(|r| !r.starts_with('⋯')),
            "{described:?}"
        );
    }

    #[test]
    fn opening_a_fold_shows_every_line_of_it() {
        let base = numbered(30);
        let script = base.replace("line 15\n", "FIFTEEN\n");
        let snapshot = DiffSnapshot::compute(&base, &script);
        let collapsed = rows(&base, &script, snapshot.hunks(), &HashSet::new());
        let key = collapsed
            .iter()
            .find_map(|r| match r {
                Row::Collapsed { key, .. } => Some(*key),
                _ => None,
            })
            .unwrap();
        let open = rows(&base, &script, snapshot.hunks(), &HashSet::from([key]));
        assert!(open.len() > collapsed.len());
        let contexts = |rows: &[Row]| {
            rows.iter()
                .filter(|r| matches!(r, Row::Context { .. }))
                .count()
        };
        // Only the fold that was opened opens: the 14 lines above the change, plus three below.
        assert_eq!(contexts(&open), 17);
        let both: HashSet<usize> = collapsed
            .iter()
            .filter_map(|r| match r {
                Row::Collapsed { key, .. } => Some(*key),
                _ => None,
            })
            .collect();
        let all = rows(&base, &script, snapshot.hunks(), &both);
        assert_eq!(contexts(&all), 29, "every unchanged line");
        assert!(all.iter().all(|r| !matches!(r, Row::Collapsed { .. })));
    }

    #[test]
    fn additions_and_deletions_alone_make_one_sided_changes() {
        let added = describe(&rows_between("a\nb\n", "a\nx\nb\n"));
        assert_eq!(added, vec![" a", "#0", "+x", " b"]);
        let removed = describe(&rows_between("a\nx\nb\n", "a\nb\n"));
        assert_eq!(removed, vec![" a", "#0", "-x", " b"]);
    }

    #[test]
    fn crlf_lines_are_shown_without_their_terminators() {
        let rows = rows_between("a\r\nb\r\n", "a\r\nB\r\n");
        assert!(describe(&rows).contains(&"-b".to_owned()));
        assert!(describe(&rows).contains(&"+B".to_owned()));
    }

    #[test]
    fn a_modified_line_carries_its_changed_words() {
        let rows = rows_between(
            "plot(x, color = color.red)\n",
            "plot(x, color = color.orange)\n",
        );
        let removed = rows.iter().find_map(|r| match r {
            Row::Removed { text, words, .. } => Some((text.clone(), words.clone())),
            _ => None,
        });
        let (text, words) = removed.unwrap();
        assert_eq!(&text[words[0].clone()], "red");
        let added = rows.iter().find_map(|r| match r {
            Row::Added { text, words, .. } => Some((text.clone(), words.clone())),
            _ => None,
        });
        let (text, words) = added.unwrap();
        assert_eq!(&text[words[0].clone()], "orange");
    }

    #[test]
    fn rows_know_their_change_and_headers_can_be_found() {
        let base = numbered(20);
        let script = base
            .replace("line 2\n", "TWO\n")
            .replace("line 18\n", "EIGHTEEN\n");
        let rows = rows_between(&base, &script);
        let header = header_row(&rows, 1).unwrap();
        assert_eq!(rows[header].hunk(), Some(1));
        assert_eq!(header_row(&rows, 5), None);
        assert!(rows.iter().any(|r| r.hunk().is_none()));
    }

    #[test]
    fn selection_state_toggles_steps_and_wraps() {
        let mut review = Review::new(3, 0, 0);
        assert_eq!(review.selected(), 3);
        review.toggle(1);
        assert_eq!(review.accepted, vec![true, false, true]);
        assert_eq!(review.selected(), 2);
        review.toggle(9); // out of range: ignored
        review.step(true);
        assert_eq!(review.current, 1);
        review.step(true);
        review.step(true);
        assert_eq!(review.current, 0, "wraps forward");
        review.step(false);
        assert_eq!(review.current, 2, "wraps back");
        review.set_all(false);
        assert_eq!(review.selected(), 0);
        review.set_all(true);
        assert_eq!(review.selected(), 3);
        review.scroll_to_current = false;
        review.select(1);
        assert!(review.scroll_to_current);
        review.select(7); // out of range: ignored
        assert_eq!(review.current, 1);
        let mut empty = Review::new(0, 0, 0);
        empty.step(true);
        assert_eq!(empty.current, 0);
    }

    #[test]
    fn the_merged_script_follows_the_selection() {
        let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
        let script = "ONE\ntwo\nthree\nfour\nfive\nsix\nSEVEN\n";
        let snapshot = DiffSnapshot::compute(base, script);
        let mut review = Review::new(snapshot.hunks().len(), 0, 0);
        assert_eq!(review.merged(base, snapshot.hunks()), script);
        review.toggle(0);
        assert_eq!(
            review.merged(base, snapshot.hunks()),
            "one\ntwo\nthree\nfour\nfive\nsix\nSEVEN\n"
        );
        review.set_all(false);
        assert_eq!(review.merged(base, snapshot.hunks()), base);
    }

    #[test]
    fn opening_builds_the_rows_and_folds_can_be_opened_by_refreshing() {
        let base = numbered(30);
        let script = base.replace("line 15\n", "FIFTEEN IS A LONGER LINE\n");
        let snapshot = DiffSnapshot::compute(&base, &script);
        let mut review = Review::open(&base, &script, snapshot.hunks(), 0, 0);
        assert!(!review.rows.is_empty());
        assert_eq!(review.max_chars, "FIFTEEN IS A LONGER LINE".chars().count());
        let before = review.rows.len();
        review.expanded.insert(0);
        review.refresh_rows(&base, &script, snapshot.hunks());
        assert!(review.rows.len() > before);
    }

    #[test]
    fn a_fold_hides_at_least_four_lines() {
        // Change at line 5 of 11: the run after it is 6 lines, 3 kept, 3 hidden: not folded.
        let base = numbered(11);
        let script = base.replace("line 5\n", "FIVE\n");
        let described = describe(&rows_between(&base, &script));
        assert!(
            described.iter().all(|r| !r.starts_with('⋯')),
            "{described:?}"
        );
        // One line longer and four hide: folded.
        let base = numbered(12);
        let script = base.replace("line 5\n", "FIVE\n");
        let described = describe(&rows_between(&base, &script));
        assert!(described.contains(&"⋯4".to_owned()), "{described:?}");
    }
}
