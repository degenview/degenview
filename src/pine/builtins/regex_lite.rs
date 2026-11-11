//! A small backtracking regular-expression matcher for `str.match`, standing in for
//! `NSRegularExpression` (the crate set has no regex engine). It reads the common subset Pine
//! scripts use: literals, `.`, classes `[a-z0-9_]` and `[^…]`, the escapes `\d \D \w \W \s \S \b \B`
//! and escaped punctuation, anchors `^ $`, groups `( )` and `(?: )`, alternation `|`, and the
//! quantifiers `* + ? {n} {n,} {n,m}` with an optional lazy `?`. An expression outside that subset
//! (lookahead, back-references, named groups…) does not compile, and `str.match` then yields "".
//!
//! Matching is leftmost, tries alternatives in order, and gives up after a fixed number of steps,
//! so a pathological pattern cannot hang a script.

use std::cell::Cell;

const STEP_LIMIT: usize = 200_000;
/// Repetition counts above this are refused at compile time.
const REPEAT_LIMIT: usize = 1_000;

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Char(char),
    Any,
    /// `[...]`: ranges and single characters, possibly negated.
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Start,
    End,
    WordBoundary(bool),
    Group(Vec<Vec<Node>>),
    Repeat {
        node: Box<Node>,
        min: usize,
        max: usize,
        greedy: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ClassItem {
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

pub struct Regex {
    alternatives: Vec<Vec<Node>>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl ClassItem {
    fn matches(self, c: char) -> bool {
        match self {
            Self::Range(low, high) => (low..=high).contains(&c),
            Self::Digit(want) => c.is_ascii_digit() == want,
            Self::Word(want) => is_word(c) == want,
            Self::Space(want) => c.is_whitespace() == want,
        }
    }
}

// MARK: Parsing

struct Parser {
    chars: Vec<char>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.position += 1;
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn alternatives(&mut self) -> Option<Vec<Vec<Node>>> {
        let mut alternatives = vec![self.sequence()?];
        while self.eat('|') {
            alternatives.push(self.sequence()?);
        }
        Some(alternatives)
    }

    fn sequence(&mut self) -> Option<Vec<Node>> {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            nodes.push(self.quantified(atom)?);
        }
        Some(nodes)
    }

    fn atom(&mut self) -> Option<Node> {
        Some(match self.bump()? {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '(' => {
                if self.eat('?') && !self.eat(':') {
                    return None; // lookahead, named groups…
                }
                let inner = self.alternatives()?;
                if !self.eat(')') {
                    return None;
                }
                Node::Group(inner)
            }
            '[' => self.class()?,
            '\\' => self.escape()?,
            '*' | '+' | '?' | '{' => return None, // nothing to repeat
            c => Node::Char(c),
        })
    }

    fn escape(&mut self) -> Option<Node> {
        Some(match self.bump()? {
            'd' => Node::Class {
                negated: false,
                items: vec![ClassItem::Digit(true)],
            },
            'D' => Node::Class {
                negated: false,
                items: vec![ClassItem::Digit(false)],
            },
            'w' => Node::Class {
                negated: false,
                items: vec![ClassItem::Word(true)],
            },
            'W' => Node::Class {
                negated: false,
                items: vec![ClassItem::Word(false)],
            },
            's' => Node::Class {
                negated: false,
                items: vec![ClassItem::Space(true)],
            },
            'S' => Node::Class {
                negated: false,
                items: vec![ClassItem::Space(false)],
            },
            'b' => Node::WordBoundary(true),
            'B' => Node::WordBoundary(false),
            'n' => Node::Char('\n'),
            't' => Node::Char('\t'),
            'r' => Node::Char('\r'),
            c if c.is_ascii_digit() => return None, // back-reference
            c if c.is_alphabetic() => return None,  // unsupported class such as \p
            c => Node::Char(c),
        })
    }

    fn class(&mut self) -> Option<Node> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.bump()?;
            if c == ']' && !first {
                break;
            }
            first = false;
            let low = if c == '\\' {
                match self.bump()? {
                    'd' => {
                        items.push(ClassItem::Digit(true));
                        continue;
                    }
                    'D' => {
                        items.push(ClassItem::Digit(false));
                        continue;
                    }
                    'w' => {
                        items.push(ClassItem::Word(true));
                        continue;
                    }
                    'W' => {
                        items.push(ClassItem::Word(false));
                        continue;
                    }
                    's' => {
                        items.push(ClassItem::Space(true));
                        continue;
                    }
                    'S' => {
                        items.push(ClassItem::Space(false));
                        continue;
                    }
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other,
                }
            } else {
                c
            };
            if self.peek() == Some('-')
                && self.chars.get(self.position + 1).is_some_and(|n| *n != ']')
            {
                self.position += 1;
                let mut high = self.bump()?;
                if high == '\\' {
                    high = self.bump()?;
                }
                if high < low {
                    return None;
                }
                items.push(ClassItem::Range(low, high));
            } else {
                items.push(ClassItem::Range(low, low));
            }
        }
        Some(Node::Class { negated, items })
    }

    fn quantified(&mut self, atom: Node) -> Option<Node> {
        let (min, max) = match self.peek() {
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('?') => (0, 1),
            Some('{') => {
                let close =
                    self.chars[self.position..].iter().position(|c| *c == '}')? + self.position;
                let body: String = self.chars[self.position + 1..close].iter().collect();
                let bounds = match body.split_once(',') {
                    None => {
                        let n = body.trim().parse().ok()?;
                        (n, n)
                    }
                    Some((low, high)) => {
                        let low = low.trim().parse().ok()?;
                        let high = if high.trim().is_empty() {
                            usize::MAX
                        } else {
                            high.trim().parse().ok()?
                        };
                        (low, high)
                    }
                };
                if bounds.0 > bounds.1
                    || bounds.0 > REPEAT_LIMIT
                    || (bounds.1 != usize::MAX && bounds.1 > REPEAT_LIMIT)
                {
                    return None;
                }
                self.position = close;
                bounds
            }
            _ => return Some(atom),
        };
        self.position += 1;
        if matches!(atom, Node::Start | Node::End | Node::WordBoundary(_)) {
            return None;
        }
        let greedy = !self.eat('?');
        Some(Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            greedy,
        })
    }
}

impl Regex {
    /// None when the expression is malformed or outside the supported subset.
    pub fn new(pattern: &str) -> Option<Self> {
        let mut parser = Parser {
            chars: pattern.chars().collect(),
            position: 0,
        };
        let alternatives = parser.alternatives()?;
        (parser.position == parser.chars.len()).then_some(Self { alternatives })
    }

    /// The non-overlapping non-empty matches in `text`, leftmost first, as byte ranges: at most
    /// `limit` of them. One step budget covers the whole text, so a pathological pattern gives up
    /// with what it found rather than hanging.
    pub fn find_all(&self, text: &str, limit: usize) -> Vec<std::ops::Range<usize>> {
        let chars: Vec<char> = text.chars().collect();
        let mut offsets: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
        offsets.push(text.len());
        let steps = Cell::new(0_usize);
        let mut found = Vec::new();
        let mut start = 0;
        while start <= chars.len() && found.len() < limit {
            let mut end = None;
            let matched = match_alternatives(&self.alternatives, &chars, start, &steps, &mut |p| {
                end = Some(p);
                true
            });
            if steps.get() > STEP_LIMIT {
                break;
            }
            match end.filter(|end| matched && *end > start) {
                Some(end) => {
                    found.push(offsets[start]..offsets[end]);
                    start = end;
                }
                None => start += 1,
            }
        }
        found
    }

    /// The first (leftmost) match in `text`, as a substring.
    pub fn first_match<'t>(&self, text: &'t str) -> Option<&'t str> {
        let chars: Vec<char> = text.chars().collect();
        let byte_of = |index: usize| {
            text.char_indices()
                .nth(index)
                .map_or(text.len(), |(byte, _)| byte)
        };
        let steps = Cell::new(0_usize);
        for start in 0..=chars.len() {
            let mut end = None;
            let matched = match_alternatives(&self.alternatives, &chars, start, &steps, &mut |p| {
                end = Some(p);
                true
            });
            if steps.get() > STEP_LIMIT {
                return None;
            }
            if matched {
                return Some(&text[byte_of(start)..byte_of(end?)]);
            }
        }
        None
    }
}

// MARK: Matching

type Continuation<'k> = &'k mut dyn FnMut(usize) -> bool;

fn match_alternatives(
    alternatives: &[Vec<Node>],
    text: &[char],
    position: usize,
    steps: &Cell<usize>,
    k: Continuation,
) -> bool {
    for alternative in alternatives {
        if match_sequence(alternative, 0, text, position, steps, k) {
            return true;
        }
    }
    false
}

fn match_sequence(
    nodes: &[Node],
    index: usize,
    text: &[char],
    position: usize,
    steps: &Cell<usize>,
    k: Continuation,
) -> bool {
    steps.set(steps.get() + 1);
    if steps.get() > STEP_LIMIT {
        return false;
    }
    let Some(node) = nodes.get(index) else {
        return k(position);
    };
    let single = |accept: bool, k: Continuation| {
        accept && match_sequence(nodes, index + 1, text, position + 1, steps, k)
    };
    let here = text.get(position).copied();
    match node {
        Node::Char(c) => single(here == Some(*c), k),
        Node::Any => single(here.is_some_and(|c| c != '\n'), k),
        Node::Class { negated, items } => single(
            here.is_some_and(|c| items.iter().any(|item| item.matches(c)) != *negated),
            k,
        ),
        Node::Start => position == 0 && match_sequence(nodes, index + 1, text, position, steps, k),
        Node::End => {
            position == text.len() && match_sequence(nodes, index + 1, text, position, steps, k)
        }
        Node::WordBoundary(want) => {
            let before = position > 0 && is_word(text[position - 1]);
            let after = here.is_some_and(is_word);
            (before != after) == *want && match_sequence(nodes, index + 1, text, position, steps, k)
        }
        Node::Group(alternatives) => {
            match_alternatives(alternatives, text, position, steps, &mut |p| {
                match_sequence(nodes, index + 1, text, p, steps, k)
            })
        }
        Node::Repeat {
            node,
            min,
            max,
            greedy,
        } => repeat(
            node, *min, *max, *greedy, 0, nodes, index, text, position, steps, k,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn repeat(
    node: &Node,
    min: usize,
    max: usize,
    greedy: bool,
    count: usize,
    nodes: &[Node],
    index: usize,
    text: &[char],
    position: usize,
    steps: &Cell<usize>,
    k: Continuation,
) -> bool {
    steps.set(steps.get() + 1);
    if steps.get() > STEP_LIMIT {
        return false;
    }
    let rest = |position: usize, k: Continuation| {
        match_sequence(nodes, index + 1, text, position, steps, k)
    };
    let more = |k: Continuation| {
        count < max
            && match_sequence(
                std::slice::from_ref(node),
                0,
                text,
                position,
                steps,
                &mut |p| {
                    // A repetition that consumed nothing cannot loop forever.
                    if p == position && count >= min {
                        return false;
                    }
                    repeat(
                        node,
                        min,
                        max,
                        greedy,
                        count + 1,
                        nodes,
                        index,
                        text,
                        p,
                        steps,
                        k,
                    )
                },
            )
    };
    if greedy {
        if more(k) {
            return true;
        }
        count >= min && rest(position, k)
    } else {
        if count >= min && rest(position, k) {
            return true;
        }
        more(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(pattern: &str, text: &str) -> Option<String> {
        Regex::new(pattern)?.first_match(text).map(str::to_owned)
    }

    #[test]
    fn literals_classes_and_escapes() {
        assert_eq!(found("abc", "xxabcxx").as_deref(), Some("abc"));
        assert_eq!(
            found("a.c", "a\nc abc").as_deref(),
            Some("abc"),
            "dot skips newlines"
        );
        assert_eq!(found("[a-c]+", "xxabcabcz").as_deref(), Some("abcabc"));
        assert_eq!(found("[^a-c]+", "abcxyzabc").as_deref(), Some("xyz"));
        assert_eq!(found(r"\d+", "order 4521 shipped").as_deref(), Some("4521"));
        assert_eq!(found(r"\w+", "  hello_1 ").as_deref(), Some("hello_1"));
        assert_eq!(found(r"\s+", "a \t b").as_deref(), Some(" \t "));
        assert_eq!(found(r"\D+", "12ab34").as_deref(), Some("ab"));
        assert_eq!(
            found(r"[\d.]+", "price 12.50 USD").as_deref(),
            Some("12.50")
        );
        assert_eq!(found(r"\.", "a.b").as_deref(), Some("."));
        assert_eq!(
            found(r"[]a]", "x]").as_deref(),
            Some("]"),
            "a leading ] is literal"
        );
        assert_eq!(
            found("日本", "こんにちは日本語").as_deref(),
            Some("日本"),
            "multibyte text"
        );
    }

    #[test]
    fn anchors_and_word_boundaries() {
        assert_eq!(found("^ab", "abab").as_deref(), Some("ab"));
        assert_eq!(found("^b", "ab"), None);
        assert_eq!(found("ab$", "abab").as_deref(), Some("ab"));
        assert_eq!(found("a$", "ab"), None);
        assert_eq!(found(r"\bcat\b", "concat cat").as_deref(), Some("cat"));
        assert_eq!(found(r"\Bcat", "cat concat").as_deref(), Some("cat"));
        assert_eq!(found(r"\bcat\b", "concatenate"), None);
    }

    #[test]
    fn quantifiers_greedy_and_lazy() {
        assert_eq!(found("a*", "aaab").as_deref(), Some("aaa"));
        assert_eq!(
            found("a*", "baaa").as_deref(),
            Some(""),
            "empty match at the start"
        );
        assert_eq!(found("a+?", "aaa").as_deref(), Some("a"));
        assert_eq!(found("<.+>", "<a><b>").as_deref(), Some("<a><b>"));
        assert_eq!(found("<.+?>", "<a><b>").as_deref(), Some("<a>"));
        assert_eq!(found("colou?r", "color").as_deref(), Some("color"));
        assert_eq!(found("a{2}", "aaaa").as_deref(), Some("aa"));
        assert_eq!(found("a{2,}", "aaaa").as_deref(), Some("aaaa"));
        assert_eq!(found("a{1,2}", "aaaa").as_deref(), Some("aa"));
        assert_eq!(found("a{3}", "aa"), None);
    }

    #[test]
    fn groups_and_alternation_backtrack() {
        assert_eq!(found("cat|dog", "hotdog").as_deref(), Some("dog"));
        assert_eq!(found("(ab)+", "xababab").as_deref(), Some("ababab"));
        assert_eq!(found("(?:a|b)c", "bc").as_deref(), Some("bc"));
        assert_eq!(
            found("(a|ab)(c|bcd)", "abcd").as_deref(),
            Some("abcd"),
            "backtracks into alternatives"
        );
        assert_eq!(found("a*ab", "aaab").as_deref(), Some("aaab"));
        assert_eq!(found("x*", "").as_deref(), Some(""));
        assert_eq!(
            found("(a*)*b", "aaab").as_deref(),
            Some("aaab"),
            "empty iterations terminate"
        );
    }

    #[test]
    fn unsupported_or_broken_patterns_do_not_compile() {
        for bad in [
            "(", ")", "[abc", "*a", "a{2", "a{3,1}", "(?=a)", r"\1", r"\p{L}", "[z-a]", "^*",
            "a{1001}",
        ] {
            assert!(Regex::new(bad).is_none(), "{bad}");
        }
        assert!(
            Regex::new("").is_some(),
            "the empty expression matches the empty string"
        );
    }

    #[test]
    fn find_all_returns_byte_ranges_of_every_match() {
        let regex = Regex::new("[0-9]+").unwrap();
        let text = "é12 ✓ 345";
        let ranges = regex.find_all(text, 10);
        let found: Vec<&str> = ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(found, ["12", "345"]);
        assert_eq!(regex.find_all(text, 1).len(), 1);
        // An expression that can match nothing yields no empty matches.
        assert!(Regex::new("x*").unwrap().find_all("abc", 10).is_empty());
    }

    #[test]
    fn a_pathological_pattern_gives_up_instead_of_hanging() {
        let text = "a".repeat(40) + "!";
        assert_eq!(found("(a+)+$", &text), None);
        assert_eq!(found("(a*)*(a*)*(a*)*c", &"a".repeat(30)), None);
    }
}
