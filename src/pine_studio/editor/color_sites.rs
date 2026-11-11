//! Colour expressions in Pine source: where they are, what they evaluate to, and how to rewrite
//! one when the picker changes it. Pure text in, character ranges out; no egui.
//!
//! Detection walks lexer tokens (the AST gives no spans for a call and drops statements it cannot
//! parse), so comments and strings never match and a half-typed expression is simply not a site.
//! A site is the *outermost* expression that evaluates to a constant colour: a literal, a
//! `color.<name>`, a `color.rgb(..)` or a `color.new(..)` whose arguments are numeric literals
//! (with unary minus, `+ - * /` and parentheses). When an outer expression is not constant
//! (`color.new(c, transp)`), the scan goes on inside it, so a constant inner colour still gets its
//! own site; `cond ? color.green : color.red` gives two.
//!
//! Values use the runtime's own arithmetic (`pine::builtins`), so a swatch shows what the script
//! draws. Every range is in characters (what egui's cursor counts), not bytes or UTF-16 units.

use crate::pine::builtins;
use crate::pine::syntax::lexer::Lexer;
use crate::pine::syntax::token::{Token, TokenKind, ValueType};
use std::ops::Range;

/// Deep enough for any real expression; stops `((((…` from running the parser out of stack.
const MAX_DEPTH: usize = 24;

/// A constant colour expression and the spans needed to rewrite it.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorSite {
    /// The whole expression.
    pub range: Range<usize>,
    /// What it evaluates to, `0xRRGGBBAA` with Pine's alpha (255 opaque).
    pub rgba: u32,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// `#RRGGBB` or `#RRGGBBAA`; the digit count is the range's length less the `#`.
    Hex { lowercase: bool },
    /// `color.red` and the other constants.
    Named,
    /// `color.rgb(r, g, b)` or `color.rgb(r, g, b, t)`: three or four arguments.
    Rgb { args: Vec<Arg> },
    /// `color.new(<colour>, t)`.
    New {
        base: Box<ColorSite>,
        transparency: Arg,
    },
}

/// A numeric argument: where it is written and what it evaluates to.
#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub range: Range<usize>,
    pub value: f64,
}

impl ColorSite {
    pub fn rgb(&self) -> [u8; 3] {
        split(self.rgba)
    }

    /// Pine transparency (0 opaque … 100 invisible), not rounded.
    pub fn transparency(&self) -> f64 {
        alpha_to_transparency((self.rgba & 0xff) as u8)
    }

    /// The transparency a slider shows: the nearest whole percent.
    pub fn transparency_percent(&self) -> u8 {
        self.transparency().round().clamp(0.0, 100.0) as u8
    }
}

fn split(rgba: u32) -> [u8; 3] {
    [(rgba >> 24) as u8, (rgba >> 16) as u8, (rgba >> 8) as u8]
}

/// Pine transparency to the alpha byte, the way `color.new` does it.
pub fn transparency_to_alpha(transparency: u8) -> u8 {
    (builtins::with_transparency(0, f64::from(transparency)) & 0xff) as u8
}

/// The alpha byte as a Pine transparency (`color.t`).
pub fn alpha_to_transparency(alpha: u8) -> f64 {
    100.0 - f64::from(alpha) / 2.55
}

/// What the picker changed. A channel left `None` is written back exactly as it was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Picked {
    pub rgb: Option<[u8; 3]>,
    /// Pine transparency, 0..=100.
    pub transparency: Option<u8>,
}

/// The colours of one text, with where each begins in the forms the editor needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorMap {
    pub sites: Vec<ColorSite>,
    /// The byte offset each site starts at (ascending), for opening room in a layout.
    pub bytes: Vec<usize>,
    /// The zero-based line each site starts on (ascending).
    pub lines: Vec<usize>,
    /// How many characters into its line each site starts.
    pub columns: Vec<usize>,
}

impl ColorMap {
    /// The index of the site that starts at character `start`.
    pub fn at(&self, start: usize) -> Option<usize> {
        self.sites
            .binary_search_by_key(&start, |site| site.range.start)
            .ok()
    }

    /// The sites that start on lines `first..end`, as an index range.
    pub fn on_lines(&self, first: usize, end: usize) -> Range<usize> {
        self.lines.partition_point(|line| *line < first)
            ..self.lines.partition_point(|line| *line < end)
    }
}

/// Every constant colour in `source`, in order and without overlap.
pub fn scan(source: &str) -> Vec<ColorSite> {
    map(source).sites
}

/// [`scan`] with each site's place in the text.
pub fn map(source: &str) -> ColorMap {
    let chars: Vec<char> = source.chars().collect();
    let mut by_unit: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    for (index, character) in chars.iter().enumerate() {
        for _ in 0..character.len_utf16() {
            by_unit.push(index);
        }
    }
    by_unit.push(chars.len());
    let at = |unit: usize| by_unit.get(unit).copied().unwrap_or(chars.len());

    let lexed = Lexer::new(source).lex();
    let tokens: Vec<Piece> = lexed
        .tokens
        .iter()
        .map(|token: &Token| Piece {
            kind: &token.kind,
            start: at(token.range.start.offset),
            end: at(token.range.end.offset),
        })
        .collect();
    let parser = Parser {
        tokens: &tokens,
        chars: &chars,
    };

    let mut sites = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        match parser.color(index, 0) {
            Some((site, next)) => {
                sites.push(site);
                index = next;
            }
            None => index += 1,
        }
    }

    // Where each site starts: one walk over the text, sites being in order.
    let mut map = ColorMap {
        bytes: Vec::with_capacity(sites.len()),
        lines: Vec::with_capacity(sites.len()),
        columns: Vec::with_capacity(sites.len()),
        sites,
    };
    let (mut line, mut line_start, mut byte) = (0, 0, 0);
    let mut walked = 0;
    for site in &map.sites {
        while walked < site.range.start {
            byte += chars[walked].len_utf8();
            if chars[walked] == '\n' {
                line += 1;
                line_start = walked + 1;
            }
            walked += 1;
        }
        map.bytes.push(byte);
        map.lines.push(line);
        map.columns.push(site.range.start - line_start);
    }
    map
}

struct Piece<'a> {
    kind: &'a TokenKind,
    start: usize,
    end: usize,
}

struct Parser<'a> {
    tokens: &'a [Piece<'a>],
    chars: &'a [char],
}

impl Parser<'_> {
    fn kind(&self, index: usize) -> Option<&TokenKind> {
        self.tokens.get(index).map(|piece| piece.kind)
    }

    fn is(&self, index: usize, wanted: &TokenKind) -> bool {
        self.kind(index) == Some(wanted)
    }

    /// A colour expression starting at token `index`, and the index after it.
    fn color(&self, index: usize, depth: usize) -> Option<(ColorSite, usize)> {
        if depth > MAX_DEPTH {
            return None;
        }
        let piece = self.tokens.get(index)?;
        match piece.kind {
            TokenKind::Color(rgba) => {
                let digits = &self.chars[piece.start + 1..piece.end];
                let lowercase = digits.iter().any(char::is_ascii_lowercase)
                    && !digits.iter().any(char::is_ascii_uppercase);
                Some((
                    ColorSite {
                        range: piece.start..piece.end,
                        rgba: *rgba,
                        shape: Shape::Hex { lowercase },
                    },
                    index + 1,
                ))
            }
            TokenKind::TypeKeyword(ValueType::Color) => {
                // `input.color(` and `x.color` name something else.
                if index > 0 && self.is(index - 1, &TokenKind::Dot) {
                    return None;
                }
                if !self.is(index + 1, &TokenKind::Dot) {
                    return None;
                }
                let Some(TokenKind::Identifier(member)) = self.kind(index + 2) else {
                    return None;
                };
                let name_end = self.tokens[index + 2].end;
                let full = format!("color.{member}");
                if let Some(rgba) = builtins::color(&full) {
                    return Some((
                        ColorSite {
                            range: piece.start..name_end,
                            rgba,
                            shape: Shape::Named,
                        },
                        index + 3,
                    ));
                }
                if !self.is(index + 3, &TokenKind::LeftParen) {
                    return None;
                }
                match member.as_str() {
                    "rgb" => self.rgb(index, piece.start),
                    "new" => self.recolored(index, piece.start, depth),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// `color.rgb(` at `index + 3`.
    fn rgb(&self, index: usize, start: usize) -> Option<(ColorSite, usize)> {
        let mut next = index + 4;
        let mut args = Vec::new();
        loop {
            let (arg, after) = self.argument(next)?;
            args.push(arg);
            next = after;
            if self.is(next, &TokenKind::Comma) {
                next += 1;
                // A trailing comma before `)` is not Pine; leave it unresolved.
                continue;
            }
            break;
        }
        if !self.is(next, &TokenKind::RightParen) || !(3..=4).contains(&args.len()) {
            return None;
        }
        let transparency = args.get(3).map_or(0.0, |arg| arg.value);
        let rgba = builtins::rgb(args[0].value, args[1].value, args[2].value, transparency);
        Some((
            ColorSite {
                range: start..self.tokens[next].end,
                rgba,
                shape: Shape::Rgb { args },
            },
            next + 1,
        ))
    }

    /// `color.new(` at `index + 3`.
    fn recolored(&self, index: usize, start: usize, depth: usize) -> Option<(ColorSite, usize)> {
        let (base, mut next) = self.color(index + 4, depth + 1)?;
        if !self.is(next, &TokenKind::Comma) {
            return None;
        }
        next += 1;
        let (transparency, after) = self.argument(next)?;
        if !self.is(after, &TokenKind::RightParen) {
            return None;
        }
        let rgba = builtins::with_transparency(base.rgba, transparency.value);
        Some((
            ColorSite {
                range: start..self.tokens[after].end,
                rgba,
                shape: Shape::New {
                    base: Box::new(base),
                    transparency,
                },
            },
            after + 1,
        ))
    }

    fn argument(&self, index: usize) -> Option<(Arg, usize)> {
        let (value, after) = self.additive(index, 0)?;
        Some((
            Arg {
                range: self.tokens[index].start..self.tokens[after - 1].end,
                value,
            },
            after,
        ))
    }

    fn additive(&self, index: usize, depth: usize) -> Option<(f64, usize)> {
        let (mut value, mut next) = self.term(index, depth)?;
        loop {
            match self.kind(next) {
                Some(TokenKind::Plus) => {
                    let (right, after) = self.term(next + 1, depth)?;
                    value += right;
                    next = after;
                }
                Some(TokenKind::Minus) => {
                    let (right, after) = self.term(next + 1, depth)?;
                    value -= right;
                    next = after;
                }
                _ => return Some((value, next)),
            }
        }
    }

    fn term(&self, index: usize, depth: usize) -> Option<(f64, usize)> {
        let (mut value, mut next) = self.unary(index, depth)?;
        loop {
            match self.kind(next) {
                Some(TokenKind::Star) => {
                    let (right, after) = self.unary(next + 1, depth)?;
                    value *= right;
                    next = after;
                }
                Some(TokenKind::Slash) => {
                    let (right, after) = self.unary(next + 1, depth)?;
                    if right == 0.0 {
                        return None;
                    }
                    value /= right;
                    next = after;
                }
                _ => return Some((value, next)),
            }
        }
    }

    fn unary(&self, index: usize, depth: usize) -> Option<(f64, usize)> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.kind(index)? {
            TokenKind::Minus => {
                let (value, next) = self.unary(index + 1, depth + 1)?;
                Some((-value, next))
            }
            TokenKind::Plus => self.unary(index + 1, depth + 1),
            TokenKind::Number { value, .. } => Some((*value, index + 1)),
            TokenKind::LeftParen => {
                let (value, next) = self.additive(index + 1, depth + 1)?;
                self.is(next, &TokenKind::RightParen)
                    .then_some((value, next + 1))
            }
            _ => None,
        }
    }
}

/// The text `site` becomes when `picked` is applied. `text` is the site's own source (its range
/// of the document), so the result replaces exactly `site.range`. Anything the picker did not
/// touch keeps its original spelling; style is kept where the colour allows it.
pub fn rewrite(site: &ColorSite, text: &[char], picked: Picked) -> String {
    if picked.rgb.is_none() && picked.transparency.is_none() {
        return text.iter().collect();
    }
    let mut edits = Vec::new();
    collect(site, text, site.range.start, picked, &mut edits);
    edits.sort_by_key(|(range, _): &(Range<usize>, String)| range.start);

    let mut out = String::with_capacity(text.len() + 16);
    let mut at = site.range.start;
    for (range, replacement) in edits {
        out.extend(&text[at - site.range.start..range.start - site.range.start]);
        out.push_str(&replacement);
        at = range.end;
    }
    out.extend(&text[at - site.range.start..]);
    out
}

fn collect(
    site: &ColorSite,
    text: &[char],
    origin: usize,
    picked: Picked,
    out: &mut Vec<(Range<usize>, String)>,
) {
    let written = |range: &Range<usize>| -> String {
        text[range.start - origin..range.end - origin]
            .iter()
            .collect()
    };
    match &site.shape {
        Shape::Hex { lowercase } => {
            let [r, g, b] = picked.rgb.unwrap_or_else(|| site.rgb());
            let alpha = picked
                .transparency
                .map_or((site.rgba & 0xff) as u8, transparency_to_alpha);
            let eight = site.range.len() == 9 || alpha != 255;
            let mut hex = if eight {
                format!("#{r:02X}{g:02X}{b:02X}{alpha:02X}")
            } else {
                format!("#{r:02X}{g:02X}{b:02X}")
            };
            if *lowercase {
                hex = hex.to_lowercase();
            }
            out.push((site.range.clone(), hex));
        }
        Shape::Named => {
            let transparency = picked.transparency.unwrap_or(0);
            let base = match picked.rgb {
                None => written(&site.range),
                Some(rgb) => name_of(rgb).map_or_else(
                    || format!("color.rgb({}, {}, {})", rgb[0], rgb[1], rgb[2]),
                    str::to_string,
                ),
            };
            let replacement = if transparency == 0 {
                base
            } else {
                format!("color.new({base}, {transparency})")
            };
            out.push((site.range.clone(), replacement));
        }
        Shape::Rgb { args } => {
            if let Some(rgb) = picked.rgb {
                for (arg, wanted) in args.iter().zip(rgb) {
                    if channel(arg.value) != wanted {
                        out.push((arg.range.clone(), wanted.to_string()));
                    }
                }
            }
            if let Some(transparency) = picked.transparency {
                match args.get(3) {
                    Some(arg) => {
                        if arg.value != f64::from(transparency) {
                            out.push((arg.range.clone(), transparency.to_string()));
                        }
                    }
                    None if transparency != 0 => {
                        let end = args[2].range.end;
                        out.push((end..end, format!(", {transparency}")));
                    }
                    None => {}
                }
            }
        }
        Shape::New { base, transparency } => {
            if let Some(percent) = picked.transparency
                && transparency.value != f64::from(percent)
            {
                out.push((transparency.range.clone(), percent.to_string()));
            }
            if picked.rgb.is_some() {
                // The base's own transparency is replaced by `color.new`, so only its colour moves.
                let inner = Picked {
                    rgb: picked.rgb,
                    transparency: None,
                };
                collect(base, text, origin, inner, out);
            }
        }
    }
}

fn channel(value: f64) -> u8 {
    value.clamp(0.0, 255.0) as u8
}

/// The `color.*` constant with exactly this colour, if there is one.
fn name_of(rgb: [u8; 3]) -> Option<&'static str> {
    builtins::COLOR_NAMES
        .iter()
        .copied()
        .find(|name| builtins::color(name).is_some_and(|rgba| split(rgba) == rgb))
}

#[cfg(test)]
#[path = "color_sites_tests.rs"]
mod tests;
