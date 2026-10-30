//! Settings ▸ Appearance: the app theme, picked from small previews of a window in each theme.

use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme::{self, AppTheme};
use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2,
};

const PREVIEW_HEIGHT: f32 = 92.0;
const TILE_GAP: f32 = 12.0;

/// The theme tiles. Returns the theme picked this frame.
pub fn body(ui: &mut egui::Ui, current: AppTheme) -> Option<AppTheme> {
    let mut picked = None;
    super::settings_window::section(
        ui,
        "Theme",
        Some(&format!(
            "System follows your {}'s light or dark setting.",
            crate::platform::computer()
        )),
        |ui| {
            let width = ui.available_width();
            let tile_width = (width - TILE_GAP * 2.0) / 3.0;
            let (row, _) = ui.allocate_exact_size(
                Vec2::new(width, 10.0 + PREVIEW_HEIGHT + 10.0 + 18.0 + 10.0),
                Sense::hover(),
            );
            for (index, theme) in AppTheme::ALL.into_iter().enumerate() {
                let rect = Rect::from_min_size(
                    pos2(
                        row.left() + index as f32 * (tile_width + TILE_GAP),
                        row.top(),
                    ),
                    Vec2::new(tile_width, row.height()),
                );
                if tile(ui, rect, theme, theme == current) {
                    picked = Some(theme);
                }
            }
        },
    );
    picked.filter(|theme| *theme != current)
}

fn icon(theme: AppTheme) -> Icon {
    match theme {
        AppTheme::System => Icon::CircleLeftHalfFill,
        AppTheme::Light => Icon::SunMaxFill,
        AppTheme::Dark => Icon::MoonFill,
    }
}

/// One theme as a miniature window in that theme (System shows both halves), with its name.
fn tile(ui: &mut egui::Ui, rect: Rect, theme: AppTheme, selected: bool) -> bool {
    let response = ui
        .interact(
            rect,
            egui::Id::new(("appearance-theme", theme.title())),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label_as(
        response,
        egui::WidgetType::RadioButton,
        format!("{} theme", theme.title()),
    );
    let painter = ui.painter();
    let fill = if selected {
        theme::selected_tint(false)
    } else if response.hovered() {
        kit::primary(0.07)
    } else {
        kit::primary(0.03)
    };
    painter.rect_filled(rect, 12.0, fill);
    painter.rect_stroke(
        rect,
        12.0,
        if selected {
            theme::selected_stroke()
        } else {
            Stroke::new(1.0, kit::primary(0.12))
        },
        StrokeKind::Inside,
    );

    let preview = Rect::from_min_size(
        rect.min + Vec2::splat(10.0),
        Vec2::new(rect.width() - 20.0, PREVIEW_HEIGHT),
    );
    match theme {
        AppTheme::Light => mini_window(painter, preview, false, CornerRadius::same(8)),
        AppTheme::Dark => mini_window(painter, preview, true, CornerRadius::same(8)),
        AppTheme::System => {
            let (left, right) = preview.split_left_right_at_fraction(0.5);
            mini_window(painter, left, false, corners(8, true, false));
            mini_window(painter, right, true, corners(8, false, true));
        }
    }
    painter.rect_stroke(
        preview,
        8.0,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );

    let label_y = preview.bottom() + 10.0 + 9.0;
    let tint = if selected {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    icons::paint(
        painter,
        Rect::from_center_size(pos2(preview.left() + 7.0, label_y), Vec2::splat(13.0)),
        icon(theme),
        tint,
    );
    let title = kit::galley(
        ui,
        theme.title(),
        kit::size::SUBHEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
        preview.width() - 44.0,
    );
    painter.galley(
        pos2(preview.left() + 20.0, label_y - title.size().y / 2.0),
        title,
        Color32::PLACEHOLDER,
    );
    let check = Rect::from_center_size(pos2(preview.right() - 7.0, label_y), Vec2::splat(14.0));
    if selected {
        icons::paint(painter, check, Icon::CheckCircleFill, theme::text_primary());
    } else {
        icons::paint(
            painter,
            check,
            Icon::Circle,
            theme::text_secondary().gamma_multiply(0.6),
        );
    }
    response.clicked()
}

fn corners(radius: u8, left: bool, right: bool) -> CornerRadius {
    let left = if left { radius } else { 0 };
    let right = if right { radius } else { 0 };
    CornerRadius {
        nw: left,
        sw: left,
        ne: right,
        se: right,
    }
}

/// A stylised window in fixed light or dark colours: title bar with traffic lights, a rising
/// line chart on a card and two cards under it.
fn mini_window(painter: &egui::Painter, rect: Rect, dark: bool, radius: CornerRadius) {
    let gray = |white: f32| {
        let v = (white * 255.0).round() as u8;
        Color32::from_rgb(v, v, v)
    };
    let background = if dark { gray(0.13) } else { gray(0.96) };
    let bar = if dark { gray(0.2) } else { gray(0.88) };
    let card = if dark { gray(0.2) } else { Color32::WHITE };

    painter.rect_filled(rect, radius, background);
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(rect.width(), 14.0)),
        CornerRadius {
            nw: radius.nw,
            ne: radius.ne,
            sw: 0,
            se: 0,
        },
        bar,
    );
    for (index, color) in [
        Color32::from_rgb(255, 95, 87),
        Color32::from_rgb(254, 188, 46),
        Color32::from_rgb(40, 200, 64),
    ]
    .into_iter()
    .enumerate()
    {
        painter.circle_filled(
            pos2(
                rect.left() + 6.0 + 2.5 + index as f32 * 8.0,
                rect.top() + 7.0,
            ),
            2.5,
            color.gamma_multiply(0.85),
        );
    }

    let width = rect.width() - 16.0;
    let chart = Rect::from_min_size(rect.min + Vec2::new(8.0, 22.0), Vec2::new(width, 34.0));
    painter.rect_filled(chart, 4.0, card);
    let at = |x: f32, y: f32| Pos2::new(chart.left() + x, chart.top() + y);
    painter.add(egui::Shape::line(
        vec![
            at(4.0, 28.0),
            at(width * 0.3, 20.0),
            at(width * 0.5, 24.0),
            at(width * 0.75, 10.0),
            at(width - 4.0, 14.0),
        ],
        Stroke::new(1.5, Color32::from_rgb(52, 199, 89)),
    ));

    let half = (width - 4.0) / 2.0;
    for index in 0..2 {
        painter.rect_filled(
            Rect::from_min_size(
                rect.min + Vec2::new(8.0 + index as f32 * (half + 4.0), 62.0),
                Vec2::new(half, 22.0),
            ),
            3.0,
            card,
        );
    }
}
