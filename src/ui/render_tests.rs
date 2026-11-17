//! Writing frames of egui as PNGs, for the tests that are run by hand to look at a screen the way
//! a person would. They are `#[ignore]`d and only write when `DEGENVIEW_GALLERY_DIR` names a
//! folder, so a normal test run neither needs nor leaves anything.

use crate::service::snapshot::{export, raster};
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

    /// The text `output` (a frame this screen ran) shows cut short with an ellipsis, in full: a label
    /// that did not fit its room. A text that has an ellipsis of its own ("Drawing the image…")
    /// does not count; only an ellipsis the layout added does. A page drawn in the pseudo-locale,
    /// which is 40% longer than English, must report none.
    pub(crate) fn truncated_text(output: &egui::FullOutput) -> Vec<String> {
        let ellipses = |text: &str| text.matches('…').count();
        painted_texts(output)
            .into_iter()
            .filter(|text| {
                let shown: String = text
                    .galley
                    .rows
                    .iter()
                    .flat_map(|row| row.glyphs.iter().map(|glyph| glyph.chr))
                    .collect();
                ellipses(&shown) > ellipses(text.galley.text())
            })
            .map(|text| text.galley.text().to_owned())
            .collect()
    }

    /// Every text `output` (a frame this screen ran) paints, as written in the source: the words of
    /// a screen, for the tests that check a page is drawn in the language they run in.
    pub(crate) fn words(output: &egui::FullOutput) -> Vec<String> {
        painted_texts(output)
            .into_iter()
            .map(|text| text.galley.text().to_owned())
            .collect()
    }

    /// Writes `output` (a frame this screen ran) as `name`.png in the gallery folder.
    pub(crate) fn save(&mut self, name: &str, output: egui::FullOutput) {
        let Some(dir) = gallery_dir() else {
            return;
        };
        let ppp = output.pixels_per_point;
        let scene = raster::Scene::new(
            self.ctx.tessellate(output.shapes, ppp),
            self.textures.clone(),
            ppp,
            ((self.size.x * ppp) as u32, (self.size.y * ppp) as u32),
            crate::ui::theme::chrome(),
        );
        let image = raster::rasterize(&scene).expect("drawn");
        let png = export::encode_png(&image).expect("encoded");
        std::fs::write(dir.join(format!("{name}.png")), png).expect("written");
    }
}

/// Every text shape of a frame, grouped shapes opened: the one walk `Screen::words` and
/// `Screen::truncated_text` share.
fn painted_texts(output: &egui::FullOutput) -> Vec<&egui::epaint::TextShape> {
    fn visit<'a>(shape: &'a egui::epaint::Shape, found: &mut Vec<&'a egui::epaint::TextShape>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|shape| visit(shape, found)),
            egui::epaint::Shape::Text(text) => found.push(text),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        visit(&clipped.shape, &mut found);
    }
    found
}

impl Drop for Screen {
    fn drop(&mut self) {
        // Kept for later pictures, never handed on: say so, or egui objects to dropping them.
        self.textures.clear();
    }
}

impl Screen {
    /// The words of `texts` (a frame's `Screen::words`) that read as English interface text but carry
    /// no pseudo-locale brackets: a literal nobody put through `tr!`. Text with no lowercase run of
    /// three letters (a glyph, a number, a ticker) and the `known` data (a symbol, a layout name) are
    /// not counted. A page drawn in the pseudo-locale must report none.
    pub(crate) fn untranslated<'a>(texts: &'a [String], known: &[&str]) -> Vec<&'a str> {
        texts
            .iter()
            .map(String::as_str)
            .filter(|text| !text.trim().is_empty())
            .filter(|text| !text.contains('[') && !text.contains(']'))
            .filter(|text| !known.iter().any(|known| text.contains(known)))
            .filter(|text| {
                let mut run = 0;
                text.chars().any(|c| {
                    run = if c.is_ascii_lowercase() { run + 1 } else { 0 };
                    run >= 3
                })
            })
            .collect()
    }
}
