//! Page furniture: the empty state, the footer bar and the tinted notice card.

use super::{BOLD, REGULAR, SEMIBOLD, galley, push_button, size, text, window};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, FontId, Rect, Sense, Stroke, Ui, Vec2, pos2, text::LayoutJob, vec2,
};

/// `ContentUnavailableView`: a large glyph, a title and a description, centred in `rect`.
pub fn empty_state(ui: &Ui, rect: Rect, icon: Icon, title: &str, detail: &str) {
    let painter = ui.painter();
    let width = (rect.width() - 60.0).max(80.0);
    let title = galley(ui, title, size::TITLE2, BOLD, theme::text_primary(), width);
    let mut job = LayoutJob::default();
    job.append(
        detail,
        0.0,
        egui::TextFormat {
            font_id: FontId::proportional(size::BODY),
            color: theme::text_secondary(),
            ..Default::default()
        },
    );
    job.wrap.max_width = width.min(380.0);
    job.halign = egui::Align::Center;
    let detail = painter.layout_job(job);
    let total = 38.0 + 14.0 + title.size().y + 6.0 + detail.size().y;
    let mut y = rect.center().y - total / 2.0;
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.center().x, y + 19.0), Vec2::splat(38.0)),
        icon,
        theme::text_secondary(),
    );
    y += 52.0;
    painter.galley(
        pos2(rect.center().x - title.size().x / 2.0, y),
        title.clone(),
        theme::text_primary(),
    );
    y += title.size().y + 6.0;
    painter.galley(pos2(rect.center().x, y), detail, theme::text_secondary());
}

/// `AlertFooterBar`: a hairline, then a note on the left and the page's action on the right.
/// Returns whether the action was clicked.
pub fn footer_bar(
    ui: &mut Ui,
    icon: Icon,
    note: &str,
    action_icon: Icon,
    action: &str,
    enabled: bool,
    tooltip: &str,
) -> bool {
    footer_bar_with(ui, icon, note, Some(action_icon), action, enabled, tooltip)
}

/// `footer_bar` whose action may go without a glyph, like the Paper Trading panel's Close All.
pub fn footer_bar_with(
    ui: &mut Ui,
    icon: Icon,
    note: &str,
    action_icon: Option<Icon>,
    action: &str,
    enabled: bool,
    tooltip: &str,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 45.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, window());
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        Stroke::new(1.0, theme::separator()),
    );
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(20.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let clicked = push_button(&mut row, action_icon, action, enabled)
        .on_hover_text(tooltip)
        .clicked();
    let note_rect = Rect::from_min_max(
        pos2(rect.left() + 20.0, rect.top()),
        pos2(row.min_rect().left() - 12.0, rect.bottom()),
    );
    icons::paint(
        ui.painter(),
        Rect::from_min_size(
            pos2(note_rect.left(), rect.center().y - 5.5),
            Vec2::splat(11.0),
        ),
        icon,
        theme::text_secondary(),
    );
    let note = galley(
        ui,
        note,
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
        note_rect.width() - 18.0,
    );
    ui.painter().galley(
        pos2(
            note_rect.left() + 17.0,
            rect.center().y - note.size().y / 2.0,
        ),
        note,
        theme::text_secondary(),
    );
    clicked
}

/// A tinted callout: a glyph, a semibold title, a detail line and an optional trailing button.
/// Returns whether the button was clicked.
pub fn notice_card(
    ui: &mut Ui,
    icon: Icon,
    tint: Color32,
    title: &str,
    detail: Option<&str>,
    action: Option<&str>,
) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.08))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::with_alpha(tint, 0.25)))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::inline(ui, icon, 14.0, tint);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(text(
                        title,
                        size::SUBHEADLINE + 1.0,
                        SEMIBOLD,
                        theme::text_primary(),
                    ));
                    if let Some(detail) = detail {
                        ui.add(
                            egui::Label::new(text(
                                detail,
                                size::CAPTION + 0.5,
                                REGULAR,
                                theme::text_secondary(),
                            ))
                            .wrap(),
                        );
                    }
                });
                if let Some(action) = action {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clicked = push_button(ui, None, action, true).clicked();
                    });
                }
            });
        });
    clicked
}
