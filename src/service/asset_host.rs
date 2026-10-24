//! The pictures drawings show, loaded for painting.
//!
//! A drawing names its picture by hash; this holds the decoded pixels of the ones in view. A
//! picture is read from the store and decoded once, on a worker (never while painting), and kept
//! until it is not asked for. One that is not in the store (it may be on its way from another Mac)
//! is asked for again a little later, so it appears by itself when it arrives. One whose bytes are
//! not what its name says is marked damaged and left alone.
//!
//! The host lives in the egui context, so painting reaches it from wherever it has the context,
//! and a snapshot (which paints in a context of its own) finds the pictures it was handed instead.

use super::image_ingest;
use super::work_queue;
use crate::store::SharedStore;
use eframe::egui::{self, ColorImage};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a picture that was not in the store waits before the store is looked in again.
const RETRY_AFTER: Duration = Duration::from_secs(2);

/// What a picture is, as far as painting is concerned.
#[derive(Clone)]
pub enum AssetImage {
    /// Being read from the store and decoded.
    Loading,
    /// Not in the store (yet).
    Missing,
    /// In the store but not a picture it can show.
    Corrupt,
    Ready(Arc<ColorImage>),
}

struct Slot {
    image: AssetImage,
    since: Instant,
}

pub struct AssetHost {
    store: SharedStore,
    slots: Mutex<HashMap<String, Slot>>,
}

impl AssetHost {
    pub fn new(store: SharedStore) -> Arc<Self> {
        Arc::new(Self {
            store,
            slots: Mutex::new(HashMap::new()),
        })
    }

    /// The picture named `id`, started loading if nothing has asked for it. Never blocks.
    pub fn image(self: &Arc<Self>, id: &str, ctx: &egui::Context) -> AssetImage {
        let Ok(mut slots) = self.slots.lock() else {
            return AssetImage::Missing;
        };
        let start = match slots.get(id) {
            None => true,
            Some(slot) => {
                matches!(slot.image, AssetImage::Missing) && slot.since.elapsed() >= RETRY_AFTER
            }
        };
        if start {
            slots.insert(
                id.to_owned(),
                Slot {
                    image: AssetImage::Loading,
                    since: Instant::now(),
                },
            );
            let (host, ctx, id) = (Arc::clone(self), ctx.clone(), id.to_owned());
            work_queue::ASSETS.submit(move || host.load(&id, &ctx));
        }
        let image = slots
            .get(id)
            .map_or(AssetImage::Missing, |slot| slot.image.clone());
        if matches!(image, AssetImage::Loading | AssetImage::Missing) {
            // Look again: it is on its way, or it may arrive from another Mac.
            ctx.request_repaint_after(RETRY_AFTER);
        }
        image
    }

    /// The pixels of a picture that is already loaded, without asking for it to be.
    pub fn ready(&self, id: &str) -> Option<Arc<ColorImage>> {
        match &self.slots.lock().ok()?.get(id)?.image {
            AssetImage::Ready(image) => Some(Arc::clone(image)),
            _ => None,
        }
    }

    /// Records the pixels of a picture just added, so showing it needs no read and no decode.
    pub fn insert_ready(&self, id: &str, image: ColorImage) {
        if let Ok(mut slots) = self.slots.lock() {
            slots.insert(
                id.to_owned(),
                Slot {
                    image: AssetImage::Ready(Arc::new(image)),
                    since: Instant::now(),
                },
            );
        }
    }

    fn load(&self, id: &str, ctx: &egui::Context) {
        let found = self
            .store
            .lock()
            .ok()
            .and_then(|store| store.asset(id).ok())
            .flatten();
        let image = match found {
            None => AssetImage::Missing,
            Some(asset) => asset
                .verified_bytes()
                .and_then(|bytes| image_ingest::pixels(&bytes))
                .map_or(AssetImage::Corrupt, |pixels| {
                    AssetImage::Ready(Arc::new(pixels))
                }),
        };
        if let Ok(mut slots) = self.slots.lock() {
            slots.insert(
                id.to_owned(),
                Slot {
                    image,
                    since: Instant::now(),
                },
            );
        }
        ctx.request_repaint();
    }
}

/// The picture a snapshot was handed: decoded pixels frozen on the UI thread, so painting it
/// needs no store and no worker.
#[derive(Clone, Default)]
pub struct FrozenAssets(pub Arc<HashMap<String, Arc<ColorImage>>>);

const HOST_ID: &str = "asset-host";
const FROZEN_ID: &str = "frozen-assets";

/// Puts the host in the context, for painting to find.
pub fn install(ctx: &egui::Context, host: Arc<AssetHost>) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(HOST_ID), host));
}

/// The host in the context, if there is one.
pub fn host(ctx: &egui::Context) -> Option<Arc<AssetHost>> {
    ctx.data(|data| data.get_temp::<Arc<AssetHost>>(egui::Id::new(HOST_ID)))
}

/// Gives a context (a snapshot's) the pictures to paint instead of a host.
pub fn freeze(ctx: &egui::Context, frozen: FrozenAssets) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(FROZEN_ID), frozen));
}

/// The picture named `id` as the context shows it: from what it was handed, else from the host,
/// else missing.
pub fn lookup(ctx: &egui::Context, id: &str) -> AssetImage {
    if let Some(frozen) = ctx.data(|data| data.get_temp::<FrozenAssets>(egui::Id::new(FROZEN_ID))) {
        return frozen.0.get(id).map_or(AssetImage::Missing, |image| {
            AssetImage::Ready(Arc::clone(image))
        });
    }
    host(ctx).map_or(AssetImage::Missing, |host| host.image(id, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::store::asset_store::StoredAsset;

    fn shared() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    fn stored(seed: u8) -> StoredAsset {
        let image =
            image::RgbaImage::from_fn(12, 8, |x, y| image::Rgba([seed, x as u8, y as u8, 255]));
        let mut png = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut png),
            image.as_raw(),
            12,
            8,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        StoredAsset::from_ingested(&image_ingest::ingest(&png).unwrap())
    }

    fn wait(host: &Arc<AssetHost>, ctx: &egui::Context, id: &str) -> AssetImage {
        for _ in 0..200 {
            let image = host.image(id, ctx);
            if !matches!(image, AssetImage::Loading) {
                return image;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("still loading");
    }

    #[test]
    fn a_stored_picture_loads_off_the_calling_thread_and_is_kept_decoded() {
        let store = shared();
        let asset = stored(1);
        store.lock().unwrap().put_asset(&asset).unwrap();
        let host = AssetHost::new(store);
        let ctx = egui::Context::default();
        assert!(matches!(host.image(&asset.id, &ctx), AssetImage::Loading));
        let AssetImage::Ready(pixels) = wait(&host, &ctx, &asset.id) else {
            panic!("not ready");
        };
        assert_eq!(pixels.size, [12, 8]);
        assert!(Arc::ptr_eq(&pixels, &host.ready(&asset.id).unwrap()));
        // Asked again, it is the same decode.
        let AssetImage::Ready(again) = host.image(&asset.id, &ctx) else {
            panic!();
        };
        assert!(Arc::ptr_eq(&pixels, &again));
    }

    #[test]
    fn a_picture_not_stored_is_missing_and_appears_once_it_arrives() {
        let store = shared();
        let asset = stored(2);
        let host = AssetHost::new(store.clone());
        let ctx = egui::Context::default();
        assert!(matches!(wait(&host, &ctx, &asset.id), AssetImage::Missing));
        store.lock().unwrap().put_asset_from_cloud(&asset).unwrap();
        // It is looked for again after a moment, not on every frame.
        assert!(matches!(host.image(&asset.id, &ctx), AssetImage::Missing));
        std::thread::sleep(RETRY_AFTER + Duration::from_millis(100));
        let _ = host.image(&asset.id, &ctx);
        assert!(matches!(wait(&host, &ctx, &asset.id), AssetImage::Ready(_)));
    }

    #[test]
    fn a_stored_row_that_is_not_its_name_is_damaged_not_shown() {
        let store = shared();
        let mut asset = stored(3);
        let real = asset.id.clone();
        // A row whose bytes do not hash to its id (as damage on disk would leave it).
        asset.data = stored(4).data;
        store.lock().unwrap().put_asset_from_cloud(&asset).unwrap();
        let host = AssetHost::new(store);
        let ctx = egui::Context::default();
        assert!(matches!(wait(&host, &ctx, &real), AssetImage::Corrupt));
        assert!(host.ready(&real).is_none());
    }

    #[test]
    fn a_just_added_picture_needs_no_read() {
        let host = AssetHost::new(shared());
        host.insert_ready("x", ColorImage::example());
        assert!(host.ready("x").is_some());
        assert!(host.ready("y").is_none());
    }

    #[test]
    fn a_context_with_frozen_pictures_never_asks_the_host() {
        let ctx = egui::Context::default();
        assert!(
            matches!(lookup(&ctx, "a"), AssetImage::Missing),
            "no host, no pictures"
        );
        let host = AssetHost::new(shared());
        install(&ctx, host);
        let picture = Arc::new(ColorImage::example());
        freeze(
            &ctx,
            FrozenAssets(Arc::new(HashMap::from([("a".to_owned(), picture)]))),
        );
        assert!(matches!(lookup(&ctx, "a"), AssetImage::Ready(_)));
        assert!(matches!(lookup(&ctx, "b"), AssetImage::Missing));
    }
}
