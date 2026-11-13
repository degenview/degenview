//! Shared drawing for the small scene pictures that head the Settings pages, the Alerts window and
//! the portfolio pickers: a card and marks that float over it, on a soft disc in a section colour
//! (not on Settings: its pictures set `flat`). Painter code only, so every picture follows the theme.

use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, Vec2, vec2};

/// The room a picture takes in a header.
pub const SIZE: Vec2 = vec2(96.0, 84.0);
pub(crate) const CARD: Vec2 = vec2(64.0, 54.0);

pub(crate) struct Art<'a> {
    pub(crate) ui: &'a Ui,
    pub(crate) painter: &'a Painter,
    pub(crate) tint: Color32,
    pub(crate) centre: Pos2,
    /// Settings pictures: no coloured disc behind the card, as the app's flat style asks.
    pub(crate) flat: bool,
}

/// The faint strokes that stand for text.
pub(crate) fn ghost() -> Color32 {
    theme::text_tertiary().gamma_multiply(0.7)
}

impl Art<'_> {
    pub(crate) fn disc(&self, centre: Pos2) {
        if self.flat {
            return;
        }
        self.painter
            .circle_filled(centre, 40.0, self.tint.gamma_multiply(0.13));
    }

    /// A card with the soft shadow and hairline the empty-tab pictures have.
    pub(crate) fn card_at(&self, rect: Rect, fill: Color32) {
        self.painter.add(
            egui::Shadow {
                offset: [0, 2],
                blur: 6,
                spread: 0,
                color: Color32::from_black_alpha(34),
            }
            .as_shape(rect, 9.0),
        );
        self.painter.rect_filled(rect, 9.0, fill);
        self.painter.rect_stroke(
            rect,
            9.0,
            Stroke::new(1.5, theme::text_tertiary()),
            StrokeKind::Inside,
        );
    }

    /// The disc and the card at `centre`; returns the card.
    pub(crate) fn scene_at(&self, centre: Pos2) -> Rect {
        self.disc(centre);
        let card = Rect::from_center_size(centre, CARD);
        self.card_at(card, theme::chrome());
        card
    }

    pub(crate) fn scene(&self) -> Rect {
        self.scene_at(self.centre)
    }

    pub(crate) fn line(&self, from: Pos2, length: f32, color: Color32) {
        self.painter
            .hline(from.x..=from.x + length, from.y, Stroke::new(2.2, color));
    }

    /// A rounded chip in the section's colour, for the marks that float over a card's edge.
    pub(crate) fn chip(&self, rect: Rect) {
        self.painter.add(
            egui::Shadow {
                offset: [0, 1],
                blur: 4,
                spread: 0,
                color: Color32::from_black_alpha(48),
            }
            .as_shape(rect, rect.height() / 2.0),
        );
        self.painter
            .rect_filled(rect, rect.height() / 2.0, self.tint);
    }

    pub(crate) fn glyph(&self, icon: Icon, centre: Pos2, size: f32, color: Color32) {
        icons::paint(
            self.painter,
            Rect::from_center_size(centre, Vec2::splat(size)),
            icon,
            color,
        );
    }

    /// A four-point sparkle.
    pub(crate) fn star(&self, centre: Pos2, radius: f32, color: Color32) {
        let points = (0..8)
            .map(|index| {
                let angle =
                    index as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
                let reach = if index % 2 == 0 {
                    radius
                } else {
                    radius * 0.36
                };
                centre + vec2(angle.cos(), angle.sin()) * reach
            })
            .collect();
        self.painter
            .add(Shape::convex_polygon(points, color, Stroke::NONE));
    }

    /// An arc about `centre` from `from` to `to` (radians, y down), drawn as a polyline.
    pub(crate) fn arc(
        &self,
        centre: Pos2,
        radius: f32,
        from: f32,
        to: f32,
        stroke: Stroke,
    ) -> Vec<Pos2> {
        let steps = 24;
        let points: Vec<Pos2> = (0..=steps)
            .map(|step| {
                let angle = from + (to - from) * step as f32 / steps as f32;
                centre + vec2(angle.cos(), angle.sin()) * radius
            })
            .collect();
        self.painter.add(Shape::line(points.clone(), stroke));
        points
    }

    /// A small triangle at `tip` pointing along `direction`.
    pub(crate) fn arrow_head(&self, tip: Pos2, direction: Vec2, size: f32, color: Color32) {
        let forward = direction.normalized();
        let side = vec2(-forward.y, forward.x);
        self.painter.add(Shape::convex_polygon(
            vec![
                tip + forward * size * 0.6,
                tip - forward * size * 0.4 + side * size * 0.55,
                tip - forward * size * 0.4 - side * size * 0.55,
            ],
            color,
            Stroke::NONE,
        ));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::ui::theme::AppTheme;

    /// Runs `paint` once on a 96x84 box and returns the box and every shape's bounds.
    pub fn drawn(app_theme: AppTheme, paint: impl FnMut(&Ui, Rect)) -> (Rect, Vec<Rect>) {
        drawn_sized(app_theme, SIZE, paint)
    }

    /// [`drawn`] for a picture of another size.
    pub fn drawn_sized(
        app_theme: AppTheme,
        size: Vec2,
        mut paint: impl FnMut(&Ui, Rect),
    ) -> (Rect, Vec<Rect>) {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        crate::ui::theme::apply(&ctx, app_theme);
        let rect = Rect::from_min_size(Pos2::new(40.0, 40.0), size);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size + vec2(80.0, 80.0))),
                ..Default::default()
            },
            |ui| paint(ui, rect),
        );
        output.textures_delta.clear();
        let bounds = output
            .shapes
            .iter()
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .filter(|bounds| bounds.is_finite() && bounds.width() > 0.0)
            .collect();
        (rect, bounds)
    }

    /// Every picture draws something, inside its box, in both themes.
    pub fn assert_all_inside<T: std::fmt::Debug + Copy>(
        scenes: &[T],
        paint: impl Fn(&Ui, Rect, T) + Copy,
    ) {
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        for scene in scenes {
            for app_theme in [AppTheme::Dark, AppTheme::Light] {
                let (rect, bounds) = drawn(app_theme, |ui, rect| paint(ui, rect, *scene));
                assert!(bounds.len() >= 5, "{scene:?} drew {} shapes", bounds.len());
                for shape in bounds {
                    assert!(
                        rect.expand(9.0).contains_rect(shape),
                        "{scene:?} {app_theme:?}: {shape:?} is outside {rect:?}"
                    );
                }
            }
        }
    }

    /// A contact sheet of every picture, in both themes, for a person to look at.
    pub fn gallery<T: Copy>(name: &str, scenes: &[T], paint: impl Fn(&Ui, Rect, T) + Copy) {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let cell = vec2(130.0, 112.0);
            let rows = scenes.len().div_ceil(3) as f32;
            let mut screen = Screen::new(vec2(cell.x * 3.0, cell.y * rows));
            egui_extras::install_image_loaders(&screen.ctx);
            crate::ui::theme::apply(&screen.ctx, app_theme);
            let mut last = None;
            for _ in 0..3 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let full = ui.max_rect();
                    ui.painter()
                        .rect_filled(full, 0.0, crate::ui::kit::window());
                    for (index, scene) in scenes.iter().enumerate() {
                        let centre = full.min
                            + vec2(
                                cell.x * ((index % 3) as f32 + 0.5),
                                cell.y * ((index / 3) as f32 + 0.5),
                            );
                        paint(ui, Rect::from_center_size(centre, SIZE), *scene);
                    }
                }));
            }
            screen.save(&format!("{name}-{theme_name}"), last.expect("a frame"));
        }
    }
}
