//! A small Markdown view for the assistant's replies: paragraphs, headings, bullet and numbered
//! items, quotes, fenced code with a Copy button, and inline bold, italic, code and links.
//!
//! This is deliberately not full CommonMark. A model's reply uses a handful of constructs, and a
//! reply arrives a few characters at a time, so the parser never fails and never waits: a fence
//! that has not closed yet is an open code block, and a `**` with nothing to close it is just two
//! stars. Underscores are never emphasis, because `plot_color` in prose must stay `plot_color`.
//! `parse` is pure and tested; `show` draws it.

use crate::script_manager::syntax;
use crate::ui::icons::{self, Icon};
use crate::ui::{editor_appearance, kit, theme};
use eframe::egui::{
    self, Color32, FontId, Id, Margin, Sense, Stroke, TextFormat, Ui, text::LayoutJob,
};
use std::time::{Duration, Instant};

// MARK: Model

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    /// An http(s) address this text links to.
    pub link: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    /// One bullet or numbered item. `marker` is "•" or "3.".
    Item {
        depth: u8,
        marker: String,
        spans: Vec<Span>,
    },
    Quote(Vec<Span>),
    Code {
        lang: String,
        text: String,
        /// The closing fence has arrived. False while a reply streams.
        closed: bool,
    },
    Rule,
}

// MARK: Parsing

pub fn parse(text: &str) -> Vec<Block> {
    let text = text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut quote: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        // A quote ends at the first line that is not one.
        if !quote.is_empty() && !trimmed.starts_with('>') {
            blocks.push(Block::Quote(inline(&quote.join("\n"))));
            quote.clear();
        }
        if let Some(width) = fence_width(trimmed) {
            flush(&mut paragraph, &mut blocks);
            let lang = trimmed[width..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned();
            let mut code = Vec::new();
            let mut closed = false;
            i += 1;
            while i < lines.len() {
                let candidate = lines[i].trim();
                if closes_fence(candidate, width) {
                    closed = true;
                    break;
                }
                code.push(lines[i]);
                i += 1;
            }
            blocks.push(Block::Code {
                lang,
                text: code.join("\n"),
                closed,
            });
            i += 1;
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else if let Some((level, rest)) = heading(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading {
                level,
                spans: inline(rest),
            });
        } else if is_rule(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Rule);
        } else if let Some(rest) = trimmed.strip_prefix('>') {
            flush(&mut paragraph, &mut blocks);
            quote.push(rest.strip_prefix(' ').unwrap_or(rest));
        } else if let Some((marker, rest)) = item(trimmed) {
            flush(&mut paragraph, &mut blocks);
            let indent = line.len() - line.trim_start().len();
            blocks.push(Block::Item {
                depth: (indent / 2).min(3) as u8,
                marker,
                spans: inline(rest),
            });
        } else if !paragraph.is_empty() || !continues_item(&blocks, line) {
            paragraph.push(trimmed);
        } else if let Some(Block::Item { spans, .. }) = blocks.last_mut() {
            // An indented line under an item belongs to it.
            spans.push(Span {
                text: " ".to_owned(),
                style: Style::default(),
            });
            spans.extend(inline(trimmed));
        }
        i += 1;
    }
    if !quote.is_empty() {
        blocks.push(Block::Quote(inline(&quote.join("\n"))));
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

fn flush(paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>) {
    if !paragraph.is_empty() {
        blocks.push(Block::Paragraph(inline(&paragraph.join("\n"))));
        paragraph.clear();
    }
}

/// The width of an opening fence (three or more backticks), if the line is one.
fn fence_width(trimmed: &str) -> Option<usize> {
    let ticks = trimmed.chars().take_while(|c| *c == '`').count();
    (ticks >= 3).then_some(ticks)
}

fn closes_fence(trimmed: &str, width: usize) -> bool {
    let ticks = trimmed.chars().take_while(|c| *c == '`').count();
    ticks >= width && ticks == trimmed.chars().count()
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed[hashes..].strip_prefix(' ')?;
    Some((hashes as u8, rest.trim()))
}

fn is_rule(trimmed: &str) -> bool {
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    matches!(first, '-' | '*' | '_') && trimmed.len() >= 3 && trimmed.chars().all(|c| c == first)
}

/// "- x", "* x", "+ x", "3. x" or "3) x": the marker to show and the text after it.
fn item(trimmed: &str) -> Option<(String, &str)> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(bullet) {
            return Some(("•".to_owned(), rest.trim_start()));
        }
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits) {
        let after = &trimmed[digits..];
        for stop in [". ", ") "] {
            if let Some(rest) = after.strip_prefix(stop) {
                return Some((format!("{}.", &trimmed[..digits]), rest.trim_start()));
            }
        }
    }
    None
}

fn continues_item(blocks: &[Block], line: &str) -> bool {
    matches!(blocks.last(), Some(Block::Item { .. })) && line.starts_with("  ")
}

/// Inline spans: `code`, **bold**, *italic*, [text](https://…) and bare http(s) addresses.
pub fn inline(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Span> = Vec::new();
    let mut buffer = String::new();
    let mut style = Style::default();
    let mut i = 0;
    fn push(out: &mut Vec<Span>, text: &mut String, style: &Style) {
        if text.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.style == *style => last.text.push_str(text),
            _ => out.push(Span {
                text: std::mem::take(text),
                style: style.clone(),
            }),
        }
        text.clear();
    }
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        // A backslash makes the next punctuation literal.
        if c == '\\' && next.is_some_and(|n| n.is_ascii_punctuation()) {
            buffer.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '`' {
            let run = chars[i..].iter().take_while(|c| **c == '`').count();
            if let Some(end) = find_run(&chars, i + run, run) {
                push(&mut out, &mut buffer, &style);
                let inner: String = chars[i + run..end].iter().collect();
                let mut code_style = style.clone();
                code_style.code = true;
                let mut inner = inner;
                if inner.len() > 2 && inner.starts_with(' ') && inner.ends_with(' ') {
                    inner = inner[1..inner.len() - 1].to_owned();
                }
                push(&mut out, &mut inner, &code_style);
                i = end + run;
                continue;
            }
            buffer.extend(std::iter::repeat_n('`', run));
            i += run;
            continue;
        }
        if c == '*' && next == Some('*') {
            if style.bold {
                push(&mut out, &mut buffer, &style);
                style.bold = false;
                i += 2;
                continue;
            }
            let opens = chars.get(i + 2).is_some_and(|n| !n.is_whitespace());
            if opens && close_double(&chars, i + 2).is_some() {
                push(&mut out, &mut buffer, &style);
                style.bold = true;
                i += 2;
                continue;
            }
            buffer.push_str("**");
            i += 2;
            continue;
        }
        if c == '*' {
            if style.italic {
                push(&mut out, &mut buffer, &style);
                style.italic = false;
                i += 1;
                continue;
            }
            let opens = next.is_some_and(|n| !n.is_whitespace() && n != '*');
            if opens && close_single(&chars, i + 1).is_some() {
                push(&mut out, &mut buffer, &style);
                style.italic = true;
                i += 1;
                continue;
            }
            buffer.push('*');
            i += 1;
            continue;
        }
        if c == '['
            && let Some((label_end, url_end)) = link_at(&chars, i)
        {
            push(&mut out, &mut buffer, &style);
            let label: String = chars[i + 1..label_end].iter().collect();
            let url: String = chars[label_end + 2..url_end].iter().collect();
            for mut span in inline(&label) {
                span.style.link = Some(url.clone());
                out.push(span);
            }
            i = url_end + 1;
            continue;
        }
        if c == 'h' && (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(') {
            let rest: String = chars[i..].iter().collect();
            if rest.starts_with("http://") || rest.starts_with("https://") {
                let raw: String = rest
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != ')' && *c != '>')
                    .collect();
                let url = raw
                    .trim_end_matches(['.', ',', ';', ':', '!', '?'])
                    .to_owned();
                if url.len() > "https://".len() {
                    push(&mut out, &mut buffer, &style);
                    let mut link_style = style.clone();
                    link_style.link = Some(url.clone());
                    let mut text = url.clone();
                    push(&mut out, &mut text, &link_style);
                    i += url.chars().count();
                    continue;
                }
            }
        }
        buffer.push(c);
        i += 1;
    }
    push(&mut out, &mut buffer, &style);
    out
}

/// Where a run of exactly `len` backticks starts, at or after `from`.
fn find_run(chars: &[char], from: usize, len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '`' {
            let run = chars[i..].iter().take_while(|c| **c == '`').count();
            if run == len {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// A closing `**` after `from`, not preceded by a space.
fn close_double(chars: &[char], from: usize) -> Option<usize> {
    (from + 1..chars.len().saturating_sub(1)).find(|&j| {
        chars[j] == '*'
            && chars[j + 1] == '*'
            && !chars[j - 1].is_whitespace()
            && chars[j - 1] != '*'
    })
}

/// A closing single `*` after `from`, not part of `**` and not preceded by a space.
fn close_single(chars: &[char], from: usize) -> Option<usize> {
    (from + 1..chars.len()).find(|&j| {
        chars[j] == '*'
            && !chars[j - 1].is_whitespace()
            && chars[j - 1] != '*'
            && chars.get(j + 1) != Some(&'*')
    })
}

/// `[label](http…)` starting at `open`: the index of `]` and of the closing `)`.
fn link_at(chars: &[char], open: usize) -> Option<(usize, usize)> {
    let label_end = (open + 1..chars.len()).find(|&j| chars[j] == ']')?;
    if chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let url_end = (label_end + 2..chars.len()).find(|&j| chars[j] == ')')?;
    let url: String = chars[label_end + 2..url_end].iter().collect();
    (label_end > open + 1 && (url.starts_with("https://") || url.starts_with("http://")))
        .then_some((label_end, url_end))
}

/// The link under character `index` of the text the spans spell, if any.
pub fn link_at_char(spans: &[Span], index: usize) -> Option<&str> {
    let mut start = 0;
    for span in spans {
        let end = start + span.text.chars().count();
        if index >= start && index < end {
            return span.style.link.as_deref();
        }
        start = end;
    }
    None
}

/// The plain text of some spans.
#[cfg(test)]
pub fn plain_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

// MARK: View

const BODY_SIZE: f32 = kit::size::BODY;
const CODE_SIZE: f32 = 12.0;
const BLOCK_GAP: f32 = 8.0;
/// How long a Copy button says "Copied".
const COPIED_FOR: Duration = Duration::from_millis(1400);

fn job(spans: &[Span], size: f32, color: Color32, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for span in spans {
        let style = &span.style;
        let (font, weight) = if style.code {
            (FontId::monospace(size - 1.0), kit::REGULAR)
        } else if style.bold {
            (FontId::proportional(size), kit::BOLD)
        } else {
            (FontId::proportional(size), kit::REGULAR)
        };
        let linked = style.link.is_some();
        job.append(
            &span.text,
            0.0,
            TextFormat {
                font_id: font,
                color: if linked { kit::ACCENT } else { color },
                coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
                italics: style.italic,
                background: if style.code {
                    theme::with_alpha(theme::text_primary(), 0.09)
                } else {
                    Color32::TRANSPARENT
                },
                underline: if linked {
                    Stroke::new(1.0, kit::ACCENT)
                } else {
                    Stroke::NONE
                },
                ..Default::default()
            },
        );
    }
    job.wrap.max_width = wrap.max(1.0);
    job
}

/// Draws `blocks` one under another. `id` keeps each block's Copy state apart from other messages'.
pub fn show(ui: &mut Ui, id: Id, blocks: &[Block]) {
    ui.spacing_mut().item_spacing.y = BLOCK_GAP;
    for (index, block) in blocks.iter().enumerate() {
        let id = id.with(index);
        match block {
            Block::Paragraph(spans) => text_block(ui, spans, BODY_SIZE, theme::text_primary()),
            Block::Heading { level, spans } => {
                let size = match level {
                    1 => 16.0,
                    2 => 15.0,
                    _ => 14.0,
                };
                let bold: Vec<Span> = spans
                    .iter()
                    .map(|span| Span {
                        text: span.text.clone(),
                        style: Style {
                            bold: true,
                            ..span.style.clone()
                        },
                    })
                    .collect();
                text_block(ui, &bold, size, theme::text_primary());
            }
            Block::Item {
                depth,
                marker,
                spans,
            } => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_space(f32::from(*depth) * 14.0 + 2.0);
                    ui.add_sized(
                        [16.0, 16.0],
                        egui::Label::new(kit::text(
                            marker,
                            BODY_SIZE,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ))
                        .selectable(false),
                    );
                    ui.vertical(|ui| text_block(ui, spans, BODY_SIZE, theme::text_primary()));
                });
            }
            Block::Quote(spans) => {
                let frame = egui::Frame::new().inner_margin(Margin {
                    left: 10,
                    right: 0,
                    top: 0,
                    bottom: 0,
                });
                let response = frame.show(ui, |ui| {
                    text_block(ui, spans, BODY_SIZE, theme::text_secondary())
                });
                let bar = response.response.rect;
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(bar.min, egui::pos2(bar.min.x + 2.0, bar.max.y)),
                    1.0,
                    theme::with_alpha(kit::ACCENT, 0.5),
                );
            }
            Block::Code { lang, text, closed } => code_block(ui, id, lang, text, *closed),
            Block::Rule => {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    Stroke::new(1.0, kit::hairline()),
                );
            }
        }
    }
}

/// A wrapped, selectable run of text whose links open in the browser.
fn text_block(ui: &mut Ui, spans: &[Span], size: f32, color: Color32) {
    let wrap = ui.available_width();
    let has_link = spans.iter().any(|span| span.style.link.is_some());
    let mut label = egui::Label::new(job(spans, size, color, wrap)).selectable(true);
    if has_link {
        label = label.sense(Sense::click());
    }
    let response = ui.add(label);
    if !has_link {
        return;
    }
    // Which character is under the pointer, from a second layout of the same text.
    let Some(pointer) = response.hover_pos().or(response.interact_pointer_pos()) else {
        return;
    };
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job(spans, size, color, wrap)));
    let index = galley.cursor_from_pos(pointer - response.rect.min).index.0;
    if let Some(url) = link_at_char(spans, index) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if response.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    }
}

fn code_block(ui: &mut Ui, id: Id, lang: &str, text: &str, closed: bool) {
    let copied_id = id.with("copied");
    let copied: Option<Instant> = ui.data(|data| data.get_temp(copied_id));
    let just_copied = copied.is_some_and(|at| at.elapsed() < COPIED_FOR);
    egui::Frame::new()
        .fill(theme::with_alpha(theme::text_primary(), 0.05))
        .stroke(Stroke::new(1.0, kit::hairline()))
        .corner_radius(8.0)
        .inner_margin(Margin::same(0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                let label = if lang.is_empty() { "code" } else { lang };
                ui.label(kit::text(
                    if closed {
                        label.to_owned()
                    } else {
                        format!("{label} …")
                    },
                    kit::size::CAPTION + 0.5,
                    kit::MEDIUM,
                    theme::text_tertiary(),
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    let (icon, tip, color) = if just_copied {
                        (Icon::Check, "Copied", kit::GREEN)
                    } else {
                        (Icon::CopyDoc, "Copy code", theme::text_secondary())
                    };
                    if icons::button(ui, icon, 24.0, color, tip).clicked() {
                        ui.ctx().copy_text(text.to_owned());
                        ui.data_mut(|data| data.insert_temp(copied_id, Instant::now()));
                        ui.ctx().request_repaint_after(COPIED_FOR);
                    }
                });
            });
            let font = editor_appearance::editor_font().at(ui.ctx(), CODE_SIZE);
            let plain = theme::text_primary();
            let mut code_job = if matches!(lang, "" | "pine" | "pinescript" | "pine-script") {
                let spans = syntax::classify(text, None);
                syntax::layout_job(
                    text,
                    &spans,
                    font,
                    None,
                    &editor_appearance::chrome(),
                    plain,
                )
            } else {
                let mut job = LayoutJob::default();
                job.append(text, 0.0, TextFormat::simple(font, plain));
                job
            };
            code_job.wrap.max_width = f32::INFINITY;
            egui::ScrollArea::horizontal()
                .id_salt(id.with("scroll"))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(Margin {
                            left: 10,
                            right: 10,
                            top: 2,
                            bottom: 10,
                        })
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(code_job).selectable(true));
                        });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        plain_text(&inline(text))
    }

    fn styles(text: &str) -> Vec<(String, bool, bool, bool)> {
        inline(text)
            .into_iter()
            .map(|s| (s.text, s.style.bold, s.style.italic, s.style.code))
            .collect()
    }

    #[test]
    fn plain_text_is_one_paragraph() {
        assert_eq!(
            parse("hello world"),
            vec![Block::Paragraph(inline("hello world"))]
        );
        assert!(parse("").is_empty());
        assert!(parse("\n\n  \n").is_empty());
    }

    #[test]
    fn blank_lines_split_paragraphs_and_single_breaks_are_kept() {
        let blocks = parse("one\ntwo\n\nthree");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            Block::Paragraph(spans) => assert_eq!(plain_text(spans), "one\ntwo"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn headings_need_a_space_and_one_to_six_hashes() {
        assert!(matches!(
            &parse("## Title")[0],
            Block::Heading { level: 2, .. }
        ));
        assert!(matches!(&parse("#nospace")[0], Block::Paragraph(_)));
        assert!(matches!(&parse("####### seven")[0], Block::Paragraph(_)));
    }

    #[test]
    fn bullets_and_numbers_become_items_with_markers_and_depth() {
        let blocks = parse("- a\n  - b\n1. first\n2) second");
        let items: Vec<(u8, String)> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Item { depth, marker, .. } => Some((*depth, marker.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            items,
            vec![
                (0, "•".to_owned()),
                (1, "•".to_owned()),
                (0, "1.".to_owned()),
                (0, "2.".to_owned())
            ]
        );
    }

    #[test]
    fn an_indented_line_continues_the_item_above() {
        let blocks = parse("1. Add an input\n   for the length\n2. Use it");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            Block::Item { spans, .. } => {
                assert_eq!(plain_text(spans), "Add an input for the length")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_star_without_a_space_is_emphasis_not_a_bullet() {
        assert!(matches!(
            &parse("*emphasis* then text")[0],
            Block::Paragraph(_)
        ));
        assert!(matches!(&parse("* a bullet")[0], Block::Item { .. }));
        assert!(matches!(&parse("3 apples")[0], Block::Paragraph(_)));
        assert!(matches!(&parse("2024. A year")[0], Block::Paragraph(_)));
    }

    #[test]
    fn fenced_code_keeps_its_language_and_text_exactly() {
        let blocks = parse("Before\n```pine\nplot(close)\n\n  x = 1\n```\nAfter");
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks[1],
            Block::Code {
                lang: "pine".to_owned(),
                text: "plot(close)\n\n  x = 1".to_owned(),
                closed: true
            }
        );
    }

    #[test]
    fn markdown_inside_code_is_left_alone() {
        let blocks = parse("```\n# not a heading\n- not an item\n**x**\n```");
        match &blocks[0] {
            Block::Code { text, .. } => assert_eq!(text, "# not a heading\n- not an item\n**x**"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_unterminated_fence_is_an_open_block() {
        let blocks = parse("Here:\n```pine\nplot(cl");
        assert_eq!(
            blocks[1],
            Block::Code {
                lang: "pine".to_owned(),
                text: "plot(cl".to_owned(),
                closed: false
            }
        );
    }

    #[test]
    fn a_longer_fence_is_only_closed_by_an_equal_or_longer_one() {
        let blocks = parse("````\n```\ninner\n```\n````\ntail");
        assert_eq!(
            blocks[0],
            Block::Code {
                lang: String::new(),
                text: "```\ninner\n```".to_owned(),
                closed: true
            }
        );
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn crlf_is_read_like_lf() {
        assert_eq!(parse("a\r\n\r\nb"), parse("a\n\nb"));
        assert_eq!(parse("```\r\nx\r\n```"), parse("```\nx\n```"));
    }

    #[test]
    fn quotes_and_rules() {
        let blocks = parse("> quoted\n> more\n\n---\ntext");
        assert!(matches!(&blocks[0], Block::Quote(spans) if plain_text(spans) == "quoted\nmore"));
        assert_eq!(blocks[1], Block::Rule);
        assert!(matches!(&blocks[2], Block::Paragraph(_)));
    }

    #[test]
    fn bold_italic_and_code() {
        assert_eq!(
            styles("a **b** *c* `d`"),
            vec![
                ("a ".to_owned(), false, false, false),
                ("b".to_owned(), true, false, false),
                (" ".to_owned(), false, false, false),
                ("c".to_owned(), false, true, false),
                (" ".to_owned(), false, false, false),
                ("d".to_owned(), false, false, true),
            ]
        );
    }

    #[test]
    fn unmatched_markers_are_literal_text() {
        assert_eq!(plain("2 * 3 = 6"), "2 * 3 = 6");
        assert_eq!(plain("a **b"), "a **b");
        assert_eq!(plain("a *b"), "a *b");
        assert_eq!(plain("`open"), "`open");
        assert_eq!(styles("**b").len(), 1);
    }

    #[test]
    fn underscores_are_never_emphasis() {
        assert_eq!(
            plain("use plot_color and my_var_name"),
            "use plot_color and my_var_name"
        );
        assert_eq!(inline("use plot_color").len(), 1);
    }

    #[test]
    fn bold_can_hold_italic_and_code_keeps_stars() {
        let spans = inline("**a *b* c** `x * y`");
        assert!(
            spans
                .iter()
                .any(|s| s.text == "b" && s.style.bold && s.style.italic)
        );
        assert!(spans.iter().any(|s| s.text == "x * y" && s.style.code));
    }

    #[test]
    fn backslash_escapes_punctuation() {
        assert_eq!(plain(r"\*not italic\*"), "*not italic*");
        assert_eq!(plain(r"a \ b"), r"a \ b");
    }

    #[test]
    fn links_are_http_only_and_map_back_from_a_character_index() {
        let spans = inline("See [the docs](https://example.com/a) now");
        assert_eq!(plain_text(&spans), "See the docs now");
        assert_eq!(link_at_char(&spans, 5), Some("https://example.com/a"));
        assert_eq!(link_at_char(&spans, 0), None);
        assert_eq!(link_at_char(&spans, 14), None);
        // Other schemes are not linked.
        let spans = inline("[x](javascript:alert(1)) and [y](file:///etc/passwd)");
        assert!(spans.iter().all(|s| s.style.link.is_none()));
    }

    #[test]
    fn bare_addresses_link_without_their_trailing_punctuation() {
        let spans = inline("Docs at https://example.com/pine.");
        let link = spans.iter().find(|s| s.style.link.is_some()).unwrap();
        assert_eq!(link.text, "https://example.com/pine");
        assert_eq!(plain_text(&spans), "Docs at https://example.com/pine.");
        assert!(inline("nohttp://x").iter().all(|s| s.style.link.is_none()));
    }

    #[test]
    fn non_ascii_text_survives() {
        assert_eq!(plain("日本語 **太字** 🙂"), "日本語 太字 🙂");
        let spans = inline("[日本](https://example.com)");
        assert_eq!(link_at_char(&spans, 1), Some("https://example.com"));
    }

    #[test]
    fn streaming_a_reply_never_panics_at_any_cut() {
        let reply = "## Plan\n1. Add **input** `length`\n2. See [docs](https://x.io)\n```pine\nplot(close)\n```\n> note\n- end *it* _x_";
        for end in 0..=reply.len() {
            if reply.is_char_boundary(end) {
                let _ = parse(&reply[..end]);
            }
        }
    }
}
