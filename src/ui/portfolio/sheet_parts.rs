//! Parts the portfolio sheets share: the card, a pill, the status dot and a tinted problem list.
//! Painter code only, so each sheet keeps its own layout and its rows at fixed heights.

use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

/// A card's fill and outline, shared by the import sheets' bands, rows and cards.
pub(crate) fn card_background(ui: &Ui, rect: Rect, outline: Option<Color32>) {
    ui.painter().rect_filled(rect, 12.0, theme::quaternary(0.4));
    ui.painter().rect_stroke(
        rect,
        12.0,
        Stroke::new(
            1.0,
            outline.unwrap_or_else(|| theme::separator().gamma_multiply(0.6)),
        ),
        StrokeKind::Inside,
    );
}

/// A tinted callout: a glyph, a title, an optional detail line and `lines` as bullets in a
/// scroll area at most `max_height` tall. The import sheets' errors and warnings.
pub(crate) fn problem_card(
    ui: &mut Ui,
    icon: Icon,
    tint: Color32,
    title: &str,
    detail: Option<&str>,
    lines: &[String],
    max_height: f32,
) {
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.08))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::with_alpha(tint, 0.25)))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::inline(ui, icon, 14.0, tint);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(kit::text(
                        title,
                        kit::size::SUBHEADLINE + 1.0,
                        kit::SEMIBOLD,
                        theme::text_primary(),
                    ));
                    if let Some(detail) = detail {
                        ui.add(
                            egui::Label::new(kit::text(
                                detail,
                                kit::size::CAPTION + 0.5,
                                kit::REGULAR,
                                theme::text_secondary(),
                            ))
                            .wrap(),
                        );
                    }
                    if !lines.is_empty() {
                        egui::ScrollArea::vertical()
                            .id_salt(title)
                            .max_height(max_height)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for line in lines {
                                    ui.add(
                                        egui::Label::new(kit::text(
                                            format!("\u{2022} {line}"),
                                            kit::size::CAPTION + 0.5,
                                            kit::REGULAR,
                                            theme::text_secondary(),
                                        ))
                                        .wrap(),
                                    );
                                }
                            });
                    }
                });
            });
        });
}

/// A dot and a label, for a footer's status line.
pub(crate) fn status_dot(ui: &mut Ui, color: Color32, label: &str, weight: f32, text: Color32) {
    let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(dot.center(), 3.5, color);
    ui.label(kit::text(label, kit::size::BODY, weight, text));
}

/// A small rounded label at `left`, centred on `middle`; returns its right edge.
pub(crate) fn pill(ui: &Ui, left: f32, middle: f32, label: &str, tint: Color32) -> f32 {
    let text = kit::galley(ui, label, kit::size::CAPTION, kit::SEMIBOLD, tint, 200.0);
    let rect = Rect::from_min_size(pos2(left, middle - 9.0), vec2(text.size().x + 14.0, 18.0));
    ui.painter()
        .rect_filled(rect, 9.0, tint.gamma_multiply(0.14));
    ui.painter().galley(
        pos2(rect.left() + 7.0, rect.center().y - text.size().y / 2.0),
        text,
        Color32::PLACEHOLDER,
    );
    rect.right()
}
