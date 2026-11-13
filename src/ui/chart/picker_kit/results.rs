//! Result rows and the grouped result well the portfolio pickers list markets in.

use super::artwork::result_icon;
use super::text::{SEMIBOLD, galley, size, text};
use crate::model::DataSource;
use crate::service::SearchResult;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit::source_logo, theme};
use eframe::egui::{self, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

const RESULT_ROW_HEIGHT: f32 = 38.0;

const RESULT_COUNT_HEIGHT: f32 = 18.0;

const RESULT_COUNT_MIN_WIDTH: f32 = 30.0;

pub(crate) fn same_market(a: &SearchResult, b: &SearchResult) -> bool {
    a.source == b.source && a.full_symbol == b.full_symbol
}

/// One search hit: logo (with the data source badged on its corner when `shows_source`), symbol
/// and price, and a check when selected.
pub(crate) fn result_row(
    ui: &mut Ui,
    result: &SearchResult,
    selected: bool,
    shows_source: bool,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, RESULT_ROW_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, &result.symbol);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let fill = if selected {
        theme::selected_tint(false)
    } else if response.hovered() {
        theme::ink(15)
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, 8.0, fill);
    if selected {
        painter.rect_stroke(rect, 8.0, theme::selected_stroke(), StrokeKind::Inside);
    }
    let content = rect.shrink2(vec2(10.0, 5.0));
    let icon = Rect::from_min_size(
        pos2(content.left(), rect.center().y - 14.0),
        Vec2::splat(28.0),
    );
    result_icon(ui, icon, result);
    if shows_source {
        crate::ui::kit::source_badge(ui, icon, result.source);
    }

    let mut right = content.right();
    let check = Rect::from_center_size(pos2(right - 7.0, rect.center().y), Vec2::splat(14.0));
    if selected {
        icons::paint(painter, check, Icon::CheckCircleFill, theme::text_primary());
    }
    right = check.left() - 10.0;
    if let Some(price) = result.price {
        let label = painter.layout_no_wrap(
            format!("${}", crate::format::price(price)),
            FontId::monospace(size::CALLOUT - 0.5),
            theme::text_secondary(),
        );
        right -= label.size().x;
        painter.galley(
            pos2(right, rect.center().y - label.size().y / 2.0),
            label,
            theme::text_secondary(),
        );
        right -= 10.0;
    }
    let left = icon.right() + 10.0;
    let symbol = galley(
        ui,
        &result.symbol,
        size::BODY,
        SEMIBOLD,
        theme::text_primary(),
        right - left,
    );
    painter.galley(
        pos2(left, rect.center().y - symbol.size().y / 2.0),
        symbol,
        theme::text_primary(),
    );
    let response = if shows_source {
        response.on_hover_text(result.source.display_name())
    } else {
        response
    };
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The grouped result well over any `(source, hits)` sections.
pub(crate) fn source_groups(
    ui: &mut Ui,
    results: &[(DataSource, Vec<SearchResult>)],
    selected: &mut Option<SearchResult>,
) -> Option<SearchResult> {
    let mut commit = None;
    let height = ui.available_height();
    egui::Frame::new()
        .fill(theme::quaternary(0.25))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::ink(18)))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(height - 10.0)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let width = ui.available_width();
                    let sections = results.len();
                    for (index, (source, list)) in results.iter().enumerate() {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            ui.spacing_mut().item_spacing.x = 6.0;
                            source_logo::show(ui, *source, 14.0);
                            ui.label(text(
                                source.display_name(),
                                size::CAPTION + 0.5,
                                SEMIBOLD,
                                theme::text_secondary(),
                            ));
                            result_count_badge(ui, list.len());
                        });
                        ui.add_space(4.0);
                        for result in list {
                            let is_selected = selected
                                .as_ref()
                                .is_some_and(|picked| same_market(picked, result));
                            let response = result_row(ui, result, is_selected, false, width);
                            ui.add_space(2.0);
                            if response.double_clicked() {
                                commit = Some(result.clone());
                            } else if response.clicked() {
                                *selected = Some(result.clone());
                            }
                        }
                        if index + 1 < sections {
                            ui.add_space(4.0);
                            let (line, _) =
                                ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
                            ui.painter().hline(
                                line.shrink2(vec2(4.0, 0.0)).x_range(),
                                line.center().y,
                                Stroke::new(1.0, theme::separator()),
                            );
                        }
                    }
                    ui.add_space(6.0);
                });
        });
    commit
}

/// A compact, deliberately wide capsule for a provider's result count.
fn result_count_badge(ui: &mut Ui, count: usize) {
    let label = galley(
        ui,
        &count.to_string(),
        size::CAPTION,
        SEMIBOLD,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let width = result_count_width(label.size().x);
    let (rect, _) = ui.allocate_exact_size(vec2(width, RESULT_COUNT_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, RESULT_COUNT_HEIGHT / 2.0, theme::quaternary(0.9));
    painter.rect_stroke(
        rect,
        RESULT_COUNT_HEIGHT / 2.0,
        Stroke::new(1.0, theme::ink(20)),
        StrokeKind::Inside,
    );
    painter.galley(
        rect.center() - label.size() / 2.0,
        label,
        theme::text_secondary(),
    );
}

fn result_count_width(label_width: f32) -> f32 {
    (label_width + 16.0).max(RESULT_COUNT_MIN_WIDTH)
}
