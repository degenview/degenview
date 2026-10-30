//! The saved-versus-current popover a change marker opens: the hunk's lines side by side in a
//! compact diff, with the one action that undoes it. A view only: it reads a [`DiffHunk`] and
//! reports what was pressed; the editor applies it.

use super::diff::{ChangeKind, DiffHunk};
use super::syntax;
use crate::ui::{accessibility, editor_appearance, icons::Icon, kit, popover, theme};
use eframe::egui::{self, Align, Color32, Id, Margin, Rect, RichText, Ui};

const WIDTH: f32 = 500.0;
const PADDING: i8 = 12;
const MAX_CODE_HEIGHT: f32 = 260.0;
const CODE_SIZE: f32 = 12.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewEvent {
    None,
    Dismiss,
    /// The primary button: revert, or restore.
    Apply,
    /// Previous (`false`) or next (`true`) change.
    Step(bool),
}

/// "line 4", "lines 4–6" for a zero-based half-open range.
pub fn lines_label(range: &std::ops::Range<usize>) -> String {
    match range.len() {
        0 => String::new(),
        1 => format!("line {}", range.start + 1),
        _ => format!("lines {}–{}", range.start + 1, range.end),
    }
}

pub fn title(hunk: &DiffHunk) -> String {
    match hunk.kind {
        ChangeKind::Added => format!("Added {}", lines_label(&hunk.new_lines)),
        ChangeKind::Modified => format!("Modified {}", lines_label(&hunk.new_lines)),
        ChangeKind::Deleted if hunk.new_lines.start == 0 => "Deleted at the start".to_owned(),
        ChangeKind::Deleted => format!("Deleted after line {}", hunk.new_lines.start),
    }
}

/// The primary action's wording, also the context menu's.
pub fn action_label(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "Revert Added Lines",
        ChangeKind::Deleted => "Restore Lines",
        ChangeKind::Modified => "Revert Change",
    }
}

/// What the action does, said in words under the lines.
fn explanation(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "Reverting removes these lines; the rest of the script is untouched.",
        ChangeKind::Deleted => "Restoring puts these lines back where they were deleted.",
        ChangeKind::Modified => "Reverting replaces the current lines with the saved ones.",
    }
}

/// Shows the popover for `hunk` (number `position.0` of `position.1`) next to `anchor`.
pub fn show(
    ctx: &egui::Context,
    id: Id,
    anchor: Rect,
    hunk: &DiffHunk,
    position: (usize, usize),
) -> PreviewEvent {
    let mut event = PreviewEvent::None;
    let dismissed = popover::show(ctx, id, anchor, WIDTH, PADDING, true, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.horizontal(|ui| {
            ui.label(kit::text(
                title(hunk),
                kit::size::HEADLINE,
                600.0,
                theme::text_primary(),
            ));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if position.1 > 1 {
                    let step = |ui: &mut Ui, glyph: &str, label: &'static str| {
                        let button = egui::Button::new(RichText::new(glyph).size(13.0));
                        accessibility::label(ui.add(button), label)
                            .on_hover_text(label)
                            .clicked()
                    };
                    if step(ui, "›", "Next Change") {
                        event = PreviewEvent::Step(true);
                    }
                    if step(ui, "‹", "Previous Change") {
                        event = PreviewEvent::Step(false);
                    }
                    ui.label(kit::text(
                        format!("{} of {}", position.0 + 1, position.1),
                        kit::size::SUBHEADLINE,
                        400.0,
                        theme::text_secondary(),
                    ));
                }
            });
        });
        egui::ScrollArea::both()
            .max_height(MAX_CODE_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                if hunk.kind != ChangeKind::Added {
                    block(
                        ui,
                        "SAVED",
                        &hunk.old_text,
                        hunk.old_lines.start,
                        '−',
                        theme::editor_diff_deleted_background(),
                    );
                }
                if hunk.kind != ChangeKind::Deleted {
                    block(
                        ui,
                        "CURRENT",
                        &hunk.new_text,
                        hunk.new_lines.start,
                        '+',
                        theme::editor_diff_added_background(),
                    );
                }
            });
        ui.label(kit::text(
            explanation(hunk.kind),
            kit::size::SUBHEADLINE,
            400.0,
            theme::text_secondary(),
        ));
        ui.horizontal(|ui| {
            if kit::push_button(ui, Some(Icon::Undo), action_label(hunk.kind), true).clicked() {
                event = PreviewEvent::Apply;
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if kit::push_button(ui, None, "Close", true).clicked() {
                    event = PreviewEvent::Dismiss;
                }
            });
        });
    });
    if dismissed && event == PreviewEvent::None {
        event = PreviewEvent::Dismiss;
    }
    event
}

/// One side of the comparison: a heading, then numbered, syntax-coloured lines on a tint. The
/// sign sits in the number column, so the side reads without the colour.
fn block(ui: &mut Ui, heading: &str, text: &str, first_line: usize, sign: char, fill: Color32) {
    ui.label(kit::text(
        heading,
        kit::size::CAPTION,
        600.0,
        theme::text_secondary(),
    ));
    // Shown without line terminators; the lines are counted from the text, not the display.
    let display = text.replace("\r\n", "\n");
    let display = display.strip_suffix('\n').unwrap_or(&display);
    let count = display.split('\n').count();
    let numbers: Vec<String> = (0..count)
        .map(|offset| format!("{sign} {:>4}", first_line + offset + 1))
        .collect();
    let font = editor_appearance::editor_font().at(ui.ctx(), CODE_SIZE);
    let spans = syntax::classify(display, None);
    let look = editor_appearance::chrome();
    let mut job = syntax::layout_job(
        display,
        &spans,
        font.clone(),
        None,
        &look,
        theme::text_primary(),
    );
    job.wrap.max_width = f32::INFINITY;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(numbers.join("\n"))
                            .font(font)
                            .color(theme::text_secondary()),
                    )
                    .selectable(false),
                );
                ui.add(egui::Label::new(galley));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_manager::diff::DiffSnapshot;

    fn hunk(saved: &str, current: &str) -> DiffHunk {
        DiffSnapshot::compute(saved, current).hunks()[0].clone()
    }

    #[test]
    fn titles_use_one_based_lines() {
        assert_eq!(title(&hunk("a\n", "a\nb\nc\n")), "Added lines 2–3");
        assert_eq!(title(&hunk("a\nb\n", "a\nx\n")), "Modified line 2");
        assert_eq!(title(&hunk("a\nb\nc\n", "a\nc\n")), "Deleted after line 1");
        assert_eq!(title(&hunk("a\nb\n", "b\n")), "Deleted at the start");
    }

    #[test]
    fn the_action_is_named_for_the_change() {
        assert_eq!(action_label(ChangeKind::Added), "Revert Added Lines");
        assert_eq!(action_label(ChangeKind::Deleted), "Restore Lines");
        assert_eq!(action_label(ChangeKind::Modified), "Revert Change");
    }
}
