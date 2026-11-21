//! The outlined control look shared by segmented controls, tab bars and pull-down pickers: a dark
//! well with a hairline border, the chosen segment raised on a flat lighter plate in bold type.
//! One definition, so the three never drift apart.

use super::{BOLD, primary};
use crate::ui::theme;
use eframe::egui::{Color32, Painter, Rect, Stroke, StrokeKind, pos2};

/// The height of a segmented control or a picker in a form row, level with the fields and buttons
/// beside it.
pub const HEIGHT: f32 = 24.0;
/// The height of a control in a header row (the Paper panel's account chip and tab bar).
pub const TALL_HEIGHT: f32 = 28.0;
/// The corner radius of the well.
pub const RADIUS: f32 = 8.0;
/// The gap between the well's edge and the plates inside it.
pub const INSET: f32 = 3.0;
/// The corner radius of a plate: the well's, less the gap around it.
pub const PLATE_RADIUS: f32 = RADIUS - INSET;
/// The weight of the chosen segment's label.
pub const SELECTED_WEIGHT: f32 = BOLD;

/// How the well is drawn: at rest, under the pointer, while its menu is open, or disabled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wash {
    Rest,
    Hover,
    Open,
    Disabled,
}

/// The well: a darker fill and a clearly visible hairline, lifted a little by `wash`.
pub fn paint_track(painter: &Painter, rect: Rect, radius: f32, wash: Wash) {
    let fade = if wash == Wash::Disabled { 0.6 } else { 1.0 };
    painter.rect_filled(rect, radius, theme::well_fill().gamma_multiply(fade));
    let lift = match wash {
        Wash::Rest | Wash::Disabled => 0.0,
        Wash::Hover => 0.05,
        Wash::Open => 0.09,
    };
    if lift > 0.0 {
        painter.rect_filled(rect, radius, primary(lift));
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0,
            theme::card_border(matches!(wash, Wash::Hover | Wash::Open)).gamma_multiply(fade),
        ),
        StrokeKind::Inside,
    );
}

/// The plate under the chosen segment: flat, lighter than the well, no edge or shadow. `faded`
/// dims it for a disabled control.
pub fn paint_plate(painter: &Painter, rect: Rect, radius: f32, faded: bool) {
    let fill = theme::pick(primary(0.16), Color32::WHITE);
    painter.rect_filled(
        rect,
        radius,
        if faded {
            fill.gamma_multiply(0.6)
        } else {
            fill
        },
    );
}

/// The hairline between two neighbouring segments, drawn at `x` and inset from `segment`'s ends.
pub fn paint_divider(painter: &Painter, x: f32, segment: Rect) {
    painter.vline(
        x,
        (segment.top() + 5.0)..=(segment.bottom() - 5.0),
        Stroke::new(1.0, primary(0.16)),
    );
}

/// The plate rectangle of a segment `width` wide at `left` in a well whose top edge is at `top`
/// and whose content is `content_height` tall.
pub fn plate_rect(left: f32, top: f32, width: f32, content_height: f32) -> Rect {
    Rect::from_min_size(
        pos2(left, top + INSET),
        eframe::egui::vec2(width, content_height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::icons::Icon;
    use crate::ui::kit::{self, TabItem};
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::AppTheme;
    use eframe::egui::{self, Id, vec2};

    /// The outlined controls side by side, as the Paper and Liquidations headers put them, in both
    /// themes: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_outlined_controls -- --ignored`.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_outlined_controls() {
        if gallery_dir().is_none() {
            return;
        }
        let _restore = theme::RestoreDefaultTheme;
        for (suffix, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let mut screen = Screen::new(vec2(640.0, 230.0));
            theme::apply(&screen.ctx, app_theme);
            let mut mode = 1;
            let mut tab = 0;
            let mut last = None;
            for _ in 0..2 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let ground = ui.max_rect();
                    ui.painter().rect_filled(ground, 0.0, theme::background());
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.spacing_mut().item_spacing.x = 10.0;
                        kit::segmented(
                            ui,
                            Id::new("gallery-mode"),
                            &[(0, "Liquidations"), (1, "Taker flow"), (2, "Book")],
                            &mut mode,
                        );
                        let labels = vec!["10".to_owned(), "100".to_owned()];
                        kit::menu_picker(ui, 96.0, &labels, 0, true);
                        kit::menu_picker(ui, 96.0, &labels, 1, false);
                    });
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let (rect, _) = ui.allocate_exact_size(
                            vec2(120.0, crate::ui::trade::panel_header::CONTROL_HEIGHT),
                            egui::Sense::hover(),
                        );
                        crate::ui::trade::panel_header::plate(ui.painter(), rect, false, false);
                        let items = [
                            TabItem {
                                value: 0,
                                title: "Positions",
                                icon: Icon::ChartUptrend,
                                count: 2,
                            },
                            TabItem {
                                value: 1,
                                title: "Orders",
                                icon: Icon::List,
                                count: 0,
                            },
                            TabItem {
                                value: 2,
                                title: "History",
                                icon: Icon::Clock,
                                count: 0,
                            },
                        ];
                        kit::tab_bar_slim(ui, Id::new("gallery-tabs"), &items, &mut tab);
                    });
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let items = [
                            TabItem {
                                value: 0,
                                title: "Style",
                                icon: Icon::List,
                                count: 0,
                            },
                            TabItem {
                                value: 1,
                                title: "Canvas",
                                icon: Icon::Clock,
                                count: 0,
                            },
                        ];
                        kit::tab_bar_regular(ui, Id::new("gallery-regular"), &items, &mut tab);
                    });
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        kit::segmented_filling(
                            ui,
                            Id::new("gallery-fill"),
                            &[(0, "All"), (1, "Unresolved"), (2, "Mine")],
                            &mut mode,
                            300.0,
                        );
                    });
                }));
            }
            screen.save(
                &format!("outlined-controls-{suffix}"),
                last.expect("a frame"),
            );
        }
    }

    #[test]
    fn plates_sit_inside_the_well_by_the_inset() {
        let well = Rect::from_min_size(pos2(10.0, 20.0), eframe::egui::vec2(200.0, HEIGHT));
        let plate = plate_rect(well.left() + INSET, well.top(), 60.0, HEIGHT - 2.0 * INSET);
        assert_eq!(plate.top() - well.top(), INSET);
        assert_eq!(well.bottom() - plate.bottom(), INSET);
        assert_eq!(plate.left() - well.left(), INSET);
        assert_eq!(PLATE_RADIUS, RADIUS - INSET);
    }
}
