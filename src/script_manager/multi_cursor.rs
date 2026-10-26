//! Several carets at once, for the Pine editor, in the manner of Sublime Text.
//!
//! Pure like [`edit_ops`]: text plus selections in, new text plus selections out, every index a
//! `char` index. Typing, Return, Tab and Backspace reuse `edit_ops` per caret (so auto-pairs and
//! auto-indent behave exactly as they do with one caret); each caret's edit is computed against
//! the same original text and the edits are laid down together, which is why they must not
//! overlap. A caret whose edit would overlap an earlier one is dropped, the way two carets
//! that meet become one.

use crate::script_manager::edit_ops::{self, Context, Edit};
use std::ops::Range;

/// The most carets one editor keeps; more than this and a stray Select All would hang the frame.
pub const MAX: usize = 1000;

/// A selection: `head` is the end that moves, `anchor` the end that stays. Empty is a caret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sel {
    pub anchor: usize,
    pub head: usize,
}

impl Sel {
    pub const fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    pub const fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

/// Every selection, with the one egui's own text field mirrors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selections {
    pub sels: Vec<Sel>,
    pub primary: usize,
}

impl Selections {
    pub fn single(sel: Sel) -> Self {
        Self {
            sels: vec![sel],
            primary: 0,
        }
    }

    /// `primary` and the others, in any order.
    pub fn with_extras(primary: Sel, extras: &[Sel]) -> Self {
        let mut sels = vec![primary];
        sels.extend_from_slice(extras);
        Self { sels, primary: 0 }.normalized()
    }

    pub fn primary_sel(&self) -> Sel {
        self.sels[self.primary.min(self.sels.len() - 1)]
    }

    pub fn extras(&self) -> Vec<Sel> {
        let primary = self.primary.min(self.sels.len() - 1);
        self.sels
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != primary)
            .map(|(_, sel)| *sel)
            .collect()
    }

    /// Sorted, with overlapping selections (and carets inside or touching one) merged, and at
    /// most [`MAX`] of them.
    pub fn normalized(self) -> Self {
        let primary = self.primary.min(self.sels.len().saturating_sub(1));
        let mut order: Vec<usize> = (0..self.sels.len()).collect();
        order.sort_by_key(|i| {
            let range = self.sels[*i].range();
            (range.start, range.end)
        });
        let mut merged: Vec<Sel> = Vec::new();
        let mut primary_at = 0;
        for index in order {
            let sel = self.sels[index];
            let touches = merged.last().is_some_and(|last: &Sel| {
                let (last, now) = (last.range(), sel.range());
                now.start < last.end
                    || (now.start == last.end && (now.is_empty() || last.is_empty()))
            });
            if touches {
                let last = merged.last_mut().expect("checked above");
                let (before, now) = (last.range(), sel.range());
                let range = before.start.min(now.start)..before.end.max(now.end);
                *last = if sel.head >= sel.anchor {
                    Sel::new(range.start, range.end)
                } else {
                    Sel::new(range.end, range.start)
                };
            } else {
                merged.push(sel);
            }
            if index == primary {
                primary_at = merged.len() - 1;
            }
        }
        merged.truncate(MAX);
        let primary = primary_at.min(merged.len().saturating_sub(1));
        Self {
            sels: merged,
            primary,
        }
    }
}

/// The result of an edit: the whole new text and where the carets end up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edited {
    pub text: String,
    pub selections: Selections,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

fn line_start(chars: &[char], at: usize) -> usize {
    chars[..at.min(chars.len())]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |i| i + 1)
}

/// End of the line's content: before its `\n`, and before a `\r` that goes with it.
fn line_end(chars: &[char], at: usize) -> usize {
    let at = at.min(chars.len());
    let mut end = chars[at..]
        .iter()
        .position(|c| *c == '\n')
        .map_or(chars.len(), |i| at + i);
    if end > 0 && end > line_start(chars, at) && chars[end - 1] == '\r' {
        end -= 1;
    }
    end
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Applies one edit per caret. `ops` holds `(caret index, edit)`.
fn lay_down(chars: &[char], mut ops: Vec<(usize, Edit)>, primary: usize) -> Option<Edited> {
    ops.sort_by_key(|(_, edit)| (edit.range.start, edit.range.end));
    let mut kept: Vec<(usize, Edit)> = Vec::with_capacity(ops.len());
    for (caret, edit) in ops {
        if kept
            .last()
            .is_some_and(|(_, last)| edit.range.start < last.range.end)
        {
            continue;
        }
        kept.push((caret, edit));
    }
    if kept.is_empty() {
        return None;
    }
    let mut text = String::with_capacity(chars.len());
    let mut taken = 0;
    let mut shift: isize = 0;
    let mut sels = Vec::with_capacity(kept.len());
    let mut primary_at = None;
    for (caret, edit) in &kept {
        text.extend(&chars[taken..edit.range.start]);
        text.push_str(&edit.replacement);
        taken = edit.range.end;
        let at = |index: usize| (index as isize + shift).max(0) as usize;
        sels.push(Sel::new(at(edit.selection.start), at(edit.selection.end)));
        if *caret == primary {
            primary_at = Some(sels.len() - 1);
        }
        shift += edit.replacement.chars().count() as isize
            - (edit.range.end - edit.range.start) as isize;
    }
    text.extend(&chars[taken..]);
    let primary = primary_at.unwrap_or(sels.len() - 1);
    Some(Edited {
        text,
        selections: Selections { sels, primary }.normalized(),
    })
}

fn insert_over(sel: Sel, text: &str) -> Edit {
    let range = sel.range();
    let end = range.start + text.chars().count();
    Edit {
        range: range.clone(),
        replacement: text.to_owned(),
        selection: end..end,
    }
}

/// Runs `make` for every caret against one shared scan of `text`.
fn per_caret(
    text: &str,
    selections: &Selections,
    mut make: impl FnMut(&Context, Sel) -> Option<Edit>,
) -> Option<Edited> {
    let chars: Vec<char> = text.chars().collect();
    let mut context = Context::new(text, 0..0);
    let ops: Vec<(usize, Edit)> = selections
        .sels
        .iter()
        .enumerate()
        .filter_map(|(index, sel)| {
            context.select(sel.range());
            make(&context, *sel).map(|edit| (index, edit))
        })
        .collect();
    lay_down(&chars, ops, selections.primary)
}

/// Typed text at every caret, with the auto-pairing a single caret gets.
pub fn type_text(text: &str, selections: &Selections, typed: &str) -> Option<Edited> {
    per_caret(text, selections, |context, sel| {
        Some(edit_ops::typed(context, typed).unwrap_or_else(|| insert_over(sel, typed)))
    })
}

pub fn newline(text: &str, selections: &Selections) -> Option<Edited> {
    per_caret(text, selections, |context, _| {
        Some(edit_ops::newline(context))
    })
}

pub fn indent(text: &str, selections: &Selections) -> Option<Edited> {
    per_caret(text, selections, |context, _| {
        Some(edit_ops::indent(context))
    })
}

pub fn outdent(text: &str, selections: &Selections) -> Option<Edited> {
    per_caret(text, selections, |context, _| edit_ops::outdent(context))
}

/// Backspace: each selection is deleted; each bare caret removes the character before it, or
/// an empty pair around it.
pub fn backspace(text: &str, selections: &Selections) -> Option<Edited> {
    per_caret(text, selections, |context, sel| {
        if let Some(edit) = edit_ops::backspace(context) {
            return Some(edit);
        }
        let range = sel.range();
        let range = if range.is_empty() {
            range.start.checked_sub(1)?..range.start
        } else {
            range
        };
        Some(Edit {
            selection: range.start..range.start,
            replacement: String::new(),
            range,
        })
    })
}

/// Forward delete.
pub fn delete(text: &str, selections: &Selections) -> Option<Edited> {
    let length = text.chars().count();
    per_caret(text, selections, |_, sel| {
        let range = sel.range();
        let range = if range.is_empty() {
            if range.start >= length {
                return None;
            }
            range.start..range.start + 1
        } else {
            range
        };
        Some(Edit {
            selection: range.start..range.start,
            replacement: String::new(),
            range,
        })
    })
}

/// What Copy puts on the clipboard: the selected text, one selection per line. `None` when
/// nothing is selected, so the single-caret behaviour applies.
pub fn copy_text(text: &str, selections: &Selections) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let parts: Vec<String> = selections
        .sels
        .iter()
        .filter(|sel| !sel.is_empty())
        .map(|sel| chars[sel.range()].iter().collect())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// Cut: the selections removed.
pub fn cut(text: &str, selections: &Selections) -> Option<Edited> {
    if selections.sels.iter().all(|sel| sel.is_empty()) {
        return None;
    }
    delete(text, selections)
}

/// Paste. Text of as many lines as there are carets gives each caret one line, in order;
/// anything else is pasted whole at each.
pub fn paste(text: &str, selections: &Selections, clipboard: &str) -> Option<Edited> {
    let clipboard = clipboard.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = clipboard.split('\n').collect();
    let distribute = lines.len() == selections.sels.len() && lines.len() > 1;
    per_caret(text, selections, |_, sel| {
        let index = selections.sels.iter().position(|other| *other == sel)?;
        let piece = if distribute { lines[index] } else { &clipboard };
        Some(insert_over(sel, piece))
    })
}

fn move_head(chars: &[char], sel: Sel, motion: Motion, extend: bool) -> Sel {
    let head = sel.head.min(chars.len());
    let range = sel.range();
    let target = match motion {
        Motion::Left | Motion::Right if !extend && !sel.is_empty() => {
            let at = if motion == Motion::Left {
                range.start
            } else {
                range.end
            };
            return Sel::caret(at);
        }
        Motion::Left => head.saturating_sub(1),
        Motion::Right => (head + 1).min(chars.len()),
        Motion::Up | Motion::Down => {
            let start = line_start(chars, head);
            let column = head - start;
            if motion == Motion::Up {
                if start == 0 {
                    0
                } else {
                    let above = line_start(chars, start - 1);
                    (above + column).min(line_end(chars, above))
                }
            } else {
                let end = chars[head..]
                    .iter()
                    .position(|c| *c == '\n')
                    .map(|i| head + i);
                match end {
                    None => chars.len(),
                    Some(newline) => {
                        let below = newline + 1;
                        (below + column).min(line_end(chars, below))
                    }
                }
            }
        }
        Motion::WordLeft => {
            let mut at = head;
            while at > 0 && chars[at - 1].is_whitespace() {
                at -= 1;
            }
            if at > 0 {
                let word = is_word(chars[at - 1]);
                while at > 0 && !chars[at - 1].is_whitespace() && is_word(chars[at - 1]) == word {
                    at -= 1;
                }
            }
            at
        }
        Motion::WordRight => {
            let mut at = head;
            while at < chars.len() && chars[at].is_whitespace() {
                at += 1;
            }
            if at < chars.len() {
                let word = is_word(chars[at]);
                while at < chars.len() && !chars[at].is_whitespace() && is_word(chars[at]) == word {
                    at += 1;
                }
            }
            at
        }
        Motion::LineStart => {
            // First press: the first non-blank; again: the very start.
            let start = line_start(chars, head);
            let content = start
                + chars[start..]
                    .iter()
                    .take_while(|c| matches!(c, ' ' | '\t'))
                    .count();
            if head != content && content <= line_end(chars, start) {
                content
            } else {
                start
            }
        }
        Motion::LineEnd => line_end(chars, head),
        Motion::DocStart => 0,
        Motion::DocEnd => chars.len(),
    };
    if extend {
        Sel::new(sel.anchor, target)
    } else {
        Sel::caret(target)
    }
}

/// Moves (or, with `extend`, stretches) every selection.
pub fn moved(text: &str, selections: &Selections, motion: Motion, extend: bool) -> Selections {
    let chars: Vec<char> = text.chars().collect();
    Selections {
        sels: selections
            .sels
            .iter()
            .map(|sel| move_head(&chars, *sel, motion, extend))
            .collect(),
        primary: selections.primary,
    }
    .normalized()
}

fn word_at(chars: &[char], at: usize) -> Option<Range<usize>> {
    let mut index = at.min(chars.len());
    if index == chars.len() || !is_word(chars[index]) {
        if index > 0 && is_word(chars[index - 1]) {
            index -= 1;
        } else {
            return None;
        }
    }
    let mut start = index;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = index;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    Some(start..end)
}

/// Whether `range` is a whole word of the text, so its matches should be too.
fn is_whole_word(chars: &[char], range: &Range<usize>) -> bool {
    !range.is_empty()
        && chars[range.clone()].iter().all(|c| is_word(*c))
        && (range.start == 0 || !is_word(chars[range.start - 1]))
        && (range.end == chars.len() || !is_word(chars[range.end]))
}

fn matches_of(chars: &[char], needle: &[char], whole: bool) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    if needle.is_empty() || needle.len() > chars.len() {
        return found;
    }
    let mut at = 0;
    while at + needle.len() <= chars.len() {
        if chars[at..at + needle.len()] == *needle {
            let range = at..at + needle.len();
            let bounded = !whole
                || ((range.start == 0 || !is_word(chars[range.start - 1]))
                    && (range.end == chars.len() || !is_word(chars[range.end])));
            if bounded {
                found.push(range.clone());
                at = range.end;
                continue;
            }
        }
        at += 1;
    }
    found
}

/// The text Select Next/All hunts for, and whether it matches whole words only.
fn needle(chars: &[char], selections: &Selections) -> Option<(Vec<char>, bool)> {
    let primary = selections.primary_sel();
    let range = if primary.is_empty() {
        word_at(chars, primary.head)?
    } else {
        primary.range()
    };
    let whole = is_whole_word(chars, &range);
    Some((chars[range].to_vec(), whole))
}

/// Cmd+D. With only bare carets: selects the word at each. Otherwise adds the next match of the
/// primary selection (after it, wrapping), as the new primary.
pub fn select_next(text: &str, selections: &Selections) -> Option<Selections> {
    let chars: Vec<char> = text.chars().collect();
    if selections.sels.iter().all(|sel| sel.is_empty()) {
        let sels: Vec<Sel> = selections
            .sels
            .iter()
            .map(|sel| {
                word_at(&chars, sel.head).map_or(*sel, |word| Sel::new(word.start, word.end))
            })
            .collect();
        let changed = sels != selections.sels;
        return changed.then(|| {
            Selections {
                sels,
                primary: selections.primary,
            }
            .normalized()
        });
    }
    let (needle, whole) = needle(&chars, selections)?;
    let all = matches_of(&chars, &needle, whole);
    let after = selections.primary_sel().range().end;
    let taken = |range: &Range<usize>| selections.sels.iter().any(|sel| sel.range() == *range);
    let next = all
        .iter()
        .filter(|range| range.start >= after)
        .chain(all.iter().filter(|range| range.start < after))
        .find(|range| !taken(range))?
        .clone();
    let mut sels = selections.sels.clone();
    sels.push(Sel::new(next.start, next.end));
    let primary = sels.len() - 1;
    Some(Selections { sels, primary }.normalized())
}

/// Alt+F3: a selection on every match of the primary selection (or the word at the caret).
pub fn select_all_matches(text: &str, selections: &Selections) -> Option<Selections> {
    let chars: Vec<char> = text.chars().collect();
    let (needle, whole) = needle(&chars, selections)?;
    let all = matches_of(&chars, &needle, whole);
    if all.is_empty() {
        return None;
    }
    let primary = selections.primary_sel();
    let range = if primary.is_empty() {
        word_at(&chars, primary.head)?
    } else {
        primary.range()
    };
    let primary = all.iter().position(|r| *r == range).unwrap_or(0);
    Some(
        Selections {
            sels: all.iter().map(|r| Sel::new(r.start, r.end)).collect(),
            primary,
        }
        .normalized(),
    )
}

/// Ctrl+Shift+Up/Down: a new caret on the line above the topmost, or below the bottommost,
/// at the same column.
pub fn add_caret_vertically(text: &str, selections: &Selections, up: bool) -> Option<Selections> {
    let chars: Vec<char> = text.chars().collect();
    let edge = if up {
        selections.sels.iter().map(|s| s.head).min()?
    } else {
        selections.sels.iter().map(|s| s.head).max()?
    };
    let start = line_start(&chars, edge);
    let column = edge - start;
    let target = if up {
        if start == 0 {
            return None;
        }
        let above = line_start(&chars, start - 1);
        (above + column).min(line_end(&chars, above))
    } else {
        let newline = chars[edge..].iter().position(|c| *c == '\n')? + edge;
        let below = newline + 1;
        (below + column).min(line_end(&chars, below))
    };
    let mut sels = selections.sels.clone();
    sels.push(Sel::caret(target));
    let primary = sels.len() - 1;
    Some(Selections { sels, primary }.normalized())
}

/// A position as a line and a (possibly past-the-end) column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
    pub line: usize,
    pub column: usize,
}

/// The cell for a character index.
pub fn cell_of(text: &str, at: usize) -> Cell {
    let chars: Vec<char> = text.chars().collect();
    let at = at.min(chars.len());
    Cell {
        line: chars[..at].iter().filter(|c| **c == '\n').count(),
        column: at - line_start(&chars, at),
    }
}

/// Column select: one selection per line between the two cells, over the same columns; a line
/// too short for them gets a caret at its end.
pub fn column_select(text: &str, anchor: Cell, head: Cell) -> Selections {
    let chars: Vec<char> = text.chars().collect();
    let mut starts = vec![0];
    starts.extend(
        chars
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == '\n')
            .map(|(i, _)| i + 1),
    );
    let last = starts.len() - 1;
    let (top, bottom) = (
        anchor.line.min(head.line).min(last),
        anchor.line.max(head.line).min(last),
    );
    let at = |line: usize, column: usize| {
        let start = starts[line];
        (start + column).min(line_end(&chars, start))
    };
    let sels: Vec<Sel> = (top..=bottom)
        .map(|line| Sel::new(at(line, anchor.column), at(line, head.column)))
        .collect();
    let primary = if head.line >= anchor.line {
        sels.len() - 1
    } else {
        0
    };
    Selections { sels, primary }.normalized()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `|` marking carets and `[`…`]` selections (anchor first); returns the
    /// plain text and the selections.
    fn parse(marked: &str) -> (String, Selections) {
        let mut text = String::new();
        let mut sels = Vec::new();
        let mut open: Option<usize> = None;
        let mut count = 0;
        for c in marked.chars() {
            match c {
                '|' => sels.push(Sel::caret(count)),
                '[' => open = Some(count),
                ']' => sels.push(Sel::new(open.take().expect("open"), count)),
                _ => {
                    text.push(c);
                    count += 1;
                }
            }
        }
        (text, Selections { sels, primary: 0 })
    }

    fn show(text: &str, selections: &Selections) -> String {
        let mut marks: Vec<(usize, char)> = Vec::new();
        for sel in &selections.sels {
            if sel.is_empty() {
                marks.push((sel.head, '|'));
            } else {
                let range = sel.range();
                marks.push((range.start, '['));
                marks.push((range.end, ']'));
            }
        }
        marks.sort_by_key(|(at, mark)| (*at, *mark != ']'));
        let mut out = String::new();
        let mut next = 0;
        for (index, c) in text.chars().enumerate() {
            while next < marks.len() && marks[next].0 == index {
                out.push(marks[next].1);
                next += 1;
            }
            out.push(c);
        }
        for mark in &marks[next..] {
            out.push(mark.1);
        }
        out
    }

    fn edited(marked: &str, f: impl Fn(&str, &Selections) -> Option<Edited>) -> String {
        let (text, sels) = parse(marked);
        let result = f(&text, &sels).expect("an edit");
        show(&result.text, &result.selections)
    }

    #[test]
    fn typing_goes_to_every_caret() {
        assert_eq!(
            edited("a|\nb|\nc|", |t, s| type_text(t, s, "x")),
            "ax|\nbx|\ncx|"
        );
    }

    #[test]
    fn typing_replaces_every_selection() {
        assert_eq!(
            edited("[ab]c [ab]d", |t, s| type_text(t, s, "Z")),
            "Z|c Z|d"
        );
    }

    #[test]
    fn typing_a_bracket_pairs_at_each_caret_like_one_caret_does() {
        assert_eq!(edited("f|\ng|", |t, s| type_text(t, s, "(")), "f(|)\ng(|)");
    }

    #[test]
    fn typing_a_closer_overtypes_at_each_caret() {
        assert_eq!(
            edited("f(a|)\ng(b|)", |t, s| type_text(t, s, ")")),
            "f(a)|\ng(b)|"
        );
    }

    #[test]
    fn return_indents_each_caret() {
        assert_eq!(edited("if a|\nif b|", newline), "if a\n    |\nif b\n    |");
    }

    #[test]
    fn backspace_removes_a_character_or_a_selection_per_caret() {
        assert_eq!(edited("ab|\n[cd]e|", backspace), "a|\n|");
    }

    #[test]
    fn backspace_removes_an_empty_pair() {
        assert_eq!(edited("f(|)\ng(|)", backspace), "f|\ng|");
    }

    #[test]
    fn carets_that_meet_become_one() {
        assert_eq!(edited("a|b|c", backspace), "|c");
    }

    #[test]
    fn delete_removes_the_next_character() {
        assert_eq!(edited("|ab\n|cd", delete), "|b\n|d");
    }

    #[test]
    fn tab_indents_each_caret_to_the_next_stop() {
        assert_eq!(edited("|a\n  |b", indent), "    |a\n    |b");
    }

    #[test]
    fn copy_joins_the_selections_by_lines() {
        let (text, sels) = parse("[ab] cd [ef]");
        assert_eq!(copy_text(&text, &sels).as_deref(), Some("ab\nef"));
        let (text, sels) = parse("a|b");
        assert_eq!(copy_text(&text, &sels), None);
    }

    #[test]
    fn cut_removes_the_selections() {
        assert_eq!(edited("[ab]cd[ef]", cut), "|cd|");
    }

    #[test]
    fn paste_gives_each_caret_one_line_when_the_counts_match() {
        assert_eq!(edited("|\n|", |t, s| paste(t, s, "one\ntwo")), "one|\ntwo|");
    }

    #[test]
    fn paste_otherwise_repeats_the_whole_text() {
        assert_eq!(
            edited("|\n|", |t, s| paste(t, s, "x\ny\nz")),
            "x\ny\nz|\nx\ny\nz|"
        );
        assert_eq!(edited("|\n|", |t, s| paste(t, s, "q")), "q|\nq|");
    }

    #[test]
    fn horizontal_moves_apply_to_every_caret() {
        let (text, sels) = parse("a|bc\nd|ef");
        assert_eq!(
            show(&text, &moved(&text, &sels, Motion::Right, false)),
            "ab|c\nde|f"
        );
        assert_eq!(
            show(&text, &moved(&text, &sels, Motion::Left, false)),
            "|abc\n|def"
        );
    }

    #[test]
    fn a_selection_collapses_to_its_edge_on_left_and_right() {
        let (text, sels) = parse("a[bc]d");
        assert_eq!(
            show(&text, &moved(&text, &sels, Motion::Left, false)),
            "a|bcd"
        );
        assert_eq!(
            show(&text, &moved(&text, &sels, Motion::Right, false)),
            "abc|d"
        );
    }

    #[test]
    fn shift_extends_every_selection() {
        let (text, sels) = parse("a|bc\nd|ef");
        assert_eq!(
            show(&text, &moved(&text, &sels, Motion::Right, true)),
            "a[b]c\nd[e]f"
        );
    }

    #[test]
    fn vertical_moves_keep_the_column_and_clamp_to_short_lines() {
        let (text, sels) = parse("abcd|\nx\nabcd");
        let down = moved(&text, &sels, Motion::Down, false);
        assert_eq!(show(&text, &down), "abcd\nx|\nabcd");
        let (text, sels) = parse("abcd\nx\nab|cd");
        let up = moved(&text, &sels, Motion::Up, false);
        assert_eq!(show(&text, &up), "abcd\nx|\nabcd");
    }

    #[test]
    fn word_moves_skip_a_word_or_a_run_of_punctuation() {
        let (text, sels) = parse("|foo.bar baz");
        let one = moved(&text, &sels, Motion::WordRight, false);
        assert_eq!(show(&text, &one), "foo|.bar baz");
        let two = moved(&text, &one, Motion::WordRight, false);
        assert_eq!(show(&text, &two), "foo.|bar baz");
        let back = moved(&text, &two, Motion::WordLeft, false);
        assert_eq!(show(&text, &back), "foo|.bar baz");
    }

    #[test]
    fn home_goes_to_the_first_non_blank_then_the_line_start() {
        let (text, sels) = parse("    ab|c");
        let first = moved(&text, &sels, Motion::LineStart, false);
        assert_eq!(show(&text, &first), "    |abc");
        let second = moved(&text, &first, Motion::LineStart, false);
        assert_eq!(show(&text, &second), "|    abc");
        let end = moved(&text, &sels, Motion::LineEnd, false);
        assert_eq!(show(&text, &end), "    abc|");
    }

    #[test]
    fn select_next_expands_carets_to_words_then_adds_matches() {
        let (text, sels) = parse("foo f|oo bar foo");
        let first = select_next(&text, &sels).unwrap();
        assert_eq!(show(&text, &first), "foo [foo] bar foo");
        let second = select_next(&text, &first).unwrap();
        assert_eq!(show(&text, &second), "foo [foo] bar [foo]");
        assert_eq!(second.primary, 1);
        let third = select_next(&text, &second).unwrap();
        assert_eq!(show(&text, &third), "[foo] [foo] bar [foo]");
        assert!(select_next(&text, &third).is_none(), "all are taken");
    }

    #[test]
    fn a_whole_word_selection_does_not_match_inside_longer_words() {
        let (text, sels) = parse("[foo] foobar foo");
        let next = select_next(&text, &sels).unwrap();
        assert_eq!(show(&text, &next), "[foo] foobar [foo]");
    }

    #[test]
    fn a_partial_selection_matches_substrings() {
        let (text, sels) = parse("[fo]o foobar");
        let next = select_next(&text, &sels).unwrap();
        assert_eq!(show(&text, &next), "[fo]o [fo]obar");
    }

    #[test]
    fn select_all_matches_selects_every_one() {
        let (text, sels) = parse("a|b ab foobar ab");
        let all = select_all_matches(&text, &sels).unwrap();
        assert_eq!(show(&text, &all), "[ab] [ab] foobar [ab]");
        assert_eq!(all.sels.len(), 3);
    }

    #[test]
    fn carets_add_above_and_below_at_the_same_column() {
        let (text, sels) = parse("abc\nab|cd\nabc");
        let up = add_caret_vertically(&text, &sels, true).unwrap();
        assert_eq!(show(&text, &up), "ab|c\nab|cd\nabc");
        let down = add_caret_vertically(&text, &up, false).unwrap();
        assert_eq!(show(&text, &down), "ab|c\nab|cd\nab|c");
        assert!(
            add_caret_vertically(&text, &down, true).is_none(),
            "top reached"
        );
    }

    #[test]
    fn column_select_takes_the_same_columns_on_each_line() {
        let text = "abcdef\nab\nabcdef";
        let sels = column_select(
            text,
            Cell { line: 0, column: 1 },
            Cell { line: 2, column: 3 },
        );
        assert_eq!(show(text, &sels), "a[bc]def\na[b]\na[bc]def");
        assert_eq!(sels.primary, 2);
    }

    #[test]
    fn normalizing_merges_overlaps_and_keeps_the_primary() {
        let sels = Selections {
            sels: vec![Sel::new(5, 8), Sel::new(2, 6), Sel::caret(7)],
            primary: 0,
        }
        .normalized();
        assert_eq!(sels.sels, vec![Sel::new(2, 8)]);
        assert_eq!(sels.primary, 0);
    }

    #[test]
    fn edits_with_astral_characters_count_characters() {
        assert_eq!(
            edited("\u{1F600}|\n\u{1F600}|", |t, s| type_text(t, s, "x")),
            "\u{1F600}x|\n\u{1F600}x|"
        );
    }

    #[test]
    fn the_caret_count_is_capped() {
        let sels = Selections {
            sels: (0..MAX + 50).map(|i| Sel::caret(i * 2)).collect(),
            primary: 0,
        }
        .normalized();
        assert_eq!(sels.sels.len(), MAX);
    }
}
