//! The review sheet over an assistant proposal: every change in a list on the left, and the whole
//! file's diff on the right, with line numbers, syntax colour, the changed words marked, and
//! unchanged stretches folded. Each change can be accepted or rejected; only accepted ones are
//! applied. A view only: it reads the [`Review`] and reports what was pressed; `ai_host` acts on it.

use super::ai::Proposal;
use super::ai_review::{Review, Row, header_row};
use super::diff::{ChangeKind, DiffHunk};
use super::diff_view;
use super::syntax;
use crate::ui::icons::{self, Icon};
use crate::ui::{editor_appearance, kit, sheet, theme};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Key, Layout, Rect, Sense, Stroke, Ui, UiBuilder,
    epaint::text::LayoutJob, pos2, vec2,
};

const ROW_HEIGHT: f32 = 20.0;
const NUMBER_WIDTH: f32 = 44.0;
const SIGN_WIDTH: f32 = 18.0;
const RAIL_WIDTH: f32 = 264.0;
const RAIL_ITEM_HEIGHT: f32 = 50.0;
const HEADER_HEIGHT: f32 = 64.0;
const FOOTER_HEIGHT: f32 = 60.0;
const CODE_SIZE: f32 = 12.0;
/// Narrower than this the list of changes is dropped and the diff takes the whole sheet.
const RAIL_MIN_SHEET_WIDTH: f32 = 760.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewAction {
    Toggle(usize),
    Select(usize),
    Step(bool),
    SetAll(bool),
    /// Open the fold that starts at this old line.
    Expand(usize),
    Apply,
    DiscardAll,
    Close,
}

/// How large the sheet is for a window of `screen`.
pub fn sheet_size(screen: egui::Vec2) -> egui::Vec2 {
    vec2(
        (screen.x - 80.0).clamp(520.0, 1040.0),
        (screen.y - 120.0).clamp(300.0, 720.0),
    )
}

pub fn show(ctx: &egui::Context, review: &Review, proposal: &Proposal) -> Vec<ReviewAction> {
    let mut actions = Vec::new();
    let size = sheet_size(ctx.content_rect().size());
    keyboard(ctx, review, &mut actions);
    sheet::show(ctx, Id::new("ai-review-sheet"), size.x, |ui| {
        header(ui, size.x, review, proposal, &mut actions);
        rule(ui);
        let body_height = (size.y - HEADER_HEIGHT - FOOTER_HEIGHT).max(120.0);
        let (_, body) = ui.allocate_space(vec2(size.x, body_height));
        let hunks = proposal.changes.hunks();
        let mut diff_rect = body;
        if size.x >= RAIL_MIN_SHEET_WIDTH {
            let rail_rect = Rect::from_min_size(body.min, vec2(RAIL_WIDTH, body.height()));
            rail(ui, rail_rect, review, hunks, &mut actions);
            ui.painter().vline(
                rail_rect.right(),
                body.y_range(),
                Stroke::new(1.0, kit::hairline()),
            );
            diff_rect.min.x = rail_rect.right() + 1.0;
        }
        diff_pane(ui, diff_rect, review, hunks, &mut actions);
        rule(ui);
        footer(ui, size.x, review, hunks.len(), &mut actions);
    });
    actions
}

fn keyboard(ctx: &egui::Context, review: &Review, actions: &mut Vec<ReviewAction>) {
    ctx.input(|input| {
        if input.key_pressed(Key::ArrowDown) || input.key_pressed(Key::J) {
            actions.push(ReviewAction::Step(true));
        }
        if input.key_pressed(Key::ArrowUp) || input.key_pressed(Key::K) {
            actions.push(ReviewAction::Step(false));
        }
        if input.key_pressed(Key::Space) {
            actions.push(ReviewAction::Toggle(review.current));
        }
        if input.modifiers.command && input.key_pressed(Key::Enter) {
            actions.push(ReviewAction::Apply);
        }
    });
}

fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, kit::hairline()),
    );
}

fn kind_color(kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added => theme::editor_diff_added(),
        ChangeKind::Deleted => theme::editor_diff_deleted(),
        ChangeKind::Modified => theme::editor_diff_modified(),
    }
}

// MARK: Header and footer

fn header(
    ui: &mut Ui,
    width: f32,
    review: &Review,
    proposal: &Proposal,
    actions: &mut Vec<ReviewAction>,
) {
    let total = proposal.changes.hunks().len();
    let added = proposal.changes.lines_added();
    let removed = proposal.changes.lines_removed();
    ui.allocate_ui_with_layout(
        vec2(width, HEADER_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(22.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                // The block is centred by hand: a vertical layout inside a centred row hugs the top.
                ui.add_space(11.0);
                ui.label(kit::text(
                    "Review changes",
                    kit::size::TITLE2,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(kit::text(
                        format!("{} of {} selected", review.selected(), total),
                        kit::size::SUBHEADLINE + 1.0,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    ui.label(kit::text(
                        format!("+{added}"),
                        kit::size::SUBHEADLINE + 1.0,
                        kit::MEDIUM,
                        kit::GREEN,
                    ));
                    ui.label(kit::text(
                        format!("−{removed}"),
                        kit::size::SUBHEADLINE + 1.0,
                        kit::MEDIUM,
                        kit::RED,
                    ));
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(14.0);
                ui.spacing_mut().item_spacing.x = 2.0;
                let color = theme::text_secondary();
                if icons::button(ui, Icon::Xmark, 28.0, color, "Close (Esc)").clicked() {
                    actions.push(ReviewAction::Close);
                }
                ui.add_space(6.0);
                if total > 1 {
                    if icons::button(ui, Icon::ChevronDown, 28.0, color, "Next change (↓ or J)")
                        .clicked()
                    {
                        actions.push(ReviewAction::Step(true));
                    }
                    if icons::button(ui, Icon::ChevronUp, 28.0, color, "Previous change (↑ or K)")
                        .clicked()
                    {
                        actions.push(ReviewAction::Step(false));
                    }
                    ui.label(kit::text(
                        format!("{} / {}", review.current + 1, total),
                        kit::size::SUBHEADLINE + 1.0,
                        kit::REGULAR,
                        theme::text_tertiary(),
                    ));
                }
            });
        },
    );
}

fn footer(ui: &mut Ui, width: f32, review: &Review, total: usize, actions: &mut Vec<ReviewAction>) {
    let selected = review.selected();
    ui.allocate_ui_with_layout(
        vec2(width, FOOTER_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(22.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            let (icon, color, text) = status(review);
            icons::inline(ui, icon, 13.0, color);
            ui.label(kit::text(text, kit::size::BODY, kit::MEDIUM, color));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(22.0);
                ui.spacing_mut().item_spacing.x = 10.0;
                let label = if selected == total {
                    format!("Apply all {total}")
                } else {
                    format!("Apply {selected} of {total}")
                };
                let label = if review.errors > 0 {
                    format!("{label} anyway")
                } else {
                    label
                };
                if kit::sheet_button(ui, &label, review.errors == 0, selected > 0)
                    .on_hover_text("⌘↩")
                    .clicked()
                {
                    actions.push(ReviewAction::Apply);
                }
                if kit::sheet_button(ui, "Discard all", false, true).clicked() {
                    actions.push(ReviewAction::DiscardAll);
                }
                ui.label(kit::text(
                    "↑↓ move · space toggles",
                    kit::size::CAPTION + 0.5,
                    kit::REGULAR,
                    theme::text_tertiary(),
                ));
            });
        },
    );
}

/// What the current selection compiles to, in words.
fn status(review: &Review) -> (Icon, Color32, String) {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    if review.selected() == 0 {
        (
            Icon::InfoCircle,
            theme::text_secondary(),
            "Nothing selected".to_owned(),
        )
    } else if review.errors > 0 {
        (
            Icon::TriangleFill,
            kit::RED,
            format!(
                "{} with this selection",
                plural(review.errors, "compile error")
            ),
        )
    } else if review.warnings > 0 {
        (
            Icon::TriangleFill,
            Color32::from_rgb(255, 159, 10),
            format!("Compiles · {}", plural(review.warnings, "warning")),
        )
    } else {
        (Icon::CheckCircleFill, kit::GREEN, "Compiles".to_owned())
    }
}

// MARK: The list of changes

fn rail(
    ui: &mut Ui,
    rect: Rect,
    review: &Review,
    hunks: &[DiffHunk],
    actions: &mut Vec<ReviewAction>,
) {
    let mut column = ui.new_child(UiBuilder::new().max_rect(rect));
    column.set_clip_rect(rect.intersect(ui.clip_rect()));
    column
        .painter()
        .rect_filled(rect, 0.0, theme::with_alpha(theme::text_primary(), 0.025));
    let strip = Rect::from_min_size(rect.min, vec2(rect.width(), 30.0));
    column.painter().text(
        pos2(strip.left() + 14.0, strip.center().y),
        Align2::LEFT_CENTER,
        "CHANGES",
        FontId::proportional(kit::size::CAPTION),
        theme::text_tertiary(),
    );
    let link = |ui: &mut Ui, id: &str, label: &str, right: f32| -> bool {
        let galley = kit::galley(
            ui,
            label,
            kit::size::SUBHEADLINE,
            kit::MEDIUM,
            kit::ACCENT,
            f32::INFINITY,
        );
        let at = Rect::from_min_size(
            pos2(
                right - galley.size().x,
                strip.center().y - galley.size().y / 2.0,
            ),
            galley.size(),
        );
        let response = ui.interact(
            at.expand(4.0),
            Id::new(("ai-review-link", id)),
            Sense::click(),
        );
        ui.painter().galley(at.min, galley, kit::ACCENT);
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    };
    if link(&mut column, "none", "None", strip.right() - 14.0) {
        actions.push(ReviewAction::SetAll(false));
    }
    if link(&mut column, "all", "All", strip.right() - 56.0) {
        actions.push(ReviewAction::SetAll(true));
    }
    let list = Rect::from_min_max(pos2(rect.left(), strip.bottom()), rect.max);
    let mut list_ui = column.new_child(UiBuilder::new().max_rect(list));
    list_ui.set_clip_rect(list.intersect(column.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("ai-review-rail")
        .auto_shrink([false, false])
        .show(&mut list_ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, hunk) in hunks.iter().enumerate() {
                rail_item(ui, index, hunk, review, actions);
            }
        });
}

fn rail_item(
    ui: &mut Ui,
    index: usize,
    hunk: &DiffHunk,
    review: &Review,
    actions: &mut Vec<ReviewAction>,
) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), RAIL_ITEM_HEIGHT), Sense::click());
    let current = index == review.current;
    let accepted = review.accepted[index];
    let painter = ui.painter();
    if current {
        painter.rect_filled(rect, 0.0, theme::with_alpha(kit::ACCENT, 0.13));
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            0.0,
            kit::ACCENT,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, theme::with_alpha(theme::text_primary(), 0.05));
    }
    // The accept box.
    let check = Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), vec2(16.0, 16.0));
    painter.rect(
        check,
        4.0,
        if accepted {
            kit::ACCENT
        } else {
            Color32::TRANSPARENT
        },
        Stroke::new(
            1.2,
            if accepted {
                kit::ACCENT
            } else {
                theme::text_tertiary()
            },
        ),
        egui::StrokeKind::Inside,
    );
    if accepted {
        icons::paint(
            painter,
            Rect::from_center_size(check.center(), vec2(11.0, 11.0)),
            Icon::Check,
            Color32::WHITE,
        );
    }
    let dim = if accepted { 1.0 } else { 0.5 };
    painter.circle_filled(
        pos2(rect.left() + 46.0, rect.top() + 18.0),
        3.5,
        theme::with_alpha(kind_color(hunk.kind), dim),
    );
    painter.text(
        pos2(rect.left() + 58.0, rect.top() + 18.0),
        Align2::LEFT_CENTER,
        diff_view::title(hunk),
        FontId::proportional(12.5),
        if accepted {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        },
    );
    let preview_source = if hunk.kind == ChangeKind::Deleted {
        &hunk.old_text
    } else {
        &hunk.new_text
    };
    let preview = preview_source
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut job = LayoutJob::default();
    job.append(
        preview,
        0.0,
        egui::TextFormat::simple(FontId::monospace(11.0), theme::text_tertiary()),
    );
    job.wrap.max_width = rect.width() - 72.0;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(
            rect.left() + 58.0,
            rect.top() + 33.0 - galley.size().y / 2.0,
        ),
        galley,
        theme::text_tertiary(),
    );
    painter.hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(1.0, kit::hairline()),
    );
    let toggle = ui.interact(
        check.expand(8.0),
        Id::new(("ai-review-check", index)),
        Sense::click(),
    );
    if toggle.clicked() {
        actions.push(ReviewAction::Toggle(index));
    } else if response.clicked() {
        actions.push(ReviewAction::Select(index));
    }
}

// MARK: The diff

fn diff_pane(
    ui: &mut Ui,
    rect: Rect,
    review: &Review,
    hunks: &[DiffHunk],
    actions: &mut Vec<ReviewAction>,
) {
    let mut pane = ui.new_child(UiBuilder::new().max_rect(rect));
    pane.set_clip_rect(rect.intersect(ui.clip_rect()));
    let font = editor_appearance::editor_font().at(ui.ctx(), CODE_SIZE);
    let char_width = pane.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
    let rows = &review.rows;
    let code_left = NUMBER_WIDTH * 2.0 + SIGN_WIDTH;
    let content_width = (code_left + review.max_chars as f32 * char_width + 40.0).max(rect.width());
    let mut area = egui::ScrollArea::both()
        .id_salt("ai-review-diff")
        .auto_shrink([false, false]);
    if review.scroll_to_current
        && let Some(row) = header_row(rows, review.current)
    {
        area = area.vertical_scroll_offset((row as f32 * ROW_HEIGHT - ROW_HEIGHT).max(0.0));
    }
    area.show_rows(&mut pane, ROW_HEIGHT, rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.set_min_width(content_width);
        for index in range {
            paint_row(
                ui,
                content_width,
                &rows[index],
                review,
                hunks,
                &font,
                char_width,
                actions,
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &mut Ui,
    width: f32,
    row: &Row,
    review: &Review,
    hunks: &[DiffHunk],
    font: &FontId,
    char_width: f32,
    actions: &mut Vec<ReviewAction>,
) {
    let clickable = matches!(row, Row::Header { .. } | Row::Collapsed { .. });
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, ROW_HEIGHT),
        if clickable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let accepted = row.hunk().is_none_or(|hunk| review.accepted[hunk]);
    let dim = if accepted { 1.0 } else { 0.4 };
    let painter = ui.painter().clone();
    let middle = rect.center().y;
    let code_left = rect.left() + NUMBER_WIDTH * 2.0 + SIGN_WIDTH;
    let number = |at: f32, value: usize| {
        painter.text(
            pos2(rect.left() + at - 8.0, middle),
            Align2::RIGHT_CENTER,
            (value + 1).to_string(),
            font.clone(),
            theme::text_tertiary(),
        );
    };
    let look = editor_appearance::chrome();
    let code = |text: &str, color: Color32, colored: bool| {
        let spans = if colored {
            syntax::classify(text, None)
        } else {
            Vec::new()
        };
        let mut job = syntax::layout_job(text, &spans, font.clone(), None, &look, color);
        job.wrap.max_width = f32::INFINITY;
        let galley = painter.layout_job(job);
        let at = pos2(code_left, middle - galley.size().y / 2.0);
        if accepted {
            painter.galley(at, galley, color);
        } else {
            painter.galley_with_override_text_color(at, galley, theme::text_tertiary());
        }
    };
    let words = |text: &str, ranges: &[std::ops::Range<usize>], color: Color32| {
        for range in ranges {
            let before = text[..range.start].chars().count() as f32;
            let length = text[range.clone()].chars().count() as f32;
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(code_left + before * char_width, rect.top() + 2.0),
                    vec2(length * char_width, ROW_HEIGHT - 4.0),
                ),
                3.0,
                theme::with_alpha(color, 0.42 * dim),
            );
        }
    };
    match row {
        Row::Header { hunk } => {
            let change = &hunks[*hunk];
            let current = *hunk == review.current;
            painter.rect_filled(
                rect,
                0.0,
                theme::with_alpha(theme::text_primary(), if current { 0.1 } else { 0.05 }),
            );
            painter.circle_filled(
                pos2(rect.left() + 14.0, middle),
                3.5,
                theme::with_alpha(kind_color(change.kind), dim),
            );
            painter.text(
                pos2(rect.left() + 26.0, middle),
                Align2::LEFT_CENTER,
                format!(
                    "Change {} of {}  ·  {}",
                    hunk + 1,
                    hunks.len(),
                    diff_view::title(change)
                ),
                FontId::proportional(12.0),
                if accepted {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                },
            );
            // The toggle stays at the visible right edge however far the lines scroll.
            let visible = ui.clip_rect();
            let chip = Rect::from_center_size(
                pos2((visible.right() - 76.0).max(rect.left() + 260.0), middle),
                vec2(120.0, 16.0),
            );
            let toggle = ui.interact(chip, Id::new(("ai-review-chip", *hunk)), Sense::click());
            let (label, color) = if accepted {
                ("✓ Accepted", kit::GREEN)
            } else {
                ("✕ Rejected", theme::text_secondary())
            };
            painter.rect_filled(
                chip,
                8.0,
                theme::with_alpha(color, if toggle.hovered() { 0.26 } else { 0.16 }),
            );
            painter.text(
                chip.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(11.0),
                color,
            );
            let toggle = toggle.on_hover_cursor(egui::CursorIcon::PointingHand);
            if toggle.clicked() {
                actions.push(ReviewAction::Toggle(*hunk));
            } else if response.clicked() {
                actions.push(ReviewAction::Select(*hunk));
            }
        }
        Row::Context { old, new, text } => {
            number(NUMBER_WIDTH, *old);
            number(NUMBER_WIDTH * 2.0, *new);
            code(text, theme::text_primary(), true);
        }
        Row::Removed {
            old,
            text,
            words: changed,
            ..
        } => {
            // Scaled, not replaced: the tint is already translucent.
            painter.rect_filled(
                rect,
                0.0,
                theme::editor_diff_deleted_background().gamma_multiply(dim),
            );
            words(text, changed, theme::editor_diff_deleted());
            number(NUMBER_WIDTH, *old);
            painter.text(
                pos2(rect.left() + NUMBER_WIDTH * 2.0 + 5.0, middle),
                Align2::LEFT_CENTER,
                "−",
                font.clone(),
                theme::with_alpha(kit::RED, dim),
            );
            code(text, theme::text_primary(), true);
        }
        Row::Added {
            new,
            text,
            words: changed,
            ..
        } => {
            painter.rect_filled(
                rect,
                0.0,
                theme::editor_diff_added_background().gamma_multiply(dim),
            );
            words(text, changed, theme::editor_diff_added());
            number(NUMBER_WIDTH * 2.0, *new);
            painter.text(
                pos2(rect.left() + NUMBER_WIDTH * 2.0 + 5.0, middle),
                Align2::LEFT_CENTER,
                "+",
                font.clone(),
                theme::with_alpha(kit::GREEN, dim),
            );
            code(text, theme::text_primary(), true);
        }
        Row::Collapsed { count, key } => {
            let hovered = response.hovered();
            painter.rect_filled(
                rect,
                0.0,
                theme::with_alpha(theme::text_primary(), if hovered { 0.07 } else { 0.03 }),
            );
            let visible = ui.clip_rect();
            let noun = if *count == 1 { "line" } else { "lines" };
            painter.text(
                pos2(visible.center().x.max(rect.left() + 200.0), middle),
                Align2::CENTER_CENTER,
                format!("⋯  {count} unchanged {noun}  ·  click to show"),
                FontId::proportional(11.5),
                if hovered {
                    kit::ACCENT
                } else {
                    theme::text_secondary()
                },
            );
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                actions.push(ReviewAction::Expand(*key));
            }
        }
    }
    // The change being looked at carries a bar down its left edge.
    if row.hunk() == Some(review.current) {
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, ROW_HEIGHT)),
            0.0,
            kit::ACCENT,
        );
    }
}
