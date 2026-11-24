//! The controls under the title: Area, Style, Size, Background and Include, each a small muted
//! label over its control, with a hairline between neighbours. Where the window is too narrow the
//! groups that do not fit wrap onto a second line.

use super::{Action, Input, Output, PAD, swatches};
use crate::i18n::tr;
use crate::model::chart::snapshot::studio::{Area, AspectRatio, Include, Style};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, Segment, size};
use crate::ui::theme;
use eframe::egui::{
    self, Align, Id, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};

/// The height of a group: its label, the gap and the control.
const GROUP_HEIGHT: f32 = 13.0 + LABEL_GAP + CONTROL_HEIGHT;
const LABEL_GAP: f32 = 7.0;
const CONTROL_HEIGHT: f32 = kit::outline::HEIGHT;
/// The room around a hairline between two groups, each side.
const GROUP_GAP: f32 = 14.0;
const LINE_GAP: f32 = 14.0;
/// The room above the first line and below the last.
const TOP: f32 = 4.0;
const BOTTOM: f32 = 18.0;
/// One toggle of the Include pill.
const TOGGLE: egui::Vec2 = vec2(30.0, CONTROL_HEIGHT - 2.0 * kit::outline::INSET);
/// The room between neighbouring toggles.
const TOGGLE_GAP: f32 = 2.0;
/// What the three toggles and their gaps take, with the well's inset around them.
const INCLUDE_WIDTH: f32 = 3.0 * TOGGLE.x + 2.0 * TOGGLE_GAP + 2.0 * kit::outline::INSET;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Group {
    Area,
    Style,
    Size,
    Background,
    Include,
}

impl Group {
    const ALL: [Self; 5] = [
        Self::Area,
        Self::Style,
        Self::Size,
        Self::Background,
        Self::Include,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Area => tr!("Area"),
            Self::Style => tr!("Style"),
            Self::Size => tr!("Size"),
            Self::Background => tr!("Background"),
            Self::Include => tr!("Include"),
        }
    }
}

/// Which groups share a line: each takes the next place on the line it is on unless it would
/// pass `room`, then starts the next. Always at least one group a line.
pub(super) fn lines(widths: &[f32], room: f32, gap: f32) -> Vec<Vec<usize>> {
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0;
    for (index, width) in widths.iter().enumerate() {
        match lines.last_mut() {
            Some(line) if used + gap + width <= room => {
                line.push(index);
                used += gap + width;
            }
            _ => {
                lines.push(vec![index]);
                used = *width;
            }
        }
    }
    lines
}

fn area_options(input: &Input<'_>) -> Vec<Segment<'static, Area>> {
    input
        .areas
        .iter()
        .map(|&area| Segment::new(area, area.control_label()).tooltip(area.tooltip()))
        .collect()
}

fn style_options() -> Vec<Segment<'static, Style>> {
    Style::ALL
        .into_iter()
        .map(|style| Segment::new(style, style.label()).tooltip(style.tooltip()))
        .collect()
}

fn size_options(enabled: bool) -> Vec<Segment<'static, AspectRatio>> {
    AspectRatio::ALL
        .into_iter()
        .map(|size| {
            Segment::new(size, size.label()).tooltip(if enabled {
                size.tooltip()
            } else {
                tr!("Size applies to the Styled look")
            })
        })
        .collect()
}

/// The Size control shows what is applied: the capture's own proportions in the Chart style.
fn size_shown(input: &Input<'_>) -> AspectRatio {
    input.config.effective_size()
}

fn control_width(ui: &Ui, group: Group, input: &Input<'_>) -> f32 {
    match group {
        Group::Area => kit::segments_width(ui, &area_options(input)),
        Group::Style => kit::segments_width(ui, &style_options()),
        Group::Size => kit::segments_width(ui, &size_options(input.config.size_enabled())),
        Group::Background => swatches::WIDTH,
        Group::Include => INCLUDE_WIDTH,
    }
}

fn group_width(ui: &Ui, group: Group, input: &Input<'_>) -> f32 {
    let label = kit::galley(
        ui,
        group.label(),
        size::SUBHEADLINE,
        REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    control_width(ui, group, input).max(label.size().x)
}

/// Draws the groups, wrapped to the window, and the hairline under them.
pub(super) fn show(ui: &mut Ui, input: &Input<'_>, width: f32, output: &mut Output) {
    let widths: Vec<f32> = Group::ALL
        .iter()
        .map(|group| group_width(ui, *group, input))
        .collect();
    let rows = lines(&widths, width - 2.0 * PAD, 2.0 * GROUP_GAP + 1.0);
    let height =
        TOP + rows.len() as f32 * GROUP_HEIGHT + (rows.len() - 1) as f32 * LINE_GAP + BOTTOM;
    let (band, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    for (row, line) in rows.iter().enumerate() {
        let y = band.top() + TOP + row as f32 * (GROUP_HEIGHT + LINE_GAP);
        let mut x = band.left() + PAD;
        for (place, &index) in line.iter().enumerate() {
            if place > 0 {
                let at = x - GROUP_GAP - 0.5;
                ui.painter().vline(
                    at,
                    y..=(y + GROUP_HEIGHT),
                    Stroke::new(1.0, theme::separator()),
                );
            }
            let rect = Rect::from_min_size(pos2(x, y), vec2(widths[index], GROUP_HEIGHT));
            let group = Group::ALL[index];
            // A child that is not allocated: the band is already, and a scope would move the
            // cursor back to its own bottom.
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            draw(&mut child, group, input, output);
            x += widths[index] + 2.0 * GROUP_GAP + 1.0;
        }
    }
    ui.painter().hline(
        band.x_range(),
        band.bottom(),
        Stroke::new(1.0, theme::separator()),
    );
}

fn draw(ui: &mut Ui, group: Group, input: &Input<'_>, output: &mut Output) {
    ui.spacing_mut().item_spacing = vec2(0.0, LABEL_GAP);
    ui.label(kit::text(
        group.label(),
        size::SUBHEADLINE,
        REGULAR,
        theme::text_secondary(),
    ));
    let config = input.config;
    match group {
        Group::Area => {
            let id = Id::new("studio-area");
            if let Some(area) = kit::segments(ui, id, &area_options(input), config.area, true)
                && (area != config.area || area == Area::Portion)
            {
                output.actions.push(Action::SetArea(area));
            }
        }
        Group::Style => {
            let id = Id::new("studio-style");
            if let Some(style) = kit::segments(ui, id, &style_options(), config.style, true)
                && style != config.style
            {
                output.actions.push(Action::SetStyle(style));
            }
        }
        Group::Size => {
            let enabled = config.size_enabled();
            let options = size_options(enabled);
            let id = Id::new("studio-size");
            if let Some(size) = kit::segments(ui, id, &options, size_shown(input), enabled)
                && size != config.size
            {
                output.actions.push(Action::SetSize(size));
            }
        }
        Group::Background => {
            if let Some(background) = swatches::row(ui, config.background)
                && background != config.background
            {
                output.actions.push(Action::SetBackground(background));
            }
        }
        Group::Include => include(ui, input, output),
    }
}

/// What an Include toggle says, and whether it can be used.
struct Toggle {
    icon: Icon,
    name: &'static str,
    on: bool,
    /// Why it cannot be used, if it cannot.
    unavailable: Option<&'static str>,
}

/// The three toggles in one pill, each switching its part on and off by itself.
fn include(ui: &mut Ui, input: &Input<'_>, output: &mut Output) {
    let config = input.config;
    let include = config.include;
    let toggles = [
        Toggle {
            icon: Icon::Indicators,
            name: tr!("Indicators"),
            on: include.indicators,
            unavailable: (!input.has_indicators).then_some(tr!("No indicators on this chart")),
        },
        Toggle {
            icon: Icon::Pencil,
            name: tr!("Drawings"),
            on: include.drawings,
            unavailable: (!input.has_drawings).then_some(tr!("No visible drawings")),
        },
        Toggle {
            icon: Icon::Branding,
            name: tr!("DegenView mark"),
            on: include.branding,
            unavailable: (config.style != Style::Styled)
                .then_some(tr!("The mark belongs to the Styled look")),
        },
    ];
    let (track, _) = ui.allocate_exact_size(vec2(INCLUDE_WIDTH, CONTROL_HEIGHT), Sense::hover());
    kit::outline::paint_track(
        ui.painter(),
        track,
        kit::outline::RADIUS,
        kit::outline::Wash::Rest,
    );
    for (index, toggle) in toggles.iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(
                track.left() + kit::outline::INSET + index as f32 * (TOGGLE.x + TOGGLE_GAP),
                track.top() + kit::outline::INSET,
            ),
            TOGGLE,
        );
        let usable = toggle.unavailable.is_none();
        let response = ui.interact(
            cell,
            Id::new(("studio-include", index)),
            if usable {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let described = if toggle.on && usable {
            tr!("{name}, on", name = toggle.name)
        } else {
            tr!("{name}, off", name = toggle.name)
        };
        let response = crate::ui::accessibility::label(response, described)
            .on_hover_text(toggle.unavailable.unwrap_or(toggle.name));
        let lit = toggle.on && usable;
        if lit {
            kit::outline::paint_plate(ui.painter(), cell, kit::outline::PLATE_RADIUS, false);
        } else if usable && response.hovered() {
            ui.painter()
                .rect_filled(cell, kit::outline::PLATE_RADIUS, theme::ink(10));
        }
        if usable && response.has_focus() {
            ui.painter().rect_stroke(
                cell,
                kit::outline::PLATE_RADIUS,
                Stroke::new(1.5, theme::text_secondary()),
                StrokeKind::Inside,
            );
        }
        let tint = match (usable, lit) {
            (false, _) => theme::text_tertiary(),
            (true, true) => theme::text_primary(),
            (true, false) => theme::text_secondary(),
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(cell.center(), egui::Vec2::splat(13.0)),
            toggle.icon,
            tint,
        );
        if usable && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if usable && response.clicked() {
            output
                .actions
                .push(Action::SetInclude(toggled(include, index)));
        }
    }
}

fn toggled(mut include: Include, index: usize) -> Include {
    match index {
        0 => include.indicators = !include.indicators,
        1 => include.drawings = !include.drawings,
        _ => include.branding = !include.branding,
    }
    include
}

#[cfg(test)]
mod line_tests {
    use super::*;

    #[test]
    fn groups_share_a_line_until_the_next_would_not_fit() {
        let widths = [100.0, 100.0, 100.0, 100.0];
        assert_eq!(lines(&widths, 1_000.0, 10.0), vec![vec![0, 1, 2, 3]]);
        assert_eq!(lines(&widths, 330.0, 10.0), vec![vec![0, 1, 2], vec![3]]);
        assert_eq!(lines(&widths, 210.0, 10.0), vec![vec![0, 1], vec![2, 3]]);
        // Exactly the room is room enough.
        assert_eq!(lines(&widths, 430.0, 10.0), vec![vec![0, 1, 2, 3]]);
    }

    #[test]
    fn a_group_wider_than_the_window_still_gets_a_line() {
        assert_eq!(lines(&[500.0, 40.0], 300.0, 10.0), vec![vec![0], vec![1]]);
        assert!(lines(&[], 300.0, 10.0).is_empty());
    }
}
