//! Writing frames of egui as PNGs, for the tests that are run by hand to look at a screen the way
//! a person would. They are `#[ignore]`d and only write when `DEGENVIEW_GALLERY_DIR` names a
//! folder, so a normal test run neither needs nor leaves anything.

use crate::service::{snapshot_export, snapshot_raster};
use eframe::egui::{self, TexturesDelta, Vec2};
use std::path::PathBuf;

/// The folder the gallery tests write to, if one was asked for.
pub(crate) fn gallery_dir() -> Option<PathBuf> {
    std::env::var_os("DEGENVIEW_GALLERY_DIR").map(PathBuf::from)
}

/// An egui context that draws like the app at 2x, and remembers every texture it was asked to
/// make: a texture is sent once, with the frame that first needs it, and a picture of a later
/// frame is blank text without it.
pub(crate) struct Screen {
    pub(crate) ctx: egui::Context,
    pub(crate) size: Vec2,
    textures: TexturesDelta,
    time: f64,
}

impl Screen {
    pub(crate) fn new(size: Vec2) -> Self {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        ctx.set_pixels_per_point(2.0);
        Self {
            ctx,
            size,
            textures: TexturesDelta::default(),
            time: 0.0,
        }
    }

    /// Runs one frame with `events`.
    pub(crate) fn run(
        &mut self,
        events: Vec<egui::Event>,
        body: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        self.time += 0.1;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(raw, body);
        self.textures
            .append(std::mem::take(&mut output.textures_delta));
        output
    }

    /// Writes `output` (a frame this screen ran) as `name`.png in the gallery folder.
    pub(crate) fn save(&mut self, name: &str, output: egui::FullOutput) {
        let Some(dir) = gallery_dir() else {
            return;
        };
        let ppp = output.pixels_per_point;
        let scene = snapshot_raster::Scene::new(
            self.ctx.tessellate(output.shapes, ppp),
            self.textures.clone(),
            ppp,
            ((self.size.x * ppp) as u32, (self.size.y * ppp) as u32),
            crate::ui::theme::chrome(),
        );
        let image = snapshot_raster::rasterize(&scene).expect("drawn");
        let png = snapshot_export::encode_png(&image).expect("encoded");
        std::fs::write(dir.join(format!("{name}.png")), png).expect("written");
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // Kept for later pictures, never handed on: say so, or egui objects to dropping them.
        self.textures.clear();
    }
}

/// Runs `body` for a few frames on a fresh `size` screen and writes the last as `name`.png.
pub(crate) fn render(name: &str, size: Vec2, mut body: impl FnMut(&mut egui::Ui)) {
    if gallery_dir().is_none() {
        return;
    }
    let mut screen = Screen::new(size);
    let mut last = None;
    for _ in 0..4 {
        last = Some(screen.run(Vec::new(), &mut body));
    }
    screen.save(name, last.expect("a frame"));
}
