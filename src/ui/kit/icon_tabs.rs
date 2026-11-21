//! The section switcher (Swift `IconTabBar`, compact): a pill track whose segments hug an icon and a
//! title, the selected one raised on a darker plate. Used by the asset picker and the new-tab
//! page.

use super::outline::{self, SELECTED_WEIGHT};
use super::{MEDIUM, galley};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, Galley, Rect, Sense, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

/// The pill-track section switcher over any set of `(value, title, icon)` segments.
pub fn icon_tabs<T: Copy + PartialEq>(
    ui: &mut Ui,
    items: &[(T, &str, Icon)],
    selected: T,
) -> Option<T> {
    let segments: Vec<(T, &str, Icon, Arc<Galley>, f32)> = items
        .iter()
        .map(|&(tab, title, icon)| {
            let weight = if tab == selected {
                SELECTED_WEIGHT
            } else {
                MEDIUM
            };
            let color = if tab == selected {
                theme::text_primary()
            } else {
                theme::text_secondary()
            };
            let label = galley(ui, title, 12.5, weight, color, f32::INFINITY);
            let width = 11.0 + 13.0 + 5.0 + label.size().x + 11.0;
            (tab, title, icon, label, width)
        })
        .collect();
    let segment_height = 28.0;
    let track_width = segments.iter().map(|segment| segment.4).sum::<f32>()
        + 2.0 * (segments.len() - 1) as f32
        + 6.0;
    let (row, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), segment_height + 6.0),
        Sense::hover(),
    );
    let track = Rect::from_center_size(row.center(), vec2(track_width, segment_height + 6.0));
    let painter = ui.painter();
    outline::paint_track(painter, track, outline::RADIUS, outline::Wash::Rest);

    let mut clicked = None;
    let mut x = track.left() + 3.0;
    for (tab, title, icon, label, width) in segments {
        let segment = Rect::from_min_size(pos2(x, track.top() + 3.0), vec2(width, segment_height));
        let response = ui
            .interact(
                segment,
                ui.id().with(("segmented_tab", title)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, title);
        let active = tab == selected;
        if active {
            outline::paint_plate(painter, segment, outline::PLATE_RADIUS, false);
        } else if response.hovered() {
            painter.rect_filled(segment, outline::PLATE_RADIUS, theme::ink(13));
        }
        let icon_rect = Rect::from_min_size(
            pos2(segment.left() + 11.0, segment.center().y - 6.5),
            Vec2::splat(13.0),
        );
        icons::paint(
            painter,
            icon_rect,
            icon,
            if active {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
        painter.galley(
            pos2(
                icon_rect.right() + 5.0,
                segment.center().y - label.size().y / 2.0,
            ),
            label,
            theme::text_primary(),
        );
        if response.clicked() {
            clicked = Some(tab);
        }
        x += width + 2.0;
    }
    clicked
}
