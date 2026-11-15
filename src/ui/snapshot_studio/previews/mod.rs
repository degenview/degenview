//! Making the Studio's preview: which picture is wanted, which one is being drawn, which one is
//! shown.
//!
//! A picture takes two steps. The scene is composed on the UI thread (the cards are read there,
//! and it is a few milliseconds) and rasterized on a worker; the result is uploaded as one texture
//! that every later picture replaces, so the old picture stays on screen until a newer one has
//! arrived and nothing flickers.
//!
//! Every change of what is wanted is a new *generation*. At most one raster job is in flight; the
//! newest wanted picture waits for it to end and is started then (latest wins, and a picture that
//! was overtaken is never started). A job's picture is shown if it is newer than the one shown, so
//! a drag is followed by the pictures that finish while it goes on, not only by its last; a job
//! of before a [`restart`](Previews::restart) (the capture was replaced) is dropped. The module
//! holds no workspace and no thread: the caller says how a scene is composed and where it is
//! rasterized.

use super::compose::ComposeError;
use crate::model::chart::snapshot::studio::StudioConfig;
use crate::service::snapshot::raster::{RasterError, Scene};
use crate::service::work_queue::Pending;
use eframe::egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions, Vec2};

/// What a picture is made from: the look, the room it may take and how coarsely.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub config: StudioConfig,
    /// The most pixels it may have, across and down.
    pub max_px: (u32, u32),
    /// Drawn at half the pixels each way, for as long as a drag goes on.
    pub coarse: bool,
}

impl Key {
    /// The box the scene is fitted into.
    pub fn target_px(&self) -> (u32, u32) {
        if self.coarse {
            ((self.max_px.0 / 2).max(1), (self.max_px.1 / 2).max(1))
        } else {
            (self.max_px.0.max(1), self.max_px.1.max(1))
        }
    }
}

/// A composed scene, ready for a worker, and the size in points of the composition it shows.
pub struct Composed {
    pub scene: Scene,
    pub composition: Vec2,
}

/// The picture on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub texture: TextureId,
    pub pixels: [usize; 2],
    pub composition: Vec2,
    pub generation: u64,
}

struct Flight {
    epoch: u64,
    generation: u64,
    composition: Vec2,
    pending: Pending<Result<ColorImage, RasterError>>,
}

#[derive(Default)]
pub struct Previews {
    /// Raised by [`restart`](Self::restart): jobs of an earlier epoch are not shown.
    epoch: u64,
    generation: u64,
    /// The last thing asked for, so asking again for the same changes nothing.
    asked: Option<Key>,
    /// The newest thing asked for that no job has been started for.
    wanted: Option<(u64, Key)>,
    flight: Option<Flight>,
    texture: Option<TextureHandle>,
    shown: Option<Shown>,
    failure: Option<ComposeError>,
}

impl Previews {
    /// Asks for the picture of `key`. The same key as last time asks for nothing new.
    pub fn want(&mut self, key: Key) {
        if self.asked.as_ref() == Some(&key) {
            return;
        }
        self.generation += 1;
        self.asked = Some(key.clone());
        self.wanted = Some((self.generation, key));
    }

    /// The capture was replaced: nothing in flight is shown and the next [`want`](Self::want) is
    /// started whatever it asks. The picture on screen stays until a new one replaces it.
    pub fn restart(&mut self) {
        self.epoch += 1;
        self.asked = None;
        self.wanted = None;
        self.failure = None;
    }

    /// Takes the finished job's picture, and starts the wanted one if no job is running.
    /// `compose` builds the scene for a key on the calling thread; `rasterize` hands it to a
    /// worker.
    pub fn tick(
        &mut self,
        ctx: &Context,
        compose: impl FnOnce(&Key) -> Result<Composed, ComposeError>,
        rasterize: impl FnOnce(Scene) -> Pending<Result<ColorImage, RasterError>>,
    ) {
        self.collect(ctx);
        if self.flight.is_some() {
            return;
        }
        let Some((generation, key)) = self.wanted.take() else {
            return;
        };
        match compose(&key) {
            Ok(composed) => {
                self.failure = None;
                self.flight = Some(Flight {
                    epoch: self.epoch,
                    generation,
                    composition: composed.composition,
                    pending: rasterize(composed.scene),
                });
            }
            Err(error) => self.failure = Some(error),
        }
    }

    fn collect(&mut self, ctx: &Context) {
        let Some(flight) = &mut self.flight else {
            return;
        };
        let Some(result) = flight.pending.poll_or(|| Err(RasterError::Empty)) else {
            return;
        };
        let Some(flight) = self.flight.take() else {
            return;
        };
        let newer = self
            .shown
            .is_none_or(|shown| flight.generation > shown.generation);
        if flight.epoch != self.epoch || !newer {
            return;
        }
        match result {
            Ok(image) => self.show(ctx, image, &flight),
            Err(_) => self.failure = Some(ComposeError::TooLarge),
        }
    }

    fn show(&mut self, ctx: &Context, image: ColorImage, flight: &Flight) {
        let pixels = image.size;
        let handle = match self.texture.take() {
            Some(mut handle) => {
                handle.set(image, TextureOptions::LINEAR);
                handle
            }
            None => ctx.load_texture("snapshot-studio-preview", image, TextureOptions::LINEAR),
        };
        self.shown = Some(Shown {
            texture: handle.id(),
            pixels,
            composition: flight.composition,
            generation: flight.generation,
        });
        self.texture = Some(handle);
    }

    /// The picture to show, if one has been made.
    pub fn shown(&self) -> Option<Shown> {
        self.shown
    }

    /// Why the newest picture could not be made, if it could not.
    pub fn failure(&self) -> Option<ComposeError> {
        self.failure
    }

    /// Whether a picture is being made or waits for its turn.
    #[cfg(test)]
    pub fn is_busy(&self) -> bool {
        self.flight.is_some() || self.wanted.is_some()
    }

    /// The generation of the newest picture asked for.
    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether a job is in flight.
    #[cfg(test)]
    pub fn in_flight(&self) -> bool {
        self.flight.is_some()
    }
}

#[cfg(test)]
mod tests;
