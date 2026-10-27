//! The one comparison between a script's saved text and what is in the editor. The change gutter,
//! the `+N −M` chip, the diff preview and the hunk revert all read the same [`DiffSnapshot`].
//!
//! Pure: no egui. Lines keep their terminators (`split_inclusive('\n')`), so CRLF, a missing final
//! newline, tabs and Unicode round-trip byte for byte, and a terminator-only change shows as a
//! modified line. Ranges are zero-based and half-open; the UI adds one when it shows a number.

use super::edit_ops::Edit;
use similar::{Algorithm, DiffOp};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::time::{Duration, Instant};

/// A very large or pathological pair of texts falls back to a coarser (still valid) diff after this.
const DEADLINE: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
}

/// One contiguous change. `Added` has an empty `old_lines`; `Deleted` has an empty `new_lines`,
/// whose `start` is the boundary in the current text where the lines used to be (no fake lines).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffHunk {
    pub kind: ChangeKind,
    pub old_lines: Range<usize>,
    pub new_lines: Range<usize>,
    /// The saved lines, terminators included.
    pub old_text: String,
    /// The current lines, terminators included.
    pub new_text: String,
}

impl DiffHunk {
    /// The same change, whatever line it has moved to since (edits above shift `new_lines`).
    pub fn same_change(&self, other: &DiffHunk) -> bool {
        self.kind == other.kind
            && self.old_lines == other.old_lines
            && self.old_text == other.old_text
            && self.new_text == other.new_text
    }

    pub fn added_lines(&self) -> usize {
        self.new_lines.len()
    }

    pub fn removed_lines(&self) -> usize {
        self.old_lines.len()
    }
}

/// The hunks between a saved text and a current one, tagged with what they were computed from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiffSnapshot {
    hunks: Vec<DiffHunk>,
}

/// The text a hunk was computed from is not what is in the editor any more.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stale;

pub fn text_key(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

impl DiffSnapshot {
    pub fn compute(saved: &str, current: &str) -> Self {
        if saved == current {
            return Self::default();
        }
        let old = lines(saved);
        let new = lines(current);
        let ops = similar::capture_diff_slices_deadline(
            Algorithm::Myers,
            &old,
            &new,
            Some(Instant::now() + DEADLINE),
        );
        let hunks = ops
            .into_iter()
            .filter_map(|op| {
                let (kind, old_lines, new_lines) = match op {
                    DiffOp::Equal { .. } => return None,
                    DiffOp::Delete {
                        old_index,
                        old_len,
                        new_index,
                    } => (
                        ChangeKind::Deleted,
                        old_index..old_index + old_len,
                        new_index..new_index,
                    ),
                    DiffOp::Insert {
                        old_index,
                        new_index,
                        new_len,
                    } => (
                        ChangeKind::Added,
                        old_index..old_index,
                        new_index..new_index + new_len,
                    ),
                    DiffOp::Replace {
                        old_index,
                        old_len,
                        new_index,
                        new_len,
                    } => (
                        ChangeKind::Modified,
                        old_index..old_index + old_len,
                        new_index..new_index + new_len,
                    ),
                };
                Some(DiffHunk {
                    kind,
                    old_text: old[old_lines.clone()].concat(),
                    new_text: new[new_lines.clone()].concat(),
                    old_lines,
                    new_lines,
                })
            })
            .collect();
        Self { hunks }
    }

    pub fn hunks(&self) -> &[DiffHunk] {
        &self.hunks
    }

    pub fn is_clean(&self) -> bool {
        self.hunks.is_empty()
    }

    pub fn lines_added(&self) -> usize {
        self.hunks.iter().map(DiffHunk::added_lines).sum()
    }

    pub fn lines_removed(&self) -> usize {
        self.hunks.iter().map(DiffHunk::removed_lines).sum()
    }

    /// The indices of the hunks that touch current lines `first..=last` (a deleted hunk touches the line it sits
    /// above), found by binary search: hunks are ordered and never overlap.
    pub fn in_view(&self, first: usize, last: usize) -> Range<usize> {
        let from = self.hunks.partition_point(|h| h.new_lines.end < first);
        let to = self.hunks.partition_point(|h| h.new_lines.start <= last);
        from..to.max(from)
    }

    /// Index of the hunk that is the same change as `wanted`, wherever it sits now.
    pub fn find(&self, wanted: &DiffHunk) -> Option<usize> {
        self.hunks.iter().position(|h| h.same_change(wanted))
    }

    /// The hunk after (`forward`) or before the one at `index`, wrapping around.
    pub fn neighbour(&self, index: usize, forward: bool) -> Option<usize> {
        let count = self.hunks.len();
        (count > 0).then(|| {
            if forward {
                (index + 1) % count
            } else {
                (index + count - 1) % count
            }
        })
    }
}

/// `base` with only the accepted hunks applied: unchanged lines from `base`, and for each hunk its
/// new lines if accepted, else its old ones. `hunks` must come from `DiffSnapshot::compute(base, ..)`.
/// With every hunk accepted this is the new text; with none, `base`.
pub fn merge_selected(base: &str, hunks: &[DiffHunk], accepted: &[bool]) -> String {
    let old = lines(base);
    let mut out = String::with_capacity(base.len());
    let mut position = 0;
    for (hunk, accept) in hunks.iter().zip(accepted) {
        out.extend(old[position..hunk.old_lines.start].iter().copied());
        out.push_str(if *accept {
            &hunk.new_text
        } else {
            &hunk.old_text
        });
        position = hunk.old_lines.end;
    }
    out.extend(old[position..].iter().copied());
    out
}

/// A line longer than this is not compared word by word; the line tint is enough.
const MAX_WORD_DIFF_LINE: usize = 300;
const WORD_DEADLINE: Duration = Duration::from_millis(10);

/// The words that differ between a line and its replacement, as byte ranges into each.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WordRanges {
    pub old: Vec<Range<usize>>,
    pub new: Vec<Range<usize>>,
}

/// Identifier-like runs, whitespace runs, and every other character on its own.
fn tokens(line: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq)]
    enum Kind {
        Word,
        Space,
        Other,
    }
    let kind = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Kind::Word
        } else if c.is_whitespace() {
            Kind::Space
        } else {
            Kind::Other
        }
    };
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut last: Option<Kind> = None;
    for (index, c) in line.char_indices() {
        let k = kind(c);
        let end = index + c.len_utf8();
        match (&last, out.last_mut()) {
            (Some(previous), Some(range)) if *previous == k && k != Kind::Other => range.end = end,
            _ => out.push(index..end),
        }
        last = Some(k);
    }
    out
}

/// Joins ranges that touch or are separated only by whitespace in `line`.
fn coalesce(line: &str, ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match out.last_mut() {
            Some(last) if line[last.end..range.start].trim().is_empty() => last.end = range.end,
            _ => out.push(range),
        }
    }
    out
}

/// Which words changed from `old` to `new` (both without line terminators). Empty when the lines
/// are equal, too long, or so different that highlighting words would paint the whole line.
pub fn word_ranges(old: &str, new: &str) -> WordRanges {
    if old == new || old.len() > MAX_WORD_DIFF_LINE || new.len() > MAX_WORD_DIFF_LINE {
        return WordRanges::default();
    }
    let (old_tokens, new_tokens) = (tokens(old), tokens(new));
    let old_text: Vec<&str> = old_tokens.iter().map(|r| &old[r.clone()]).collect();
    let new_text: Vec<&str> = new_tokens.iter().map(|r| &new[r.clone()]).collect();
    let ops = similar::capture_diff_slices_deadline(
        Algorithm::Myers,
        &old_text,
        &new_text,
        Some(Instant::now() + WORD_DEADLINE),
    );
    let span = |tokens: &[Range<usize>], from: usize, len: usize| {
        (len > 0).then(|| tokens[from].start..tokens[from + len - 1].end)
    };
    let (mut old_out, mut new_out) = (Vec::new(), Vec::new());
    for op in ops {
        match op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete {
                old_index, old_len, ..
            } => old_out.extend(span(&old_tokens, old_index, old_len)),
            DiffOp::Insert {
                new_index, new_len, ..
            } => new_out.extend(span(&new_tokens, new_index, new_len)),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                old_out.extend(span(&old_tokens, old_index, old_len));
                new_out.extend(span(&new_tokens, new_index, new_len));
            }
        }
    }
    let (old_out, new_out) = (coalesce(old, old_out), coalesce(new, new_out));
    let changed = |ranges: &[Range<usize>]| ranges.iter().map(Range::len).sum::<usize>();
    // Most of the line changed: the line tint says so already.
    if changed(&old_out) * 10 > old.len() * 7 || changed(&new_out) * 10 > new.len() * 7 {
        return WordRanges::default();
    }
    WordRanges {
        old: old_out,
        new: new_out,
    }
}

/// Char offset (what egui's cursor counts) of the start of every line of `text`, plus the total.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, c) in text.chars().enumerate() {
        if c == '\n' {
            starts.push(index + 1);
        }
    }
    let total = text.chars().count();
    if starts.last() != Some(&total) {
        starts.push(total);
    }
    starts
}

/// The edit that puts the saved lines back in place of `hunk`'s current ones: removes an added
/// block, restores a deleted one, replaces a modified one. `Stale` unless the current text still
/// holds exactly the lines the hunk describes where it says. The caret ends after the restored
/// text (at the start of the spot, for a removal).
pub fn revert_edit(hunk: &DiffHunk, current: &str) -> Result<Edit, Stale> {
    let starts = line_starts(current);
    let at = |line: usize| starts.get(line).copied().ok_or(Stale);
    let range = at(hunk.new_lines.start)?..at(hunk.new_lines.end)?;
    let present: String = current
        .chars()
        .skip(range.start)
        .take(range.len())
        .collect();
    if present != hunk.new_text {
        return Err(Stale);
    }
    let end = range.start + hunk.old_text.chars().count();
    Ok(Edit {
        selection: end..end,
        replacement: hunk.old_text.clone(),
        range,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunks(saved: &str, current: &str) -> Vec<DiffHunk> {
        DiffSnapshot::compute(saved, current).hunks
    }

    fn one(saved: &str, current: &str) -> DiffHunk {
        let mut all = hunks(saved, current);
        assert_eq!(all.len(), 1, "{all:?}");
        all.remove(0)
    }

    /// Reverting every hunk, last first, must give the saved text back exactly.
    fn revert_all(saved: &str, current: &str) -> String {
        let snapshot = DiffSnapshot::compute(saved, current);
        let mut text = current.to_owned();
        for hunk in snapshot.hunks().iter().rev() {
            text = revert_edit(hunk, &text).expect("fresh hunk").apply(&text);
        }
        text
    }

    #[test]
    fn identical_text_has_no_hunks() {
        assert!(DiffSnapshot::compute("a\nb\n", "a\nb\n").is_clean());
        assert!(DiffSnapshot::compute("", "").is_clean());
    }

    #[test]
    fn one_added_line() {
        let hunk = one("a\nc\n", "a\nb\nc\n");
        assert_eq!(hunk.kind, ChangeKind::Added);
        assert_eq!((hunk.old_lines, hunk.new_lines), (1..1, 1..2));
        assert_eq!(
            (hunk.old_text.as_str(), hunk.new_text.as_str()),
            ("", "b\n")
        );
    }

    #[test]
    fn contiguous_added_lines_are_one_hunk() {
        let hunk = one("a\nz\n", "a\nx\ny\nw\nz\n");
        assert_eq!(hunk.kind, ChangeKind::Added);
        assert_eq!(hunk.new_lines, 1..4);
        assert_eq!(hunk.new_text, "x\ny\nw\n");
    }

    #[test]
    fn deleted_lines_sit_at_a_boundary_with_no_current_lines() {
        let hunk = one("a\nb\nc\n", "a\nc\n");
        assert_eq!(hunk.kind, ChangeKind::Deleted);
        assert_eq!((hunk.old_lines, hunk.new_lines), (1..2, 1..1));
        assert_eq!(hunk.old_text, "b\n");

        let hunk = one("a\nb\nc\nd\n", "a\nd\n");
        assert_eq!((hunk.old_lines, hunk.new_lines), (1..3, 1..1));
        assert_eq!(hunk.old_text, "b\nc\n");
    }

    #[test]
    fn a_changed_line_is_modified_not_added_and_deleted() {
        let hunk = one("length = input.int(14)\n", "length = input.int(21)\n");
        assert_eq!(hunk.kind, ChangeKind::Modified);
        assert_eq!((hunk.old_lines, hunk.new_lines), (0..1, 0..1));
        assert_eq!(hunk.old_text, "length = input.int(14)\n");
        assert_eq!(hunk.new_text, "length = input.int(21)\n");
    }

    #[test]
    fn three_lines_replaced_by_one_is_one_modified_hunk() {
        let hunk = one("a\nb\nc\nd\ne\n", "a\nX\ne\n");
        assert_eq!(hunk.kind, ChangeKind::Modified);
        assert_eq!((hunk.old_lines, hunk.new_lines), (1..4, 1..2));
    }

    #[test]
    fn insertion_at_the_top_is_one_added_line_not_a_shifted_file() {
        let hunk = one("A\nB\nC\nD\n", "X\nA\nB\nC\nD\n");
        assert_eq!(hunk.kind, ChangeKind::Added);
        assert_eq!((hunk.old_lines, hunk.new_lines), (0..0, 0..1));
    }

    #[test]
    fn insertion_and_deletion_at_both_ends() {
        let hunk = one("a\nb\n", "a\nb\nc\n");
        assert_eq!((hunk.kind, hunk.new_lines), (ChangeKind::Added, 2..3));
        let hunk = one("a\nb\nc\n", "a\nb\n");
        assert_eq!(
            (hunk.kind, hunk.old_lines, hunk.new_lines),
            (ChangeKind::Deleted, 2..3, 2..2)
        );
        let hunk = one("a\nb\nc\n", "b\nc\n");
        assert_eq!(
            (hunk.kind, hunk.old_lines, hunk.new_lines),
            (ChangeKind::Deleted, 0..1, 0..0)
        );
    }

    #[test]
    fn whole_file_deleted_and_empty_baseline() {
        let hunk = one("a\nb\n", "");
        assert_eq!(
            (hunk.kind, hunk.old_lines, hunk.new_lines),
            (ChangeKind::Deleted, 0..2, 0..0)
        );
        let hunk = one("", "a\nb\n");
        assert_eq!(
            (hunk.kind, hunk.old_lines, hunk.new_lines),
            (ChangeKind::Added, 0..0, 0..2)
        );
    }

    #[test]
    fn separate_changes_are_separate_hunks_in_order() {
        let all = hunks("a\nb\nc\nd\ne\nf\n", "a\nB\nc\nd\nf\nz\n");
        let kinds: Vec<_> = all.iter().map(|h| h.kind).collect();
        assert_eq!(
            kinds,
            [ChangeKind::Modified, ChangeKind::Deleted, ChangeKind::Added]
        );
        assert!(
            all.windows(2)
                .all(|w| w[0].new_lines.end <= w[1].new_lines.start)
        );
    }

    #[test]
    fn repeated_identical_lines() {
        let hunk = one("x\nx\nx\n", "x\nx\nx\nx\n");
        assert_eq!(hunk.kind, ChangeKind::Added);
        assert_eq!(hunk.new_lines.len(), 1);
        let hunk = one("x\nx\nx\nx\n", "x\nx\n");
        assert_eq!(hunk.kind, ChangeKind::Deleted);
        assert_eq!(hunk.old_lines.len(), 2);
    }

    #[test]
    fn a_moved_line_is_a_delete_plus_an_add() {
        let all = hunks("a\nb\nc\nd\n", "b\nc\nd\na\n");
        assert_eq!(all.len(), 2);
        assert_eq!(all.iter().map(DiffHunk::added_lines).sum::<usize>(), 1);
        assert_eq!(all.iter().map(DiffHunk::removed_lines).sum::<usize>(), 1);
    }

    #[test]
    fn adjacent_insert_and_delete_merge_into_one_modified_hunk() {
        let hunk = one("a\nb\nc\n", "a\nx\ny\nc\n");
        assert_eq!(hunk.kind, ChangeKind::Modified);
        assert_eq!((hunk.old_lines, hunk.new_lines), (1..2, 1..3));
    }

    #[test]
    fn line_endings_are_part_of_the_line() {
        // LF → CRLF on one line is a modified line; the others stay untouched.
        let hunk = one("a\nb\nc\n", "a\r\nb\nc\n");
        assert_eq!(hunk.kind, ChangeKind::Modified);
        assert_eq!(
            (hunk.old_text.as_str(), hunk.new_text.as_str()),
            ("a\n", "a\r\n")
        );
        // A CRLF file with one added line stays CRLF.
        let hunk = one("a\r\nb\r\n", "a\r\nx\r\nb\r\n");
        assert_eq!(hunk.new_text, "x\r\n");
    }

    #[test]
    fn final_newline_changes_show() {
        let hunk = one("a\nb", "a\nb\n");
        assert_eq!(hunk.kind, ChangeKind::Modified);
        assert_eq!(
            (hunk.old_text.as_str(), hunk.new_text.as_str()),
            ("b", "b\n")
        );
        let hunk = one("a\nb\n", "a\nb");
        assert_eq!(hunk.kind, ChangeKind::Modified);
        // Appending a line to a file with no final newline.
        let hunk = one("a", "a\nb");
        assert_eq!(hunk.kind, ChangeKind::Modified);
        assert_eq!(hunk.new_text, "a\nb");
    }

    #[test]
    fn unicode_and_whitespace_only_changes() {
        let hunk = one(
            "名前 = \"héllo\"\nplot(close)\n",
            "名前 = \"héllo 🚀\"\nplot(close)\n",
        );
        assert_eq!(hunk.kind, ChangeKind::Modified);
        let hunk = one("a\n\tb\n", "a\n    b\n");
        assert_eq!(hunk.kind, ChangeKind::Modified);
        let hunk = one("a\nb\n", "a\n\nb\n");
        assert_eq!(
            (hunk.kind, hunk.new_text.as_str()),
            (ChangeKind::Added, "\n")
        );
    }

    #[test]
    fn counts_come_from_the_hunks() {
        let snapshot = DiffSnapshot::compute("a\nb\nc\nd\n", "a\nX\nY\nd\ne\n");
        assert_eq!(snapshot.lines_added(), 3);
        assert_eq!(snapshot.lines_removed(), 2);
        assert!(!snapshot.is_clean());
    }

    #[test]
    fn a_line_edited_then_restored_is_no_longer_changed() {
        assert!(DiffSnapshot::compute("a\nb\n", "a\nb\n").is_clean());
        let edited = "a\nbb\n";
        assert!(!DiffSnapshot::compute("a\nb\n", edited).is_clean());
        assert!(DiffSnapshot::compute("a\nb\n", &edited.replace("bb", "b")).is_clean());
    }

    #[test]
    fn in_view_picks_only_visible_hunks() {
        let saved: String = (0..100).map(|i| format!("l{i}\n")).collect();
        let mut current = saved
            .replace("l10\n", "L10\n")
            .replace("l50\n", "")
            .replace("l90\n", "L90\n");
        current.push_str("tail\n");
        let snapshot = DiffSnapshot::compute(&saved, &current);
        assert_eq!(snapshot.hunks().len(), 4);
        let seen = snapshot.in_view(5, 20);
        assert_eq!(seen.len(), 1);
        assert_eq!(snapshot.hunks()[seen.start].new_text, "L10\n");
        // A deleted hunk shows with the line it sits above.
        assert_eq!(snapshot.in_view(50, 50).len(), 1);
        assert!(snapshot.in_view(30, 40).is_empty());
    }

    // MARK: Revert

    #[test]
    fn revert_added_lines_leaves_the_rest() {
        let saved = "plot(close)\n";
        let current = "fast = ta.ema(close, 9)\nslow = ta.ema(close, 21)\nplot(close)\n";
        let hunk = one(saved, current);
        let edit = revert_edit(&hunk, current).unwrap();
        assert_eq!(edit.apply(current), saved);
        assert_eq!(edit.selection, 0..0);
    }

    #[test]
    fn restore_deleted_lines_in_the_middle_start_and_end() {
        for (saved, current) in [
            ("a = 1\nb = 2\nc = 3\n", "a = 1\nc = 3\n"),
            ("a = 1\nb = 2\nc = 3\n", "b = 2\nc = 3\n"),
            ("a = 1\nb = 2\nc = 3\n", "a = 1\nb = 2\n"),
            ("a = 1\nb = 2\nc = 3\n", ""),
            ("a = 1\nb = 2\nc = 3", "a = 1\nb = 2\n"),
        ] {
            let hunk = one(saved, current);
            assert_eq!(hunk.kind, ChangeKind::Deleted);
            let edit = revert_edit(&hunk, current).unwrap();
            assert_eq!(edit.apply(current), saved, "{current:?}");
            let end = edit.range.start + hunk.old_text.chars().count();
            assert_eq!(edit.selection, end..end);
        }
    }

    #[test]
    fn revert_modified_touches_only_its_lines() {
        let saved = "length = input.int(14)\nplot(ta.ema(close, length))\nplot(volume)\n";
        let current = "length = input.int(21)\nplot(ta.sma(close, length))\nplot(volume * 2)\n";
        let snapshot = DiffSnapshot::compute(saved, current);
        assert_eq!(snapshot.hunks().len(), 1);
        // Edit elsewhere first: only the chosen hunk reverts.
        let current =
            "length = input.int(21)\nplot(ta.sma(close, length))\nplot(volume)\nplot(open)\n";
        let hunks = hunks(saved, current);
        assert_eq!(hunks.len(), 2);
        let edit = revert_edit(&hunks[0], current).unwrap();
        assert_eq!(
            edit.apply(current),
            "length = input.int(14)\nplot(ta.ema(close, length))\nplot(volume)\nplot(open)\n"
        );
    }

    #[test]
    fn revert_keeps_line_endings_and_unicode_exact() {
        let saved = "名前 = 1\r\nb = \"é\"\r\nc = 3\r\n";
        let current = "名前 = 1\r\nc = 3\r\n";
        let hunk = one(saved, current);
        assert_eq!(revert_edit(&hunk, current).unwrap().apply(current), saved);
        let current = "x = 0\r\n名前 = 1\r\nb = \"é\"\r\nc = 3\r\n";
        let hunk = one(saved, current);
        assert_eq!(revert_edit(&hunk, current).unwrap().apply(current), saved);
        // No final newline either side.
        assert_eq!(revert_all("a\nb", "a\nb\nc"), "a\nb");
        assert_eq!(revert_all("a\nb\n", "a\nb"), "a\nb\n");
    }

    #[test]
    fn a_stale_hunk_is_rejected_not_applied() {
        let saved = "a\nb\nc\n";
        let current = "a\nX\nc\n";
        let hunk = one(saved, current);
        // The user kept typing in the hunk's line after the diff was taken.
        assert_eq!(revert_edit(&hunk, "a\nXY\nc\n"), Err(Stale));
        // Or the text is shorter than the hunk's range.
        assert_eq!(revert_edit(&hunk, "a\n"), Err(Stale));
        // Lines inserted above shift it: same text at other lines is stale, never "close enough".
        assert_eq!(revert_edit(&hunk, "new\na\nX\nc\n"), Err(Stale));
    }

    #[test]
    fn find_follows_a_hunk_across_edits_above_it() {
        let saved: String = (0..10).map(|i| format!("l{i}\n")).collect();
        let before = saved.replace("l8\n", "L8\n");
        let hunk = one(&saved, &before);
        let after = format!("top\n{before}");
        let snapshot = DiffSnapshot::compute(&saved, &after);
        // The new top line is its own hunk; the L8 one is still found, on its new line.
        let found = snapshot.find(&hunk).expect("same change");
        assert_eq!(snapshot.hunks()[found].new_lines, 9..10);
        assert_eq!(snapshot.neighbour(found, false), Some(0));
        assert_eq!(snapshot.neighbour(found, true), Some(0));
    }

    #[test]
    fn every_edit_reverts_to_the_baseline_and_agrees_with_inequality() {
        // A deterministic spread of edits over a base text, including awkward line endings.
        let base: Vec<String> = (0..40).map(|i| format!("line {i} é\n")).collect();
        let saved = base.concat();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        for round in 0..300 {
            let mut lines = base.clone();
            for _ in 0..next(6) {
                let at = next(lines.len().max(1)).min(lines.len().saturating_sub(1));
                match next(5) {
                    0 if !lines.is_empty() => {
                        lines.remove(at);
                    }
                    1 => lines.insert(at.min(lines.len()), format!("new {round}\n")),
                    2 if !lines.is_empty() => lines[at] = format!("changed {round}\n"),
                    3 if !lines.is_empty() => lines[at] = lines[at].replace('\n', "\r\n"),
                    _ if !lines.is_empty() => lines[at] = lines[at].trim_end().to_owned(),
                    _ => {}
                }
            }
            let current = lines.concat();
            let snapshot = DiffSnapshot::compute(&saved, &current);
            assert_eq!(snapshot.is_clean(), saved == current, "round {round}");
            assert_eq!(revert_all(&saved, &current), saved, "round {round}");
            // The hunks are ordered, disjoint, and account for every line difference.
            assert!(
                snapshot
                    .hunks()
                    .windows(2)
                    .all(|w| w[0].new_lines.end <= w[1].new_lines.start
                        && w[0].old_lines.end <= w[1].old_lines.start)
            );
            let delta = snapshot.lines_added() as isize - snapshot.lines_removed() as isize;
            assert_eq!(
                delta,
                current.split_inclusive('\n').count() as isize
                    - saved.split_inclusive('\n').count() as isize,
                "round {round}"
            );
        }
    }

    #[test]
    fn diffing_a_big_script_is_fast() {
        for count in [500usize, 5_000] {
            let saved: String = (0..count)
                .map(|i| format!("value{i} = ta.sma(close, {i})\n"))
                .collect();
            let mut current = saved.clone();
            for step in (0..count).step_by(count / 20) {
                current = current.replace(
                    &format!("value{step} = ta.sma(close, {step})\n"),
                    &format!("value{step} = ta.ema(close, {step})\nextra{step} = 1\n"),
                );
            }
            let started = std::time::Instant::now();
            let snapshot = DiffSnapshot::compute(&saved, &current);
            let took = started.elapsed();
            println!(
                "diff {count} lines, {} hunks: {took:?}",
                snapshot.hunks().len()
            );
            assert_eq!(snapshot.hunks().len(), 20);
            // Loose: unoptimised test builds are an order slower than release.
            assert!(took < Duration::from_millis(250), "{took:?}");
        }
    }

    #[test]
    fn merging_every_hunk_gives_the_new_text_and_none_gives_the_base() {
        let base = "a\nb\nc\nd\ne\n";
        let new = "a\nB\nc\ne\nf\n";
        let snapshot = DiffSnapshot::compute(base, new);
        let hunks = snapshot.hunks();
        assert!(hunks.len() >= 2);
        assert_eq!(merge_selected(base, hunks, &vec![true; hunks.len()]), new);
        assert_eq!(merge_selected(base, hunks, &vec![false; hunks.len()]), base);
    }

    #[test]
    fn merging_a_subset_applies_only_those_hunks() {
        let base = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
        let new = "ONE\ntwo\nthree\nfour\nfive\nsix\nSEVEN\nextra\n";
        let snapshot = DiffSnapshot::compute(base, new);
        let hunks = snapshot.hunks();
        assert_eq!(hunks.len(), 2);
        assert_eq!(
            merge_selected(base, hunks, &[true, false]),
            "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\n"
        );
        assert_eq!(
            merge_selected(base, hunks, &[false, true]),
            "one\ntwo\nthree\nfour\nfive\nsix\nSEVEN\nextra\n"
        );
    }

    #[test]
    fn merging_handles_additions_deletions_crlf_and_a_missing_final_newline() {
        for (base, new) in [
            ("a\nb\n", "a\nx\ny\nb\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("a\r\nb\r\n", "a\r\nB\r\n"),
            ("a\nb", "a\nb\nc"),
            ("", "a\n"),
            ("a\n", ""),
            ("日本\n語\n", "日本\n🙂\n"),
        ] {
            let snapshot = DiffSnapshot::compute(base, new);
            let hunks = snapshot.hunks();
            assert_eq!(
                merge_selected(base, hunks, &vec![true; hunks.len()]),
                new,
                "{base:?} -> {new:?}"
            );
            assert_eq!(
                merge_selected(base, hunks, &vec![false; hunks.len()]),
                base,
                "{base:?} -> {new:?}"
            );
        }
    }

    #[test]
    fn word_ranges_mark_just_the_changed_words() {
        let ranges = word_ranges(
            "plot(value, color = color.red)",
            "plot(value, color = color.orange)",
        );
        assert_eq!(ranges.old.len(), 1);
        assert_eq!(
            &"plot(value, color = color.red)"[ranges.old[0].clone()],
            "red"
        );
        assert_eq!(
            &"plot(value, color = color.orange)"[ranges.new[0].clone()],
            "orange"
        );
        let ranges = word_ranges("x = ta.sma(close, 14)", "x = ta.sma(close, length)");
        assert_eq!(
            &"x = ta.sma(close, length)"[ranges.new[0].clone()],
            "length"
        );
    }

    #[test]
    fn word_ranges_are_empty_for_equal_long_or_wholly_different_lines() {
        assert_eq!(word_ranges("same", "same"), WordRanges::default());
        let long = "a".repeat(400);
        assert_eq!(
            word_ranges(&long, &format!("{long}b")),
            WordRanges::default()
        );
        assert_eq!(word_ranges("foo", "bar"), WordRanges::default());
        assert_eq!(word_ranges("", "x"), WordRanges::default());
    }

    #[test]
    fn word_ranges_land_on_char_boundaries_in_unicode_lines() {
        let old = "label = \"日本語\" + x";
        let new = "label = \"日本人\" + x";
        let ranges = word_ranges(old, new);
        for range in &ranges.old {
            let _ = &old[range.clone()];
        }
        for range in &ranges.new {
            let _ = &new[range.clone()];
        }
    }
}
