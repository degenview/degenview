//! `IconTabBar`: a pill track of segments, each an icon, a title and an optional count.

use super::{MEDIUM, SEMIBOLD, control, digits, galley, hairline, primary, size};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

pub struct TabItem<T> {
    pub value: T,
    pub title: &'static str,
    pub icon: Icon,
    pub count: usize,
}

/// `IconTabBar` (compact, filling the width): a pill track of equal segments, each an icon, a
/// title and an optional count, the selected one raised on a darker plate.
pub fn tab_bar<T: Copy + PartialEq>(ui: &mut Ui, id: Id, items: &[TabItem<T>], selected: &mut T) {
    tab_bar_impl(ui, id, items, selected, true);
}

/// The compact bar at 28 pt tall, so it lines up with other 28 pt controls in a header row.
pub fn tab_bar_slim<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[TabItem<T>],
    selected: &mut T,
) {
    tab_bar_with(ui, id, items, selected, false, Density::Slim);
}

/// The regular `IconTabBar` (14 pt titles, 16 pt of padding) that hugs its titles, as the chart
/// settings sheet draws it; the sheet centres it.
pub fn tab_bar_regular<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[TabItem<T>],
    selected: &mut T,
) {
    tab_bar_with(ui, id, items, selected, false, Density::Regular);
}

/// How wide `tab_bar_regular` is for these items whichever one is selected: every title is
/// measured as the selected (heavier) one, so the width does not change with the tab.
pub fn tab_bar_regular_width<T: Copy + PartialEq>(ui: &Ui, items: &[TabItem<T>]) -> f32 {
    let widest: f32 = items
        .iter()
        .map(|item| segment_width(ui, item, true, Density::Regular))
        .sum();
    widest + 2.0 * items.len().saturating_sub(1) as f32 + 6.0
}

/// The two `IconTabBar` sizes: Swift's `isCompact` flag.
#[derive(Clone, Copy, PartialEq)]
enum Density {
    /// A 28 pt track, the height of the Paper panel's header controls.
    Slim,
    Compact,
    Regular,
}

impl Density {
    fn font(self) -> f32 {
        match self {
            Self::Slim => 12.0,
            Self::Compact => 12.5,
            Self::Regular => 14.0,
        }
    }

    fn glyph(self) -> f32 {
        match self {
            Self::Slim => 11.5,
            Self::Compact => 12.0,
            Self::Regular => 14.0,
        }
    }

    fn pad(self) -> f32 {
        match self {
            Self::Slim => 10.0,
            Self::Compact => 11.0,
            Self::Regular => 16.0,
        }
    }

    fn gap(self) -> f32 {
        match self {
            Self::Slim | Self::Compact => 5.0,
            Self::Regular => 7.0,
        }
    }

    fn height(self) -> f32 {
        match self {
            Self::Slim => 22.0,
            Self::Compact => 28.0,
            Self::Regular => 34.0,
        }
    }

    /// The track's corner radius; the plates inside sit 3 pt in, so theirs is that much less.
    fn track_radius(self) -> f32 {
        match self {
            Self::Slim => 6.0,
            Self::Compact | Self::Regular => 11.0,
        }
    }
}

/// Widths for a bar that fills `available`: equal segments, like Swift's `fillsWidth`, unless
/// one would be too narrow for its title, in which case every segment keeps its natural width
/// and shares what is left equally, so no title is ever cut.
fn fill_widths(natural: &[f32], available: f32) -> Vec<f32> {
    let count = natural.len().max(1) as f32;
    let equal = available / count;
    if natural.iter().all(|width| *width <= equal) {
        return vec![equal; natural.len()];
    }
    let spare = (available - natural.iter().sum::<f32>()).max(0.0) / count;
    natural.iter().map(|width| width + spare).collect()
}

/// How wide a hugging segment is: padding, glyph, title and count capsule.
fn segment_width<T: Copy + PartialEq>(
    ui: &Ui,
    item: &TabItem<T>,
    active: bool,
    density: Density,
) -> f32 {
    let weight = if active { SEMIBOLD } else { MEDIUM };
    let title = galley(
        ui,
        item.title,
        density.font(),
        weight,
        Color32::WHITE,
        f32::INFINITY,
    );
    let count = (item.count > 0).then(|| {
        let text = item.count.to_string();
        digits(
            ui,
            &text,
            size::CAPTION + 0.5,
            SEMIBOLD,
            Color32::WHITE,
            f32::INFINITY,
        )
    });
    let count_width = count.map_or(0.0, |count| 5.0 + count.size().x + 12.0);
    density.pad() + density.glyph() + density.gap() + title.size().x + count_width + density.pad()
}

fn tab_bar_impl<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[TabItem<T>],
    selected: &mut T,
    fills: bool,
) {
    tab_bar_with(ui, id, items, selected, fills, Density::Compact);
}

fn tab_bar_with<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[TabItem<T>],
    selected: &mut T,
    fills: bool,
    density: Density,
) {
    let height = density.height();
    let widths: Vec<f32> = items
        .iter()
        .map(|item| segment_width(ui, item, item.value == *selected, density))
        .collect();
    let track_width = if fills {
        ui.available_width()
    } else {
        widths.iter().sum::<f32>() + 2.0 * (items.len() - 1) as f32 + 6.0
    };
    let (track, _) = ui.allocate_exact_size(vec2(track_width, height + 6.0), Sense::hover());
    ui.painter()
        .rect_filled(track, density.track_radius(), theme::quaternary(0.6));
    ui.painter().rect_stroke(
        track,
        density.track_radius(),
        Stroke::new(1.0, hairline()),
        StrokeKind::Inside,
    );
    let segment_widths = if fills {
        fill_widths(
            &widths,
            track.width() - 6.0 - 2.0 * (items.len() - 1) as f32,
        )
    } else {
        widths.clone()
    };
    let mut left = track.left() + 3.0;
    for (index, item) in items.iter().enumerate() {
        let width = segment_widths[index];
        let segment = Rect::from_min_size(pos2(left, track.top() + 3.0), vec2(width, height));
        let plate = density.track_radius() - 3.0;
        left += width + 2.0;
        let response = ui
            .interact(segment, id.with(index), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, item.title);
        let active = item.value == *selected;
        let painter = ui.painter();
        if active {
            painter.add(
                egui::Shadow {
                    offset: [0, 1],
                    blur: 3,
                    spread: 0,
                    color: Color32::from_black_alpha(40),
                }
                .as_shape(segment, plate),
            );
            painter.rect_filled(segment, plate, control());
            painter.rect_stroke(
                segment,
                plate,
                Stroke::new(1.0, hairline()),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(segment, plate, primary(0.04));
        }
        let title_color = if active {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let title = galley(
            ui,
            item.title,
            density.font(),
            if active { SEMIBOLD } else { MEDIUM },
            title_color,
            // The segment's room less its padding, glyph and gap; the count capsule is measured
            // into the segment, so it never squeezes the title of a hugging bar.
            (width - 2.0 * density.pad() - density.glyph() - density.gap()).max(24.0),
        );
        let count = (item.count > 0).then(|| {
            digits(
                ui,
                &item.count.to_string(),
                size::CAPTION + 0.5,
                SEMIBOLD,
                if active {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                },
                f32::INFINITY,
            )
        });
        let count_width = count
            .as_ref()
            .map_or(0.0, |count| 5.0 + count.size().x + 12.0);
        let glyph = density.glyph();
        let content = glyph + density.gap() + title.size().x + count_width;
        let mut x = segment.center().x - content / 2.0;
        let y = segment.center().y;
        icons::paint(
            painter,
            Rect::from_min_size(pos2(x, y - glyph / 2.0), Vec2::splat(glyph)),
            item.icon,
            if active {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
        x += glyph + density.gap();
        let title_width = title.size().x;
        painter.galley(pos2(x, y - title.size().y / 2.0), title, title_color);
        x += title_width + 5.0;
        if let Some(count) = count {
            let capsule = Rect::from_min_size(
                pos2(x, y - (count.size().y + 2.0) / 2.0),
                vec2(count.size().x + 12.0, count.size().y + 2.0),
            );
            painter.rect_filled(
                capsule,
                capsule.height() / 2.0,
                theme::with_alpha(
                    if active {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    },
                    0.15,
                ),
            );
            painter.galley(
                pos2(capsule.left() + 6.0, y - count.size().y / 2.0),
                count,
                Color32::WHITE,
            );
        }
        if response.clicked() {
            *selected = item.value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filling_bar_is_equal_unless_a_title_would_be_cut() {
        assert_eq!(
            fill_widths(&[60.0, 70.0, 80.0], 300.0),
            [100.0, 100.0, 100.0]
        );
        // 300 / 3 = 100, but the middle title needs 120: natural widths plus an even share.
        let widths = fill_widths(&[60.0, 120.0, 80.0], 300.0);
        for (found, wanted) in widths.iter().zip([73.333, 133.333, 93.333]) {
            assert!((found - wanted).abs() < 0.01);
        }
        assert!((widths.iter().sum::<f32>() - 300.0).abs() < 1e-3);
        // Too little room for anyone: natural widths, the overflow is the caller's.
        assert_eq!(fill_widths(&[60.0, 120.0], 100.0), [60.0, 120.0]);
    }
}
