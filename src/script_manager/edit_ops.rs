//! Pure text-editing assists for the Pine editor, ported from `PineEditorCommands`,
//! `PineEditorPairing` and `PineIndentationEngine`. Every function takes the text and selection
//! (in `char` indices, what egui's cursor counts) and returns one contiguous [`Edit`], or `None`
//! to let the text field handle the input normally.

use crate::pine::lexer::Lexer;
use crate::pine::token::TokenKind;
use std::ops::Range;

pub const UNIT: &str = "    ";
const WIDTH: usize = 4;
const MARKER: &str = "//";

/// Replace `range` with `replacement`, then select `selection` (indices into the *new* text).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub selection: Range<usize>,
}

impl Edit {
    fn caret(at: usize) -> Self {
        Self {
            range: at..at,
            replacement: String::new(),
            selection: at..at,
        }
    }

    fn insert(text: &str, at: usize, caret: usize) -> Self {
        Self {
            range: at..at,
            replacement: text.to_owned(),
            selection: caret..caret,
        }
    }

    /// Applies the edit, returning the new text.
    pub fn apply(&self, text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out: String = chars[..self.range.start].iter().collect();
        out.push_str(&self.replacement);
        out.extend(&chars[self.range.end..]);
        out
    }
}

#[derive(Clone, Debug)]
struct StringLiteral {
    range: Range<usize>,
    quote: char,
    terminated: bool,
}

/// The text, its selection and what the lexical scan found in it.
pub struct Context {
    chars: Vec<char>,
    sel: Range<usize>,
    comments: Vec<Range<usize>>,
    strings: Vec<StringLiteral>,
    /// `( ) [ ]` outside strings and comments, with the index of the matching delimiter.
    delimiters: Vec<(usize, char, Option<usize>)>,
}

impl Context {
    pub fn new(text: &str, selection: Range<usize>) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let sel = selection.start.min(chars.len())..selection.end.min(chars.len());
        let mut context = Self {
            chars,
            sel,
            comments: Vec::new(),
            strings: Vec::new(),
            delimiters: Vec::new(),
        };
        context.scan();
        context
    }

    fn scan(&mut self) {
        let mut i = 0;
        let n = self.chars.len();
        let mut stack: Vec<usize> = Vec::new();
        while i < n {
            let c = self.chars[i];
            if c == '/' && self.chars.get(i + 1) == Some(&'/') {
                let end = self.line_content_end(i);
                self.comments.push(i..end);
                i = end;
            } else if c == '"' || c == '\'' {
                let start = i;
                i += 1;
                let mut terminated = false;
                while i < n && self.chars[i] != '\n' {
                    if self.chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if self.chars[i] == c {
                        terminated = true;
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                let end = i.min(n);
                self.strings.push(StringLiteral {
                    range: start..end,
                    quote: c,
                    terminated,
                });
                i = end;
            } else {
                match c {
                    '(' | '[' => {
                        stack.push(self.delimiters.len());
                        self.delimiters.push((i, c, None));
                    }
                    ')' | ']' => {
                        let open = if c == ')' { '(' } else { '[' };
                        let index = self.delimiters.len();
                        let partner = stack
                            .last()
                            .copied()
                            .filter(|&p| self.delimiters[p].1 == open);
                        if let Some(p) = partner {
                            stack.pop();
                            self.delimiters[p].2 = Some(index);
                        }
                        self.delimiters.push((i, c, partner));
                    }
                    _ => {}
                }
                i += 1;
            }
        }
    }

    pub fn caret(&self) -> usize {
        self.sel.start
    }

    /// Points the context at another selection of the same text, so several carets share one
    /// scan.
    pub fn select(&mut self, selection: Range<usize>) {
        let end = self.chars.len();
        self.sel = selection.start.min(end)..selection.end.min(end);
    }

    fn unit(&self, at: usize) -> Option<char> {
        self.chars.get(at).copied()
    }

    fn unit_before(&self, at: usize) -> Option<char> {
        at.checked_sub(1).and_then(|i| self.unit(i))
    }

    fn line_start(&self, at: usize) -> usize {
        let at = at.min(self.chars.len());
        self.chars[..at]
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(0, |i| i + 1)
    }

    fn line_content_end(&self, at: usize) -> usize {
        self.chars[at.min(self.chars.len())..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(self.chars.len(), |i| at + i)
    }

    /// The line's range including its terminator.
    fn full_line(&self, at: usize) -> Range<usize> {
        let start = self.line_start(at);
        let end = (self.line_content_end(at) + 1).min(self.chars.len());
        start..end
    }

    fn leading_ws_len(&self, line_start: usize) -> usize {
        self.chars[line_start..]
            .iter()
            .take_while(|&&c| c == ' ' || c == '\t')
            .count()
    }

    fn leading_ws(&self, line_start: usize) -> String {
        self.chars[line_start..line_start + self.leading_ws_len(line_start)]
            .iter()
            .collect()
    }

    fn is_blank_line(&self, at: usize) -> bool {
        let start = self.line_start(at);
        let end = self.line_content_end(at);
        self.chars[start..end].iter().all(|c| c.is_whitespace())
    }

    /// Every line the selection touches (the caret's line when nothing is selected). A selection
    /// ending exactly at a line start does not take that last line.
    fn covered_lines(&self) -> Vec<Range<usize>> {
        let mut lines = Vec::new();
        let mut at = self.line_start(self.sel.start);
        let last = if self.sel.end > self.sel.start && self.line_start(self.sel.end) == self.sel.end
        {
            self.sel.end - 1
        } else {
            self.sel.end
        };
        loop {
            let line = self.full_line(at);
            lines.push(line.clone());
            if line.end > last || line.end >= self.chars.len() {
                break;
            }
            at = line.end;
        }
        lines
    }

    /// Whether `at` sits after the `//` of a comment on its line (typing there is plain text).
    fn inside_comment(&self, at: usize) -> bool {
        self.comments.iter().any(|r| r.start < at && at <= r.end)
    }

    /// Whether the character at `at` sits inside a string literal or a comment.
    fn in_text(&self, at: usize) -> bool {
        self.comments.iter().any(|r| r.start <= at && at < r.end)
            || self
                .strings
                .iter()
                .any(|s| s.range.start <= at && at < s.range.end)
    }

    fn enclosing_string(&self, at: usize) -> Option<&StringLiteral> {
        self.strings
            .iter()
            .find(|s| s.range.start < at && at < s.range.end + usize::from(!s.terminated))
    }

    /// The nearest opener before `at` that is still open there.
    fn enclosing_opener(&self, at: usize) -> Option<usize> {
        let mut stack: Vec<usize> = Vec::new();
        for (index, (position, c, _)) in self.delimiters.iter().enumerate() {
            if *position >= at {
                break;
            }
            match c {
                '(' | '[' => stack.push(index),
                _ => {
                    if let Some(&top) = stack.last()
                        && self.delimiters[top].2 == Some(index)
                    {
                        stack.pop();
                    }
                }
            }
        }
        stack.last().copied()
    }

    fn delimiter_at(&self, at: usize) -> Option<usize> {
        self.delimiters.iter().position(|d| d.0 == at)
    }
}

// MARK: Line edit sets

/// Insertions and removals at several places, folded into one contiguous edit.
#[derive(Default)]
struct LineEdits {
    ops: Vec<(usize, usize, String)>,
}

impl LineEdits {
    fn insert(&mut self, at: usize, text: &str) {
        self.ops.push((at, 0, text.to_owned()));
    }

    fn remove(&mut self, at: usize, len: usize) {
        self.ops.push((at, len, String::new()));
    }

    fn make(mut self, context: &Context) -> Option<Edit> {
        if self.ops.is_empty() {
            return None;
        }
        self.ops.sort_by_key(|op| op.0);
        let start = self.ops.first()?.0;
        let end = self.ops.iter().map(|op| op.0 + op.1).max()?;
        let mut replacement = String::new();
        let mut cursor = start;
        for (at, remove, insert) in &self.ops {
            replacement.extend(&context.chars[cursor..*at]);
            replacement.push_str(insert);
            cursor = at + remove;
        }
        replacement.extend(&context.chars[cursor..end]);
        let map = |position: usize| -> usize {
            let mut delta: isize = 0;
            for (at, remove, insert) in &self.ops {
                if *at < position || (*at == position && !insert.is_empty()) {
                    let removed_before = (*remove).min(position.saturating_sub(*at));
                    delta += insert.chars().count() as isize - removed_before as isize;
                }
            }
            (position as isize + delta).max(0) as usize
        };
        // A caret in the middle of the inserted marker lands after it, like Swift's edit set.
        let selection = map(context.sel.start)..map(context.sel.end);
        Some(Edit {
            range: start..end,
            replacement,
            selection,
        })
    }
}

// MARK: Commands

/// Cmd/Ctrl+/ — comments every selected line at the shallowest indentation among them, or, when they are
/// all commented already, removes the `//` (and the space after it).
pub fn toggle_comment(context: &Context) -> Option<Edit> {
    let lines = context.covered_lines();
    let mut candidates: Vec<&Range<usize>> = lines
        .iter()
        .filter(|l| !context.is_blank_line(l.start))
        .collect();
    if candidates.is_empty() && lines.len() == 1 {
        candidates = lines.iter().collect();
    }
    if candidates.is_empty() {
        return None;
    }
    let is_commented = |line: &Range<usize>| {
        let start = line.start + context.leading_ws_len(line.start);
        context.chars.get(start) == Some(&'/') && context.chars.get(start + 1) == Some(&'/')
    };
    let mut edits = LineEdits::default();
    if candidates.iter().all(|l| is_commented(l)) {
        for line in candidates {
            let start = line.start + context.leading_ws_len(line.start);
            let space = context.unit(start + 2) == Some(' ');
            edits.remove(start, if space { 3 } else { 2 });
        }
    } else {
        let column = candidates
            .iter()
            .map(|l| context.leading_ws_len(l.start))
            .min()
            .unwrap_or(0);
        for line in candidates {
            edits.insert(line.start + column, &format!("{MARKER} "));
        }
    }
    edits.make(context)
}

fn line_block(context: &Context) -> Range<usize> {
    let lines = context.covered_lines();
    lines[0].start..lines[lines.len() - 1].end
}

fn ends_with_break(context: &Context, block: &Range<usize>) -> bool {
    block.end > 0 && context.unit(block.end - 1) == Some('\n')
}

fn text_of(context: &Context, range: &Range<usize>) -> String {
    context.chars[range.clone()].iter().collect()
}

/// Shift+Alt/Option+↓ / ↑ — copies the selected lines below or above themselves. The selection follows the
/// copy going down and stays put (now on the upper copy) going up.
pub fn duplicate(context: &Context, down: bool) -> Option<Edit> {
    let block = line_block(context);
    let text = text_of(context, &block);
    let breaks = ends_with_break(context, &block);
    if down {
        let inserted = if breaks { text } else { format!("\n{text}") };
        let shift = inserted.chars().count();
        return Some(Edit {
            range: block.end..block.end,
            replacement: inserted,
            selection: context.sel.start + shift..context.sel.end + shift,
        });
    }
    Some(Edit {
        range: block.start..block.start,
        replacement: if breaks { text } else { format!("{text}\n") },
        selection: context.sel.clone(),
    })
}

/// Alt/Option+↑ / ↓ — swaps the selected lines with the line above or below.
pub fn move_lines(context: &Context, down: bool) -> Option<Edit> {
    let block = line_block(context);
    let mut block_text = text_of(context, &block);
    let strip = |t: String| t.strip_suffix('\n').map(str::to_owned).unwrap_or(t);
    if down {
        if block.end >= context.chars.len() {
            return None;
        }
        let neighbour = context.full_line(block.end);
        let mut neighbour_text = text_of(context, &neighbour);
        if !neighbour_text.ends_with('\n') {
            neighbour_text.push('\n');
            block_text = strip(block_text);
        }
        let shift = neighbour_text.chars().count();
        return Some(Edit {
            range: block.start..neighbour.end,
            replacement: neighbour_text + &block_text,
            selection: context.sel.start + shift..context.sel.end + shift,
        });
    }
    if block.start == 0 {
        return None;
    }
    let neighbour = context.full_line(block.start - 1);
    let mut neighbour_text = text_of(context, &neighbour);
    if !block_text.ends_with('\n') {
        block_text.push('\n');
        neighbour_text = strip(neighbour_text);
    }
    let shift = neighbour.end - neighbour.start;
    Some(Edit {
        range: neighbour.start..block.end,
        replacement: block_text + &neighbour_text,
        selection: context.sel.start - shift..context.sel.end - shift,
    })
}

// MARK: Indentation

fn kinds_of(line: &str) -> Vec<TokenKind> {
    Lexer::new(line)
        .lex()
        .tokens
        .into_iter()
        .map(|t| t.kind)
        .filter(|k| {
            !matches!(
                k,
                TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
            )
        })
        .collect()
}

/// Whether a line made of these tokens expects an indented body: `if`, `else`, `for`, `while`,
/// `switch`, a trailing `=>`, a `type`/`enum` header, or `x = if …` / `x = switch …`.
pub fn opens_block(kinds: &[TokenKind]) -> bool {
    let mut kinds = kinds;
    if let Some(TokenKind::Identifier(word)) = kinds.first()
        && word == "export"
        && kinds.len() > 1
    {
        kinds = &kinds[1..];
    }
    let Some(first) = kinds.first() else {
        return false;
    };
    if matches!(
        first,
        TokenKind::If | TokenKind::Else | TokenKind::For | TokenKind::While | TokenKind::Switch
    ) {
        return true;
    }
    if kinds.last() == Some(&TokenKind::Arrow) {
        return true;
    }
    if let TokenKind::Identifier(word) = first
        && (word == "type" || word == "enum")
        && kinds.len() == 2
        && matches!(kinds[1], TokenKind::Identifier(_))
    {
        return true;
    }
    if let Some(assign) = kinds
        .iter()
        .position(|k| matches!(k, TokenKind::Assign | TokenKind::Reassign))
        && let Some(next) = kinds.get(assign + 1)
    {
        return matches!(next, TokenKind::If | TokenKind::Switch);
    }
    false
}

/// Whether a line ending in this token continues onto the next one (the lexer's rule).
pub fn continues(kind: Option<&TokenKind>) -> bool {
    matches!(
        kind,
        Some(
            TokenKind::Assign
                | TokenKind::Reassign
                | TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::Power
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Comma
                | TokenKind::Question
                | TokenKind::Colon
        )
    )
}

/// Return: replaces the selection with a line break and the indentation the next line needs.
pub fn newline(context: &Context) -> Edit {
    let caret = context.caret();
    let line_start = context.line_start(caret);
    let leading_end = line_start + context.leading_ws_len(line_start);
    let mut replace_start = caret;
    let mut replace_end = context.sel.end;
    let in_indentation = caret <= leading_end;
    if in_indentation {
        replace_start = line_start;
        replace_end = replace_end.max(leading_end);
    }
    let base: String = context.chars[line_start..caret.min(leading_end)]
        .iter()
        .collect();

    let line_break = |indent: &str| Edit {
        range: replace_start..replace_end,
        replacement: format!("\n{indent}"),
        selection: {
            let at = replace_start + 1 + indent.chars().count();
            at..at
        },
    };

    if let Some(opener) = context.enclosing_opener(caret) {
        let (opener_pos, _, partner) = context.delimiters[opener];
        let opener_line = context.line_start(opener_pos);
        let opener_indent = context.leading_ws(opener_line);
        let child = if opener_line == line_start {
            format!("{opener_indent}{UNIT}")
        } else {
            base
        };
        // `(|)`: open the pair onto its own lines, the closer back at the opener's indent.
        if caret > 0
            && caret - 1 == opener_pos
            && partner.map(|p| context.delimiters[p].0) == Some(context.sel.end)
        {
            let at = replace_start + 1 + child.chars().count();
            return Edit {
                range: replace_start..replace_end,
                replacement: format!("\n{child}\n{opener_indent}"),
                selection: at..at,
            };
        }
        return line_break(&child);
    }

    let line_end = context.line_content_end(caret);
    let before: String = context.chars[line_start..caret].iter().collect();
    let after: String = context.chars[context.sel.end.min(line_end)..line_end]
        .iter()
        .collect();
    let before_kinds = kinds_of(&before);
    let mut indent = base;
    if after.trim().is_empty() && (opens_block(&before_kinds) || continues(before_kinds.last())) {
        indent.push_str(UNIT);
    }
    line_break(&indent)
}

/// Tab: one indentation step at the caret (spaces to the next multiple of four), or every
/// selected line indented when the selection spans lines.
pub fn indent(context: &Context) -> Edit {
    let lines = context.covered_lines();
    if context.sel.is_empty() || lines.len() <= 1 {
        let column = context.caret() - context.line_start(context.caret());
        let spaces = " ".repeat(WIDTH - column % WIDTH);
        let at = context.sel.start + spaces.len();
        return Edit {
            range: context.sel.clone(),
            replacement: spaces,
            selection: at..at,
        };
    }
    let mut edits = LineEdits::default();
    for line in &lines {
        if context.line_content_end(line.start) > line.start {
            edits.insert(line.start, UNIT);
        }
    }
    edits
        .make(context)
        .unwrap_or_else(|| Edit::caret(context.caret()))
}

/// Shift-Tab: removes up to one indentation step from every covered line.
pub fn outdent(context: &Context) -> Option<Edit> {
    let mut edits = LineEdits::default();
    for line in context.covered_lines() {
        let removable = if context.unit(line.start) == Some('\t') {
            1
        } else {
            (0..WIDTH)
                .take_while(|i| context.unit(line.start + i) == Some(' '))
                .count()
        };
        if removable > 0 {
            edits.remove(line.start, removable);
        }
    }
    edits.make(context)
}

/// Typing `)` or `]` on an otherwise empty line puts it at the indentation of the line holding
/// its opener, so a multiline call closes flush with where it began.
fn align_closing(context: &Context, typed: char) -> Option<Edit> {
    if !context.sel.is_empty() {
        return None;
    }
    let caret = context.caret();
    let opener = context.enclosing_opener(caret)?;
    let (opener_pos, opener_char, _) = context.delimiters[opener];
    if opener_char != if typed == ')' { '(' } else { '[' } {
        return None;
    }
    let line_start = context.line_start(caret);
    let leading_end = line_start + context.leading_ws_len(line_start);
    if caret > leading_end
        || line_start == 0
        || context.line_start(opener_pos) == line_start
        || !context.is_blank_line(caret)
    {
        return None;
    }
    let indent = context.leading_ws(context.line_start(opener_pos));
    let line = line_start..context.line_content_end(caret);
    let at = line_start + indent.chars().count() + 1;
    Some(Edit {
        range: line,
        replacement: format!("{indent}{typed}"),
        selection: at..at,
    })
}

// MARK: Pairing

fn is_word_unit(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn can_open_pair(next: Option<char>) -> bool {
    match next {
        None => true,
        Some(c) => c.is_whitespace() || matches!(c, ')' | ']' | ',' | ';' | ':' | '.'),
    }
}

/// The edit for typing `text`, or `None` to let the text field insert it normally.
pub fn typed(context: &Context, text: &str) -> Option<Edit> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    match c {
        '(' | '[' | '"' | '\'' => opening(context, c),
        ')' | ']' => closing(context, c),
        _ => None,
    }
}

fn opening(context: &Context, c: char) -> Option<Edit> {
    let is_quote = c == '"' || c == '\'';
    let closer = match c {
        '(' => ')',
        '[' => ']',
        _ => c,
    };
    let sel = &context.sel;
    if !sel.is_empty() {
        if context.inside_comment(sel.start + 1) {
            return None;
        }
        if is_quote
            && let Some(literal) = context.enclosing_string(sel.start)
            && literal.quote == c
        {
            return None;
        }
        let inner: String = context.chars[sel.clone()].iter().collect();
        return Some(Edit {
            range: sel.clone(),
            replacement: format!("{c}{inner}{closer}"),
            selection: sel.start + 1..sel.start + 1 + (sel.end - sel.start),
        });
    }
    let caret = sel.start;
    if context.inside_comment(caret) {
        return None;
    }
    if let Some(literal) = context.enclosing_string(caret) {
        if !is_quote || literal.quote != c {
            return None;
        }
        if literal.terminated && context.unit(caret) == Some(c) && caret == literal.range.end - 1 {
            return Some(Edit::caret(caret + 1));
        }
        if literal.terminated && context.unit_before(caret) != Some('\\') {
            return Some(Edit::insert(&format!("\\{c}"), caret, caret + 2));
        }
        return None;
    }
    if !can_open_pair(context.unit(caret)) {
        return None;
    }
    if is_quote {
        let previous = context.unit_before(caret);
        if is_word_unit(previous) || previous == Some(c) || previous == Some('\\') {
            return None;
        }
    }
    Some(Edit::insert(&format!("{c}{closer}"), caret, caret + 1))
}

fn closing(context: &Context, c: char) -> Option<Edit> {
    if !context.sel.is_empty() {
        return None;
    }
    let caret = context.caret();
    if context.unit(caret) == Some(c)
        && let Some(index) = context.delimiter_at(caret)
        && context.delimiters[index].2.is_some()
    {
        return Some(Edit::caret(caret + 1));
    }
    if context.enclosing_string(caret).is_some() || context.inside_comment(caret) {
        return None;
    }
    align_closing(context, c)
}

/// Backspace between an empty pair removes both halves; anywhere else `None`.
pub fn backspace(context: &Context) -> Option<Edit> {
    if !context.sel.is_empty() || context.caret() == 0 {
        return None;
    }
    let caret = context.caret();
    let previous = context.unit_before(caret)?;
    let next = context.unit(caret)?;
    let removal = Edit {
        range: caret - 1..caret + 1,
        replacement: String::new(),
        selection: caret - 1..caret - 1,
    };
    let closes = |open: char| match open {
        '(' => Some(')'),
        '[' => Some(']'),
        _ => None,
    };
    if closes(previous) == Some(next) {
        let open = context.delimiter_at(caret - 1)?;
        let close = context.delimiter_at(caret)?;
        return (context.delimiters[open].2 == Some(close)).then_some(removal);
    }
    if (previous == '"' || previous == '\'')
        && previous == next
        && context
            .strings
            .iter()
            .any(|s| s.range == (caret - 1..caret + 1))
    {
        return Some(removal);
    }
    None
}

/// The call the caret sits inside: its (dotted) name, which argument the caret is in, and that
/// argument's keyword when it has one (`title = |`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallAt {
    pub name: String,
    pub argument: usize,
    pub keyword: Option<String>,
}

pub fn enclosing_call(text: &str, caret: usize) -> Option<CallAt> {
    let context = Context::new(text, caret..caret);
    let opener = context.enclosing_opener(caret)?;
    let (open, kind, _) = context.delimiters[opener];
    if kind != '(' {
        return None;
    }
    let is_name = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    let mut start = open;
    while start > 0 && is_name(context.chars[start - 1]) {
        start -= 1;
    }
    let name: String = context.chars[start..open].iter().collect();
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let (mut depth, mut argument, mut segment_start) = (0usize, 0usize, open + 1);
    for i in open + 1..caret.min(context.chars.len()) {
        if context.in_text(i) {
            continue;
        }
        match context.chars[i] {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                argument += 1;
                segment_start = i + 1;
            }
            _ => {}
        }
    }
    let segment: String = context.chars[segment_start..caret.min(context.chars.len())]
        .iter()
        .collect();
    let keyword = segment.split_once('=').and_then(|(left, right)| {
        let left = left.trim();
        let valid = !left.is_empty() && left.chars().all(|c| c.is_alphanumeric() || c == '_');
        (valid && !right.starts_with('=')).then(|| left.to_owned())
    });
    Some(CallAt {
        name,
        argument,
        keyword,
    })
}

/// Whether `at` is inside a string literal or a comment (completion stays quiet there).
pub fn in_text(text: &str, at: usize) -> bool {
    let context = Context::new(text, at..at);
    context.in_text(at.saturating_sub(1)) && at > 0 || context.enclosing_string(at).is_some()
}

/// How far either side of the caret occurrences are searched, like the Swift editor's visible
/// range margin.
const OCCURRENCE_WINDOW: usize = 2000;

/// The identifier under (or just before) the caret and every other whole-token use of it, outside
/// strings and comments. A member (`ta.len`) and a bare name (`len`) are different things, so only
/// uses of the same kind count. The first range is the one under the caret.
pub fn occurrences(text: &str, caret: usize) -> Vec<Range<usize>> {
    let context = Context::new(text, caret..caret);
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let chars = &context.chars;
    let at = caret.min(chars.len());
    let on_word = at < chars.len() && is_word(chars[at]);
    let after_word = at > 0 && is_word(chars[at - 1]);
    if !on_word && !after_word {
        return Vec::new();
    }
    let mut start = if on_word { at } else { at - 1 };
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = start;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    if chars[start].is_ascii_digit() || context.in_text(start) {
        return Vec::new();
    }
    let word: Vec<char> = chars[start..end].to_vec();
    let is_member = |from: usize| from > 0 && chars[from - 1] == '.';
    let kind = is_member(start);
    let (from, to) = (
        start.saturating_sub(OCCURRENCE_WINDOW),
        (end + OCCURRENCE_WINDOW).min(chars.len()),
    );
    let mut found: Vec<Range<usize>> = Vec::new();
    found.push(start..end);
    let mut i = from;
    while i + word.len() <= to {
        if i != start
            && chars[i..i + word.len()] == word[..]
            && (i == 0 || !is_word(chars[i - 1]))
            && chars.get(i + word.len()).is_none_or(|c| !is_word(*c))
            && is_member(i) == kind
            && !context.in_text(i)
        {
            found.push(i..i + word.len());
            i += word.len();
        } else {
            i += 1;
        }
    }
    found
}

/// The delimiter at the caret (the one just before it, else the one just after) and its partner;
/// `None` for the partner when it is unmatched.
pub fn bracket_match(text: &str, caret: usize) -> Option<(usize, Option<usize>)> {
    let context = Context::new(text, caret..caret);
    let candidates = [caret.checked_sub(1), Some(caret)];
    candidates.into_iter().flatten().find_map(|position| {
        let index = context.delimiter_at(position)?;
        let partner = context.delimiters[index].2.map(|p| context.delimiters[p].0);
        // A closer records its opener as its partner too; either way it is the matching position.
        let partner = partner.or_else(|| {
            context
                .delimiters
                .iter()
                .find(|d| d.2 == Some(index))
                .map(|d| d.0)
        });
        Some((position, partner))
    })
}

/// The word under `at`, splitting dotted names (`table` in `table.cell`).
pub fn word_range(text: &str, at: usize) -> Option<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies `edit` produced from `text` with the selection `sel`.
    fn run(
        text: &str,
        sel: Range<usize>,
        f: impl Fn(&Context) -> Option<Edit>,
    ) -> (String, Range<usize>) {
        let context = Context::new(text, sel);
        let edit = f(&context).expect("an edit");
        (edit.apply(text), edit.selection)
    }

    #[test]
    fn comment_toggle_round_trips_at_the_shallowest_indent() {
        let text = "a = 1\n    b = 2\n";
        let (commented, _) = run(text, 0..text.chars().count(), toggle_comment);
        assert_eq!(
            commented, "// a = 1\n//     b = 2\n",
            "the marker goes at the shallowest indent of the whole block"
        );
        let (back, _) = run(&commented, 0..commented.chars().count(), |c| {
            toggle_comment(c)
        });
        assert_eq!(back, text);
        // The caret's own line only; a blank line inside a block is skipped.
        let (one, caret) = run("x\n\ny\n", 0..0, toggle_comment);
        assert_eq!(one, "// x\n\ny\n");
        assert_eq!(caret, 3..3);
        let blank_block = "a\n\nb\n";
        let (both, _) = run(blank_block, 0..4, toggle_comment);
        assert_eq!(both, "// a\n\n// b\n");
    }

    #[test]
    fn move_and_duplicate_lines() {
        let (down, sel) = run("a\nb\nc\n", 0..0, |c| move_lines(c, true));
        assert_eq!(down, "b\na\nc\n");
        assert_eq!(sel, 2..2);
        let (up, _) = run("a\nb\nc\n", 2..2, |c| move_lines(c, false));
        assert_eq!(up, "b\na\nc\n");
        assert!(move_lines(&Context::new("a\nb", 0..0), false).is_none());
        // Moving the unterminated last line up hands its end to the neighbour.
        let (last_up, _) = run("a\nb", 2..2, |c| move_lines(c, false));
        assert_eq!(last_up, "b\na");
        let (dup_down, sel) = run("a\nb\n", 0..0, |c| duplicate(c, true));
        assert_eq!(dup_down, "a\na\nb\n");
        assert_eq!(sel, 2..2, "selection follows the lower copy");
        let (dup_up, sel) = run("a\nb\n", 2..2, |c| duplicate(c, false));
        assert_eq!(dup_up, "a\nb\nb\n");
        assert_eq!(sel, 2..2, "selection stays on the upper copy");
        let (dup_last, _) = run("a", 0..0, |c| duplicate(c, true));
        assert_eq!(dup_last, "a\na");
    }

    #[test]
    fn return_keeps_indent_and_opens_blocks() {
        let at_end = |text: &str| {
            let n = text.chars().count();
            newline(&Context::new(text, n..n)).apply(text)
        };
        assert_eq!(at_end("x = 1"), "x = 1\n");
        assert_eq!(at_end("    x = 1"), "    x = 1\n    ");
        assert_eq!(at_end("if close > open"), "if close > open\n    ");
        assert_eq!(at_end("f(x) =>"), "f(x) =>\n    ");
        assert_eq!(at_end("y = if a"), "y = if a\n    ");
        assert_eq!(at_end("a = 1 +"), "a = 1 +\n    ");
        assert_eq!(at_end("for i = 0 to 3"), "for i = 0 to 3\n    ");
        // Inside an open call the new line aligns one level in from the opener's line.
        assert_eq!(at_end("plot(close,"), "plot(close,\n    ");
        // `(|)` opens onto its own lines.
        let text = "plot()";
        let edit = newline(&Context::new(text, 5..5));
        assert_eq!(edit.apply(text), "plot(\n    \n)");
        assert_eq!(edit.selection, 10..10);
    }

    #[test]
    fn tab_and_shift_tab() {
        let (indented, sel) = run("ab", 1..1, |c| Some(indent(c)));
        assert_eq!(indented, "a   b");
        assert_eq!(sel, 4..4);
        let (block, _) = run("a\n\nb\n", 0..5, |c| Some(indent(c)));
        assert_eq!(block, "    a\n\n    b\n");
        let (out, _) = run("    a\n  b\n", 0..9, outdent);
        assert_eq!(out, "a\nb\n");
        assert!(outdent(&Context::new("a", 0..0)).is_none());
    }

    #[test]
    fn typing_pairs_wraps_and_overtypes() {
        let t = |text: &str, sel: Range<usize>, typed_text: &str| {
            typed(&Context::new(text, sel), typed_text).map(|e| (e.apply(text), e.selection))
        };
        assert_eq!(t("", 0..0, "("), Some(("()".into(), 1..1)));
        assert_eq!(t("", 0..0, "\""), Some(("\"\"".into(), 1..1)));
        assert_eq!(
            t("ab", 0..2, "("),
            Some(("(ab)".into(), 1..3)),
            "wrap keeps selection"
        );
        // Not before a word: it would swallow it.
        assert_eq!(t("x", 0..0, "("), None);
        // Apostrophes after a word are plain text; braces are never paired.
        assert_eq!(t("don", 3..3, "'"), None);
        assert_eq!(t("", 0..0, "{"), None);
        // Overtype a real closer, not a stray one.
        assert_eq!(t("()", 1..1, ")"), Some(("()".into(), 2..2)));
        assert_eq!(t(")", 0..0, ")"), None);
        // Comments are plain text; inside a string only its own quote is special.
        assert_eq!(t("// a ", 5..5, "("), None);
        assert_eq!(t("\"a", 2..2, "("), None);
        assert_eq!(
            t("\"\"", 1..1, "\""),
            Some(("\"\"".into(), 2..2)),
            "step over the closing quote"
        );
        assert_eq!(t("\"a\"", 2..2, "\""), Some(("\"a\"".into(), 3..3)));
        // A closer typed alone on a continuation line aligns with the opener's line.
        let text = "    f(\n        ";
        let n = text.chars().count();
        let e = typed(&Context::new(text, n..n), ")").unwrap();
        assert_eq!(e.apply(text), "    f(\n    )");
    }

    #[test]
    fn backspace_removes_empty_pairs_only() {
        let b = |text: &str, at: usize| {
            backspace(&Context::new(text, at..at)).map(|e| (e.apply(text), e.selection))
        };
        assert_eq!(b("()", 1), Some((String::new(), 0..0)));
        assert_eq!(b("[]", 1), Some((String::new(), 0..0)));
        assert_eq!(b("\"\"", 1), Some((String::new(), 0..0)));
        assert_eq!(b("(a)", 1), None);
        assert_eq!(b("\"()\"", 2), None, "delimiters inside a string are text");
        assert_eq!(b("", 0), None);
    }

    #[test]
    fn dotted_names_split_on_double_click() {
        assert_eq!(word_range("table.cell(", 2), Some(0..5));
        assert_eq!(word_range("table.cell(", 8), Some(6..10));
        assert_eq!(
            word_range("a + b", 1),
            Some(0..1),
            "just past a word still picks it"
        );
        assert_eq!(word_range("a  b", 2), None);
    }

    #[test]
    fn occurrences_are_whole_tokens_of_the_same_kind() {
        let text = "len = 1\nx = ta.sma(close, len) + ta.len\n// len\ns = \"len\"\nlength = len2\n";
        let ranges = occurrences(text, 1);
        let words: Vec<String> = ranges
            .iter()
            .map(|r| text.chars().skip(r.start).take(r.len()).collect())
            .collect();
        assert_eq!(
            words,
            ["len", "len"],
            "bare uses only: not `ta.len`, comments, strings, `length`"
        );
        assert_eq!(ranges[0], 0..3, "the token under the caret comes first");
        // A member is its own kind.
        let member_at = text.find("ta.len").unwrap() + 4;
        assert_eq!(occurrences(text, member_at).len(), 1);
        assert!(
            occurrences("a + b", 2).is_empty(),
            "no word under the caret"
        );
        assert!(
            occurrences("12 + 12", 0).is_empty(),
            "numbers are not identifiers"
        );
    }

    #[test]
    fn brackets_match_before_or_after_the_caret() {
        let text = "f(a[1], (2))";
        assert_eq!(
            bracket_match(text, 2),
            Some((1, Some(11))),
            "after the opener"
        );
        assert_eq!(
            bracket_match(text, 12),
            Some((11, Some(1))),
            "after the closer"
        );
        assert_eq!(
            bracket_match(text, 1),
            Some((1, Some(11))),
            "before the opener"
        );
        assert_eq!(bracket_match("f(1", 2), Some((1, None)), "unmatched");
        assert_eq!(
            bracket_match("f(\"(\")", 4),
            None,
            "a paren in a string is text"
        );
        assert_eq!(bracket_match("abc", 1), None);
    }

    #[test]
    fn the_enclosing_call_knows_its_name_argument_and_keyword() {
        let at = |text: &str| enclosing_call(text, text.chars().count());
        assert_eq!(
            at("plot(close, "),
            Some(CallAt {
                name: "plot".into(),
                argument: 1,
                keyword: None
            })
        );
        assert_eq!(
            at("x = ta.sma(close"),
            Some(CallAt {
                name: "ta.sma".into(),
                argument: 0,
                keyword: None
            })
        );
        assert_eq!(
            at("plot(close, title = "),
            Some(CallAt {
                name: "plot".into(),
                argument: 1,
                keyword: Some("title".into())
            })
        );
        // Nested calls and brackets: the commas of inner calls don't count.
        assert_eq!(at("plot(f(a, b), x[1, 2], ").map(|c| c.argument), Some(2));
        assert_eq!(
            at("plot(f(a, ").map(|c| (c.name, c.argument)),
            Some(("f".into(), 1))
        );
        // A closed call, a bare group and text are not calls.
        assert_eq!(at("plot(close) "), None);
        assert_eq!(at("(1 + "), None);
        assert_eq!(
            at("plot(\"a, b"),
            Some(CallAt {
                name: "plot".into(),
                argument: 0,
                keyword: None
            })
        );
        assert_eq!(at("x == "), None);
    }
}
