//! The text itself: laying out and colouring it, the caret line, flashes and debugger values,
//! diagnostic underlines and their tooltips, and the rows the gutter is drawn against.

use super::color_sites::ColorMap;
use super::indent_guides::indent_guides;
use super::symbols::SymbolTable;
use super::syntax::Span;
use super::{
    EditorState, FLASH, Flash, GutterRows, TEXT_INSET, carets, color_sites, edit_ops, find,
    navigation, swatches, symbols, syntax, text_hash,
};
use crate::model::editor_theme::{SurfaceColor, SyntaxRole};
use crate::pine::syntax::token::{Diagnostic, DiagnosticSeverity};
use crate::ui::editor_appearance::{Appearance, EditorFont};
use crate::ui::theme;
use eframe::egui::{
    self, Align, Align2, Margin, Rect, Shape, Stroke, TextBuffer, Ui, UiBuilder, Vec2,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};
use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

/// Room kept under the last line, so it can be scrolled clear of the status bar and the
/// horizontal scrollbar.
const END_CLEARANCE: f32 = 28.0;
/// How far below the caret's line a scroll to the caret keeps visible.
const CARET_CLEARANCE: f32 = 14.0;

/// A text's classification, done while laying it out.
struct Classified {
    key: u64,
    spans: Arc<Vec<Span>>,
    table: Arc<SymbolTable>,
    colors: Arc<ColorMap>,
}

pub(super) fn text_view(
    ui: &mut Ui,
    state: &mut EditorState,
    area: Rect,
    look: &Appearance,
    code: &EditorFont,
    edited: &mut bool,
) -> GutterRows {
    let id = state.id;
    let font = code.code(ui.ctx());
    let line_height = code.line_height_points();
    let spans = state.spans();
    let colors = state.colors();
    let current = text_hash(&state.source);
    let char_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, ' '));
    let metrics = swatches::Metrics::new(font.size, char_width);
    state.swatches.gap = metrics.gap;

    if let Some(offset) = state.jump_to.take() {
        let index = syntax::compiled_utf16_to_char_index(&state.source, offset);
        let mut edit_state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
        edit_state.store(ui.ctx(), id);
        ui.memory_mut(|memory| memory.request_focus(id));
        state.flash = Some(Flash::at(index));
        ui.ctx().request_repaint();
    }
    if let Some(range) = state.select_to.take() {
        let mut edit_state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(range.start),
            CCursor::new(range.end),
        )));
        edit_state.store(ui.ctx(), id);
        // The find field keeps focus so typing refines the search; the match is selected and
        // scrolled into view.
        state.flash = Some(Flash::at(range.start));
        ui.ctx().request_repaint();
    }
    let flash = state.flash;
    let mut scroll_to = state.scroll_to_line.take();
    let mut reveal_stop = state.reveal_stop.take();
    let find_matches = if state.find.open {
        find::matches(&state.source, &state.find.query)
    } else {
        Vec::new()
    };
    let find_current = state.find.current;

    // Text typed this frame is classified here, once however often the field asks for a layout,
    // and handed to the state afterwards, so the next frame finds it cached.
    let typed: RefCell<Option<Classified>> = RefCell::new(None);
    // The colours of the text the field last laid out, which is the text its galley shows.
    let laid_out: RefCell<Arc<ColorMap>> = RefCell::new(Arc::clone(&colors));
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| {
        let key = text_hash(text.as_str());
        let mut typed = typed.borrow_mut();
        let (spans, text_colors): (Arc<Vec<Span>>, Arc<ColorMap>) = if key == current {
            (Arc::clone(&spans), Arc::clone(&colors))
        } else if let Some(done) = typed.as_ref().filter(|done| done.key == key) {
            (Arc::clone(&done.spans), Arc::clone(&done.colors))
        } else {
            let table = Arc::new(symbols::analyze(text.as_str()));
            let spans = Arc::new(syntax::classify(text.as_str(), Some(&table)));
            let found = Arc::new(color_sites::map(text.as_str()));
            *typed = Some(Classified {
                key,
                spans: Arc::clone(&spans),
                table,
                colors: Arc::clone(&found),
            });
            (spans, found)
        };
        let mut job = syntax::layout_job_with_gaps(
            text.as_str(),
            &spans,
            font.clone(),
            line_height,
            look,
            look.role(SyntaxRole::Default),
            &text_colors.bytes,
            metrics.gap,
        );
        *laid_out.borrow_mut() = text_colors;
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };

    let mut pane = ui.new_child(UiBuilder::new().max_rect(area));
    pane.set_clip_rect(area.intersect(ui.clip_rect()));
    let mut rows = GutterRows {
        rows: Vec::new(),
        caret_line: None,
    };
    let scroll = egui::ScrollArea::both()
        .id_salt(("script-editor-scroll", id))
        .auto_shrink([false, false])
        .show_viewport(&mut pane, |ui, viewport| {
            // The text field paints the selection and the caret from the visuals, so this ui
            // (and the extra carets, which read the same) wears the theme's.
            ui.visuals_mut().selection.bg_fill = look.surface(SurfaceColor::Selection);
            ui.visuals_mut().text_cursor.stroke.color = look.surface(SurfaceColor::Caret);
            // Decorations that sit behind the text are painted into a slot reserved first.
            let behind = ui.painter().add(Shape::Noop);
            let visible_width = viewport.width().max(ui.available_width());
            let text_edit = egui::TextEdit::multiline(&mut state.source)
                .id(id)
                .font(font.clone())
                .frame(egui::Frame::NONE)
                .margin(Margin {
                    left: TEXT_INSET as i8,
                    right: TEXT_INSET as i8,
                    top: TEXT_INSET as i8,
                    bottom: (TEXT_INSET + END_CLEARANCE) as i8,
                })
                .desired_width(f32::INFINITY)
                .min_size(Vec2::new(visible_width, viewport.height()))
                .lock_focus(true)
                .layouter(&mut layouter)
                .show(ui);
            if text_edit.response.response.changed() {
                *edited = true;
            }
            let galley = &text_edit.galley;
            let origin = text_edit.galley_pos;
            carets::resolve(ui.ctx(), state, galley, origin, char_width);
            navigation::hover(ui, state, &text_edit.response.response, galley, origin);
            navigation::context_menu(
                ui.ctx(),
                state,
                &text_edit.response.response,
                galley,
                origin,
            );
            for placed in &galley.rows {
                rows.rows.push((origin.y + placed.pos.y, placed.row.size.y));
            }

            if let Some(line) = scroll_to.take()
                && let Some(row) = galley
                    .rows
                    .get(line.min(galley.rows.len().saturating_sub(1)))
            {
                let top = origin.y + row.pos.y;
                ui.scroll_to_rect(
                    Rect::from_min_size(egui::pos2(origin.x, top), Vec2::new(1.0, row.row.size.y))
                        .expand2(Vec2::new(0.0, 40.0)),
                    Some(Align::Center),
                );
            }
            if let Some(line) = reveal_stop.take()
                && let Some(row) = galley.rows.get(line)
            {
                let top = origin.y + row.pos.y;
                let target =
                    Rect::from_min_size(egui::pos2(origin.x, top), Vec2::new(1.0, row.row.size.y))
                        .expand2(Vec2::new(0.0, 40.0));
                if !ui.clip_rect().contains_rect(target) {
                    ui.scroll_to_rect(target, Some(Align::Center));
                }
            }
            let caret = text_edit.cursor_range;
            state.caret_rect = caret.map(|range| {
                galley
                    .pos_from_cursor(range.primary)
                    .translate(origin.to_vec2())
            });
            // A caret move keeps its line, and some room under it, in view: egui alone stops at
            // the viewport's edge, which leaves the last line cut off.
            let caret_index = caret.map(|range| range.primary.index.0);
            if caret_index != state.caret_seen {
                state.caret_seen = caret_index;
                if let Some(rect) = state.caret_rect
                    && text_edit.response.response.has_focus()
                {
                    let target =
                        Rect::from_min_max(rect.min, rect.max + Vec2::new(0.0, CARET_CLEARANCE));
                    ui.scroll_to_rect(target, None);
                }
            }
            let mut under = Vec::new();
            let clip = ui.clip_rect();
            let band_x = clip.x_range();
            // Current-line band (caret only) and indent guides, behind the text.
            let mut behind_shapes = Vec::new();
            if let Some(stop) = state.debug.stop
                && let Some(row) = galley.rows.get(stop.line.saturating_sub(1))
            {
                let top = origin.y + row.pos.y;
                let color = if stop.error {
                    theme::editor_debug_error_background()
                } else if stop.innermost {
                    theme::editor_debug_line_background()
                } else {
                    theme::editor_debug_caller_background()
                };
                behind_shapes.push(Shape::rect_filled(
                    Rect::from_x_y_ranges(band_x, top..=top + row.row.size.y),
                    0.0,
                    color,
                ));
            }
            // The code the pointer is on in the profile column, first line to last.
            if let Some((first, last)) = state.profile_hover
                && let Some(first_row) = galley.rows.get(first.saturating_sub(1))
                && let Some(last_row) = galley.rows.get(
                    last.saturating_sub(1)
                        .min(galley.rows.len().saturating_sub(1)),
                )
            {
                let top = origin.y + first_row.pos.y;
                let bottom = origin.y + last_row.pos.y + last_row.row.size.y;
                behind_shapes.push(Shape::rect_filled(
                    Rect::from_x_y_ranges(band_x, top..=bottom.max(top)),
                    0.0,
                    theme::profiler_block(),
                ));
            }
            if let Some(range) = caret
                && range.primary == range.secondary
                && let Some(row) = galley
                    .rows
                    .get(galley.layout_from_cursor(range.primary).row)
            {
                let top = origin.y + row.pos.y;
                rows.caret_line = Some(galley.layout_from_cursor(range.primary).row);
                behind_shapes.push(Shape::rect_filled(
                    Rect::from_x_y_ranges(band_x, top..=top + row.row.size.y),
                    0.0,
                    look.surface(SurfaceColor::CurrentLine),
                ));
            }
            for (index, span) in find_matches.iter().enumerate() {
                let a = galley.pos_from_cursor(CCursor::new(span.start));
                let b = galley.pos_from_cursor(CCursor::new(span.end));
                if (a.min.y - b.min.y).abs() < 1.0 {
                    let rect = Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y))
                        .translate(origin.to_vec2());
                    let color = look.surface(if index == find_current {
                        SurfaceColor::FindMatchActive
                    } else {
                        SurfaceColor::FindMatch
                    });
                    behind_shapes.push(Shape::rect_filled(rect, 2.0, color));
                }
            }
            // Same-identifier highlights and the bracket pair, only while the caret is a point.
            if let Some(range) = caret
                && range.primary == range.secondary
            {
                let at = range.primary.index.0;
                let span_rect = |span: std::ops::Range<usize>| {
                    let a = galley.pos_from_cursor(CCursor::new(span.start));
                    let b = galley.pos_from_cursor(CCursor::new(span.end));
                    ((a.min.y - b.min.y).abs() < 1.0).then(|| {
                        Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y))
                            .translate(origin.to_vec2())
                    })
                };
                for (index, span) in edit_ops::occurrences(&state.source, at)
                    .into_iter()
                    .enumerate()
                {
                    if let Some(rect) = span_rect(span) {
                        // The token under the caret is stronger than the others (by 115/178).
                        let strong = look.surface(SurfaceColor::WordHighlight);
                        let color = if index == 0 {
                            strong
                        } else {
                            strong.gamma_multiply(115.0 / 178.0)
                        };
                        behind_shapes.push(Shape::rect_filled(rect, 2.0, color));
                    }
                }
                if let Some((position, partner)) = edit_ops::bracket_match(&state.source, at) {
                    let (color, spans) = match partner {
                        Some(other) => (
                            look.surface(SurfaceColor::BracketMatch),
                            vec![position, other],
                        ),
                        None => (
                            theme::with_alpha(look.surface(SurfaceColor::DiagnosticError), 0.3),
                            vec![position],
                        ),
                    };
                    let border = look.surface(SurfaceColor::BracketMatchBorder);
                    for position in spans {
                        if let Some(rect) = span_rect(position..position + 1) {
                            behind_shapes.push(Shape::rect_filled(rect, 2.0, color));
                            if partner.is_some() && border.a() > 0 {
                                behind_shapes.push(Shape::rect_stroke(
                                    rect,
                                    2.0,
                                    Stroke::new(1.0, border),
                                    egui::StrokeKind::Inside,
                                ));
                            }
                        }
                    }
                }
            }
            behind_shapes.extend(indent_guides(
                &state.source,
                galley,
                origin,
                char_width,
                caret.map(|c| c.primary.index.0),
                look,
            ));
            let (extra_behind, extra_over) =
                carets::shapes(state, galley, origin, ui.visuals(), char_width);
            behind_shapes.extend(extra_behind);
            if let Some(flash) = flash
                && flash.whole_line
                && flash.at.elapsed() < FLASH
            {
                let placed = galley
                    .pos_from_cursor(CCursor::new(flash.index))
                    .translate(origin.to_vec2());
                behind_shapes.push(Shape::rect_filled(
                    Rect::from_x_y_ranges(band_x, placed.top()..=placed.bottom()),
                    0.0,
                    theme::with_alpha(theme::YELLOW, 0.38 * flash.strength()),
                ));
            }
            ui.painter().set(behind, Shape::Vec(behind_shapes));
            ui.painter().extend(extra_over);
            let shown_colors = Arc::clone(&laid_out.borrow());
            swatches::paint(ui, state, &shown_colors, galley, origin, look, metrics);

            // Diagnostic underlines and their merged hover messages (over the text).
            let marked = state
                .diagnostics
                .iter()
                .map(|diagnostic| (diagnostic, false))
                .chain(state.runtime_problems.iter().map(|d| (d, true)));
            for (diagnostic, runtime) in marked {
                // A failure with no place in the text has nothing to underline.
                if runtime && diagnostic.range.end.offset == 0 {
                    continue;
                }
                if let Some(rect) = underline_rect(&state.source, galley, origin, diagnostic) {
                    let color = look.surface(match diagnostic.severity {
                        DiagnosticSeverity::Error => SurfaceColor::DiagnosticError,
                        DiagnosticSeverity::Warning => SurfaceColor::DiagnosticWarning,
                    });
                    ui.painter().line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        Stroke::new(1.0, color),
                    );
                    under.push((rect, diagnostic.message.clone()));
                }
            }
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
                && text_edit.response.response.hovered()
            {
                let hits: Vec<&(Rect, String)> = under
                    .iter()
                    .filter(|(rect, _)| rect.expand2(Vec2::new(0.0, 2.0)).contains(pointer))
                    .collect();
                // The tooltip hangs off the underlined text, not the whole editor (whose bottom
                // edge is where a plain `on_hover_text` would put it).
                if let Some((anchor, _)) = hits.first() {
                    let messages: Vec<&str> =
                        hits.iter().map(|(_, message)| message.as_str()).collect();
                    let mut at_text = text_edit.response.response.clone();
                    at_text.interact_rect = *anchor;
                    at_text.on_hover_text(messages.join("\n"));
                }
            }

            // A name under the pointer shows its value while the script is stopped.
            if !state.debug.values.is_empty()
                && under.is_empty()
                && let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
                && text_edit.response.response.hovered()
            {
                let index = galley
                    .cursor_from_pos(pointer.to_vec2() - origin.to_vec2())
                    .index
                    .0;
                if let Some(word) = word_at(&state.source, index)
                    && let Some((value, type_name)) = state.debug.values.get(&word.text)
                {
                    let a = galley.pos_from_cursor(CCursor::new(word.range.start));
                    let b = galley.pos_from_cursor(CCursor::new(word.range.end));
                    if (a.min.y - b.min.y).abs() < 1.0 {
                        let anchor = Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y))
                            .translate(origin.to_vec2());
                        if anchor.expand2(Vec2::new(0.0, 1.0)).contains(pointer) {
                            let mut at_text = text_edit.response.response.clone();
                            at_text.interact_rect = anchor;
                            at_text
                                .on_hover_text(format!("{} = {value}  ({type_name})", word.text));
                        }
                    }
                }
            }
            // The values of the stopped line's names, after its text.
            if let Some(stop) = state.debug.stop
                && !state.debug.values.is_empty()
                && let Some(row) = galley.rows.get(stop.line.saturating_sub(1))
                && let Some(text) = state.source.split('\n').nth(stop.line.saturating_sub(1))
            {
                let inline = inline_values(text, &state.debug.values);
                if !inline.is_empty() {
                    ui.painter().text(
                        egui::pos2(
                            origin.x + row.pos.x + row.row.size.x + 3.0 * char_width,
                            origin.y + row.pos.y + row.row.size.y / 2.0,
                        ),
                        Align2::LEFT_CENTER,
                        inline,
                        code.at(ui.ctx(), code.size - 1.5),
                        theme::text_tertiary(),
                    );
                }
            }

            // Flash the revealed position, and bring it into view.
            if let Some(flash) = flash {
                let age = flash.at.elapsed();
                let cursor_rect = galley.pos_from_cursor(CCursor::new(flash.index));
                let placed = cursor_rect.translate(origin.to_vec2());
                if age < FLASH {
                    if age.is_zero() || age < Duration::from_millis(40) {
                        ui.scroll_to_rect(
                            placed.expand2(Vec2::new(0.0, 40.0)),
                            Some(Align::Center),
                        );
                    }
                    // A whole line is painted behind the text, with the other bands.
                    if !flash.whole_line {
                        let size = Vec2::new(char_width * 1.2, placed.height());
                        ui.painter().rect_filled(
                            Rect::from_min_size(placed.min - Vec2::new(2.0, 0.0), size).expand(3.0),
                            4.0,
                            theme::with_alpha(theme::YELLOW, 0.35 * flash.strength()),
                        );
                    }
                    ui.ctx().request_repaint();
                }
            }
        });
    if state.flash.is_some_and(|flash| flash.at.elapsed() >= FLASH) {
        state.flash = None;
    }
    // Text typed this frame was classified for its layout: keep that for the next frame, when it
    // is still the text (the layouter may have seen a half-applied edit).
    if let Some(done) = typed.take()
        && done.key == text_hash(&state.source)
    {
        state.highlighted = Some((done.key, done.spans));
        state.nav.table = Some((done.key, done.table));
        state.swatches.cache(done.key, done.colors);
    }
    #[cfg(test)]
    {
        state.scroll_y = scroll.state.offset.y;
    }
    #[cfg(not(test))]
    let _ = scroll;
    rows
}

/// An identifier in the text, with its place as character offsets.
struct Word {
    text: String,
    range: std::ops::Range<usize>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The identifier the character at `index` is part of (or ends), if any.
fn word_at(source: &str, index: usize) -> Option<Word> {
    let chars: Vec<char> = source.chars().collect();
    let at = if chars.get(index).is_some_and(|c| is_word(*c)) {
        index
    } else if index > 0 && chars.get(index - 1).is_some_and(|c| is_word(*c)) {
        index - 1
    } else {
        return None;
    };
    let mut start = at;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    let text: String = chars[start..end].iter().collect();
    // A number, or the field of a dotted name, is not a variable.
    if text.chars().next().is_some_and(|c| c.is_ascii_digit())
        || (start > 0 && chars[start - 1] == '.')
    {
        return None;
    }
    Some(Word {
        text,
        range: start..end,
    })
}

type Values = std::collections::HashMap<String, (String, String)>;

/// `name = value` for each distinct variable named on the line, in order of appearance.
fn inline_values(line: &str, values: &Values) -> String {
    let code = line.split("//").next().unwrap_or(line);
    let mut seen: Vec<&str> = Vec::new();
    let mut parts = Vec::new();
    let mut start: Option<usize> = None;
    for (offset, c) in code
        .char_indices()
        .chain(std::iter::once((code.len(), ' ')))
    {
        match (start, is_word(c)) {
            (None, true) => start = Some(offset),
            (Some(begin), false) => {
                let word = &code[begin..offset];
                let field = code[..begin].ends_with('.');
                if !field
                    && !seen.contains(&word)
                    && let Some((value, _)) = values.get(word)
                {
                    seen.push(word);
                    parts.push(format!("{word} = {value}"));
                }
                start = None;
            }
            _ => {}
        }
    }
    parts.join("   ")
}

/// The screen rect a diagnostic underlines: its start to its end on the first line it touches
/// (one character wide when empty).
fn underline_rect(
    source: &str,
    galley: &egui::Galley,
    origin: egui::Pos2,
    diagnostic: &Diagnostic,
) -> Option<Rect> {
    let start = syntax::compiled_utf16_to_char_index(source, diagnostic.range.start.offset);
    let mut end =
        syntax::compiled_utf16_to_char_index(source, diagnostic.range.end.offset).max(start);
    // Underline only the first line of a multi-line range.
    let line_end = source
        .chars()
        .skip(start)
        .position(|c| c == '\n')
        .map_or(source.chars().count(), |n| start + n);
    end = end.min(line_end);
    if end == start {
        end = (start + 1).min(line_end.max(start + 1));
    }
    let a = galley.pos_from_cursor(CCursor::new(start));
    let b = galley.pos_from_cursor(CCursor::new(end));
    if (a.min.y - b.min.y).abs() > 1.0 {
        return None;
    }
    Some(
        Rect::from_min_max(
            egui::pos2(a.min.x, a.min.y),
            egui::pos2(b.min.x.max(a.min.x + 4.0), a.max.y),
        )
        .translate(origin.to_vec2()),
    )
}
