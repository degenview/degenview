//! A frame painted on a context of its own and tessellated into a [`Scene`] the rasterizer
//! draws: what a picture of a chart, or of a composition of them, is made of.
//!
//! The context is a scratch one at the image's scale, so text and lines are laid out for the
//! pixels they land on and never stretched to them. It has the app's fonts and theme and no
//! window, no input and no store.
//!
//! Pictures the frame paints are handed in by name, decoded, because the context has no store to
//! ask: a caller adds a logo or an icon to the map it passes as `images`, and the painter reads
//! it back with [`asset_host::lookup`] and uploads it with `ctx.load_texture`, as the picture
//! drawings do. A name that is not in the map paints as missing.

use crate::service::annotation::asset_host::{self, FrozenAssets};
use crate::service::snapshot::raster::Scene;
use crate::ui::theme;
use eframe::egui::{self, Color32, ColorImage, Painter, Pos2, Rect, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// The pictures a frame may paint, by name.
pub type Images = HashMap<String, Arc<ColorImage>>;

/// Paints one frame of `size` points at `pixels_per_point` and gives back what `paint` returned
/// with the frame's output.
fn run_frame<R>(
    ctx: &egui::Context,
    size: Vec2,
    pixels_per_point: f32,
    paint: impl FnOnce(&Painter, Rect) -> R,
) -> (R, egui::FullOutput) {
    let rect = Rect::from_min_size(Pos2::ZERO, size);
    let raw = egui::RawInput {
        screen_rect: Some(rect),
        viewports: std::iter::once((
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(pixels_per_point),
                ..Default::default()
            },
        ))
        .collect(),
        ..Default::default()
    };
    let mut paint = Some(paint);
    let mut result = None;
    let output = ctx.run_ui(raw, |ui| {
        if let Some(paint) = paint.take() {
            result = Some(paint(&ui.painter_at(rect), rect));
        }
    });
    (result.expect("egui runs a frame once"), output)
}

fn scratch_context() -> egui::Context {
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    ctx
}

/// `paint` tessellated into a `size_px` image at `pixels_per_point` that starts out as
/// `background`. `paint` gets a painter and the rectangle of the whole frame, in points from the
/// origin; its error is the render's and leaves no scene.
pub fn render<E>(
    size_px: (u32, u32),
    pixels_per_point: f32,
    background: Color32,
    images: Option<&Images>,
    paint: impl FnOnce(&Painter, Rect) -> Result<(), E>,
) -> Result<Scene, E> {
    let ctx = scratch_context();
    if let Some(images) = images {
        asset_host::freeze(&ctx, FrozenAssets(Arc::new(images.clone())));
    }
    let size = Vec2::new(
        size_px.0 as f32 / pixels_per_point,
        size_px.1 as f32 / pixels_per_point,
    );
    let (result, mut output) = run_frame(&ctx, size, pixels_per_point, paint);
    if let Err(error) = result {
        // The fonts it uploaded have nowhere to go.
        output.textures_delta.clear();
        return Err(error);
    }
    Ok(Scene::new(
        ctx.tessellate(output.shapes, output.pixels_per_point),
        output.textures_delta,
        output.pixels_per_point,
        size_px,
        background,
    ))
}

/// What `measure` finds out in a frame of `size` points at `pixels_per_point`, with nothing
/// tessellated: where things would land if `render` painted them.
pub fn measure<R>(
    size: Vec2,
    pixels_per_point: f32,
    inspect: impl FnOnce(&Painter, Rect) -> R,
) -> R {
    let (found, mut output) = run_frame(&scratch_context(), size, pixels_per_point, inspect);
    // Nothing is drawn, so the fonts it uploaded have nowhere to go.
    output.textures_delta.clear();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::annotation::asset_host::AssetImage;
    use crate::service::snapshot::raster::rasterize;

    #[test]
    fn a_frame_is_an_image_of_the_size_and_background_asked_for() {
        let scene = render((120, 80), 2.0, Color32::RED, None, |painter, rect| {
            assert_eq!(rect.size(), Vec2::new(60.0, 40.0), "points, not pixels");
            assert_eq!(painter.ctx().pixels_per_point(), 2.0);
            painter.rect_filled(
                Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)),
                0.0,
                Color32::BLUE,
            );
            Ok::<(), ()>(())
        })
        .expect("drawn");
        let image = rasterize(&scene).expect("drawn");
        assert_eq!(image.size, [120, 80]);
        assert_eq!(image.pixels[0], Color32::BLUE, "the square, 20 px across");
        assert_eq!(image.pixels[19 * 120 + 19], Color32::BLUE);
        assert_eq!(image.pixels[30 * 120 + 30], Color32::RED, "the background");
    }

    #[test]
    fn a_frame_that_fails_leaves_no_scene() {
        let result = render((10, 10), 1.0, Color32::BLACK, None, |_, _| Err("no chart"));
        assert_eq!(result.err(), Some("no chart"));
    }

    #[test]
    fn named_pictures_reach_the_painter_and_other_names_are_missing() {
        let logo = Arc::new(ColorImage::example());
        let images = Images::from([("logo".to_owned(), Arc::clone(&logo))]);
        let found = std::cell::RefCell::new(Vec::new());
        render(
            (10, 10),
            1.0,
            Color32::BLACK,
            Some(&images),
            |painter, _| {
                for name in ["logo", "icon"] {
                    found
                        .borrow_mut()
                        .push(asset_host::lookup(painter.ctx(), name));
                }
                Ok::<(), ()>(())
            },
        )
        .expect("drawn");
        let found = found.into_inner();
        assert!(matches!(&found[0], AssetImage::Ready(image) if Arc::ptr_eq(image, &logo)));
        assert!(matches!(found[1], AssetImage::Missing));
    }

    #[test]
    fn measuring_gives_back_what_was_found_at_the_scale_asked_for() {
        let width = |ppp| {
            measure(Vec2::new(100.0, 50.0), ppp, |painter, rect| {
                assert_eq!(painter.ctx().pixels_per_point(), ppp);
                rect.width()
            })
        };
        assert_eq!(width(1.0), 100.0);
        assert_eq!(width(3.0), 100.0, "layout is in points at any scale");
    }
}
